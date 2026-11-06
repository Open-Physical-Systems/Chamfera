//! Tests of projections, sheets, dimensions and the writers. Expected values are worked
//! out by hand from the part's size; the derivation is in each test's comments.

use crate::*;
use chamfera_doc::drawing::{SheetDimension, SheetView};
use chamfera_geom::{Frame, Point2, Point3, Vec3};
use chamfera_kernel::{Body, Profile, ProfileEdge, extrude};

const EPS: f64 = 1e-6;

/// 50 x 30 x 10 box from the origin: x 0..50, y 0..30, z 0..10.
pub(crate) fn block() -> Body {
    Body::cuboid(Point3::new(0.0, 0.0, 0.0), Vec3::new(50.0, 30.0, 10.0)).unwrap()
}

/// The same box with a through hole of diameter 6 at (25, 15), along Z.
pub(crate) fn plate_with_hole() -> Body {
    let outer = chamfera_kernel::polygon_edges(&[
        Point2::new(0.0, 0.0),
        Point2::new(50.0, 0.0),
        Point2::new(50.0, 30.0),
        Point2::new(0.0, 30.0),
    ]);
    let hole = vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 3.0)];
    extrude(&Frame::XY, &Profile::new(outer, vec![hole]), 10.0).unwrap()
}

/// A cylinder r = 5, h = 10 on the XY plane at the origin.
pub(crate) fn cylinder() -> Body {
    extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap()
}

pub(crate) fn lines(g: &ViewGeometry, hidden: bool) -> Vec<(Point2, Point2)> {
    g.curves
        .iter()
        .filter(|c| c.hidden == hidden)
        .filter_map(|c| match c.curve {
            Curve2::Line { a, b } => Some((a, b)),
            _ => None,
        })
        .collect()
}

pub(crate) fn sheet_with(views: Vec<SheetView>, dims: Vec<SheetDimension>) -> Sheet {
    Sheet {
        id: 1,
        name: "Sheet 1".into(),
        paper: Paper::A3,
        orientation: Orientation::Landscape,
        scale: 1.0,
        projection: ProjectionAngle::Third,
        title: TitleBlock {
            part_name: "Block".into(),
            drawn_by: "Test".into(),
            date: "2026-11-06".into(),
        },
        views,
        dimensions: dims,
        annotations: Vec::new(),
        parts_list: None,
    }
}

pub(crate) fn view(id: u32, d: ViewDirection, x: f64, y: f64, parent: Option<u32>) -> SheetView {
    SheetView {
        id,
        body: None,
        direction: d,
        x,
        y,
        scale: None,
        parent,
        section: None,
        detail: None,
    }
}

#[test]
fn the_front_view_of_a_50_30_10_box_is_a_50_by_10_rectangle() {
    let b = block();
    let g = project(&[&b], ViewDirection::Front).unwrap();
    // Seen from -Y with X right and Z up: the front face, 50 wide and 10 high. The back
    // edges lie exactly behind the front ones and the four edges along Y are points,
    // so four visible lines remain and nothing is hidden.
    let vis = lines(&g, false);
    assert_eq!(vis.len(), 4, "{vis:?}");
    assert!(lines(&g, true).is_empty());
    let (lo, hi) = g.bounds().unwrap();
    assert!((hi.x - lo.x - 50.0).abs() < EPS && (hi.y - lo.y - 10.0).abs() < EPS);
    let mut lengths: Vec<f64> = vis.iter().map(|(a, b)| a.distance(*b)).collect();
    lengths.sort_by(f64::total_cmp);
    for (got, want) in lengths.iter().zip([10.0, 10.0, 50.0, 50.0]) {
        assert!((got - want).abs() < EPS, "{lengths:?}");
    }
    // At scale 1 on a sheet the rectangle keeps its size.
    let s = sheet_with(
        vec![view(2, ViewDirection::Front, 150.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let (lo, hi) = out.view(2).unwrap().bounds().unwrap();
    assert!((hi.x - lo.x - 50.0).abs() < EPS && (hi.y - lo.y - 10.0).abs() < EPS);
    // The model's centre (25, 5) goes to the view's position.
    assert!((lo.x - 125.0).abs() < EPS && (lo.y - 145.0).abs() < EPS);
}

#[test]
fn a_through_hole_seen_from_the_front_gives_two_hidden_lines_6_apart() {
    let b = plate_with_hole();
    let g = project(&[&b], ViewDirection::Front).unwrap();
    // The hole's silhouettes are the generators at x = 25 - 3 and 25 + 3, from z = 0 to
    // z = 10, behind the front face: hidden. Its circles are edge-on and lie on the
    // plate's top and bottom outline, so they add nothing.
    let hid = lines(&g, true);
    assert_eq!(hid.len(), 2, "{hid:?}");
    let mut xs: Vec<f64> = hid.iter().map(|(a, _)| a.x).collect();
    xs.sort_by(f64::total_cmp);
    assert!(
        (xs[0] - 22.0).abs() < EPS && (xs[1] - 28.0).abs() < EPS,
        "{xs:?}"
    );
    assert!((xs[1] - xs[0] - 6.0).abs() < EPS);
    for (a, b) in &hid {
        assert!((a.x - b.x).abs() < EPS, "vertical");
        assert!((a.distance(*b) - 10.0).abs() < 1e-4, "full height");
    }
    assert_eq!(lines(&g, false).len(), 4);
    // The hole's axis is a centre line from z = 0 to z = 10 at x = 25.
    assert_eq!(g.centre_lines.len(), 1);
    let [a, c] = g.centre_lines[0];
    assert!((a.x - 25.0).abs() < EPS && (c.x - 25.0).abs() < EPS);
    assert!(((a.y - c.y).abs() - 10.0).abs() < EPS);
}

#[test]
fn the_top_view_of_a_cylinder_is_a_circle_with_a_centre_mark() {
    let b = cylinder();
    let g = project(&[&b], ViewDirection::Top).unwrap();
    // Seen from above, both end circles project onto one circle r = 5 at the origin,
    // the visible top one; the side face is seen end-on and adds nothing.
    assert_eq!(g.curves.len(), 1, "{:?}", g.curves);
    match g.curves[0] {
        ViewCurve {
            curve: Curve2::Circle { centre, radius },
            hidden: false,
        } => {
            assert!(centre.distance(Point2::ORIGIN) < EPS);
            assert!((radius - 5.0).abs() < EPS);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(g.centre_marks.len(), 1);
    assert!(g.centre_marks[0].0.distance(Point2::ORIGIN) < EPS);
    assert!((g.centre_marks[0].1 - 5.0).abs() < EPS);
    // On the sheet the centre mark is two centre lines through the centre, reaching
    // 2 mm past the circle: 2 (5 + 2) = 14 mm long.
    let s = sheet_with(
        vec![view(2, ViewDirection::Top, 200.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let marks: Vec<f64> = out
        .items
        .iter()
        .filter(|i| i.layer == Layer::Center)
        .filter_map(|i| match i.shape {
            Shape::Line { a, b } if a.distance(Point2::new(200.0, 150.0)) < 8.0 => {
                Some(a.distance(b))
            }
            _ => None,
        })
        .collect();
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert!(marks.iter().all(|l| (l - 14.0).abs() < EPS));
}

#[test]
fn a_cylinder_seen_from_the_front_has_two_silhouettes_and_a_centre_line() {
    let b = cylinder();
    let g = project(&[&b], ViewDirection::Front).unwrap();
    // A 10 x 10 rectangle: silhouettes at x = ±5 (z 0..10), the end circles edge-on as
    // the lines z = 0 and z = 10 from x = -5 to 5. The seam is not an edge on paper.
    let vis = lines(&g, false);
    assert_eq!(vis.len(), 4, "{vis:?}");
    assert!(lines(&g, true).is_empty());
    let (lo, hi) = g.bounds().unwrap();
    assert!((lo.x + 5.0).abs() < 1e-4 && (hi.x - 5.0).abs() < 1e-4);
    assert!((lo.y).abs() < 1e-4 && (hi.y - 10.0).abs() < 1e-4);
    assert_eq!(g.centre_lines.len(), 1);
}

#[test]
fn projected_views_follow_third_and_first_angle() {
    let b = block();
    let views = vec![
        view(2, ViewDirection::Front, 150.0, 150.0, None),
        view(3, ViewDirection::Top, 140.0, 220.0, Some(2)),
        view(4, ViewDirection::Right, 250.0, 160.0, Some(2)),
    ];
    let mut s = sheet_with(views, vec![]);
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    // Third angle: the top view above the front view, aligned in x; the right view to
    // its right, aligned in y. Distances kept: 70 up, 100 right.
    let top = out.view(3).unwrap().centre;
    let right = out.view(4).unwrap().centre;
    assert!((top.x - 150.0).abs() < EPS && (top.y - 220.0).abs() < EPS);
    assert!((right.x - 250.0).abs() < EPS && (right.y - 150.0).abs() < EPS);
    // First angle swaps the sides.
    s.projection = ProjectionAngle::First;
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let top = out.view(3).unwrap().centre;
    let right = out.view(4).unwrap().centre;
    assert!((top.x - 150.0).abs() < EPS && (top.y - 80.0).abs() < EPS);
    assert!((right.x - 50.0).abs() < EPS && (right.y - 150.0).abs() < EPS);
    // The box's 50 mm along X lines up in the front and top views.
    let f = out.view(2).unwrap().bounds().unwrap();
    let t = out.view(3).unwrap().bounds().unwrap();
    assert!((f.0.x - t.0.x).abs() < EPS && (f.1.x - t.1.x).abs() < EPS);
    assert!(projects_from(ViewDirection::Front, ViewDirection::Top));
    assert!(!projects_from(ViewDirection::Front, ViewDirection::Back));
    assert!(!projects_from(ViewDirection::Front, ViewDirection::Iso));
}

#[test]
fn a_linear_dimension_across_the_box_measures_50() {
    let b = block();
    let s = sheet_with(
        vec![view(2, ViewDirection::Front, 150.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let v = out.view(2).unwrap();
    // The front view's lower corners are at (125, 145) and (175, 145) on the sheet;
    // picks within the snap distance find them.
    let plan = plan_dimension(
        v,
        DimensionKind::Horizontal,
        &[Point2::new(125.8, 145.5), Point2::new(174.6, 144.2)],
    )
    .unwrap();
    assert!((plan.value - 50.000).abs() < 1e-9, "{}", plan.value);
    assert_eq!(format_value(plan.kind, plan.value), "50");
    let vert = plan_dimension(
        v,
        DimensionKind::Vertical,
        &[Point2::new(175.0, 145.0), Point2::new(175.0, 155.0)],
    )
    .unwrap();
    assert!((vert.value - 10.0).abs() < 1e-9);
    // Drawn: two extension lines, a dimension line, two arrows and "50".
    let d = SheetDimension {
        id: 3,
        view: 2,
        kind: plan.kind,
        points: plan.points,
        offset: -10.0,
        refs: Vec::new(),
    };
    let s = sheet_with(s.views.clone(), vec![d]);
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let dim: Vec<&Item> = out.items.iter().filter(|i| i.layer == Layer::Dim).collect();
    let count = |f: fn(&Shape) -> bool| dim.iter().filter(|i| f(&i.shape)).count();
    assert_eq!(count(|s| matches!(s, Shape::Line { .. })), 3);
    assert_eq!(count(|s| matches!(s, Shape::Fill { .. })), 2);
    assert!(
        dim.iter()
            .any(|i| matches!(&i.shape, Shape::Text { text, .. } if text == "50"))
    );
    // The dimension line is 10 mm below the box's bottom (y 145): y = 135.
    assert!(dim.iter().any(|i| matches!(i.shape, Shape::Line { a, b }
        if (a.y - 135.0).abs() < EPS && (b.y - 135.0).abs() < EPS && (a.distance(b) - 50.0).abs() < EPS)));
}

#[test]
fn diameter_radius_and_angle_dimensions_measure_the_model() {
    let b = plate_with_hole();
    let s = sheet_with(
        vec![view(2, ViewDirection::Top, 200.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let v = out.view(2).unwrap();
    // The hole (r 3) is at the model centre (25, 15), which sits at (200, 150).
    let d = plan_dimension(v, DimensionKind::Diameter, &[Point2::new(203.5, 150.2)]).unwrap();
    assert!((d.value - 6.0).abs() < 1e-9);
    assert_eq!(format_value(d.kind, d.value), "Ø6");
    let r = plan_dimension(v, DimensionKind::Radius, &[Point2::new(200.0, 152.5)]).unwrap();
    assert!((r.value - 3.0).abs() < 1e-9);
    // Two sides of the plate's outline meet at a right angle at the corner (175, 135).
    let a = plan_dimension(
        v,
        DimensionKind::Angular,
        &[Point2::new(190.0, 135.2), Point2::new(175.3, 140.0)],
    )
    .unwrap();
    assert!((a.value - 90.0).abs() < 1e-9, "{}", a.value);
    assert_eq!(format_value(a.kind, a.value), "90°");
    // Nothing to snap to far from the view.
    assert!(
        plan_dimension(
            v,
            DimensionKind::Horizontal,
            &[Point2::new(5.0, 5.0), Point2::new(6.0, 6.0)]
        )
        .is_err()
    );
}

/// A tiny DXF reader for tests: the ENTITIES section as (type, layer) pairs.
pub(crate) fn dxf_entities(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs = Vec::new();
    for p in lines.chunks(2) {
        assert_eq!(p.len(), 2, "a group code without a value");
        let code: i32 = p[0].trim().parse().expect("group code");
        pairs.push((code, p[1].to_owned()));
    }
    assert_eq!(pairs.last().map(|p| p.1.as_str()), Some("EOF"));
    let start = pairs
        .iter()
        .position(|(c, v)| *c == 2 && v == "ENTITIES")
        .expect("ENTITIES");
    let mut out: Vec<(String, String)> = Vec::new();
    for (c, v) in &pairs[start + 1..] {
        match c {
            0 if v == "ENDSEC" => break,
            0 => out.push((v.clone(), String::new())),
            8 => {
                if let Some(e) = out.last_mut() {
                    e.1 = v.clone();
                }
            }
            _ => {}
        }
    }
    out
}

/// A tiny SVG reader for tests: how many elements named `tag` each layer group holds.
pub(crate) fn svg_count(text: &str, layer: &str, tag: &str) -> usize {
    let open = format!("<g id=\"{layer}\"");
    let start = text.find(&open).expect("layer group");
    let end = start + text[start..].find("</g>").expect("group end");
    text[start..end].matches(&format!("<{tag} ")).count()
}

#[test]
fn svg_and_dxf_parse_back_with_the_expected_entities() {
    let b = plate_with_hole();
    let s = sheet_with(
        vec![view(2, ViewDirection::Front, 150.0, 150.0, None)],
        vec![],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    // Expected, by layer:
    // - VISIBLE: the front view's rectangle, 4 lines.
    // - HIDDEN: the hole's two silhouettes.
    // - CENTER: the hole's centre line, and the projection symbol's two axes.
    // - BORDER: the frame (4), the title block's box (4), the symbol column's divider
    //   (1), the row divider (1), three cell dividers (3), the symbol's cone (4): 17
    //   lines, and the symbol's two circles.
    let svg = svg::write(&out);
    assert!(svg.starts_with("<?xml"));
    assert!(svg.trim_end().ends_with("</svg>"));
    assert_eq!(svg_count(&svg, "VISIBLE", "line"), 4);
    assert_eq!(svg_count(&svg, "HIDDEN", "line"), 2);
    assert!(svg.contains(r#"<g id="HIDDEN" fill="none" stroke="black" stroke-width="0.25" stroke-linecap="round" stroke-dasharray="3 1.5">"#));
    assert_eq!(svg_count(&svg, "CENTER", "line"), 3);
    assert_eq!(svg_count(&svg, "BORDER", "line"), 17);
    assert_eq!(svg_count(&svg, "BORDER", "circle"), 2);
    // 9 texts in the title block: PART, CHAMFERA, the name, 4 labels... and 4 values,
    // and the projection's name: 3 + 8 + 1 = 12.
    assert_eq!(svg_count(&svg, "BORDER", "text"), 12);

    let dxf = dxf::write(&out);
    assert!(dxf.contains("$ACADVER\n  1\nAC1009"));
    let e = dxf_entities(&dxf);
    let n = |t: &str, l: &str| e.iter().filter(|(a, b)| a == t && b == l).count();
    assert_eq!(n("LINE", "VISIBLE"), 4);
    assert_eq!(n("LINE", "HIDDEN"), 2);
    assert_eq!(n("LINE", "CENTER"), 3);
    assert_eq!(n("LINE", "BORDER"), 17);
    assert_eq!(n("CIRCLE", "BORDER"), 2);
    assert_eq!(n("TEXT", "BORDER"), 12);
    // Every layer is declared with its line type.
    for (layer, lt) in [
        ("HIDDEN", "DASHED"),
        ("CENTER", "CENTER"),
        ("VISIBLE", "CONTINUOUS"),
    ] {
        assert!(dxf.contains(&format!("  0\nLAYER\n  2\n{layer}\n 70\n0\n 62\n")));
        assert!(dxf.contains(&format!("\n  6\n{lt}\n")));
    }
    // The top view's hole is a real CIRCLE, with a centre mark of two lines.
    let s = sheet_with(
        vec![view(2, ViewDirection::Top, 150.0, 150.0, None)],
        vec![],
    );
    let e = dxf_entities(&dxf::write(&render_sheet(&s, &[("Body1", &b)]).unwrap()));
    assert_eq!(
        e.iter()
            .filter(|(a, b)| a == "CIRCLE" && b == "VISIBLE")
            .count(),
        1
    );
    assert_eq!(
        e.iter()
            .filter(|(a, b)| a == "LINE" && b == "CENTER")
            .count(),
        2 + 2
    );
}

#[test]
fn the_pdf_has_a_valid_header_xref_and_object_offsets() {
    let b = plate_with_hole();
    let d = SheetDimension {
        id: 3,
        view: 2,
        kind: DimensionKind::Diameter,
        points: vec![Point2::new(25.0, 15.0), Point2::new(28.0, 15.0)],
        offset: 8.0,
        refs: Vec::new(),
    };
    let s = sheet_with(
        vec![view(2, ViewDirection::Top, 150.0, 150.0, None)],
        vec![d],
    );
    let out = render_sheet(&s, &[("Body1", &b)]).unwrap();
    let pdf = pdf::write(&out);
    assert!(pdf.starts_with(b"%PDF-1.4\n"));
    assert!(pdf.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(&pdf).into_owned();
    // startxref gives the byte offset of the xref table.
    let sx = text.rfind("startxref\n").unwrap();
    let xref: usize = text[sx + 10..].lines().next().unwrap().parse().unwrap();
    assert!(pdf[xref..].starts_with(b"xref\n0 6\n"));
    // Entries are 20 bytes each: object 0 free, then objects 1..5 in use, each offset
    // pointing at "n 0 obj".
    let entries = xref + "xref\n0 6\n".len();
    for i in 0..6 {
        let e = &pdf[entries + 20 * i..entries + 20 * (i + 1)];
        let e = std::str::from_utf8(e).unwrap();
        assert!(e.ends_with("\r\n"), "{e:?}");
        if i == 0 {
            assert_eq!(e, "0000000000 65535 f\r\n");
            continue;
        }
        assert!(e.ends_with(" 00000 n\r\n"));
        let off: usize = e[..10].parse().unwrap();
        assert!(
            pdf[off..].starts_with(format!("{i} 0 obj\n").as_bytes()),
            "object {i}"
        );
    }
    // The stream's /Length is the bytes between "stream\n" and "endstream".
    let l = text.find("/Length ").unwrap();
    let len: usize = text[l + 8..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let st = pdf.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
    assert_eq!(&pdf[st + len..st + len + 9], b"endstream");
    // A3 landscape: 420 x 297 mm = 1190.5512 x 841.8898 pt.
    assert!(text.contains("/MediaBox [0 0 1190.5512 841.8898]"));
    assert!(text.contains("/BaseFont /Helvetica"));
    // Ø6 in WinAnsiEncoding.
    assert!(text.contains("(\\3306) Tj"));
}

#[test]
fn hostile_input_never_panics() {
    let b = plate_with_hole();
    let bodies = [("Body1", &b)];
    for x in [f64::NAN, f64::INFINITY, -1e300, 1e300, 0.0] {
        let mut v = view(2, ViewDirection::Iso, x, x, None);
        v.scale = Some(x);
        let mut c = view(3, ViewDirection::Right, x, -x, Some(2));
        c.body = Some("nope".into());
        let dims = vec![
            SheetDimension {
                id: 4,
                view: 2,
                kind: DimensionKind::Angular,
                points: vec![Point2::new(x, x)],
                offset: x,
                refs: Vec::new(),
            },
            SheetDimension {
                id: 5,
                view: 3,
                kind: DimensionKind::Aligned,
                points: vec![Point2::new(x, 0.0), Point2::new(x, 0.0)],
                offset: x,
                refs: Vec::new(),
            },
            SheetDimension {
                id: 6,
                view: 99,
                kind: DimensionKind::Diameter,
                points: vec![],
                offset: x,
                refs: Vec::new(),
            },
        ];
        let mut s = sheet_with(vec![v, c], dims);
        s.scale = x;
        s.title.part_name = "\u{0}<&>\"(\\)%Ø°é\n".repeat(20);
        let out = render_sheet(&s, &bodies).unwrap();
        let _ = svg::write(&out);
        let _ = dxf::write(&out);
        let _ = pdf::write(&out);
        for kind in DimensionKind::ALL {
            for v in &out.views {
                let _ = plan_dimension(v, kind, &[Point2::new(x, x), Point2::new(-x, x)]);
                let _ = plan_dimension(v, kind, &[]);
            }
            let _ = format_value(kind, x);
            let _ = measure(kind, &[Point2::new(x, x)]);
        }
    }
    // A missing body leaves the view empty with a note, and the sheet still renders.
    let mut v = view(2, ViewDirection::Front, 100.0, 100.0, None);
    v.body = Some("Gone".into());
    let out = render_sheet(&sheet_with(vec![v], vec![]), &bodies).unwrap();
    assert!(out.views[0].problem.is_some());
    assert!(
        project(&[], ViewDirection::Front)
            .unwrap()
            .curves
            .is_empty()
    );
}

#[test]
fn scales_read_as_ratios() {
    assert_eq!(scale_text(1.0), "1:1");
    assert_eq!(scale_text(0.5), "1:2");
    assert_eq!(scale_text(2.0), "2:1");
    assert_eq!(scale_text(0.4), "1:2.5");
}

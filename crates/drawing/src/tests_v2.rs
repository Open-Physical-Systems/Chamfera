//! Tests of drawings version 2: silhouettes of cones, spheres and tori, sections,
//! details, annotations, the parts list and the writers. Expected values are derived by
//! hand in each test's comments.

use crate::tests::{cylinder, dxf_entities, lines, plate_with_hole, sheet_with, svg_count, view};
use crate::*;
use chamfera_doc::drawing::{
    Annotation, AnnotationKind, DetailCircle, GdtCharacteristic, PartsList, SectionCut,
};
use chamfera_geom::{Frame, Point2, Vec2};
use chamfera_kernel::{Body, Line2, Profile, ProfileEdge, revolve};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const EPS: f64 = 1e-6;

/// Revolves a profile drawn in the XZ plane (x = X, y = Z) a whole turn about Z.
fn about_z(p: &Profile) -> Body {
    let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
    revolve(&Frame::XZ, p, &axis, TAU).unwrap()
}

/// A cone: base radius 10 on z = 0, apex at (0, 0, 20).
fn cone() -> Body {
    about_z(&Profile::polygon(&[
        Point2::ORIGIN,
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 20.0),
    ]))
}

/// A sphere of radius 10 about the origin.
fn sphere() -> Body {
    let r = 10.0;
    about_z(&Profile::new(
        vec![
            ProfileEdge::Arc {
                centre: Point2::ORIGIN,
                radius: r,
                start_angle: -FRAC_PI_2,
                sweep: PI,
            },
            ProfileEdge::line(Point2::new(0.0, r), Point2::new(0.0, -r)),
        ],
        vec![],
    ))
}

/// A torus about Z: major radius 20, minor radius 5, centred on the origin.
fn torus() -> Body {
    about_z(&Profile::circle(Point2::new(20.0, 0.0), 5.0))
}

fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) < 1e-6
}

#[test]
fn the_side_view_of_a_cone_shows_two_outline_lines_meeting_at_the_apex() {
    let b = cone();
    let g = project(&[&b], ViewDirection::Front).unwrap();
    // Seen from -Y (x right, z up), the cone's contour generators are where its normal
    // is perpendicular to d = -Y: cos(u - φ) = tan α (z · d) / |d_xy| = 0, so u = 0 and
    // u = π, the generators through (±10, 0, 0). They meet the apex (0, 0, 20): lines
    // (-10, 0)-(0, 20) and (10, 0)-(0, 20). The base circle is edge-on: the line z = 0
    // from x = -10 to 10. Nothing is hidden.
    let vis = lines(&g, false);
    assert!(lines(&g, true).is_empty(), "{:?}", lines(&g, true));
    let apex = Point2::new(0.0, 20.0);
    for foot in [Point2::new(-10.0, 0.0), Point2::new(10.0, 0.0)] {
        assert!(
            vis.iter()
                .any(|(a, c)| (near(*a, apex) && near(*c, foot))
                    || (near(*c, apex) && near(*a, foot))),
            "no outline from {foot:?} to the apex: {vis:?}"
        );
    }
    assert!(
        vis.iter()
            .any(|(a, c)| (a.y.abs() < EPS && c.y.abs() < EPS)
                && (a.distance(*c) - 20.0).abs() < EPS),
        "the base edge-on: {vis:?}"
    );
    assert_eq!(vis.len(), 3, "{vis:?}");
    // The axis is a centre line from the base (z = 0) to the apex (z = 20) at x = 0.
    assert_eq!(g.centre_lines.len(), 1);
    let [p, q] = g.centre_lines[0];
    assert!(p.x.abs() < EPS && q.x.abs() < EPS && ((p.y - q.y).abs() - 20.0).abs() < EPS);
    // Looking down the axis there is no generator outline: only the base circle.
    let top = project(&[&b], ViewDirection::Top).unwrap();
    assert!(lines(&top, false).is_empty());
}

#[test]
fn a_sphere_is_a_circle_and_a_torus_has_its_outline_from_every_side() {
    let s = sphere();
    let g = project(&[&s], ViewDirection::Front).unwrap();
    // The great circle perpendicular to the view: radius 10 about the origin, seen
    // end-on, whole and visible.
    let swept: f64 = g
        .curves
        .iter()
        .filter(|c| !c.hidden)
        .map(|c| match c.curve {
            Curve2::Circle { centre, radius } if near(centre, Point2::ORIGIN) => {
                assert!((radius - 10.0).abs() < EPS);
                TAU
            }
            Curve2::Arc {
                centre,
                radius,
                start,
                end,
            } if near(centre, Point2::ORIGIN) && (radius - 10.0).abs() < EPS => end - start,
            _ => 0.0,
        })
        .sum();
    assert!((swept - TAU).abs() < 1e-6, "{:?}", g.curves);
    let (lo, hi) = g.bounds().unwrap();
    assert!(near(lo, Point2::new(-10.0, -10.0)) && near(hi, Point2::new(10.0, 10.0)));

    let t = torus();
    // From above (along the axis): the circles R + a = 25 and R - a = 15.
    let top = project(&[&t], ViewDirection::Top).unwrap();
    let mut radii: Vec<f64> = top
        .curves
        .iter()
        .filter(|c| !c.hidden)
        .filter_map(|c| match c.curve {
            Curve2::Circle { radius, .. } | Curve2::Arc { radius, .. } => Some(radius),
            Curve2::Line { .. } => None,
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    radii.dedup_by(|a, b| (*a - *b).abs() < EPS);
    assert_eq!(radii.len(), 2, "{radii:?}");
    assert!((radii[0] - 15.0).abs() < EPS && (radii[1] - 25.0).abs() < EPS);
    // From the front (side-on): x from -(R + a) = -25 to 25, z from -a = -5 to 5; the
    // top circle z = 5 of radius R = 20 seen edge-on is the line from x = -20 to 20.
    let front = project(&[&t], ViewDirection::Front).unwrap();
    let (lo, hi) = front.bounds().unwrap();
    assert!(
        near(lo, Point2::new(-25.0, -5.0)) && near(hi, Point2::new(25.0, 5.0)),
        "{lo:?} {hi:?}"
    );
    let vis = lines(&front, false);
    let along: f64 = vis
        .iter()
        .filter(|(a, b)| (a.y - 5.0).abs() < 1e-6 && (b.y - 5.0).abs() < 1e-6)
        .map(|(a, b)| a.distance(*b))
        .sum();
    assert!((along - 40.0).abs() < 1e-4, "{vis:?}");
    // Iso: the sampled branches. The torus fits in a sphere of radius 25 about the
    // origin, so every outline point is within 25 of the projected centre, and the
    // outermost points (the outer equator's rim) reach 25 along the paper's x axis.
    let iso = project(&[&t], ViewDirection::Iso).unwrap();
    let (lo, hi) = iso.bounds().unwrap();
    assert!(lo.x > -25.0 - 1e-3 && hi.x < 25.0 + 1e-3);
    assert!(
        (hi.x - 25.0).abs() < 0.05 && (lo.x + 25.0).abs() < 0.05,
        "{lo:?} {hi:?}"
    );
}

fn section_sheet() -> Sheet {
    let mut top = view(2, ViewDirection::Top, 120.0, 200.0, None);
    top.scale = Some(1.0);
    // The cutting line across the top view at y = 15 (through the hole's axis), drawn
    // from right to left so the arrows point up the paper: the section looks along +Y,
    // as the front view does, and keeps the material at y > 15.
    let mut sec = view(3, ViewDirection::Front, 120.0, 100.0, None);
    sec.section = Some(SectionCut {
        from: 2,
        a: Point2::new(55.0, 15.0),
        b: Point2::new(-5.0, 15.0),
        label: "A".into(),
        flip: false,
    });
    let mut s = sheet_with(vec![top, sec], vec![]);
    s.annotations = vec![];
    s
}

#[test]
fn a_section_through_the_hole_plate_hatches_440_square_mm() {
    let b = plate_with_hole();
    let out = render_sheet(&section_sheet(), &[("Body1", &b)]).unwrap();
    let sec = out.view(3).unwrap();
    assert_eq!(sec.problem, None);
    // The plane y = 15 meets the 50 x 10 plate in a 50 x 10 rectangle; the hole of
    // diameter 6 about x = 25 takes the strip 22 < x < 28 out of it. Two rectangles of
    // 22 x 10: 440 mm².
    let area: f64 = sec
        .geometry
        .regions
        .iter()
        .map(|r| section::region_area(r))
        .sum();
    assert!((area - 440.0).abs() < 1e-6, "{area}");
    // Seen like the front view: x right and z up, 50 wide and 10 high, no hidden lines.
    let (lo, hi) = sec.bounds().unwrap();
    assert!((hi.x - lo.x - 50.0).abs() < 1e-6 && (hi.y - lo.y - 10.0).abs() < 1e-6);
    assert!(sec.geometry.curves.iter().all(|c| !c.hidden));
    // Hatch lines at 45°, 2.5 mm apart at 1:1, inside the section's box. Their total
    // length is about area / spacing = 440 / 2.5 = 176 mm (less a little where lines
    // meet the boundary at an angle).
    let hatch: Vec<(Point2, Point2)> = out
        .items
        .iter()
        .filter(|i| i.layer == Layer::Hatch)
        .filter_map(|i| match i.shape {
            Shape::Line { a, b } => Some((a, b)),
            _ => None,
        })
        .collect();
    assert!(!hatch.is_empty());
    let mut total = 0.0;
    for (a, b) in &hatch {
        let d = *b - *a;
        assert!((d.y.atan2(d.x) - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
        for p in [a, b] {
            assert!(p.x >= lo.x - 1e-6 && p.x <= hi.x + 1e-6);
            assert!(p.y >= lo.y - 1e-6 && p.y <= hi.y + 1e-6);
            // Never across the hole: x = 120 - 25 + 22 .. + 28 on the sheet.
            assert!(!(p.x > 117.0 + 1e-6 && p.x < 123.0 - 1e-6), "{p:?}");
        }
        total += d.length();
    }
    assert!((total - 176.0).abs() < 12.0, "{total}");
    // The cutting-plane line on the top view and the label below the section.
    let texts: Vec<&str> = out
        .items
        .iter()
        .filter_map(|i| match &i.shape {
            Shape::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"SECTION A-A"));
    assert_eq!(texts.iter().filter(|t| **t == "A").count(), 2);
    assert!(out.items.iter().any(|i| i.layer == Layer::Center
        && matches!(i.shape, Shape::Line { a, b } if (a.y - 200.0).abs() < 1e-6 && (b.y - 200.0).abs() < 1e-6)));
}

#[test]
fn a_2_to_1_detail_of_a_10_mm_circle_draws_it_20_mm_across() {
    let b = cylinder();
    let top = view(2, ViewDirection::Top, 100.0, 150.0, None);
    let mut det = view(3, ViewDirection::Top, 250.0, 150.0, None);
    det.scale = Some(2.0);
    det.detail = Some(DetailCircle {
        from: 2,
        centre: Point2::new(0.0, 0.0),
        radius: 8.0,
        label: "B".into(),
    });
    let out = render_sheet(&sheet_with(vec![top, det], vec![]), &[("Body1", &b)]).unwrap();
    // The cylinder's Ø10 circle at 2:1 is a circle of radius 10 (Ø20) about the detail's
    // centre (250, 150); the detail's boundary, radius 8 at 2:1, is 16 there, and 8 on
    // the parent about (100, 150).
    let circles: Vec<(Layer, Point2, f64, f64)> = out
        .items
        .iter()
        .filter_map(|i| match i.shape {
            Shape::Circle { centre, radius } => Some((i.layer, centre, radius, TAU)),
            Shape::Arc {
                centre,
                radius,
                start,
                end,
            } => Some((i.layer, centre, radius, end - start)),
            _ => None,
        })
        .collect();
    let has = |layer: Layer, c: Point2, r: f64| {
        let sweep: f64 = circles
            .iter()
            .filter(|x| x.0 == layer && near(x.1, c) && (x.2 - r).abs() < EPS)
            .map(|x| x.3)
            .sum();
        (sweep - TAU).abs() < 1e-9
    };
    assert!(
        has(Layer::Visible, Point2::new(250.0, 150.0), 10.0),
        "{circles:?}"
    );
    assert!(has(Layer::Dim, Point2::new(250.0, 150.0), 16.0));
    assert!(has(Layer::Dim, Point2::new(100.0, 150.0), 8.0));
    let detail = out.view(3).unwrap();
    assert_eq!(detail.scale, 2.0);
    assert!(
        out.items
            .iter()
            .any(|i| matches!(&i.shape, Shape::Text { text, .. } if text == "DETAIL B (2:1)"))
    );
    // A detail clips: one of radius 3 shows only the centre mark's middle, 2 x 3 x 2 =
    // 12 mm long on paper, not the circle (radius 5 > 3).
    let mut small = view(4, ViewDirection::Top, 250.0, 60.0, None);
    small.scale = Some(2.0);
    small.detail = Some(DetailCircle {
        from: 2,
        centre: Point2::new(0.0, 0.0),
        radius: 3.0,
        label: "C".into(),
    });
    let out = render_sheet(
        &sheet_with(
            vec![view(2, ViewDirection::Top, 100.0, 150.0, None), small],
            vec![],
        ),
        &[("Body1", &b)],
    )
    .unwrap();
    let marks: Vec<f64> = out
        .items
        .iter()
        .filter(|i| i.layer == Layer::Center)
        .filter_map(|i| match i.shape {
            Shape::Line { a, b } if a.distance(Point2::new(250.0, 60.0)) < 7.0 => {
                Some(a.distance(b))
            }
            _ => None,
        })
        .collect();
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert!(marks.iter().all(|l| (l - 12.0).abs() < 1e-9));
    assert!(!out.items.iter().any(|i| i.layer == Layer::Visible
        && matches!(i.shape, Shape::Circle { centre, .. } | Shape::Arc { centre, .. } if centre.distance(Point2::new(250.0, 60.0)) < 1.0)));
}

fn annotated_sheet() -> Sheet {
    let mut s = section_sheet();
    let mut det = view(4, ViewDirection::Top, 330.0, 200.0, None);
    det.scale = Some(2.0);
    det.detail = Some(DetailCircle {
        from: 2,
        centre: Point2::new(25.0, 15.0),
        radius: 6.0,
        label: "B".into(),
    });
    s.views.push(det);
    let at = |x: f64, y: f64| Point2::new(x, y);
    let kinds = vec![
        AnnotationKind::Note {
            at: at(30.0, 60.0),
            text: "BREAK SHARP EDGES".into(),
            height: 3.5,
        },
        AnnotationKind::Leader {
            points: vec![at(120.0, 215.0), at(150.0, 240.0)],
            text: "Ø6 THRU".into(),
        },
        AnnotationKind::SurfaceFinish {
            at: at(120.0, 205.0),
            text: "Ra 1.6".into(),
            removal: true,
        },
        AnnotationKind::Gdt {
            at: at(160.0, 90.0),
            characteristic: GdtCharacteristic::Perpendicularity,
            tolerance: 0.05,
            diameter: false,
            datums: vec!["A".into()],
            leader: Some(at(145.0, 95.0)),
        },
        AnnotationKind::Gdt {
            at: at(160.0, 75.0),
            characteristic: GdtCharacteristic::Position,
            tolerance: 0.1,
            diameter: true,
            datums: vec!["A".into(), "B".into(), "C".into()],
            leader: None,
        },
        AnnotationKind::Gdt {
            at: at(160.0, 60.0),
            characteristic: GdtCharacteristic::Flatness,
            tolerance: 0.02,
            diameter: false,
            datums: vec![],
            leader: None,
        },
        AnnotationKind::Balloon {
            view: 2,
            component: 7,
            at: at(60.0, 240.0),
            target: Point2::new(10.0, 10.0),
        },
    ];
    s.annotations = kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| Annotation {
            id: 10 + i as u32,
            kind,
        })
        .collect();
    s.parts_list = Some(PartsList {
        id: 30,
        x: 410.0,
        y: 40.0,
    });
    s
}

fn parts() -> Vec<PartRow> {
    vec![
        PartRow {
            components: vec![7],
            name: "Plate".into(),
            quantity: 1,
            material: "steel".into(),
        },
        PartRow {
            components: vec![8, 9],
            name: "Pin".into(),
            quantity: 2,
            material: String::new(),
        },
    ]
}

#[test]
fn frames_symbols_balloons_and_the_parts_list_are_drawn_and_exported() {
    let b = plate_with_hole();
    let out = render_with_parts(&annotated_sheet(), &[("Body1", &b)], &parts()).unwrap();
    let texts: Vec<String> = out
        .items
        .iter()
        .filter_map(|i| match &i.shape {
            Shape::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    for t in [
        "BREAK SHARP EDGES",
        "Ø6 THRU",
        "Ra 1.6",
        "0.05",
        "Ø0.1",
        "0.02",
        "ITEM",
        "QTY",
        "Plate",
        "Pin",
        "steel",
        "2",
        "DETAIL B (2:1)",
    ] {
        assert!(texts.iter().any(|x| x == t), "{t} missing from {texts:?}");
    }
    // The position frame: symbol and tolerance cells, then three datum cells of 7 mm.
    // Its box spans 7 + max(7, width("Ø0.1") + 3) + 3 x 7.
    let tol_w = (text_width("Ø0.1", annot::TEXT) + 3.0).max(7.0);
    let frame_w = 7.0 + tol_w + 21.0;
    assert!(out.items.iter().any(|i| matches!(i.shape,
        Shape::Line { a, b } if (a.y - 75.0).abs() < EPS && (b.y - 75.0).abs() < EPS
            && ((b.x - a.x).abs() - frame_w).abs() < 1e-9)));
    // The balloon of component 7 says 1 (its row), with a circle of radius 4.5 at
    // (60, 240).
    assert!(out.items.iter().any(|i| matches!(i.shape,
        Shape::Circle { centre, radius } if near(centre, Point2::new(60.0, 240.0)) && (radius - 4.5).abs() < EPS)));
    assert!(out.dangling_annotations.is_empty());
    // The parts list: a header and two rows of 7 mm, 124 wide, its lower-right corner
    // at (410, 40): its top line is at 40 + 3 x 7 = 61.
    let top_line = out.items.iter().any(|i| i.layer == Layer::Border
        && matches!(i.shape, Shape::Line { a, b } if (a.y - 61.0).abs() < EPS && (b.y - 61.0).abs() < EPS
            && ((a.x - b.x).abs() - 124.0).abs() < EPS));
    assert!(top_line);
    assert_eq!(out.parts.len(), 2);
    // The exports carry the new layers and entities.
    let svg = svg::write(&out);
    assert!(svg_count(&svg, "HATCH", "line") > 10);
    assert!(svg.contains("SECTION A-A") && svg.contains("Ra 1.6") && svg.contains("ITEM"));
    let dxf = dxf::write(&out);
    let ents = dxf_entities(&dxf);
    let hatch = ents
        .iter()
        .filter(|e| e.1 == "HATCH" && e.0 == "LINE")
        .count();
    assert_eq!(
        hatch,
        out.items.iter().filter(|i| i.layer == Layer::Hatch).count()
    );
    assert!(ents.iter().any(|e| e.0 == "TEXT" && e.1 == "DIM"));
    assert!(dxf.contains("\n%%c0.1\n"), "Ø as %%c in DXF");
    assert!(dxf.contains("\nSECTION A-A\n"));
    assert!(dxf.contains("\nDANGLING\n") && dxf.contains("\nHATCH\n"));
    let pdf = pdf::write(&out);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("(SECTION A-A) Tj") && text.contains("(\\3300.1) Tj"));
    assert!(text.contains("(Plate) Tj"));
}

#[test]
fn a_balloon_for_a_missing_component_is_dangling_not_dropped() {
    let b = plate_with_hole();
    let out = render_with_parts(&annotated_sheet(), &[("Body1", &b)], &[]).unwrap();
    // No parts: the balloon of component 7 shows "?" on the dangling layer.
    assert_eq!(out.dangling_annotations.len(), 1);
    assert!(out.items.iter().any(|i| i.layer == Layer::Dangling
        && matches!(&i.shape, Shape::Text { text, .. } if text == "?")));
}

#[test]
fn hostile_sections_details_and_annotations_never_panic() {
    let b = plate_with_hole();
    let bodies = [("Body1", &b)];
    for x in [f64::NAN, f64::INFINITY, -1e300, 1e300, 0.0, 1e-300] {
        let mut s = annotated_sheet();
        if let Some(c) = s.views[1].section.as_mut() {
            c.a = Point2::new(x, x);
            c.b = Point2::new(-x, 15.0);
        }
        if let Some(d) = s.views[2].detail.as_mut() {
            d.radius = x;
            d.centre = Point2::new(x, -x);
        }
        s.views[2].scale = Some(x);
        for a in &mut s.annotations {
            match &mut a.kind {
                AnnotationKind::Note { at, height, .. } => {
                    *at = Point2::new(x, x);
                    *height = x;
                }
                AnnotationKind::Gdt { tolerance, at, .. } => {
                    *tolerance = x;
                    *at = Point2::new(x, 0.0);
                }
                AnnotationKind::Balloon { target, .. } => *target = Point2::new(x, x),
                AnnotationKind::Leader { points, .. } => points.push(Point2::new(x, x)),
                _ => {}
            }
        }
        s.parts_list = Some(PartsList { id: 30, x, y: x });
        let out = render_with_parts(&s, &bodies, &parts()).unwrap();
        let _ = svg::write(&out);
        let _ = dxf::write(&out);
        let _ = pdf::write(&out);
        let _ = section::hatch(&[vec![Point2::new(x, 0.0), Point2::new(0.0, x)]], x, x);
        let _ = annot::clip_arc(Point2::new(x, x), x, 0.0, x, Point2::ORIGIN, x);
        let _ = annot::clip_line(Point2::new(x, x), Point2::ORIGIN, Point2::ORIGIN, x);
    }
    // A cutting line that misses the part keeps nothing in front: the view says so.
    let mut s = section_sheet();
    if let Some(c) = s.views[1].section.as_mut() {
        c.a = Point2::new(55.0, 40.0);
        c.b = Point2::new(-5.0, 40.0);
    }
    let out = render_sheet(&s, &bodies).unwrap();
    assert!(out.view(3).unwrap().problem.is_some());
}

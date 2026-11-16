//! Sketch gaps: SVG import, intersection curves, copy and paste, control-point splines
//! and spline end tangents, polygons by an edge and arc slots. Expected values are
//! worked out by hand in the comments.

use crate::Session;
use chamfera_sketch::{Curve, CurveId};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn sketching() -> Session {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s
}

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: got {a}, expected {b}");
}

/// Writes `svg` to a file of its own and imports it with `extra` parameters.
fn import_svg(s: &mut Session, name: &str, svg: &str, extra: Value) -> Value {
    let dir = std::env::temp_dir().join(format!("chamfera-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, svg).unwrap();
    let mut p = json!({ "path": path.to_string_lossy() });
    if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    let r = s.execute("sketch.import_svg", &p);
    let _ = std::fs::remove_file(&path);
    r.unwrap()
}

fn active(s: &Session) -> &chamfera_sketch::Sketch {
    &s.active_sketch().unwrap().sketch
}

fn curves_of<'a>(s: &'a Session, pick: impl Fn(&Curve) -> bool + 'a) -> Vec<(CurveId, Curve)> {
    active(s)
        .curves()
        .filter(|(_, c)| pick(c))
        .map(|(id, c)| (id, c.clone()))
        .collect()
}

// ----- SVG -----

#[test]
fn an_svg_arc_lands_as_an_arc_with_its_hand_length() {
    // M10 0 A10 10 0 0 1 0 10, one user unit a mm: the quarter circle about (0, 0)
    // (SVG 1.1 F.6.5), y turned up: from (0, -10) to (10, 0), length 10 · π/2.
    let mut s = sketching();
    let r = import_svg(
        &mut s,
        "arc.svg",
        r#"<svg><path d="M10 0 A10 10 0 0 1 0 10"/></svg>"#,
        json!({"units": "mm"}),
    );
    assert_eq!(r["curves"], 1, "{r}");
    let sk = active(&s);
    let arcs = curves_of(&s, |c| matches!(c, Curve::Arc { .. }));
    let Some((
        _,
        Curve::Arc {
            centre, start, end, ..
        },
    )) = arcs.first()
    else {
        panic!("{r}")
    };
    let (c, a, b) = (
        sk.point(*centre).unwrap(),
        sk.point(*start).unwrap(),
        sk.point(*end).unwrap(),
    );
    close(
        c.distance(chamfera_geom::Point2::new(0.0, 0.0)),
        0.0,
        1e-9,
        "centre",
    );
    close(a.x, 0.0, 1e-9, "start x");
    close(a.y, -10.0, 1e-9, "start y");
    close(b.x, 10.0, 1e-9, "end x");
    close(b.y, 0.0, 1e-9, "end y");
    let sweep = ((b - c).angle() - (a - c).angle()).rem_euclid(2.0 * PI);
    close(10.0 * sweep, 5.0 * PI, 1e-9, "arc length");
}

#[test]
fn an_svg_quadratic_is_an_exact_cubic_spline_with_the_parabolas_length() {
    // Q from (0, 0) over (10, 20) to (20, 0): B(t) = (20 t, 40 t (1 - t)) (y down), so
    // |B'(t)| = sqrt(400 + (40 - 80 t)²). With u = 40 - 80 t the length is
    // (1/40) ∫₀⁴⁰ sqrt(20² + u²) du = (1/40) [20 √2000 + 200 ln((40 + √2000) / 20)]
    // ≈ 29.5789 mm. Its apex is (10, 10), y turned up (10, -10).
    let mut s = sketching();
    import_svg(
        &mut s,
        "quad.svg",
        r#"<svg><path d="M0 0 Q10 20 20 0"/></svg>"#,
        json!({"units": "mm"}),
    );
    let splines = curves_of(&s, Curve::is_spline);
    assert_eq!(splines.len(), 1);
    let sk = active(&s);
    let curve = sk.spline_curve(splines[0].0).unwrap();
    let r2000 = 2000f64.sqrt();
    let hand = (20.0 * r2000 + 200.0 * ((40.0 + r2000) / 20.0).ln()) / 40.0;
    close(curve.length().unwrap(), hand, 1e-6, "parabola length");
    let (lo, hi) = curve.domain();
    let (a, b) = (curve.point_at(lo), curve.point_at(hi));
    close(a.x.hypot(a.y), 0.0, 1e-9, "start");
    close(b.x, 20.0, 1e-9, "end x");
    close(b.y, 0.0, 1e-9, "end y");
    close(
        curve
            .closest_point(chamfera_geom::Point2::new(10.0, -10.0))
            .distance,
        0.0,
        1e-9,
        "apex",
    );
}

#[test]
fn an_svg_cubic_with_a_control_point_on_its_end_stays_exact() {
    // C with the first control point on the start: (0,0) (0,0) (30,0) (30,0) is the
    // segment from 0 to 30 run at a varying speed; length 30.
    let mut s = sketching();
    import_svg(
        &mut s,
        "cubic.svg",
        r#"<svg><path d="M0 0 C0 0 30 0 30 0"/></svg>"#,
        json!({"units": "mm"}),
    );
    let splines = curves_of(&s, Curve::is_spline);
    let curve = active(&s).spline_curve(splines[0].0).unwrap();
    close(curve.length().unwrap(), 30.0, 1e-6, "length");
}

#[test]
fn an_svg_page_in_millimetres_keeps_its_size_and_the_scale_applies() {
    // A 100 mm page with viewBox 0 0 100 100: a circle r = 20 at (50, 50) has area
    // 400 π mm², centred 50 mm above the page's bottom edge. Scale 2: 1600 π.
    let svg = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><circle cx="50" cy="50" r="20"/></svg>"#;
    let mut s = sketching();
    let r = import_svg(&mut s, "page.svg", svg, json!({}));
    close(r["profiles"][0].as_f64().unwrap(), 400.0 * PI, 1e-6, "area");
    close(
        r["mm_per_unit"].as_f64().unwrap(),
        1.0,
        1e-12,
        "mm per unit",
    );
    let circles = curves_of(&s, |c| matches!(c, Curve::Circle { .. }));
    let Some((_, Curve::Circle { centre, .. })) = circles.first() else {
        panic!()
    };
    let c = active(&s).point(*centre).unwrap();
    close(c.x, 50.0, 1e-9, "x");
    close(c.y, 50.0, 1e-9, "y");
    let mut t = sketching();
    let r = import_svg(&mut t, "page2.svg", svg, json!({"scale": 2}));
    close(
        r["profiles"][0].as_f64().unwrap(),
        1600.0 * PI,
        1e-6,
        "scaled area",
    );
}

#[test]
fn a_closed_svg_outline_of_lines_and_beziers_is_one_profile() {
    // A 40 x 20 rectangle whose top side is a cubic with its controls on the side
    // (straight): area 800, one profile, the joins coincident.
    let mut s = sketching();
    let r = import_svg(
        &mut s,
        "outline.svg",
        r#"<svg><path d="M0 0 H40 V20 C30 20 10 20 0 20 Z"/></svg>"#,
        json!({"units": "mm"}),
    );
    assert_eq!(r["profiles"].as_array().unwrap().len(), 1, "{r}");
    close(r["profiles"][0].as_f64().unwrap(), 800.0, 1e-6, "area");
    assert_eq!(r["joined"], 4, "{r}");
}

#[test]
fn hostile_svg_files_never_panic_and_change_nothing() {
    let mut s = sketching();
    let before = active(&s).clone();
    let cases = [
        "",
        "<svg",
        "<svg/>",
        "<html/>",
        "<svg><path d=\"M 0 0 L 1e300 1e300\"/></svg>",
        "<svg><circle r=\"1e300\"/></svg>",
        "<svg><path d=\"M0 0 A 1 1 0 0 0 0 0 Z\"/></svg>",
        "<svg><g transform=\"scale(0)\"><circle r=\"5\"/></g></svg>",
    ];
    for (i, c) in cases.iter().enumerate() {
        let dir = std::env::temp_dir().join(format!("chamfera-svg-h-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("h{i}.svg"));
        std::fs::write(&path, c).unwrap();
        let r = s.execute(
            "sketch.import_svg",
            &json!({"path": path.to_string_lossy()}),
        );
        assert!(r.is_err(), "{c}: {r:?}");
        assert_eq!(active(&s), &before, "{c}");
    }
    for p in [
        json!({"data": "!!!"}),
        json!({"data": "PHN2Zy8+", "units": "furlong"}),
        json!({"data": "PHN2Zy8+", "scale": -1}),
        json!({"data": "PHN2Zy8+", "scale": "big"}),
    ] {
        assert!(s.execute("sketch.import_svg_bytes", &p).is_err(), "{p}");
    }
}

// ----- intersection curves -----

/// A sphere of radius 20 about (0, 0, -6): a half disc on the XZ plane (arc from the
/// bottom (0, -26) counter-clockwise through +x to the top (0, 14), closed along the
/// axis) turned about the Z axis. Returns the session and the revolve's id.
fn sphere() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    s.execute(
        "sketch.arc",
        &json!({"cx": 0, "cy": -6, "x1": 0, "y1": -26, "x2": 0, "y2": 14}),
    )
    .unwrap();
    s.execute(
        "sketch.line",
        &json!({"x1": 0, "y1": 14, "x2": 0, "y2": -26}),
    )
    .unwrap();
    let axis = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": -40, "x2": 0, "y2": 30}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.construction",
        &json!({"curve": axis, "construction": true}),
    )
    .unwrap();
    let r = s
        .execute(
            "solid.revolve",
            &json!({"sketch": 1, "profiles": [{"x": 5, "y": -6}], "axis": axis}),
        )
        .unwrap();
    let id = r["feature"].as_u64().unwrap_or(2);
    (s, id)
}

/// The circle or arcs the intersection made: (centre, radius, total sweep).
fn cut_circle(s: &Session) -> (chamfera_geom::Point2, f64, f64) {
    let sk = active(s);
    let f = s.active_sketch().unwrap();
    let ids: Vec<CurveId> = f
        .intersections
        .iter()
        .flat_map(|i| i.curves.iter().map(|c| CurveId(*c)))
        .collect();
    assert!(!ids.is_empty());
    let (mut centre, mut radius, mut sweep) = (chamfera_geom::Point2::new(0.0, 0.0), 0.0, 0.0);
    for id in ids {
        match sk.curve(id).unwrap() {
            Curve::Circle {
                centre: c,
                radius: r,
                ..
            } => {
                centre = sk.point(*c).unwrap();
                radius = *r;
                sweep += 2.0 * PI;
            }
            Curve::Arc {
                centre: c,
                start,
                end,
                ..
            } => {
                let (c, a, b) = (
                    sk.point(*c).unwrap(),
                    sk.point(*start).unwrap(),
                    sk.point(*end).unwrap(),
                );
                centre = c;
                radius = c.distance(a);
                sweep += ((b - c).angle() - (a - c).angle()).rem_euclid(2.0 * PI);
            }
            other => panic!("{other:?}"),
        }
    }
    (centre, radius, sweep)
}

#[test]
fn a_plane_cuts_a_sphere_in_a_circle_of_the_hand_radius_and_follows_it() {
    // Sphere radius 20 about z = -6; the plane z = 6 is 12 from its centre, so the cut
    // is the circle of radius √(20² - 12²) = 16 about the plane's origin.
    let (mut s, _) = sphere();
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "xy", "distance": 6}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    s.execute("sketch.create", &json!({"plane": plane}))
        .unwrap();
    let r = s.execute("sketch.intersect", &json!({})).unwrap();
    assert_eq!(r["intersections"].as_array().unwrap().len(), 1, "{r}");
    let (c, radius, sweep) = cut_circle(&s);
    close(radius, 16.0, 1e-9, "cut radius");
    close(c.x.hypot(c.y), 0.0, 1e-9, "cut centre");
    close(sweep, 2.0 * PI, 1e-9, "all the way round");
    // Linked: moving the plane to z = 12 (18 from the centre) makes the circle
    // √(400 - 324) = √76.
    s.execute("feature.edit", &json!({"feature": plane, "distance": 12}))
        .unwrap();
    let sketch = s.active_sketch().map(|f| f.id).unwrap();
    s.execute("sketch.edit", &json!({"sketch": sketch})).ok();
    let (_, radius, _) = cut_circle(&s);
    close(radius, 76f64.sqrt(), 1e-9, "followed radius");
    // A plane that misses the sphere cuts nothing.
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let before = s.doc.clone();
    assert!(
        s.execute("sketch.intersect", &json!({"bodies": ["Nothing"]}))
            .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn an_intersection_through_a_box_is_its_rectangle() {
    // A 40 x 30 x 10 box from z = 0 cut by the XZ plane... the YZ plane (x = 0) through
    // its middle: the box spans x in [-20, 20], so the cut is the 30 x 10 rectangle,
    // four lines and a profile of 300 mm².
    let mut s = sketching();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": -20, "y1": -15, "x2": 20, "y2": 15}),
    )
    .unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 10}),
    )
    .unwrap();
    s.execute("sketch.create", &json!({"plane": "yz"})).unwrap();
    let r = s.execute("sketch.intersect", &json!({})).unwrap();
    let kinds = r["intersections"][0]["kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 4, "{r}");
    assert!(kinds.iter().all(|k| k == "line"));
    let areas: Vec<f64> = active(&s).profiles().iter().map(|p| p.area).collect();
    assert_eq!(areas.len(), 1);
    close(areas[0], 300.0, 1e-9, "section area");
    // Unlinked copies stay as drawn.
    let r = s
        .execute("sketch.intersect", &json!({"linked": false}))
        .unwrap();
    assert_eq!(r["linked"], false);
}

// ----- copy and paste -----

#[test]
fn pasted_geometry_keeps_its_constraints_and_solves_in_another_sketch() {
    // A circle of radius 5 with its radius dimensioned and its centre fixed at (10, 10):
    // fully constrained. Copied and pasted 30 mm to the right in the same sketch, then
    // into a second sketch at (0, 0): each copy has its radius dimension and fix, so
    // every sketch stays at 0 degrees of freedom and each disc is 25 π.
    let mut s = sketching();
    let c = s
        .execute("sketch.circle", &json!({"cx": 10, "cy": 10, "radius": 5}))
        .unwrap();
    let curve = c["curve"].as_u64().unwrap();
    let centre = active(&s).curve(CurveId(curve as u32)).unwrap().points()[0];
    s.execute(
        "sketch.constrain",
        &json!({"constraint": {"kind": "radius", "curve": curve, "value": 5}}),
    )
    .unwrap();
    s.execute(
        "sketch.constrain",
        &json!({"constraint": {"kind": "fix", "point": centre.0, "at": {"x": 10, "y": 10}}}),
    )
    .unwrap();
    let copied = s
        .execute("sketch.copy", &json!({"curves": [curve]}))
        .unwrap();
    assert_eq!(copied["constraints"], 2, "{copied}");
    let r = s
        .execute("sketch.paste", &json!({"dx": 30, "dy": 0}))
        .unwrap();
    assert_eq!(r["created_constraints"].as_array().unwrap().len(), 2);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["dof"], 0, "{g}");
    let areas = r["profiles"].as_array().unwrap();
    assert_eq!(areas.len(), 2);
    for a in areas {
        close(a.as_f64().unwrap(), 25.0 * PI, 1e-9, "disc");
    }
    // Undo takes the paste back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(active(&s).curves().count(), 1);
    // Into another sketch, its middle at the origin.
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    let r = s.execute("sketch.paste", &json!({"x": 0, "y": 0})).unwrap();
    assert_eq!(r["created_curves"].as_array().unwrap().len(), 1);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["dof"], 0, "{g}");
    let circles = curves_of(&s, |c| matches!(c, Curve::Circle { .. }));
    let Some((_, Curve::Circle { centre, .. })) = circles.first() else {
        panic!()
    };
    close(active(&s).point(*centre).unwrap().x, 0.0, 1e-9, "pasted x");
    assert!(s.execute("sketch.paste", &json!({"x": 1})).is_err());
    assert!(Session::new().execute("sketch.paste", &json!({})).is_err());
}

// ----- shapes and spline handles -----

#[test]
fn edge_polygons_arc_slots_and_spline_handles_run_as_commands() {
    let mut s = sketching();
    let r = s
        .execute(
            "sketch.polygon.edge",
            &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 0, "sides": 4}),
        )
        .unwrap();
    assert_eq!(r["created_curves"].as_array().unwrap().len(), 5); // 4 sides + circle
    // A square of side 10 above the edge: 100 mm².
    let areas: Vec<f64> = active(&s).profiles().iter().map(|p| p.area).collect();
    close(areas[0], 100.0, 1e-9, "square");
    let mut t = sketching();
    t.execute(
        "sketch.slot.arc",
        &json!({"cx": 0, "cy": 0, "x1": 30, "y1": 0, "x2": 0, "y2": 30, "width": 10}),
    )
    .unwrap();
    let areas: Vec<f64> = active(&t).profiles().iter().map(|p| p.area).collect();
    close(
        areas[0],
        175.0 * PI,
        1e-6,
        "arc slot (see the sketch tests)",
    );
    let sp = t
        .execute(
            "sketch.spline",
            &json!({"points": [{"x": 50, "y": 0}, {"x": 60, "y": 10}, {"x": 70, "y": 0}]}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    let h = t
        .execute("sketch.spline.handles", &json!({"curve": sp}))
        .unwrap();
    assert!(
        h["start_handle"].is_u64() && h["end_handle"].is_u64(),
        "{h}"
    );
    t.execute(
        "sketch.spline.handles",
        &json!({"curve": sp, "remove": true}),
    )
    .unwrap();
    assert!(
        t.execute("sketch.spline.handles", &json!({"curve": 1}))
            .is_err()
    );
}

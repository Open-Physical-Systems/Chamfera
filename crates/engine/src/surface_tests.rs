//! Tests of the surface features: swept surfaces, patch, stitch, trim and thickening a
//! surface, each against areas and volumes worked out by hand.

use super::*;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

fn id(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or_else(|| panic!("no {key} in {v}"))
}

/// A sketch on xy with the 50 x 30 rectangle from the origin; returns the sketch id.
fn rect_sketch(s: &mut Session) -> u64 {
    let sketch = id(&s.execute("sketch.create", &json!({"plane": "xy"})).unwrap(), "sketch");
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    sketch
}

/// The open sides of the rectangle extruded by 10; returns the feature id.
fn tube(s: &mut Session) -> u64 {
    let sketch = rect_sketch(s);
    let r = s
        .execute(
            "surface.extrude",
            &json!({"sketch": sketch, "profiles": [{"x": 25, "y": 15}], "distance": 10}),
        )
        .unwrap();
    id(&r, "feature")
}

fn solid<'a>(s: &'a Session, name: &str) -> &'a chamfera_kernel::Body {
    &s.solids()
        .iter()
        .find(|x| x.name == name)
        .unwrap_or_else(|| panic!("no body {name}"))
        .body
}

#[test]
fn a_surface_extrude_of_a_rectangle_has_area_1600_and_no_volume() {
    let mut s = Session::new();
    tube(&mut s);
    let b = solid(&s, "SurfaceExtrude1");
    // Sides 2 (50 + 30) x 10 = 1600; 4 rim edges at each end.
    assert!(close(b.area().unwrap(), 1600.0));
    assert_eq!(b.boundary_edges().len(), 8);
    assert!(b.volume().is_err());
    // It is shown, as a surface.
    let x = s.solids().iter().find(|x| x.name == "SurfaceExtrude1").unwrap();
    let scene = s.scene.body(x.scene_id.unwrap()).unwrap();
    assert!(scene.surface);
    // Editing the distance to 20 doubles the area.
    let f = s.doc.features.last().unwrap().id;
    s.execute("feature.edit", &json!({"feature": f, "distance": 20}))
        .unwrap();
    assert!(close(solid(&s, "SurfaceExtrude1").area().unwrap(), 3200.0));
    // A key of another feature is refused.
    assert!(s
        .execute("feature.edit", &json!({"feature": f, "angle": 90}))
        .is_err());
}

#[test]
fn a_revolved_open_line_is_a_cylinder_of_area_two_pi_r_h() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    // The axis along y and the line x = 10 from y = 0 to y = 20: r = 10, h = 20.
    let axis = id(
        &s.execute("sketch.line", &json!({"x1": 0, "y1": -5, "x2": 0, "y2": 25}))
            .unwrap(),
        "curve",
    );
    let line = id(
        &s.execute("sketch.line", &json!({"x1": 10, "y1": 0, "x2": 10, "y2": 20}))
            .unwrap(),
        "curve",
    );
    s.execute("sketch.finish", &json!({})).unwrap();
    let sketch = s.doc.sketches.last().unwrap().id;
    s.execute(
        "surface.revolve",
        &json!({"sketch": sketch, "curves": [line], "axis": axis}),
    )
    .unwrap();
    let b = solid(&s, "SurfaceRevolve1");
    // 2 pi r h = 400 pi.
    assert!(close(b.area().unwrap(), 400.0 * PI));
    // Trimmed by the plane y = 10, half is left. The xz plane's normal is -y, so the
    // plane y = 10 is xz offset by -10.
    let plane = id(
        &s.execute(
            "construct.plane_offset",
            &json!({"base": "xz", "distance": -10}),
        )
        .unwrap(),
        "feature",
    );
    s.execute(
        "surface.trim",
        &json!({"body": "SurfaceRevolve1", "plane": plane, "keep": "below"}),
    )
    .unwrap();
    assert!(close(solid(&s, "SurfaceRevolve1").area().unwrap(), 200.0 * PI));
}

#[test]
fn patching_both_ends_and_stitching_gives_the_15000_box() {
    let mut s = Session::new();
    let f = tube(&mut s);
    let rim = |end: &str| -> Vec<String> { (0..4).map(|i| format!("{f}:{end} {i}")).collect() };
    s.execute("surface.patch", &json!({"edges": rim("top")}))
        .unwrap();
    s.execute("surface.patch", &json!({"edges": rim("bottom")}))
        .unwrap();
    assert!(close(solid(&s, "Patch1").area().unwrap(), 1500.0));
    s.execute(
        "surface.stitch",
        &json!({"bodies": ["SurfaceExtrude1", "Patch1", "Patch2"]}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 1);
    let b = solid(&s, "Stitch1");
    assert!(!b.is_open());
    // 50 x 30 x 10.
    assert!(close(b.volume().unwrap(), 15000.0));
}

#[test]
fn thickening_a_50_by_30_patch_by_2_gives_3000_and_uses_it_up() {
    let mut s = Session::new();
    let sketch = rect_sketch(&mut s);
    s.execute(
        "surface.patch",
        &json!({"sketch": sketch, "profiles": [{"x": 25, "y": 15}]}),
    )
    .unwrap();
    let f = s.doc.features.last().unwrap().id;
    s.execute(
        "solid.thicken",
        &json!({"faces": [format!("{f}:patch")], "thickness": 2}),
    )
    .unwrap();
    // 50 x 30 x 2; the patch became the solid.
    assert_eq!(s.solids().len(), 1);
    assert!(close(s.solids()[0].body.volume().unwrap(), 3000.0));
}

#[test]
fn refused_surface_commands_leave_the_design_unchanged() {
    let mut s = Session::new();
    tube(&mut s);
    let sketch = s.doc.sketches.last().unwrap().id;
    let before = s.doc.to_json().unwrap();
    let refused = [
        ("surface.extrude", json!({"sketch": sketch, "distance": 10})),
        (
            "surface.extrude",
            json!({"sketch": sketch, "profiles": [{"x": 25, "y": 15}], "distance": 0}),
        ),
        ("surface.patch", json!({"edges": ["1:nothing 0"]})),
        ("surface.patch", json!({})),
        ("surface.stitch", json!({"bodies": ["SurfaceExtrude1"]})),
        ("surface.stitch", json!({"bodies": ["NoSuchBody"]})),
        ("surface.trim", json!({"body": "SurfaceExtrude1"})),
        (
            "surface.trim",
            json!({"body": "SurfaceExtrude1", "plane": "xy", "keep": "below"}),
        ),
        (
            "surface.trim",
            json!({"body": "SurfaceExtrude1", "tool": "SurfaceExtrude1"}),
        ),
        ("surface.revolve", json!({"sketch": sketch, "curves": [1]})),
        ("surface.sweep", json!({"sketch": sketch, "profiles": [{"x": 25, "y": 15}]})),
    ];
    for (cmd, p) in refused {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert_eq!(s.doc.to_json().unwrap(), before, "{cmd} changed the design");
    }
    // Undo still takes back the surface extrude itself: no step was added.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.solids().is_empty());
}

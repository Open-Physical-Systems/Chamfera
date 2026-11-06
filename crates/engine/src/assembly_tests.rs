//! Components and joints driven through commands, as a user or agent would. Expected
//! placements and bounds are worked out by hand in the comments.

use crate::Session;
use serde_json::{Value, json};

/// Sketches an x1..x2 by y1..y2 rectangle on XY and extrudes it `h` up; returns the
/// feature id.
fn plate(s: &mut Session, x1: f64, y1: f64, x2: f64, y2: f64, h: f64) -> u64 {
    let sk = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let f = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sk["sketch"], "distance": h,
                    "profiles": [{"x": (x1 + x2) / 2.0, "y": (y1 + y2) / 2.0}]}),
        )
        .unwrap();
    f["feature"].as_u64().unwrap()
}

fn component(s: &mut Session, id: u64) -> Value {
    let list = s.execute("component.list", &json!({})).unwrap();
    list["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["component"] == json!(id))
        .cloned()
        .unwrap()
}

/// The exact bounds of a component's first body: [min x, min y, min z, max x, ...].
fn bounds(s: &mut Session, id: u64) -> [f64; 6] {
    let c = component(s, id);
    let b = &c["bodies"][0]["bounds"];
    let v = |p: &str, k: &str| b[p][k].as_f64().unwrap();
    [
        v("min", "x"),
        v("min", "y"),
        v("min", "z"),
        v("max", "x"),
        v("max", "y"),
        v("max", "z"),
    ]
}

fn close(a: [f64; 6], b: [f64; 6]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

fn new_component(s: &mut Session, name: &str, feature: u64) -> u64 {
    s.execute(
        "component.new",
        &json!({"name": name, "from_bodies": [feature]}),
    )
    .unwrap()["component"]
        .as_u64()
        .unwrap()
}

/// Two components: A (grounded), a 40 x 30 x 4 plate, and B, a 40 x 10 x 4 plate, both
/// from the origin. Returns their ids.
fn two_plates(s: &mut Session) -> (u64, u64) {
    let fa = plate(s, 0.0, 0.0, 40.0, 30.0, 4.0);
    let fb = plate(s, 0.0, 0.0, 40.0, 10.0, 4.0);
    let a = new_component(s, "A", fa);
    let b = new_component(s, "B", fb);
    s.execute("component.ground", &json!({"component": a}))
        .unwrap();
    (a, b)
}

fn at_origin(component: u64, axis: [f64; 3]) -> Value {
    json!({"component": component, "point": {"x": 0, "y": 0, "z": 0},
           "axis": {"x": axis[0], "y": axis[1], "z": axis[2]}})
}

#[test]
fn a_moved_box_component_shifts_its_bounds_by_exactly_the_move() {
    let mut s = Session::new();
    let f = plate(&mut s, 0.0, 0.0, 50.0, 30.0, 10.0);
    let c = new_component(&mut s, "Box", f);
    assert!(close(bounds(&mut s, c), [0.0, 0.0, 0.0, 50.0, 30.0, 10.0]));
    s.execute(
        "component.move",
        &json!({"component": c, "transform": {"translate": {"x": 10, "y": 0, "z": 0}}}),
    )
    .unwrap();
    // 0..50 moved by 10 is 10..60; y and z do not change.
    assert!(close(bounds(&mut s, c), [10.0, 0.0, 0.0, 60.0, 30.0, 10.0]));
    let v = component(&mut s, c)["bodies"][0]["volume"]
        .as_f64()
        .unwrap();
    assert!((v - 15_000.0).abs() < 1e-6, "a move keeps the volume: {v}");
    // Undo puts it back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&mut s, c), [0.0, 0.0, 0.0, 50.0, 30.0, 10.0]));
}

#[test]
fn a_revolute_joint_at_90_degrees_turns_b_about_z() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let r = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "a": at_origin(a, [0.0, 0.0, 1.0]),
                    "b": at_origin(b, [0.0, 0.0, 1.0]), "value": 90}),
        )
        .unwrap();
    assert!((r["value"].as_f64().unwrap() - 90.0).abs() < 1e-9);
    // B's plate 0..40 x 0..10 turned a quarter about +z: (x, y) -> (-y, x), so x in
    // -10..0 and y in 0..40; the point (20, 0, 0) of B lands on (0, 20, 0).
    assert!(close(bounds(&mut s, b), [-10.0, 0.0, 0.0, 0.0, 40.0, 4.0]));
    let w = component(&mut s, b)["world"].clone();
    let (m, t) = (&w["matrix"], &w["translation"]);
    let row = |i: usize, k: &str| m[i][0].as_f64().unwrap() * 20.0 + t[k].as_f64().unwrap();
    let q = [row(0, "x"), row(1, "y"), row(2, "z")];
    assert!(
        q[0].abs() < 1e-9 && (q[1] - 20.0).abs() < 1e-9 && q[2].abs() < 1e-9,
        "{q:?}"
    );
    // A grounded component does not move.
    assert!(close(bounds(&mut s, a), [0.0, 0.0, 0.0, 40.0, 30.0, 4.0]));
    // A component a joint positions cannot be moved by hand.
    assert!(
        s.execute(
            "component.move",
            &json!({"component": b, "transform": {"translate": {"x": 1, "y": 0, "z": 0}}}),
        )
        .is_err()
    );
}

#[test]
fn a_slider_at_15_shifts_b_by_15_along_its_axis() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let r = s
        .execute(
            "joint.create",
            &json!({"type": "slider", "a": at_origin(a, [1.0, 0.0, 0.0]),
                    "b": at_origin(b, [1.0, 0.0, 0.0]), "min": 0, "max": 20}),
        )
        .unwrap();
    let j = r["joint"].clone();
    assert!(close(bounds(&mut s, b), [0.0, 0.0, 0.0, 40.0, 10.0, 4.0]));
    s.execute("joint.set_value", &json!({"joint": j, "value": 15}))
        .unwrap();
    assert!(close(bounds(&mut s, b), [15.0, 0.0, 0.0, 55.0, 10.0, 4.0]));
    // Outside the limits: refused, nothing changes.
    let e = s
        .execute("joint.set_value", &json!({"joint": j, "value": 25}))
        .unwrap_err();
    assert!(e.to_string().contains("outside the limits"), "{e}");
    assert!(close(bounds(&mut s, b), [15.0, 0.0, 0.0, 55.0, 10.0, 4.0]));
    // Deleting the joint leaves B where the joint had put it.
    s.execute("joint.delete", &json!({"joint": j})).unwrap();
    assert!(close(bounds(&mut s, b), [15.0, 0.0, 0.0, 55.0, 10.0, 4.0]));
}

#[test]
fn a_chain_of_joints_composes_through_the_session() {
    // A grounded; B: revolute 90° about z at the origin; C: slider 5 along B's x axis.
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let fc = plate(&mut s, 0.0, 0.0, 2.0, 2.0, 2.0);
    let c = new_component(&mut s, "C", fc);
    s.execute(
        "joint.create",
        &json!({"type": "revolute", "a": at_origin(a, [0.0, 0.0, 1.0]),
                "b": at_origin(b, [0.0, 0.0, 1.0]), "value": 90}),
    )
    .unwrap();
    s.execute(
        "joint.create",
        &json!({"type": "slider", "a": at_origin(b, [1.0, 0.0, 0.0]),
                "b": at_origin(c, [1.0, 0.0, 0.0]), "value": 5}),
    )
    .unwrap();
    // C's 2 x 2 x 2 cube first slides 5 along B's x (x in 5..7, y in 0..2), then turns
    // with B a quarter about z, (x, y) -> (-y, x): x in -2..0, y in 5..7.
    assert!(close(bounds(&mut s, c), [-2.0, 5.0, 0.0, 0.0, 7.0, 2.0]));
}

#[test]
fn a_joint_loop_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    s.execute(
        "joint.create",
        &json!({"type": "rigid", "a": at_origin(a, [0.0, 0.0, 1.0]),
                "b": at_origin(b, [0.0, 0.0, 1.0])}),
    )
    .unwrap();
    let before = s.doc.clone();
    let e = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "a": at_origin(b, [0.0, 0.0, 1.0]),
                    "b": at_origin(a, [0.0, 0.0, 1.0])}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("loop"), "{e}");
    assert_eq!(s.doc, before);
    // Grounding B too would hold it twice: refused.
    assert!(
        s.execute("component.ground", &json!({"component": b}))
            .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn components_and_joints_survive_save_and_open() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    s.execute(
        "joint.create",
        &json!({"type": "revolute", "name": "Hinge", "a": at_origin(a, [0.0, 0.0, 1.0]),
                "b": at_origin(b, [0.0, 0.0, 1.0]), "value": 90, "min": -90, "max": 90}),
    )
    .unwrap();
    s.execute("component.tint", &json!({"component": a, "tint": true}))
        .unwrap();
    let saved = s.execute("design.save_bytes", &json!({})).unwrap();
    let before = s.doc.clone();
    let mut t = Session::new();
    t.execute("design.open_bytes", &json!({"data": saved["data"]}))
        .unwrap();
    assert_eq!(t.doc, before);
    assert!(close(bounds(&mut t, b), [-10.0, 0.0, 0.0, 0.0, 40.0, 4.0]));
    assert!(t.scene.bodies.iter().any(|x| x.tint.is_some()));
}

#[test]
fn an_old_design_opens_as_one_root_component() {
    let mut s = Session::new();
    plate(&mut s, 0.0, 0.0, 50.0, 30.0, 10.0);
    // The same design as a format 11 file: no assembly in it.
    let mut text: Value = serde_json::from_str(&s.doc.to_json().unwrap()).unwrap();
    assert!(
        text.get("assembly").is_none(),
        "a design without components writes none"
    );
    text["version"] = json!(11);
    let old = chamfera_doc::Document::from_json(&text.to_string()).unwrap();
    assert!(old.assembly.components.is_empty());
    let mut t = Session::new();
    t.doc = old;
    t.rebuild();
    let list = t.execute("component.list", &json!({})).unwrap();
    let comps = list["components"].as_array().unwrap();
    assert_eq!(comps.len(), 1, "the root only");
    assert_eq!(comps[0]["component"], json!(0));
    assert_eq!(comps[0]["bodies"].as_array().unwrap().len(), 1);
}

#[test]
fn new_features_go_into_the_active_component_and_copies_follow_it() {
    let mut s = Session::new();
    let c = s.execute("component.new", &json!({"name": "Peg"})).unwrap()["component"]
        .as_u64()
        .unwrap();
    assert_eq!(
        u64::from(s.doc.assembly.active),
        c,
        "a new empty component is active"
    );
    plate(&mut s, 0.0, 0.0, 10.0, 10.0, 10.0);
    let copy = s
        .execute(
            "component.copy",
            &json!({"component": c, "transform": {"translate": {"x": 100, "y": 0, "z": 0}}}),
        )
        .unwrap()["component"]
        .as_u64()
        .unwrap();
    assert!(close(bounds(&mut s, c), [0.0, 0.0, 0.0, 10.0, 10.0, 10.0]));
    assert!(close(
        bounds(&mut s, copy),
        [100.0, 0.0, 0.0, 110.0, 10.0, 10.0]
    ));
    s.execute("component.activate", &json!({"component": 0}))
        .unwrap();
    assert_eq!(s.solids().len(), 2);
    s.execute("component.delete", &json!({"component": c}))
        .unwrap();
    assert_eq!(s.solids().len(), 1, "the copy goes with its original");
    assert_eq!(s.solids()[0].component, 0, "the body returns to the root");
}

#[test]
fn a_face_origin_follows_the_face_and_a_pick_names_the_component() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let fa = s.doc.assembly.component(a as u32).unwrap().features[0];
    let fb = s.doc.assembly.component(b as u32).unwrap().features[0];
    // B's bottom centre (20, 5, 0) on A's top centre (20, 15, 4). The bottom's outward
    // normal points down, so flip B (half a turn about x) to stack it on A.
    s.execute(
        "joint.create",
        &json!({"type": "rigid", "a": {"face": format!("{fa}:top")},
                "b": {"face": format!("{fb}:bottom")}, "flip": true}),
    )
    .unwrap();
    // Turned about x, B's 40 x 10 x 4 plate keeps x 0..40, spans y 10..20 around 15 and
    // z 4..8 on top of A.
    assert!(close(bounds(&mut s, b), [0.0, 10.0, 4.0, 40.0, 20.0, 8.0]));
    // Looking down at the stack, the middle of the view is on B.
    s.execute(
        "view.standard",
        &json!({"view": "top", "width": 800, "height": 600}),
    )
    .unwrap();
    s.execute("view.fit", &json!({"width": 800, "height": 600}))
        .unwrap();
    let r = s
        .execute(
            "select.pick",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert_eq!(r["component"], json!(b));
    // The pixel under B's top names B's face as a joint origin.
    let o = s
        .execute(
            "joint.origin_at",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert_eq!(o["origin"]["component"], json!(b));
}

#[test]
fn a_face_origin_follows_its_face_when_the_part_changes() {
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "thick", "expression": "4 mm"}))
        .unwrap();
    let sk = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 40, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let fa = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sk["sketch"], "distance": "thick",
                    "profiles": [{"x": 20, "y": 15}]}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let fb = plate(&mut s, 0.0, 0.0, 10.0, 10.0, 10.0);
    let a = new_component(&mut s, "A", fa);
    let b = new_component(&mut s, "B", fb);
    s.execute("component.ground", &json!({"component": a}))
        .unwrap();
    // B's own origin (0, 0, 0) on A's top centre (20, 15, thick), axes aligned.
    s.execute(
        "joint.create",
        &json!({"type": "rigid", "a": {"face": format!("{fa}:top")}, "b": at_origin(b, [0.0, 0.0, 1.0])}),
    )
    .unwrap();
    assert!(close(
        bounds(&mut s, b),
        [20.0, 15.0, 4.0, 30.0, 25.0, 14.0]
    ));
    // A thicker A raises its top, and B rides on it: z from 6 to 16.
    s.execute("param.set", &json!({"name": "thick", "expression": "6 mm"}))
        .unwrap();
    assert!(close(
        bounds(&mut s, b),
        [20.0, 15.0, 6.0, 30.0, 25.0, 16.0]
    ));
}

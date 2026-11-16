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
fn a_loop_that_fits_is_solved_and_one_that_cannot_close_is_refused() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    s.execute(
        "joint.create",
        &json!({"type": "rigid", "a": at_origin(a, [0.0, 0.0, 1.0]),
                "b": at_origin(b, [0.0, 0.0, 1.0])}),
    )
    .unwrap();
    // A revolute joint back from B to A about the same axis closes a loop that already
    // fits: its angle solves to 0 (from the 30 degrees typed) and nothing moves.
    let r = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "a": at_origin(b, [0.0, 0.0, 1.0]),
                    "b": at_origin(a, [0.0, 0.0, 1.0]), "value": 30}),
        )
        .unwrap();
    let v = s
        .doc
        .assembly
        .joint(r["joint"].as_u64().unwrap() as u32)
        .unwrap()
        .value;
    assert!(v.abs() < 1e-9, "{v}");
    assert!(close(bounds(&mut s, b), [0.0, 0.0, 0.0, 40.0, 10.0, 4.0]));
    // A rigid joint that holds B's origin 5 mm from where the first one holds it cannot
    // fit: nothing in that loop can move. Refused, and the design is unchanged.
    let before = s.doc.clone();
    let off = json!({"component": a, "point": {"x": 5, "y": 0, "z": 0},
                     "axis": {"x": 0, "y": 0, "z": 1}});
    let e = s
        .execute(
            "joint.create",
            &json!({"type": "rigid", "a": off, "b": at_origin(b, [0.0, 0.0, 1.0])}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("over-constrained"), "{e}");
    assert_eq!(s.doc, before);
    // Grounding B too fits (B sits where A's joint holds it), so it is allowed.
    s.execute("component.ground", &json!({"component": b}))
        .unwrap();
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

// ---- Assembly tools: limits, drag, motion study, interference, exploded view ----

fn revolute_about_z(s: &mut Session, a: u64, b: u64, extra: Value) -> u64 {
    let mut p = json!({"type": "revolute", "a": at_origin(a, [0.0, 0.0, 1.0]),
                       "b": at_origin(b, [0.0, 0.0, 1.0])});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            p.insert(k.clone(), v.clone());
        }
    }
    s.execute("joint.create", &p).unwrap()["joint"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_revolute_limited_to_0_to_90_degrees_refuses_120() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({"min": 0, "max": 90}));
    let before = s.doc.clone();
    let e = s
        .execute("joint.set_value", &json!({"joint": j, "value": 120}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("120°") && e.contains("0° to 90°"), "{e}");
    assert_eq!(s.doc, before, "a refused value changes nothing");
    // 90 is the limit itself and is allowed.
    s.execute("joint.set_value", &json!({"joint": j, "value": 90}))
        .unwrap();
}

#[test]
fn dragging_from_the_x_side_to_the_y_side_turns_a_hinge_a_quarter() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({}));
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    let (w, h) = (800.0, 600.0);
    // Seen from above, a drag from the point (20, 0, 0) to (0, 20, 0) sweeps a quarter
    // turn counter-clockwise about +z through the origin: +90°.
    let from = s
        .world_to_screen(chamfera_geom::Point3::new(20.0, 0.0, 0.0), w, h)
        .unwrap();
    let to = s
        .world_to_screen(chamfera_geom::Point3::new(0.0, 20.0, 0.0), w, h)
        .unwrap();
    let doc = s.doc.clone();
    let r = s
        .execute(
            "joint.drag",
            &json!({"joint": j, "from": {"x": from.0, "y": from.1},
                    "to": {"x": to.0, "y": to.1}, "width": w, "height": h}),
        )
        .unwrap();
    assert!((r["value"].as_f64().unwrap() - 90.0).abs() < 1e-6, "{r}");
    assert_eq!(s.doc, doc, "a drag only shows the move");
    // B turned a quarter: 0..40 x 0..10 becomes -10..0 x 0..40.
    assert!(close(bounds(&mut s, b), [-10.0, 0.0, 0.0, 0.0, 40.0, 4.0]));
    // Ending the drag keeps it as one undo step.
    s.execute("joint.set_value", &json!({"joint": j, "value": r["value"]}))
        .unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&mut s, b), [0.0, 0.0, 0.0, 40.0, 10.0, 4.0]));
}

#[test]
fn a_limited_drag_stops_at_the_limit() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({"min": 0, "max": 30}));
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    let (w, h) = (800.0, 600.0);
    let p = |s: &Session, x: f64, y: f64| {
        s.world_to_screen(chamfera_geom::Point3::new(x, y, 0.0), w, h)
            .unwrap()
    };
    let (from, to) = (p(&s, 20.0, 0.0), p(&s, 0.0, 20.0));
    let r = s
        .execute(
            "joint.drag",
            &json!({"joint": j, "from": {"x": from.0, "y": from.1},
                    "to": {"x": to.0, "y": to.1}, "width": w, "height": h}),
        )
        .unwrap();
    assert!((r["value"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{r}");
    assert_eq!(r["clamped"], json!(true));
}

#[test]
fn halfway_between_keys_at_0_and_90_degrees_the_hinge_is_at_45() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({}));
    s.execute(
        "motion.add_key",
        &json!({"joint": j, "time": 0, "value": 0}),
    )
    .unwrap();
    s.execute(
        "motion.add_key",
        &json!({"joint": j, "time": 1, "value": 90}),
    )
    .unwrap();
    let doc = s.doc.clone();
    let r = s.execute("motion.set_time", &json!({"t": 0.5})).unwrap();
    // Linear: 0 + (90 - 0) * 0.5 = 45.
    assert!(
        (r["joints"][0]["value"].as_f64().unwrap() - 45.0).abs() < 1e-9,
        "{r}"
    );
    assert_eq!(s.doc, doc, "showing a time changes nothing in the design");
    // B (0..40 x 0..10) turned 45° about z: (x, y) -> ((x - y)/√2, (x + y)/√2). Its far
    // edge's outer corner (40, 0) lands on (20√2, 20√2); the corner (40, 10) on
    // (15√2, 25√2) and (0, 10) on (-5√2, 5√2). So x runs -5√2..20√2, y 0..25√2.
    let r2 = std::f64::consts::SQRT_2;
    assert!(close(
        bounds(&mut s, b),
        [-5.0 * r2, 0.0, 0.0, 20.0 * r2, 25.0 * r2, 4.0]
    ));
    let world = component(&mut s, b)["world"].clone();
    let (m, t) = (&world["matrix"], &world["translation"]);
    let at = |i: usize, k: &str| m[i][0].as_f64().unwrap() * 40.0 + t[k].as_f64().unwrap();
    assert!((at(0, "x") - 20.0 * r2).abs() < 1e-9 && (at(1, "y") - 20.0 * r2).abs() < 1e-9);
    // Playing adds no undo step: undo takes back the last key, not a time.
    s.execute("motion.set_time", &json!({"t": 0.75})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.motion.keys.len(), 1);
    // Leaving the study shows the joint's own value (0) again.
    s.execute("motion.set_time", &json!({"t": null})).unwrap();
    assert!(close(bounds(&mut s, b), [0.0, 0.0, 0.0, 40.0, 10.0, 4.0]));
}

#[test]
fn a_key_outside_the_joints_limits_is_refused() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({"min": 0, "max": 90}));
    assert!(
        s.execute(
            "motion.add_key",
            &json!({"joint": j, "time": 1, "value": 120})
        )
        .is_err()
    );
    assert!(s.doc.motion.keys.is_empty());
}

/// A `size` mm cube from x0 in x (y and z from 0), as its own component.
fn cube(s: &mut Session, name: &str, x0: f64, size: f64) -> u64 {
    let f = plate(s, x0, 0.0, x0 + size, size, size);
    new_component(s, name, f)
}

#[test]
fn two_cubes_overlapping_by_5_interfere_by_500_and_moving_one_6_clears_it() {
    let mut s = Session::new();
    let a = cube(&mut s, "A", 0.0, 10.0);
    let b = cube(&mut s, "B", 5.0, 10.0);
    let r = s.execute("inspect.interference", &json!({})).unwrap();
    // x overlap 5..10 (5 mm) by the full 10 x 10 in y and z: 500 mm³.
    assert_eq!(r["count"], json!(1), "{r}");
    let v = r["pairs"][0]["volume"].as_f64().unwrap();
    assert!((v - 500.0).abs() < 1e-6, "{v}");
    // The overlap is drawn in the interference colour.
    assert!(
        s.scene
            .fills
            .iter()
            .any(|f| f.style == chamfera_render::FillStyle::Interference)
    );
    // Limited to one component there is nothing to compare it with.
    let r = s
        .execute("inspect.interference", &json!({"components": [a]}))
        .unwrap();
    assert_eq!(r["count"], json!(0));
    // B moved 6 in x sits at 11..21: a 1 mm gap from A.
    s.execute(
        "component.move",
        &json!({"component": b, "transform": {"translate": {"x": 6, "y": 0, "z": 0}}}),
    )
    .unwrap();
    let r = s.execute("inspect.interference", &json!({})).unwrap();
    assert_eq!(r["count"], json!(0), "{r}");
    assert!(
        s.scene
            .fills
            .iter()
            .all(|f| f.style != chamfera_render::FillStyle::Interference)
    );
}

#[test]
fn hinge_leaves_collide_at_180_degrees_but_not_at_90() {
    // Leaf A (grounded): x 0..20, y 0..10, z 0..2. Leaf B lies open beside it: x -22..-2.
    // The hinge turns B about the y axis through (-2, 0, 1).
    //   At 90°, a point at (x, z) relative to the pivot goes to (z, -x): B's x -20..0,
    //   z -1..1 relative becomes x -1..1, z 0..20, so world x -3..-1: 1 mm short of A.
    //   At 180°, (x, z) -> (-x, -z): world x -2..18, z 0..2, overlapping A in x 0..18,
    //   all of y 0..10 and z 0..2: 18 * 10 * 2 = 360 mm³.
    let mut s = Session::new();
    let fa = plate(&mut s, 0.0, 0.0, 20.0, 10.0, 2.0);
    let fb = plate(&mut s, -22.0, 0.0, -2.0, 10.0, 2.0);
    let a = new_component(&mut s, "LeafA", fa);
    let b = new_component(&mut s, "LeafB", fb);
    s.execute("component.ground", &json!({"component": a}))
        .unwrap();
    let pivot = |c: u64| {
        json!({"component": c, "point": {"x": -2, "y": 0, "z": 1},
               "axis": {"x": 0, "y": 1, "z": 0}})
    };
    let j = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "a": pivot(a), "b": pivot(b)}),
        )
        .unwrap()["joint"]
        .as_u64()
        .unwrap();
    for (t, v) in [(0, 90), (1, 180)] {
        s.execute(
            "motion.add_key",
            &json!({"joint": j, "time": t, "value": v}),
        )
        .unwrap();
    }
    let r = s
        .execute("inspect.interference", &json!({"at_keys": true}))
        .unwrap();
    let keys = r["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0]["time"], json!(0.0));
    assert_eq!(keys[0]["count"], json!(0), "{r}");
    assert_eq!(keys[1]["count"], json!(1), "{r}");
    let v = keys[1]["pairs"][0]["volume"].as_f64().unwrap();
    assert!((v - 360.0).abs() < 1e-6, "{v}");
    // As designed (value 0) the leaves lie side by side: no interference, and the check
    // left the design and the time shown as they were.
    assert_eq!(r["count"], json!(0));
    assert!(s.assembly_view().motion_time.is_none());
    assert!(close(bounds(&mut s, b), [-22.0, 0.0, 0.0, -2.0, 10.0, 2.0]));
}

#[test]
fn explode_factor_2_moves_the_outer_cubes_20_mm_apart_and_keeps_the_model() {
    // Cubes at x 0..10, 20..30 and 40..50: the assembly's box is 0..50 by 10 by 10,
    // centre (25, 5, 5). The outer cubes' centres are 20 from it along x (and nothing in
    // y or z), so factor 2 shows them (2 - 1) * 20 = 20 mm further out; the middle one,
    // at the centre, does not move.
    let mut s = Session::new();
    let a = cube(&mut s, "A", 0.0, 10.0);
    let m = cube(&mut s, "M", 20.0, 10.0);
    let c = cube(&mut s, "C", 40.0, 10.0);
    let doc = s.doc.clone();
    let r = s.execute("view.explode", &json!({"factor": 2})).unwrap();
    let comps = r["components"].as_array().unwrap();
    assert_eq!(comps.len(), 2, "{r}");
    let off = |id: u64| {
        comps
            .iter()
            .find(|x| x["component"] == json!(id))
            .map(|x| x["offset"]["x"].as_f64().unwrap())
    };
    assert!((off(a).unwrap() + 20.0).abs() < 1e-9);
    assert!((off(c).unwrap() - 20.0).abs() < 1e-9);
    assert!(off(m).is_none());
    assert_eq!(s.doc, doc, "an exploded view is not a change to the design");
    // The exact bodies stay where they are; the shown mesh of A moves to -20..-10.
    assert!(close(bounds(&mut s, a), [0.0, 0.0, 0.0, 10.0, 10.0, 10.0]));
    let shown = s
        .solids()
        .iter()
        .find(|x| x.component == a as u32)
        .and_then(|x| x.scene_id)
        .and_then(|id| s.scene.body(id))
        .map(|b| {
            b.mesh
                .positions
                .iter()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min)
        })
        .unwrap();
    assert!((shown + 20.0).abs() < 1e-9, "{shown}");
    // Trails run from each moved cube's centre to where it is shown.
    let trail = s
        .scene
        .overlays
        .iter()
        .find(|o| o.style == chamfera_render::OverlayStyle::ExplodeTrail)
        .unwrap();
    assert_eq!(trail.lines.len(), 2);
    // A manual offset adds to the automatic one; clearing shows the assembly again.
    let r = s
        .execute(
            "view.explode",
            &json!({"component": m, "offset": {"x": 0, "y": 0, "z": 15}}),
        )
        .unwrap();
    assert_eq!(r["components"].as_array().unwrap().len(), 3);
    s.execute("view.explode_clear", &json!({})).unwrap();
    assert!(s.exploded().is_empty());
}

#[test]
fn motion_keys_survive_save_and_open() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let j = revolute_about_z(&mut s, a, b, json!({}));
    for (t, v) in [(0.0, 0.0), (1.5, 90.0), (3.0, 45.0)] {
        s.execute(
            "motion.add_key",
            &json!({"joint": j, "time": t, "value": v}),
        )
        .unwrap();
    }
    let saved = s.execute("design.save_bytes", &json!({})).unwrap();
    let mut t = Session::new();
    t.execute("design.open_bytes", &json!({"data": saved["data"]}))
        .unwrap();
    assert_eq!(t.doc.motion, s.doc.motion);
    let l = t.execute("motion.list", &json!({})).unwrap();
    assert_eq!(l["times"], json!([0.0, 1.5, 3.0]));
    assert!((l["keys"][1]["value"].as_f64().unwrap() - 90.0).abs() < 1e-9);
    // Deleting the joint drops its keys.
    t.execute("joint.delete", &json!({"joint": j})).unwrap();
    assert!(t.doc.motion.keys.is_empty());
}

/// A planar joint between a 60 x 60 x 4 plate's top (centre (30, 30, 4)) and the bottom
/// of an r 5 puck made at (100, 100) centres the puck on the plate: its axis lands on
/// x = y = 30 (QA user guide pass: the puck's face centre was averaged from its one
/// seam vertex, putting it at (25, 30)).
#[test]
fn a_planar_joint_centres_a_round_puck_on_the_plate() {
    let mut s = Session::new();
    let fa = plate(&mut s, 0.0, 0.0, 60.0, 60.0, 4.0);
    let sk = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 100, "cy": 100, "radius": 5}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let fb = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sk["sketch"], "distance": 3, "profiles": [{"x": 100, "y": 100}]}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let a = new_component(&mut s, "Plate", fa);
    let b = new_component(&mut s, "Puck", fb);
    s.execute("component.ground", &json!({"component": a}))
        .unwrap();
    s.execute(
        "joint.create",
        &json!({"type": "planar", "a": {"face": format!("{fa}:top")},
                "b": {"face": format!("{fb}:bottom")}, "flip": true}),
    )
    .unwrap();
    // The puck's box is x and y 25..35 around the axis, z 4..7 on the top.
    assert!(
        close(bounds(&mut s, b), [25.0, 25.0, 4.0, 35.0, 35.0, 7.0]),
        "{:?}",
        bounds(&mut s, b)
    );
}

/// A linked copy of a jointed component starts where the original is shown, not where
/// it was before the joint moved it (QA user guide pass). B stacked on A by a rigid
/// joint spans x 0..40, y 10..20, z 4..8 (see
/// `a_face_origin_follows_the_face_and_a_pick_names_the_component`); its copy moved
/// 50 along x spans x 50..90 at the same y and z.
#[test]
fn a_linked_copy_of_a_jointed_component_starts_where_it_is_shown() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let fa = s.doc.assembly.component(a as u32).unwrap().features[0];
    let fb = s.doc.assembly.component(b as u32).unwrap().features[0];
    s.execute(
        "joint.create",
        &json!({"type": "rigid", "a": {"face": format!("{fa}:top")},
                "b": {"face": format!("{fb}:bottom")}, "flip": true}),
    )
    .unwrap();
    let copy = s
        .execute(
            "component.copy",
            &json!({"component": b, "transform": {"translate": {"x": 50, "y": 0, "z": 0}}}),
        )
        .unwrap()["component"]
        .as_u64()
        .unwrap();
    assert!(
        close(bounds(&mut s, copy), [50.0, 10.0, 4.0, 90.0, 20.0, 8.0]),
        "{:?}",
        bounds(&mut s, copy)
    );
}

fn limits_of(s: &Session, j: &Value) -> Option<[f64; 2]> {
    let id = u32::try_from(j.as_u64().unwrap()).unwrap();
    s.doc.assembly.joint(id).unwrap().limits
}

/// joint.edit sets new limits (QA user guide pass: joint limits could not be changed).
/// A slider at 15 mm moves B's 0..40 plate to 15..55; limits 0..10 move the value to the
/// nearer limit, 10, so B spans 10..50. Limits of -5..30 then let 25 through (B at
/// 25..65); `limits: false` removes them, and undo puts them back.
#[test]
fn editing_a_joints_limits_keeps_its_value_inside_them() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let r = s
        .execute(
            "joint.create",
            &json!({"type": "slider", "a": at_origin(a, [1.0, 0.0, 0.0]),
                    "b": at_origin(b, [1.0, 0.0, 0.0]), "value": 15}),
        )
        .unwrap();
    let j = r["joint"].clone();
    assert!(close(bounds(&mut s, b), [15.0, 0.0, 0.0, 55.0, 10.0, 4.0]));
    let r = s
        .execute("joint.edit", &json!({"joint": j, "min": 0, "max": 10}))
        .unwrap();
    assert_eq!(r["limits"], json!([0.0, 10.0]));
    assert!((r["value"].as_f64().unwrap() - 10.0).abs() < 1e-9);
    assert!(close(bounds(&mut s, b), [10.0, 0.0, 0.0, 50.0, 10.0, 4.0]));
    s.execute(
        "joint.edit",
        &json!({"joint": j, "min": -5, "max": "30 mm"}),
    )
    .unwrap();
    s.execute("joint.set_value", &json!({"joint": j, "value": 25}))
        .unwrap();
    assert!(close(bounds(&mut s, b), [25.0, 0.0, 0.0, 65.0, 10.0, 4.0]));
    let r = s
        .execute(
            "joint.edit",
            &json!({"joint": j, "limits": false, "name": "Rail"}),
        )
        .unwrap();
    assert!(r["limits"].is_null());
    assert_eq!(r["name"], "Rail");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(limits_of(&s, &j), Some([-5.0, 30.0]));
    // Refused, changing nothing: one limit only, min above max, nothing to change, both
    // new limits and none, an empty name.
    for bad in [
        json!({"joint": j, "min": 0}),
        json!({"joint": j, "min": 5, "max": 1}),
        json!({"joint": j}),
        json!({"joint": j, "min": 0, "max": 1, "limits": false}),
        json!({"joint": j, "name": ""}),
    ] {
        assert!(s.execute("joint.edit", &bad).is_err(), "{bad}");
    }
    assert_eq!(limits_of(&s, &j), Some([-5.0, 30.0]));
}

/// A revolute joint's limits are typed in degrees: 0..45 with the joint at 90 turns it
/// back to 45 (the reply in degrees, stored as pi / 4).
#[test]
fn a_revolute_joints_limits_are_in_degrees() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    let r = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "a": at_origin(a, [0.0, 0.0, 1.0]),
                    "b": at_origin(b, [0.0, 0.0, 1.0]), "value": 90}),
        )
        .unwrap();
    let j = r["joint"].clone();
    let r = s
        .execute("joint.edit", &json!({"joint": j, "min": 0, "max": 45}))
        .unwrap();
    assert!((r["value"].as_f64().unwrap() - 45.0).abs() < 1e-9);
    let id = u32::try_from(j.as_u64().unwrap()).unwrap();
    let stored = s.doc.assembly.joint(id).unwrap().value;
    assert!((stored - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
}

/// A linked copy's bodies are named after the copy (QA user guide pass: they reused the
/// original's names, so one name meant two bodies). Peg's body Extrude1 shows in its
/// copy Peg:2 as "Extrude1 (Peg:2)"; the original keeps its name.
#[test]
fn a_linked_copys_bodies_have_names_of_their_own() {
    let mut s = Session::new();
    let f = plate(&mut s, 0.0, 0.0, 10.0, 10.0, 10.0);
    let c = new_component(&mut s, "Peg", f);
    let copy = s
        .execute("component.copy", &json!({"component": c}))
        .unwrap();
    assert_eq!(copy["name"], "Peg:2");
    let mut names: Vec<String> = s.solids().iter().map(|x| x.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Extrude1", "Extrude1 (Peg:2)"]);
    // The name finds the copy's own body: 10 x 10 x 10 is 1000 mm³.
    let m = s
        .execute("inspect.mass", &json!({"body": "Extrude1 (Peg:2)"}))
        .unwrap();
    assert!((m["volume"].as_f64().unwrap() - 1000.0).abs() < 1e-6);
    assert_eq!(m["solid"], "Extrude1 (Peg:2)");
}

/// A component's world transform from `component.list`: its matrix rows and translation.
fn world_of(s: &mut Session, id: u64) -> ([[f64; 3]; 3], [f64; 3]) {
    let w = component(s, id)["world"].clone();
    let m = |i: usize, j: usize| w["matrix"][i][j].as_f64().unwrap();
    let t = |k: &str| w["translation"][k].as_f64().unwrap();
    (
        [
            [m(0, 0), m(0, 1), m(0, 2)],
            [m(1, 0), m(1, 1), m(1, 2)],
            [m(2, 0), m(2, 1), m(2, 2)],
        ],
        [t("x"), t("y"), t("z")],
    )
}

/// A component's own point `p` in the world.
fn world_point(s: &mut Session, id: u64, p: [f64; 3]) -> [f64; 3] {
    let (m, t) = world_of(s, id);
    let row = |i: usize| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + t[i];
    [row(0), row(1), row(2)]
}

fn point_on(component: u64, x: f64) -> Value {
    json!({"component": component, "point": {"x": x, "y": 0, "z": 0},
           "axis": {"x": 0, "y": 0, "z": 1}})
}

/// The rocker angle of a four-bar linkage from the Freudenstein equation (Freudenstein,
/// "Approximate synthesis of four-bar linkages", Trans. ASME 77, 1955):
/// `K1 cos θ4 − K2 cos θ2 + K3 = cos(θ2 − θ4)` with `K1 = d/a`, `K2 = d/c`,
/// `K3 = (a² − b² + c² + d²) / 2ac`. With `t = tan(θ4 / 2)` it is the quadratic
/// `A t² + B t + C = 0`, `A = cos θ2 − K1 − K2 cos θ2 + K3`, `B = −2 sin θ2`,
/// `C = K1 − (K2 + 1) cos θ2 + K3`; the open branch (coupler above the ground link)
/// is the root with the minus sign.
fn freudenstein(a: f64, b: f64, c: f64, d: f64, theta2: f64) -> f64 {
    let (k1, k2, k3) = (
        d / a,
        d / c,
        (a * a - b * b + c * c + d * d) / (2.0 * a * c),
    );
    let (s2, c2) = theta2.sin_cos();
    let qa = c2 - k1 - k2 * c2 + k3;
    let qb = -2.0 * s2;
    let qc = k1 - (k2 + 1.0) * c2 + k3;
    let t = (-qb - (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
    2.0 * t.atan()
}

/// The rocker's angle in the world, degrees: the direction of its own x axis (from its
/// pivot to its tip).
fn rocker_angle(s: &mut Session, rocker: u64) -> f64 {
    let (m, _) = world_of(s, rocker);
    m[1][0].atan2(m[0][0]).to_degrees()
}

/// Builds a four-bar linkage of empty components: Ground (grounded), Crank (a = 40),
/// Coupler (b = 120) and Rocker (c = 80), ground pivots 100 apart (d), joined by four
/// revolute joints about z. Each link lies along its own x axis from its first pivot.
/// The joints start near the open branch at a crank angle of 90 degrees; the solve
/// closes the loop exactly. Returns (crank joint, rocker joint, coupler id, rocker id).
fn four_bar(s: &mut Session) -> (u64, u64, u64, u64) {
    let mut part = |name: &str| -> u64 {
        s.execute("component.new", &json!({"name": name, "parent": 0}))
            .unwrap()["component"]
            .as_u64()
            .unwrap()
    };
    let (g, crank, coupler, rocker) = (
        part("Ground"),
        part("Crank"),
        part("Coupler"),
        part("Rocker"),
    );
    s.execute("component.ground", &json!({"component": g}))
        .unwrap();
    let mut joint = |a: Value, b: Value, value: f64| -> u64 {
        s.execute(
            "joint.create",
            &json!({"type": "revolute", "a": a, "b": b, "value": value}),
        )
        .unwrap()["joint"]
            .as_u64()
            .unwrap()
    };
    // At θ2 = 90°: Freudenstein gives θ4 = 80.26°, so the coupler's far end is at
    // (100 + 80 cos θ4, 80 sin θ4) = (113.53, 78.85) and the coupler, from the crank's
    // tip (0, 40), points at θ3 = atan2(38.85, 113.53) = 18.89°. Joint values are
    // relative turns: crank 90, coupler θ3 − θ2 = −71.11, rocker on the coupler
    // θ4 − θ3 = 61.37, rocker on the ground θ4 = 80.26; whole degrees are given and the
    // solve does the rest.
    let input = joint(point_on(g, 0.0), point_on(crank, 0.0), 90.0);
    joint(point_on(crank, 40.0), point_on(coupler, 0.0), -71.0);
    joint(point_on(coupler, 120.0), point_on(rocker, 80.0), 61.0);
    let output = joint(point_on(g, 100.0), point_on(rocker, 0.0), 80.0);
    (input, output, coupler, rocker)
}

#[test]
fn a_four_bar_linkage_follows_the_freudenstein_equation() {
    let mut s = Session::new();
    let (input, output, coupler, rocker) = four_bar(&mut s);
    let (a, b, c, d) = (40.0, 120.0, 80.0, 100.0);
    for theta2 in [90.0_f64, 60.0, 30.0, 0.0, -45.0] {
        s.execute("joint.set_value", &json!({"joint": input, "value": theta2}))
            .unwrap();
        let expected = freudenstein(a, b, c, d, theta2.to_radians()).to_degrees();
        let measured = rocker_angle(&mut s, rocker);
        assert!(
            (measured - expected).abs() < 1e-6,
            "θ2 = {theta2}: rocker at {measured}°, Freudenstein says {expected}°"
        );
        // The loop is closed: the coupler's end (120, 0, 0) is the rocker's tip.
        let p = world_point(&mut s, coupler, [120.0, 0.0, 0.0]);
        let q = world_point(&mut s, rocker, [80.0, 0.0, 0.0]);
        let gap = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        assert!(gap < 1e-7, "θ2 = {theta2}: the loop is open by {gap} mm");
    }
    // Hand values from the quadratic (see `freudenstein`): at 90° θ4 = 80.26°, at 60°
    // A = −2.0625, B = −√3, C = 1.9375, t = (√3 − √(3 + 15.984)) / −4.125 = 0.6364,
    // θ4 = 2 atan(0.6364) = 64.94°.
    assert!((freudenstein(a, b, c, d, 90f64.to_radians()).to_degrees() - 80.26).abs() < 0.01);
    assert!((freudenstein(a, b, c, d, 60f64.to_radians()).to_degrees() - 64.94).abs() < 0.01);
    // The crank is the loop's input; the rocker joint is solved, not driven.
    let list = s.execute("component.list", &json!({})).unwrap();
    let joints = list["joints"].as_array().unwrap();
    let driven =
        |id: u64| joints.iter().find(|j| j["joint"] == json!(id)).unwrap()["driven"].clone();
    assert_eq!(driven(input), json!(true));
    assert_eq!(driven(output), json!(false));
    let loops = list["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 1);
    // One degree of freedom (the crank), held: none left.
    assert_eq!(loops[0]["freedom"], json!(0));
    assert_eq!(loops[0]["unknowns"], json!(3));
}

#[test]
fn an_over_constrained_loop_is_refused_with_its_joints_named() {
    let mut s = Session::new();
    let (_, _, coupler, _) = four_bar(&mut s);
    let ground = 1;
    let before = s.doc.clone();
    // Pinning the coupler's start to the ground at (50, 50) leaves the linkage no motion
    // with the crank driven, and the crank's tip (0, 40) is not there.
    let e = s
        .execute(
            "joint.create",
            &json!({"type": "revolute", "name": "Pin", "a": {"component": ground,
                    "point": {"x": 50, "y": 50, "z": 0}, "axis": {"x": 0, "y": 0, "z": 1}},
                    "b": point_on(coupler, 0.0)}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("over-constrained"), "{e}");
    assert!(e.contains("\"Pin\"") && e.contains("\"Joint1\""), "{e}");
    assert_eq!(s.doc, before);
}

#[test]
fn an_as_built_joint_keeps_placements_bit_for_bit_and_turns_about_its_origin() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    // B turned 30° about z and moved 50 along x: a placement with rounding in it.
    s.execute(
        "component.move",
        &json!({"component": b, "transform": {"rotate": {"axis": {"x": 0, "y": 0, "z": 1},
                "angle": 30}, "translate": {"x": 50, "y": 0, "z": 0}}}),
    )
    .unwrap();
    let before = component(&mut s, b);
    let bounds_before = bounds(&mut s, b);
    let r = s
        .execute(
            "joint.as_built",
            &json!({"type": "revolute", "a": a, "b": b,
                    "origin": {"point": {"x": 50, "y": 0, "z": 0}, "axis": {"x": 0, "y": 0, "z": 1}}}),
        )
        .unwrap();
    assert_eq!(r["as_built"], json!(true));
    let after = component(&mut s, b);
    // Bit for bit: the stored placement, the solved world transform and the bounds.
    assert_eq!(after["placement"], before["placement"]);
    assert_eq!(after["world"], before["world"]);
    assert_eq!(bounds(&mut s, b), bounds_before);
    // Driven to −30° about z through (50, 0, 0), B's turn is undone: its plate lies at
    // x 50..90, y 0..10 (the move alone).
    let j = r["joint"].clone();
    s.execute("joint.set_value", &json!({"joint": j, "value": -30}))
        .unwrap();
    assert!(close(bounds(&mut s, b), [50.0, 0.0, 0.0, 90.0, 10.0, 4.0]));
    // And back at 0 it is where it was built, bit for bit.
    s.execute("joint.set_value", &json!({"joint": j, "value": 0}))
        .unwrap();
    assert_eq!(component(&mut s, b)["world"], before["world"]);
    // It survives save and open, still keeping B where it was built.
    let saved = s.execute("design.save_bytes", &json!({})).unwrap();
    let mut t = Session::new();
    t.execute("design.open_bytes", &json!({"data": saved["data"]}))
        .unwrap();
    assert_eq!(t.doc.assembly.joints.len(), 1);
    assert!(t.doc.assembly.joints[0].as_built);
    assert_eq!(component(&mut t, b)["world"], before["world"]);
}

#[test]
fn a_rigid_group_moves_as_one_and_ungrouping_leaves_it_in_place() {
    let mut s = Session::new();
    let (a, b) = two_plates(&mut s);
    // C: a 2 mm cube at x 60..62, y 0..2.
    let fc = plate(&mut s, 60.0, 0.0, 62.0, 2.0, 2.0);
    let c = new_component(&mut s, "C", fc);
    let g = s
        .execute("component.rigid_group", &json!({"components": [b, c]}))
        .unwrap();
    assert_eq!(g["name"], "Group1");
    // Nothing moved when grouped.
    assert!(close(bounds(&mut s, c), [60.0, 0.0, 0.0, 62.0, 2.0, 2.0]));
    // Turning B a quarter about z through the origin takes C with it:
    // (x, y) -> (−y, x), so C is at x −2..0, y 60..62.
    s.execute(
        "joint.create",
        &json!({"type": "revolute", "a": at_origin(a, [0.0, 0.0, 1.0]),
                "b": at_origin(b, [0.0, 0.0, 1.0]), "value": 90}),
    )
    .unwrap();
    assert!(close(bounds(&mut s, c), [-2.0, 60.0, 0.0, 0.0, 62.0, 2.0]));
    assert!(close(bounds(&mut s, b), [-10.0, 0.0, 0.0, 0.0, 40.0, 4.0]));
    // C is held by the group: moving it alone is refused, naming the group.
    let e = s
        .execute(
            "component.move",
            &json!({"component": c, "transform": {"translate": {"x": 1, "y": 0, "z": 0}}}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("Group1"), "{e}");
    // Ungrouped, C stays where the group held it.
    s.execute("component.ungroup", &json!({"group": g["group"]}))
        .unwrap();
    assert!(close(bounds(&mut s, c), [-2.0, 60.0, 0.0, 0.0, 62.0, 2.0]));
    assert!(s.doc.assembly.groups.is_empty());
    // A component in two groups, or a group of one, is refused.
    s.execute("component.rigid_group", &json!({"components": [b, c]}))
        .unwrap();
    assert!(
        s.execute("component.rigid_group", &json!({"components": [c, a]}))
            .is_err()
    );
    assert!(
        s.execute("component.rigid_group", &json!({"components": [a]}))
            .is_err()
    );
}

#[test]
fn the_bill_of_materials_counts_nested_linked_copies() {
    let mut s = Session::new();
    // Axle (top level) owns a 100 x 10 x 10 shaft; Wheel, inside Axle, a 20 x 20 x 5
    // plate; Bolt, top level, a 2 x 2 x 10 pin.
    let shaft = plate(&mut s, 0.0, 0.0, 100.0, 10.0, 10.0);
    let wheel = plate(&mut s, 0.0, 20.0, 20.0, 40.0, 5.0);
    let bolt = plate(&mut s, 50.0, 50.0, 52.0, 52.0, 10.0);
    let axle = new_component(&mut s, "Axle", shaft);
    let w = s
        .execute(
            "component.new",
            &json!({"name": "Wheel", "from_bodies": [wheel], "parent": axle}),
        )
        .unwrap()["component"]
        .as_u64()
        .unwrap();
    new_component(&mut s, "Bolt", bolt);
    // Wheel: two copies of it, and a copy of a copy (which copies the original), all
    // inside Axle: four wheels. Axle: one copy, which shows the shaft only (a linked
    // copy shows its original's own bodies): two shafts.
    let w2 = s
        .execute("component.copy", &json!({"component": w}))
        .unwrap();
    s.execute("component.copy", &json!({"component": w}))
        .unwrap();
    s.execute("component.copy", &json!({"component": w2["component"]}))
        .unwrap();
    s.execute("component.copy", &json!({"component": axle}))
        .unwrap();
    let r = s.execute("bom.list", &json!({})).unwrap();
    let rows = r["rows"].as_array().unwrap();
    let got: Vec<(String, u64, f64)> = rows
        .iter()
        .map(|x| {
            (
                x["part"].as_str().unwrap().to_owned(),
                x["quantity"].as_u64().unwrap(),
                x["volume"].as_f64().unwrap(),
            )
        })
        .collect();
    let want = [
        ("Axle", 2, 10_000.0),
        ("Wheel", 4, 2_000.0),
        ("Bolt", 1, 40.0),
    ];
    assert_eq!(got.len(), want.len(), "{got:?}");
    for ((part, q, v), (wp, wq, wv)) in got.iter().zip(want) {
        assert_eq!((part.as_str(), *q), (wp, wq));
        assert!((v - wv).abs() < 1e-6, "{part}: {v}");
    }
    assert_eq!(r["parts"], json!(7));
    // The quantities match what is shown: each instance shows its part's bodies.
    let shown: u64 = rows
        .iter()
        .map(|x| x["quantity"].as_u64().unwrap() * x["bodies"].as_u64().unwrap())
        .sum();
    assert_eq!(shown, s.solids().len() as u64);
    // CSV: a header and a line per row.
    let csv = s.execute("bom.export_csv", &json!({})).unwrap();
    let text = csv["csv"].as_str().unwrap();
    assert!(
        text.starts_with("item,part,quantity,material,volume_mm3"),
        "{text}"
    );
    assert!(text.contains("\r\n2,Wheel,4,,2000.000,,\r\n"), "{text}");
}

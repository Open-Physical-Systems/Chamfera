//! Tests of the body tools: combine, split body, split face, thicken and remove, each
//! against volumes and areas worked out by hand.

use super::*;
use serde_json::{Value, json};
use std::f64::consts::PI;

/// A box from (x1, y1, 0) to (x2, y2, h) as a new body; `h` may be an expression.
fn block(s: &mut Session, corners: [f64; 4], h: Value) -> u64 {
    let [x1, y1, x2, y2] = corners;
    let sketch = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sketch, "profiles": [{"x": (x1 + x2) / 2.0, "y": (y1 + y2) / 2.0}],
                "distance": h}),
    )
    .unwrap()["feature"]
        .as_u64()
        .unwrap()
}

fn volume(s: &Session, name: &str) -> f64 {
    s.solids()
        .iter()
        .find(|x| x.name == name)
        .unwrap_or_else(|| panic!("no solid {name}"))
        .body
        .volume()
        .unwrap()
}

fn total_volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

fn names(s: &Session) -> Vec<String> {
    s.solids().iter().map(|x| x.name.clone()).collect()
}

/// Two 10 mm cubes, A at x 0..10 and B at x 5..15 (overlap 5 x 10 x 10 = 500).
fn two_cubes(s: &mut Session) {
    block(s, [0.0, 0.0, 10.0, 10.0], json!(10));
    block(s, [5.0, 0.0, 15.0, 10.0], json!(10));
}

#[test]
fn combining_two_overlapping_cubes_joins_cuts_and_intersects() {
    // Join: 1000 + 1000 - 500 = 1500; cut: 1000 - 500 = 500; intersect: 500.
    for (op, want) in [("join", 1500.0), ("cut", 500.0), ("intersect", 500.0)] {
        let mut s = Session::new();
        two_cubes(&mut s);
        let r = s
            .execute(
                "solid.combine",
                &json!({"target": "Extrude1", "tools": ["Extrude2"], "operation": op}),
            )
            .unwrap();
        assert!(r["error"].is_null());
        assert_eq!(names(&s), vec!["Extrude1"], "{op}: the tool is used up");
        assert!((volume(&s, "Extrude1") - want).abs() < 1e-6, "{op}");
        // The combine is one undo step.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.solids().len(), 2);
    }
}

#[test]
fn keep_tools_keeps_the_tool_bodies() {
    // A third cube C at x -5..5. A ∪ B ∪ C spans x -5..15: 20 x 10 x 10 = 2000, and B
    // and C stay: 3 bodies.
    let mut s = Session::new();
    two_cubes(&mut s);
    block(&mut s, [-5.0, 0.0, 5.0, 10.0], json!(10));
    s.execute(
        "solid.combine",
        &json!({"target": "Extrude1", "tools": ["Extrude2", "Extrude3"], "keep_tools": true}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 3);
    assert!((volume(&s, "Extrude1") - 2000.0).abs() < 1e-6);
    assert!((volume(&s, "Extrude2") - 1000.0).abs() < 1e-6);
    assert!((volume(&s, "Extrude3") - 1000.0).abs() < 1e-6);
}

#[test]
fn combine_takes_the_selected_bodies_target_first() {
    let mut s = Session::new();
    two_cubes(&mut s);
    // Select B, then A: B is the target, A the tool; B - A = x 10..15: 500.
    let id = |s: &Session, n: &str| {
        s.solids()
            .iter()
            .find(|x| x.name == n)
            .and_then(|x| x.scene_id)
            .unwrap()
    };
    s.scene.selected = vec![id(&s, "Extrude2"), id(&s, "Extrude1")];
    s.execute("solid.combine", &json!({"operation": "cut"}))
        .unwrap();
    assert_eq!(names(&s), vec!["Extrude2"]);
    assert!((volume(&s, "Extrude2") - 500.0).abs() < 1e-6);
}

#[test]
fn combine_follows_a_parameter_change_upstream() {
    // A is 10 x 10 x h; B is x 5..15, 10 tall. A ∪ B = 100 h + 1000 - 50 min(h, 10):
    // h = 10 gives 1500, h = 20 gives 2000 + 1000 - 500 = 2500.
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "h", "expression": "10 mm"}))
        .unwrap();
    block(&mut s, [0.0, 0.0, 10.0, 10.0], json!("h"));
    block(&mut s, [5.0, 0.0, 15.0, 10.0], json!(10));
    let r = s
        .execute(
            "solid.combine",
            &json!({"target": "Extrude1", "tools": ["Extrude2"]}),
        )
        .unwrap();
    let combine = r["feature"].as_u64().unwrap();
    assert!((volume(&s, "Extrude1") - 1500.0).abs() < 1e-6);
    s.execute("param.set", &json!({"name": "h", "expression": "20 mm"}))
        .unwrap();
    assert!(s.feature_errors().is_empty());
    assert!((volume(&s, "Extrude1") - 2500.0).abs() < 1e-6);
    // Edited to a cut: A - B = 100 h - 50 min(h, 10) = 2000 - 500 = 1500.
    s.execute(
        "feature.edit",
        &json!({"feature": combine, "operation": "cut", "keep_tools": true}),
    )
    .unwrap();
    assert!((volume(&s, "Extrude1") - 1500.0).abs() < 1e-6);
    assert_eq!(s.solids().len(), 2);
}

#[test]
fn refused_body_tools_change_nothing() {
    let mut s = Session::new();
    two_cubes(&mut s);
    // A small cube inside A: cutting A from it leaves nothing.
    block(&mut s, [2.0, 2.0, 4.0, 4.0], json!(4));
    let before = s.doc.clone();
    let bad = [
        (
            "solid.combine",
            json!({"target": "Extrude1", "tools": ["Nope"]}),
        ),
        (
            "solid.combine",
            json!({"target": "Extrude1", "tools": ["Extrude1"]}),
        ),
        (
            "solid.combine",
            json!({"target": "Extrude1", "tools": ["Extrude2"], "operation": "new_body"}),
        ),
        (
            "solid.combine",
            json!({"target": "Extrude3", "tools": ["Extrude1"], "operation": "cut"}),
        ),
        ("solid.combine", json!({"target": "Extrude1"})),
        (
            "solid.split_body",
            json!({"bodies": ["Extrude1"], "plane": "yz"}),
        ),
        ("solid.split_body", json!({"bodies": ["Extrude1"]})),
        (
            "solid.split_body",
            json!({"bodies": ["Nope"], "plane": "xz"}),
        ),
        (
            "solid.split_face",
            json!({"faces": ["2:top"], "plane": "xy"}),
        ),
        (
            "solid.split_face",
            json!({"faces": ["2:nothing"], "plane": "yz"}),
        ),
        ("solid.split_face", json!({"faces": ["2:top"]})),
        ("solid.thicken", json!({"faces": ["2:top"], "thickness": 0})),
        (
            "solid.thicken",
            json!({"faces": ["2:top"], "thickness": 1, "sides": "up"}),
        ),
        (
            "solid.thicken",
            json!({"faces": ["2:gone"], "thickness": 1}),
        ),
        ("solid.remove", json!({"bodies": ["Nope"]})),
        ("solid.remove", json!({})),
    ];
    for (id, p) in bad {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
        assert_eq!(s.doc, before, "{id} {p} changed the design");
    }
    assert_eq!(s.solids().len(), 3);
    // Nothing was added to undo: the last step is still the third extrude.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.solids().len(), 2);
}

#[test]
fn splitting_the_box_at_x_20_gives_6000_and_9000() {
    // The 50 x 30 x 10 box, split by a plane offset 20 from YZ: 20 * 30 * 10 = 6000
    // behind it, 30 * 30 * 10 = 9000 in front; the area grows by 2 * 30 * 10.
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 50.0, 30.0], json!(10));
    let before = s.solids()[0].body.area().unwrap();
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "yz", "distance": 20}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let r = s
        .execute(
            "solid.split_body",
            &json!({"bodies": ["Extrude1"], "plane": plane}),
        )
        .unwrap();
    let split = r["feature"].as_u64().unwrap();
    assert_eq!(names(&s), vec!["Extrude1", "SplitBody1"]);
    assert!((volume(&s, "Extrude1") - 6000.0).abs() < 1e-6);
    assert!((volume(&s, "SplitBody1") - 9000.0).abs() < 1e-6);
    let after: f64 = s.solids().iter().map(|x| x.body.area().unwrap()).sum();
    assert!((after - before - 600.0).abs() < 1e-6, "{after}");
    // Both halves keep the box's faces and gain `split plane <id>`.
    let face = format!("{split}:split plane {plane}");
    for x in s.solids() {
        let names: Vec<&str> = x
            .body
            .faces()
            .into_iter()
            .filter_map(|f| x.body.face_name(f).ok().flatten())
            .collect();
        assert!(names.contains(&face.as_str()), "{names:?}");
        assert!(names.contains(&"2:top"), "{names:?}");
    }
    // The plane is used by the split, so deleting it is refused; moving the plane to
    // 30 moves the split: 9000 and 6000.
    assert!(
        s.execute("feature.delete", &json!({"feature": plane}))
            .is_err()
    );
    s.execute("feature.edit", &json!({"feature": plane, "distance": 30}))
        .unwrap();
    assert!((volume(&s, "Extrude1") - 9000.0).abs() < 1e-6);
    assert!((volume(&s, "SplitBody1") - 6000.0).abs() < 1e-6);
    // Split by the XZ origin plane instead: the box lies all on one side (y 0..30).
    assert!(
        s.execute("feature.edit", &json!({"feature": split, "plane": "xz"}))
            .is_err()
    );
}

#[test]
fn split_body_uses_the_selection_and_a_flat_face() {
    // The box, and a cube x 20..30 on top of it as a separate body: split the box by
    // the cube's left face (x = 20, normal -x): the part behind (x > 20) keeps the name.
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 50.0, 30.0], json!(10));
    let cube = block(&mut s, [20.0, 0.0, 30.0, 10.0], json!(10));
    let id = s
        .solids()
        .iter()
        .find(|x| x.name == "Extrude1")
        .and_then(|x| x.scene_id)
        .unwrap();
    s.scene.selected = vec![id];
    let left = s
        .solids()
        .iter()
        .find(|x| x.name == "Extrude2")
        .unwrap()
        .body
        .faces()
        .into_iter()
        .map(|f| {
            let b = &s.solids()[1].body;
            (
                b.face_name(f).unwrap().unwrap().to_owned(),
                crate::solids::face_plane(b, f).unwrap(),
            )
        })
        .find(|(_, p)| p.z().x < -0.99)
        .unwrap()
        .0;
    let role = left.split_once(':').unwrap().1.to_owned();
    s.execute(
        "solid.split_body",
        &json!({"plane": {"feature": cube, "face": role}}),
    )
    .unwrap();
    assert!((volume(&s, "Extrude1") - 9000.0).abs() < 1e-6);
    assert!((volume(&s, "SplitBody1") - 6000.0).abs() < 1e-6);
}

#[test]
fn splitting_the_top_face_gives_seven_faces_and_the_same_volume() {
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 50.0, 30.0], json!(10));
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "yz", "distance": 20}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let r = s
        .execute(
            "solid.split_face",
            &json!({"faces": ["2:top"], "plane": plane}),
        )
        .unwrap();
    let split = r["feature"].as_u64().unwrap();
    assert_eq!(s.solids().len(), 1);
    let b = &s.solids()[0].body;
    assert_eq!(b.faces().len(), 7);
    assert!((b.volume().unwrap() - 15_000.0).abs() < 1e-6);
    // The piece behind the plane keeps `2:top` (20 x 30), the one in front is new
    // (30 x 30).
    let area = |name: &str| {
        b.faces()
            .into_iter()
            .find(|f| b.face_name(*f).unwrap() == Some(name))
            .map(|f| b.face_area(f).unwrap())
            .unwrap_or_else(|| panic!("no face {name}"))
    };
    assert!((area("2:top") - 600.0).abs() < 1e-6);
    let front = format!("{split}:split plane {plane} of 2:top");
    assert!((area(&front) - 900.0).abs() < 1e-6);
    // Moving the plane to x = 40 moves the cut: 40 x 30 and 10 x 30.
    s.execute("feature.edit", &json!({"feature": plane, "distance": 40}))
        .unwrap();
    let b = &s.solids()[0].body;
    let top = b
        .faces()
        .into_iter()
        .find(|f| b.face_name(*f).unwrap() == Some("2:top"))
        .unwrap();
    assert!((b.face_area(top).unwrap() - 1200.0).abs() < 1e-6);
}

#[test]
fn a_sketch_line_splits_the_top_face_square_to_its_sketch() {
    // A line at x = 20 on the XY plane, carried along z: the same cut as above.
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 50.0, 30.0], json!(10));
    let sketch = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap()["sketch"]
        .as_u64()
        .unwrap();
    let line = s
        .execute(
            "sketch.line",
            &json!({"x1": 20, "y1": -5, "x2": 20, "y2": 40}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.split_face",
        &json!({"faces": ["2:top"], "sketch": sketch, "line": line}),
    )
    .unwrap();
    let b = &s.solids()[0].body;
    assert_eq!(b.faces().len(), 7);
    assert!((b.volume().unwrap() - 15_000.0).abs() < 1e-6);
    // The sketch is used by the split now.
    assert!(
        s.execute("sketch.delete", &json!({"sketch": sketch}))
            .is_err()
    );
}

#[test]
fn thickening_the_top_face_by_3_gives_4500() {
    // 50 x 30 x 3 = 4500, from z = 10 to 13, as a new body.
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 50.0, 30.0], json!(10));
    let r = s
        .execute(
            "solid.thicken",
            &json!({"faces": ["2:top"], "thickness": 3}),
        )
        .unwrap();
    let id = r["feature"].as_u64().unwrap();
    assert!((volume(&s, "Thicken1") - 4500.0).abs() < 1e-6);
    let bb = s
        .solids()
        .iter()
        .find(|x| x.name == "Thicken1")
        .unwrap()
        .body
        .bounding_box()
        .unwrap();
    assert!((bb.min.z - 10.0).abs() < 1e-9 && (bb.max.z - 13.0).abs() < 1e-9);
    // Both sides: 6 mm in all, z 7..13; edited to 5 one side: 7500.
    s.execute(
        "feature.edit",
        &json!({"feature": id, "sides": "both_sides"}),
    )
    .unwrap();
    assert!((volume(&s, "Thicken1") - 9000.0).abs() < 1e-6);
    s.execute(
        "feature.edit",
        &json!({"feature": id, "sides": "one_side", "thickness": 5}),
    )
    .unwrap();
    assert!((volume(&s, "Thicken1") - 7500.0).abs() < 1e-6);
    // Joined onto the box: 15000 + 7500 in one body.
    s.execute("feature.edit", &json!({"feature": id, "operation": "join"}))
        .unwrap();
    assert_eq!(s.solids().len(), 1);
    assert!((total_volume(&s) - 22_500.0).abs() < 1e-6);
}

/// A top face split by a cut into two pieces that share its name (`2:top#1`, `2:top#2`)
/// thickens both for the whole name: a 40 x 20 x 10 block with a 4 mm slot across the
/// top at x 18..22 leaves two 18 x 20 pieces, each 18 x 20 x 2 = 720 thickened.
#[test]
fn thickening_a_whole_name_thickens_every_piece_of_a_cut_face() {
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 40.0, 20.0], json!(10));
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "xy", "distance": 10}),
        )
        .unwrap()["feature"]
        .clone();
    let sketch = s
        .execute("sketch.create", &json!({"plane": plane}))
        .unwrap()["sketch"]
        .clone();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 18, "y1": -5, "x2": 22, "y2": 25}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sketch, "profiles": [{"x": 20, "y": 10}], "distance": -2,
                "operation": "cut"}),
    )
    .unwrap();
    s.execute(
        "solid.thicken",
        &json!({"faces": ["2:top"], "thickness": 2}),
    )
    .unwrap();
    assert!((volume(&s, "Thicken1 (1)") - 720.0).abs() < 1e-6);
    assert!((volume(&s, "Thicken1 (2)") - 720.0).abs() < 1e-6);
}

/// QA (user guide pass): after Split Face, `2:top` names only the piece behind the
/// plane; the piece in front is renamed `4:split plane 3 of 2:top`. A whole-name thicken
/// should take both (40 x 20 x 2 = 1600), but a UI pick of the back piece also gives
/// `2:top`, so widening the name would thicken both pieces there too.
#[test]
#[ignore = "needs a naming decision: Split Face renames the front piece, so `2:top` is one piece (QA guide pass, solids 4)"]
fn thickening_the_whole_name_of_a_split_face_thickens_both_pieces() {
    let mut s = Session::new();
    block(&mut s, [0.0, 0.0, 40.0, 20.0], json!(10));
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "yz", "distance": 20}),
        )
        .unwrap()["feature"]
        .clone();
    s.execute(
        "solid.split_face",
        &json!({"faces": ["2:top"], "plane": plane}),
    )
    .unwrap();
    s.execute(
        "solid.thicken",
        &json!({"faces": ["2:top"], "thickness": 2}),
    )
    .unwrap();
    let thickened: f64 = s
        .solids()
        .iter()
        .filter(|x| x.name.starts_with("Thicken1"))
        .map(|x| x.body.volume().unwrap())
        .sum();
    assert!((thickened - 1600.0).abs() < 1e-6, "{thickened}");
}

#[test]
fn thickening_a_cylinder_face_by_2_outward_gives_a_ring() {
    // A cylinder r = 10, h = 20; its side thickened 2 outward: π (12² − 10²) 20.
    let mut s = Session::new();
    let sketch = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 10}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let f = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sketch, "profiles": [{"x": 0, "y": 0}], "distance": 20}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let b = &s.solids()[0].body;
    let side = b
        .faces()
        .into_iter()
        .filter_map(|x| b.face_name(x).unwrap())
        .find(|n| n.contains("side"))
        .unwrap()
        .to_owned();
    assert!(side.starts_with(&format!("{f}:")));
    s.execute("solid.thicken", &json!({"faces": [side], "thickness": 2}))
        .unwrap();
    let want = PI * (144.0 - 100.0) * 20.0;
    assert!((volume(&s, "Thicken1") - want).abs() < 1e-6);
    // Thickening the sphere-free cylinder's top works too (π 10² · 1).
    s.execute(
        "solid.thicken",
        &json!({"faces": [format!("{f}:top")], "thickness": 1}),
    )
    .unwrap();
    assert!((volume(&s, "Thicken2") - PI * 100.0).abs() < 1e-6);
}

#[test]
fn remove_takes_a_body_out_and_keeps_its_feature() {
    let mut s = Session::new();
    two_cubes(&mut s);
    let r = s
        .execute("solid.remove", &json!({"bodies": ["Extrude2"]}))
        .unwrap();
    let remove = r["feature"].as_u64().unwrap();
    assert_eq!(names(&s), vec!["Extrude1"]);
    assert_eq!(s.doc.features.len(), 3, "the extrude is still there");
    // Suppressing the remove brings the body back.
    s.execute(
        "feature.edit",
        &json!({"feature": remove, "suppressed": true}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 2);
    // Bodies belong to splits and removes, side to thickens.
    assert!(
        s.execute(
            "feature.edit",
            &json!({"feature": 2, "bodies": ["Extrude1"]})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "feature.edit",
            &json!({"feature": remove, "sides": "symmetric"})
        )
        .is_err()
    );
}

#[test]
fn body_tools_save_and_open() {
    let mut s = Session::new();
    two_cubes(&mut s);
    s.execute(
        "solid.combine",
        &json!({"target": "Extrude1", "tools": ["Extrude2"], "operation": "intersect"}),
    )
    .unwrap();
    s.execute(
        "solid.thicken",
        &json!({"faces": ["2:top"], "thickness": 2, "sides": "symmetric"}),
    )
    .unwrap();
    let text = s.doc.to_json().unwrap();
    let doc = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(doc.features, s.doc.features);
    // The intersection is x 5..10 (500); its top thickened 1 each way: 5 x 10 x 2 = 100.
    let mut t = Session::new();
    t.replace_document(doc);
    t.rebuild();
    assert!((volume(&t, "Extrude1") - 500.0).abs() < 1e-6);
    assert!((volume(&t, "Thicken1") - 100.0).abs() < 1e-6);
}

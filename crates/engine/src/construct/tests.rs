//! Construction geometry: frames checked against values derived by hand.

use crate::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 1e-9, "{what}: {a} vs {b}");
}

/// `(x, y, z)` of a JSON point or vector.
fn xyz(v: &Value) -> (f64, f64, f64) {
    let f = |k: &str| v[k].as_f64().unwrap_or(f64::NAN);
    (f("x"), f("y"), f("z"))
}

fn close3(v: &Value, want: (f64, f64, f64), what: &str) {
    let (x, y, z) = xyz(v);
    close(x, want.0, &format!("{what}.x"));
    close(y, want.1, &format!("{what}.y"));
    close(z, want.2, &format!("{what}.z"));
}

/// An 80 x 50 plate, `t` thick, on XY from the origin (sketch 1, extrude 2).
fn plate(t: &str) -> Session {
    let mut s = Session::new();
    run(&mut s, "sketch.create", json!({ "plane": "xy" }));
    run(
        &mut s,
        "sketch.rectangle",
        json!({ "x1": 0, "y1": 0, "x2": 80, "y2": 50 }),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": t }),
    );
    s
}

#[test]
fn an_offset_of_15_from_xy_is_at_z_15_and_undo_removes_it() {
    let mut s = Session::new();
    let r = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": "xy", "distance": 15 }),
    );
    assert_eq!(r["name"], "Plane1");
    close3(&r["geometry"]["origin"], (0.0, 0.0, 15.0), "origin");
    close3(&r["geometry"]["normal"], (0.0, 0.0, 1.0), "normal");
    // It is drawn: an outline and a see-through fill.
    assert!(!s.scene.fills.is_empty());
    assert!(
        s.scene
            .overlays
            .iter()
            .any(|o| o.style == chamfera_render::OverlayStyle::Construct && o.lines.len() == 4)
    );
    // Offsetting the offset plane by -20 lands at z = -5.
    let id = r["feature"].as_u64().unwrap();
    let r = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": id, "distance": "-20 mm" }),
    );
    close3(&r["geometry"]["origin"], (0.0, 0.0, -5.0), "second origin");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.features.is_empty() && s.constructs().is_empty());
    assert!(s.scene.fills.is_empty());
}

#[test]
fn the_midplane_of_the_faces_z_0_and_z_10_is_at_z_5() {
    let mut s = plate("10 mm");
    let r = run(
        &mut s,
        "construct.plane_midplane",
        json!({ "planes": [
            { "feature": 2, "face": "bottom" },
            { "feature": 2, "face": "top" },
        ] }),
    );
    // The bottom's outward normal is -z; the midplane faces like the first plane.
    close(r["geometry"]["origin"]["z"].as_f64().unwrap(), 5.0, "z");
    close3(&r["geometry"]["normal"], (0.0, 0.0, -1.0), "normal");
    // The top is not parallel to XZ.
    assert!(
        s.execute(
            "construct.plane_midplane",
            &json!({ "planes": [{ "feature": 2, "face": "top" }, "xz"] }),
        )
        .is_err()
    );
    assert_eq!(s.doc.features.len(), 2, "the failed one changed nothing");
}

#[test]
fn three_points_give_the_right_handed_normal_and_collinear_points_are_refused() {
    let mut s = Session::new();
    let r = run(
        &mut s,
        "construct.plane_three_points",
        json!({ "points": [
            "origin",
            { "x": 1, "y": 0, "z": 0 },
            { "x": 0, "y": 1, "z": 0 },
        ] }),
    );
    // (1,0,0) x (0,1,0) = (0,0,1).
    close3(&r["geometry"]["normal"], (0.0, 0.0, 1.0), "normal");
    close3(&r["geometry"]["x_axis"], (1.0, 0.0, 0.0), "x axis");
    let e = s.execute(
        "construct.plane_three_points",
        &json!({ "points": [
            "origin",
            { "x": 1, "y": 1, "z": 1 },
            { "x": 2, "y": 2, "z": 2 },
        ] }),
    );
    assert!(e.is_err());
    assert_eq!(s.doc.features.len(), 1);
}

#[test]
fn a_plane_at_an_angle_turns_about_its_axis() {
    let mut s = Session::new();
    // About X from XY (normal +z): +90 deg turns +z to x × z = -y.
    let r = run(
        &mut s,
        "construct.plane_angle",
        json!({ "axis": "x", "angle": 90 }),
    );
    close3(&r["geometry"]["normal"], (0.0, -1.0, 0.0), "90 deg");
    // 30 deg: (0, -sin 30, cos 30).
    let r = run(
        &mut s,
        "construct.plane_angle",
        json!({ "axis": "x", "angle": "30 deg" }),
    );
    close3(
        &r["geometry"]["normal"],
        (0.0, -0.5, 3f64.sqrt() / 2.0),
        "30 deg",
    );
    // Editing the angle to 0 gives back XY's normal.
    let id = r["feature"].as_u64().unwrap() as u32;
    run(&mut s, "feature.edit", json!({ "feature": id, "angle": 0 }));
    let Some(super::Constructed::Plane { frame, .. }) = s.constructs().get(&id) else {
        panic!("a plane");
    };
    close(frame.z().z, 1.0, "0 deg");
}

#[test]
fn an_offset_plane_on_a_face_follows_the_face_when_the_extrude_changes() {
    let mut s = plate("10 mm");
    let r = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": { "feature": 2, "face": "top" }, "distance": 5 }),
    );
    let id = r["feature"].as_u64().unwrap() as u32;
    close(
        r["geometry"]["origin"]["z"].as_f64().unwrap(),
        15.0,
        "10 + 5",
    );
    run(
        &mut s,
        "feature.edit",
        json!({ "feature": 2, "distance": "20 mm" }),
    );
    let Some(super::Constructed::Plane { frame, .. }) = s.constructs().get(&id) else {
        panic!("a plane");
    };
    close(frame.origin().z, 25.0, "20 + 5");
    // The plate cannot be deleted while the plane uses its face.
    assert!(
        s.execute("feature.delete", &json!({ "feature": 2 }))
            .is_err()
    );
}

#[test]
fn a_sketch_on_an_offset_plane_extrudes_to_its_volume_and_follows_the_plane() {
    let mut s = Session::new();
    let plane = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": "xy", "distance": 15 }),
    )["feature"]
        .clone();
    let sk = run(&mut s, "sketch.create", json!({ "plane": plane }));
    close3(&sk["origin"], (0.0, 0.0, 15.0), "sketch origin");
    run(
        &mut s,
        "sketch.rectangle",
        json!({ "x1": 0, "y1": 0, "x2": 20, "y2": 10 }),
    );
    run(&mut s, "sketch.finish", json!({}));
    let e = run(
        &mut s,
        "solid.extrude",
        json!({ "sketch": sk["sketch"], "profiles": [{ "x": 5, "y": 5 }], "distance": 5 }),
    );
    // 20 x 10 x 5 = 1000 mm³, from z = 15 to 20.
    close(e["solids"][0]["volume"].as_f64().unwrap(), 1000.0, "volume");
    let b = s.solids()[0].body.bounding_box().unwrap();
    close(b.min.z, 15.0, "bottom");
    close(b.max.z, 20.0, "top");
    // Moving the plane to 30 moves the sketch and the solid with it.
    run(
        &mut s,
        "feature.edit",
        json!({ "feature": plane, "distance": "30 mm" }),
    );
    let b = s.solids()[0].body.bounding_box().unwrap();
    close(b.min.z, 30.0, "moved bottom");
    close(b.max.z, 35.0, "moved top");
    // The plane is in use by the sketch.
    assert!(
        s.execute("feature.delete", &json!({ "feature": plane }))
            .is_err()
    );
    // A design with it survives a save and load.
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back, s.doc);
}

#[test]
fn axes_and_points_from_a_cylinder_its_edges_and_vertices() {
    // A plate 10 thick and a cylinder r = 5 at (40, 25), 20 high, both on XY.
    let mut s = plate("10 mm");
    run(&mut s, "sketch.create", json!({ "plane": "xy" }));
    run(
        &mut s,
        "sketch.circle",
        json!({ "cx": 40, "cy": 25, "radius": 5 }),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({ "sketch": 3, "profiles": [{ "x": 40, "y": 25 }], "distance": 20 }),
    );
    let r = run(
        &mut s,
        "construct.axis_cylinder",
        json!({ "face": { "feature": 4, "face": "side 0" } }),
    );
    let (x, y, _) = xyz(&r["geometry"]["origin"]);
    close(x, 40.0, "axis x");
    close(y, 25.0, "axis y");
    close(
        r["geometry"]["direction"]["z"].as_f64().unwrap().abs(),
        1.0,
        "along z",
    );
    // The centre of the circle where the top meets the side: (40, 25, 20).
    let r = run(
        &mut s,
        "construct.point_centre",
        json!({ "edge": { "faces": [
            { "feature": 4, "face": "top" },
            { "feature": 4, "face": "side 0" },
        ] } }),
    );
    close3(&r["geometry"]["point"], (40.0, 25.0, 20.0), "centre");
    // A straight edge of the plate's top lies at z = 10, along x or y.
    let r = run(
        &mut s,
        "construct.axis_edge",
        json!({ "edge": { "faces": [
            { "feature": 2, "face": "top" },
            { "feature": 2, "face": "side 0" },
        ] } }),
    );
    close(
        r["geometry"]["origin"]["z"].as_f64().unwrap(),
        10.0,
        "edge z",
    );
    let (dx, dy, dz) = xyz(&r["geometry"]["direction"]);
    close(dz, 0.0, "edge is level");
    close(dx.abs() + dy.abs(), 1.0, "edge along x or y");
    // A corner of the top is at z = 10 with x in {0, 80} and y in {0, 50}.
    let r = run(
        &mut s,
        "construct.point_vertex",
        json!({ "faces": [
            { "feature": 2, "face": "top" },
            { "feature": 2, "face": "side 0" },
            { "feature": 2, "face": "side 1" },
        ] }),
    );
    let (x, y, z) = xyz(&r["geometry"]["point"]);
    close(z, 10.0, "corner z");
    assert!(
        (x.abs() < 1e-9 || (x - 80.0).abs() < 1e-9) && (y.abs() < 1e-9 || (y - 50.0).abs() < 1e-9),
        "corner ({x}, {y})"
    );
    // The curved side is not flat, the top is not curved.
    assert!(
        s.execute(
            "construct.plane_offset",
            &json!({ "base": { "feature": 4, "face": "side 0" }, "distance": 1 }),
        )
        .is_err()
    );
    assert!(
        s.execute(
            "construct.axis_cylinder",
            &json!({ "face": { "feature": 4, "face": "top" } }),
        )
        .is_err()
    );
    // Tangent to the cylinder nearest (60, 25, 5): it touches at (45, 25, 5), normal +x.
    let r = run(
        &mut s,
        "construct.plane_tangent",
        json!({ "face": { "feature": 4, "face": "side 0" }, "point": { "x": 60, "y": 25, "z": 5 } }),
    );
    close3(&r["geometry"]["origin"], (45.0, 25.0, 5.0), "touch");
    close3(&r["geometry"]["normal"], (1.0, 0.0, 0.0), "tangent normal");
}

#[test]
fn two_planes_cross_in_an_axis_and_two_points_make_one() {
    let mut s = Session::new();
    let p = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": "xy", "distance": 10 }),
    )["feature"]
        .clone();
    // z = 10 meets XZ (y = 0) along x, through (0, 0, 10).
    let r = run(
        &mut s,
        "construct.axis_two_planes",
        json!({ "planes": [p, "xz"] }),
    );
    close3(&r["geometry"]["origin"], (0.0, 0.0, 10.0), "line point");
    close(
        r["geometry"]["direction"]["x"].as_f64().unwrap().abs(),
        1.0,
        "along x",
    );
    assert!(
        s.execute("construct.axis_two_planes", &json!({ "planes": [p, "xy"] }),)
            .is_err(),
        "parallel planes do not cross"
    );
    // (1, 2, 3) to (1, 2, 7): direction +z.
    let r = run(
        &mut s,
        "construct.axis_two_points",
        json!({ "points": [{ "x": 1, "y": 2, "z": 3 }, { "x": 1, "y": 2, "z": "7 mm" }] }),
    );
    close3(&r["geometry"]["direction"], (0.0, 0.0, 1.0), "two points");
    let r = run(
        &mut s,
        "construct.point_coordinates",
        json!({ "x": 1, "y": "2 mm", "z": "10 mm / 4" }),
    );
    close3(&r["geometry"]["point"], (1.0, 2.0, 2.5), "point");
}

#[test]
fn a_revolve_and_a_circular_pattern_turn_about_a_construction_axis() {
    let mut s = Session::new();
    // The Z axis, where XZ meets YZ.
    let axis = run(
        &mut s,
        "construct.axis_two_planes",
        json!({ "planes": ["xz", "yz"] }),
    )["feature"]
        .clone();
    // On XZ (sketch y is world z): a 10 x 10 square from r = 10 to 20.
    let sk = run(&mut s, "sketch.create", json!({ "plane": "xz" }))["sketch"].clone();
    run(
        &mut s,
        "sketch.rectangle",
        json!({ "x1": 10, "y1": 0, "x2": 20, "y2": 10 }),
    );
    run(&mut s, "sketch.finish", json!({}));
    let r = run(
        &mut s,
        "solid.revolve",
        json!({ "sketch": sk, "profiles": [{ "x": 15, "y": 5 }], "axis_feature": axis }),
    );
    // A ring: pi (20² - 10²) 10 = 3000 pi.
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!(
        (v - 3000.0 * std::f64::consts::PI).abs() < 1e-6,
        "{v} vs 3000 pi"
    );
    // An axis off the sketch plane is refused.
    let off = run(
        &mut s,
        "construct.axis_two_points",
        json!({ "points": [{ "x": 0, "y": 5, "z": 0 }, { "x": 0, "y": 5, "z": 1 }] }),
    )["feature"]
        .clone();
    assert!(
        s.execute(
            "solid.revolve",
            &json!({ "sketch": sk, "profiles": [{ "x": 15, "y": 5 }], "axis_feature": off }),
        )
        .is_err()
    );
    // A 4 x 4 x 4 cube at x = 30 repeated 4 times about the axis: 4 x 64 mm³.
    let cube = run(&mut s, "sketch.create", json!({ "plane": "xy" }))["sketch"].clone();
    run(
        &mut s,
        "sketch.rectangle",
        json!({ "x1": 28, "y1": -2, "x2": 32, "y2": 2 }),
    );
    run(&mut s, "sketch.finish", json!({}));
    let c = run(
        &mut s,
        "solid.extrude",
        json!({ "sketch": cube, "profiles": [{ "x": 30, "y": 0 }], "distance": 4 }),
    )["feature"]
        .clone();
    run(
        &mut s,
        "solid.pattern_circular",
        json!({ "feature": c, "count": 4, "axis_feature": axis }),
    );
    let cubes: f64 = s
        .solids()
        .iter()
        .filter(|x| x.name.starts_with("Extrude") || x.name.starts_with("Pattern"))
        .map(|x| x.body.volume().unwrap())
        .sum();
    close(cubes, 256.0, "four cubes");
    // The copy turned by 90 deg sits around y = 30.
    assert!(s.solids().iter().any(|x| {
        let b = x.body.bounding_box().unwrap();
        (b.min.y - 28.0).abs() < 1e-6 && (b.max.y - 32.0).abs() < 1e-6
    }));
}

#[test]
fn picking_names_the_face_its_nearest_edge_and_vertex() {
    let mut s = plate("10 mm");
    // Looking straight down at the plate.
    run(&mut s, "view.standard", json!({ "view": "top" }));
    run(&mut s, "view.fit", json!({ "width": 800, "height": 600 }));
    let r = run(
        &mut s,
        "construct.pick",
        json!({ "x": 400, "y": 300, "width": 800, "height": 600 }),
    );
    assert_eq!(r["face"]["face"], "top");
    assert_eq!(r["surface"], "plane");
    assert_eq!(r["edge"]["curve"], "line");
    assert_eq!(r["edge"]["faces"][0]["face"], "top");
    assert_eq!(r["vertex"]["faces"].as_array().unwrap().len(), 3);
    // What it names builds: an axis along that edge, a point at that vertex.
    let edge = r["edge"].clone();
    run(&mut s, "construct.axis_edge", json!({ "edge": edge }));
    let faces = r["vertex"]["faces"].clone();
    let p = run(&mut s, "construct.point_vertex", json!({ "faces": faces }));
    close(
        p["geometry"]["point"]["z"].as_f64().unwrap(),
        10.0,
        "vertex z",
    );
}

#[test]
fn a_construction_that_cannot_build_any_more_reports_why() {
    let mut s = plate("10 mm");
    let r = run(
        &mut s,
        "construct.plane_offset",
        json!({ "base": { "feature": 2, "face": "top" }, "distance": 5 }),
    );
    let id = r["feature"].as_u64().unwrap() as u32;
    // Suppressing the plate removes the face: the plane keeps an error, nothing panics.
    run(
        &mut s,
        "feature.edit",
        json!({ "feature": 2, "suppressed": true }),
    );
    assert!(s.feature_errors().get(&id).is_some());
    assert!(s.constructs().is_empty());
    assert!(s.scene.fills.is_empty());
}

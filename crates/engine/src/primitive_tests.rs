//! Tests of the modelling quick wins: primitives, physical materials, scale, align and
//! the validity check, each against values worked out by hand.

use super::*;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn volume(s: &Session, name: &str) -> f64 {
    s.solids()
        .iter()
        .find(|x| x.name == name)
        .and_then(|x| x.body.volume().ok())
        .unwrap_or(f64::NAN)
}

fn bounds(s: &Session, name: &str) -> chamfera_geom::Aabb {
    s.solids()
        .iter()
        .find(|x| x.name == name)
        .and_then(|x| x.body.bounding_box())
        .unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

/// The name of the first face of `body` on a surface `matches` accepts.
fn face_named(s: &Session, body: &str, matches: fn(&chamfera_kernel::Surface) -> bool) -> String {
    let b = &s.solids().iter().find(|x| x.name == body).unwrap().body;
    b.faces()
        .into_iter()
        .find(|f| {
            b.face_surface(*f)
                .ok()
                .flatten()
                .is_some_and(|(sf, _)| matches(&sf))
        })
        .and_then(|f| b.face_name(f).ok().flatten().map(str::to_owned))
        .unwrap()
}

#[test]
fn each_primitive_has_its_analytic_volume_and_is_valid() {
    let mut s = Session::new();
    // (command, params, name, volume worked out by hand)
    let cases: [(&str, Value, &str, f64); 6] = [
        // 10 x 20 x 30.
        (
            "solid.box",
            json!({"length": 10, "width": 20, "height": 30}),
            "Box1",
            6000.0,
        ),
        // pi r² h = pi 25 10.
        (
            "solid.cylinder",
            json!({"radius": 5, "height": 10, "origin": {"x": 40, "y": 0, "z": 0}}),
            "Cylinder1",
            250.0 * PI,
        ),
        // 4/3 pi r³ = 4/3 pi 1000.
        (
            "solid.sphere",
            json!({"radius": "10 mm", "origin": {"x": 80, "y": 0, "z": 0}}),
            "Sphere1",
            4000.0 / 3.0 * PI,
        ),
        // Pappus: 2 pi R * pi r² = 2 pi² 20 25 = 1000 pi².
        (
            "solid.torus",
            json!({"major_radius": 20, "minor_radius": 5, "origin": {"x": 0, "y": 80, "z": 0}}),
            "Torus1",
            1000.0 * PI * PI,
        ),
        // pi r² h / 3 = pi 9 9 / 3.
        (
            "solid.cone",
            json!({"radius": 3, "height": 9, "origin": {"x": 0, "y": -50, "z": 0}}),
            "Cone1",
            27.0 * PI,
        ),
        // Frustum: pi h / 3 (r1² + r1 r2 + r2²) = pi 4 / 3 (36 + 18 + 9) = 84 pi.
        (
            "solid.cone",
            json!({"radius": 6, "top_radius": 3, "height": 4, "axis": "x",
                   "origin": {"x": 0, "y": 0, "z": 60}}),
            "Cone2",
            84.0 * PI,
        ),
    ];
    for (id, p, name, want) in cases {
        let r = s.execute(id, &p).unwrap();
        assert_eq!(r["name"], name);
        assert!(near(volume(&s, name), want), "{name}: {}", volume(&s, name));
        assert!(near(r["expected_volume"].as_f64().unwrap(), want), "{name}");
    }
    let c = s.execute("inspect.check", &json!({})).unwrap();
    assert_eq!(c["checked"], 6);
    assert_eq!(c["invalid"], 0);
    // Euler characteristic 2 (S - G): 2 for every lump of genus 0, 0 for the torus.
    for b in c["bodies"].as_array().unwrap() {
        let (euler, genus) = if b["name"] == "Torus1" {
            (0, 1)
        } else {
            (2, 0)
        };
        assert_eq!(b["euler"], euler, "{b}");
        assert_eq!(b["genus"], genus, "{b}");
    }
    // The box sits centred across its base: x -5..5, y -10..10, z 0..30.
    let bb = bounds(&s, "Box1");
    assert!(near(bb.min.x, -5.0) && near(bb.max.y, 10.0) && near(bb.max.z, 30.0));
    // Cone2 runs along x from 0 to 4 at z = 60, its base radius 6.
    let bb = bounds(&s, "Cone2");
    assert!(near(bb.min.x, 0.0) && near(bb.max.x, 4.0) && near(bb.max.z, 66.0));
}

#[test]
fn a_cylinder_primitive_cuts_a_box_and_undo_takes_it_back() {
    let mut s = Session::new();
    s.execute(
        "solid.box",
        &json!({"length": 20, "width": 20, "height": 20}),
    )
    .unwrap();
    // A r = 5 cylinder through the whole box: 8000 - 25 pi 20 = 8000 - 500 pi.
    s.execute(
        "solid.cylinder",
        &json!({"radius": 5, "height": 20, "operation": "cut"}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 1);
    assert!(near(volume(&s, "Box1"), 8000.0 - 500.0 * PI));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(volume(&s, "Box1"), 8000.0));
}

#[test]
fn primitive_sizes_are_parameters_and_can_be_edited() {
    let mut s = Session::new();
    let id = s.execute("solid.sphere", &json!({"radius": 10})).unwrap()["feature"]
        .as_u64()
        .unwrap();
    // feature.edit: r = 5 gives 4/3 pi 125.
    s.execute("feature.edit", &json!({"feature": id, "radius": 5}))
        .unwrap();
    assert!(near(volume(&s, "Sphere1"), 500.0 / 3.0 * PI));
    // A model parameter: Sphere1.radius = 3 gives 36 pi.
    s.execute(
        "param.set",
        &json!({"name": "Sphere1.radius", "expression": "3 mm"}),
    )
    .unwrap();
    assert!(near(volume(&s, "Sphere1"), 36.0 * PI));
    // Keys of another shape are refused and change nothing.
    assert!(
        s.execute("feature.edit", &json!({"feature": id, "length": 5}))
            .is_err()
    );
    assert!(near(volume(&s, "Sphere1"), 36.0 * PI));
}

#[test]
fn bad_primitives_are_refused_and_change_nothing() {
    let mut s = Session::new();
    for (id, p) in [
        ("solid.box", json!({"length": 0, "width": 1, "height": 1})),
        ("solid.box", json!({"length": 1, "width": 1})),
        ("solid.sphere", json!({"radius": -2})),
        ("solid.sphere", json!({"radius": "5 deg"})),
        ("solid.torus", json!({"major_radius": 5, "minor_radius": 6})),
        (
            "solid.cone",
            json!({"radius": 4, "top_radius": 4, "height": 3}),
        ),
        (
            "solid.cylinder",
            json!({"radius": 1, "height": 1, "axis": "w"}),
        ),
        (
            "solid.cylinder",
            json!({"radius": 1, "height": 1, "operation": "melt"}),
        ),
        (
            "solid.cylinder",
            json!({"radius": 1, "height": 1, "origin": {"x": 1e300, "y": 0, "z": 0}}),
        ),
        ("solid.cylinder", json!({"radius": 1e9, "height": 1})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert!(s.doc.features.is_empty());
    assert!(!s.can_undo());
}

#[test]
fn a_steel_cube_of_10_mm_weighs_7_85_g() {
    let mut s = Session::new();
    s.execute(
        "solid.box",
        &json!({"length": 10, "width": 10, "height": 10}),
    )
    .unwrap();
    let r = s
        .execute("body.material", &json!({"body": "Box1", "preset": "steel"}))
        .unwrap();
    // 1000 mm³ = 1 cm³ at 7.85 g/cm³.
    assert!(near(r["mass"].as_f64().unwrap(), 7.85));
    let m = s.execute("inspect.mass", &json!({"body": "Box1"})).unwrap();
    assert!(near(m["mass"].as_f64().unwrap(), 7.85), "{m}");
    assert_eq!(m["mass_unit"], "g");
    assert_eq!(m["material"], "steel");
    // A density typed with a unit: 2700 kg/m3 is aluminium's 2.7 g/cm³, so 2.7 g.
    s.execute(
        "body.material",
        &json!({"body": "Box1", "density": "2700 kg/m3", "name": "my alloy"}),
    )
    .unwrap();
    let m = s.execute("inspect.mass", &json!({"body": "Box1"})).unwrap();
    assert!(near(m["mass"].as_f64().unwrap(), 2.7), "{m}");
    // An explicit density still wins: 1 per mm³ gives 1000.
    let m = s
        .execute("inspect.mass", &json!({"body": "Box1", "density": 1}))
        .unwrap();
    assert!(near(m["mass"].as_f64().unwrap(), 1000.0));
    assert_eq!(m["material"], Value::Null);
    // The material is saved with the design and undone like any change.
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back.materials, s.doc.materials);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(
        s.doc.materials.values().next().map(|m| m.name.as_str()),
        Some("steel")
    );
    s.execute("body.material", &json!({"body": "Box1", "clear": true}))
        .unwrap();
    assert!(s.doc.materials.is_empty());
    // Bad ones are refused.
    for p in [
        json!({"body": "Box1", "preset": "unobtainium"}),
        json!({"body": "Box1", "density": 0}),
        json!({"body": "Box1", "density": 1e6}),
        json!({"body": "Box1", "density": "7 furlongs"}),
        json!({"body": "Box1", "preset": "steel", "density": 2}),
        json!({"body": "Box1"}),
        json!({"body": "Nothing", "preset": "steel"}),
    ] {
        assert!(s.execute("body.material", &p).is_err(), "{p}");
    }
}

#[test]
fn scale_two_multiplies_volume_by_eight() {
    let mut s = Session::new();
    s.execute("solid.sphere", &json!({"radius": 10})).unwrap();
    let r = s
        .execute("solid.scale", &json!({"bodies": ["Sphere1"], "factor": 2}))
        .unwrap();
    assert_eq!(r["name"], "Scale1");
    // 4/3 pi 1000 * 8.
    assert!(near(volume(&s, "Sphere1"), 32_000.0 / 3.0 * PI));
    assert_eq!(r["volume_ratio"], 8.0);
    // The feature replies with the body it changed.
    assert_eq!(r["solids"][0]["name"], "Sphere1");
}

#[test]
fn a_box_scales_non_uniformly_about_a_point() {
    let mut s = Session::new();
    // x -5..5, y -5..5, z 0..10.
    s.execute(
        "solid.box",
        &json!({"length": 10, "width": 10, "height": 10}),
    )
    .unwrap();
    s.execute(
        "solid.scale",
        &json!({"bodies": ["Box1"], "x": 2, "y": 3, "z": 0.5,
                "origin": {"x": 5, "y": 0, "z": 10}}),
    )
    .unwrap();
    // 20 x 30 x 5 = 3000; x: 5 + 2 (-5 - 5) = -15 to 5; z: 10 + 0.5 (0 - 10) = 5 to 10.
    assert!(near(volume(&s, "Box1"), 3000.0));
    let bb = bounds(&s, "Box1");
    assert!(near(bb.min.x, -15.0) && near(bb.max.x, 5.0));
    assert!(near(bb.min.y, -15.0) && near(bb.max.y, 15.0));
    assert!(near(bb.min.z, 5.0) && near(bb.max.z, 10.0));
    // Edit one factor: x back to 1 gives 10 x 30 x 5 = 1500.
    let id = s.doc.features.last().unwrap().id;
    s.execute("feature.edit", &json!({"feature": id, "x": 1}))
        .unwrap();
    assert!(near(volume(&s, "Box1"), 1500.0));
}

#[test]
fn a_scale_that_would_make_an_elliptic_cylinder_is_refused() {
    let mut s = Session::new();
    s.execute("solid.cylinder", &json!({"radius": 5, "height": 10}))
        .unwrap();
    let before = s.doc.features.len();
    let e = s
        .execute("solid.scale", &json!({"bodies": ["Cylinder1"], "x": 2}))
        .unwrap_err();
    assert!(e.to_string().contains("non-uniform"), "{e}");
    assert_eq!(s.doc.features.len(), before);
    assert!(near(volume(&s, "Cylinder1"), 250.0 * PI));
    // Along its axis it stays a cylinder: r = 5, h = 30, 750 pi.
    s.execute("solid.scale", &json!({"bodies": ["Cylinder1"], "z": 3}))
        .unwrap();
    assert!(near(volume(&s, "Cylinder1"), 750.0 * PI));
    for p in [
        json!({"bodies": ["Cylinder1"], "factor": 0}),
        json!({"bodies": ["Cylinder1"], "factor": -2}),
        json!({"bodies": ["Cylinder1"], "factor": "2 mm"}),
        json!({"bodies": ["Cylinder1"], "factor": 2, "x": 3}),
        json!({"bodies": ["Cylinder1"]}),
        json!({"bodies": ["Nothing"], "factor": 2}),
        json!({"factor": 2}),
    ] {
        assert!(s.execute("solid.scale", &p).is_err(), "{p}");
    }
}

#[test]
fn align_puts_a_face_on_a_face_and_an_axis_on_an_axis() {
    let mut s = Session::new();
    // A: z 0..10 under B: z 20..30, both 10 x 10 boxes, B off to the side in x.
    s.execute(
        "solid.box",
        &json!({"length": 10, "width": 10, "height": 10}),
    )
    .unwrap();
    s.execute(
        "solid.box",
        &json!({"length": 10, "width": 10, "height": 10,
                "origin": {"x": 30, "y": 0, "z": 20}}),
    )
    .unwrap();
    let a_top = face_named(
        &s,
        "Box1",
        |sf| matches!(sf, chamfera_kernel::Surface::Plane(f) if f.origin().z > 9.0 && f.z().z.abs() > 0.5),
    );
    let b_bottom = face_named(
        &s,
        "Box2",
        |sf| matches!(sf, chamfera_kernel::Surface::Plane(f) if f.origin().z < 21.0 && f.z().z.abs() > 0.5),
    );
    // Top of A (z = 10) onto the bottom of B (z = 20): straight up by 10.
    let r = s
        .execute(
            "solid.align",
            &json!({"from": {"solid": "Box1", "face": a_top},
                    "to": {"solid": "Box2", "face": b_bottom}}),
        )
        .unwrap();
    let t: Vec<f64> = serde_json::from_value(r["translation"].clone()).unwrap();
    assert!(
        near(t[0], 0.0) && near(t[1], 0.0) && near(t[2], 10.0),
        "{t:?}"
    );
    let bb = bounds(&s, "Box1");
    assert!(near(bb.min.z, 10.0) && near(bb.max.z, 20.0));
    assert!(near(volume(&s, "Box1"), 1000.0));

    // Two cylinders: the axis of C (at 0, 50) onto the axis of D (at 30, 90): moves
    // (30, 40, 0) square to the axes, keeping its height.
    s.execute(
        "solid.cylinder",
        &json!({"radius": 3, "height": 5, "origin": {"x": 0, "y": 50, "z": 0}}),
    )
    .unwrap();
    s.execute(
        "solid.cylinder",
        &json!({"radius": 4, "height": 5, "origin": {"x": 30, "y": 90, "z": 7}}),
    )
    .unwrap();
    let cyl = |sf: &chamfera_kernel::Surface| matches!(sf, chamfera_kernel::Surface::Cylinder(_));
    let (c, d) = (
        face_named(&s, "Cylinder1", cyl),
        face_named(&s, "Cylinder2", cyl),
    );
    let r = s
        .execute(
            "solid.align",
            &json!({"from": {"solid": "Cylinder1", "face": c},
                    "to": {"solid": "Cylinder2", "face": d}}),
        )
        .unwrap();
    let t: Vec<f64> = serde_json::from_value(r["translation"].clone()).unwrap();
    assert!(
        near(t[0], 30.0) && near(t[1], 40.0) && near(t[2], 0.0),
        "{t:?}"
    );
    assert!(near(r["distance"].as_f64().unwrap(), 50.0));
    // A vertex onto a free point: Box2's lowest corner (25, -5, 20) onto (0, 0, 0).
    let r = s
        .execute(
            "solid.align",
            &json!({"from": {"solid": "Box2", "vertex": [25, -5, 20]},
                    "to": {"point": [0, 0, 0]}, "copy": true}),
        )
        .unwrap();
    let t: Vec<f64> = serde_json::from_value(r["translation"].clone()).unwrap();
    assert!(
        near(t[0], -25.0) && near(t[1], 5.0) && near(t[2], -20.0),
        "{t:?}"
    );
    assert_eq!(s.solids().len(), 5);
    // Refusals: a face onto a face square to it, nothing to move, no `to`.
    let side = face_named(
        &s,
        "Box2",
        |sf| matches!(sf, chamfera_kernel::Surface::Plane(f) if f.z().z.abs() < 0.5),
    );
    for p in [
        json!({"from": {"solid": "Box1", "face": a_top}, "to": {"solid": "Box2", "face": side}}),
        json!({"from": {"solid": "Box1", "face": a_top}, "to": {"point": [99, 99, 20]}}),
        json!({"from": {"solid": "Box1", "face": a_top}}),
        json!({"from": {"point": [0, 0, 0]}, "to": {"point": [1, 1, 1]}}),
    ] {
        assert!(s.execute("solid.align", &p).is_err(), "{p}");
    }
}

#[test]
fn the_check_names_the_bodies_it_was_asked_for() {
    let mut s = Session::new();
    s.execute("solid.sphere", &json!({"radius": 2})).unwrap();
    let r = s
        .execute("inspect.check", &json!({"bodies": ["Sphere1"]}))
        .unwrap();
    assert_eq!(r["checked"], 1);
    assert_eq!(r["bodies"][0]["valid"], true);
    assert_eq!(r["bodies"][0]["kind"], "solid");
    // 4/3 pi 8.
    assert!(near(
        r["bodies"][0]["volume"].as_f64().unwrap(),
        32.0 / 3.0 * PI
    ));
    assert!(
        s.execute("inspect.check", &json!({"bodies": ["Nope"]}))
            .is_err()
    );
    assert!(s.execute("inspect.check", &json!({"bodies": 5})).is_err());
}

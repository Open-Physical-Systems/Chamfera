//! Persistent naming (rung K8): upstream edits that change the topology, and the
//! downstream references that must still find the right face or edge. Every expected
//! value is derived by hand in the comment next to it.

use crate::Session;
use crate::naming::{Kind, Want};
use serde_json::{Value, json};
use std::f64::consts::PI;

/// The corner of a fillet's cross-section: (1 - π/4) r² per unit of edge length.
fn corner(r: f64) -> f64 {
    (1.0 - PI / 4.0) * r * r
}

fn volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

fn no_errors(s: &Session) {
    assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
    assert!(s.sketch_errors().is_empty(), "{:?}", s.sketch_errors());
}

fn set(s: &mut Session, name: &str, expression: &str) {
    s.execute(
        "param.set",
        &json!({"name": name, "expression": expression}),
    )
    .unwrap();
}

/// A plate `w` x 50, `t` thick, on the XY plane from the origin: sketch 1, feature 2.
fn plate(s: &mut Session, w: &str) {
    set(s, "w", w);
    set(s, "t", "10 mm");
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 100, "y2": 50}),
        )
        .unwrap();
    let pts: Vec<u64> = r["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let lines: Vec<u64> = r["curves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    s.execute(
        "sketch.constrain",
        &json!({"constraints": [
            {"kind": "fix", "point": pts[0], "at": {"x": 0.0, "y": 0.0}},
            {"kind": "length", "line": lines[0], "value": 100.0},
            {"kind": "length", "line": lines[1], "value": 50.0},
        ]}),
    )
    .unwrap();
    let width = s.doc.sketches[0]
        .sketch
        .constraints()
        .find(|(_, c)| matches!(c, chamfera_sketch::Constraint::Length { line, .. } if u64::from(line.0) == lines[0]))
        .map(|(id, _)| id.0)
        .unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "expression": "w"}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let r = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 1, "y": 1}], "distance": "t"}),
        )
        .unwrap();
    assert_eq!(r["feature"], 2);
}

/// A cut across the plate along Y, 10 wide, from x = `a` (a parameter) to `a` + 10, from
/// height `z0` up: a plane `a` along X (feature 3), a rectangle on it (sketch 4) covering
/// y -5..55 and z `z0`..40, cut 10 along +X (feature 5). With `z0` > 0 the cut is a slot
/// from the top, below it the plate stays whole.
fn slot(s: &mut Session, a: &str, z0: f64) {
    set(s, "a", a);
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "yz", "distance": "a"}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let sk = s
        .execute("sketch.create", &json!({"plane": plane}))
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    // On the YZ frame sketch x is world Y and sketch y is world Z.
    s.execute(
        "sketch.rectangle",
        &json!({"x1": -5, "y1": z0, "x2": 55, "y2": 40}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sk, "profiles": [{"x": 25, "y": 20}], "distance": 10, "operation": "cut"}),
    )
    .unwrap();
}

/// The name of the edge from `a` to `b` in `solid.edges`.
fn edge_between(s: &mut Session, a: [f64; 3], b: [f64; 3]) -> String {
    let r = s.execute("solid.edges", &json!({})).unwrap();
    let near =
        |v: &Value, p: [f64; 3]| (0..3).all(|i| (v[i].as_f64().unwrap() - p[i]).abs() < 1e-9);
    r["solids"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|x| x["edges"].as_array().unwrap().iter())
        .find(|e| {
            (near(&e["from"], a) && near(&e["to"], b)) || (near(&e["from"], b) && near(&e["to"], a))
        })
        .and_then(|e| e["edge"].as_str())
        .unwrap_or_else(|| panic!("no edge {a:?} - {b:?} in {r}"))
        .to_owned()
}

/// The x range of a face's piece, from the face's edges.
fn x_range(s: &Session, f: &crate::naming::Found) -> (f64, f64) {
    let body = &s.solids()[f.body].body;
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for e in body.face_edges(f.face().unwrap()).unwrap() {
        for v in body.edge_vertices(e).unwrap() {
            let p = body.vertex_point(v).unwrap();
            lo = lo.min(p.x);
            hi = hi.max(p.x);
        }
    }
    (lo, hi)
}

#[test]
fn a_sketch_stays_on_its_piece_when_a_moved_slot_splits_the_face() {
    // Plate 100 x 50 x 10; a slot 5 deep from the top (z 5..10) at x 95..105 notches the
    // end, so the top is one face (x 0..95). A sketch on the top, picked at (75, 25, 10),
    // holds a circle r 5 at (75, 25) extruded 5 as a boss.
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    slot(&mut s, "95 mm", 5.0);
    let sk = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top", "point": {"x": 75, "y": 25, "z": 10}}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute("sketch.circle", &json!({"cx": 75, "cy": 25, "radius": 5}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sk, "profiles": [{"x": 75, "y": 25}], "distance": 5, "operation": "join"}),
    )
    .unwrap();
    no_errors(&s);
    // 50000 - slot 5 x 50 x 5 (x 95..100) + boss 25π · 5.
    assert!((volume(&s) - (50_000.0 - 1250.0 + 125.0 * PI)).abs() < 1e-6);
    // Move the slot to x 45..55: the top splits into x 0..45 and x 55..100.
    set(&mut s, "a", "45 mm");
    no_errors(&s);
    assert!((volume(&s) - (50_000.0 - 2500.0 + 125.0 * PI)).abs() < 1e-6);
    // The plain name now has two pieces; the sketch's reference resolves to the one
    // holding its point: x 55..100, area 45 · 50 less the boss's circle 25π.
    let found = s
        .resolve_reference(Some(sk as u32), Kind::Face, "2:top", Want::One)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(x_range(&s, &found[0]), (55.0, 100.0));
    let area = s.solids()[found[0].body]
        .body
        .face_area(found[0].face().unwrap())
        .unwrap();
    assert!((area - (2250.0 - 25.0 * PI)).abs() < 1e-6, "{area}");
    assert_eq!(
        s.reference_name(found[0].body, found[0].entity).as_deref(),
        Some("2:top#2")
    );
    // Without a point a single-face consumer is told to pick a piece, naming both.
    let e = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": s.solids()[0].name, "face": "2:top"}}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("split into 2 pieces; pick one: \"2:top#1\", \"2:top#2\""),
        "{e}"
    );
    // Either piece by its own name.
    let r = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": s.solids()[0].name, "face": "2:top#1"}}),
        )
        .unwrap();
    assert!(
        (r["a"]["area"].as_f64().unwrap() - 2250.0).abs() < 1e-6,
        "{r}"
    );
}

#[test]
fn a_fillet_follows_its_edge_through_a_moved_slot() {
    // Plate 100 x 50 x 10, slot 5 deep at x 95..105: the top front edge runs x 0..95.
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    slot(&mut s, "95 mm", 5.0);
    let edge = edge_between(&mut s, [0.0, 0.0, 10.0], [95.0, 0.0, 10.0]);
    s.execute("solid.fillet", &json!({"edges": [edge], "radius": 2}))
        .unwrap();
    no_errors(&s);
    // 50000 - slot 1250 - fillet corner(2) · 95.
    let v = 50_000.0 - 1250.0 - corner(2.0) * 95.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // The slot moves to x 45..55 and splits the edge in two: the fillet names the whole
    // edge, so both pieces (45 + 45 long) are rounded.
    set(&mut s, "a", "45 mm");
    no_errors(&s);
    let v = 50_000.0 - 2500.0 - corner(2.0) * 90.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
}

#[test]
fn a_fillet_on_one_piece_of_a_split_edge_keeps_to_its_piece() {
    // With the slot at x 45..55 the top front edge is two pieces, x 0..45 (#1) and
    // x 55..100 (#2). Round #2 only, r 2: 50000 - 2500 - corner(2) · 45.
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    slot(&mut s, "45 mm", 5.0);
    let edge = edge_between(&mut s, [55.0, 0.0, 10.0], [100.0, 0.0, 10.0]);
    assert!(edge.ends_with("#2"), "{edge}");
    assert!(edge_between(&mut s, [0.0, 0.0, 10.0], [45.0, 0.0, 10.0]).ends_with("#1"));
    s.execute("solid.fillet", &json!({"edges": [edge], "radius": 2}))
        .unwrap();
    let v = 50_000.0 - 2500.0 - corner(2.0) * 45.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // The slot moves to x 60..70: the pieces are x 0..60 and x 70..100, and the fillet's
    // point (the old piece's midpoint, x 77.5) lies on the second: 30 long.
    set(&mut s, "a", "60 mm");
    no_errors(&s);
    let v = 50_000.0 - 2500.0 - corner(2.0) * 30.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // The slot moves to x 30..40: now the point x 77.5 lies on x 40..100, 60 long. A
    // piece number alone would have said x 0..30 or the first piece.
    set(&mut s, "a", "30 mm");
    no_errors(&s);
    let v = 50_000.0 - 2500.0 - corner(2.0) * 60.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // Back at the end (x 95..105) the edge is whole again (x 0..95): the piece is the
    // whole edge, rounded over 95.
    set(&mut s, "a", "95 mm");
    no_errors(&s);
    let v = 50_000.0 - 1250.0 - corner(2.0) * 95.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
}

#[test]
fn holes_find_their_piece_when_a_growing_plate_is_cut_in_two() {
    // Plate w x 50 x 10 with a full-depth slot at x 45..55 (from z -5 up). At w = 90 the
    // slot splits the plate into two lumps, x 0..45 and x 55..90, each with a piece of
    // `2:top`. A hole Ø6 through at (20, 25) is on the first, one at (80, 25) on the
    // second.
    let mut s = Session::new();
    plate(&mut s, "90 mm");
    slot(&mut s, "45 mm", -5.0);
    let pieces = s
        .resolve_reference(None, Kind::Face, "2:top", Want::All)
        .unwrap();
    assert_eq!(pieces.len(), 2);
    let near =
        |(a, b): (f64, f64), (c, d): (f64, f64)| (a - c).abs() < 1e-9 && (b - d).abs() < 1e-9;
    assert!(near(x_range(&s, &pieces[0]), (0.0, 45.0)));
    assert!(near(x_range(&s, &pieces[1]), (55.0, 90.0)));
    for x in [20, 80] {
        s.execute(
            "solid.hole",
            &json!({"face": "2:top", "points": [{"x": x, "y": 25}], "diameter": 6, "through_all": true}),
        )
        .unwrap();
    }
    no_errors(&s);
    // (45 + 35) · 50 · 10 - 2 · 9π · 10.
    let v = 40_000.0 - 180.0 * PI;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // The plate grows to 120: the second body is x 55..120, the holes stay.
    set(&mut s, "w", "120 mm");
    no_errors(&s);
    let v = (45.0 + 65.0) * 500.0 - 180.0 * PI;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // Each lump has one hole: 45 · 500 - 90π and 65 · 500 - 90π.
    let mut each: Vec<f64> = s
        .solids()
        .iter()
        .flat_map(|x| {
            x.body
                .shells()
                .into_iter()
                .map(|sh| x.body.shell_volume(sh).unwrap())
        })
        .collect();
    each.sort_by(f64::total_cmp);
    assert!((each[0] - (22_500.0 - 90.0 * PI)).abs() < 1e-6, "{each:?}");
    assert!((each[1] - (32_500.0 - 90.0 * PI)).abs() < 1e-6, "{each:?}");
}

#[test]
fn a_shell_keeps_its_open_face_through_a_moved_notch_and_a_thicker_plate() {
    // Plate 100 x 50 x t with a notch cut through at x a..a+10, y 30..50. Shell 2 thick
    // with `2:top` open. The cavity is the outline moved in by 2, (t - 2) deep.
    // a = 90 (corner notch): outer 5000 - 200 = 4800; inner (96 · 46) - (10 · 20) = 4216:
    // 4800 · 10 - 4216 · 8 = 14272.
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    set(&mut s, "a", "90 mm");
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "yz", "distance": "a"}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let sk = s
        .execute("sketch.create", &json!({"plane": plane}))
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 30, "y1": -5, "x2": 60, "y2": 40}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sk, "profiles": [{"x": 40, "y": 20}], "distance": 10, "operation": "cut"}),
    )
    .unwrap();
    s.execute("solid.shell", &json!({"faces": ["2:top"], "thickness": 2}))
        .unwrap();
    no_errors(&s);
    assert!((volume(&s) - 14_272.0).abs() < 1e-6, "{}", volume(&s));
    // a = 45 (notch in the middle of the back): outer 4800; inner 4416 - 14 · 20 = 4136:
    // 48000 - 4136 · 8 = 14912.
    set(&mut s, "a", "45 mm");
    no_errors(&s);
    assert!((volume(&s) - 14_912.0).abs() < 1e-6, "{}", volume(&s));
    // t = 20: 4800 · 20 - 4136 · 18 = 21552.
    set(&mut s, "t", "20 mm");
    no_errors(&s);
    assert!((volume(&s) - 21_552.0).abs() < 1e-6, "{}", volume(&s));
}

#[test]
fn fillets_on_pattern_copies_follow_the_count() {
    // Plate 100 x 50 x 10 (50000); a boss 10 x 10 x 5 at x 10..20, y 20..30 on the top,
    // joined (500); a linear pattern of it along X every 25, `n` = 3: copies at x 35..45
    // and 60..70.
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    s.execute(
        "param.set",
        &json!({"name": "n", "expression": "3", "unit": "unitless"}),
    )
    .unwrap();
    let sk = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top"}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 10, "y1": 20, "x2": 20, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let boss = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sk, "profiles": [{"x": 15, "y": 25}], "distance": 5, "operation": "join"}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    s.execute(
        "solid.pattern_linear",
        &json!({"feature": boss, "count": "n", "direction": {"x": 1, "y": 0, "z": 0}, "spacing": 25}),
    )
    .unwrap();
    no_errors(&s);
    assert!((volume(&s) - 51_500.0).abs() < 1e-6);
    // The source boss's top front edge and the third copy's are different names now.
    let source = edge_between(&mut s, [10.0, 20.0, 15.0], [20.0, 20.0, 15.0]);
    let third = edge_between(&mut s, [60.0, 20.0, 15.0], [70.0, 20.0, 15.0]);
    assert_ne!(source, third);
    // Rounding the source's edge rounds that one edge only (10 long, r 1).
    let f1 = s
        .execute("solid.fillet", &json!({"edges": [source], "radius": 1}))
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    assert!((volume(&s) - (51_500.0 - corner(1.0) * 10.0)).abs() < 1e-6);
    // And the third copy's edge, r 1: two edges rounded.
    s.execute("solid.fillet", &json!({"edges": [third], "radius": 1}))
        .unwrap();
    assert!((volume(&s) - (51_500.0 - corner(1.0) * 20.0)).abs() < 1e-6);
    // Four copies: the third stays where it was (x 60..70), its fillet with it.
    s.execute(
        "param.set",
        &json!({"name": "n", "expression": "4", "unit": "unitless"}),
    )
    .unwrap();
    no_errors(&s);
    assert!((volume(&s) - (52_000.0 - corner(1.0) * 20.0)).abs() < 1e-6);
    // Two copies: the third is gone and its fillet says so; the source's still builds.
    s.execute(
        "param.set",
        &json!({"name": "n", "expression": "2", "unit": "unitless"}),
    )
    .unwrap();
    let errors = s.feature_errors();
    assert!(!errors.contains_key(&(f1 as u32)), "{errors:?}");
    let e = errors
        .values()
        .next()
        .expect("the third copy's fillet fails");
    assert!(e.contains("is no longer there"), "{e}");
}

#[test]
fn signatures_save_and_open_and_old_files_open_without_them() {
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    let sk = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top", "point": {"x": 75, "y": 25, "z": 10}}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap() as u32;
    let sig = s.doc.signature(sk, "2:top").unwrap().clone();
    assert_eq!(sig.shape, "plane");
    assert_eq!(sig.direction, Some([0.0, 0.0, 1.0]));
    assert_eq!(sig.point, Some([75.0, 25.0, 10.0]));
    let text = s.doc.to_json().unwrap();
    let back = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(back.signature(sk, "2:top"), Some(&sig));
    // A version 26 file holds no `references`: it opens with none.
    let mut old: Value = serde_json::from_str(&text).unwrap();
    old["version"] = json!(26);
    old.as_object_mut().unwrap().remove("references");
    let back = chamfera_doc::Document::from_json(&old.to_string()).unwrap();
    assert!(back.references.is_empty());
    // Hostile signatures in a file are refused.
    for bad in [
        json!({"point": [1e300, 0, 0]}),
        json!({"direction": [0, 0, 0]}),
        json!({"neighbours": vec!["x"; 100]}),
        json!({"shape": "x".repeat(100)}),
    ] {
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["references"][sk.to_string()]["2:top"] = bad.clone();
        assert!(
            chamfera_doc::Document::from_json(&v.to_string()).is_err(),
            "{bad}"
        );
    }
    // References of an item that is not in the design are refused.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["references"]["999"] = json!({"2:top": {}});
    assert!(chamfera_doc::Document::from_json(&v.to_string()).is_err());
    // Deleting the sketch drops its signatures.
    s.execute("sketch.delete", &json!({"sketch": sk})).unwrap();
    assert!(s.doc.signature(sk, "2:top").is_none());
}

#[test]
fn hostile_references_never_panic() {
    let mut s = Session::new();
    plate(&mut s, "100 mm");
    slot(&mut s, "45 mm", 5.0);
    let long = "x".repeat(10_000);
    for r in [
        "",
        "#",
        "#1",
        "2:top#",
        "2:top#0",
        "2:top#99999999999999",
        "2:top##2",
        &long,
        "2:top#-1",
        "2:top#1#1",
        "\u{0}",
        "between 2:top and",
        "2:top 0#3",
    ] {
        for kind in [Kind::Face, Kind::Edge, Kind::Vertex] {
            for want in [Want::One, Want::Surface, Want::All] {
                let _ = s.resolve_reference(Some(2), kind, r, want);
                let _ = s.resolve_at_point(
                    kind,
                    r,
                    Some(chamfera_geom::Point3::new(1e300, 0.0, f64::NAN)),
                    want,
                );
            }
        }
        for cmd in [
            json!({"feature": 2, "face": r, "point": {"x": 1e300, "y": 0, "z": 0}}),
            json!({"feature": 2, "face": r, "point": {"x": f64::NAN}}),
            json!({"feature": 2, "face": r, "point": "here"}),
        ] {
            let _ = s.execute("sketch.create_on_face", &cmd);
        }
        let _ = s.execute("solid.fillet", &json!({"edges": [r], "radius": 1}));
        let _ = s.execute("solid.shell", &json!({"faces": [r], "thickness": 1}));
        let _ = s.execute(
            "solid.hole",
            &json!({"face": r, "points": [{"x": 1, "y": 1}], "diameter": 1, "depth": 1}),
        );
    }
}

/// Runs an example script and returns the session.
fn example(name: &str) -> Session {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap();
    let steps: Vec<Value> = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    crate::script::run_steps(&mut s, &steps).unwrap();
    no_errors(&s);
    s
}

#[test]
fn the_example_designs_with_references_build_to_their_volumes() {
    // boss-on-plate: plate 80 x 50 x 10 (40000), boss 30 x 20 x 15 on its top (9000),
    // a hole r 6 from the boss top (z 25) down through both (25 deep): - 36π · 25.
    let s = example("boss-on-plate.json");
    assert!(
        (volume(&s) - (49_000.0 - 900.0 * PI)).abs() < 1e-6,
        "{}",
        volume(&s)
    );
    // filleted-block: 60 x 40 x 15 with a hole r 8 (36000 - 960π); four vertical edges
    // rounded r 8 (each (64 - 16π) · 15); the hole's top rim chamfered 1.5: a ring of
    // triangle area 1.5²/2 at radius 8 + 1.5/3 (Pappus: 2π · 8.5 · 1.125 = 19.125π).
    let s = example("filleted-block.json");
    let v = 36_000.0 - 960.0 * PI - 4.0 * (64.0 - 16.0 * PI) * 15.0 - 19.125 * PI;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // shelled-enclosure: 80 x 50 x 30 rounded r 8 at the vertical edges, shelled 2 with
    // the top open. Outer (4000 - (4 - π) 64) · 30; cavity 76 x 46 rounded r 6, 28 deep.
    let s = example("shelled-enclosure.json");
    let v = (4000.0 - (4.0 - PI) * 64.0) * 30.0 - (3496.0 - (4.0 - PI) * 36.0) * 28.0;
    assert!((volume(&s) - v).abs() < 1e-6, "{} vs {v}", volume(&s));
    // patterned-flange: ring r 40 / r 15, 8 thick (11000π), six bolt holes r 3.5
    // through (6 · 12.25π · 8 = 588π).
    let s = example("patterned-flange.json");
    assert!((volume(&s) - 10_412.0 * PI).abs() < 1e-6, "{}", volume(&s));
    // construction-planes: plate 80 x 50 x 10 and a lid 40 x 30 x 5 on a plane offset
    // from its top: 40000 + 6000.
    let s = example("construction-planes.json");
    assert!((volume(&s) - 46_000.0).abs() < 1e-6, "{}", volume(&s));
}

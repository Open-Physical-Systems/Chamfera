//! Sheet metal commands against hand-derived values.

use crate::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

/// A sketch on XY with a 100 x 50 rectangle from the origin, its sides fixed by length
/// constraints. Returns the id of the 100 mm length constraint.
fn plate_sketch(s: &mut Session) -> u64 {
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
    let c = |s: &mut Session, c: Value| {
        s.execute("sketch.constrain", &json!({ "constraint": c }))
            .unwrap()
    };
    let long = c(
        s,
        json!({"kind": "length", "line": lines[0], "value": 100.0}),
    );
    c(
        s,
        json!({"kind": "length", "line": lines[1], "value": 50.0}),
    );
    c(
        s,
        json!({"kind": "fix", "point": pts[0], "at": {"x": 0.0, "y": 0.0}}),
    );
    s.execute("sketch.finish", &json!({})).unwrap();
    long["constraint"].as_u64().unwrap()
}

/// The 2 mm base flange of the plate, with the rule R = 2, K = 0.44.
fn base(s: &mut Session) -> Value {
    s.execute(
        "sheet.rule",
        &json!({"name": "Steel", "thickness": 2, "bend_radius": 2, "k_factor": 0.44}),
    )
    .unwrap();
    s.execute(
        "sheet.base",
        &json!({"sketch": 1, "profile": {"x": 50, "y": 25}, "rule": "Steel"}),
    )
    .unwrap()
}

/// The name of the edge from `a` to `b` (either way) of any solid.
fn edge_between(s: &mut Session, a: [f64; 3], b: [f64; 3]) -> String {
    let r = s.execute("solid.edges", &json!({})).unwrap();
    let near =
        |v: &Value, p: [f64; 3]| (0..3).all(|i| (v[i].as_f64().unwrap() - p[i]).abs() < 1e-9);
    for solid in r["solids"].as_array().unwrap() {
        for e in solid["edges"].as_array().unwrap() {
            if (near(&e["from"], a) && near(&e["to"], b))
                || (near(&e["from"], b) && near(&e["to"], a))
            {
                return e["edge"].as_str().unwrap().to_owned();
            }
        }
    }
    panic!("no edge from {a:?} to {b:?}");
}

fn volume(r: &Value) -> f64 {
    r["solids"][0]["volume"].as_f64().unwrap()
}

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} != {b}");
}

#[test]
fn a_base_flange_of_100_by_50_by_2_has_volume_10000() {
    let mut s = Session::new();
    plate_sketch(&mut s);
    let r = base(&mut s);
    close(volume(&r), 10_000.0, 1e-9);
    assert_eq!(r["name"], "BaseFlange1");
}

#[test]
fn a_flange_bends_exactly_and_its_flat_pattern_adds_the_bend_allowance() {
    let mut s = Session::new();
    plate_sketch(&mut s);
    base(&mut s);
    let edge = edge_between(&mut s, [100.0, 0.0, 2.0], [100.0, 50.0, 2.0]);
    let r = s
        .execute("sheet.flange", &json!({"edge": edge, "height": 30}))
        .unwrap();
    // Inside position (the default) trims the base back by the outside setback
    // (R + T) tan 45° = 4: base 96 * 50 * 2; the bend a quarter annulus of radii 2 and 4,
    // (π/4)(4² - 2²) * 50; the straight part (30 - 4) * 2 * 50.
    let expected = 96.0 * 50.0 * 2.0 + PI / 4.0 * (16.0 - 4.0) * 50.0 + 26.0 * 2.0 * 50.0;
    close(volume(&r), expected, 1e-6);
    let f = s.execute("sheet.flat_pattern", &json!({})).unwrap();
    // (100 - setback) + BA + (flange straight part), BA = π/2 (2 + 0.44 * 2).
    let ba = PI / 2.0 * (2.0 + 0.44 * 2.0);
    close(
        f["length"].as_f64().unwrap(),
        (100.0 - 4.0) + ba + 26.0,
        1e-9,
    );
    close(f["width"].as_f64().unwrap(), 50.0, 1e-9);
    close(f["bends"][0]["allowance"].as_f64().unwrap(), ba, 1e-12);
    close(
        f["area"].as_f64().unwrap(),
        ((100.0 - 4.0) + ba + 26.0) * 50.0,
        1e-6,
    );
    // The flat body is shown beside the part.
    assert!(s.solids().iter().any(|x| x.name == "FlatPattern1"));
    // Asking again gives the same pattern, no second feature.
    let again = s.execute("sheet.flat_pattern", &json!({})).unwrap();
    assert_eq!(again["feature"], f["feature"]);
}

#[test]
fn a_u_channel_unfolds_to_its_hand_derived_width() {
    let mut s = Session::new();
    plate_sketch(&mut s);
    base(&mut s);
    let right = edge_between(&mut s, [100.0, 0.0, 2.0], [100.0, 50.0, 2.0]);
    let left = edge_between(&mut s, [0.0, 0.0, 2.0], [0.0, 50.0, 2.0]);
    s.execute("sheet.flange", &json!({"edge": right, "height": 30}))
        .unwrap();
    s.execute(
        "sheet.flange",
        &json!({"edge": left, "height": "30 mm", "angle": 90}),
    )
    .unwrap();
    let f = s.execute("sheet.flat_pattern", &json!({})).unwrap();
    // Web 100 - 2 * 4, two bends of π/2 (2 + 0.88), two straight parts of 30 - 4.
    let width = 92.0 + 2.0 * PI / 2.0 * 2.88 + 2.0 * 26.0;
    close(f["length"].as_f64().unwrap(), width, 1e-9);
    assert_eq!(f["bends"].as_array().unwrap().len(), 2);
}

#[test]
fn a_flange_on_a_curved_edge_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 20}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "sheet.base",
        &json!({"sketch": 1, "profile": {"x": 0, "y": 0}}),
    )
    .unwrap();
    // The made "Default" rule is 2 mm.
    close(s.solids()[0].body.volume().unwrap(), PI * 400.0 * 2.0, 1e-6);
    let edges = s.execute("solid.edges", &json!({})).unwrap();
    let circle = edges["solids"][0]["edges"][0]["edge"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = s.doc.clone();
    let r = s.execute("sheet.flange", &json!({"edge": circle, "height": 30}));
    let err = r.unwrap_err().to_string();
    assert!(err.contains("straight edge"), "{err}");
    assert_eq!(s.doc, before);
    assert_eq!(s.solids().len(), 1);
    close(s.solids()[0].body.volume().unwrap(), PI * 400.0 * 2.0, 1e-6);
    // A made-up edge and a bad value are refused too.
    assert!(
        s.execute("sheet.flange", &json!({"edge": "2:nowhere", "height": 30}))
            .is_err()
    );
    assert!(
        s.execute("sheet.flange", &json!({"edge": circle, "height": -3}))
            .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn a_flange_follows_the_base_when_its_sketch_grows_and_edits_rebuild_it() {
    let mut s = Session::new();
    let long = plate_sketch(&mut s);
    base(&mut s);
    let edge = edge_between(&mut s, [0.0, 50.0, 2.0], [100.0, 50.0, 2.0]);
    let r = s
        .execute("sheet.flange", &json!({"edge": edge, "height": 30}))
        .unwrap();
    let id = r["feature"].as_u64().unwrap();
    // The flange runs along the 100 mm edge: base 100 * 46 * 2, bend over 100, wall 26.
    let v = |len: f64| len * 46.0 * 2.0 + PI / 4.0 * 12.0 * len + 26.0 * 2.0 * len;
    close(volume(&r), v(100.0), 1e-6);
    // The sketch grows to 120: the flange follows, along the whole new edge.
    s.execute("sketch.edit", &json!({"sketch": 1})).unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": long, "value": 120}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
    close(s.solids()[0].body.volume().unwrap(), v(120.0), 1e-6);
    // feature.edit: 45° instead, and a rule change follows too.
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": id, "angle": 45, "height": 20}),
        )
        .unwrap();
    // 45°: setback (R + T) tan 22.5°, bend (π/8)(16 - 4) * 120, wall 20 - setback.
    let sb = 4.0 * (PI / 8.0).tan();
    let expected = 120.0 * (50.0 - sb) * 2.0 + PI / 8.0 * 12.0 * 120.0 + (20.0 - sb) * 2.0 * 120.0;
    close(volume(&r), expected, 1e-6);
    // An edit the kernel refuses (a height below the setback) changes nothing.
    let before = s.doc.clone();
    assert!(
        s.execute("feature.edit", &json!({"feature": id, "height": 1}))
            .is_err()
    );
    assert_eq!(s.doc, before);
    // Keys of other features are refused on a flange.
    assert!(
        s.execute("feature.edit", &json!({"feature": id, "rule": "Steel"}))
            .is_err()
    );
    s.execute("sheet.rule", &json!({"name": "Steel", "thickness": 1}))
        .unwrap();
    assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
}

#[test]
fn rules_are_named_document_data_and_export_writes_dxf() {
    let mut s = Session::new();
    plate_sketch(&mut s);
    base(&mut s);
    s.execute(
        "sheet.rule",
        &json!({"name": "Alu 3", "thickness": 3, "relief": "obround"}),
    )
    .unwrap();
    let r = s.execute("sheet.rules", &json!({})).unwrap();
    assert_eq!(r["rules"].as_array().unwrap().len(), 2);
    assert_eq!(r["rules"][1]["relief"], "obround");
    assert!(
        s.execute("sheet.rule", &json!({"name": "Bad", "k_factor": 2}))
            .is_err()
    );
    assert!(
        s.execute("sheet.rule", &json!({"name": "Steel", "remove": true}))
            .is_err()
    );
    // Exporting needs a flat pattern first.
    let dir = std::env::temp_dir().join(format!("chamfera-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("flat.dxf");
    let p = path.to_string_lossy().into_owned();
    assert!(s.execute("sheet.export_flat", &json!({"path": p})).is_err());
    let edge = edge_between(&mut s, [100.0, 0.0, 2.0], [100.0, 50.0, 2.0]);
    s.execute(
        "sheet.flange",
        &json!({"edge": edge, "height": 30, "offset_start": 10, "offset_end": 10}),
    )
    .unwrap();
    s.execute("sheet.flat_pattern", &json!({})).unwrap();
    let r = s.execute("sheet.export_flat", &json!({"path": p})).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("AC1009") && text.contains("\nLINE\n") && text.ends_with("EOF\n"));
    assert!(
        r["entities"].as_u64().unwrap() > 4,
        "reliefs add outline edges: {r}"
    );
    // The design round-trips with its rules and sheet features.
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back, s.doc);
    let _ = std::fs::remove_dir_all(&dir);
}

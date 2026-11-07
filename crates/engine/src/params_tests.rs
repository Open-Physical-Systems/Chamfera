//! Parameters v2 (model parameters, rename, comments, CSV) and configurations, through
//! the commands. Expected values are worked out by hand in the comments.

use crate::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A scratch folder of its own for each test.
fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chamfera-params-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The plate: a `w` × `h` rectangle fixed at the origin with a hole of diameter `d` whose
/// centre is fixed at (20, 12), extruded `t`. Volume (w h − π d²/4) t.
fn plate(s: &mut Session) {
    run(s, "param.set", json!({"name": "w", "expression": "50 mm"}));
    run(s, "param.set", json!({"name": "h", "expression": "30 mm"}));
    run(s, "param.set", json!({"name": "d", "expression": "8 mm"}));
    run(s, "param.set", json!({"name": "t", "expression": "10 mm"}));
    run(s, "sketch.create", json!({"plane": "xy"}));
    let r = run(
        s,
        "sketch.rectangle",
        json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    );
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
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "fix", "point": pts[0], "at": {"x": 0.0, "y": 0.0}}}),
    );
    let cw = run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "length", "line": lines[0], "value": 50}}),
    )["constraint"]
        .clone();
    let ch = run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "length", "line": lines[1], "value": 30}}),
    )["constraint"]
        .clone();
    let c = run(s, "sketch.circle", json!({"cx": 20, "cy": 12, "radius": 4}));
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "fix", "point": c["centre"], "at": {"x": 20.0, "y": 12.0}}}),
    );
    let cd = run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "diameter", "curve": c["curve"], "value": 8}}),
    )["constraint"]
        .clone();
    run(
        s,
        "sketch.dimension",
        json!({"constraint": cw, "expression": "w"}),
    );
    run(
        s,
        "sketch.dimension",
        json!({"constraint": ch, "expression": "h"}),
    );
    run(
        s,
        "sketch.dimension",
        json!({"constraint": cd, "expression": "d"}),
    );
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": 1, "profiles": [{"x": 2, "y": 2}], "distance": "t"}),
    );
}

fn volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

fn plate_volume(w: f64, h: f64, d: f64, t: f64) -> f64 {
    (w * h - PI * d * d / 4.0) * t
}

fn table_row(s: &mut Session, name: &str) -> Value {
    let t = run(s, "param.list", json!({}));
    t["table"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no row {name} in {}", t["table"]))
}

#[test]
fn the_plate_has_its_hand_volume() {
    let mut s = Session::new();
    plate(&mut s);
    // (50·30 − π·16)·10 = 15000 − 160π.
    assert!(
        (volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6,
        "{}",
        volume(&s)
    );
}

#[test]
fn every_dimension_and_feature_value_is_a_model_parameter() {
    let mut s = Session::new();
    plate(&mut s);
    let row = table_row(&mut s, "Extrude1.distance");
    assert_eq!(row["kind"], "model");
    assert_eq!(row["expression"], "t");
    assert!((row["value"].as_f64().unwrap() - 10.0).abs() < 1e-12);
    assert_eq!(row["unit"], "length");
    // The user parameter t is used by the extrude's distance.
    let t = table_row(&mut s, "t");
    assert_eq!(t["used_by"], json!(["Extrude1.distance"]));
    // The sketch's three dimensions are listed too, named after the sketch.
    let list = run(&mut s, "param.list", json!({}));
    let dims: Vec<&Value> = list["table"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["name"].as_str().unwrap().starts_with("Sketch1.d"))
        .collect();
    assert_eq!(dims.len(), 3, "width, height and the hole's diameter");
    // Editing a model parameter in place drives the feature: t·2 = 20 thick.
    run(
        &mut s,
        "param.set",
        json!({"name": "Extrude1.distance", "expression": "t * 2"}),
    );
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 20.0)).abs() < 1e-6);
    // Another expression may use a model parameter: a user parameter of half the
    // thickness, 20 / 2 = 10 mm.
    run(
        &mut s,
        "param.set",
        json!({"name": "half", "expression": "Extrude1.distance / 2"}),
    );
    assert!((s.doc.parameters["half"].value - 10.0).abs() < 1e-12);
    // Comments and favourites, on both kinds.
    run(
        &mut s,
        "param.annotate",
        json!({"name": "Extrude1.distance", "comment": "plate", "favourite": true}),
    );
    run(
        &mut s,
        "param.set",
        json!({"name": "w", "comment": "overall width"}),
    );
    assert_eq!(table_row(&mut s, "Extrude1.distance")["favourite"], true);
    assert_eq!(table_row(&mut s, "w")["comment"], "overall width");
    assert_eq!(
        s.doc.parameters["w"].expression.as_deref(),
        Some("50 mm"),
        "a note alone keeps the value"
    );
}

#[test]
fn renaming_a_model_parameter_keeps_its_users_working() {
    let mut s = Session::new();
    plate(&mut s);
    run(
        &mut s,
        "param.set",
        json!({"name": "lip", "expression": "Extrude1.distance + 1 mm"}),
    );
    run(
        &mut s,
        "param.rename",
        json!({"name": "Extrude1.distance", "new_name": "thickness"}),
    );
    assert_eq!(
        s.doc.parameters["lip"].expression.as_deref(),
        Some("thickness + 1 mm")
    );
    // 10 + 1 = 11, and it still follows: t = 4 gives 5.
    assert!((s.doc.parameters["lip"].value - 11.0).abs() < 1e-12);
    run(
        &mut s,
        "param.set",
        json!({"name": "t", "expression": "4 mm"}),
    );
    assert!((s.doc.parameters["lip"].value - 5.0).abs() < 1e-12);
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 4.0)).abs() < 1e-6);
    // A user parameter renamed: the extrude (thickness = t) follows.
    run(
        &mut s,
        "param.rename",
        json!({"name": "t", "new_name": "plate_t"}),
    );
    assert_eq!(table_row(&mut s, "thickness")["expression"], "plate_t");
    run(
        &mut s,
        "param.set",
        json!({"name": "plate_t", "expression": "6 mm"}),
    );
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 6.0)).abs() < 1e-6);
    // Renaming the feature changes default names; expressions that used them follow.
    let r = s.execute(
        "feature.rename",
        &json!({"feature": 2, "name": "Base Plate"}),
    );
    assert!(r.is_ok(), "{r:?}");
    // The renamed one keeps its own name.
    assert!(table_row(&mut s, "thickness")["kind"] == "model");
    // Names already taken, or not a name, are refused.
    for bad in ["w", "thickness", "2x", "a b", ""] {
        assert!(
            s.execute("param.rename", &json!({"name": "h", "new_name": bad}))
                .is_err(),
            "{bad}"
        );
    }
    // The feature cannot be deleted while lip uses its value.
    assert!(s.execute("feature.delete", &json!({"feature": 2})).is_err());
}

#[test]
fn default_model_names_follow_a_feature_rename() {
    let mut s = Session::new();
    plate(&mut s);
    run(
        &mut s,
        "param.set",
        json!({"name": "lip", "expression": "Extrude1.distance * 3"}),
    );
    run(
        &mut s,
        "feature.rename",
        json!({"feature": 2, "name": "Base Plate"}),
    );
    assert_eq!(
        s.doc.parameters["lip"].expression.as_deref(),
        Some("BasePlate.distance * 3")
    );
    assert!((s.doc.parameters["lip"].value - 30.0).abs() < 1e-12);
}

#[test]
fn a_cycle_is_refused_with_its_names_and_errors_show_inline() {
    let mut s = Session::new();
    plate(&mut s);
    run(
        &mut s,
        "param.set",
        json!({"name": "b", "expression": "1 mm"}),
    );
    run(
        &mut s,
        "param.set",
        json!({"name": "a", "expression": "b + 1 mm"}),
    );
    let e = s
        .execute("param.set", &json!({"name": "b", "expression": "a * 2"}))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("loop") && e.contains("a") && e.contains("b"),
        "{e}"
    );
    // Through a model parameter: t → Extrude1.distance → t.
    let e = s
        .execute(
            "param.set",
            &json!({"name": "t", "expression": "Extrude1.distance"}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("t → Extrude1.distance → t"), "{e}");
    // Unknown names and unit mismatches are refused with what is wrong.
    let e = s
        .execute(
            "param.set",
            &json!({"name": "Extrude1.distance", "expression": "nope"}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown name \"nope\""), "{e}");
    let e = s
        .execute(
            "param.set",
            &json!({"name": "Extrude1.distance", "expression": "30 deg"}),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("angle") && e.contains("length"), "{e}");
    // Nothing changed: still 10 thick.
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6);
}

#[test]
fn three_configurations_give_three_hand_volumes_and_activating_rebuilds() {
    let mut s = Session::new();
    plate(&mut s);
    // Small 40 × 20, hole 6, 5 thick; Medium the base; Large 100 × 60, hole 12, 15 thick.
    let sizes = [
        ("Small", 40.0, 20.0, 6.0, 5.0),
        ("Medium", 50.0, 30.0, 8.0, 10.0),
        ("Large", 100.0, 60.0, 12.0, 15.0),
    ];
    for (name, w, h, d, t) in sizes {
        run(&mut s, "config.add", json!({"name": name}));
        for (p, v) in [("w", w), ("h", h), ("d", d), ("t", t)] {
            run(
                &mut s,
                "config.set",
                json!({"config": name, "parameter": p, "expression": format!("{v} mm")}),
            );
        }
    }
    for (name, w, h, d, t) in sizes {
        let r = run(&mut s, "config.activate", json!({"config": name}));
        let want = plate_volume(w, h, d, t);
        assert!(
            (r["volume"].as_f64().unwrap() - want).abs() < 1e-6,
            "{name}: {r}"
        );
        assert!((volume(&s) - want).abs() < 1e-6, "{name}");
    }
    // Large: (6000 − 36π)·15 = 90000 − 540π.
    assert!((volume(&s) - (90_000.0 - 540.0 * PI)).abs() < 1e-6);
    let list = run(&mut s, "config.list", json!({}));
    assert_eq!(list["columns"], json!(["d", "h", "t", "w"]));
    assert_eq!(list["active"], "Large");
    // Editing an overridden parameter while its configuration is active edits the override.
    run(
        &mut s,
        "param.set",
        json!({"name": "t", "expression": "20 mm"}),
    );
    assert_eq!(
        s.doc.configurations.get("Large").unwrap().parameters["t"],
        "20 mm"
    );
    assert_eq!(
        s.doc.parameters["t"].expression.as_deref(),
        Some("10 mm"),
        "the base keeps its own"
    );
    // The base design again.
    run(&mut s, "config.activate", json!({}));
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6);
    // Undo goes back to Large active (20 thick).
    run(&mut s, "edit.undo", json!({}));
    assert!((volume(&s) - plate_volume(100.0, 60.0, 12.0, 20.0)).abs() < 1e-6);
    // A configuration copied from another, and a bad override refused.
    run(
        &mut s,
        "config.add",
        json!({"name": "Large copy", "from": "Large"}),
    );
    assert_eq!(
        s.doc
            .configurations
            .get("Large copy")
            .unwrap()
            .parameters
            .len(),
        4
    );
    assert!(
        s.execute(
            "config.set",
            &json!({"config": "Small", "parameter": "w", "expression": "5 deg"})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "config.set",
            &json!({"config": "Small", "parameter": "Extrude1.distance", "expression": "5"})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "config.set",
            &json!({"config": "Nope", "parameter": "w", "expression": "5"})
        )
        .is_err()
    );
    assert!(
        s.execute("config.add", &json!({"name": "Small"})).is_err(),
        "taken"
    );
    // An overridden parameter cannot be removed.
    assert!(s.execute("param.remove", &json!({"name": "d"})).is_err());
    // It survives save and open.
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back.configurations, s.doc.configurations);
    // Deleting the active configuration builds the base design.
    run(&mut s, "config.delete", json!({"config": "Large"}));
    assert!(s.doc.configurations.active.is_none());
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6);
}

#[test]
fn a_configuration_suppresses_features() {
    let mut s = Session::new();
    plate(&mut s);
    run(&mut s, "config.add", json!({"name": "Blank"}));
    run(
        &mut s,
        "config.suppress",
        json!({"config": "Blank", "feature": 2, "suppressed": true}),
    );
    let r = run(&mut s, "config.activate", json!({"config": "Blank"}));
    assert!(s.solids().is_empty(), "the only feature is suppressed");
    // Nothing built: volume 0, not -0 (QA user guide pass, manufacture 8).
    let v = r["volume"].as_f64().unwrap();
    assert!(v == 0.0 && v.is_sign_positive(), "{v}");
    assert!(
        !s.doc.feature(2).unwrap().suppressed,
        "the feature's own state is kept"
    );
    run(&mut s, "config.activate", json!({"config": null}));
    assert_eq!(s.solids().len(), 1);
    assert!(
        s.execute(
            "config.suppress",
            &json!({"config": "Blank", "feature": 99, "suppressed": true})
        )
        .is_err()
    );
}

#[test]
fn export_all_writes_one_file_per_configuration() {
    let dir = scratch("export");
    let mut s = Session::new();
    plate(&mut s);
    for (name, t) in [("Small", 5), ("Medium", 10), ("Large", 15)] {
        run(&mut s, "config.add", json!({"name": name}));
        run(
            &mut s,
            "config.set",
            json!({"config": name, "parameter": "t", "expression": format!("{t} mm")}),
        );
    }
    run(&mut s, "config.activate", json!({"config": "Medium"}));
    let out = dir.join("parts");
    let r = run(
        &mut s,
        "config.export_all",
        json!({"dir": out.to_str().unwrap(), "format": "stl"}),
    );
    assert_eq!(r["files"].as_array().unwrap().len(), 3);
    for (name, t) in [("Small", 5.0), ("Medium", 10.0), ("Large", 15.0)] {
        let p = out.join(format!("{name}.stl"));
        let bytes = std::fs::read(&p).unwrap();
        let mesh = chamfera_io::stl::read(&bytes).unwrap();
        // The mesh's volume is near the exact one (the hole is faceted, so within 1 %).
        let want = plate_volume(50.0, 30.0, 8.0, t);
        assert!((mesh.signed_volume() - want).abs() < want * 0.01, "{name}");
    }
    let r = run(
        &mut s,
        "config.export_all",
        json!({"dir": out.to_str().unwrap()}),
    );
    assert_eq!(r["format"], "step");
    for name in ["Small", "Medium", "Large"] {
        assert!(out.join(format!("{name}.step")).is_file(), "{name}");
    }
    // The design is back where it was: Medium active, 10 thick.
    assert_eq!(s.doc.configurations.active.as_deref(), Some("Medium"));
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6);
    assert!(
        s.execute(
            "config.export_all",
            &json!({"dir": out.to_str().unwrap(), "format": "obj"})
        )
        .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parameters_round_trip_through_csv() {
    let dir = scratch("csv");
    let path = dir.join("p.csv");
    let mut s = Session::new();
    run(
        &mut s,
        "param.set",
        json!({"name": "w", "expression": "max(40 mm, 1 in)", "comment": "width, \"outer\""}),
    );
    run(
        &mut s,
        "param.set",
        json!({"name": "a", "expression": "30 deg", "unit": "angle"}),
    );
    run(
        &mut s,
        "param.set",
        json!({"name": "n", "value": 4, "unit": "unitless"}),
    );
    run(&mut s, "param.set", json!({"name": "v", "value": 0.1}));
    run(
        &mut s,
        "param.export_csv",
        json!({"path": path.to_str().unwrap()}),
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("name,expression,unit,comment\r\n"),
        "{text}"
    );
    assert!(
        text.contains("w,\"max(40 mm, 1 in)\",length,\"width, \"\"outer\"\"\""),
        "{text}"
    );
    let mut t = Session::new();
    let r = run(
        &mut t,
        "param.import_csv",
        json!({"path": path.to_str().unwrap()}),
    );
    assert_eq!(r["imported"].as_array().unwrap().len(), 4);
    for (name, p) in &s.doc.parameters {
        let q = &t.doc.parameters[name];
        assert_eq!(q.unit, p.unit, "{name}");
        assert_eq!(q.comment, p.comment, "{name}");
        assert_eq!(q.value, p.value, "{name}: values read back exactly");
    }
    // max(40 mm, 25.4 mm) = 40 mm; 30° = π/6; 0.1 mm stays 0.1 exactly.
    assert_eq!(t.doc.parameters["w"].value, 40.0);
    assert!((t.doc.parameters["a"].value - PI / 6.0).abs() < 1e-15);
    assert_eq!(t.doc.parameters["v"].value, 0.1);
    // Rows may refer to later rows; a bad row changes nothing.
    std::fs::write(
        &path,
        "name,expression,unit\nb,c * 2,length\nc,3 mm,length\n",
    )
    .unwrap();
    run(
        &mut t,
        "param.import_csv",
        json!({"path": path.to_str().unwrap()}),
    );
    assert_eq!(t.doc.parameters["b"].value, 6.0);
    let before = t.doc.clone();
    for bad in [
        "name,expression,unit\nz,1 mm,length\nq,nope,length\n",
        "name,expression,unit\nx,y,length\ny,x,length\n",
        "name,expression,unit\n2bad,1,length\n",
        "nonsense",
        "name,expression,unit\n\"open,1,length\n",
    ] {
        std::fs::write(&path, bad).unwrap();
        assert!(
            t.execute("param.import_csv", &json!({"path": path.to_str().unwrap()}))
                .is_err(),
            "{bad}"
        );
        assert_eq!(t.doc, before, "{bad}");
    }
    assert!(
        t.execute("param.import_csv", &json!({"path": "/dev/zero"}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_expressions_never_panic() {
    let mut s = Session::new();
    plate(&mut s);
    let long = "1+".repeat(50_000) + "1";
    let deep = "(".repeat(10_000);
    let ifs = "if(".repeat(5_000);
    for e in [
        "((((((((((((((((((((",
        "1e308*1e308",
        "-1e308 * 1e308",
        "0/0",
        "sqrt(-1)",
        "if(0/0 > 1, 1, 2)",
        "atan2(0, 0) / 0",
        long.as_str(),
        deep.as_str(),
        ifs.as_str(),
        "Extrude1.distance.",
        "Extrude1..distance",
        "a.b.c.d",
        "<=>",
        "w ==",
        "\u{0}",
    ] {
        for name in ["w", "Extrude1.distance", "Sketch1.d1"] {
            let _ = s.execute("param.set", &json!({"name": name, "expression": e}));
        }
        let _ = s.execute("config.add", &json!({"name": "C"}));
        let _ = s.execute(
            "config.set",
            &json!({"config": "C", "parameter": "w", "expression": e}),
        );
        let _ = s.execute("param.rename", &json!({"name": "w", "new_name": e}));
        let _ = s.execute("param.annotate", &json!({"name": "w", "comment": e}));
    }
    // A chain of 600 parameters is refused rather than recursing without end. Names
    // count down (q1000, q0999, …) so evaluation in name order starts at the far end.
    let mut t = Session::new();
    run(
        &mut t,
        "param.set",
        json!({"name": "q1000", "expression": "1 mm"}),
    );
    let mut chain_err = None;
    for i in 1..600 {
        if let Err(e) = t.execute(
            "param.set",
            &json!({"name": format!("q{:04}", 1000 - i), "expression": format!("q{:04} + 1 mm", 1001 - i)}),
        ) {
            chain_err = Some(e.to_string());
            break;
        }
    }
    assert!(chain_err.is_some_and(|e| e.contains("more than")));
    // Still the plate, 10 thick.
    assert!((volume(&s) - plate_volume(50.0, 30.0, 8.0, 10.0)).abs() < 1e-6);
}

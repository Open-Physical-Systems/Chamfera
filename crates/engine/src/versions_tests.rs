//! Version history, atomic saves and recovery, through the commands. Expected values are
//! worked out by hand in the comments.

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
    let d = std::env::temp_dir().join(format!("chamfera-versions-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The plate: a `w` × 30 rectangle fixed at the origin with a hole of diameter 8 whose
/// centre is fixed at (20, 12), extruded 10. Volume (30 w − 16π) · 10.
fn plate(s: &mut Session) {
    run(s, "param.set", json!({"name": "w", "expression": "50 mm"}));
    run(s, "sketch.create", json!({"plane": "xy"}));
    let r = run(
        s,
        "sketch.rectangle",
        json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    );
    let ids = |k: &str| -> Vec<u64> {
        r[k].as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect()
    };
    let (pts, lines) = (ids("points"), ids("curves"));
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
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "length", "line": lines[1], "value": 30}}),
    );
    let c = run(s, "sketch.circle", json!({"cx": 20, "cy": 12, "radius": 4}));
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "fix", "point": c["centre"], "at": {"x": 20.0, "y": 12.0}}}),
    );
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "diameter", "curve": c["curve"], "value": 8}}),
    );
    run(
        s,
        "sketch.dimension",
        json!({"constraint": cw, "expression": "w"}),
    );
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": 1, "profiles": [{"x": 2, "y": 2}], "distance": 10}),
    );
}

fn volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

/// (30 w − π 8²/4) · 10.
fn plate_volume(w: f64) -> f64 {
    (30.0 * w - PI * 16.0) * 10.0
}

fn ids(list: &Value) -> Vec<u64> {
    list["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_u64().unwrap())
        .collect()
}

#[test]
fn each_save_keeps_a_version_and_compare_lists_the_width_change_and_volume_delta() {
    let dir = scratch("compare");
    let path = dir.join("plate.chamfera");
    let mut s = Session::new();
    plate(&mut s);
    assert!((volume(&s) - plate_volume(50.0)).abs() < 1e-6);
    let first = run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    assert_eq!(first["version"], 1);
    run(
        &mut s,
        "param.set",
        json!({"name": "w", "expression": "60 mm"}),
    );
    assert!((volume(&s) - plate_volume(60.0)).abs() < 1e-6);
    let second = run(&mut s, "design.save", json!({}));
    assert_eq!(second["version"], 2);
    // Saving again without a change keeps no new version.
    let again = run(&mut s, "design.save", json!({}));
    assert_eq!(again["version"], 2);
    assert_eq!(again["version_unchanged"], true);
    let list = run(&mut s, "version.list", json!({}));
    assert_eq!(ids(&list), vec![1, 2]);
    // The summary holds the volume worked out at the save.
    let v1 = list["versions"][0]["summary"]["volume"].as_f64().unwrap();
    assert!((v1 - plate_volume(50.0)).abs() < 1e-6, "{v1}");
    assert_eq!(list["versions"][0]["summary"]["features"], 1);

    let diff = run(&mut s, "version.compare", json!({"a": 1, "b": 2}));
    let params = diff["parameters"].as_array().unwrap();
    let w = params
        .iter()
        .find(|p| p["name"] == "w")
        .unwrap_or_else(|| panic!("no w in {diff}"));
    assert_eq!(w["change"], "changed");
    assert_eq!(w["from"], "50 mm");
    assert_eq!(w["to"], "60 mm");
    assert_eq!(w["from_value"], 50.0);
    assert_eq!(w["to_value"], 60.0);
    // B − A = (30·60 − 16π)·10 − (30·50 − 16π)·10 = 300·10 = 3000: the hole cancels.
    let delta = diff["volume_delta"].as_f64().unwrap();
    assert!((delta - 3000.0).abs() < 1e-6, "{delta}");
    // The extrude itself did not change: only the sketch it follows moved.
    let changed = diff["features"]["changed"].as_array().unwrap();
    assert!(changed.iter().all(|c| c["kind"] == "sketch"), "{changed:?}");
    assert!(diff["features"]["added"].as_array().unwrap().is_empty());
    assert!(diff["features"]["removed"].as_array().unwrap().is_empty());
    // Both versions show in the viewport, A and B, until cleared.
    assert_eq!(diff["shown"], true);
    let sides: Vec<_> = s.scene.compare.iter().map(|c| c.side).collect();
    assert!(sides.contains(&chamfera_render::CompareSide::A));
    assert!(sides.contains(&chamfera_render::CompareSide::B));
    run(&mut s, "version.compare", json!({"clear": true}));
    assert!(s.scene.compare.is_empty());
    // Against the open design (the default b) nothing differs from version 2.
    let same = run(&mut s, "version.compare", json!({"a": 2, "show": false}));
    assert_eq!(same["same"], true, "{same}");
    assert!(s.scene.compare.is_empty());
}

#[test]
fn restore_gives_back_the_exact_earlier_design_as_one_undo_step() {
    let dir = scratch("restore");
    let path = dir.join("plate.chamfera");
    let mut s = Session::new();
    plate(&mut s);
    run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    let saved_v1 = s.doc.clone();
    run(
        &mut s,
        "param.set",
        json!({"name": "w", "expression": "60 mm"}),
    );
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": 1, "profiles": [{"x": 2, "y": 2}], "distance": 5}),
    );
    run(&mut s, "design.save", json!({}));
    assert_eq!(s.doc.features.len(), 2);
    let r = run(&mut s, "version.restore", json!({"id": 1}));
    assert_eq!(r["features"], 1);
    // The design is the one saved as version 1: w = 50 and one extrude, volume
    // (30·50 − 16π)·10 = 15000 − 160π.
    assert_eq!(s.doc, saved_v1);
    assert_eq!(s.doc.parameters["w"].value, 50.0);
    assert_eq!(s.doc.parameters["w"].expression.as_deref(), Some("50 mm"));
    assert_eq!(s.doc.features.len(), 1);
    assert!(
        (volume(&s) - plate_volume(50.0)).abs() < 1e-6,
        "{}",
        volume(&s)
    );
    // It is an unsaved change to the file, undone in one step.
    assert!(s.has_unsaved_changes());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.parameters["w"].value, 60.0);
    assert_eq!(s.doc.features.len(), 2);
    assert!(!s.has_unsaved_changes());
}

#[test]
fn versions_are_capped_by_count() {
    let dir = scratch("cap");
    let path = dir.join("p.chamfera");
    let mut s = Session::new();
    s.history.limits.max_count = 3;
    for i in 1..=5 {
        run(&mut s, "param.set", json!({"name": "w", "value": i}));
        run(
            &mut s,
            "design.save",
            json!({"path": path.to_str().unwrap()}),
        );
    }
    let list = run(&mut s, "version.list", json!({}));
    // Five saves, the newest three kept.
    assert_eq!(ids(&list), vec![3, 4, 5]);
}

#[test]
fn a_version_with_a_note_is_kept_listed_and_deleted() {
    let dir = scratch("note");
    let mut s = Session::new();
    // Not saved yet: nothing to list, and a version needs a file first.
    assert_eq!(run(&mut s, "version.list", json!({}))["saved"], false);
    assert!(s.execute("version.save", &json!({"note": "x"})).is_err());
    assert!(s.execute("version.restore", &json!({"id": 1})).is_err());
    let path = dir.join("p.chamfera");
    run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    run(&mut s, "param.set", json!({"name": "w", "value": 2}));
    let v = run(&mut s, "version.save", json!({"note": "wider\tplate "}));
    assert_eq!(v["version"]["id"], 2);
    assert!(!s.has_unsaved_changes(), "version.save saves the file");
    let list = run(&mut s, "version.list", json!({}));
    assert_eq!(list["versions"][1]["note"], "widerplate");
    assert_eq!(list["versions"][1]["kind"], "note");
    run(&mut s, "version.delete", json!({"id": 1}));
    assert_eq!(ids(&run(&mut s, "version.list", json!({}))), vec![2]);
    assert!(s.execute("version.delete", &json!({"id": 1})).is_err());
    let long = "n".repeat(501);
    assert!(s.execute("version.save", &json!({"note": long})).is_err());
}

#[test]
fn a_broken_snapshot_cannot_be_restored_and_changes_nothing() {
    let dir = scratch("broken");
    let path = dir.join("p.chamfera");
    let mut s = Session::new();
    run(&mut s, "param.set", json!({"name": "w", "value": 1}));
    run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    let history = dir.join("p.chamfera.history");
    std::fs::write(history.join("v000001.chamfera.z"), b"zeros\0\0\0\0").unwrap();
    let before = s.doc.clone();
    assert!(s.execute("version.restore", &json!({"id": 1})).is_err());
    assert!(s.execute("version.compare", &json!({"a": 1})).is_err());
    assert_eq!(s.doc, before);
    // A history folder that is a file is reported by the save, which itself succeeds.
    let other = dir.join("q.chamfera");
    std::fs::write(dir.join("q.chamfera.history"), b"not a folder").unwrap();
    let r = run(
        &mut s,
        "design.save",
        json!({"path": other.to_str().unwrap()}),
    );
    assert!(r["history_error"].is_string(), "{r}");
    assert!(other.exists());
}

#[test]
fn a_save_replaces_the_file_whole_and_leaves_no_temporary_file() {
    let dir = scratch("atomic");
    let path = dir.join("p.chamfera");
    let mut s = Session::new();
    s.history.enabled = false;
    run(&mut s, "param.set", json!({"name": "w", "value": 1}));
    run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    // A crash during the next save: the new design is staged but never renamed in.
    run(&mut s, "param.set", json!({"name": "w", "value": 2}));
    let text = s.doc.to_json().unwrap();
    let staged = chamfera_io::atomic::stage(&path, text.as_bytes()).unwrap();
    let old = chamfera_io::open_design(&path).unwrap();
    assert_eq!(old.parameters["w"].value, 1.0);
    std::fs::remove_file(staged).unwrap();
    run(&mut s, "design.save", json!({}));
    assert_eq!(
        chamfera_io::open_design(&path).unwrap().parameters["w"].value,
        2.0
    );
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["p.chamfera".to_owned()]);
}

#[test]
fn recovered_work_is_unsaved_and_saves_to_its_own_file() {
    let dir = scratch("recover");
    let design = dir.join("p.chamfera");
    let mut s = Session::new();
    run(&mut s, "param.set", json!({"name": "w", "value": 1}));
    run(
        &mut s,
        "design.save",
        json!({"path": design.to_str().unwrap()}),
    );
    // Work after the save, autosaved to a recovery file, then a crash.
    run(&mut s, "param.set", json!({"name": "w", "value": 7}));
    let rec_dir = dir.join("recovery");
    let entry = chamfera_io::recovery::write(&rec_dir, "k-1", &s.doc, Some(&design)).unwrap();
    let mut fresh = Session::new();
    let r = run(
        &mut fresh,
        "design.recover",
        json!({"path": entry.file.to_str().unwrap(), "design_path": design.to_str().unwrap()}),
    );
    assert_eq!(r["unsaved"], true);
    assert_eq!(fresh.doc.parameters["w"].value, 7.0);
    assert_eq!(fresh.path.as_deref(), Some(design.as_path()));
    run(&mut fresh, "design.save", json!({}));
    assert_eq!(
        chamfera_io::open_design(&design).unwrap().parameters["w"].value,
        7.0
    );
    // A broken recovery file is refused and the open design stays.
    std::fs::write(&entry.file, b"\0\0\0\0").unwrap();
    assert!(
        fresh
            .execute(
                "design.recover",
                &json!({"path": entry.file.to_str().unwrap()})
            )
            .is_err()
    );
    assert_eq!(fresh.doc.parameters["w"].value, 7.0);
}

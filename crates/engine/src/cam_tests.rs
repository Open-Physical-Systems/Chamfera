//! `cam.*` commands against hand-derived values, on examples/cam-plate.json.

use super::*;
use serde_json::json;

/// A session that has run examples/cam-plate.json: a 100 x 60 x 12 plate with four
/// Ø6 through holes and a 50 x 30 pocket 5 deep round a Ø12 island, and its CAM setup.
fn hole_plate() -> Session {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/cam-plate.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    s
}

#[test]
fn drilling_finds_every_vertical_hole_of_the_hole_plate_example() {
    // examples/hole-plate.json: an 80 x 50 x 10 plate with four Ø6.6 counterbored
    // through holes at (10, 10), (70, 10), (70, 40), (10, 40), a Ø6 countersunk through
    // hole at (40, 25) and two Ø5 blind holes 6 deep at (25, 25) and (55, 25). A
    // counterbore is drilled at its smaller (through) diameter.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/hole-plate.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    let t = s
        .execute("cam.tool_add", &json!({"kind": "drill", "diameter": 4}))
        .unwrap()["tool"]
        .clone();
    let r = s
        .execute(
            "cam.op_add",
            &json!({"kind": "drill", "tool": t, "peck": 2}),
        )
        .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    let mut holes: Vec<(f64, f64, f64)> = r["holes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["x"].as_f64().unwrap(),
                h["y"].as_f64().unwrap(),
                (h["diameter"].as_f64().unwrap() * 1000.0).round() / 1000.0,
            )
        })
        .collect();
    holes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let want = [
        (10.0, 10.0, 6.6),
        (10.0, 40.0, 6.6),
        (25.0, 25.0, 5.0),
        (40.0, 25.0, 6.0),
        (55.0, 25.0, 5.0),
        (70.0, 10.0, 6.6),
        (70.0, 40.0, 6.6),
    ];
    assert_eq!(holes.len(), want.len(), "{holes:?}");
    for (h, w) in holes.iter().zip(want) {
        assert!(
            (h.0 - w.0).abs() < 1e-9 && (h.1 - w.1).abs() < 1e-9 && (h.2 - w.2).abs() < 1e-9,
            "{h:?} vs {w:?}"
        );
    }
    // The blind holes stop 6 below the top (z 10): bottom at 4; the through holes at 0.
    for h in r["holes"].as_array().unwrap() {
        let blind = (h["diameter"].as_f64().unwrap() - 5.0).abs() < 1e-9;
        let bottom = h["bottom"].as_f64().unwrap();
        assert!(bottom <= if blind { 4.0 + 1e-9 } else { 1e-9 }, "{h}");
        assert!(bottom >= if blind { 3.0 } else { -1e-9 }, "{h}");
    }
}

#[test]
fn drilling_finds_every_vertical_hole_of_the_cam_plate() {
    let mut s = hole_plate();
    let list = s.execute("cam.list", &json!({})).unwrap();
    let drill = list["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "drill")
        .unwrap()
        .clone();
    let mut holes: Vec<(f64, f64)> = drill["holes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            assert!((h["diameter"].as_f64().unwrap() - 6.0).abs() < 1e-9);
            (h["x"].as_f64().unwrap(), h["y"].as_f64().unwrap())
        })
        .collect();
    holes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // The four holes of the sketch; the Ø12 island is a boss, not a hole.
    assert_eq!(
        holes,
        vec![(8.0, 8.0), (8.0, 52.0), (92.0, 8.0), (92.0, 52.0)]
    );
    assert!(drill.get("error").is_none(), "{drill}");
}

#[test]
fn toolpaths_follow_the_model_when_it_changes() {
    // Deepening the pocket cut from 5 to 8 moves its floor from 12 - 5 = 7 down to 4:
    // the pocket toolpath's lowest cut follows.
    let mut s = hole_plate();
    let lowest = |s: &Session| {
        let op = s
            .doc
            .cam
            .operations
            .iter()
            .find(|o| o.op.key() == "pocket")
            .unwrap();
        let tp = s.toolpath(op.id).unwrap().as_ref().unwrap();
        tp.polylines(0.1)
            .1
            .iter()
            .flat_map(|l| [l[0].z, l[1].z])
            .fold(f64::INFINITY, f64::min)
    };
    assert!((lowest(&s) - 7.0).abs() < 1e-9);
    s.execute("feature.edit", &json!({"feature": 4, "distance": 8}))
        .unwrap();
    assert!((lowest(&s) - 4.0).abs() < 1e-9, "{}", lowest(&s));
    // Undo brings the old floor back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((lowest(&s) - 7.0).abs() < 1e-9);
}

#[test]
fn the_cam_section_saves_posts_and_refuses_what_it_cannot_cut() {
    let mut s = hole_plate();
    // It survives a save and reopen (format 20).
    let text = s.doc.to_json().unwrap();
    let back = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(back.cam, s.doc.cam);
    assert_eq!(back.version, 20);
    // G-code for both dialects; GRBL has no canned cycles or tool changes.
    let lin = s.execute("cam.post", &json!({})).unwrap();
    let g = lin["gcode"].as_str().unwrap();
    assert!(g.starts_with("(Chamfera Setup 1)\nG21 G90 G17\n"), "{g}");
    assert!(g.contains("G98 G83") && g.contains("T2 M6"));
    let grbl = s.execute("cam.post", &json!({"dialect": "grbl"})).unwrap();
    let g = grbl["gcode"].as_str().unwrap();
    assert!(!g.contains("G81") && !g.contains("G83") && !g.contains("M6"));
    // Written to a file.
    let path = std::env::temp_dir().join(format!("chamfera-cam-{}.nc", std::process::id()));
    s.execute("cam.post", &json!({"path": path.to_string_lossy()}))
        .unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(written, lin["gcode"].as_str().unwrap());
    // The plate's top is open on every side: not a pocket. The operation keeps its
    // error and posting refuses until it is fixed.
    let tool = s.doc.cam.tools[1].id;
    let r = s
        .execute(
            "cam.op_add",
            &json!({"kind": "pocket", "tool": tool, "face": "2:top"}),
        )
        .unwrap();
    assert!(r["error"].as_str().unwrap().contains("open"), "{r}");
    assert!(s.execute("cam.post", &json!({})).is_err());
    // A bad value changes nothing.
    let before = s.doc.clone();
    assert!(
        s.execute(
            "cam.op_add",
            &json!({"kind": "facing", "tool": tool, "stepover": 150})
        )
        .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn hostile_cam_params_never_panic() {
    let long = "x".repeat(100_000);
    let hostile = [
        json!({"kind": "pocket", "tool": 2, "face": long, "stepover": 1e-300, "stepdown": 1e-300}),
        json!({"kind": "facing", "tool": 2, "stepover": 0.0001, "stepdown": 1e-9}),
        json!({"kind": "contour", "tool": 3, "edges": ["between 2:top and 2:side 0", 5], "side": "up"}),
        json!({"kind": "contour", "tool": 3, "edges": [long], "lead_radius": -1}),
        json!({"kind": "drill", "tool": 4, "faces": "all", "peck": 0, "retract": f64::MAX}),
        json!({"kind": "drill", "tool": 4, "peck": "1e9 mm", "breakthrough": "((("}),
        json!({"kind": "pocket", "tool": 3, "face": "4:top", "entry": "dive", "ramp_angle": 90}),
        json!({"kind": "pocket", "tool": 3, "face": "4:top", "helix_radius": 1e9, "ramp_angle": 1e-9}),
        json!({"operation": 6, "suppressed": "no", "tool": 99, "setup": 0}),
        json!({"operation": 6, "stepover": 100, "stepdown": 0.0001}),
        json!({"setup": 1, "stock": {"kind": "fixed", "x": 0, "y": -1, "z": 1e300}}),
        json!({"stock": {"kind": "from_model", "side": f64::MAX}, "origin": {"x": 1e300}}),
        json!({"stock": "big", "origin": "moon", "x_axis": 5, "clearance": -1, "rapid_rate": 0}),
        json!({"x_axis": "between 2:top and 2:side 0", "origin": "stock_top_centre"}),
        json!({"kind": "flat_end_mill", "diameter": 1e300, "flutes": 1e9, "number": -1}),
        json!({"kind": "bull_nose", "diameter": 6, "corner_radius": 4, "tip_angle": 1e300}),
        json!({"kind": "drill", "diameter": "0 mm", "feed": f64::NAN, "plunge_feed": -1}),
        json!({"dialect": "fanuc", "setup": 99, "path": "/nonexistent/dir/x.nc"}),
        json!({"visible": "yes", "operation": 4294967296u64}),
        json!(null),
        json!([1, 2]),
    ];
    let cam_ids: Vec<&str> = registry()
        .into_iter()
        .map(|c| c.id)
        .filter(|id| id.starts_with("cam."))
        .collect();
    assert!(cam_ids.len() >= 8);
    let base = hole_plate();
    for id in cam_ids {
        for p in &hostile {
            let mut s = Session::new();
            s.doc = base.doc.clone();
            s.rebuild();
            let before = s.doc.clone();
            if s.execute(id, p).is_err() {
                assert_eq!(s.doc, before, "{id} {p} failed but changed the design");
            }
        }
    }
}

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
    // It survives a save and reopen (the CAM section came with format 20; files are
    // written at the current version).
    let text = s.doc.to_json().unwrap();
    let back = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(back.cam, s.doc.cam);
    assert_eq!(back.version, chamfera_doc::FORMAT_VERSION);
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
    // The pocket left two pieces of the plate's top at z 12: the frame round the pocket
    // and the island's top. A pocket needs one floor, so the plain name says to pick
    // (rung K8).
    let tool = s.doc.cam.tools[1].id;
    let r = s
        .execute(
            "cam.op_add",
            &json!({"kind": "pocket", "tool": tool, "face": "2:top"}),
        )
        .unwrap();
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("split into 2 pieces; pick one: \"2:top#1\", \"2:top#2\""),
        "{r}"
    );
    s.execute("edit.undo", &json!({})).unwrap();
    // The frame (piece 1: its anchor, x 50, comes before the island's, x 56) is open on
    // every side: not a pocket. The operation keeps its error and posting refuses until
    // it is fixed.
    let r = s
        .execute(
            "cam.op_add",
            &json!({"kind": "pocket", "tool": tool, "face": "2:top#1"}),
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
        json!({"kind": "parallel", "tool": 3, "scallop": 1e9, "angle": "(((", "tolerance": 0}),
        json!({"kind": "parallel", "tool": 2, "stepover": 1e-9, "tolerance": 1e-300, "stock_to_leave": -1}),
        json!({"kind": "waterline", "tool": 3, "tolerance": 1e9, "stepdown": 1e-9}),
        json!({"kind": "adaptive", "tool": 3, "max_engagement": 0, "stock_to_leave": 1e300}),
        json!({"kind": "adaptive", "tool": 4, "max_engagement": "1e9 deg", "stepdown": -2}),
        json!({"resolution": 1e9, "time": 2, "ops": [99], "tolerance": -1}),
        json!({"resolution": 0, "time": f64::NAN, "ops": "all", "clear": "yes"}),
        json!({"resolution": 8, "time": 0.5, "ops": [4294967296u64, -1], "setup": 99}),
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

// ---- 3D operations and the stock simulation ----

/// A 50 x 30 x 10 block with a 30 x 14 pocket 4 deep in the middle of its top (x 10..40,
/// y 8..22, floor at z = 6), the stock exactly the block, a 6 mm flat end mill.
fn pocket_block() -> (Session, serde_json::Value) {
    let steps = json!([
        {"command": "sketch.create", "params": {"plane": "xy"}, "as": "base"},
        {"command": "sketch.rectangle", "params": {"x1": 0, "y1": 0, "x2": 50, "y2": 30}},
        {"command": "sketch.finish"},
        {"command": "solid.extrude", "params": {"sketch": "$base.sketch", "profiles": [{"x": 25, "y": 15}], "distance": 10}, "as": "block"},
        {"command": "sketch.create_on_face", "params": {"feature": "$block.feature", "face": "top"}, "as": "pocket"},
        {"command": "sketch.rectangle", "params": {"x1": 10, "y1": 8, "x2": 40, "y2": 22}},
        {"command": "sketch.finish"},
        {"command": "solid.extrude", "params": {"sketch": "$pocket.sketch", "profiles": [{"x": 25, "y": 15}], "distance": 4, "direction": "reversed", "operation": "cut"}},
        {"command": "cam.setup", "params": {"stock": {"kind": "fixed", "x": 50, "y": 30, "z": 10}}},
        {"command": "cam.tool_add", "params": {"kind": "flat_end_mill", "diameter": 6}, "as": "mill"}
    ]);
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    let tool = s.doc.cam.tools[0].id;
    (s, json!(tool))
}

#[test]
fn simulating_a_pocket_removes_the_pocket_to_within_the_dexel_tolerance() {
    // The pocket op clears the 30 x 14 floor 4 deep except where a 6 mm tool cannot
    // reach: each of the four sharp corners keeps r² (1 - π/4) with r = 3. Removed:
    // V = 4 (420 - 36 (1 - π/4)).
    //
    // Dexel tolerance: each column is cut exactly at its centre, so a column is wrong
    // only where the cut region's boundary passes through it: within half a cell's
    // diagonal h/√2 of the boundary, a band of area at most 2 (h/√2) L (L its length,
    // here at most the pocket's perimeter 88) plus the four corners' π h²/2. Times the
    // depth: |error| <= 4 (√2 h L + 2π h²).
    let (mut s, tool) = pocket_block();
    let r = s
        .execute(
            "cam.op_add",
            &json!({"kind": "pocket", "tool": tool, "face": "4:top", "stepdown": 2}),
        )
        .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    let sim = s
        .execute("cam.simulate", &json!({"resolution": 500}))
        .unwrap();
    let h = sim["cell"].as_f64().unwrap();
    assert!((h - 0.1).abs() < 1e-12, "{sim}");
    let want = 4.0 * (420.0 - 36.0 * (1.0 - std::f64::consts::FRAC_PI_4));
    let tol = 4.0 * (2f64.sqrt() * h * 88.0 + 2.0 * std::f64::consts::PI * h * h);
    let got = sim["removed_volume"].as_f64().unwrap();
    assert!((got - want).abs() <= tol, "{got} vs {want} ± {tol}");
    assert_eq!(sim["gouges"]["cells"], 0, "{sim}");
    // The rest is the corners: 4 · 36 (1 - π/4) · ... within the same tolerance of the
    // corner material, 4 mm deep: 4 · 4 · 9 (1 - π/4).
    let rest = sim["rest_volume"].as_f64().unwrap();
    let corners = 4.0 * 36.0 * (1.0 - std::f64::consts::FRAC_PI_4);
    assert!((rest - corners).abs() <= tol, "{rest} vs {corners}");
    // The stock shows in the viewport, in place of the bodies.
    assert!(!s.scene.stock.is_empty());
    // Halfway through the cycle time less is gone; at 0 nothing.
    let half = s
        .execute("cam.simulate", &json!({"resolution": 500, "time": 0.5}))
        .unwrap();
    let v = half["removed_volume"].as_f64().unwrap();
    assert!(v > 0.0 && v < got, "{half}");
    let none = s
        .execute("cam.simulate", &json!({"resolution": 500, "time": 0}))
        .unwrap();
    assert!(none["removed_volume"].as_f64().unwrap().abs() < 1e-9);
    s.execute("cam.simulate", &json!({"clear": true})).unwrap();
    assert!(s.scene.stock.is_empty());
}

#[test]
fn a_contour_cut_below_the_pocket_floor_shows_as_a_gouge() {
    // The pocket's inside contour cut 1 mm below its floor (bottom_offset 1) cuts into the
    // part along the walls: gouges, the deepest 1 mm.
    let (mut s, tool) = pocket_block();
    s.execute(
        "cam.op_add",
        &json!({"kind": "contour", "tool": tool, "face": "4:top", "side": "inside",
                "bottom_offset": 1, "stepdown": 5}),
    )
    .unwrap();
    let sim = s
        .execute(
            "cam.simulate",
            &json!({"resolution": 250, "tolerance": 0.02}),
        )
        .unwrap();
    assert!(sim["gouges"]["cells"].as_u64().unwrap() > 0, "{sim}");
    assert!(
        (sim["gouges"]["max_depth"].as_f64().unwrap() - 1.0).abs() < 1e-9,
        "{sim}"
    );
    assert!(
        s.scene
            .stock
            .iter()
            .any(|m| m.style == chamfera_render::StockStyle::Gouge)
    );
}

#[test]
fn three_d_operations_are_added_generated_and_their_resolutions_capped() {
    // A ball end mill: parallel finishing at a 0.01 mm scallop over the pocket block
    // (stepover 2 sqrt(2·3·0.01 - 0.0001) = 0.4895 mm), a waterline and an adaptive with
    // the flat mill. Each generates without an error and simulates without a gouge.
    let (mut s, flat) = pocket_block();
    let ball = s
        .execute(
            "cam.tool_add",
            &json!({"kind": "ball_end_mill", "diameter": 6}),
        )
        .unwrap()["tool"]
        .clone();
    let a = s
        .execute(
            "cam.op_add",
            &json!({"kind": "adaptive", "tool": flat, "stepdown": 2, "stock_to_leave": 0}),
        )
        .unwrap();
    assert!(a.get("error").is_none(), "{a}");
    let p = s
        .execute(
            "cam.op_add",
            &json!({"kind": "parallel", "tool": ball, "scallop": 0.01, "angle": 90}),
        )
        .unwrap();
    assert!(p.get("error").is_none(), "{p}");
    let w = s
        .execute(
            "cam.op_add",
            &json!({"kind": "waterline", "tool": ball, "stepdown": 1}),
        )
        .unwrap();
    assert!(w.get("error").is_none(), "{w}");
    // A scallop needs a ball.
    let bad = s
        .execute(
            "cam.op_add",
            &json!({"kind": "parallel", "tool": flat, "scallop": 0.01}),
        )
        .unwrap();
    assert!(bad["error"].as_str().unwrap().contains("ball"), "{bad}");
    let id = bad["operation"].clone();
    s.execute("cam.op_delete", &json!({"operation": id}))
        .unwrap();
    let sim = s
        .execute("cam.simulate", &json!({"resolution": 200}))
        .unwrap();
    assert_eq!(sim["gouges"]["cells"], 0, "{sim}");
    assert!(
        s.execute("cam.simulate", &json!({"resolution": 1025}))
            .is_err()
    );
    // The design file keeps the 3D operations.
    let text = s.doc.to_json().unwrap();
    assert!(text.contains("\"parallel\"") && text.contains("\"adaptive\""));
    let back = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(back.cam, s.doc.cam);
}

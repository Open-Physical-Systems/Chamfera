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

/// QA user guide pass: nothing said that a drill and its holes do not match. The
/// example drills its four Ø6 holes with a Ø5 drill: the toolpath stands, but the
/// replies carry a warning, named after the operation in cam.generate. A Ø6 drill
/// fits (no warning); the Ø6 flat end mill is not a drill (a warning).
#[test]
fn a_drill_that_does_not_match_its_holes_is_warned_about() {
    let mut s = hole_plate();
    let r = s.execute("cam.generate", &json!({})).unwrap();
    assert_eq!(r["errors"], 0, "{r}");
    assert_eq!(
        r["warnings"],
        json!(["Drill 1: the 5 mm tool is smaller than 4 6 mm holes: they would come out 5 mm"])
    );
    let list = s.execute("cam.list", &json!({})).unwrap();
    let tool = |kind: &str, d: f64| {
        list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["kind"] == kind && t["diameter"] == json!(d))
            .unwrap()["tool"]
            .clone()
    };
    let (drill, mill) = (tool("drill", 5.0), tool("flat_end_mill", 6.0));
    let op = list["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "drill")
        .unwrap()["operation"]
        .clone();
    s.execute("cam.tool_edit", &json!({"tool": drill, "diameter": 6}))
        .unwrap();
    let r = s.execute("cam.generate", &json!({})).unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    let r = s
        .execute("cam.op_edit", &json!({"operation": op, "tool": mill}))
        .unwrap();
    assert_eq!(
        r["warnings"],
        json!(["the tool is a flat end mill, not a drill: drilling plunges it straight down"])
    );
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

#[test]
fn the_simulation_knows_the_part_height_over_every_column_of_a_filleted_block() {
    // examples/cam-3d-finish.json: a 60 x 40 x 20 block with its four top edges filleted
    // at radius 8 (two pairs of cylinders lying along X and Y, mitred at the corners).
    // With dx = max(0, 8 - x, x - 52) and dy = max(0, 8 - y, y - 32) into the fillets,
    // the top is at 12 + sqrt(64 - max(dx, dy)²), including on the mitre diagonals (where
    // a column lies on the edge between two fillets).
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/cam-3d-finish.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    // The block, its setup and tools; a facing op (quick to make) to simulate.
    script::run_steps(&mut s, &steps.as_array().unwrap()[..8]).unwrap();
    let tool = s.doc.cam.tools[0].id;
    s.execute("cam.op_add", &json!({"kind": "facing", "tool": tool}))
        .unwrap();
    s.execute("cam.simulate", &json!({"resolution": 128}))
        .unwrap();
    let sim = s.simulation().unwrap();
    let d = &sim.blank;
    let mut checked = 0;
    for j in 0..d.ny {
        for i in 0..d.nx {
            let c = d.centre(i, j);
            if !(c.x > 1e-6 && c.x < 60.0 - 1e-6 && c.y > 1e-6 && c.y < 40.0 - 1e-6) {
                continue;
            }
            let dx = 0f64.max(8.0 - c.x).max(c.x - 52.0);
            let dy = 0f64.max(8.0 - c.y).max(c.y - 32.0);
            let want = 12.0 + (64.0 - dx.max(dy).powi(2)).sqrt();
            let got = sim.heights[j * d.nx + i];
            assert!(
                got.is_some_and(|h| (h - want).abs() < 1e-6),
                "{c:?}: {got:?} vs {want}"
            );
            checked += 1;
        }
    }
    assert!(checked > 8_000);
}

// ---- The toolpath cache, background generation, holders, pencil, tool libraries ----

/// Each operation's toolpath, by operation id.
fn paths(s: &Session) -> std::collections::BTreeMap<u32, Result<chamfera_cam::Toolpath, String>> {
    s.cam.paths.clone()
}

#[test]
fn the_toolpath_cache_hits_after_a_far_edit_and_misses_after_an_edit_to_the_machined_face() {
    // examples/cam-plate.json: facing, a pocket on 4:top (x 25..75, y 15..45), drilling
    // every hole and a contour round 2:bottom. A 10 x 16 slot 2 deep at x 85..95,
    // y 22..38 is far from the pocket (none of its edges reaches the pocket's box), does
    // not touch the bottom face and leaves the model's box as it was; it adds no hole.
    // So facing, the pocket and the contour are taken from the cache; drilling reads the
    // whole model and is made again. Deepening the pocket changes its floor (a miss) but
    // not the box or the bottom face; drilling is made again.
    let mut s = hole_plate();
    let mut fresh = hole_plate();
    fresh.set_rebuild_cache(false);
    let ops: Vec<(String, u32)> = s
        .doc
        .cam
        .operations
        .iter()
        .map(|o| (o.op.key().to_owned(), o.id))
        .collect();
    let id = |k: &str| ops.iter().find(|o| o.0 == k).map(|o| o.1).unwrap();
    let before = paths(&s);
    let slot = json!([
        {"command": "sketch.create_on_face", "params": {"feature": 2, "face": "top"}, "as": "slot"},
        {"command": "sketch.rectangle", "params": {"x1": 85, "y1": 22, "x2": 95, "y2": 38}},
        {"command": "sketch.finish"},
        {"command": "solid.extrude", "params": {"sketch": "$slot.sketch", "profiles": [{"x": 90, "y": 30}], "distance": 2, "direction": "reversed", "operation": "cut"}}
    ]);
    for t in [&mut s, &mut fresh] {
        script::run_steps(t, slot.as_array().unwrap()).unwrap();
    }
    let st = s.toolpath_cache_stats();
    assert_eq!((st.last_hits, st.last_misses), (3, 1), "{st:?}");
    let after = paths(&s);
    for k in ["facing", "pocket", "contour"] {
        assert_eq!(after.get(&id(k)), before.get(&id(k)), "{k}");
    }
    // Cached equals freshly generated.
    assert_eq!(after, paths(&fresh));
    // An edit to the machined face: the pocket 8 deep instead of 5.
    for t in [&mut s, &mut fresh] {
        t.execute("feature.edit", &json!({"feature": 4, "distance": 8}))
            .unwrap();
    }
    let st = s.toolpath_cache_stats();
    assert_eq!((st.last_hits, st.last_misses), (2, 2), "{st:?}");
    assert_ne!(paths(&s).get(&id("pocket")), after.get(&id("pocket")));
    assert_eq!(paths(&s), paths(&fresh));
    // Undo: every toolpath of the slotted plate is still cached.
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.toolpath_cache_stats();
    assert_eq!((st.last_hits, st.last_misses), (4, 0), "{st:?}");
    assert_eq!(paths(&s), after);
}

/// The pocket block with a ball end mill's parallel and pencil passes (coarse, to be
/// quick).
fn block_with_3d_ops() -> Session {
    let (mut s, _) = pocket_block();
    let ball = s
        .execute(
            "cam.tool_add",
            &json!({"kind": "ball_end_mill", "diameter": 6}),
        )
        .unwrap()["tool"]
        .clone();
    for p in [
        json!({"kind": "parallel", "tool": ball, "stepover": 50, "tolerance": 0.05}),
        json!({"kind": "pencil", "tool": ball, "tolerance": 0.05}),
    ] {
        let r = s.execute("cam.op_add", &p).unwrap();
        assert!(r.get("error").is_none(), "{r}");
    }
    s
}

#[test]
fn background_generation_gives_the_same_toolpaths_as_synchronous_generation() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut sync = block_with_3d_ops();
    let mut bg = block_with_3d_ops();
    sync.set_toolpath_cache(false);
    bg.set_cam_background(true);
    let latest = Arc::new(AtomicU64::new(0));
    let run = |job: crate::cam_cache::CamJob, latest: &Arc<AtomicU64>| {
        let token = chamfera_cam::cancel::Token {
            latest: latest.clone(),
            generation: job.generation,
        };
        std::thread::spawn(move || chamfera_cam::cancel::with(token, || job.run()))
            .join()
            .unwrap()
    };
    // A model edit: the pocket 6 deep. The background session keeps its old toolpaths,
    // stale (drawn dimmed), and leaves a job.
    for (k, t) in [&mut sync, &mut bg].into_iter().enumerate() {
        t.execute("feature.edit", &json!({"feature": 4, "distance": 6}))
            .unwrap();
        if k == 1 {
            assert!(t.cam_regenerating());
        }
    }
    let styles: Vec<_> = bg.scene.overlays.iter().map(|o| o.style).collect();
    assert!(styles.contains(&chamfera_render::OverlayStyle::ToolpathStale));
    let first = bg.take_cam_job().unwrap();
    assert!(bg.take_cam_job().is_none());
    // A newer edit before the worker ran the first job: the first is cancelled.
    for t in [&mut sync, &mut bg] {
        t.execute("feature.edit", &json!({"feature": 4, "distance": 7}))
            .unwrap();
    }
    let second = bg.take_cam_job().unwrap();
    latest.store(bg.cam_generation(), Ordering::SeqCst);
    let old = run(first, &latest);
    assert!(old.cancelled);
    assert!(!bg.accept_cam_result(old));
    assert!(bg.cam_regenerating());
    let new = run(second, &latest);
    assert!(!new.cancelled);
    assert!(bg.accept_cam_result(new));
    assert!(!bg.cam_regenerating());
    assert_eq!(paths(&bg), paths(&sync));
    let styles: Vec<_> = bg.scene.overlays.iter().map(|o| o.style).collect();
    assert!(!styles.contains(&chamfera_render::OverlayStyle::ToolpathStale));
    // The pure snapshot function gives the same again.
    let bodies: Vec<chamfera_kernel::Body> = sync.solids().iter().map(|x| x.body.clone()).collect();
    assert_eq!(
        crate::cam_cache::generate_snapshot(&sync.doc.cam, &bodies),
        paths(&sync)
    );
    // A command that reads toolpaths never waits for the worker: with a job pending it
    // makes the stale ones itself, the same as the synchronous session.
    for t in [&mut sync, &mut bg] {
        t.execute("feature.edit", &json!({"feature": 4, "distance": 5}))
            .unwrap();
    }
    assert!(bg.cam_regenerating());
    let a = sync.execute("cam.post", &json!({})).unwrap();
    let b = bg.execute("cam.post", &json!({})).unwrap();
    assert_eq!(a, b);
    assert!(!bg.cam_regenerating());
}

#[test]
fn a_short_tool_in_a_deep_pocket_hits_the_stock_with_its_holder() {
    // A 60 x 40 x 30 block with a 20 x 20 pocket 20 deep (x 20..40, y 10..30, floor at
    // z = 10), the stock the block. A 6 mm flat end mill with 8 mm flutes sticks 12 mm
    // out of a Ø30 holder: the holder's face is 12 above the tip, its radius 15.
    // Wherever the tool is in the pocket (its axis within 10 - 3 = 7 of the pocket's
    // middle) the holder reaches past the walls (7 + 15 > 10) over the uncut top at 30.
    // Cutting at 2 mm steps down from 30, the levels 28 .. 18 keep the face at 30 or
    // above; from 16 down it is below the top: at the floor (tip at 10, face at 22) the
    // stock stands 30 - 22 = 8 mm into the holder. The shank (r 3 = the tool's, from 8 to
    // 12 above the tip) only meets material the levels above have cut (2 below it).
    // Sticking out 25 mm the face stays at 35 or above: no collision at all.
    let steps = json!([
        {"command": "sketch.create", "params": {"plane": "xy"}, "as": "base"},
        {"command": "sketch.rectangle", "params": {"x1": 0, "y1": 0, "x2": 60, "y2": 40}},
        {"command": "sketch.finish"},
        {"command": "solid.extrude", "params": {"sketch": "$base.sketch", "profiles": [{"x": 5, "y": 5}], "distance": 30}, "as": "block"},
        {"command": "sketch.create_on_face", "params": {"feature": "$block.feature", "face": "top"}, "as": "pocket"},
        {"command": "sketch.rectangle", "params": {"x1": 20, "y1": 10, "x2": 40, "y2": 30}},
        {"command": "sketch.finish"},
        {"command": "solid.extrude", "params": {"sketch": "$pocket.sketch", "profiles": [{"x": 30, "y": 20}], "distance": 20, "direction": "reversed", "operation": "cut"}, "as": "cut"},
        {"command": "cam.setup", "params": {"stock": {"kind": "fixed", "x": 60, "y": 40, "z": 30}}},
        {"command": "cam.tool_add", "params": {"kind": "flat_end_mill", "diameter": 6, "flute_length": 8,
            "holder": {"stickout": 12, "segments": [{"diameter": 30, "length": 40}]}}, "as": "mill"},
        {"command": "cam.op_add", "params": {"kind": "pocket", "tool": "$mill.tool", "face": "4:top", "stepover": 40, "stepdown": 2}, "as": "op"}
    ]);
    let mut s = Session::new();
    let replies = script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    let op = replies.last().unwrap();
    assert!(op.get("error").is_none(), "{op}");
    let sim = s
        .execute("cam.simulate", &json!({"resolution": 128}))
        .unwrap();
    let h = &sim["holder"];
    assert_eq!(h["checked"], true, "{sim}");
    assert!(h["moves"].as_u64().unwrap() > 0, "{sim}");
    assert!(h["cells"].as_u64().unwrap() > 0, "{sim}");
    assert!(
        (h["max_depth"].as_f64().unwrap() - 8.0).abs() < 1e-6,
        "{sim}"
    );
    // The holder collision shows in its own colour.
    assert!(
        s.scene
            .stock
            .iter()
            .any(|m| m.style == chamfera_render::StockStyle::HolderCollision)
    );
    // A third of the way through only the levels down to about 24 are cut: no collision.
    let early = s
        .execute("cam.simulate", &json!({"resolution": 128, "time": 0.2}))
        .unwrap();
    assert_eq!(early["holder"]["moves"], 0, "{early}");
    // A longer stickout clears.
    let tool = s.doc.cam.tools[0].id;
    s.execute(
        "cam.tool_edit",
        &json!({"tool": tool, "holder": {"stickout": 25, "segments": [{"diameter": 30, "length": 40}]}}),
    )
    .unwrap();
    let clear = s
        .execute("cam.simulate", &json!({"resolution": 128}))
        .unwrap();
    assert_eq!(clear["holder"]["moves"], 0, "{clear}");
    assert_eq!(clear["holder"]["cells"], 0, "{clear}");
}

#[test]
fn pencil_passes_follow_the_filleted_pocket_floor_at_the_tool_radius_from_the_walls() {
    // examples/cam-3d-pencil.json: a 60 x 40 x 20 block with a 30 x 20 pocket 10 deep
    // (x 15..45, y 10..30, floor z = 10) whose floor edges are filleted at radius 2. A
    // ball of radius 4 cannot follow a fillet of radius 2: it rests on the floor and
    // touches the wall's top edge when its axis is 4 from the wall (2 beyond the fillet's
    // edge on the floor, at x = 17 and 43, y = 12 and 28). So the pencil passes run round
    // the rectangle x 19..41, y 14..26, the tip at the floor's height, 10.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/cam-3d-pencil.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    let op = s
        .doc
        .cam
        .operations
        .iter()
        .find(|o| o.op.key() == "pencil")
        .unwrap()
        .id;
    let tp = s.toolpath(op).unwrap().as_ref().unwrap().clone();
    // The cutting moves along the passes (not the plunges into them).
    let mut feeds = Vec::new();
    let mut at = None;
    for m in &tp.moves {
        match m {
            chamfera_cam::Move::Line { to, .. } => {
                if let Some(a) = at {
                    feeds.push([a, *to]);
                }
                at = Some(*to);
            }
            chamfera_cam::Move::Rapid { to } | chamfera_cam::Move::Plunge { to, .. } => {
                at = Some(*to);
            }
            _ => {}
        }
    }
    assert!(!feeds.is_empty());
    let on = |v: f64, w: f64| (v - w).abs() < 0.01;
    let mut sides = [0.0f64; 4];
    for [a, b] in &feeds {
        for p in [*a, *b, *a + (*b - *a) * 0.5] {
            if p.z < 5.0 {
                continue; // in the air beside the part
            }
            assert!(on(p.z, 10.0), "{p:?}");
            let side = (on(p.x, 19.0) || on(p.x, 41.0)) && (14.0 - 0.01..=26.01).contains(&p.y)
                || (on(p.y, 14.0) || on(p.y, 26.0)) && (19.0 - 0.01..=41.01).contains(&p.x);
            assert!(side, "a pencil point off the offset rectangle: {p:?}");
        }
        for (k, (fixed, x)) in [(true, 19.0), (true, 41.0), (false, 14.0), (false, 26.0)]
            .iter()
            .enumerate()
        {
            let (u, v) = if *fixed { (a.x, b.x) } else { (a.y, b.y) };
            if on(u, *x) && on(v, *x) {
                sides[k] += if *fixed {
                    (a.y - b.y).abs()
                } else {
                    (a.x - b.x).abs()
                };
            }
        }
    }
    // Each side followed its whole length (12 or 22 mm), give or take the corners.
    for (k, want) in [12.0, 12.0, 22.0, 22.0].iter().enumerate() {
        assert!(sides[k] > want - 1.0, "{sides:?}");
    }
}

#[test]
fn a_tool_library_round_trips_through_its_file() {
    let dir = std::env::temp_dir().join(format!("chamfera-tools-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tools.json");
    let mut s = Session::new();
    s.execute(
        "cam.tool_add",
        &json!({"kind": "ball_end_mill", "diameter": 6, "number": 4, "name": "Ball 6",
                "flute_length": 12, "spindle_rpm": 18000, "feed": 1500,
                "holder": {"stickout": 30, "shank_diameter": 6,
                           "segments": [{"diameter": 20, "top_diameter": 32, "length": 15},
                                        {"diameter": 40, "length": 30}]}}),
    )
    .unwrap();
    s.execute(
        "cam.tool_add",
        &json!({"kind": "bull_nose", "diameter": 10, "corner_radius": 1, "number": 7}),
    )
    .unwrap();
    s.execute(
        "cam.tool_library_export",
        &json!({"path": path.to_string_lossy()}),
    )
    .unwrap();
    // Into a design that already has a T4: the imported T4 becomes the next free number,
    // 4 + 1 = 5; the T7 keeps its number.
    let mut t = Session::new();
    t.execute(
        "cam.tool_add",
        &json!({"kind": "drill", "diameter": 5, "number": 4}),
    )
    .unwrap();
    let r = t
        .execute(
            "cam.tool_library_import",
            &json!({"path": path.to_string_lossy()}),
        )
        .unwrap();
    assert_eq!(r["tools"].as_array().unwrap().len(), 2);
    let strip = |x: &chamfera_doc::cam::Tool| chamfera_doc::cam::LibraryTool::from_tool(x);
    let mut ball = strip(&t.doc.cam.tools[1]);
    assert_eq!(ball.number, 5);
    ball.number = 4;
    assert_eq!(ball, strip(&s.doc.cam.tools[0]));
    assert_eq!(strip(&t.doc.cam.tools[2]), strip(&s.doc.cam.tools[1]));
    // Undo takes the import back as one step.
    t.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(t.doc.cam.tools.len(), 1);
    // Bad files change nothing.
    for (k, text) in [
        "not json",
        r#"{"format":"chamfera-tools","version":1,"tools":[{"number":1}]}"#,
        r#"{"format":"chamfera-tools","version":1,"tools":[{"number":1,"name":"x","kind":"flat_end_mill","diameter":-6,"flute_length":1,"corner_radius":0,"flutes":2,"tip_angle":0,"spindle_rpm":1,"feed":1,"plunge_feed":1}]}"#,
    ]
    .iter()
    .enumerate()
    {
        let bad = dir.join(format!("bad{k}.json"));
        std::fs::write(&bad, text).unwrap();
        let before = t.doc.clone();
        assert!(
            t.execute("cam.tool_library_import", &json!({"path": bad.to_string_lossy()}))
                .is_err()
        );
        assert_eq!(t.doc, before);
    }
    // A tool an operation uses cannot be deleted; one no operation uses can.
    let id = t.doc.cam.tools[0].id;
    t.execute("cam.op_add", &json!({"kind": "drill", "tool": id}))
        .unwrap();
    assert!(t.execute("cam.tool_delete", &json!({"tool": id})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

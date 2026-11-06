//! End-to-end parts (`examples/qa/*.json`): each script builds a realistic part with many
//! features together, and the tests check the result against hand-derived values and run
//! it through the life of a design: undo everything, redo it, roll the timeline back and
//! forward, suppress a feature, save and reopen. See `docs/qa-report.md`.

use super::*;
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh folder for one test's files.
fn out_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chamfera-qa-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Rewrites the scripts' relative paths: `target/qa/` into `out`, `examples/` to the repo's.
fn fix_paths(v: &Value, out: &Path) -> Value {
    match v {
        Value::String(s) if s.starts_with("target/qa/") => {
            json!(out.join(&s["target/qa/".len()..]).to_string_lossy())
        }
        Value::String(s) if s.starts_with("examples/") => {
            json!(repo().join(s).to_string_lossy())
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fix_paths(x, out)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), fix_paths(x, out)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn run_qa(s: &mut Session, file: &str, out: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(repo().join("examples/qa").join(file)).unwrap();
    let steps = fix_paths(&serde_json::from_str(&text).unwrap(), out);
    script::run_steps(s, steps.as_array().unwrap()).unwrap()
}

/// Each solid's name and exact volume, in build order.
fn volumes(s: &Session) -> Vec<(String, f64)> {
    s.solids()
        .iter()
        .map(|x| (x.name.clone(), x.body.volume().unwrap()))
        .collect()
}

fn assert_same(a: &[(String, f64)], b: &[(String, f64)], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: {a:?} vs {b:?}");
    for ((na, va), (nb, vb)) in a.iter().zip(b) {
        assert_eq!(na, nb, "{what}");
        assert!((va - vb).abs() <= 1e-9 * va.abs().max(1.0), "{what}: {na} {va} vs {vb}");
    }
}

fn close(v: f64, want: f64, what: &str) {
    assert!((v - want).abs() <= 1e-6 * want.abs().max(1.0), "{what}: {v} vs {want}");
}

/// Undo to empty and redo it all; roll the timeline to the middle and back; suppress and
/// unsuppress `suppress`; save and reopen. Every round trip must give the same solids.
fn life_cycle(s: &mut Session, suppress: u32) {
    let built = volumes(s);
    let mut n = 0;
    while s.execute("edit.undo", &json!({})).is_ok() {
        n += 1;
        assert!(n < 10_000);
    }
    assert!(s.solids().is_empty(), "undo leaves {:?}", volumes(s));
    for _ in 0..n {
        s.execute("edit.redo", &json!({})).unwrap();
    }
    assert_same(&built, &volumes(s), "redo");
    let items = s.execute("timeline.list", &json!({})).unwrap()["items"]
        .as_array()
        .unwrap()
        .len();
    s.execute("timeline.roll_to", &json!({"position": items / 2}))
        .unwrap();
    assert!(volumes(s).len() <= built.len());
    s.execute("timeline.roll_to_end", &json!({})).unwrap();
    assert_same(&built, &volumes(s), "roll forward");
    s.execute(
        "feature.edit",
        &json!({"feature": suppress, "suppressed": true}),
    )
    .unwrap();
    s.execute(
        "feature.edit",
        &json!({"feature": suppress, "suppressed": false}),
    )
    .unwrap();
    assert_same(&built, &volumes(s), "unsuppress");
    let data = s.execute("design.save_bytes", &json!({})).unwrap()["data"].clone();
    let mut t = Session::new();
    t.execute("design.open_bytes", &json!({ "data": data }))
        .unwrap();
    assert_same(&built, &volumes(&t), "reopen");
}

/// The enclosure's base and lid for a width `w` (depth 50): a 30 high box with r = 6
/// vertical fillets, shelled 2 mm open at the top (inside (w - 4) x 46 with r = 4
/// corners, 28 deep); a 3 mm lid with four counterbored holes (Ø3.4 through, Ø6.5 x 1.5).
/// A rounded rectangle loses (4 - π) r² to its corners.
fn enclosure(w: f64) -> (f64, f64) {
    let k = 4.0 - PI;
    let base = 30.0 * (w * 50.0 - 36.0 * k) - 28.0 * ((w - 4.0) * 46.0 - 16.0 * k);
    let hole = PI * 1.5 * (3.25f64.powi(2) + 1.7f64.powi(2));
    (base, w * 50.0 * 3.0 - 4.0 * hole)
}

#[test]
fn qa_enclosure_follows_its_width_and_survives_the_life_cycle() {
    let out = out_dir("enclosure");
    let mut s = Session::new();
    run_qa(&mut s, "01-enclosure.json", &out);
    let v = volumes(&s);
    let (base, lid) = enclosure(80.0);
    close(v[0].1, base, "base");
    close(v[1].1, lid, "lid");
    s.execute("param.set", &json!({"name": "width", "expression": "100 mm"}))
        .unwrap();
    let v = volumes(&s);
    let (base, lid) = enclosure(100.0);
    close(v[0].1, base, "base at width 100");
    close(v[1].1, lid, "lid at width 100");
    // Suppressing the fillet leaves a square box: 30 * 5000 - 28 * 96 * 46 = 26352.
    s.execute("feature.edit", &json!({"feature": 3, "suppressed": true}))
        .unwrap();
    close(volumes(&s)[0].1, 26_352.0, "unfilleted");
    s.execute("edit.undo", &json!({})).unwrap();
    life_cycle(&mut s, 3);
}

#[test]
fn qa_flanged_pipe_has_its_hand_volume() {
    // Flange: annulus 40 / 15 x 8 = 11000π. Transition: frustum r 20 -> 12.5 over 20
    // (πh/3 (R² + Rr + r²) = 5375π) less the bore frustum 15 -> 10 (9500π/3). Pipe: the
    // annulus 12.5 / 10 (56.25π) swept along 50 + (π/2) 30 + 50 (Pappus). Six Ø7 through
    // holes: 6 * 98π. The rim chamfer 1 x 1 removes a triangle of area 1/2 whose centroid
    // is 40 - 1/3 from the axis: π (40 - 1/3) = 119π/3.
    let out = out_dir("pipe");
    let mut s = Session::new();
    run_qa(&mut s, "02-flanged-pipe.json", &out);
    let want = 11_000.0 * PI + (5375.0 - 9500.0 / 3.0) * PI + 56.25 * PI * (100.0 + 15.0 * PI)
        - 588.0 * PI
        - 119.0 * PI / 3.0;
    let v = volumes(&s);
    assert_eq!(v.len(), 1, "{v:?}");
    close(v[0].1, want, "flanged pipe");
}

#[test]
fn qa_sheet_bracket_and_its_flat_pattern() {
    // Rule: T 2, R 2, k 0.44. Base 100 x 60 x 2 = 12000. An "inside" flange of height H on
    // an edge of length L adds a quarter bend L (π/4)((R+T)² - R²) = 3π L and a wall
    // L T (H - R - T), and takes L T (R + T) off the base: right and left (L 60, H 25):
    // 60 (3π + 42) - 480 each. The front one (H 15) runs along what the side flanges
    // left of its edge, 100 - 2 * 4 = 92, less offsets 10 + 10: L 72, so
    // 72 (3π + 22) - 576, less two reliefs 2 wide, R + T + 2 = 6 deep: 2 * 2 * 6 * 2.
    let out = out_dir("sheet");
    let mut s = Session::new();
    let r = run_qa(&mut s, "03-sheet-bracket.json", &out);
    let side = 60.0 * (3.0 * PI + 42.0) - 480.0;
    let front = 72.0 * (3.0 * PI + 22.0) - 576.0 - 48.0;
    close(volumes(&s)[0].1, 12_000.0 + 2.0 * side + front, "bracket");
    // Flat: bend allowance (π/2)(R + kT) = 1.44π. Length 92 + 2 (1.44π + 21); width
    // 56 + 1.44π + 11. Area: base 92 x 56, side strips 60 (1.44π + 21), the front strip
    // 72 (1.44π + 11), and the corners the front flange leaves, 2 * 10 * 4, less the
    // reliefs 2 * 2 * 6.
    let flat = &r[8];
    let ba = 1.44 * PI;
    close(flat["length"].as_f64().unwrap(), 92.0 + 2.0 * (ba + 21.0), "flat length");
    close(flat["width"].as_f64().unwrap(), 67.0 + ba, "flat width");
    let area = 92.0 * 56.0 + 120.0 * (ba + 21.0) + 72.0 * (ba + 11.0) + 80.0 - 24.0;
    close(flat["area"].as_f64().unwrap(), area, "flat area");
    // The exported DXF reads back into a sketch as one profile of that area.
    let mut t = Session::new();
    t.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let back = t
        .execute(
            "sketch.import_dxf",
            &json!({"path": out.join("03-bracket-flat.dxf").to_string_lossy()}),
        )
        .unwrap();
    let profiles = back["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 1, "{back}");
    close(profiles[0].as_f64().unwrap(), area, "flat DXF profile");
    life_cycle(&mut s, 4);
}

#[test]
fn qa_hinge_measures_between_its_leaves_at_three_angles() {
    // Leaves 40 x 30 x 3, A at x 2..42, B at x -42..-2, hinged about the line x = 0,
    // z = 1.5 along y. At 0°: the gap, 4. At 90°: B's corner (1.5, 3.5) (in x, z) to A's
    // (2, 3): √0.5. At 45°: B's end corner (-2, 3) turned about (0, 1.5) is
    // (-0.5c, 1.5 + 3.5c) with c = √2/2, to A's (2, 3): √((2 + 0.5c)² + (3.5c - 1.5)²).
    let out = out_dir("hinge");
    let mut s = Session::new();
    let r = run_qa(&mut s, "04-hinge.json", &out);
    let c = std::f64::consts::FRAC_1_SQRT_2;
    close(r[16]["distance"].as_f64().unwrap(), 4.0, "0 deg");
    let d45 = ((2.0 + 0.5 * c).powi(2) + (3.5 * c - 1.5).powi(2)).sqrt();
    close(r[18]["distance"].as_f64().unwrap(), d45, "45 deg");
    close(r[20]["distance"].as_f64().unwrap(), 0.5f64.sqrt(), "90 deg");
    life_cycle(&mut s, 4);
    let d = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": "Extrude1"}, "b": {"solid": "Extrude2"}}),
        )
        .unwrap();
    close(d["distance"].as_f64().unwrap(), 0.5f64.sqrt(), "after the life cycle");
}

#[test]
fn qa_tapered_hexagon_and_spline_mirror() {
    // Hexagon, circumradius 10 (inradius r0 = 5√3), 10 high, 5° taper: the inradius
    // shrinks by z tan 5°, area 2√3 r², V = 2√3 (r0³ - (r0 - h t)³) / (3t). The ellipse
    // a 15, b 8 has area 120π. Mirrors keep both volumes.
    let out = out_dir("taper");
    let mut s = Session::new();
    let r = run_qa(&mut s, "05-spline-ellipse-taper.json", &out);
    let areas: Vec<f64> = r[4]["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["area"].as_f64().unwrap())
        .collect();
    assert!(areas.iter().any(|a| (a - 120.0 * PI).abs() < 1e-9), "{areas:?}");
    let (r0, h, t) = (5.0 * 3f64.sqrt(), 10.0, 5f64.to_radians().tan());
    let hex = 2.0 * 3f64.sqrt() * (r0.powi(3) - (r0 - h * t).powi(3)) / (3.0 * t);
    let v = volumes(&s);
    assert_eq!(v.len(), 4, "{v:?}");
    close(v[0].1, hex, "hexagon");
    close(v[2].1, hex, "mirrored hexagon");
    close(v[3].1, v[1].1, "mirrored spline");
    life_cycle(&mut s, 3);
}

#[test]
fn qa_cam_on_the_enclosure_base_posts_both_dialects() {
    let out = out_dir("cam");
    let mut s = Session::new();
    run_qa(&mut s, "01-enclosure.json", &out);
    let mut s = Session::new();
    let r = run_qa(&mut s, "06-cam-enclosure-base.json", &out);
    // Four Ø4 holes through the 2 mm floor: 21569.49 - 4 * 4π * 2 (see `enclosure`).
    close(volumes(&s)[0].1, enclosure(80.0).0 - 32.0 * PI, "drilled base");
    let generated = &r[10];
    assert_eq!(generated["errors"], 0, "{generated}");
    let holes = generated["operations"][2]["holes"].as_array().unwrap();
    assert_eq!(holes.len(), 4);
    // Stock 3 mm larger on each side and 1 mm above the top (z 31): the work origin is
    // the stock's top corner (-3, -3, 31). Hole (65, 15) is at X68 Y18; through the floor
    // (z 0) and 1 mm past it is Z-32.
    let lc = std::fs::read_to_string(out.join("06-base-linuxcnc.ngc")).unwrap();
    assert!(lc.contains("G83 X68 Y18 Z-32"), "{lc}");
    assert!(lc.contains("T2 M6"));
    let grbl = std::fs::read_to_string(out.join("06-base-grbl.nc")).unwrap();
    assert!(!grbl.contains("M6") && !grbl.contains("G83"));
    assert!(grbl.contains("M0"));
    // Facing takes the 1 mm of stock above the model's top.
    assert!(grbl.contains("G1 Z-1"));
}

#[test]
fn qa_enclosure_drawing_dimensions_and_exports() {
    let out = out_dir("drawing");
    let mut s = Session::new();
    run_qa(&mut s, "01-enclosure.json", &out);
    let mut s = Session::new();
    let r = run_qa(&mut s, "07-enclosure-drawing.json", &out);
    // Width 80, height 30 + 5 + 3 = 38, depth 50 (top and right views), the r = 6 corner
    // and the Ø6.5 counterbore.
    let want = [80.0, 38.0, 50.0, 50.0, 6.0, 6.5];
    for (i, w) in want.iter().enumerate() {
        close(r[6 + i]["value"].as_f64().unwrap(), *w, "dimension");
    }
    for ext in ["pdf", "svg", "dxf"] {
        let len = std::fs::metadata(out.join(format!("07-enclosure.{ext}")))
            .unwrap()
            .len();
        assert!(len > 1000, "{ext}: {len} bytes");
    }
}

#[test]
#[ignore = "open: drawing dimensions keep their first value when the model changes (docs/qa-report.md)"]
fn qa_drawing_dimensions_follow_the_model() {
    let out = out_dir("drawing-follow");
    let mut s = Session::new();
    run_qa(&mut s, "01-enclosure.json", &out);
    let mut s = Session::new();
    run_qa(&mut s, "07-enclosure-drawing.json", &out);
    s.execute("param.set", &json!({"name": "width", "expression": "100 mm"}))
        .unwrap();
    let l = s.execute("drawing.list", &json!({})).unwrap();
    // The front view now spans 100 mm, and so must its width dimension.
    close(
        l["sheets"][0]["dimensions"][0]["value"].as_f64().unwrap(),
        100.0,
        "width dimension",
    );
}

#[test]
fn qa_dxf_plate_combine_cut_and_step_round_trip() {
    // The DXF: 80 x 40 with two r = 5 holes, extruded 10: 32000 - 500π. A r = 10
    // cylinder through it cut away: a further 1000π. The STEP file reads back as an
    // exact body with that volume.
    let out = out_dir("dxf");
    let mut s = Session::new();
    let r = run_qa(&mut s, "08-dxf-combine-export.json", &out);
    close(
        r[3]["solids"][0]["volume"].as_f64().unwrap(),
        32_000.0 - 500.0 * PI,
        "plate",
    );
    let want = 32_000.0 - 1500.0 * PI;
    let back = &r[12]["solids"];
    close(back[0]["volume"].as_f64().unwrap(), want, "STEP read back");
    let v = volumes(&s);
    assert!(v.iter().all(|(_, x)| (x - want).abs() < 1e-6), "{v:?}");
    for f in ["08-plate.step", "08-plate.3mf", "08-plate.glb"] {
        assert!(std::fs::metadata(out.join(f)).unwrap().len() > 100, "{f}");
    }
}

#[test]
#[ignore = "open: a plane face and a torus face whose boxes overlap are refused (docs/qa-report.md)"]
fn qa_a_sweep_bend_near_a_flat_face_joins() {
    // Part 2 with a 40 mm straight run before the bend (the shipped script uses 50): the
    // bend's full torus box (centre z 68, reach 42.5) reaches down to z 25.5, past the
    // transition's top face at z 28, although the bend itself stays above z 68 - 12.5.
    // Volume as in qa_flanged_pipe_has_its_hand_volume with a path 40 + 15π + 50.
    let text = std::fs::read_to_string(repo().join("examples/qa/02-flanged-pipe.json")).unwrap();
    let text = text
        .replace(r#""x2": 0, "y2": 78"#, r#""x2": 0, "y2": 68"#)
        .replace(r#""cy": 78, "x1": 30, "y1": 108, "x2": 0, "y2": 78"#, r#""cy": 68, "x1": 30, "y1": 98, "x2": 0, "y2": 68"#)
        .replace(r#""y1": 108, "x2": 80, "y2": 108"#, r#""y1": 98, "x2": 80, "y2": 98"#);
    let steps: Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    let want = 11_000.0 * PI + (5375.0 - 9500.0 / 3.0) * PI + 56.25 * PI * (90.0 + 15.0 * PI)
        - 588.0 * PI
        - 119.0 * PI / 3.0;
    close(volumes(&s)[0].1, want, "flanged pipe with a shorter run");
}

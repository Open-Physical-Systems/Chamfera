//! `render.*` commands and physically based materials in `body.appearance`.

use super::*;
use serde_json::json;

/// examples/extruded-plate.json: an 80 x 50 x 8 plate with two Ø16 holes.
fn plate() -> Session {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extruded-plate.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    s
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chamfera-render-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Width and height from a PNG's IHDR chunk (big-endian at bytes 16 and 20).
fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    (be(16), be(20))
}

#[test]
fn render_image_writes_a_png_of_the_size_asked_for() {
    let mut s = plate();
    let dir = temp("image");
    let path = dir.join("plate.png");
    let r = s
        .execute(
            "render.image",
            &json!({"width": 48, "height": 30, "samples": 2, "path": path.to_str().unwrap()}),
        )
        .unwrap();
    assert_eq!(r["samples"], 2);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(png_size(&bytes), (48, 30));
    assert_eq!(r["bytes"], bytes.len());
    // The browser form gives the same file as base64 ("iVBORw0KGgo" is the signature).
    let r = s
        .execute(
            "render.image_bytes",
            &json!({"width": 12, "height": 7, "samples": 1}),
        )
        .unwrap();
    assert!(r["data"].as_str().unwrap().starts_with("iVBORw0KGgo"));
    // A time limit stops early but always after a whole sample.
    let r = s
        .execute(
            "render.image_bytes",
            &json!({"width": 8, "height": 8, "samples": 4096, "time_limit": 1e-9}),
        )
        .unwrap();
    assert_eq!(r["samples"], 1);
    assert_eq!(r["target_samples"], 4096);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn renders_beyond_the_caps_are_refused() {
    let mut s = plate();
    for p in [
        json!({"width": 4097, "height": 10, "samples": 1}),
        json!({"width": 10, "height": 4097, "samples": 1}),
        json!({"width": 10, "height": 10, "samples": 4097}),
        json!({"width": 0, "height": 10, "samples": 1}),
        json!({"width": 10.5, "height": 10, "samples": 1}),
        json!({"width": 10, "height": 10}),
        json!({"width": 10, "height": 10, "samples": 1, "time_limit": 0}),
    ] {
        assert!(s.execute("render.image_bytes", &p).is_err(), "{p}");
        if p.get("time_limit").is_none() {
            assert!(s.execute("render.start", &p).is_err(), "{p}");
        }
    }
    // Only PNG files are written.
    let e = s.execute(
        "render.image",
        &json!({"width": 4, "height": 4, "samples": 1, "path": "/tmp/x.jpg"}),
    );
    assert!(e.is_err());
    // Nothing started yet, so there is nothing to save.
    assert!(s.execute("render.save_bytes", &json!({})).is_err());
}

#[test]
fn a_started_render_refines_and_saves() {
    let mut s = plate();
    s.execute(
        "render.start",
        &json!({"width": 16, "height": 10, "samples": 3}),
    )
    .unwrap();
    let job = s.render_job.clone().unwrap();
    assert_eq!(job.samples(), 0);
    while !job.done() {
        job.step();
    }
    let r = s.execute("render.save_bytes", &json!({})).unwrap();
    assert_eq!(r["samples"], 3);
    let dir = temp("save");
    let path = dir.join("job.png");
    s.execute("render.save", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(png_size(&std::fs::read(&path).unwrap()), (16, 10));
    // A new render replaces and stops the old one.
    s.execute(
        "render.start",
        &json!({"width": 4, "height": 4, "samples": 1}),
    )
    .unwrap();
    assert!(job.is_cancelled());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_environment_is_a_view_state_and_unknown_ones_are_refused() {
    let mut s = plate();
    assert_eq!(
        s.scene.display.environment,
        chamfera_render::EnvironmentPreset::Studio
    );
    s.execute("render.environment", &json!({"preset": "dark"}))
        .unwrap();
    assert_eq!(
        s.scene.display.environment,
        chamfera_render::EnvironmentPreset::Dark
    );
    assert!(
        s.execute("render.environment", &json!({"preset": "moon"}))
            .is_err()
    );
    s.execute("view.style", &json!({"style": "realistic"}))
        .unwrap();
    assert_eq!(
        s.scene.display.style,
        chamfera_render::VisualStyle::Realistic
    );
    let list = s.execute("render.materials", &json!({})).unwrap();
    let list = list["materials"].as_array().unwrap();
    assert!(list.len() >= 20);
    assert!(
        list.iter()
            .any(|m| m["id"] == "brass" && m["metallic"] == 1.0)
    );
}

#[test]
fn materials_survive_save_and_open_and_bad_ones_change_nothing() {
    let mut s = plate();
    let body = s.scene.bodies[0].id;
    // Brass from the library: metallic 1, roughness 0.22 (crates/render/src/material.rs).
    let r = s
        .execute("body.appearance", &json!({"body": body, "preset": "brass"}))
        .unwrap();
    assert_eq!(r["metallic"], 1.0);
    // Then made a little rougher by hand: the preset name goes, the rest stays.
    // Every change rebuilds the scene, so the body has a new id.
    let body = s.scene.bodies[0].id;
    let r = s
        .execute("body.appearance", &json!({"body": body, "roughness": 0.4}))
        .unwrap();
    assert_eq!(r["preset"], serde_json::Value::Null);
    let a = s.doc.appearances.get(&2).cloned().unwrap();
    assert_eq!(a.metallic, Some(1.0));
    assert_eq!(a.roughness, Some(0.4));
    assert_eq!(a.ior, Some(1.5));
    let before = s.doc.clone();
    let body = s.scene.bodies[0].id;
    for p in [
        json!({"body": body, "metallic": 1.5}),
        json!({"body": body, "roughness": -0.1}),
        json!({"body": body, "transmission": 2}),
        json!({"body": body, "ior": 0.9}),
        json!({"body": body, "ior": 3.5}),
        json!({"body": body, "ior": f64::NAN}),
    ] {
        assert!(s.execute("body.appearance", &p).is_err(), "{p}");
        assert_eq!(s.doc, before);
    }

    let dir = temp("materials");
    let path = dir.join("brass.chamfera");
    s.execute("design.save", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"metallic\""));
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(t.doc, s.doc);
    assert_eq!(t.doc.version, 34);
    let shown = t.scene.bodies[0].appearance;
    assert_eq!(shown.metallic, 1.0);
    assert!((shown.roughness - 0.4).abs() < 1e-6);
    assert_eq!(shown.colour, Some(chamfera_render::Rgba(214, 176, 92, 255)));
    std::fs::remove_dir_all(&dir).unwrap();
}

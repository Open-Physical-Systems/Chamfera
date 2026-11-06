use super::*;
use serde_json::json;

#[test]
fn every_command_id_is_our_own_format_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for c in registry() {
        assert!(cmd::is_valid_id(c.id), "bad id {}", c.id);
        assert!(seen.insert(c.id), "duplicate id {}", c.id);
        assert!(!c.label.is_empty(), "{} has no label", c.id);
        assert!(!c.params.is_empty(), "{} has no params doc", c.id);
    }
}

#[test]
fn id_format_rules() {
    for ok in ["param.set", "sketch.rectangle.two_point", "solid.extrude2"] {
        assert!(cmd::is_valid_id(ok), "{ok}");
    }
    for bad in [
        "param",
        "Param.set",
        "param..set",
        "param.set.",
        "2d.line",
        "param.se-t",
        "",
    ] {
        assert!(!cmd::is_valid_id(bad), "{bad}");
    }
}

#[test]
fn set_undo_redo_round_trip() {
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "w", "value": 10}))
        .unwrap();
    s.execute("param.set", &json!({"name": "w", "value": 20}))
        .unwrap();
    assert_eq!(s.doc.parameters["w"].value, 20.0);

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.parameters["w"].value, 10.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc.parameters.is_empty());
    assert!(s.execute("edit.undo", &json!({})).is_err());

    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc.parameters["w"].value, 20.0);
    assert!(s.execute("edit.redo", &json!({})).is_err());
}

#[test]
fn a_new_change_clears_redo() {
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "a", "value": 1}))
        .unwrap();
    s.execute("edit.undo", &json!(null)).unwrap();
    s.execute("param.set", &json!({"name": "b", "value": 2}))
        .unwrap();
    assert!(!s.can_redo());
}

#[test]
fn a_failing_command_changes_nothing_and_adds_no_undo_step() {
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "w", "value": 10}))
        .unwrap();
    let before = s.doc.clone();
    assert!(
        s.execute("param.set", &json!({"name": "2bad", "value": 1}))
            .is_err()
    );
    assert!(
        s.execute("param.set", &json!({"name": "w", "value": 1e300}))
            .is_err()
    );
    assert!(
        s.execute("param.remove", &json!({"name": "missing"}))
            .is_err()
    );
    assert_eq!(s.doc, before);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.can_undo(), "failed commands must not add undo steps");
}

#[test]
fn undo_history_is_capped() {
    let mut s = Session::new();
    for i in 0..(UNDO_LIMIT + 50) {
        s.execute("param.set", &json!({"name": "n", "value": i}))
            .unwrap();
    }
    let mut steps = 0;
    while s.execute("edit.undo", &json!({})).is_ok() {
        steps += 1;
    }
    assert_eq!(steps, UNDO_LIMIT);
}

#[test]
fn unknown_commands_are_refused() {
    let mut s = Session::new();
    assert!(matches!(
        s.execute("solid.does_not_exist", &json!({})),
        Err(CommandError::UnknownCommand(_))
    ));
}

#[test]
fn a_design_saves_and_opens() {
    let dir = std::env::temp_dir().join(format!("chamfera-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.chamfera");
    let path_str = path.to_str().unwrap();

    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "width", "value": 50}))
        .unwrap();
    s.execute("design.save", &json!({"path": path_str}))
        .unwrap();
    s.execute("design.new", &json!({})).unwrap();
    assert!(s.doc.parameters.is_empty());
    s.execute("design.open", &json!({"path": path_str}))
        .unwrap();
    assert_eq!(s.doc.parameters["width"].value, 50.0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every command, fed every kind of hostile parameter, returns instead of panicking.
#[test]
fn hostile_params_never_panic() {
    let long = "x".repeat(100_000);
    let nested = (0..64).fold(json!(1), |v, _| json!([v]));
    let hostile = [
        json!(null),
        json!(true),
        json!(-1),
        json!("string"),
        json!([1, 2, 3]),
        nested.clone(),
        json!({}),
        json!({"name": null, "value": null, "unit": null, "path": null}),
        json!({"name": 5, "value": "5", "unit": 5, "path": 5}),
        json!({"name": "", "value": 0, "unit": "", "path": ""}),
        json!({"name": long, "value": 1e308, "unit": long, "path": long}),
        json!({"name": "w", "value": -1e308, "unit": "angle"}),
        json!({"name": "w", "value": 1, "unit": "LENGTH"}),
        json!({"name": "\u{0}", "value": 1, "path": "/dev/null"}),
        json!({"name": "w", "value": nested, "path": "/nonexistent/dir/x.chamfera"}),
        json!({"name": ["w"], "value": {"v": 1}, "path": ["a"]}),
        json!({"x": -1, "y": 1e308, "width": 0, "height": -5, "dx": 1e300, "dy": null}),
        json!({"x": 10, "y": 10, "width": 1e9, "height": 1e9, "factor": 0}),
        json!({"x": 5, "y": 5, "width": 10, "height": 10, "factor": 1e-300}),
        json!({"factor": "big", "mode": 3, "view": "sideways", "body": -1, "visible": "yes"}),
        json!({"yaw": 1e308, "pitch": -1e308, "body": 4294967296u64, "visible": 1}),
        json!({"body": 0, "mode": "PERSPECTIVE", "view": ""}),
        json!({"path": "/dev/zero"}),
    ];
    for c in registry() {
        for p in &hostile {
            let mut s = Session::new();
            let _ = s.execute(c.id, p);
            // Again with a body in the scene, so body ids and picks have something to hit.
            let mut s = Session::new();
            let _ = s.execute("body.sample", &json!({}));
            let _ = s.execute(c.id, p);
            assert!(s.camera.distance.is_finite() && s.camera.yaw.is_finite());
        }
    }
}

#[test]
fn the_sample_part_has_its_analytic_volume() {
    // Base 80 x 50 x 8 + upright 80 x 8 x 40 + a 64-sided prism r = 10, h = 12:
    // (n/2) r^2 sin(2pi/n) h. The parts touch but do not overlap.
    let n = 64.0_f64;
    let prism = n / 2.0 * 100.0 * (std::f64::consts::TAU / n).sin() * 12.0;
    let expected = 32_000.0 + 25_600.0 + prism;
    let mut s = Session::new();
    let reply = s.execute("body.sample", &json!({})).unwrap();
    let volume = reply["volume"].as_f64().unwrap();
    assert!((volume - expected).abs() < 1e-6, "{volume} vs {expected}");
    assert_eq!(reply["bounds"]["max"]["z"].as_f64().unwrap(), 48.0);
}

#[test]
fn home_then_a_centre_click_picks_the_body() {
    let mut s = Session::new();
    let id = s.execute("body.sample", &json!({})).unwrap()["body"]
        .as_u64()
        .unwrap();
    let vp = json!({"width": 800, "height": 600});
    s.execute("view.home", &vp).unwrap();
    let hit = s
        .execute(
            "select.pick",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert_eq!(hit["body"].as_u64(), Some(id));
    // A corner click from the home view misses, and clears the selection.
    let miss = s
        .execute(
            "select.pick",
            &json!({"x": 0, "y": 0, "width": 800, "height": 600}),
        )
        .unwrap();
    assert!(miss["body"].is_null());
}

#[test]
fn a_top_view_pick_lands_on_the_top_face_of_the_boss() {
    // The boss top is at z = 8 + 12 = 20 around (40, 21); the upright top is z = 48.
    let mut s = Session::new();
    s.execute("body.sample", &json!({})).unwrap();
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(40.0, 21.0, 0.0);
    let hit = s
        .execute(
            "select.pick",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert!(
        (hit["point"]["z"].as_f64().unwrap() - 20.0).abs() < 1e-6,
        "{hit}"
    );
    assert!((hit["normal"]["z"].as_f64().unwrap() - 1.0).abs() < 1e-9);
}

#[test]
fn hidden_bodies_are_not_picked_and_lose_the_selection() {
    let mut s = Session::new();
    let id = s.execute("body.sample", &json!({})).unwrap()["body"]
        .as_u64()
        .unwrap();
    s.execute("view.home", &json!({"width": 800, "height": 600}))
        .unwrap();
    let centre = json!({"x": 400, "y": 300, "width": 800, "height": 600});
    s.execute("select.pick", &centre).unwrap();
    s.execute("body.visibility", &json!({"body": id, "visible": false}))
        .unwrap();
    assert!(s.execute("select.get", &json!({})).unwrap()["body"].is_null());
    assert!(s.execute("select.pick", &centre).unwrap()["body"].is_null());
}

#[test]
fn zooming_toward_a_pixel_moves_the_camera_closer() {
    let mut s = Session::new();
    s.execute("body.sample", &json!({})).unwrap();
    s.execute("view.home", &json!({"width": 800, "height": 600}))
        .unwrap();
    let before = s.camera.distance;
    s.execute(
        "view.zoom",
        &json!({"factor": 0.5, "x": 600, "y": 200, "width": 800, "height": 600}),
    )
    .unwrap();
    assert!((s.camera.distance - before * 0.5).abs() < 1e-9);
    assert!(s.execute("view.zoom", &json!({"factor": 0})).is_err());
}

#[test]
fn an_stl_file_imports_as_a_body() {
    let dir = std::env::temp_dir().join(format!("chamfera-stl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cube.stl");
    let cube = chamfera_geom::TriMesh::cuboid(
        chamfera_geom::Point3::ORIGIN,
        chamfera_geom::Point3::new(10.0, 10.0, 10.0),
    );
    std::fs::write(&path, chamfera_io::stl::write_binary(&cube).unwrap()).unwrap();
    let mut s = Session::new();
    let reply = s
        .execute("body.import_stl", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(reply["name"], "cube");
    assert!((reply["volume"].as_f64().unwrap() - 1000.0).abs() < 1e-6);
    std::fs::remove_dir_all(&dir).unwrap();
}

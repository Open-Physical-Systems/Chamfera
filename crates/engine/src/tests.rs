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
        json!({"plane": "xy", "x1": 0, "y1": 0, "x2": 10, "y2": 10, "cx": 5, "cy": 5, "radius": 3}),
        json!({"plane": "diagonal", "x1": f64::MAX, "y1": -1e300, "x2": 1e7, "y2": 0, "radius": 0}),
        json!({"x1": 0, "y1": 0, "x2": 0, "y2": 0, "cx": 0, "cy": 0, "radius": -1, "start": 1, "end": 1}),
        json!({"constraint": {"kind": "length", "line": 999, "value": -5}, "kind": "tangent"}),
        json!({"constraint": {"kind": "teleport"}, "kind": "nonsense", "point": 1, "x": 1e300, "y": 0}),
        json!({"constraint": "horizontal", "value": 1e300, "sketch": 1, "curve": 1, "construction": true}),
        json!({"constraint": 1, "value": f64::MIN_POSITIVE, "point": 1, "x": 3, "y": 4, "snap": "yes"}),
        json!({"name": "w", "expression": "w + 1", "constraint": 5, "unit": "length"}),
        json!({"name": "w", "expression": "((((((", "constraint": 13}),
        json!({"name": "w", "expression": 5, "constraint": 13, "value": 3}),
        json!({"name": "q", "expression": "1e308 * 1e308", "constraint": 14}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": "5", "feature": 99}),
        json!({"sketch": 1, "profiles": [{"x": 1e300, "y": 10}], "distance": 0, "direction": "up"}),
        json!({"sketch": 1, "profiles": "all", "distance": [1], "suppressed": "no", "feature": 0}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": "1e7 mm"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": -8, "direction": "symmetric"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "axis": 5, "angle": 90}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "axis": 14, "angle": "720 deg"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "axis": 1, "angle": [0]}),
        json!({"feature": 2, "face": "top", "x": 0, "y": 0, "z": 0}),
        json!({"feature": 99, "face": "nowhere", "x": 1e300, "y": 0, "z": f64::NAN}),
        json!({"curve": 1, "x": 5, "y": 0, "point": 1, "radius": 1, "distance": 1}),
        json!({"curve": 6, "x": 1e300, "y": f64::NAN, "point": 2, "radius": 1e300, "distance": -1}),
        json!({"curves": [1, 2, 3, 4], "distance": 2, "corners": "round", "dimension": false}),
        json!({"curves": [5], "distance": 1e9, "x": 10, "y": 10, "corners": 5}),
        json!({"curves": "all", "points": [-1], "axis": 1, "distance": 0}),
        json!({"curves": [1, 99999], "points": [2], "axis": 6}),
        json!({"curves": [5], "cx": 0, "cy": 0, "count": 1e9, "angle": 1e300}),
        json!({"curves": [5], "cx": 0, "cy": 0, "count": 3, "angle": -0.5}),
        json!({"curves": [5], "columns": 1001, "rows": 1001, "dx": 1, "dy": 1}),
        json!({"curves": [5], "columns": 2, "rows": -1, "dx": 0, "dy": f64::NAN}),
        json!({"x1": 0, "y1": 0, "x2": 1, "y2": 1, "x3": 2, "y3": 2, "cx": 0, "cy": 0}),
        json!({"point": 3, "x": 50, "y": 0, "distance2": 0, "curve": null}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "cut"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "smash"}),
        json!({"feature": 2, "count": 1e9, "direction": {"x": 0, "y": 0, "z": 0}, "spacing": 0}),
        json!({"feature": 2, "count": "3", "origin": {"x": f64::NAN, "y": 0}, "axis": "z", "angle": 9999}),
        json!({"edges": ["1:top 0", 5, null], "radius": "2 mm", "distance": 1e308}),
        json!({"edges": [long], "radius": -1, "distance": "0 mm", "size": "q"}),
        json!({"edges": [], "pick": {"x": 1e300, "y": "a"}, "radius": 1}),
        json!({"pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "radius": 1e9}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "filter": ["edges", 5, null]}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "filter": "edges"}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "filter": [], "name": "   "}),
        json!({"x": 1, "y": 1, "width": 2, "height": 2, "filter": [long], "name": "a\u{7}b"}),
        json!({"filter": vec!["faces"; 100], "feature": 2, "sketch": 1, "name": "Plate"}),
        json!({"faces": ["2:top", 5, null], "thickness": "2 mm", "feature": 2}),
        json!({"faces": [long], "thickness": -1e308, "feature": 3}),
        json!({"faces": "2:top", "thickness": "0 mm"}),
        json!({"faces": [], "pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "thickness": 1}),
        json!({"pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "thickness": [2]}),
        json!({"faces": [], "thickness": f64::NAN}),
        json!({"x1": 0, "y1": 0, "x2": 800, "y2": 600, "width": 800, "height": 600, "mode": "crossing", "add": true, "remove": true}),
        json!({"x1": 900, "y1": -1, "x2": f64::NAN, "y2": 1e308, "width": 800, "height": 600, "mode": "window"}),
        json!({"x1": 0, "y1": 0, "x2": 0, "y2": 0, "width": 1, "height": 1, "mode": "crossing", "add": "yes"}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "mode": "toggle"}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "mode": "flip"}),
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
            // And with a sketch being edited that holds a rectangle, a circle and an arc.
            let mut s = Session::new();
            s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
            s.execute(
                "sketch.rectangle",
                &json!({"x1": 0, "y1": 0, "x2": 40, "y2": 20}),
            )
            .unwrap();
            s.execute("sketch.circle", &json!({"cx": 10, "cy": 10, "radius": 4}))
                .unwrap();
            s.execute(
                "sketch.arc",
                &json!({"cx": 30, "cy": 10, "x1": 34, "y1": 10, "x2": 30, "y2": 14}),
            )
            .unwrap();
            let _ = s.execute(c.id, p);
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

/// Draws a rough rectangle on the XY plane, dimensions it 50 x 30 and fixes a corner.
fn dimensioned_rectangle(s: &mut Session) -> (Vec<u64>, Vec<u64>) {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 42, "y2": 27}),
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
    let length = |s: &mut Session, line: u64, value: f64| {
        s.execute(
            "sketch.constrain",
            &json!({"constraint": {"kind": "length", "line": line, "value": value}}),
        )
        .unwrap();
    };
    length(s, lines[0], 50.0);
    length(s, lines[1], 30.0);
    s.execute(
        "sketch.constrain",
        &json!({"constraint": {"kind": "fix", "point": pts[0], "at": {"x": 0.0, "y": 0.0}}}),
    )
    .unwrap();
    (pts, lines)
}

#[test]
fn a_sketched_rectangle_solves_and_reports_no_freedom_and_its_area() {
    let mut s = Session::new();
    let (pts, _) = dimensioned_rectangle(&mut s);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["dof"], 0);
    assert_eq!(g["status"], "FullyConstrained");
    let area = g["profiles"][0]["area"].as_f64().unwrap();
    assert!((area - 1500.0).abs() < 1e-6, "{area}");
    // The far corner is at (50, 30).
    let far = g["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["point"] == pts[2])
        .unwrap();
    assert!((far["x"].as_f64().unwrap() - 50.0).abs() < 1e-6);
    assert!((far["y"].as_f64().unwrap() - 30.0).abs() < 1e-6);
}

#[test]
fn a_free_rectangle_has_four_degrees_of_freedom() {
    // Four points (8 unknowns) minus 4 horizontal/vertical equations: x, y, width, height.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 5}),
    )
    .unwrap();
    assert_eq!(s.execute("sketch.get", &json!({})).unwrap()["dof"], 4);
}

#[test]
fn changing_a_dimension_resizes_and_undo_restores_it() {
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    let width = s.doc.sketches[0]
        .sketch
        .constraints()
        .find(|(_, c)| matches!(c, chamfera_sketch::Constraint::Length { value, .. } if *value == 50.0))
        .map(|(id, _)| id.0)
        .unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "value": 80}),
    )
    .unwrap();
    let area = |s: &mut Session| {
        s.execute("sketch.get", &json!({})).unwrap()["profiles"][0]["area"]
            .as_f64()
            .unwrap()
    };
    assert!((area(&mut s) - 2400.0).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((area(&mut s) - 1500.0).abs() < 1e-6);
    // A width that cannot be solved (zero) is refused and changes nothing.
    assert!(
        s.execute(
            "sketch.dimension",
            &json!({"constraint": width, "value": 0})
        )
        .is_err()
    );
    assert!((area(&mut s) - 1500.0).abs() < 1e-6);
}

#[test]
fn a_conflicting_constraint_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    let (_, lines) = dimensioned_rectangle(&mut s);
    let before = s.doc.clone();
    let r = s.execute(
        "sketch.constrain",
        &json!({"constraint": {"kind": "length", "line": lines[2], "value": 70}}),
    );
    assert!(r.is_err());
    assert_eq!(s.doc, before);
}

#[test]
fn sketches_on_the_front_plane_map_to_x_and_z() {
    // XZ plane: sketch x is world X, sketch y is world Z (up), so y = 0 everywhere.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    s.execute("sketch.point", &json!({"x": 12, "y": 7}))
        .unwrap();
    let p = s
        .scene
        .overlays
        .iter()
        .flat_map(|o| o.points.iter())
        .next()
        .copied()
        .unwrap();
    assert_eq!((p.x, p.y, p.z), (12.0, 0.0, 7.0));
}

#[test]
fn a_finished_fully_constrained_sketch_draws_in_the_constrained_colour() {
    use chamfera_render::OverlayStyle;
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    let styles: Vec<OverlayStyle> = s.scene.overlays.iter().map(|o| o.style).collect();
    assert!(styles.contains(&OverlayStyle::Constrained));
    assert!(
        !styles.contains(&OverlayStyle::Sketch),
        "everything is determined: {styles:?}"
    );
    assert!(
        s.scene.overlays.iter().all(|o| o.on_top),
        "the sketch being edited is on top"
    );
    s.execute("sketch.finish", &json!({})).unwrap();
    assert!(
        s.scene.overlays.iter().all(|o| o.points.is_empty()),
        "points show only while editing"
    );
    assert!(
        s.scene.overlays.iter().all(|o| !o.on_top),
        "finished sketches are hidden by bodies in front"
    );
}

#[test]
fn clicks_pick_sketch_points_and_lines_and_find_the_plane() {
    // Top view centred on the origin, 800 x 600: the plane point under the centre pixel is
    // the camera target, and a point drawn there is picked by a click on that pixel.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(20.0, 10.0, 0.0);
    let centre = json!({"x": 400, "y": 300, "width": 800, "height": 600});
    let at = s.execute("sketch.plane_point", &centre).unwrap();
    assert!((at["x"].as_f64().unwrap() - 20.0).abs() < 1e-3, "{at}");
    assert!((at["y"].as_f64().unwrap() - 10.0).abs() < 1e-3, "{at}");
    let p = s
        .execute("sketch.point", &json!({"x": 20, "y": 10}))
        .unwrap()["point"]
        .as_u64()
        .unwrap();
    assert_eq!(
        s.execute("sketch.pick", &centre).unwrap()["item"]["point"],
        p
    );
    // A horizontal line through the centre, from (0, 10) to (40, 10), clicked 30 px to the
    // right of the centre (away from the point) picks the line.
    let l = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": 10, "x2": 40, "y2": 10}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    let right = json!({"x": 430, "y": 300, "width": 800, "height": 600});
    assert_eq!(
        s.execute("sketch.pick", &right).unwrap()["item"]["curve"],
        l
    );
}

#[test]
fn selection_drives_constraints_and_dimensions() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let a = s
        .execute("sketch.line", &json!({"x1": 0, "y1": 0, "x2": 30, "y2": 2}))
        .unwrap();
    let b = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": 10, "x2": 3, "y2": 40}),
        )
        .unwrap();
    let (la, lb) = (
        a["curve"].as_u64().unwrap() as u32,
        b["curve"].as_u64().unwrap() as u32,
    );
    use crate::sketching::SketchItem;
    s.sketch_selection = vec![SketchItem::Curve(la), SketchItem::Curve(lb)];
    s.execute(
        "sketch.constrain_selection",
        &json!({"kind": "perpendicular"}),
    )
    .unwrap();
    s.sketch_selection = vec![SketchItem::Curve(la)];
    let d = s.execute("sketch.dimension_selection", &json!({})).unwrap();
    assert_eq!(d["kind"], "length");
    // A line from (0,0) to (30,2) is sqrt(904) long; perpendicularity moves it but the solver
    // keeps it nearest, and the new dimension holds whatever length it then has.
    assert!(d["value"].as_f64().unwrap() > 20.0);
    s.sketch_selection = vec![SketchItem::Point(1)];
    assert!(
        s.execute("sketch.constrain_selection", &json!({"kind": "parallel"}))
            .is_err()
    );
}

#[test]
fn a_design_with_a_sketch_saves_and_opens() {
    let dir = std::env::temp_dir().join(format!("chamfera-sketch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plate.chamfera");
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    s.execute("design.save", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(t.doc, s.doc);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_parameter_drives_a_sketch_dimension_through_the_commands() {
    let mut s = Session::new();
    s.execute(
        "param.set",
        &json!({"name": "width", "expression": "50 mm"}),
    )
    .unwrap();
    let (_, lines) = dimensioned_rectangle(&mut s);
    let width = s.doc.sketches[0]
        .sketch
        .constraints()
        .find(|(_, c)| matches!(c, chamfera_sketch::Constraint::Length { line, .. } if u64::from(line.0) == lines[0]))
        .map(|(id, _)| id.0)
        .unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "expression": "width * 2"}),
    )
    .unwrap();
    let area = |s: &mut Session| {
        s.execute("sketch.get", &json!({})).unwrap()["profiles"][0]["area"]
            .as_f64()
            .unwrap()
    };
    assert!((area(&mut s) - 3000.0).abs() < 1e-6, "100 x 30");
    s.execute("param.set", &json!({"name": "width", "expression": "60"}))
        .unwrap();
    assert!((area(&mut s) - 3600.0).abs() < 1e-6, "120 x 30");
    // In use: the parameter cannot be removed while the dimension refers to it.
    assert!(
        s.execute("param.remove", &json!({"name": "width"}))
            .is_err()
    );
    // Undo goes back to width = 50, so 100 x 30.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((area(&mut s) - 3000.0).abs() < 1e-6);
    // A plain value takes over from the expression.
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "value": 10}),
    )
    .unwrap();
    s.execute("param.set", &json!({"name": "width", "expression": "99"}))
        .unwrap();
    assert!(
        (area(&mut s) - 300.0).abs() < 1e-6,
        "10 x 30, no longer driven"
    );
}

/// The plate of `dimensioned_rectangle` (50 x 30, fixed at the origin) extruded.
fn extruded_plate(s: &mut Session, distance: serde_json::Value) -> serde_json::Value {
    dimensioned_rectangle(s);
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": distance}),
    )
    .unwrap()
}

#[test]
fn extruding_a_rectangle_makes_a_box_with_its_exact_volume() {
    let mut s = Session::new();
    let r = extruded_plate(&mut s, json!(10));
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 15_000.0).abs() < 1e-6, "{v}");
    let a = r["solids"][0]["area"].as_f64().unwrap();
    assert!((a - 4_600.0).abs() < 1e-6, "{a}");
    // It is in the scene as a displayed body, and its mesh has the same volume.
    let mesh_volume = s
        .scene
        .bodies
        .iter()
        .map(|b| b.mesh.signed_volume())
        .sum::<f64>();
    assert!((mesh_volume - 15_000.0).abs() < 1e-6, "{mesh_volume}");
}

#[test]
fn a_parameter_drives_the_extrude_and_the_sketch_and_undo_follows() {
    let mut s = Session::new();
    s.execute("param.set", &json!({"name": "t", "expression": "10 mm"}))
        .unwrap();
    extruded_plate(&mut s, json!("t * 2"));
    let volume = |s: &mut Session| {
        s.execute("feature.list", &json!({})).unwrap()["features"][0]["solids"][0]["volume"]
            .as_f64()
            .unwrap()
    };
    assert!((volume(&mut s) - 30_000.0).abs() < 1e-6, "50 x 30 x 20");
    s.execute("param.set", &json!({"name": "t", "expression": "4"}))
        .unwrap();
    assert!((volume(&mut s) - 12_000.0).abs() < 1e-6, "50 x 30 x 8");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((volume(&mut s) - 30_000.0).abs() < 1e-6);
    // The sketch still drives it: a new width rebuilds the solid.
    let width = s.doc.sketches[0]
        .sketch
        .constraints()
        .find(|(_, c)| matches!(c, chamfera_sketch::Constraint::Length { value, .. } if *value == 50.0))
        .map(|(id, _)| id.0)
        .unwrap();
    s.execute("sketch.edit", &json!({"sketch": 1})).unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "value": 100}),
    )
    .unwrap();
    assert!((volume(&mut s) - 60_000.0).abs() < 1e-6, "100 x 30 x 20");
}

#[test]
fn a_hole_in_the_profile_goes_through_the_solid() {
    // An 80 x 50 plate with a centred square 10 x 10 hole, 5 thick: (4000 - 100) * 5.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 80, "y2": 50}),
    )
    .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 35, "y1": 20, "x2": 45, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let r = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 5}),
        )
        .unwrap();
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 19_500.0).abs() < 1e-6, "{v}");
}

#[test]
fn directions_place_the_solid_on_either_side_or_both() {
    for (direction, zmin, zmax) in [
        ("normal", 0.0, 10.0),
        ("reversed", -10.0, 0.0),
        ("symmetric", -5.0, 5.0),
    ] {
        let mut s = Session::new();
        dimensioned_rectangle(&mut s);
        s.execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 10, "direction": direction}),
        )
        .unwrap();
        let b = s.scene.bodies[0].mesh.bounds().unwrap();
        assert!(
            (b.min.z - zmin).abs() < 1e-9 && (b.max.z - zmax).abs() < 1e-9,
            "{direction}: {b:?}"
        );
    }
}

#[test]
fn a_bad_extrude_changes_nothing_and_a_broken_profile_is_reported() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let before = s.doc.clone();
    let bodies = s.scene.bodies.len();
    // A seed outside every profile, and a zero distance, are refused.
    for p in [
        json!({"sketch": 1, "profiles": [{"x": 500, "y": 5}], "distance": 5}),
        json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 0}),
    ] {
        assert!(s.execute("solid.extrude", &p).is_err());
        assert_eq!(s.doc, before);
        assert_eq!(s.scene.bodies.len(), bodies);
    }
    // Deleting the sketch is refused while the extrude uses it.
    assert!(s.execute("sketch.delete", &json!({"sketch": 1})).is_err());
    // Suppressing hides the solid, unsuppressing brings it back.
    s.execute("feature.edit", &json!({"feature": 2, "suppressed": true}))
        .unwrap();
    assert_eq!(s.solids().len(), 0);
    s.execute("feature.edit", &json!({"feature": 2, "suppressed": false}))
        .unwrap();
    assert_eq!(s.solids().len(), 1);
}

#[test]
fn clicking_inside_a_profile_finds_it() {
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(25.0, 15.0, 0.0);
    let r = s
        .execute(
            "solid.profile_at",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert_eq!(r["sketch"], 1);
    assert!((r["area"].as_f64().unwrap() - 1500.0).abs() < 1e-6);
}

fn exact_volume(r: &serde_json::Value) -> f64 {
    r["solids"][0]["volume"].as_f64().unwrap()
}

#[test]
fn circles_extrude_exactly_now() {
    // A full circle r = 5 extruded 10: π r² h = 250π exactly (no polygon error).
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 5}))
        .unwrap();
    let r = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 10}),
        )
        .unwrap();
    let v = exact_volume(&r);
    assert!((v - 250.0 * std::f64::consts::PI).abs() < 1e-9, "{v}");
}

#[test]
fn the_example_plate_has_its_exact_volume() {
    // examples/extruded-plate.json: 80 x 50 x 8 with two r = 8 holes:
    // (4000 - 2 * 64π) * 8 = 28783.009...; the 128-gon approximation gave 28784.30.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/extruded-plate.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    for step in steps.as_array().unwrap() {
        let params = step.get("params").cloned().unwrap_or(json!({}));
        s.execute(step["command"].as_str().unwrap(), &params)
            .unwrap();
    }
    let list = s.execute("feature.list", &json!({})).unwrap();
    let v = list["features"][0]["solids"][0]["volume"].as_f64().unwrap();
    let exact = (4000.0 - 128.0 * std::f64::consts::PI) * 8.0;
    assert!((v - exact).abs() < 1e-6, "{v} vs {exact}");
}

#[test]
fn revolving_a_rectangle_about_a_sketch_line_makes_a_tube() {
    // The rectangle x in [10, 15], y in [0, 20] turned about the sketch's y axis (the line
    // x = 0): a tube of π (15² - 10²) 20 = 2500π; a quarter turn is a quarter of it.
    for (angle, fraction) in [(json!(360), 1.0), (json!("90 deg"), 0.25)] {
        let mut s = Session::new();
        s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": 10, "y1": 0, "x2": 15, "y2": 20}),
        )
        .unwrap();
        let axis = s
            .execute(
                "sketch.line",
                &json!({"x1": 0, "y1": -5, "x2": 0, "y2": 25}),
            )
            .unwrap()["curve"]
            .as_u64()
            .unwrap();
        s.execute(
            "sketch.construction",
            &json!({"curve": axis, "construction": true}),
        )
        .unwrap();
        let r = s
            .execute(
                "solid.revolve",
                &json!({"sketch": 1, "profiles": [{"x": 12, "y": 10}], "axis": axis, "angle": angle}),
            )
            .unwrap();
        let v = exact_volume(&r);
        let expected = 2500.0 * std::f64::consts::PI * fraction;
        assert!((v - expected).abs() < 1e-6, "{v} vs {expected}");
    }
}

#[test]
fn a_revolve_that_crosses_its_axis_is_refused() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": -5, "y1": 0, "x2": 5, "y2": 10}),
    )
    .unwrap();
    let axis = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": -5, "x2": 0, "y2": 15}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.construction",
        &json!({"curve": axis, "construction": true}),
    )
    .unwrap();
    let before = s.doc.clone();
    assert!(
        s.execute(
            "solid.revolve",
            &json!({"sketch": 1, "profiles": [{"x": 2, "y": 5}], "axis": axis}),
        )
        .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn saving_tracks_the_file_and_unsaved_changes_and_exports_write_files() {
    let dir = std::env::temp_dir().join(format!("chamfera-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let design = dir.join("box.chamfera");
    let mut s = Session::new();
    assert!(!s.has_unsaved_changes());
    extruded_plate(&mut s, json!(10));
    assert!(s.has_unsaved_changes());
    // No file yet: a save needs a path.
    assert!(s.execute("design.save", &json!({})).is_err());
    s.execute("design.save", &json!({"path": design.to_str().unwrap()}))
        .unwrap();
    assert!(!s.has_unsaved_changes());
    s.execute("param.set", &json!({"name": "x", "value": 1}))
        .unwrap();
    assert!(s.has_unsaved_changes());
    s.execute("design.save", &json!({})).unwrap();
    assert!(!s.has_unsaved_changes(), "saved again to the same file");
    // STEP: one solid, a valid Part 21 file that our reader counts as a 6-face box.
    let step = dir.join("box.step");
    s.execute(
        "design.export_step",
        &json!({"path": step.to_str().unwrap()}),
    )
    .unwrap();
    let text = std::fs::read_to_string(&step).unwrap();
    assert!(text.starts_with("ISO-10303-21;"));
    assert_eq!(text.matches("ADVANCED_FACE(").count(), 6);
    // STL: the 12 triangles of the box, volume 15000 when read back.
    let stl = dir.join("box.stl");
    let r = s
        .execute("design.export_stl", &json!({"path": stl.to_str().unwrap()}))
        .unwrap();
    assert_eq!(r["triangles"], 12);
    let mesh = chamfera_io::stl::read(&std::fs::read(&stl).unwrap()).unwrap();
    assert!((mesh.signed_volume() - 15_000.0).abs() < 1e-3);
    // Opening it back gives the same design and nothing unsaved.
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": design.to_str().unwrap()}))
        .unwrap();
    assert!(!t.has_unsaved_changes());
    assert_eq!(t.solids().len(), 1, "opening rebuilds the solids");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sketch_on_the_top_face_follows_the_plate_when_it_gets_thicker() {
    // Plate 50 x 30, 10 thick; a 10 x 10 boss sketched on its top face, extruded 5:
    // the boss spans z 10..15. Make the plate 20 thick: the boss moves to z 20..25.
    let mut s = Session::new();
    extruded_plate(&mut s, json!("10"));
    let r = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top"}),
        )
        .unwrap();
    assert!(
        (r["origin"]["z"].as_f64().unwrap() - 10.0).abs() < 1e-9,
        "{r}"
    );
    assert!(
        (r["normal"]["z"].as_f64().unwrap() - 1.0).abs() < 1e-12,
        "outward normal"
    );
    let sketch = r["sketch"].as_u64().unwrap();
    // The face frame's x and y match the base sketch's: a rectangle at (20..30, 10..20).
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 20, "y1": 10, "x2": 30, "y2": 20}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let boss = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sketch, "profiles": [{"x": 25, "y": 15}], "distance": 5}),
        )
        .unwrap();
    let boss_id = boss["feature"].as_u64().unwrap();
    let z_range = |s: &Session| {
        let solid = s
            .solids()
            .iter()
            .find(|x| u64::from(x.feature) == boss_id)
            .unwrap();
        let b = s
            .scene
            .body(solid.scene_id.unwrap())
            .unwrap()
            .mesh
            .bounds()
            .unwrap();
        (b.min.z, b.max.z)
    };
    assert_eq!(z_range(&s), (10.0, 15.0));
    s.execute("feature.edit", &json!({"feature": 2, "distance": "20"}))
        .unwrap();
    assert_eq!(z_range(&s), (20.0, 25.0));
    // The plate cannot be deleted while the boss's sketch sits on it.
    assert!(s.execute("feature.delete", &json!({"feature": 2})).is_err());
    // A side face works too: its normal points out of the plate.
    let side = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "side 0"}),
        )
        .unwrap();
    let n = &side["normal"];
    let len = (n["x"].as_f64().unwrap().powi(2) + n["y"].as_f64().unwrap().powi(2)).sqrt();
    assert!(
        (len - 1.0).abs() < 1e-9 && n["z"].as_f64().unwrap().abs() < 1e-12,
        "{side}"
    );
}

#[test]
fn clicking_a_face_creates_a_sketch_on_it() {
    // Top view of the 10-thick plate centred on (25, 15): the centre pixel hits the top.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(25.0, 15.0, 5.0);
    let r = s
        .execute(
            "sketch.create_on_face",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    assert_eq!(r["face"], "top");
    // Nothing under a corner pixel: refused.
    let mut t = Session::new();
    extruded_plate(&mut t, json!(10));
    t.execute("view.standard", &json!({"view": "top"})).unwrap();
    t.camera.target = chamfera_geom::Point3::new(25.0, 15.0, 5.0);
    assert!(
        t.execute(
            "sketch.create_on_face",
            &json!({"x": 1, "y": 1, "width": 800, "height": 600}),
        )
        .is_err()
    );
}

#[test]
fn a_sketch_used_by_a_feature_is_hidden_until_it_is_edited() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    assert!(s.scene.overlays.is_empty(), "the consumed sketch is hidden");
    s.execute("sketch.edit", &json!({"sketch": 1})).unwrap();
    assert!(!s.scene.overlays.is_empty(), "editing shows it");
    s.execute("sketch.finish", &json!({})).unwrap();
    assert!(s.scene.overlays.is_empty());
}

#[test]
fn join_and_cut_need_a_body_to_act_on() {
    // A cut far away from every body: nothing to cut, refused, nothing changes.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 500, "y1": 500, "x2": 510, "y2": 510}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let before = s.doc.clone();
    let far = s.execute(
        "solid.extrude",
        &json!({"sketch": 3, "profiles": [{"x": 505, "y": 505}], "distance": 5, "operation": "cut"}),
    );
    assert!(far.unwrap_err().to_string().contains("nothing to cut"));
    assert_eq!(s.doc, before);
    // An unknown operation is a parameter error.
    assert!(s
        .execute(
            "solid.extrude",
            &json!({"sketch": 3, "profiles": [{"x": 505, "y": 505}], "distance": 5, "operation": "smash"}),
        )
        .is_err());
}

#[test]
fn a_boss_joined_on_top_makes_one_solid_and_a_cut_drills_a_hole() {
    // Plate 50 x 30 x 10 = 15000. A 10 x 10 boss on its top face, 5 tall, joined: one body
    // of 15000 + 500. Then a circle r = 4 on the boss's top, cut through everything by
    // 15 (reversed): a hole through boss and plate, 15 deep: 15500 - 16π * 15.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let top = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top"}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 20, "y1": 10, "x2": 30, "y2": 20}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let boss = s
        .execute(
            "solid.extrude",
            &json!({"sketch": top, "profiles": [{"x": 25, "y": 15}], "distance": 5, "operation": "join"}),
        )
        .unwrap();
    assert_eq!(s.solids().len(), 1, "joined into one solid");
    let v = boss["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 15_500.0).abs() < 1e-6, "{v}");
    // The plate's top is still found by name after the join.
    let boss_id = boss["feature"].as_u64().unwrap();
    let hole_sketch = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": boss_id, "face": "top"}),
        )
        .unwrap();
    assert!((hole_sketch["origin"]["z"].as_f64().unwrap() - 15.0).abs() < 1e-9);
    let hs = hole_sketch["sketch"].as_u64().unwrap();
    s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 4}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let cut = s
        .execute(
            "solid.extrude",
            &json!({"sketch": hs, "profiles": [{"x": 25, "y": 15}], "distance": 15, "direction": "reversed", "operation": "cut"}),
        )
        .unwrap();
    assert_eq!(s.solids().len(), 1);
    let v = cut["solids"][0]["volume"].as_f64().unwrap();
    let expected = 15_500.0 - 16.0 * std::f64::consts::PI * 15.0;
    assert!((v - expected).abs() < 1e-6, "{v} vs {expected}");
    // A thicker plate rebuilds everything: boss on top, hole through both.
    s.execute("feature.edit", &json!({"feature": 2, "distance": "20"}))
        .unwrap();
    let list = s.execute("feature.list", &json!({})).unwrap();
    assert!(
        list["features"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["error"].is_null()),
        "{list}"
    );
    // 50 x 30 x 20 + boss 500 - hole r 4 through 15 (boss 5 + 10 of the plate): the cut
    // starts at the boss top (z 25) and goes 15 down to z 10.
    let v = s.solids()[0].body.volume().unwrap();
    let expected = 30_000.0 + 500.0 - 16.0 * std::f64::consts::PI * 15.0;
    assert!((v - expected).abs() < 1e-6, "{v} vs {expected}");
}

#[test]
fn a_linear_pattern_of_a_cut_drills_a_row_of_holes() {
    // Plate 50 x 30 x 10; one hole r = 3 at (10, 15) cut through; a pattern of 4 along +X,
    // 10 apart: holes at x = 10, 20, 30, 40. Volume 15000 - 4 * 9π * 10.
    let mut s = Session::new();
    s.execute(
        "param.set",
        &json!({"name": "holes", "expression": "4", "unit": "unitless"}),
    )
    .unwrap();
    extruded_plate(&mut s, json!(10));
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 10, "cy": 15, "radius": 3}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let cut = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 3, "profiles": [{"x": 10, "y": 15}], "distance": 10, "operation": "cut"}),
        )
        .unwrap();
    let cut_id = cut["feature"].as_u64().unwrap();
    let r = s
        .execute(
            "solid.pattern_linear",
            &json!({"feature": cut_id, "count": "holes", "direction": {"x": 1, "y": 0, "z": 0}, "spacing": 10}),
        )
        .unwrap();
    let pi = std::f64::consts::PI;
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - (15_000.0 - 4.0 * 9.0 * pi * 10.0)).abs() < 1e-6, "{v}");
    // The parameter drives the count: 2 holes.
    s.execute(
        "param.set",
        &json!({"name": "holes", "expression": "2", "unit": "unitless"}),
    )
    .unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!((v - (15_000.0 - 2.0 * 9.0 * pi * 10.0)).abs() < 1e-6, "{v}");
    // The cut it repeats cannot be deleted while the pattern uses it.
    assert!(
        s.execute("feature.delete", &json!({"feature": cut_id}))
            .is_err()
    );
}

#[test]
fn a_circular_pattern_of_new_bodies_makes_copies_around_an_axis() {
    // A 4 x 4 x 4 cube at (20..24, -2..2) patterned 6 times around Z: 6 bodies of 64.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 20, "y1": -2, "x2": 24, "y2": 2}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 22, "y": 0}], "distance": 4}),
    )
    .unwrap();
    s.execute(
        "solid.pattern_circular",
        &json!({"feature": 2, "count": 6, "origin": {"x": 0, "y": 0, "z": 0}, "axis": {"x": 0, "y": 0, "z": 1}}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 6);
    for solid in s.solids() {
        assert!((solid.body.volume().unwrap() - 64.0).abs() < 1e-9);
    }
    // Copy 4 (180 degrees) sits at x -24..-20.
    let b = s.solids()[3].body.bounding_box().unwrap();
    assert!(
        (b.min.x + 24.0).abs() < 1e-9 && (b.max.x + 20.0).abs() < 1e-9,
        "{b:?}"
    );
    // Bad counts are refused.
    assert!(s
        .execute(
            "solid.pattern_circular",
            &json!({"feature": 2, "count": 1, "origin": {"x": 0, "y": 0, "z": 0}, "axis": {"x": 0, "y": 0, "z": 1}}),
        )
        .is_err());
}

/// The name of the edge of the first solid running between `a` and `b` (either way).
fn edge_between(s: &mut Session, a: [f64; 3], b: [f64; 3]) -> String {
    let r = s.execute("solid.edges", &json!({})).unwrap();
    let near = |v: &serde_json::Value, p: [f64; 3]| {
        (0..3).all(|i| (v[i].as_f64().unwrap() - p[i]).abs() < 1e-9)
    };
    r["solids"][0]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            (near(&e["from"], a) && near(&e["to"], b)) || (near(&e["from"], b) && near(&e["to"], a))
        })
        .and_then(|e| e["edge"].as_str())
        .unwrap()
        .to_owned()
}

#[test]
fn a_fillet_rounds_one_box_edge_by_its_exact_volume_and_follows_the_box() {
    // The 50 x 30 x 10 plate; rounding the 50 mm top front edge with radius r removes
    // the corner region (1 - π/4) r² along its length.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let edge = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    let r = s
        .execute("solid.fillet", &json!({"edges": [edge], "radius": 2}))
        .unwrap();
    let removed = |r: f64| (1.0 - std::f64::consts::FRAC_PI_4) * r * r * 50.0;
    let v = exact_volume(&r);
    assert!((v - (15_000.0 - removed(2.0))).abs() < 1e-6, "{v}");
    let fillet = r["feature"].as_u64().unwrap();
    // A thicker plate rebuilds with the same edge rounded.
    s.execute("feature.edit", &json!({"feature": 2, "distance": 20}))
        .unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!((v - (30_000.0 - removed(2.0))).abs() < 1e-6, "{v}");
    // A new radius.
    let r = s
        .execute("feature.edit", &json!({"feature": fillet, "size": "3 mm"}))
        .unwrap();
    let v = exact_volume(&r);
    assert!((v - (30_000.0 - removed(3.0))).abs() < 1e-6, "{v}");
    // It saves and opens with the fillet in place.
    let dir = std::env::temp_dir().join(format!("chamfera-fillet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.chamfera");
    s.execute("design.save", &json!({"path": path.to_string_lossy()}))
        .unwrap();
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": path.to_string_lossy()}))
        .unwrap();
    let v = t.solids()[0].body.volume().unwrap();
    assert!((v - (30_000.0 - removed(3.0))).abs() < 1e-6, "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chamfer_bevels_an_edge_by_its_exact_volume() {
    // An equal-distance chamfer d removes a right triangle d²/2 along the edge.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let edge = edge_between(&mut s, [50.0, 0.0, 0.0], [50.0, 0.0, 10.0]);
    let r = s
        .execute(
            "solid.chamfer",
            &json!({"edges": [edge], "distance": "1.5"}),
        )
        .unwrap();
    let v = exact_volume(&r);
    assert!(
        (v - (15_000.0 - 1.5 * 1.5 / 2.0 * 10.0)).abs() < 1e-6,
        "{v}"
    );
}

#[test]
fn a_blend_of_a_missing_or_too_large_edge_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let before = s.doc.features.len();
    assert!(
        s.execute(
            "solid.fillet",
            &json!({"edges": ["7:nothing"], "radius": 1})
        )
        .is_err()
    );
    let edge = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    // A radius larger than the plate is thick would swallow the faces.
    assert!(
        s.execute("solid.fillet", &json!({"edges": [edge], "radius": 40}))
            .is_err()
    );
    assert_eq!(s.doc.features.len(), before);
    assert!((s.solids()[0].body.volume().unwrap() - 15_000.0).abs() < 1e-6);
}

#[test]
fn clicking_near_a_visible_edge_picks_it_and_fillets_it() {
    use chamfera_render::{mat_mul, transform_point};
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let (w, h) = (800.0, 600.0);
    s.execute("view.home", &json!({"width": w, "height": h}))
        .unwrap();
    // Where the middle of the top front edge (25, 0, 10) lands on screen, 3 px off.
    let vp = mat_mul(
        &s.camera.projection_matrix(w / h, s.scene.bounds().as_ref()),
        &s.camera.view_matrix(),
    );
    let n = transform_point(&vp, chamfera_geom::Point3::new(25.0, 0.0, 10.0)).unwrap();
    let (x, y) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h + 3.0);
    let want = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    let hit = s
        .execute(
            "solid.edge_at",
            &json!({"x": x, "y": y, "width": w, "height": h}),
        )
        .unwrap();
    assert_eq!(hit["edge"].as_str(), Some(want.as_str()));
    // The bottom back edge (25, 30, 0) is behind the box: never picked there.
    let n = transform_point(&vp, chamfera_geom::Point3::new(25.0, 30.0, 0.0)).unwrap();
    let (bx, by) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h);
    let hidden = edge_between(&mut s, [0.0, 30.0, 0.0], [50.0, 30.0, 0.0]);
    let hit = s
        .execute(
            "solid.edge_at",
            &json!({"x": bx, "y": by, "width": w, "height": h}),
        )
        .unwrap();
    assert_ne!(hit["edge"].as_str(), Some(hidden.as_str()));
    // A fillet by pick rounds the picked edge.
    let r = s
        .execute(
            "solid.fillet",
            &json!({"pick": {"x": x, "y": y, "width": w, "height": h}, "radius": 2}),
        )
        .unwrap();
    let v = exact_volume(&r);
    let removed = (1.0 - std::f64::consts::FRAC_PI_4) * 4.0 * 50.0;
    assert!((v - (15_000.0 - removed)).abs() < 1e-6, "{v}");
}

#[test]
fn a_shell_hollows_the_plate_by_its_exact_volume_and_follows_edits() {
    // 2 mm walls with the top open: the cavity is (50 - 4) x (30 - 4) x (h - 2).
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let r = s
        .execute("solid.shell", &json!({"faces": ["2:top"], "thickness": 2}))
        .unwrap();
    assert!((exact_volume(&r) - (15_000.0 - 46.0 * 26.0 * 8.0)).abs() < 1e-6);
    let shell = r["feature"].as_u64().unwrap();
    // The inner walls are named after the faces they offset.
    let names: Vec<String> = s.solids()[0]
        .body
        .faces()
        .into_iter()
        .filter_map(|f| {
            s.solids()[0]
                .body
                .face_name(f)
                .ok()
                .flatten()
                .map(str::to_owned)
        })
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n == &format!("{shell}:shell of 2:bottom"))
    );
    // A thicker plate rebuilds with the shell in place.
    s.execute("feature.edit", &json!({"feature": 2, "distance": 20}))
        .unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!((v - (30_000.0 - 46.0 * 26.0 * 18.0)).abs() < 1e-6, "{v}");
    // A new thickness, then no open face: a closed hollow box.
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": shell, "thickness": "1 mm"}),
        )
        .unwrap();
    assert!((exact_volume(&r) - (30_000.0 - 48.0 * 28.0 * 19.0)).abs() < 1e-6);
    let r = s
        .execute("feature.edit", &json!({"feature": shell, "faces": []}))
        .unwrap();
    assert!((exact_volume(&r) - (30_000.0 - 48.0 * 28.0 * 18.0)).abs() < 1e-6);
    // Undo goes back to the open top.
    s.execute("edit.undo", &json!({})).unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!((v - (30_000.0 - 48.0 * 28.0 * 19.0)).abs() < 1e-6, "{v}");
    // Size and thickness belong to their own features.
    assert!(
        s.execute("feature.edit", &json!({"feature": 2, "thickness": 1}))
            .is_err()
    );
    assert!(
        s.execute("feature.edit", &json!({"feature": shell, "size": 1}))
            .is_err()
    );
}

#[test]
fn a_too_thick_or_zero_shell_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let before = s.doc.features.len();
    for p in [
        json!({"faces": ["2:top"], "thickness": 20}),
        json!({"faces": ["2:top"], "thickness": 0}),
        json!({"faces": ["9:nowhere"], "thickness": 1}),
        json!({"faces": ["2:top"]}),
    ] {
        assert!(s.execute("solid.shell", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc.features.len(), before);
    assert!((s.solids()[0].body.volume().unwrap() - 15_000.0).abs() < 1e-6);
}

#[test]
fn clicking_a_face_opens_it_in_a_shell() {
    use chamfera_render::{mat_mul, transform_point};
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let (w, h) = (800.0, 600.0);
    s.execute("view.home", &json!({"width": w, "height": h}))
        .unwrap();
    // Where the middle of the top face (25, 15, 10) lands on screen.
    let vp = mat_mul(
        &s.camera.projection_matrix(w / h, s.scene.bounds().as_ref()),
        &s.camera.view_matrix(),
    );
    let n = transform_point(&vp, chamfera_geom::Point3::new(25.0, 15.0, 10.0)).unwrap();
    let (x, y) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h);
    let r = s
        .execute(
            "solid.shell",
            &json!({"pick": {"x": x, "y": y, "width": w, "height": h}, "thickness": "2 mm"}),
        )
        .unwrap();
    assert_eq!(r["definition"]["faces"], json!(["2:top"]));
    assert!((exact_volume(&r) - (15_000.0 - 46.0 * 26.0 * 8.0)).abs() < 1e-6);
}

// ----- sketch tools -----

fn ids(v: &serde_json::Value) -> Vec<u64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect()
}

fn sketch_areas(s: &mut Session) -> Vec<f64> {
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let mut a: Vec<f64> = g["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["area"].as_f64().unwrap())
        .collect();
    a.sort_by(f64::total_cmp);
    a
}

#[test]
fn filleting_a_dimensioned_rectangle_keeps_it_fully_constrained_and_undo_restores_it() {
    let mut s = Session::new();
    s.execute(
        "param.set",
        &json!({"name": "width", "expression": "50 mm"}),
    )
    .unwrap();
    let (pts, lines) = dimensioned_rectangle(&mut s);
    let width = s.doc.sketches[0]
        .sketch
        .constraints()
        .find(|(_, c)| matches!(c, chamfera_sketch::Constraint::Length { line, .. } if u64::from(line.0) == lines[0]))
        .map(|(id, _)| id.0)
        .unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "expression": "width"}),
    )
    .unwrap();
    for p in &pts {
        let r = s
            .execute("sketch.fillet", &json!({"point": p, "radius": 5}))
            .unwrap();
        assert_eq!(ids(&r["created_curves"]).len(), 1);
    }
    let g = s.execute("sketch.get", &json!({})).unwrap();
    // The side lengths became distances to the kept sharp corners: still no freedom.
    assert_eq!(g["dof"], 0);
    assert_eq!(g["status"], "FullyConstrained");
    let filleted = |w: f64| w * 30.0 - (4.0 - std::f64::consts::PI) * 25.0;
    let a = sketch_areas(&mut s);
    assert!((a[0] - filleted(50.0)).abs() < 1e-6, "{a:?}");
    // The width parameter still drives the bottom side through its corners.
    s.execute("param.set", &json!({"name": "width", "expression": "60"}))
        .unwrap();
    let a = sketch_areas(&mut s);
    assert!((a[0] - filleted(60.0)).abs() < 1e-6, "{a:?}");
    for _ in 0..5 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    let a = sketch_areas(&mut s);
    assert!((a[0] - 1500.0).abs() < 1e-6, "{a:?}");
}

#[test]
fn trim_extend_and_split_through_the_commands() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let h = s
        .execute(
            "sketch.line",
            &json!({"x1": -10, "y1": 0, "x2": 10, "y2": 0}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.line",
        &json!({"x1": 0, "y1": -10, "x2": 0, "y2": 10}),
    )
    .unwrap();
    let r = s
        .execute("sketch.trim", &json!({"curve": h, "x": 5, "y": 0}))
        .unwrap();
    assert_eq!(ids(&r["changed_curves"]), vec![h]);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["curves"].as_array().unwrap().len(), 2);
    // Its free end has nothing ahead of it: refused, nothing changes.
    assert!(
        s.execute("sketch.extend", &json!({"curve": h, "x": -9, "y": 0}))
            .is_err()
    );
    // A line at x = -15 gives it something to reach: the end moves from -10 to -15.
    s.execute(
        "sketch.line",
        &json!({"x1": -15, "y1": -5, "x2": -15, "y2": 5}),
    )
    .unwrap();
    s.execute("sketch.extend", &json!({"curve": h, "x": -9, "y": 0}))
        .unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert!(
        g["points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| (q["x"].as_f64().unwrap() + 15.0).abs() < 1e-9
                && q["y"].as_f64().unwrap().abs() < 1e-9)
    );
    // Two crossing squares split into their three regions; undo puts them back.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 20, "y2": 20}),
    )
    .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 10, "y1": 10, "x2": 30, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.split", &json!({})).unwrap();
    let a = sketch_areas(&mut s);
    assert_eq!(a.len(), 3);
    assert!((a[0] - 100.0).abs() < 1e-6 && (a[2] - 300.0).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["curves"].as_array().unwrap().len(), 8);
}

#[test]
fn offset_mirror_and_patterns_through_the_commands() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        )
        .unwrap();
    let sides = ids(&r["curves"]);
    s.execute(
        "sketch.offset",
        &json!({"curves": [sides[0]], "distance": 2, "corners": "round"}),
    )
    .unwrap();
    // The ring around the original: 2 · (50 + 30) · 2 + π 2².
    let a = sketch_areas(&mut s);
    assert!(
        (a[0] - (320.0 + 4.0 * std::f64::consts::PI)).abs() < 1e-6,
        "{a:?}"
    );
    assert!(
        s.execute(
            "sketch.offset",
            &json!({"curves": [sides[0]], "distance": 2, "corners": "bevel"}),
        )
        .is_err()
    );

    // Mirror a circle in a vertical line, using the selection.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let axis = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": -10, "x2": 0, "y2": 10}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    let c = s
        .execute("sketch.circle", &json!({"cx": 4, "cy": 2, "radius": 1}))
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.sketch_selection = vec![crate::sketching::SketchItem::Curve(c as u32)];
    let r = s.execute("sketch.mirror", &json!({"axis": axis})).unwrap();
    let copy = ids(&r["created_curves"]);
    assert_eq!(copy.len(), 1);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let centre = g["curves"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["curve"] == copy[0])
        .unwrap()["geometry"]["centre"]
        .clone();
    let p = g["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["point"] == centre)
        .unwrap();
    assert!((p["x"].as_f64().unwrap() + 4.0).abs() < 1e-9);
    assert!((p["y"].as_f64().unwrap() - 2.0).abs() < 1e-9);

    // Six around the origin, then a 3 x 2 grid of the original.
    let r = s
        .execute(
            "sketch.pattern.circular",
            &json!({"curves": [c], "cx": 0, "cy": 0, "count": 6}),
        )
        .unwrap();
    assert_eq!(ids(&r["created_curves"]).len(), 5);
    let r = s
        .execute(
            "sketch.pattern.rectangular",
            &json!({"curves": [c], "columns": 3, "dx": 10, "rows": 2, "dy": 5}),
        )
        .unwrap();
    assert_eq!(ids(&r["created_curves"]).len(), 5);
    assert!(
        s.execute(
            "sketch.pattern.rectangular",
            &json!({"curves": [c], "columns": 2.5, "dx": 10}),
        )
        .is_err()
    );
}

#[test]
fn centre_rectangle_arc_and_chamfer_commands_draw_exact_shapes() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle.centre",
        &json!({"cx": 10, "cy": 10, "x": 15, "y": 13}),
    )
    .unwrap();
    let a = sketch_areas(&mut s);
    assert!((a[0] - 60.0).abs() < 1e-6, "10 x 6");
    let r = s
        .execute(
            "sketch.arc.three_point",
            &json!({"x1": 5, "y1": 0, "x2": 0, "y2": 5, "x3": -5, "y3": 0}),
        )
        .unwrap();
    assert_eq!(ids(&r["created_curves"]).len(), 1);
    let l = s
        .execute(
            "sketch.line",
            &json!({"x1": 30, "y1": 0, "x2": 40, "y2": 0}),
        )
        .unwrap();
    let r = s
        .execute(
            "sketch.arc.tangent",
            &json!({"point": l["end"], "x": 40, "y": 10}),
        )
        .unwrap();
    assert_eq!(ids(&r["created_curves"]).len(), 1);
    // A chamfer on the centre rectangle's corner (15, 13): 60 - 2 · 2 / 2.
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let corner = g["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["x"] == 15.0 && q["y"] == 13.0)
        .unwrap()["point"]
        .clone();
    s.execute("sketch.chamfer", &json!({"point": corner, "distance": 2}))
        .unwrap();
    let a = sketch_areas(&mut s);
    let largest = a.iter().cloned().fold(0.0, f64::max);
    assert!((largest - 58.0).abs() < 1e-6, "{a:?}");
}

#[test]
fn body_import_step_adds_exact_solids_that_undo_save_and_open_keep() {
    use chamfera_geom::{Point3, Vec3};
    let dir = std::env::temp_dir().join(format!("chamfera-step-in-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let step = dir.join("block.step");
    let block = chamfera_kernel::Body::cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0)).unwrap();
    chamfera_io::step::write_file(&step, &[("block", &block)], &Default::default()).unwrap();
    let mut s = Session::new();
    let r = s
        .execute(
            "body.import_step",
            &json!({ "path": step.to_str().unwrap() }),
        )
        .unwrap();
    // 50 * 30 * 10 = 15000 mm^3; area 2 (1500 + 500 + 300) = 4600 mm^2.
    let solid = &r["solids"][0];
    assert_eq!(solid["name"], "block");
    assert!((solid["volume"].as_f64().unwrap() - 15_000.0).abs() < 1e-9);
    assert!((solid["area"].as_f64().unwrap() - 4_600.0).abs() < 1e-9);
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(s.solids().len(), 1);
    assert_eq!(s.doc.imported.len(), 1);
    let scene_id = solid["body"].as_u64().unwrap() as u32;
    let shown = s.scene.body(scene_id).unwrap();
    assert!((shown.mesh.signed_volume() - 15_000.0).abs() < 1e-6);
    // Faces are named like built faces: `id:role`.
    let id = solid["id"].as_u64().unwrap() as u32;
    let b = &s.solids()[0].body;
    for f in b.faces() {
        let name = b.face_name(f).unwrap().unwrap();
        assert!(name.starts_with(&format!("{id}:")), "{name}");
    }
    // Save and open keep it; undo removes it.
    let design = dir.join("d.chamfera");
    s.execute("design.save", &json!({ "path": design.to_str().unwrap() }))
        .unwrap();
    let mut t = Session::new();
    t.execute("design.open", &json!({ "path": design.to_str().unwrap() }))
        .unwrap();
    assert_eq!(t.solids().len(), 1);
    assert!((t.solids()[0].body.volume().unwrap() - 15_000.0).abs() < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.solids().is_empty() && s.doc.imported.is_empty());
    assert!(s.scene.body(scene_id).is_none());
    // A file with nothing importable fails and changes nothing.
    let bad = dir.join("bad.step");
    std::fs::write(
        &bad,
        "ISO-10303-21;HEADER;ENDSEC;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;",
    )
    .unwrap();
    assert!(
        s.execute(
            "body.import_step",
            &json!({ "path": bad.to_str().unwrap() })
        )
        .is_err()
    );
    assert!(s.doc.imported.is_empty());
    assert!(
        s.execute("body.import_step", &json!({ "path": "/dev/zero" }))
            .is_err()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- selection filters and renaming -----

/// The extruded plate (50 x 30 x 10) seen from the top, centred on (25, 15), in an
/// 800 x 600 viewport; returns where model point `p` lands on screen.
fn plate_from_the_top(s: &mut Session) -> impl Fn([f64; 3]) -> (f64, f64) + use<> {
    use chamfera_render::{mat_mul, transform_point};
    extruded_plate(s, json!(10));
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(25.0, 15.0, 0.0);
    let vp = mat_mul(
        &s.camera
            .projection_matrix(800.0 / 600.0, s.scene.bounds().as_ref()),
        &s.camera.view_matrix(),
    );
    move |p| {
        let n = transform_point(&vp, chamfera_geom::Point3::new(p[0], p[1], p[2])).unwrap();
        ((n[0] + 1.0) * 0.5 * 800.0, (1.0 - n[1]) * 0.5 * 600.0)
    }
}

#[test]
fn a_pick_filter_chooses_edges_faces_or_bodies() {
    let mut s = Session::new();
    let screen = plate_from_the_top(&mut s);
    let pick = |s: &mut Session, (x, y): (f64, f64), filter: serde_json::Value| {
        s.execute(
            "select.pick",
            &json!({"x": x, "y": y, "width": 800, "height": 600, "filter": filter}),
        )
        .unwrap()
    };
    // The centre pixel looks down on the middle of the top face (z = 10), far from edges.
    let centre = screen([25.0, 15.0, 10.0]);
    let r = pick(&mut s, centre, json!(["edges"]));
    assert!(r["body"].is_null(), "only edges pass: {r}");
    assert_eq!(r["edges"], json!([]));
    let r = pick(&mut s, centre, json!(["faces"]));
    assert!(r["face"].as_str().unwrap().ends_with(":top"), "{r}");
    assert!(
        (r["point"]["z"].as_f64().unwrap() - 10.0).abs() < 1e-6,
        "{r}"
    );
    let r = pick(&mut s, centre, json!(["bodies"]));
    assert!(r["body"].is_u64(), "{r}");
    // 3 px off the middle of the top front edge (25, 0, 10), inside the face: edges come
    // first.
    let (ex, ey) = screen([25.0, 0.0, 10.0]);
    let near = (ex, ey - 3.0);
    let want = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    let r = pick(&mut s, near, json!(["edges", "faces", "bodies"]));
    assert_eq!(r["edges"], json!([want]), "{r}");
    assert!(r["body"].is_null(), "{r}");
    // Without edges the same pixel picks the top face.
    let r = pick(&mut s, near, json!(["faces"]));
    assert!(r["face"].as_str().unwrap().ends_with(":top"), "{r}");
    assert_eq!(r["edges"], json!([]));
    // Nothing allowed: nothing picked. Clearing drops picked edges too.
    assert_eq!(pick(&mut s, centre, json!([]))["body"], json!(null));
    pick(&mut s, near, json!(["edges"]));
    let r = s.execute("select.clear", &json!({})).unwrap();
    assert_eq!(r["edges"], json!([]));
    // Without a filter the pick is the body hit, as before.
    let r = s
        .execute(
            "select.pick",
            &json!({"x": near.0, "y": near.1, "width": 800, "height": 600}),
        )
        .unwrap();
    assert!(r["body"].is_u64(), "{r}");
    // Bad filters are refused.
    for bad in [json!("edges"), json!(["teeth"]), json!([3])] {
        let p = json!({"x": 400, "y": 300, "width": 800, "height": 600, "filter": bad});
        assert!(s.execute("select.pick", &p).is_err(), "{p}");
    }
}

#[test]
fn a_sketch_filter_passes_points_curves_or_construction() {
    // As in `clicks_pick_sketch_points_and_lines_and_find_the_plane`: a top view centred
    // on (20, 10), with a point there on a line through it.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.target = chamfera_geom::Point3::new(20.0, 10.0, 0.0);
    let curve = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": 10, "x2": 40, "y2": 10}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    let p = s
        .execute("sketch.point", &json!({"x": 20, "y": 10}))
        .unwrap()["point"]
        .as_u64()
        .unwrap();
    let select = |s: &mut Session, filter: serde_json::Value| {
        s.execute(
            "sketch.select",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600, "filter": filter}),
        )
        .unwrap()["item"]
            .clone()
    };
    assert_eq!(select(&mut s, json!(["vertices", "sketch"]))["point"], p);
    // Points filtered out: the line under the same pixel is picked instead.
    assert_eq!(select(&mut s, json!(["sketch"]))["curve"], curve);
    // A construction line passes only with construction allowed.
    s.execute(
        "sketch.construction",
        &json!({"curve": curve, "construction": true}),
    )
    .unwrap();
    assert!(select(&mut s, json!(["sketch"])).is_null());
    assert_eq!(select(&mut s, json!(["construction"]))["curve"], curve);
}

#[test]
fn renaming_features_sketches_and_bodies_and_undo() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let feature = s.doc.features[0].id;
    let before = s.doc.feature(feature).unwrap().name.clone();
    let r = s
        .execute(
            "feature.rename",
            &json!({"feature": feature, "name": "  Base plate "}),
        )
        .unwrap();
    assert_eq!(r["name"], "Base plate");
    // The solid and its scene body take the feature's name.
    assert_eq!(s.scene.bodies[0].name, "Base plate");
    let body = s.scene.bodies[0].id;
    assert!(
        s.execute("body.rename", &json!({"body": body, "name": "x"}))
            .is_err(),
        "a solid is renamed through its feature"
    );
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.feature(feature).unwrap().name, before);
    s.execute("sketch.rename", &json!({"sketch": 1, "name": "Outline"}))
        .unwrap();
    assert_eq!(s.doc.sketch(1).unwrap().name, "Outline");
    for bad in [
        json!(""),
        json!("   "),
        json!("a\u{1}b"),
        json!("x".repeat(81)),
        json!(5),
    ] {
        assert!(
            s.execute("sketch.rename", &json!({"sketch": 1, "name": bad}))
                .is_err()
        );
    }
    assert_eq!(s.doc.sketch(1).unwrap().name, "Outline");
    let mesh = s.execute("body.sample", &json!({})).unwrap()["body"]
        .as_u64()
        .unwrap();
    s.execute("body.rename", &json!({"body": mesh, "name": "Bracket"}))
        .unwrap();
    let mesh = u32::try_from(mesh).unwrap();
    assert_eq!(s.scene.body(mesh).unwrap().name, "Bracket");
}

/// Two 10 mm cubes, A at x 0..10 and B at x 30..40 (both y 0..10, z 0..10), seen from
/// the top in an orthographic 800 x 600 view centred on (20, 5) with a half height of
/// 30 mm. Then 1 mm is 300 / 30 = 10 px, and model (x, y) is pixel
/// (400 + 10 (x - 20), 300 - 10 (y - 5)): A covers pixels x 200..300, B x 500..600, both
/// y 250..350. (The top view's pitch is 1e-5 rad short of straight down, which moves a
/// corner 10 mm up by 1e-4 mm: far below a pixel.)
fn two_cubes_from_the_top() -> (Session, u32, u32) {
    use chamfera_geom::{Point3, TriMesh};
    let mut s = Session::new();
    let a = s
        .scene
        .add(
            "A",
            TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0)),
        )
        .unwrap();
    let b = s
        .scene
        .add(
            "B",
            TriMesh::cuboid(Point3::new(30.0, 0.0, 0.0), Point3::new(40.0, 10.0, 10.0)),
        )
        .unwrap();
    top_view_centred_on(&mut s, 20.0, 5.0);
    (s, a, b)
}

/// Orthographic top view centred on model (x, y), 30 mm half height: 10 px per mm in an
/// 800 x 600 viewport.
fn top_view_centred_on(s: &mut Session, x: f64, y: f64) {
    s.execute("view.standard", &json!({"view": "top"})).unwrap();
    s.camera.projection = chamfera_render::Projection::Orthographic;
    s.camera.target = chamfera_geom::Point3::new(x, y, 0.0);
    s.camera.distance = 30.0 / (s.camera.fov_y * 0.5).tan();
}

fn boxed(s: &mut Session, corners: [f64; 4], mode: &str) -> Vec<u64> {
    let r = s
        .execute(
            "select.box",
            &json!({"x1": corners[0], "y1": corners[1], "x2": corners[2], "y2": corners[3],
                    "width": 800, "height": 600, "mode": mode}),
        )
        .unwrap();
    let mut ids: Vec<u64> = r["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    ids.sort();
    ids
}

#[test]
fn a_window_selects_only_bodies_fully_inside_it() {
    let (mut s, a, b) = two_cubes_from_the_top();
    let (a, b) = (u64::from(a), u64::from(b));
    // Around A only (pixels 190..310 x 240..360).
    assert_eq!(
        boxed(&mut s, [190.0, 240.0, 310.0, 360.0], "window"),
        vec![a]
    );
    // All of A and half of B (x 190..550): B is not fully inside.
    assert_eq!(
        boxed(&mut s, [190.0, 240.0, 550.0, 360.0], "window"),
        vec![a]
    );
    // Both (x 190..610).
    assert_eq!(
        boxed(&mut s, [190.0, 240.0, 610.0, 360.0], "window"),
        vec![a, b]
    );
    // Inside A's outline (x 210..290): A pokes out of it, so nothing.
    assert!(boxed(&mut s, [210.0, 260.0, 290.0, 340.0], "window").is_empty());
}

#[test]
fn a_crossing_selects_bodies_it_touches_or_contains() {
    let (mut s, a, b) = two_cubes_from_the_top();
    let (a, b) = (u64::from(a), u64::from(b));
    // All of A and half of B: both are touched.
    assert_eq!(
        boxed(&mut s, [550.0, 360.0, 190.0, 240.0], "crossing"),
        vec![a, b]
    );
    // A thin strip across A's middle (x 250..260, past its top and bottom): no corner of A
    // is inside, but its faces are.
    assert_eq!(
        boxed(&mut s, [260.0, 200.0, 250.0, 400.0], "crossing"),
        vec![a]
    );
    // A small box wholly inside B's top face (x 540..560): no edge of B crosses it.
    assert_eq!(
        boxed(&mut s, [560.0, 290.0, 540.0, 310.0], "crossing"),
        vec![b]
    );
    // The gap between them (x 320..480) touches neither.
    assert!(boxed(&mut s, [480.0, 240.0, 320.0, 360.0], "crossing").is_empty());
}

#[test]
fn box_selection_adds_and_removes_and_hidden_bodies_are_left_out() {
    let (mut s, a, b) = two_cubes_from_the_top();
    let around_a = json!({"x1": 190, "y1": 240, "x2": 310, "y2": 360, "width": 800, "height": 600, "mode": "window"});
    let around_b = json!({"x1": 490, "y1": 240, "x2": 610, "y2": 360, "width": 800, "height": 600, "mode": "window", "add": true});
    s.execute("select.box", &around_a).unwrap();
    let r = s.execute("select.box", &around_b).unwrap();
    assert_eq!(r["count"], 2);
    assert_eq!(r["bodies"], json!([a, b]));
    // Removing with a box around A leaves B.
    let mut remove_a = around_a.clone();
    remove_a["remove"] = json!(true);
    let r = s.execute("select.box", &remove_a).unwrap();
    assert_eq!(r["bodies"], json!([b]));
    // Hiding a selected body deselects it, and a hidden body is never boxed.
    s.execute("body.visibility", &json!({"body": b, "visible": false}))
        .unwrap();
    assert_eq!(s.execute("select.get", &json!({})).unwrap()["count"], 0);
    let all = json!({"x1": 0, "y1": 0, "x2": 800, "y2": 600, "width": 800, "height": 600, "mode": "crossing"});
    assert_eq!(s.execute("select.box", &all).unwrap()["bodies"], json!([a]));
    // A corner outside the viewport, or both add and remove, is refused.
    let mut outside = all.clone();
    outside["x2"] = json!(801);
    assert!(s.execute("select.box", &outside).is_err());
    let mut both = all.clone();
    both["add"] = json!(true);
    both["remove"] = json!(true);
    assert!(s.execute("select.box", &both).is_err());
    assert_eq!(
        s.execute("select.clear", &json!({})).unwrap()["bodies"],
        json!([])
    );
}

#[test]
fn toggle_picks_build_a_set_and_a_plain_miss_clears_it() {
    let (mut s, a, b) = two_cubes_from_the_top();
    // The centres of A (5, 5) and B (35, 5) are pixels (250, 300) and (550, 300).
    let at =
        |x: f64, mode: &str| json!({"x": x, "y": 300, "width": 800, "height": 600, "mode": mode});
    s.execute("select.pick", &at(250.0, "replace")).unwrap();
    let r = s.execute("select.pick", &at(550.0, "toggle")).unwrap();
    assert_eq!(r["bodies"], json!([a, b]));
    assert_eq!(r["body"], b, "the last pick is B");
    // Toggling A off keeps B.
    let r = s.execute("select.pick", &at(250.0, "toggle")).unwrap();
    assert_eq!(r["bodies"], json!([b]));
    // A modified click on empty space (the gap at x = 20 mm, pixel 400) changes nothing.
    let r = s.execute("select.pick", &at(400.0, "toggle")).unwrap();
    assert_eq!(r["bodies"], json!([b]));
    // Add A, then remove B.
    s.execute("select.pick", &at(250.0, "add")).unwrap();
    let r = s.execute("select.pick", &at(550.0, "remove")).unwrap();
    assert_eq!(r["bodies"], json!([a]));
    // A plain click on empty space clears.
    let r = s.execute("select.pick", &at(400.0, "replace")).unwrap();
    assert_eq!(r["count"], 0);
    assert!(r["body"].is_null());
}

#[test]
fn a_box_in_an_active_sketch_selects_its_points_and_curves() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    // Line A from (10, 10) to (20, 10), line B from (40, 10) to (50, 10).
    let la = s
        .execute(
            "sketch.line",
            &json!({"x1": 10, "y1": 10, "x2": 20, "y2": 10}),
        )
        .unwrap();
    let lb = s
        .execute(
            "sketch.line",
            &json!({"x1": 40, "y1": 10, "x2": 50, "y2": 10}),
        )
        .unwrap();
    // Centred on (30, 10): model (x, y) is pixel (400 + 10 (x - 30), 300 - 10 (y - 10)).
    // A runs over pixels x 200..300, B x 500..600, both at y = 300.
    top_view_centred_on(&mut s, 30.0, 10.0);
    let window = json!({"x1": 190, "y1": 290, "x2": 310, "y2": 310, "width": 800, "height": 600, "mode": "window"});
    let r = s.execute("select.box", &window).unwrap();
    let expected = json!([
        {"point": la["start"]}, {"point": la["end"]}, {"curve": la["curve"]}
    ]);
    assert_eq!(r["selection"], expected, "{r}");
    // A crossing over B's middle (x 530..570, neither end inside) adds only curve B.
    let crossing = json!({"x1": 570, "y1": 290, "x2": 530, "y2": 310, "width": 800, "height": 600, "mode": "crossing", "add": true});
    let r = s.execute("select.box", &crossing).unwrap();
    assert_eq!(r["items"], json!([{"curve": lb["curve"]}]), "{r}");
    assert_eq!(r["selection"].as_array().unwrap().len(), 4);
    // The same box as a window selects nothing: B is not inside it.
    let mut as_window = crossing.clone();
    as_window["mode"] = json!("window");
    as_window["add"] = json!(false);
    let r = s.execute("select.box", &as_window).unwrap();
    assert_eq!(r["selection"], json!([]));
}

#[test]
fn picked_edges_follow_the_pick_modes() {
    let mut s = Session::new();
    let screen = plate_from_the_top(&mut s);
    // 3 px inside the middles of the top front (y = 0) and top back (y = 30) edges.
    let (fx, fy) = screen([25.0, 0.0, 10.0]);
    let (bx, by) = screen([25.0, 30.0, 10.0]);
    let front = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    let back = edge_between(&mut s, [0.0, 30.0, 10.0], [50.0, 30.0, 10.0]);
    let mut pick = |x: f64, y: f64, mode: &str| {
        s.execute(
            "select.pick",
            &json!({"x": x, "y": y, "width": 800, "height": 600, "mode": mode,
                    "filter": ["edges"]}),
        )
        .unwrap()["edges"]
            .clone()
    };
    assert_eq!(pick(fx, fy - 3.0, "replace"), json!([front]));
    assert_eq!(pick(bx, by + 3.0, "add"), json!([front, back]));
    assert_eq!(pick(fx, fy - 3.0, "toggle"), json!([back]));
    assert_eq!(pick(bx, by + 3.0, "remove"), json!([]));
}

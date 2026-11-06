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
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "cut"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "smash"}),
        json!({"feature": 2, "count": 1e9, "direction": {"x": 0, "y": 0, "z": 0}, "spacing": 0}),
        json!({"feature": 2, "count": "3", "origin": {"x": f64::NAN, "y": 0}, "axis": "z", "angle": 9999}),
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

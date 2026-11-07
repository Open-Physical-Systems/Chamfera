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
        json!({"name": "w", "new_name": long, "comment": long, "favourite": "yes", "config": long, "from": 5}),
        json!({"config": "", "parameter": "w", "expression": "if(", "feature": 0, "suppressed": 1}),
        json!({"dir": "/dev/null", "format": "stl", "config": null, "name": "\u{0}"}),
        json!({"dir": "/nonexistent/dir/x", "format": [1], "name": "Extrude1.distance", "expression": "Extrude1.distance"}),
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
        json!({"feature": 2, "face": "top#2", "point": {"x": 1e300, "y": 0, "z": 0}}),
        json!({"feature": 2, "face": "top#0", "point": {"x": "a", "y": [], "z": null}}),
        json!({"feature": 2, "face": "top#99999999999", "point": [1, 2, 3], "edges": ["2:top 0#7"]}),
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
        json!({"points": [{"x": 1e300, "y": 0}], "characteristic": 5, "datums": [null], "label": "ÄÄ",
               "flip": 2, "cx": f64::NAN, "radius": -1, "text": null, "tolerance": "big",
               "leader_x": [], "annotation": -3, "view": 2, "x1": 0, "y1": 0, "x2": 1e300, "y2": 0}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 10, "y": 0}], "text": "\u{202e}x", "characteristic": "position",
               "tolerance": 1e300, "datums": ["A", "A"], "diameter": "yes", "removal": 1, "height": 0}),
        json!({"x1": 0, "y1": 0, "x2": 1, "y2": 1, "x3": 2, "y3": 2, "cx": 0, "cy": 0}),
        json!({"dx": 1e300, "dy": "a", "copy": "yes", "factor": -1, "angle": f64::NAN, "cx": 0, "cy": 0}),
        json!({"dx": 5, "dy": 0, "copy": true, "factor": 1e300, "angle": 1e300, "cx": 1e300, "cy": 0, "curves": [1, 2]}),
        json!({"direction": "sideways", "arc_length": 3, "driven": "no", "x": 1, "y": null}),
        json!({"direction": "horizontal", "arc_length": true, "driven": true, "x": 1e300, "y": 0, "constraint": 1}),
        json!({"curves": [1, 1, 1, 1], "x": 0, "y": 0, "radius": 1e300}),
        json!({"curves": [1, 2], "x": f64::NAN, "y": 0, "radius": -1}),
        json!({"point": 3, "x": 50, "y": 0, "distance2": 0, "curve": null}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "cut"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "operation": "smash"}),
        json!({"feature": 2, "count": 1e9, "direction": {"x": 0, "y": 0, "z": 0}, "spacing": 0}),
        json!({"feature": 2, "count": "3", "origin": {"x": f64::NAN, "y": 0}, "axis": "z", "angle": 9999}),
        json!({"constraints": [{"kind": "horizontal", "line": 1}, {"kind": "fix", "point": 1, "at": {"x": 1e300, "y": 0}}]}),
        json!({"constraints": [{"kind": "teleport"}], "constraint": {"kind": "horizontal", "line": 1}}),
        json!({"constraints": "horizontal"}),
        json!({"constraints": vec![nested.clone(); 65]}),
        json!({"base": "xy", "distance": "1e7", "axis": "w", "angle": f64::NAN, "planes": ["xy", 0], "points": ["origin"]}),
        json!({"base": {"feature": 2, "face": ""}, "face": {"feature": 1, "face": long}, "edge": {"faces": [1, 2]}, "faces": [], "point": {"vertex": "x"}}),
        json!({"base": 99, "axis": {"faces": [{"feature": 2, "face": "top"}, {"feature": 2, "face": "top"}]}, "angle": "360 deg", "planes": [1, 1], "points": [{"x": 1e300, "y": 0, "z": 0}, 3, {"centre": []}], "x": "a", "y": [], "z": null, "plane": 1}),
        json!({"base": "yz", "distance": -5, "axis": 1, "axis_feature": 1, "planes": ["xz", "yz"], "points": [{"vertex": [{"feature": 2, "face": "top"}]}, "origin"], "x": 0, "y": 0, "z": "1e300", "plane": 4294967296u64}),
        json!({"edges": ["1:top 0", 5, null], "radius": "2 mm", "distance": 1e308}),
        json!({"edges": [long], "radius": -1, "distance": "0 mm", "size": "q"}),
        json!({"edges": [], "pick": {"x": 1e300, "y": "a"}, "radius": 1}),
        json!({"pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "radius": 1e9}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 10, "y": 5}, {"x": 20, "y": 0}], "kind": "fit"}),
        json!({"points": [{"point": 1}, {"point": 1}, {"x": f64::NAN, "y": 0}], "closed": true}),
        json!({"points": [{"x": 1e300}, nested, {"point": -1}], "kind": "control", "closed": 1}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 0, "y": 0}, {"x": 0, "y": 0}], "kind": "control"}),
        json!({"component": 0, "name": "Part", "from_bodies": [1, 2, 99999], "parent": 0}),
        json!({"component": 4294967295u64, "from_bodies": "all", "parent": -1, "grounded": "no", "tint": 1}),
        json!({"component": 1, "transform": {"translate": {"x": 1e300, "y": 0, "z": 0}}}),
        json!({"units": "furlong", "scale": -1, "data": "AAAA", "format": "3mf"}),
        json!({"units": 25.4, "scale": "2", "data": "UEsFBgAAAAAAAAAAAAAAAAAAAAAAAA==", "format": "glb"}),
        json!({"units": "inch", "scale": 1e300, "data": "MApTRUNUSU9OCg==", "format": "obj", "path": "/dev/zero"}),
        json!({"component": 1, "transform": {"rotate": {"axis": {"x": 0, "y": 0, "z": 0}, "angle": 1e308}, "absolute": "yes"}}),
        json!({"component": 1, "transform": {"rotate": {"axis": {"x": 0, "y": 0, "z": 1}, "angle": "((("}}, "name": "\u{7}"}),
        json!({"type": "revolute", "a": {"component": 0, "point": {"x": 0, "y": 0, "z": 0}, "axis": {"x": 0, "y": 0, "z": 1}}, "b": {"component": 1, "face": "2:top"}, "value": 1e300}),
        json!({"type": "teleport", "a": {"pick": {"x": 400, "y": 300, "width": 800, "height": 600}}, "b": {"edge": long, "end": "middle"}}),
        json!({"type": "slider", "a": {"component": 0, "face": ""}, "b": {"component": 0, "face": "1:top"}, "min": 5, "max": -5}),
        json!({"type": "planar", "a": {"component": 0, "edge": "1:top 0", "end": "start"}, "b": {"pick": {"x": 1e300}}, "offset": {"x": 1e300}, "flip": 3}),
        json!({"joint": 1, "value": "90 mm", "distance": 1e300, "offset": "none"}),
        json!({"joint": 4294967295u64, "value": [1]}),
        json!({"joint": 1, "time": 1e300, "t": -1, "value": "((("}),
        json!({"joint": 1, "time": 0.5, "t": 1e300, "value": 1e300}),
        json!({"id": -1, "note": long, "a": "x", "b": 1e300, "show": 3, "clear": "yes", "design_path": 5}),
        json!({"id": 4294967296u64, "note": "\u{0}", "a": 0, "b": "current", "show": true, "clear": false, "design_path": "/dev/zero"}),
        json!({"id": 1, "note": [1], "a": [1], "b": {"id": 1}, "path": "/dev/null", "design_path": long}),
        json!({"joint": 1, "from": {"x": 1e300, "y": 0}, "to": [1], "width": 800, "height": 600, "start": "q", "cancel": 3}),
        json!({"joint": 1, "from": {"x": 400, "y": 300}, "to": {"x": 401, "y": 300}, "width": 800, "height": 600, "start": 1e300}),
        json!({"runs": 1e300, "cached": "yes"}),
        json!({"runs": -2, "cached": [true]}),
        json!({"runs": 2, "cached": true}),
        json!({"t": "now", "factor": 1e300, "component": 0, "offset": {"x": 1e300}, "at_keys": "yes", "components": [-1], "bodies": [5], "clear": "no"}),
        json!({"components": "all", "bodies": vec!["x"; 1000], "at_keys": true, "factor": 0.5}),
        json!({"components": [0, 4294967296u64], "bodies": [long], "factor": 2, "component": 99, "offset": {"x": 0, "y": 0, "z": 0}}),
        json!({"item": 1, "to": 1e300, "position": -1, "items": [1, "a"], "group": 0}),
        json!({"item": 2, "to": 0, "position": 0.5, "items": [], "name": "\u{7}"}),
        json!({"item": 1, "to": 1, "position": 1e7, "items": [1, 1, 1], "group": 99, "name": long}),
        json!({"item": [1], "to": "end", "position": "0", "items": 3, "sketch": 1, "name": 5}),
        json!({"view": "front", "x": 1e300, "y": -1e300, "scale": 0, "body": long, "sheet": 0}),
        json!({"view": "iso", "x": 100, "y": 100, "scale": 1e-300, "sheet": 4294967295u64}),
        json!({"view": 1, "kind": "angular", "x1": 1e300, "y1": 0, "x2": 0, "y2": 0, "offset": 1e300}),
        json!({"view": 2, "kind": "diameter", "x1": 0, "y1": 0, "format": "exe", "path": "/dev/null"}),
        json!({"paper": "a0", "orientation": 5, "projection": "second", "part_name": long, "date": "\u{1}"}),
        json!({"sheet": 1, "dimension": -1, "view": [1], "format": "pdf", "kind": "radius"}),
        json!({"view": 2, "axis": "z", "x0": 1e300, "y0": f64::NAN, "points": [{"x": "a"}], "spacing": 0,
               "kind": "ordinate_x", "feature": -1, "drawing_number": long, "revision": "\u{7}"}),
        json!({"view": 2, "axis": "x", "x0": 0, "y0": 0, "points": vec![json!({"x": 0, "y": 0}); 70],
               "spacing": 1e300, "kind": "horizontal", "feature": 4294967295u64, "material": 5,
               "checked_by": [], "x1": 0, "y1": 0, "x": 1e300, "y": 0}),
        json!({"face": "2:top", "points": [{"x": 1e300, "y": 0}], "diameter": 1e300, "through_all": true}),
        json!({"face": "2:top", "points": [{"x": 0, "y": 0}], "diameter": "6 mm", "depth": f64::MAX, "tip_angle": 1e-300}),
        json!({"face": ":", "points": [{"x": 0, "y": 0, "width": 0, "height": -1}], "diameter": 6, "depth": 4}),
        json!({"face": long, "points": "everywhere", "kind": long, "diameter": [6], "through_all": 1}),
        json!({"face": 7, "sketch": 0, "kind": "counterbore", "diameter": 6, "cb_diameter": -1, "cb_depth": 1e308}),
        json!({"pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "points": [{"x": 400, "y": 300, "width": 800, "height": 600}], "diameter": 6, "kind": "countersink", "cs_diameter": 12, "cs_angle": 1e9, "through_all": true}),
        json!({"feature": 2, "kind": "countersink", "points": [], "cs_angle": "flat", "tip_angle": "flat", "depth": "q"}),
        json!({"bodies": ["Extrude1", 5, null], "plane": "diagonal", "operation": "join"}),
        json!({"bodies": [long], "features": [-1, 4294967296u64], "plane": {"feature": 2, "face": long}}),
        json!({"bodies": "all", "features": "all", "plane": {"origin": {"x": f64::NAN}, "normal": 0}}),
        json!({"bodies": [], "plane": {"origin": {"x": 0, "y": 0, "z": 0}, "normal": {"x": 0, "y": 0, "z": 0}}}),
        json!({"plane": "yz", "feature": 2, "dx": 1e300, "dy": "q", "dz": [1], "copy": "yes"}),
        json!({"feature": 2, "angle": 1e300, "axis": {"x": 0, "y": 0, "z": 0}, "origin": "here"}),
        json!({"feature": 2, "from": {"x": 1e300, "y": 0, "z": 0}, "to": null, "copy": true}),
        json!({"bodies": ["Sample bracket"], "dx": "1e7 mm", "angle": "5 mm", "plane": 3}),
        json!({"body": "Extrude 1", "density": -1, "a": {"solid": 5}, "b": {"point": [1, 2]}}),
        json!({"body": 1e300, "density": 1e300, "a": {"point": [f64::MAX, 0, 0]}, "b": null}),
        json!({"body": [1], "a": {"solid": "Extrude 1", "face": long}, "b": {"solid": "x", "edge": 3}}),
        json!({"a": {"solid": "Extrude 1", "vertex": [1, "a", 3]}, "b": {"body": -4, "face": "#99999999999"}}),
        json!({"a": "solid", "x": 400, "y": 300, "width": 800, "height": 600, "from": {"point": "here"}}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "from": {"solid": "Extrude 1", "edge": "#-1"}}),
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
        json!({"faces": ["2:top"], "distance": 1e308, "angle": f64::NAN, "direction": {"x": 0, "y": 0, "z": 0}}),
        json!({"faces": [long, 5], "distance": "q", "origin": "here", "axis": [1]}),
        json!({"faces": ["2:top"], "distance": 0, "angle": "500 deg", "origin": {"x": 0, "y": 0, "z": 0}, "axis": {"x": 0, "y": 0, "z": 0}}),
        json!({"face": "2:top", "distance": -1e9, "preview": true}),
        json!({"face": long, "edge": "2:top 0", "distance": 1}),
        json!({"edge": "2:top 0", "distance": "-2 mm", "preview": "yes", "clear": 1}),
        json!({"sketch": 1, "profile": {"x": 1e300, "y": 0}, "distance": 5, "operation": "smash"}),
        json!({"sketch": 1, "profile": [1], "distance": f64::NAN}),
        json!({"pick": {"x": 400, "y": 300, "width": 800, "height": 600}, "distance": 3, "preview": true}),
        json!({"face": "2:top", "distance": 2, "clear": true}),
        json!({"x1": 0, "y1": 0, "x2": 800, "y2": 600, "width": 800, "height": 600, "mode": "crossing", "add": true, "remove": true}),
        json!({"x1": 900, "y1": -1, "x2": f64::NAN, "y2": 1e308, "width": 800, "height": 600, "mode": "window"}),
        json!({"x1": 0, "y1": 0, "x2": 0, "y2": 0, "width": 1, "height": 1, "mode": "crossing", "add": "yes"}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "mode": "toggle"}),
        json!({"x": 400, "y": 300, "width": 800, "height": 600, "mode": "flip"}),
        json!({"length": "in", "precision": 3, "angle_precision": 1, "spacing": 5}),
        json!({"length": "furlong", "precision": 9, "angle_precision": -1, "spacing": 0}),
        json!({"length": 25.4, "precision": 2.5, "angle_precision": "2", "spacing": 1e300}),
        json!({"length": "", "precision": 1e300, "spacing": "auto"}),
        json!({"yaw": 0.1, "pitch": 0.1, "pivot": {"x": 1e300, "y": 0, "z": 0}}),
        json!({"yaw": 0.1, "pitch": 0.1, "pivot": {"x": "a", "y": null}}),
        json!({"yaw": 0.1, "pitch": 0.1, "pivot": [1, 2, 3]}),
        json!({"yaw": 0.1, "pitch": 5.0, "pivot": {"x": 1e6, "y": -1e6, "z": 1e6}}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "taper": 1e308,
            "start_offset": -1e308, "distance2": f64::NAN, "direction": "two_sides"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "extent": "through_all",
            "taper": "-88 deg", "direction": "symmetric", "feature": 2}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "extent": "to_object",
            "to": {"feature": 99, "face": long}, "feature": 2}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "extent": 7,
            "to": {"plane": {"origin": {"x": 1e300, "y": 0, "z": 0}, "normal": "up"}}}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5,
            "to": {"pick": {"x": 1e300, "y": -1, "width": 0, "height": 0}}, "clear": "yes"}),
        json!({"sketch": 1, "profiles": [{"x": 10, "y": 10}], "distance": 5, "to": [1],
            "taper": [5], "start_offset": {"x": 1}, "distance2": true, "clear": true}),
        json!({"style": 5, "plane": "2:top", "offset": 1e308, "flip": "yes", "body": 1}),
        json!({"style": long, "plane": long, "offset": f64::NAN, "flip": 1, "colour": long}),
        json!({"style": "wireframe", "plane": "1:top", "offset": -1e7, "flip": true}),
        json!({"body": 1, "colour": "#zzzzzz", "opacity": 2, "preset": long}),
        json!({"body": 1, "colour": "#ff8000", "opacity": 0.3, "preset": "glass"}),
        json!({"body": 1, "colour": 5, "opacity": "half", "preset": ["glass"]}),
        json!({"body": 1, "colour": "#\u{e9}\u{e9}\u{e9}", "opacity": 1e-300, "preset": "default"}),
        json!({"cx": 0, "cy": 0, "x": 1e300, "y": 0, "sides": 1e9, "mode": 5, "minor": f64::NAN, "width": -1, "overall": "no"}),
        json!({"cx": 0, "cy": 0, "x": 5, "y": 0, "sides": 64, "mode": "circumscribed", "minor": 1e-300, "width": 1e300, "overall": true, "x1": 0, "y1": 0, "x2": 1e-9, "y2": 0}),
        json!({"cx": 1, "cy": 1, "x": 1, "y": 1, "sides": 3, "minor": 3, "x1": 1, "y1": 1, "x2": 1, "y2": 1, "width": 0}),
        json!({"edges": [long], "faces": [{"feature": 4294967296u64, "face": long}], "linked": 1}),
        json!({"faces": "top", "pick": {"x": 1e300}, "edges": [null], "linked": null}),
        json!({"faces": [{"feature": 2, "face": "top"}], "edges": ["2:top 0"], "pick": {"x": 400, "y": 300, "width": 800, "height": 600}}),
        json!({"sketch": 1, "profile": {"x": 10, "y": 10}, "rule": long, "reversed": "yes"}),
        json!({"sketch": 1, "profile": {"x": 1e300, "y": 10}, "rule": 5, "base": 99}),
        json!({"edge": "2:top 1", "height": 1e308, "angle": 1e300, "position": "up", "side": 5, "offset_start": -1}),
        json!({"edge": long, "height": "q", "angle": "180 deg", "position": "inside", "side": "opposite", "offset_end": 1e308}),
        json!({"name": "R", "thickness": 1e308, "bend_radius": -1, "k_factor": f64::NAN, "relief": "star", "remove": "no"}),
        json!({"name": "", "relief_width": 0, "relief_depth": -1, "relief": 3, "remove": true}),
        json!({"sketch": 1, "curves": [1, 4294967296u64, -1], "distance": 1e308, "symmetric": "yes"}),
        json!({"sketch": 1, "curves": "all", "profiles": [{"x": 1e300, "y": 0}], "axis": -1, "angle": 1e300}),
        json!({"sketch": 1, "curves": [1], "path_sketch": 1, "path_curves": [1e300]}),
        json!({"edges": [long], "sketch": 1, "profiles": [{"x": 25, "y": 15}]}),
        json!({"edges": "2:top 0", "bodies": [5], "body": 7, "tool": [], "keep": "sideways"}),
        json!({"body": long, "plane": {"feature": -1, "face": long}, "tool": long, "keep": 1}),
        json!({"bodies": [long, long], "body": "", "plane": "xy", "keep": "above"}),
        json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": 1, "path_curves": [1, 2, 99999]}),
        json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": -1, "path_curves": [-1, 1e300]}),
        json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": 1, "path_curves": "all"}),
        json!({"sections": [{"sketch": 1, "x": 0, "y": 0}, {"sketch": 1, "x": 1e300, "y": 0}]}),
        json!({"sections": [{"sketch": 1, "x": 5, "y": 5}, {"sketch": 99, "x": 0}], "feature": 1}),
        json!({"sections": "two", "path_curves": null, "operation": "join", "plane": "xy", "offset": 1e300}),
        json!({"plane": "xz", "offset": f64::NAN, "feature": 1, "path_curves": [1]}),
        // Body tools: combine, split, thicken, remove.
        json!({"target": "Sample bracket", "tools": ["Sample bracket", 5], "operation": "cut", "keep_tools": "yes"}),
        json!({"target": long, "tools": [long], "operation": "new_body", "keep_tools": true, "feature": 2}),
        json!({"target": 5, "tools": "all", "bodies": [null], "sides": "up", "line": -1, "sketch": "x"}),
        json!({"bodies": ["Sample bracket"], "plane": 4294967296u64, "faces": ["1:top"], "sides": 5}),
        json!({"bodies": ["Extrude1"], "plane": {"feature": 99, "face": long}, "line": 1, "sketch": 1}),
        json!({"faces": ["2:top"], "plane": "yz", "line": 1, "sketch": 1, "thickness": 1e300, "sides": "both_sides"}),
        json!({"faces": [long, ""], "thickness": "3 mm", "sides": "symmetric", "operation": "smash"}),
        json!({"faces": ["2:side 0"], "thickness": -1e-300, "operation": "join", "feature": 2, "tools": []}),
        json!({"bodies": vec!["x"; 1001], "faces": vec!["y"; 1001], "tools": vec!["z"; 1001], "target": ""}),
        // Renders and materials.
        json!({"width": 4097, "height": 1, "samples": 1, "path": "/nonexistent/dir/x.png"}),
        json!({"width": 2, "height": 2, "samples": 1e9, "time_limit": -1, "preset": long}),
        json!({"width": 2.5, "height": f64::NAN, "samples": "many", "time_limit": f64::NAN}),
        json!({"width": 2, "height": 2, "samples": 1, "time_limit": 1e300, "path": "/nonexistent/dir/x.png"}),
        json!({"width": 2, "height": 2, "samples": 1, "path": "/dev/null", "preset": "dark"}),
        json!({"body": 1, "metallic": 2, "roughness": f64::NAN, "transmission": -1, "ior": 0}),
        json!({"body": 1, "metallic": "1", "roughness": [0], "transmission": null, "ior": 1e300}),
        json!({"body": 1, "metallic": 1, "roughness": 0, "transmission": 1, "ior": 3, "preset": "chrome"}),
        // Tools with holders, the holder check, pencil and scallop, tool libraries.
        json!({"kind": "flat_end_mill", "diameter": 6, "holder": {"stickout": 1e300, "segments": [{"diameter": -1, "length": f64::MAX}]}}),
        json!({"kind": "ball_end_mill", "diameter": 6, "holder": {"stickout": 2, "shank_diameter": "1e9 mm", "segments": vec![json!({"diameter": 1, "length": 1}); 33]}}),
        json!({"kind": "bull_nose", "diameter": 6, "corner_radius": 1, "holder": {"segments": "x"}, "tool": 1}),
        json!({"tool": 1, "holder": [1], "kind": "pencil", "min_angle": 1e300, "max_slope": -5, "holder_check": "maybe"}),
        json!({"tool": 1, "kind": "scallop", "stepover": 1e-300, "max_slope": "89 deg", "holder_check": 5}),
        json!({"tool": 4294967296u64, "kind": "parallel", "holder_check": "refuse", "path": "/dev/zero", "tools": [1, 99]}),
        json!({"path": "/nonexistent/dir/tools.json", "replace": "yes", "tools": "all"}),
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

#[test]
fn an_imported_mesh_is_saved_undone_and_named_apart() {
    // A 10 mm cube (1000 mm³) imported twice: the second is `cube (2)`. Both are part of
    // the design: undo removes them one at a time, redo brings them back, they survive
    // save and open, and an unrelated rebuild keeps their scene ids.
    let dir = std::env::temp_dir().join(format!("chamfera-mesh-doc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cube.stl");
    let cube = chamfera_geom::TriMesh::cuboid(
        chamfera_geom::Point3::ORIGIN,
        chamfera_geom::Point3::new(10.0, 10.0, 10.0),
    );
    std::fs::write(&path, chamfera_io::stl::write_binary(&cube).unwrap()).unwrap();
    let mut s = Session::new();
    let import = |s: &mut Session| {
        s.execute("body.import_stl", &json!({"path": path.to_str().unwrap()}))
            .unwrap()
    };
    let first = import(&mut s);
    let second = import(&mut s);
    assert_eq!(first["name"], "cube");
    assert_eq!(second["name"], "cube (2)");
    assert_eq!(s.doc.meshes.len(), 2);
    let names =
        |s: &Session| -> Vec<String> { s.scene.bodies.iter().map(|b| b.name.clone()).collect() };
    assert_eq!(names(&s), ["cube", "cube (2)"]);
    // An unrelated change rebuilds: the meshes keep their scene bodies.
    s.execute("param.set", &json!({"name": "w", "expression": "5 mm"}))
        .unwrap();
    assert_eq!(s.scene.bodies[0].id, first["body"].as_u64().unwrap() as u32);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&s), ["cube"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.scene.bodies.is_empty() && s.doc.meshes.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(names(&s), ["cube", "cube (2)"]);
    // Saved and opened again, with the volume of the mesh.
    let data = s.execute("design.save_bytes", &json!({})).unwrap()["data"].clone();
    let mut t = Session::new();
    t.execute("design.open_bytes", &json!({ "data": data }))
        .unwrap();
    assert_eq!(names(&t), ["cube", "cube (2)"]);
    let listed = t.execute("body.list", &json!({})).unwrap();
    let v = listed["bodies"][1]["volume"].as_f64().unwrap();
    assert!((v - 1000.0).abs() < 1e-6, "{v}");
    // Renaming and removing are part of the design too, and undone.
    let id = listed["bodies"][1]["body"].clone();
    t.execute("body.rename", &json!({"body": id, "name": "lid"}))
        .unwrap();
    assert_eq!(names(&t), ["cube", "lid"]);
    t.execute("body.remove", &json!({"body": id})).unwrap();
    assert_eq!(names(&t), ["cube"]);
    assert_eq!(t.doc.meshes.len(), 1);
    t.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&t), ["cube", "lid"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_step_body_read_back_gets_a_name_of_its_own() {
    // The 50 x 30 x 10 box exported to STEP and read back: the copy is `Extrude1 (2)`, so
    // `inspect.mass {body: "Extrude1"}` still means the original (15000 mm³ each).
    let dir = std::env::temp_dir().join(format!("chamfera-step-name-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("box.step");
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    s.execute(
        "design.export_step",
        &json!({"path": path.to_str().unwrap()}),
    )
    .unwrap();
    let r = s
        .execute("body.import_step", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(r["solids"][0]["name"], "Extrude1 (2)", "{r}");
    let names: Vec<&str> = s.solids().iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["Extrude1", "Extrude1 (2)"]);
    for body in ["Extrude1", "Extrude1 (2)"] {
        let m = s
            .execute("inspect.mass", &json!({"body": body, "density": 1}))
            .unwrap();
        assert!(
            (m["volume"].as_f64().unwrap() - 15_000.0).abs() < 1e-6,
            "{m}"
        );
    }
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
fn constraints_in_one_call_skip_the_implied_and_the_conflicting_ones() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute("sketch.line", &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 0}))
        .unwrap();
    let line = r["curve"].clone();
    let start = r["start"].clone();
    let before = s.execute("sketch.get", &json!({})).unwrap()["dof"].clone();
    // Horizontal (adds), horizontal again (implied), vertical (conflicts: a 10 mm line
    // cannot be both), and a length of 10 (adds).
    let r = s
        .execute(
            "sketch.constrain",
            &json!({"constraints": [
                {"kind": "horizontal", "line": line},
                {"kind": "horizontal", "line": line},
                {"kind": "vertical", "line": line},
                {"kind": "length", "line": line, "value": 10},
                {"kind": "fix", "point": start, "at": {"x": 0, "y": 0}},
            ]}),
        )
        .unwrap();
    assert_eq!(r["constraints"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(r["skipped"], 2);
    // Four unknowns less horizontal, length and the two fix equations: none left.
    assert_eq!(before, 4);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["dof"], 0);
    assert_eq!(g["status"], "FullyConstrained");
    // One undo step takes all three away.
    s.execute("edit.undo", &json!({})).unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["constraints"].as_array().unwrap().len(), 0);
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

/// The arch: a fit spline through (0, 0), (10, h), (20, 0) closed by a line. Equal
/// chords give the parameters 0, ½, 1 and the quadratic y = 4 h t (1 - t), x = 20 t:
/// a parabola over a 20 mm base whose area is 2/3 · 20 · h (Archimedes).
fn spline_arch(s: &mut Session, h: f64) -> serde_json::Value {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute(
            "sketch.spline",
            &json!({"points": [{"x": 0, "y": 0}, {"x": 10, "y": h}, {"x": 20, "y": 0}]}),
        )
        .unwrap();
    let (a, b) = (r["points"][0].clone(), r["points"][2].clone());
    s.execute(
        "sketch.line",
        &json!({"x1": 20, "y1": 0, "x2": 0, "y2": 0, "start": b, "end": a}),
    )
    .unwrap();
    r
}

fn profile_areas(s: &mut Session) -> Vec<f64> {
    s.execute("sketch.get", &json!({})).unwrap()["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["area"].as_f64().unwrap())
        .collect()
}

#[test]
fn a_fit_spline_closed_by_a_line_extrudes_to_area_times_height() {
    let mut s = Session::new();
    let r = spline_arch(&mut s, 10.0);
    assert_eq!(r["kind"], "fit");
    let areas = profile_areas(&mut s);
    assert_eq!(areas.len(), 1);
    assert!((areas[0] - 400.0 / 3.0).abs() < 1e-9, "{areas:?}");
    // Dragging the middle point to (10, 20) reshapes the spline: 2/3 · 20 · 20 = 800/3.
    let mid = r["points"][1].clone();
    s.execute("sketch.drag", &json!({"point": mid, "x": 10, "y": 20}))
        .unwrap();
    let areas = profile_areas(&mut s);
    assert!((areas[0] - 800.0 / 3.0).abs() < 1e-6, "{areas:?}");
    s.execute("sketch.finish", &json!({})).unwrap();
    // Extruded by 6: V = 800/3 · 6 = 1600.
    let e = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 10, "y": 5}], "distance": 6}),
        )
        .unwrap();
    let v = e["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 1600.0).abs() < 1e-6, "{v}");
    // Undo takes the extrude back.
    s.execute("edit.undo", &json!({})).unwrap();
    let list = s.execute("feature.list", &json!({})).unwrap();
    assert!(list["features"].as_array().unwrap().is_empty(), "{list}");
}

#[test]
fn a_closed_control_spline_is_a_profile_of_its_own() {
    // The closed uniform cubic on the square (±10, ±10): integrating ½ (x y' - y x') over
    // its four spans of uniform cubic basis polynomials gives 2440/9 mm²; extruded by 9
    // that is 2440 mm³.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let r = s
        .execute(
            "sketch.spline",
            &json!({"points": [{"x": 10, "y": -10}, {"x": 10, "y": 10}, {"x": -10, "y": 10}, {"x": -10, "y": -10}],
                    "kind": "control", "closed": true}),
        )
        .unwrap();
    assert_eq!(r["closed"], true);
    let areas = profile_areas(&mut s);
    assert!((areas[0] - 2440.0 / 9.0).abs() < 1e-9, "{areas:?}");
    s.execute("sketch.finish", &json!({})).unwrap();
    let e = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 9}),
        )
        .unwrap();
    let v = e["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 2440.0).abs() < 1e-6, "{v}");
}

#[test]
fn bad_splines_are_refused_and_change_nothing() {
    let mut s = Session::new();
    // No sketch being edited.
    assert!(
        s.execute(
            "sketch.spline",
            &json!({"points": [{"x": 0, "y": 0}, {"x": 1, "y": 1}]})
        )
        .is_err()
    );
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    for p in [
        json!({}),
        json!({"points": [{"x": 0, "y": 0}]}),
        json!({"points": "many"}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 0, "y": 0}]}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 1e300, "y": 0}]}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 5}]}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 5, "y": 1, "z": 3}]}),
        json!({"points": [{"point": 999}, {"x": 5, "y": 1}]}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 5, "y": 1}], "kind": "bezier"}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 5, "y": 1}], "closed": true}),
        json!({"points": [{"x": 0, "y": 0}, {"x": 5, "y": 1}], "closed": "yes"}),
    ] {
        assert!(s.execute("sketch.spline", &p).is_err(), "{p}");
    }
    let sk = &s.doc.sketches[0].sketch;
    assert!(sk.is_empty(), "a failed spline leaves nothing behind");
    // An existing point can be reused.
    let a = s.execute("sketch.point", &json!({"x": 0, "y": 0})).unwrap()["point"].clone();
    let r = s
        .execute(
            "sketch.spline",
            &json!({"points": [{"point": a}, {"x": 5, "y": 5}, {"x": 10, "y": 0}]}),
        )
        .unwrap();
    assert_eq!(r["points"][0], a);
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
fn feature_edit_changes_a_patterns_count_and_spacing_and_refuses_unknown_keys() {
    // Plate 50 x 30 x 10; a hole r = 3 at (10, 15) cut through, patterned 2 along +X
    // 10 apart: 15000 - 2 * 90π. Edited to 4 copies 8 apart (x = 10, 18, 26, 34, all
    // inside the plate): 15000 - 4 * 90π.
    let mut s = Session::new();
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
        .unwrap()["feature"]
        .clone();
    let pat = s
        .execute(
            "solid.pattern_linear",
            &json!({"feature": cut, "count": 2, "direction": {"x": 1, "y": 0, "z": 0}, "spacing": 10}),
        )
        .unwrap()["feature"]
        .clone();
    let pi = std::f64::consts::PI;
    let vol = |s: &Session| s.solids()[0].body.volume().unwrap();
    assert!((vol(&s) - (15_000.0 - 180.0 * pi)).abs() < 1e-6);
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": pat, "count": 4, "spacing": "8 mm"}),
        )
        .unwrap();
    assert_eq!(r["definition"]["count"], "4", "{r}");
    assert!(
        (vol(&s) - (15_000.0 - 360.0 * pi)).abs() < 1e-6,
        "{}",
        vol(&s)
    );
    // An edit that names a key feature.edit does not know changes nothing and is refused,
    // instead of succeeding silently (which made a following undo remove the pattern).
    let before = s.doc.clone();
    let e = s
        .execute("feature.edit", &json!({"feature": pat, "cuont": 3}))
        .unwrap_err();
    assert!(e.to_string().contains("cuont"), "{e}");
    assert_eq!(s.doc, before);
    // count and spacing belong to patterns.
    assert!(
        s.execute("feature.edit", &json!({"feature": cut, "count": 3}))
            .is_err()
    );
    // Undo goes back to 2 copies.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(
        (vol(&s) - (15_000.0 - 180.0 * pi)).abs() < 1e-6,
        "{}",
        vol(&s)
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
fn a_fillet_rounds_the_strut_of_an_extruded_spline_and_refuses_its_spline_rim() {
    // The arch y(x) = x (20 - x) / 10 (h = 10) extruded by 6. The strut at (20, 0) runs
    // along z between the base y = 0 and the parabola, which descends there with slope
    // y'(20) = -2. Filleting it with r extrudes the 2D fillet: the ball centre is r above
    // the base and r below the parabola, at c = (x, y(x)) + r (y', -1)/√(1 + y'²) with
    // c_y = r (bisection on the foot x). The removed area is ½ ∮ (x dy - y dx) around the
    // base from T_l = (c_x, 0) to (20, 0), the parabola back to the foot (Simpson) and
    // the fillet arc to T_l (closed form).
    let mut s = Session::new();
    spline_arch(&mut s, 10.0);
    s.execute("sketch.finish", &json!({})).unwrap();
    let h = 6.0;
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 10, "y": 5}], "distance": h}),
    )
    .unwrap();
    let v0 = 400.0 / 3.0 * h;
    let y = |x: f64| x * (20.0 - x) / 10.0;
    let dy = |x: f64| (20.0 - 2.0 * x) / 10.0;
    let r = 1.5;
    let centre = |x: f64| {
        let n = (1.0 + dy(x) * dy(x)).sqrt();
        (x + r * dy(x) / n, y(x) - r / n)
    };
    let (mut lo, mut hi) = (10.0, 20.0);
    for _ in 0..200 {
        let m = 0.5 * (lo + hi);
        if centre(m).1 > r {
            lo = m;
        } else {
            hi = m;
        }
    }
    let xs = 0.5 * (lo + hi);
    let (cx, cy) = centre(xs);
    let mut twice = 0.0 * cx * cy;
    // Base T_l -> P: y = 0, no contribution. Parabola from x = 20 to xs: x y' - y.
    let n = 2000;
    let f = |x: f64| x * dy(x) - y(x);
    let step = (xs - 20.0) / n as f64;
    let mut simpson = f(20.0) + f(xs);
    for i in 1..n {
        let x = 20.0 + step * i as f64;
        simpson += f(x) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    twice += simpson * step / 3.0;
    // Arc from the foot to T_l about (cx, cy): ∫ r² + r (cx cos θ + cy sin θ) dθ.
    let ts = (y(xs) - cy).atan2(xs - cx);
    let tl = -std::f64::consts::FRAC_PI_2;
    let mut dt = tl - ts;
    while dt > std::f64::consts::PI {
        dt -= std::f64::consts::TAU;
    }
    while dt < -std::f64::consts::PI {
        dt += std::f64::consts::TAU;
    }
    let te = ts + dt;
    twice += r * r * dt + r * (cx * (te.sin() - ts.sin()) - cy * (te.cos() - ts.cos()));
    let area = 0.5 * twice.abs();
    assert!(area > 0.0 && area < r * r, "{area}");
    let strut = edge_between(&mut s, [20.0, 0.0, 0.0], [20.0, 0.0, h]);
    let res = s
        .execute("solid.fillet", &json!({"edges": [strut], "radius": r}))
        .unwrap();
    let v = exact_volume(&res);
    assert!(
        (v - (v0 - area * h)).abs() < 1e-6,
        "{v} vs {}",
        v0 - area * h
    );
    s.execute("edit.undo", &json!({})).unwrap();

    // The top rim has two edges from (0, 0, h) to (20, 0, h): the spline's (refused: its
    // blend is free-form) and the base's (refused: it would end on the curved spline side).
    let listed = s.execute("solid.edges", &json!({})).unwrap();
    let names: Vec<String> = listed["solids"][0]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| {
            let at = |v: &serde_json::Value, x: f64| {
                (v[0].as_f64().unwrap() - x).abs() < 1e-9
                    && v[1].as_f64().unwrap().abs() < 1e-9
                    && (v[2].as_f64().unwrap() - h).abs() < 1e-9
            };
            (at(&e["from"], 0.0) && at(&e["to"], 20.0))
                || (at(&e["from"], 20.0) && at(&e["to"], 0.0))
        })
        .map(|e| e["edge"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names.len(), 2, "{listed}");
    let before = s.doc.features.len();
    let mut nurbs = 0;
    for name in &names {
        let e = s
            .execute("solid.fillet", &json!({"edges": [name], "radius": 1}))
            .unwrap_err();
        nurbs += usize::from(e.to_string().contains("NURBS"));
        assert_eq!(s.doc.features.len(), before);
        assert!((s.solids()[0].body.volume().unwrap() - v0).abs() < 1e-6);
    }
    assert_eq!(nurbs, 1);
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

/// A 50 x 30 x 10 plate with a through hole of diameter 6 at (25, 15), by commands.
fn plate_with_hole(s: &mut Session) {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 3}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
    )
    .unwrap();
}

#[test]
fn a_drawing_of_the_plate_has_projected_views_dimensions_and_exports() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    let r = s
        .execute(
            "drawing.new_sheet",
            &json!({"paper": "a3", "part_name": "Plate", "drawn_by": "CI"}),
        )
        .unwrap();
    assert_eq!(
        (r["width"].as_f64(), r["height"].as_f64()),
        (Some(420.0), Some(297.0))
    );
    let front = s
        .execute(
            "drawing.add_view",
            &json!({"view": "front", "x": 120, "y": 150}),
        )
        .unwrap();
    let front_id = front["view"].as_u64().unwrap();
    let top = s
        .execute(
            "drawing.add_view",
            &json!({"view": "top", "x": 100, "y": 220}),
        )
        .unwrap();
    // The top view is projected from the front view (third angle: above it, aligned).
    assert_eq!(top["parent"].as_u64(), Some(front_id));
    s.execute(
        "drawing.add_view",
        &json!({"view": "right", "x": 200, "y": 150}),
    )
    .unwrap();
    s.execute(
        "drawing.add_view",
        &json!({"view": "iso", "x": 320, "y": 200}),
    )
    .unwrap();
    let list = s.execute("drawing.list", &json!({})).unwrap();
    let views = list["sheets"][0]["views"].as_array().unwrap().clone();
    assert_eq!(views.len(), 4);
    assert_eq!(
        views[1]["x"].as_f64(),
        Some(120.0),
        "aligned with the front view"
    );
    // Front view: the 50 x 10 outline at (95..145, 145..155), and the hole's two hidden
    // silhouettes.
    let b = views[0]["bounds"].as_array().unwrap();
    let b: Vec<f64> = b.iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(
        (b[2] - b[0] - 50.0).abs() < 1e-6 && (b[3] - b[1] - 10.0).abs() < 1e-6,
        "{b:?}"
    );
    assert_eq!(views[0]["hidden_curves"].as_u64(), Some(2));
    // A horizontal dimension across the front view's bottom corners: 50.
    let d = s
        .execute(
            "drawing.add_dimension",
            &json!({"view": front_id, "kind": "horizontal", "x1": 95.5, "y1": 145.3, "x2": 144.8, "y2": 145.0, "offset": -10}),
        )
        .unwrap();
    assert!((d["value"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(d["text"], "50");
    // The hole in the top view (centre at (120, 220)): Ø6.
    let top_id = top["view"].as_u64().unwrap();
    let d = s
        .execute(
            "drawing.add_dimension",
            &json!({"view": top_id, "kind": "diameter", "x1": 123.2, "y1": 220.4}),
        )
        .unwrap();
    assert_eq!(d["text"], "Ø6");
    assert!(
        s.execute(
            "drawing.add_dimension",
            &json!({"view": 4, "kind": "horizontal", "x1": 0, "y1": 0, "x2": 1, "y2": 1})
        )
        .is_err()
    );
    // Exports.
    let dir = std::env::temp_dir().join(format!("chamfera-drawing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for ext in ["svg", "dxf", "pdf"] {
        let path = dir.join(format!("plate.{ext}"));
        let r = s
            .execute("drawing.export", &json!({"path": path.to_str().unwrap()}))
            .unwrap();
        assert_eq!(r["format"], ext);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(r["bytes"].as_u64(), Some(bytes.len() as u64));
    }
    assert!(
        std::fs::read(dir.join("plate.pdf"))
            .unwrap()
            .starts_with(b"%PDF-1.4")
    );
    assert!(
        s.execute(
            "drawing.export",
            &json!({"path": dir.join("x.txt").to_str().unwrap()})
        )
        .is_err()
    );
    // The drawing is saved with the design and reopens the same.
    let path = dir.join("plate.chamfera");
    s.execute("design.save", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let saved = s.doc.drawing.clone();
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(t.doc.drawing, saved);
    assert_eq!(t.doc.version, chamfera_doc::FORMAT_VERSION);
    let _ = std::fs::remove_dir_all(&dir);
    // Deleting the front view takes the views projected from it and the dimensions.
    s.execute("drawing.delete", &json!({"view": front_id}))
        .unwrap();
    let sheet = &s.doc.drawing.sheets[0];
    assert_eq!(sheet.views.len(), 1, "only the iso view is left");
    assert!(sheet.dimensions.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.drawing.sheets[0].views.len(), 4);
}

#[test]
fn views_follow_the_model() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    s.execute(
        "drawing.add_view",
        &json!({"view": "front", "x": 150, "y": 150}),
    )
    .unwrap();
    let width = |s: &mut Session| {
        let l = s.execute("drawing.list", &json!({})).unwrap();
        let b: Vec<f64> = l["sheets"][0]["views"][0]["bounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        (b[2] - b[0], b[3] - b[1])
    };
    let (w, h) = width(&mut s);
    assert!((w - 50.0).abs() < 1e-6 && (h - 10.0).abs() < 1e-6);
    // Extruding 20 instead of 10 makes the front view 20 high.
    let f = s.doc.features[0].id;
    s.execute("feature.edit", &json!({"feature": f, "distance": 20}))
        .unwrap();
    let (w, h) = width(&mut s);
    assert!(
        (w - 50.0).abs() < 1e-6 && (h - 20.0).abs() < 1e-6,
        "{w} x {h}"
    );
}

#[test]
fn drawing_commands_with_a_solid_refuse_hostile_input() {
    let mut s = Session::new();
    plate_with_hole(&mut s);
    s.execute(
        "drawing.add_view",
        &json!({"view": "front", "x": 150, "y": 150}),
    )
    .unwrap();
    for p in [
        json!({"view": "front", "x": 1e5, "y": 0}),
        json!({"view": "top", "x": 0, "y": 0, "body": "Nope"}),
        json!({"view": "top", "x": 0, "y": 0, "scale": 1e9}),
        json!({"view": 2, "kind": "horizontal", "x1": 150, "y1": 150, "x2": 150, "y2": 150}),
        json!({"view": 2, "kind": "angular", "x1": 125, "y1": 145, "x2": 175, "y2": 145}),
        json!({"view": 2, "kind": "radius", "x1": 10, "y1": 10}),
        json!({"paper": "a4", "part_name": "x".repeat(65)}),
        json!({"sheet": 99}),
    ] {
        let before = s.doc.clone();
        for id in [
            "drawing.add_view",
            "drawing.add_dimension",
            "drawing.new_sheet",
            "drawing.edit_sheet",
            "drawing.delete",
        ] {
            if s.execute(id, &p).is_err() {
                assert_eq!(s.doc, before, "{id} {p}");
            }
            s.doc = before.clone();
        }
    }
}
/// A box from a rectangle `(x1, y1)..(x2, y2)` on the XY plane extruded by `h`, with the
/// given operation. Returns the extrude's reply.
fn box_feature(s: &mut Session, corners: [f64; 4], h: f64, operation: &str) -> serde_json::Value {
    let [x1, y1, x2, y2] = corners;
    let sketch = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": sketch, "profiles": [{"x": (x1 + x2) / 2.0, "y": (y1 + y2) / 2.0}],
                "distance": h, "operation": operation}),
    )
    .unwrap()
}

fn solid_box(s: &Session, name: &str) -> chamfera_geom::Aabb {
    s.solids()
        .iter()
        .find(|x| x.name == name)
        .unwrap_or_else(|| panic!("no solid {name}"))
        .body
        .bounding_box()
        .unwrap()
}

fn near_box(b: chamfera_geom::Aabb, min: [f64; 3], max: [f64; 3]) -> bool {
    let p = |q: chamfera_geom::Point3, w: [f64; 3]| {
        q.distance(chamfera_geom::Point3::new(w[0], w[1], w[2])) < 1e-6
    };
    p(b.min, min) && p(b.max, max)
}

fn has_vertex(b: &chamfera_kernel::Body, p: [f64; 3]) -> bool {
    let p = chamfera_geom::Point3::new(p[0], p[1], p[2]);
    b.vertices()
        .into_iter()
        .any(|v| b.vertex_point(v).is_ok_and(|q| q.distance(p) < 1e-6))
}

#[test]
fn the_mirrored_bracket_example_is_one_symmetric_body() {
    // examples/mirrored-bracket.json. Half: base 40 x 30 x 6 (7200), a wall x 32..40 30 tall
    // joined (8 * 30 * 30 = 7200, less its 8 * 30 * 6 = 1440 overlap with the base), a r = 4
    // hole through the base (16π * 6): 12960 - 96π. Mirrored in x = 0 and joined: twice
    // that, one body spanning x -40..40.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/mirrored-bracket.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    for step in steps.as_array().unwrap() {
        let params = step.get("params").cloned().unwrap_or(json!({}));
        s.execute(step["command"].as_str().unwrap(), &params)
            .unwrap();
    }
    assert_eq!(s.solids().len(), 1);
    let v = s.solids()[0].body.volume().unwrap();
    let want = 2.0 * (12_960.0 - 96.0 * std::f64::consts::PI);
    assert!((v - want).abs() < 1e-6, "{v} vs {want}");
    let b = s.solids()[0].body.bounding_box().unwrap();
    assert!(near_box(b, [-40.0, 0.0, 0.0], [40.0, 30.0, 30.0]));
    assert!(s.solids()[0].scene_id.is_some());
}

#[test]
fn mirroring_a_box_about_the_yz_plane_puts_it_on_the_other_side() {
    // Box x 0..50, y 0..30, z 0..10 (15000); its mirror in x = 0 is x -50..0, same volume.
    let mut s = Session::new();
    box_feature(&mut s, [0.0, 0.0, 50.0, 30.0], 10.0, "new_body");
    let r = s
        .execute(
            "solid.mirror",
            &json!({"bodies": ["Extrude1"], "plane": "yz"}),
        )
        .unwrap();
    assert_eq!(s.solids().len(), 2);
    assert!((exact_volume(&r) - 15_000.0).abs() < 1e-6);
    let mirror = solid_box(&s, "Mirror1");
    assert!(near_box(mirror, [-50.0, 0.0, 0.0], [0.0, 30.0, 10.0]));
    // The original is untouched and both are shown.
    let original = solid_box(&s, "Extrude1");
    assert!(near_box(original, [0.0, 0.0, 0.0], [50.0, 30.0, 10.0]));
    assert!(s.solids().iter().all(|x| x.scene_id.is_some()));
    // Edit the plane to XZ (y = 0): y 0..30 becomes y -30..0; undo restores the YZ mirror.
    let id = r["feature"].as_u64().unwrap();
    s.execute("feature.edit", &json!({"feature": id, "plane": "xz"}))
        .unwrap();
    let edited = solid_box(&s, "Mirror1");
    assert!(near_box(edited, [0.0, -30.0, 0.0], [50.0, 0.0, 10.0]));
    s.execute("edit.undo", &json!({})).unwrap();
    let undone = solid_box(&s, "Mirror1");
    assert!(near_box(undone, [-50.0, 0.0, 0.0], [0.0, 30.0, 10.0]));
    // A move's key on a mirror, or a missing body, is refused and adds nothing.
    let wrong_key = s.execute("feature.edit", &json!({"feature": id, "dx": 5}));
    assert!(wrong_key.is_err());
    let missing = s.execute("solid.mirror", &json!({"bodies": ["Nope"], "plane": "yz"}));
    assert!(missing.is_err());
    assert_eq!(s.doc.features.len(), 2);
}

#[test]
fn a_mirror_joined_to_its_original_makes_one_symmetric_body_of_twice_the_volume() {
    // Two 15000 boxes touching along x = 0 join into one 100 x 30 x 10 block: 30000.
    let mut s = Session::new();
    box_feature(&mut s, [0.0, 0.0, 50.0, 30.0], 10.0, "new_body");
    let r = s
        .execute(
            "solid.mirror",
            &json!({"bodies": ["Extrude1"], "plane": "yz", "operation": "join"}),
        )
        .unwrap();
    assert_eq!(s.solids().len(), 1, "touching halves join into one body");
    assert!((exact_volume(&r) - 30_000.0).abs() < 1e-6);
    let solid = &s.solids()[0];
    let b = solid.body.bounding_box().unwrap();
    assert!(near_box(b, [-50.0, 0.0, 0.0], [50.0, 30.0, 10.0]));
    assert!(solid.scene_id.is_some(), "the joined body tessellates");
    // It exports: the STEP file holds a solid, and the STL mesh spans the whole block.
    let dir = std::env::temp_dir().join(format!("chamfera-mirror-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let step = dir.join("mirror.step");
    s.execute(
        "design.export_step",
        &json!({"path": step.to_str().unwrap()}),
    )
    .unwrap();
    let text = std::fs::read_to_string(&step).unwrap();
    assert!(text.contains("MANIFOLD_SOLID_BREP"));
    let stl = dir.join("mirror.stl");
    s.execute("design.export_stl", &json!({"path": stl.to_str().unwrap()}))
        .unwrap();
    let mesh = chamfera_io::stl::read(&std::fs::read(&stl).unwrap()).unwrap();
    let b = mesh.bounds().unwrap();
    assert!((b.min.x + 50.0).abs() < 1e-3 && (b.max.x - 50.0).abs() < 1e-3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_asymmetric_l_shape_mirrors_its_bounding_box() {
    // L = 40 x 10 foot (x 0..40, y 0..10) joined with a 10 x 20 leg (x 0..10, y 10..30),
    // 5 thick: (400 + 200) * 5 = 3000. Mirrored in the plane x = 60, x -> 120 - x: x 0..40
    // becomes 80..120, and the leg's outer corner (0, 30) goes to (120, 30).
    let mut s = Session::new();
    box_feature(&mut s, [0.0, 0.0, 40.0, 10.0], 5.0, "new_body");
    box_feature(&mut s, [0.0, 10.0, 10.0, 30.0], 5.0, "join");
    assert_eq!(s.solids().len(), 1);
    let r = s
        .execute(
            "solid.mirror",
            &json!({"bodies": ["Extrude1"],
                    "plane": {"origin": {"x": 60, "y": 0, "z": 0}, "normal": {"x": 1, "y": 0, "z": 0}}}),
        )
        .unwrap();
    assert!((exact_volume(&r) - 3_000.0).abs() < 1e-6);
    let b = solid_box(&s, "Mirror1");
    assert!(near_box(b, [80.0, 0.0, 0.0], [120.0, 30.0, 5.0]));
    // The leg's inner corner (10, 30) goes to (110, 30); nothing lands at (90, 30).
    let m = &s
        .solids()
        .iter()
        .find(|x| x.name == "Mirror1")
        .unwrap()
        .body;
    assert!(has_vertex(m, [110.0, 30.0, 0.0]));
    assert!(has_vertex(m, [120.0, 30.0, 5.0]));
    assert!(!has_vertex(m, [90.0, 30.0, 0.0]));
}

#[test]
fn mirroring_a_cut_feature_drills_the_symmetric_hole() {
    // Plate x -50..50, y -15..15, 10 thick (30000); a r = 3 hole at (20, 0) cut through,
    // mirrored as a feature in x = 0 with cut: two holes, 30000 - 2 * 9π * 10.
    let mut s = Session::new();
    box_feature(&mut s, [-50.0, -15.0, 50.0, 15.0], 10.0, "new_body");
    let sk = s.execute("sketch.create", &json!({"plane": "xy"})).unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute("sketch.circle", &json!({"cx": 20, "cy": 0, "radius": 3}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let cut = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sk, "profiles": [{"x": 20, "y": 0}], "distance": 10, "operation": "cut"}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    s.execute(
        "solid.mirror",
        &json!({"features": [cut], "plane": "yz", "operation": "cut"}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 1);
    let v = s.solids()[0].body.volume().unwrap();
    let want = 30_000.0 - 180.0 * std::f64::consts::PI;
    assert!((v - want).abs() < 1e-6, "{v}");
    // The cut is used by the mirror, so it cannot be deleted.
    let delete = s.execute("feature.delete", &json!({"feature": cut}));
    assert!(delete.is_err());
}

#[test]
fn moving_and_copying_bodies_shift_and_turn_them() {
    // Box x 0..50, y 0..30, z 0..10.
    let mut s = Session::new();
    box_feature(&mut s, [0.0, 0.0, 50.0, 30.0], 10.0, "new_body");
    // dx = 10 shifts the box to x 10..60; it stays one body with the same volume.
    let r = s
        .execute("solid.move", &json!({"bodies": ["Extrude1"], "dx": 10}))
        .unwrap();
    assert_eq!(s.solids().len(), 1);
    assert!((exact_volume(&r) - 15_000.0).abs() < 1e-6);
    let b = solid_box(&s, "Extrude1");
    assert!(near_box(b, [10.0, 0.0, 0.0], [60.0, 30.0, 10.0]));
    let mv = r["feature"].as_u64().unwrap();
    // Edited to dz = 5 only (dx and dy default to 0): z 5..15.
    s.execute("feature.edit", &json!({"feature": mv, "dz": "5 mm"}))
        .unwrap();
    let b = solid_box(&s, "Extrude1");
    assert!(near_box(b, [0.0, 0.0, 5.0], [50.0, 30.0, 15.0]));
    s.execute("feature.delete", &json!({"feature": mv}))
        .unwrap();
    // A quarter turn about z through the origin, copied: the corner (50, 0) goes to
    // (0, 50) and (0, 30) to (-30, 0), so the copy spans x -30..0, y 0..50.
    s.execute(
        "solid.move",
        &json!({"bodies": ["Extrude1"], "angle": 90, "copy": true}),
    )
    .unwrap();
    assert_eq!(s.solids().len(), 2, "the copy keeps the original");
    let b = solid_box(&s, "Extrude1");
    assert!(near_box(b, [0.0, 0.0, 0.0], [50.0, 30.0, 10.0]));
    let b = solid_box(&s, "Copy1");
    assert!(near_box(b, [-30.0, 0.0, 0.0], [0.0, 50.0, 10.0]));
    let c = &s.solids().iter().find(|x| x.name == "Copy1").unwrap().body;
    assert!(has_vertex(c, [0.0, 50.0, 0.0]));
    // Point to point: from (0, 0, 0) to (0, 0, 20) lifts the original to z 20..30.
    s.execute(
        "solid.move",
        &json!({"bodies": ["Extrude1"], "from": {"x": 0, "y": 0, "z": 0}, "to": {"x": 0, "y": 0, "z": 20}}),
    )
    .unwrap();
    let b = solid_box(&s, "Extrude1");
    assert!(near_box(b, [0.0, 0.0, 20.0], [50.0, 30.0, 30.0]));
    // Nothing to move, no transform, or a zero axis is refused.
    assert!(s.execute("solid.move", &json!({"dx": 5})).is_err());
    let none = s.execute("solid.move", &json!({"bodies": ["Extrude1"]}));
    assert!(none.is_err());
    let zero_axis = s.execute(
        "solid.move",
        &json!({"bodies": ["Extrude1"], "angle": 5, "axis": {"x": 0, "y": 0, "z": 0}}),
    );
    assert!(zero_axis.is_err());
}

#[test]
fn a_mirror_or_move_without_bodies_uses_the_selected_body() {
    let mut s = Session::new();
    box_feature(&mut s, [0.0, 0.0, 50.0, 30.0], 10.0, "new_body");
    assert!(s.execute("solid.mirror", &json!({"plane": "yz"})).is_err());
    let body = s.solids()[0].scene_id.unwrap();
    s.scene.selection = Some(chamfera_render::Hit {
        body,
        triangle: 0,
        point: chamfera_geom::Point3::ORIGIN,
        distance: 1.0,
    });
    let r = s
        .execute("solid.move", &json!({"dy": -30, "copy": true}))
        .unwrap();
    assert_eq!(r["definition"]["bodies"], json!(["Extrude1"]));
    let b = solid_box(&s, "Copy1");
    assert!(near_box(b, [0.0, -30.0, 0.0], [50.0, 0.0, 10.0]));
}

// ----- inspect -----

/// The 50 x 30 x 10 box of `extruded_plate`, and its solid's name.
fn measured_box(s: &mut Session) -> String {
    let r = extruded_plate(s, json!(10));
    r["solids"][0]["name"].as_str().unwrap().to_owned()
}

#[test]
fn mass_properties_of_the_box_match_the_textbook() {
    // 50 x 30 x 10 from the origin, density 2: m = 30000, centre (25, 15, 5),
    // I_xx = m (30² + 10²) / 12, I_yy = m (50² + 10²) / 12, I_zz = m (50² + 30²) / 12.
    let mut s = Session::new();
    let name = measured_box(&mut s);
    let r = s
        .execute("inspect.mass", &json!({"body": name, "density": 2}))
        .unwrap();
    let m = 30_000.0;
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    assert!((f(&r["mass"]) - m).abs() < 1e-8);
    assert!((f(&r["volume"]) - 15_000.0).abs() < 1e-8);
    assert!((f(&r["area"]) - 4_600.0).abs() < 1e-8);
    for (k, want) in [25.0, 15.0, 5.0].into_iter().enumerate() {
        assert!((f(&r["centre_of_mass"][k]) - want).abs() < 1e-9);
    }
    let want = [m * 1000.0 / 12.0, m * 2600.0 / 12.0, m * 3400.0 / 12.0];
    for k in 0..3 {
        assert!((f(&r["inertia"][k][k]) - want[k]).abs() < 1e-9 * want[k]);
        assert!((f(&r["principal_moments"][k]) - want[k]).abs() < 1e-9 * want[k]);
    }
    // By scene id too, density 1 by default.
    let id = r["body"].as_u64().unwrap();
    let r = s.execute("inspect.mass", &json!({"body": id})).unwrap();
    assert!((f(&r["mass"]) - 15_000.0).abs() < 1e-8);
    // Refused: no such solid, a bad density.
    assert!(
        s.execute("inspect.mass", &json!({"body": "nothing"}))
            .is_err()
    );
    assert!(
        s.execute("inspect.mass", &json!({"body": id, "density": 0}))
            .is_err()
    );
}

#[test]
fn measuring_two_box_faces_gives_ninety_degrees_and_an_edge_its_length() {
    let mut s = Session::new();
    let name = measured_box(&mut s);
    // Face names are the feature id (2) and the profile role.
    let r = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": name, "face": "2:top"}, "b": {"solid": name, "face": "2:side 0"}}),
        )
        .unwrap();
    assert!(
        (r["angle_degrees"].as_f64().unwrap() - 90.0).abs() < 1e-9,
        "{r}"
    );
    // They share an edge: distance zero.
    assert!(r["distance"].as_f64().unwrap() < 1e-12);
    assert!((r["a"]["area"].as_f64().unwrap() - 1500.0).abs() < 1e-9);
    // The top front edge is 50 long; the point (25, -7, 10) is 7 from it.
    let edge = edge_between(&mut s, [0.0, 0.0, 10.0], [50.0, 0.0, 10.0]);
    let r = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": name, "edge": edge}, "b": {"point": [25, -7, 10]}}),
        )
        .unwrap();
    assert!((r["a"]["length"].as_f64().unwrap() - 50.0).abs() < 1e-12);
    assert!((r["distance"].as_f64().unwrap() - 7.0).abs() < 1e-12);
    // (The solved sketch puts its corners within rounding of the dimensions.)
    for (k, want) in [25.0, 0.0, 10.0].into_iter().enumerate() {
        assert!((r["on_a"][k].as_f64().unwrap() - want).abs() < 1e-9, "{r}");
    }
    // A vertex by its point, to the far corner: the diagonal √(50² + 30² + 10²).
    let r = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": name, "vertex": [0, 0, 0]}, "b": {"point": [50, 30, 10]}}),
        )
        .unwrap();
    assert!((r["distance"].as_f64().unwrap() - 3500.0f64.sqrt()).abs() < 1e-12);
    // Unknown names are refused.
    let bad = json!({"a": {"solid": name, "face": "2:nowhere"}});
    assert!(s.execute("inspect.measure", &bad).is_err());
}

#[test]
fn two_separated_boxes_are_their_gap_apart() {
    // Box 1 is [0,50] x [0,30] x [0,10]; box 2 is a 10 x 10 rectangle at x 60..70
    // extruded 10 mm: the gap is 10 along x.
    let mut s = Session::new();
    let a = measured_box(&mut s);
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 60, "y1": 5, "x2": 70, "y2": 15}),
    )
    .unwrap();
    let sketch = s.active_sketch.unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let r = s
        .execute(
            "solid.extrude",
            &json!({"sketch": sketch, "profiles": [{"x": 65, "y": 10}], "distance": 10}),
        )
        .unwrap();
    let b = r["solids"][0]["name"].as_str().unwrap().to_owned();
    assert_ne!(a, b);
    let r = s
        .execute(
            "inspect.measure",
            &json!({"a": {"solid": a}, "b": {"solid": b}}),
        )
        .unwrap();
    assert!(
        (r["distance"].as_f64().unwrap() - 10.0).abs() < 1e-12,
        "{r}"
    );
    assert!((r["on_a"][0].as_f64().unwrap() - 50.0).abs() < 1e-12);
    assert!((r["on_b"][0].as_f64().unwrap() - 60.0).abs() < 1e-12);
    assert!((r["delta"][0].as_f64().unwrap() - 10.0).abs() < 1e-12);
}

#[test]
fn measure_at_picks_the_top_face_and_measures_from_an_earlier_pick() {
    let mut s = Session::new();
    let name = measured_box(&mut s);
    let (w, h) = (800.0, 600.0);
    s.execute("view.home", &json!({"width": w, "height": h}))
        .unwrap();
    // Where the top's centre (25, 15, 10) is on screen.
    let vp = chamfera_render::mat_mul(
        &s.camera.projection_matrix(w / h, s.scene.bounds().as_ref()),
        &s.camera.view_matrix(),
    );
    let n = chamfera_render::transform_point(&vp, chamfera_geom::Point3::new(25.0, 15.0, 10.0))
        .unwrap();
    let (x, y) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h);
    let r = s
        .execute(
            "inspect.measure_at",
            &json!({"x": x, "y": y, "width": w, "height": h}),
        )
        .unwrap();
    assert_eq!(r["entity"]["face"], json!("2:top"), "{r}");
    assert!((r["a"]["area"].as_f64().unwrap() - 1500.0).abs() < 1e-9);
    // From the point 5 above the top's centre to the face under the pointer: 5.
    let r = s
        .execute(
            "inspect.measure_at",
            &json!({"x": x, "y": y, "width": w, "height": h, "from": {"point": [25, 15, 15]}}),
        )
        .unwrap();
    assert!((r["distance"].as_f64().unwrap() - 5.0).abs() < 1e-12, "{r}");
    assert_eq!(r["entity"]["solid"], json!(name));
    // A corner pixel misses.
    let r = s
        .execute(
            "inspect.measure_at",
            &json!({"x": 1, "y": 1, "width": w, "height": h}),
        )
        .unwrap();
    assert!(r["entity"].is_null());
}

#[test]
fn inspect_commands_refuse_hostile_references_on_a_real_solid() {
    let mut s = Session::new();
    let name = measured_box(&mut s);
    let long = "x".repeat(100_000);
    let refs = [
        json!({"solid": name, "face": long}),
        json!({"solid": name, "edge": "#4294967296"}),
        json!({"solid": name, "vertex": [1e300, 0, 0]}),
        json!({"solid": name, "vertex": [f64::MAX, f64::MIN, 0]}),
        json!({"solid": name, "face": ""}),
        json!({"solid": name, "face": 5}),
        json!({"solid": name, "point": [0, 0]}),
        json!({"point": [1e7, 0, 0]}),
        json!({"body": 4294967296u64}),
        json!({"solid": [name]}),
        json!(null),
        json!("2:top"),
    ];
    for a in &refs {
        assert!(
            s.execute("inspect.measure", &json!({"a": a})).is_err(),
            "{a}"
        );
        for b in &refs {
            let _ = s.execute("inspect.measure", &json!({"a": a, "b": b}));
        }
        let at = json!({"x": 400, "y": 300, "width": 800, "height": 600, "from": a});
        let _ = s.execute("inspect.measure_at", &at);
    }
    for d in [json!(-1), json!(f64::MAX), json!("1"), json!(1e-300)] {
        let _ = s.execute("inspect.mass", &json!({"body": name, "density": d}));
    }
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

#[test]
fn ten_in_an_inch_design_is_254_mm_and_units_change_only_labels() {
    let mut s = Session::new();
    s.execute("design.new", &json!({"length": "in"})).unwrap();
    s.execute(
        "param.set",
        &json!({"name": "w", "expression": "10", "unit": "length"}),
    )
    .unwrap();
    // 10 in × 25.4 mm/in = 254 mm.
    assert!((s.doc.parameters["w"].value - 254.0).abs() < 1e-9);
    assert_eq!(s.doc.units.length(s.doc.parameters["w"].value), "10.00 in");
    // Back to mm: the stored "10" becomes "10 in", so w stays 254 mm.
    let r = s
        .execute("design.units", &json!({"length": "mm", "precision": 3}))
        .unwrap();
    assert_eq!(
        r,
        json!({"length": "mm", "precision": 3, "angle_precision": 2})
    );
    assert!((s.doc.parameters["w"].value - 254.0).abs() < 1e-9);
    assert_eq!(s.doc.units.length(12.3456), "12.346 mm");
    // It is one undo step, and a bad unit changes nothing.
    assert!(
        s.execute("design.units", &json!({"length": "furlong"}))
            .is_err()
    );
    assert!(s.execute("design.units", &json!({"precision": 9})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.units.length.symbol(), "in");
    assert_eq!(s.doc.parameters["w"].expression.as_deref(), Some("10"));
}

#[test]
fn orbit_about_a_pivot_keeps_the_pivot_where_it_was_relative_to_the_eye() {
    let mut s = Session::new();
    s.camera.target = chamfera_geom::Point3::new(0.0, 0.0, 0.0);
    let pivot = chamfera_geom::Point3::new(30.0, 0.0, 0.0);
    let before = s.camera.basis();
    let rel = pivot - s.camera.eye();
    let coords = (
        rel.dot(before.right),
        rel.dot(before.up),
        rel.dot(before.forward),
    );
    s.execute(
        "view.orbit",
        &json!({"yaw": 0.3, "pitch": 0.1, "pivot": {"x": 30, "y": 0, "z": 0}}),
    )
    .unwrap();
    // A rigid turn about the pivot: the pivot has the same camera-frame coordinates as
    // before, so it stays on the same pixel.
    let after = s.camera.basis();
    let rel = pivot - s.camera.eye();
    let now = (
        rel.dot(after.right),
        rel.dot(after.up),
        rel.dot(after.forward),
    );
    assert!((now.0 - coords.0).abs() < 1e-9 && (now.1 - coords.1).abs() < 1e-9);
    assert!((now.2 - coords.2).abs() < 1e-9);
    // Without a pivot the target stays put.
    let t = s.camera.target;
    s.execute("view.orbit", &json!({"yaw": 0.3, "pitch": 0.0}))
        .unwrap();
    assert!((s.camera.target - t).length() < 1e-12);
}

#[test]
fn the_grid_spacing_is_a_view_setting() {
    let mut s = Session::new();
    s.execute("view.grid", &json!({"spacing": 5})).unwrap();
    assert_eq!(s.scene.grid_spacing, Some(5.0));
    s.execute("view.grid", &json!({})).unwrap();
    assert_eq!(s.scene.grid_spacing, None);
    assert!(s.execute("view.grid", &json!({"spacing": 0})).is_err());
    assert!(!s.has_unsaved_changes());
    // view.point_at on an empty scene gives the point on the centre ray at the target.
    let r = s
        .execute(
            "view.point_at",
            &json!({"x": 400, "y": 300, "width": 800, "height": 600}),
        )
        .unwrap();
    let p = &r["point"];
    let t = s.camera.target;
    assert!((p["x"].as_f64().unwrap() - t.x).abs() < 1e-6);
    assert!((p["z"].as_f64().unwrap() - t.z).abs() < 1e-6);
}

// ---------------------------------------------------------------- extrude options

fn close_to(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-6 * want.abs().max(1.0)
}

/// The 50 x 30 plate profile of `dimensioned_rectangle`, extruded with extra options.
fn plate_with(s: &mut Session, options: serde_json::Value) -> CmdResult {
    dimensioned_rectangle(s);
    s.execute("sketch.finish", &json!({})).unwrap();
    let mut p = json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}]});
    for (k, v) in options.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("solid.extrude", &p)
}

fn z_range(s: &Session) -> (f64, f64) {
    let b = s.solids()[0].body.bounding_box().unwrap();
    (b.min.z, b.max.z)
}

#[test]
fn two_sides_extrude_each_side_by_its_own_distance() {
    // 10 up and 4 down: 50 · 30 · 10 + 50 · 30 · 4 = 15000 + 6000, spanning z -4 to 10.
    let mut s = Session::new();
    let r = plate_with(
        &mut s,
        json!({"distance": 10, "direction": "two_sides", "distance2": 4}),
    )
    .unwrap();
    assert!(close_to(exact_volume(&r), 21_000.0), "{r}");
    let (lo, hi) = z_range(&s);
    assert!(close_to(lo, -4.0) && close_to(hi, 10.0), "{lo} {hi}");
}

#[test]
fn an_offset_start_moves_the_extrude_off_the_sketch_plane() {
    // Start 5 above the sketch, 10 tall: the box spans z 5 to 15, volume 15000.
    let mut s = Session::new();
    let r = plate_with(&mut s, json!({"distance": 10, "start_offset": 5})).unwrap();
    assert!(close_to(exact_volume(&r), 15_000.0), "{r}");
    let (lo, hi) = z_range(&s);
    assert!(close_to(lo, 5.0) && close_to(hi, 15.0), "{lo} {hi}");
    // Reversed from the offset start: z -5 to 5.
    s.execute(
        "feature.edit",
        &json!({"feature": 2, "direction": "reversed"}),
    )
    .unwrap();
    let (lo, hi) = z_range(&s);
    assert!(close_to(lo, -5.0) && close_to(hi, 5.0), "{lo} {hi}");
    // A 0 offset removes it.
    s.execute("feature.edit", &json!({"feature": 2, "start_offset": 0}))
        .unwrap();
    let chamfera_doc::FeatureKind::Extrude(e) = &s.doc.feature(2).unwrap().kind else {
        panic!()
    };
    assert_eq!(e.start_offset, None);
}

/// The plate 50 x 30 x 10 and a circle d = 6 at its centre on its top face; returns the
/// circle's sketch.
fn plate_with_hole_sketch(s: &mut Session) -> u64 {
    extruded_plate(s, json!(10));
    let top = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top"}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 3}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    top
}

#[test]
fn a_through_all_cut_goes_through_the_plate_however_thick_it_gets() {
    // A d = 6 hole through the 10 mm plate: 15000 - π 3² 10 = 15000 - 90π.
    let mut s = Session::new();
    let top = plate_with_hole_sketch(&mut s);
    let cut = json!({"sketch": top, "profiles": [{"x": 25, "y": 15}], "extent": "through_all",
        "direction": "reversed", "operation": "cut"});
    let r = s.execute("solid.extrude", &cut).unwrap();
    let pi = std::f64::consts::PI;
    assert!(close_to(exact_volume(&r), 15_000.0 - 90.0 * pi), "{r}");
    // The plate made 20 thick: the hole still goes through, 30000 - 180π.
    s.execute("feature.edit", &json!({"feature": 2, "distance": 20}))
        .unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!(close_to(v, 30_000.0 - 180.0 * pi), "{v}");
    // Through all upward from the top face: no body lies that way, refused, unchanged.
    let before = s.doc.clone();
    let up = json!({"sketch": top, "profiles": [{"x": 25, "y": 15}], "extent": "through_all",
        "operation": "cut"});
    assert!(s.execute("solid.extrude", &up).is_err());
    assert_eq!(s.doc, before);
}

#[test]
fn a_through_all_needs_a_body() {
    let mut s = Session::new();
    let before_err = plate_with(&mut s, json!({"extent": "through_all"}));
    assert!(before_err.is_err());
    assert!(s.doc.features.is_empty() && s.solids().is_empty());
}

#[test]
fn to_object_goes_up_to_a_parallel_face_and_follows_it() {
    // The plate 50 x 30 x 10 (feature 2), then a block 10 x 10 beside it, 25 tall
    // (feature 4: its top face is at z = 25). A 10 x 10 square on the plate's top
    // (z = 10) extruded to the block's top: a boss 15 tall, 1500.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 60, "y1": 0, "x2": 70, "y2": 10}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let block = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 3, "profiles": [{"x": 65, "y": 5}], "distance": 25}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let on_top = s
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
    let boss = |to: serde_json::Value| json!({"sketch": on_top, "profiles": [{"x": 25, "y": 15}], "extent": "to_object", "to": to});
    // Not parallel: the block's side is vertical. Refused, nothing changes.
    let before = s.doc.clone();
    let side = s.execute(
        "solid.extrude",
        &boss(json!({"feature": block, "face": "side 0"})),
    );
    assert!(
        side.as_ref()
            .is_err_and(|e| e.to_string().contains("not parallel")),
        "{side:?}"
    );
    assert_eq!(s.doc, before);
    let r = s
        .execute(
            "solid.extrude",
            &boss(json!({"feature": block, "face": "top"})),
        )
        .unwrap();
    let id = r["feature"].as_u64().unwrap();
    assert!(close_to(exact_volume(&r), 1_500.0), "{r}");
    let boss_box = |s: &Session| {
        let solid = s
            .solids()
            .iter()
            .find(|x| u64::from(x.feature) == id)
            .unwrap();
        let b = solid.body.bounding_box().unwrap();
        (b.min.z, b.max.z)
    };
    let (lo, hi) = boss_box(&s);
    assert!(close_to(lo, 10.0) && close_to(hi, 25.0), "{lo} {hi}");
    // The block grows to 30: the boss follows its top, 2000.
    s.execute("feature.edit", &json!({"feature": block, "distance": 30}))
        .unwrap();
    let (_, hi) = boss_box(&s);
    assert!(close_to(hi, 30.0), "{hi}");
    // The block cannot be deleted while the boss goes up to its face.
    assert!(
        s.execute("feature.delete", &json!({"feature": block}))
            .is_err()
    );
    // Up to a fixed plane at z = 40 instead: 10 x 10 x 30 = 3000.
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": id, "to": {"plane": {"origin": {"x": 0, "y": 0, "z": 40},
                "normal": {"x": 0, "y": 0, "z": 1}}}}),
        )
        .unwrap();
    assert!(close_to(exact_volume(&r), 3_000.0), "{r}");
    // A tilted plane is refused, and the boss keeps its plane.
    let before = s.doc.clone();
    let tilted = json!({"feature": id, "to": {"plane": {"origin": {"x": 0, "y": 0, "z": 40},
        "normal": {"x": 0, "y": 1, "z": 1}}}});
    assert!(s.execute("feature.edit", &tilted).is_err());
    assert_eq!(s.doc, before);
}

#[test]
fn a_tapered_plate_is_the_prismatoid_frustum() {
    // 50 x 30, 10 tall, 5° taper: each side moves in by d = 10 tan 5° at the top; the
    // section (50 - 2δ)(30 - 2δ) is quadratic in the height, so the prismatoid formula
    // is exact: V = h/6 [A₀ + 4 A_mid + A_top] = 10 (1500 - 80 d + 4/3 d²).
    let mut s = Session::new();
    let r = plate_with(&mut s, json!({"distance": 10, "taper": 5})).unwrap();
    let d = 10.0 * 5f64.to_radians().tan();
    let want = 10.0 * (1500.0 - 80.0 * d + 4.0 / 3.0 * d * d);
    assert!(close_to(exact_volume(&r), want), "{r} vs {want}");
    assert!(r["display_error"].is_null(), "{r}");
    // Symmetric with the taper: two halves of 5, each narrowing away from the sketch.
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": 2, "direction": "symmetric"}),
        )
        .unwrap();
    let d = 5.0 * 5f64.to_radians().tan();
    let half = 5.0 * (1500.0 - 80.0 * d + 4.0 / 3.0 * d * d);
    assert!(
        close_to(exact_volume(&r), 2.0 * half),
        "{r} vs {}",
        2.0 * half
    );
    let (lo, hi) = z_range(&s);
    assert!(close_to(lo, -5.0) && close_to(hi, 5.0), "{lo} {hi}");
}

#[test]
fn a_tapered_circle_is_a_cone_frustum() {
    // r = 5, h = 10, 10°: r₂ = 5 - 10 tan 10°, V = π h / 3 (r₁² + r₁ r₂ + r₂²).
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 5}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let r = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 10, "taper": "10 deg"}),
        )
        .unwrap();
    let (r1, r2) = (5.0, 5.0 - 10.0 * 10f64.to_radians().tan());
    let want = std::f64::consts::PI * 10.0 / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2);
    assert!(close_to(exact_volume(&r), want), "{r} vs {want}");
    assert!(r["display_error"].is_null(), "{r}");
}

#[test]
fn refused_extrude_options_change_nothing() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let before = s.doc.clone();
    let volume = s.solids()[0].body.volume().unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    for bad in [
        // 60° closes the 30 mm side before the top (10 tan 60° > 15).
        json!({"feature": 2, "taper": 60}),
        json!({"feature": 2, "taper": "89 deg"}),
        json!({"feature": 2, "direction": "two_sides"}),
        json!({"feature": 2, "direction": "two_sides", "distance2": -1}),
        json!({"feature": 2, "extent": "to_object"}),
        json!({"feature": 2, "extent": "sideways"}),
        json!({"feature": 2, "to": {"feature": 2, "face": "top"}}),
        json!({"feature": 2, "to": {"plane": {"origin": {"x": 0, "y": 0, "z": 0},
            "normal": {"x": 0, "y": 0, "z": 0}}}}),
        json!({"feature": 2, "start_offset": "5 deg"}),
    ] {
        assert!(s.execute("feature.edit", &bad).is_err(), "{bad}");
        assert_eq!(s.doc, before, "{bad}");
        assert!(close_to(s.solids()[0].body.volume().unwrap(), volume));
    }
    // A revolve takes no extrude options.
    assert!(
        s.execute("feature.edit", &json!({"feature": 2, "size": 1}))
            .is_err()
    );
}

#[test]
fn the_extrude_preview_shows_the_shape_without_changing_the_design() {
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    s.execute("sketch.finish", &json!({})).unwrap();
    let before = s.doc.clone();
    let r = s
        .execute(
            "solid.extrude_preview",
            &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 10,
                "direction": "two_sides", "distance2": 4}),
        )
        .unwrap();
    assert!(close_to(r["volume"].as_f64().unwrap(), 21_000.0), "{r}");
    assert!(
        close_to(r["bounds"]["min"][2].as_f64().unwrap(), -4.0),
        "{r}"
    );
    assert_eq!(s.scene.previews.len(), 1);
    // The preview mesh holds the same volume; nothing was added to the design.
    assert!(close_to(s.scene.previews[0].signed_volume(), 21_000.0));
    assert_eq!(s.doc, before);
    assert!(s.solids().is_empty());
    // A refused preview shows nothing.
    assert!(
        s.execute(
            "solid.extrude_preview",
            &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 10, "taper": 60}),
        )
        .is_err()
    );
    assert!(s.scene.previews.is_empty());
    s.execute(
        "solid.extrude_preview",
        &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 3}),
    )
    .unwrap();
    s.execute("solid.extrude_preview", &json!({"clear": true}))
        .unwrap();
    assert!(s.scene.previews.is_empty());
    // Extruding for real removes the preview.
    s.execute(
        "solid.extrude_preview",
        &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 3}),
    )
    .unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 3}),
    )
    .unwrap();
    assert!(s.scene.previews.is_empty());
}

// ----- sketch shapes and projected edges -----

#[test]
fn polygon_slot_and_ellipse_commands_draw_shapes_of_their_exact_area() {
    let pi = std::f64::consts::PI;
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    // A hexagon inscribed in r = 10: six equilateral triangles, (3√3/2) · 100.
    s.execute(
        "sketch.polygon",
        &json!({"cx": 0, "cy": 0, "x": 10, "y": 0, "sides": 6}),
    )
    .unwrap();
    let hexagon = 1.5 * 3f64.sqrt() * 100.0;
    assert!((sketch_areas(&mut s)[0] - hexagon).abs() < 1e-9);
    // Circumscribed about r = 5, a square is 10 x 10.
    s.execute(
        "sketch.polygon",
        &json!({"cx": 40, "cy": 0, "x": 45, "y": 0, "sides": 4, "mode": "circumscribed"}),
    )
    .unwrap();
    // A slot with centres 20 apart, 6 wide: 20 · 6 + π · 3².
    s.execute(
        "sketch.slot",
        &json!({"x1": 0, "y1": 30, "x2": 20, "y2": 30, "width": 6}),
    )
    .unwrap();
    // An ellipse a = 10, b = 5: π a b.
    s.execute(
        "sketch.ellipse",
        &json!({"cx": 0, "cy": 60, "x": 10, "y": 60, "minor": 5}),
    )
    .unwrap();
    let a = sketch_areas(&mut s);
    let mut expected = [hexagon, 100.0, 120.0 + 9.0 * pi, 50.0 * pi];
    expected.sort_by(f64::total_cmp);
    assert_eq!(a.len(), 4, "{a:?}");
    for (got, want) in a.iter().zip(expected) {
        assert!((got - want).abs() < 1e-6, "{a:?} vs {expected:?}");
    }
    // Bad input is refused and changes nothing.
    let before = s.doc.clone();
    for (id, p) in [
        (
            "sketch.polygon",
            json!({"cx": 0, "cy": 0, "x": 1, "y": 0, "sides": 2}),
        ),
        (
            "sketch.polygon",
            json!({"cx": 0, "cy": 0, "x": 1, "y": 0, "sides": 65}),
        ),
        (
            "sketch.polygon",
            json!({"cx": 0, "cy": 0, "x": 1, "y": 0, "sides": 5, "mode": "x"}),
        ),
        (
            "sketch.slot",
            json!({"x1": 0, "y1": 0, "x2": 0, "y2": 0, "width": 2}),
        ),
        (
            "sketch.slot",
            json!({"x1": 0, "y1": 0, "x2": 4, "y2": 0, "width": 6, "overall": true}),
        ),
        (
            "sketch.ellipse",
            json!({"cx": 0, "cy": 0, "x": 0, "y": 0, "minor": 2}),
        ),
        (
            "sketch.ellipse",
            json!({"cx": 0, "cy": 0, "x": 5, "y": 0, "minor": -2}),
        ),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(s.doc, before);
}

#[test]
fn an_ellipse_extrudes_into_an_elliptic_cylinder() {
    // a = 10, b = 5, 10 high: π a b h = 500π. Its side is one extrusion surface per
    // half of the ellipse, its caps two planes.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.ellipse",
        &json!({"cx": 0, "cy": 0, "x": 10, "y": 0, "minor": 5}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 10}),
    )
    .unwrap();
    let solids = s.solids();
    assert_eq!(solids.len(), 1);
    let b = &solids[0].body;
    let v = b.volume().unwrap();
    assert!((v - 500.0 * std::f64::consts::PI).abs() < 1e-6, "{v}");
    b.validate().unwrap();
    let kinds: Vec<&str> = b
        .faces()
        .into_iter()
        .map(|f| match b.face_surface(f).unwrap().unwrap().0 {
            chamfera_kernel::Surface::Plane(_) => "plane",
            chamfera_kernel::Surface::Extrusion(_) => "extrusion",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "extrusion").count(),
        2,
        "{kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == "plane").count(),
        2,
        "{kinds:?}"
    );
    // The mass properties agree.
    let m = s
        .execute("inspect.mass", &json!({"body": "Extrude1", "density": 1}))
        .unwrap();
    let mv = m["volume"].as_f64().unwrap();
    assert!((mv - 500.0 * std::f64::consts::PI).abs() < 1e-6, "{m}");
}

/// The ellipse plate of `an_ellipse_extrudes_into_an_elliptic_cylinder` (feature 2,
/// 500π), and a second sketch (feature 3) on XY with the rectangle `x1..x2` by
/// `y1..y2`, finished.
fn ellipse_plate_and_rectangle(s: &mut Session, rect: [f64; 4]) {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.ellipse",
        &json!({"cx": 0, "cy": 0, "x": 10, "y": 0, "minor": 5}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "distance": 10}),
    )
    .unwrap();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": rect[0], "y1": rect[1], "x2": rect[2], "y2": rect[3]}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
}

#[test]
fn an_ellipse_plate_is_cut_joined_and_intersected() {
    // The rectangle 0..30 by -20..20 covers the half x >= 0 of the ellipse a = 10, b = 5.
    // Extruded 10 with the plate: cut leaves 250π, intersect keeps 250π, join makes
    // the box (30 · 40 · 10 = 12000) and the other half: 12000 + 250π.
    let pi = std::f64::consts::PI;
    for (op, expected) in [
        ("cut", 250.0 * pi),
        ("intersect", 250.0 * pi),
        ("join", 12_000.0 + 250.0 * pi),
    ] {
        let mut s = Session::new();
        ellipse_plate_and_rectangle(&mut s, [0.0, -20.0, 30.0, 20.0]);
        let r = s
            .execute(
                "solid.extrude",
                &json!({"sketch": 3, "profiles": [{"x": 15, "y": 0}], "distance": 10,
                        "operation": op}),
            )
            .unwrap();
        let v = exact_volume(&r);
        assert!(
            (v - expected).abs() < 1e-6 * expected,
            "{op}: {v} vs {expected}"
        );
        s.solids()[0].body.validate().unwrap();
    }
}

#[test]
fn a_hole_drills_through_an_ellipse_plate_and_exports_to_step() {
    // A through hole of diameter 4 at the centre of the 500π plate: 500π - π 2² 10 = 460π.
    let pi = std::f64::consts::PI;
    let mut s = Session::new();
    ellipse_plate_and_rectangle(&mut s, [20.0, 20.0, 30.0, 30.0]);
    let plane = s.named_face_plane("2:top").unwrap();
    let at = plane.to_plane(chamfera_geom::Point3::new(0.0, 0.0, 10.0));
    let r = s
        .execute(
            "solid.hole",
            &json!({"face": "2:top", "points": [{"x": at.x, "y": at.y}], "diameter": 4,
                    "through_all": true, "tip_angle": "flat"}),
        )
        .unwrap();
    let v = exact_volume(&r);
    assert!((v - 460.0 * pi).abs() < 1e-6, "{v}");
    let body = &s.solids()[0].body;
    body.validate().unwrap();
    // It tessellates and exports: the STEP file holds the extrusion surfaces and is read
    // back by our reader.
    chamfera_kernel::tessellate(body, &chamfera_kernel::TessOptions::default()).unwrap();
    let dir = std::env::temp_dir().join(format!("chamfera-ellipse-hole-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plate.step");
    s.execute(
        "design.export_step",
        &json!({"path": path.to_str().unwrap()}),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("SURFACE_OF_LINEAR_EXTRUSION"),
        "no extrusion surface"
    );
    assert!(text.contains("CYLINDRICAL_SURFACE"), "no hole wall");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_refused_spline_boolean_changes_nothing() {
    // A round bar along y (a circle on the XZ plane, cut 20 deep) crosses the plate's
    // elliptic sides, which run along z: the cylinder and the extrusion are not parallel
    // and meet in a space curve, which is refused. The design is as it was.
    let mut s = Session::new();
    ellipse_plate_and_rectangle(&mut s, [20.0, 20.0, 30.0, 30.0]);
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 5, "radius": 1}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let before = s.doc.clone();
    let e = s
        .execute(
            "solid.extrude",
            &json!({"sketch": 4, "profiles": [{"x": 0, "y": 5}], "distance": 20,
                    "operation": "cut"}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("space curve"), "{e}");
    assert_eq!(s.doc, before);
}

#[test]
fn a_half_ellipse_closed_by_its_axis_extrudes_to_half_the_volume() {
    // The upper half of a = 10, b = 5 (an elliptical arc from +x to -x) closed by the
    // major axis: area π a b / 2 = 25π, 4 high: 100π.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.ellipse.arc",
        &json!({"cx": 0, "cy": 0, "x": 10, "y": 0, "minor": 5,
                "x1": 1, "y1": 0, "x2": -1, "y2": 0}),
    )
    .unwrap();
    s.execute(
        "sketch.line",
        &json!({"x1": -10, "y1": 0, "x2": 10, "y2": 0}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 0, "y": 2}], "distance": 4}),
    )
    .unwrap();
    let v = s.solids()[0].body.volume().unwrap();
    assert!((v - 100.0 * std::f64::consts::PI).abs() < 1e-6, "{v}");
}

#[test]
fn a_tapered_ellipse_or_spline_is_refused_saying_why_and_changes_nothing() {
    // The offset of an ellipse or spline is not a spline, so a tapered side would not be
    // exact: refused, with the reason and what to do instead.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.ellipse",
        &json!({"cx": 0, "cy": 0, "x": 10, "y": 0, "minor": 5}),
    )
    .unwrap();
    s.execute(
        "sketch.spline",
        &json!({"points": [{"x": 30, "y": 0}, {"x": 40, "y": 5}, {"x": 35, "y": 12},
                           {"x": 28, "y": 6}], "kind": "fit", "closed": true}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let before = s.doc.clone();
    for seed in [json!({"x": 0, "y": 0}), json!({"x": 34, "y": 5})] {
        let e = s
            .execute(
                "solid.extrude",
                &json!({"sketch": 1, "profiles": [seed], "distance": 10, "taper": 5}),
            )
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("cannot be tapered") && e.contains("offset"),
            "{e}"
        );
        assert_eq!(s.doc, before);
        assert!(s.solids().is_empty());
    }
}

/// The length dimension of sketch 1 with this value.
fn length_dimension(s: &mut Session, value: f64) -> u64 {
    let g = s.execute("sketch.get", &json!({"sketch": 1})).unwrap();
    g["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["definition"]["kind"] == "length"
                && (c["definition"]["value"].as_f64().unwrap() - value).abs() < 1e-9
        })
        .and_then(|c| c["constraint"].as_u64())
        .unwrap()
}

#[test]
fn projecting_the_top_of_a_box_gives_its_outline_which_follows_the_box() {
    // The 50 x 30 x 10 box; its top edges projected onto a sketch on its top face are a
    // 50 x 30 rectangle, area 1500, fixed in place (no freedom).
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let sketch = s
        .execute(
            "sketch.create_on_face",
            &json!({"feature": 2, "face": "top"}),
        )
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    let edges: Vec<String> = (0..4).map(|k| format!("2:top {k}")).collect();
    let r = s
        .execute("sketch.project", &json!({ "edges": edges }))
        .unwrap();
    assert_eq!(r["curves"].as_array().unwrap().len(), 4, "{r}");
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["dof"], 0);
    assert_eq!(g["status"], "FullyConstrained");
    let a = sketch_areas(&mut s);
    assert_eq!(a.len(), 1);
    assert!((a[0] - 1500.0).abs() < 1e-6, "{a:?}");
    // Projecting the same edge again adds nothing.
    let again = s
        .execute("sketch.project", &json!({"edges": ["2:top 0"]}))
        .unwrap();
    assert!(again["curves"].as_array().unwrap().is_empty());
    // The box grows to 60 wide: the projected rectangle follows, 60 x 30.
    s.execute("sketch.finish", &json!({})).unwrap();
    let width = length_dimension(&mut s, 50.0);
    s.execute("sketch.edit", &json!({"sketch": 1})).unwrap();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "value": 60}),
    )
    .unwrap();
    s.execute("sketch.edit", &json!({ "sketch": sketch }))
        .unwrap();
    let a = sketch_areas(&mut s);
    assert_eq!(a.len(), 1);
    assert!((a[0] - 1800.0).abs() < 1e-6, "{a:?}");
    // Undo brings back the 50 x 30 box and its outline.
    s.execute("edit.undo", &json!({})).unwrap();
    let a = sketch_areas(&mut s);
    assert!((a[0] - 1500.0).abs() < 1e-6, "{a:?}");
}

#[test]
fn a_projected_hole_is_a_circle_of_its_radius_and_a_face_projects_all_its_loops() {
    // The plate with a hole of radius 4 at (25, 15) through it: projecting its top face
    // gives the outline (4 lines) and the hole (a circle of radius 4).
    let mut s = Session::new();
    dimensioned_rectangle(&mut s);
    s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 4}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
    )
    .unwrap();
    s.execute(
        "sketch.create_on_face",
        &json!({"feature": 2, "face": "top"}),
    )
    .unwrap();
    let r = s
        .execute(
            "sketch.project",
            &json!({"faces": [{"feature": 2, "face": "top"}], "linked": true}),
        )
        .unwrap();
    assert_eq!(r["curves"].as_array().unwrap().len(), 5, "{r}");
    let f = s.active_sketch().unwrap();
    let circles: Vec<f64> = f
        .sketch
        .curves()
        .filter_map(|(id, c)| match c {
            chamfera_sketch::Curve::Circle { .. } => f.sketch.radius(id),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 1);
    assert!((circles[0] - 4.0).abs() < 1e-9, "{circles:?}");
    let centre = f
        .sketch
        .curves()
        .find_map(|(_, c)| match c {
            chamfera_sketch::Curve::Circle { centre, .. } => f.sketch.point(*centre),
            _ => None,
        })
        .unwrap();
    assert!(centre.distance(chamfera_geom::Point2::new(25.0, 15.0)) < 1e-9);
    // The plate with the hole and the disc: 1500 - 16π and 16π.
    let pi = std::f64::consts::PI;
    let a = sketch_areas(&mut s);
    assert!((a[0] - 16.0 * pi).abs() < 1e-6, "{a:?}");
    assert!((a[1] - (1500.0 - 16.0 * pi)).abs() < 1e-6, "{a:?}");
    // Projected curves draw in their own colour, and their fixes have no labels.
    use chamfera_render::OverlayStyle;
    assert!(
        s.scene
            .overlays
            .iter()
            .any(|o| o.style == OverlayStyle::Projected && !o.lines.is_empty())
    );
    assert!(s.dimension_labels().is_empty());
    // The design saves and opens with its projections.
    let json = s.doc.to_json().unwrap();
    let back = chamfera_doc::Document::from_json(&json).unwrap();
    assert_eq!(back.sketches[1].projections.len(), 5);
    // Deleting a projected curve forgets its link.
    let curve = back.sketches[1].projections[0].curve;
    s.sketch_selection = vec![crate::sketching::SketchItem::Curve(curve)];
    s.execute("sketch.remove", &json!({})).unwrap();
    assert_eq!(s.active_sketch().unwrap().projections.len(), 4);
}

#[test]
fn projecting_needs_a_sketch_and_real_edges() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    // No sketch being edited.
    assert!(
        s.execute("sketch.project", &json!({"edges": ["2:top 0"]}))
            .is_err()
    );
    s.execute("sketch.create", &json!({"plane": "xz"})).unwrap();
    let before = s.doc.clone();
    for p in [
        json!({}),
        json!({"edges": ["2:nowhere 9"]}),
        json!({"edges": "2:top 0"}),
        json!({"faces": [{"feature": 2, "face": "nowhere"}]}),
        json!({"faces": [{"feature": "2"}]}),
        json!({"edges": ["2:top 0"], "linked": "yes"}),
        json!({"pick": {"x": -5, "y": -5, "width": 10, "height": 10}}),
    ] {
        assert!(s.execute("sketch.project", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc, before);
    // On the XZ plane a top edge along X stays a line 50 long at z = 10, and an edge along
    // Y is seen end-on: a point, refused.
    let r = s
        .execute(
            "sketch.project",
            &json!({"edges": ["2:top 0"], "linked": false}),
        )
        .unwrap();
    let curve = r["curves"][0].as_u64().unwrap();
    let f = s.active_sketch().unwrap();
    let Some(chamfera_sketch::Curve::Line { start, end, .. }) =
        f.sketch.curve(chamfera_sketch::CurveId(curve as u32))
    else {
        panic!("not a line");
    };
    let (a, b) = (
        f.sketch.point(*start).unwrap(),
        f.sketch.point(*end).unwrap(),
    );
    assert!((a.distance(b) - 50.0).abs() < 1e-9);
    assert!((a.y - 10.0).abs() < 1e-9 && (b.y - 10.0).abs() < 1e-9);
    assert!(!f.projections[0].linked);
    assert!(
        s.execute("sketch.project", &json!({"edges": ["2:top 1"]}))
            .is_err()
    );
}

// ----- direct face edits and Press Pull -----

fn face_names(s: &Session) -> Vec<String> {
    s.solids()
        .iter()
        .flat_map(|x| {
            x.body
                .faces()
                .into_iter()
                .filter_map(|f| x.body.face_name(f).ok().flatten().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn solid_volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

#[test]
fn offset_face_moves_the_top_of_the_plate_and_follows_upstream_edits() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    // 50 x 30 x (10 + 5) and 50 x 30 x (10 - 4).
    let r = s
        .execute(
            "solid.offset_face",
            &json!({"faces": ["2:top"], "distance": 5}),
        )
        .unwrap();
    assert!((exact_volume(&r) - 22_500.0).abs() < 1e-6);
    let id = r["feature"].as_u64().unwrap();
    assert_eq!(r["name"], "Offset Face1");
    let r = s
        .execute("feature.edit", &json!({"feature": id, "distance": -4}))
        .unwrap();
    assert!((exact_volume(&r) - 9_000.0).abs() < 1e-6);
    // The plate made 20 thick upstream: the top still drops 4, 50·30·16.
    s.execute("feature.edit", &json!({"feature": 2, "distance": 20}))
        .unwrap();
    assert!((solid_volume(&s) - 24_000.0).abs() < 1e-6);
    assert!(s.feature_errors().is_empty());
    // A face edit takes no extrude or shell values.
    assert!(
        s.execute("feature.edit", &json!({"feature": id, "thickness": 1}))
            .is_err()
    );
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((solid_volume(&s) - 9_000.0).abs() < 1e-6);
}

#[test]
fn an_offset_through_the_body_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let before = s.doc.clone();
    for p in [
        json!({"faces": ["2:top"], "distance": -10}),
        json!({"faces": ["2:top"], "distance": -15}),
        json!({"faces": ["2:top"], "distance": 0}),
        json!({"faces": ["9:nowhere"], "distance": 1}),
        json!({"faces": [], "distance": 1}),
    ] {
        assert!(s.execute("solid.offset_face", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc, before);
    assert!((solid_volume(&s) - 15_000.0).abs() < 1e-6);
}

/// A 50 x 30 x 10 plate with a through hole of diameter 6 at (25, 15); returns the hole
/// wall's face name.
fn plate_with_hole_wall(s: &mut Session) -> String {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    )
    .unwrap();
    s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 3}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
    )
    .unwrap();
    let x = &s.solids()[0].body;
    x.faces()
        .into_iter()
        .find(|f| {
            matches!(
                x.face_surface(*f).unwrap(),
                Some((chamfera_kernel::Surface::Cylinder(_), _))
            )
        })
        .and_then(|f| x.face_name(f).unwrap().map(str::to_owned))
        .unwrap()
}

#[test]
fn offsetting_a_hole_wall_widens_the_hole() {
    let mut s = Session::new();
    let wall = plate_with_hole_wall(&mut s);
    let pi = std::f64::consts::PI;
    assert!((solid_volume(&s) - (15_000.0 - 90.0 * pi)).abs() < 1e-6);
    // The wall's outward normal points into the hole: -1 takes the radius from 3 to 4,
    // 15000 - 16π·10.
    let r = s
        .execute(
            "solid.offset_face",
            &json!({"faces": [wall], "distance": -1}),
        )
        .unwrap();
    let want = 15_000.0 - 160.0 * pi;
    assert!(
        (exact_volume(&r) - want).abs() < 1e-6,
        "{}",
        exact_volume(&r)
    );
}

#[test]
fn deleting_a_fillet_face_gives_the_sharp_corner_back() {
    // The filleted-block example without its chamfer: 60 x 40 x 15 with a hole of r 8
    // and its four vertical edges filleted r 8. Deleting one fillet face adds back
    // (8² - π 8²/4) · 15 = (64 - 16π) · 15.
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 60, "y2": 40}),
    )
    .unwrap();
    s.execute("sketch.circle", &json!({"cx": 30, "cy": 20, "radius": 8}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 15}),
    )
    .unwrap();
    let r = s
        .execute(
            "solid.fillet",
            &json!({"edges": ["2:sweep 0", "2:sweep 1", "2:sweep 2", "2:sweep 3"], "radius": 8}),
        )
        .unwrap();
    let pi = std::f64::consts::PI;
    let corner = (64.0 - 16.0 * pi) * 15.0;
    let filleted = 36_000.0 - 64.0 * pi * 15.0 - 4.0 * corner;
    assert!((exact_volume(&r) - filleted).abs() < 1e-6);
    let fillet = r["feature"].as_u64().unwrap();
    let name = format!("{fillet}:fillet of 2:sweep 0");
    assert!(face_names(&s).contains(&name));
    let r = s
        .execute("solid.delete_face", &json!({"faces": [name]}))
        .unwrap();
    assert!((exact_volume(&r) - (filleted + corner)).abs() < 1e-6);
    // Deleting a plain side face cannot be healed: refused, nothing changes.
    let before = s.doc.clone();
    assert!(
        s.execute("solid.delete_face", &json!({"faces": ["2:side 0"]}))
            .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn deleting_a_blind_holes_faces_fills_it() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let plane = s.named_face_plane("2:top").unwrap();
    let at = plane.to_plane(chamfera_geom::Point3::new(25.0, 15.0, 10.0));
    let r = s
        .execute(
            "solid.hole",
            &json!({"face": "2:top", "points": [{"x": at.x, "y": at.y}], "diameter": 6,
                    "depth": 4, "tip_angle": "flat"}),
        )
        .unwrap();
    // A flat-bottomed hole r 3, 4 deep: 15000 - 9π·4.
    let pi = std::f64::consts::PI;
    assert!((exact_volume(&r) - (15_000.0 - 36.0 * pi)).abs() < 1e-6);
    let id = r["feature"].as_u64().unwrap();
    let faces = vec![format!("{id}:hole 0 wall"), format!("{id}:hole 0 floor")];
    let r = s
        .execute("solid.delete_face", &json!({"faces": faces}))
        .unwrap();
    assert!((exact_volume(&r) - 15_000.0).abs() < 1e-6);
}

#[test]
fn press_pull_picks_the_operation_from_what_it_is_given() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    // A face: the same as Offset Face.
    let r = s
        .execute("solid.press_pull", &json!({"face": "2:top", "distance": 5}))
        .unwrap();
    assert_eq!(r["operation"], "Offset Face");
    assert!((exact_volume(&r) - 22_500.0).abs() < 1e-6);
    let id = r["feature"].as_u64().unwrap();
    assert!(matches!(
        s.doc.feature(id as u32).unwrap().kind,
        chamfera_doc::FeatureKind::OffsetFace(_)
    ));
    // An edge: a fillet. `top 0` is a 50 mm edge of the (raised) top.
    let r = s
        .execute(
            "solid.press_pull",
            &json!({"edge": "2:top 0", "distance": 2}),
        )
        .unwrap();
    assert_eq!(r["operation"], "Fillet");
    let fillet = r["feature"].as_u64().unwrap();
    // 22500 - (1 - π/4) 2² · 50.
    let pi = std::f64::consts::PI;
    let v = 22_500.0 - (1.0 - pi / 4.0) * 4.0 * 50.0;
    assert!((exact_volume(&r) - v).abs() < 1e-6, "{}", exact_volume(&r));
    // The fillet's face: its radius changes (to 3) instead of a new feature.
    let n = s.doc.features.len();
    let face = format!("{fillet}:fillet of 2:top 0");
    let preview = s
        .execute(
            "solid.press_pull",
            &json!({"face": face, "distance": 3, "preview": true}),
        )
        .unwrap();
    let v3 = 22_500.0 - (1.0 - pi / 4.0) * 9.0 * 50.0;
    assert!((preview["volume"].as_f64().unwrap() - v3).abs() < 1e-6);
    assert_eq!(s.scene.previews.len(), 1);
    assert_eq!(s.doc.features.len(), n);
    let r = s
        .execute("solid.press_pull", &json!({"face": face, "distance": 3}))
        .unwrap();
    assert_eq!(r["operation"], "Fillet radius");
    assert_eq!(s.doc.features.len(), n);
    assert!(s.scene.previews.is_empty());
    assert!((solid_volume(&s) - v3).abs() < 1e-6);
    // A sketch profile: an extrude.
    let r = s
        .execute(
            "solid.press_pull",
            &json!({"sketch": 1, "profile": {"x": 25, "y": 15}, "distance": 4}),
        )
        .unwrap();
    assert_eq!(r["operation"], "Extrude");
    // Nothing or two things given: refused.
    assert!(
        s.execute("solid.press_pull", &json!({"distance": 4}))
            .is_err()
    );
    assert!(
        s.execute(
            "solid.press_pull",
            &json!({"face": "2:top", "edge": "2:top 0", "distance": 4})
        )
        .is_err()
    );
}

#[test]
fn move_face_tilts_the_top_about_its_centre_line() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    // A turn of 10° about the line y = 15, z = 10 along x: the wedges gained and lost are
    // equal, so the volume stays 15000.
    let r = s
        .execute(
            "solid.move_face",
            &json!({"faces": ["2:top"], "angle": 10,
                    "origin": {"x": 0, "y": 15, "z": 10}, "axis": {"x": 1, "y": 0, "z": 0}}),
        )
        .unwrap();
    assert!((exact_volume(&r) - 15_000.0).abs() < 1e-6);
    // The bottom moved 5 along its outward normal (down): 50 x 30 x 15 less nothing.
    let r = s
        .execute(
            "solid.move_face",
            &json!({"faces": ["2:bottom"], "distance": 5}),
        )
        .unwrap();
    assert!((exact_volume(&r) - 22_500.0).abs() < 1e-6);
}

/// A circle of radius `r` on the YZ plane at the origin (sketch 1) and the L path (sketch
/// 2, on XY): 30 along +x, a quarter arc of radius 20 about (30, 20), 30 along +y.
fn sweep_design(r: f64) -> (Session, Vec<u64>) {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "yz"})).unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": r}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let mut path = Vec::new();
    for (cmd, p) in [
        ("sketch.line", json!({"x1": 0, "y1": 0, "x2": 30, "y2": 0})),
        (
            "sketch.arc",
            json!({"cx": 30, "cy": 20, "x1": 30, "y1": 0, "x2": 50, "y2": 20}),
        ),
        (
            "sketch.line",
            json!({"x1": 50, "y1": 20, "x2": 50, "y2": 50}),
        ),
    ] {
        path.push(s.execute(cmd, &p).unwrap()["curve"].as_u64().unwrap());
    }
    s.execute("sketch.finish", &json!({})).unwrap();
    (s, path)
}

#[test]
fn sweeping_a_circle_along_an_l_path_gives_the_sum_of_its_parts() {
    let (mut s, path) = sweep_design(2.0);
    let r = s
        .execute(
            "solid.sweep",
            &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": 2, "path_curves": path}),
        )
        .unwrap();
    // π r² (30 + 10π + 30), Pappus for the quarter arc of radius 20.
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    let exact = std::f64::consts::PI * 4.0 * (60.0 + 10.0 * std::f64::consts::PI);
    assert!((v - exact).abs() < 1e-6 * exact, "{v} vs {exact}");
    assert!(s.feature_errors().is_empty());
    assert!(s.display_errors().is_empty(), "{:?}", s.display_errors());
    // Editing the path down to the first line leaves a cylinder of π r² 30.
    let id = r["feature"].as_u64().unwrap();
    let r = s
        .execute(
            "feature.edit",
            &json!({"feature": id, "path_curves": [path[0]]}),
        )
        .unwrap();
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - std::f64::consts::PI * 4.0 * 30.0).abs() < 1e-6, "{v}");
    // The design round-trips through a file.
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back, s.doc);
}

#[test]
fn a_sweep_that_cannot_be_built_changes_nothing() {
    // A profile of radius 25 cannot bend around the arc of radius 20.
    let (mut s, path) = sweep_design(25.0);
    let before = s.doc.clone();
    let undo = s.undo.len();
    let e = s
        .execute(
            "solid.sweep",
            &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": 2, "path_curves": path}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("radius"), "{e}");
    assert_eq!(s.doc, before);
    assert_eq!(s.undo.len(), undo);
    // A path curve that is not there, an empty path, a curve twice.
    for bad in [json!([999]), json!([]), json!([1, 1])] {
        assert!(
            s.execute(
                "solid.sweep",
                &json!({"sketch": 1, "profiles": [{"x": 0, "y": 0}], "path_sketch": 2, "path_curves": bad}),
            )
            .is_err()
        );
    }
    assert_eq!(s.doc, before);
}

#[test]
fn picking_one_curve_of_a_path_picks_its_chain() {
    let (s, path) = sweep_design(2.0);
    let f = s.doc.sketch(2).unwrap();
    let ids: Vec<u32> = path.iter().map(|p| *p as u32).collect();
    for c in &ids {
        assert_eq!(crate::solids::path_chain(&f.sketch, *c).len(), 3);
    }
}

#[test]
fn lofting_squares_and_circles_gives_frusta() {
    let mut s = Session::new();
    for (offset, half) in [(0.0, 10.0), (10.0, 5.0)] {
        s.execute("sketch.create", &json!({"plane": "xy", "offset": offset}))
            .unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": -half, "y1": -half, "x2": half, "y2": half}),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
    }
    for (offset, r) in [(0.0, 10.0), (10.0, 5.0)] {
        s.execute(
            "sketch.create",
            &json!({"plane": "xz", "offset": offset - 100.0}),
        )
        .unwrap();
        s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": r}))
            .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
    }
    let r = s
        .execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 1, "x": 0, "y": 0}, {"sketch": 2, "x": 0, "y": 0}]}),
        )
        .unwrap();
    // h/3 (A1 + A2 + √(A1 A2)) = 10/3 (400 + 100 + 200).
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    assert!((v - 7000.0 / 3.0).abs() < 1e-9, "{v}");
    let r = s
        .execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 3, "x": 0, "y": 0}, {"sketch": 4, "x": 0, "y": 0}]}),
        )
        .unwrap();
    // π h/3 (R² + R r + r²) with R = 10, r = 5, h = 10.
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    let exact = std::f64::consts::PI * 10.0 / 3.0 * 175.0;
    assert!((v - exact).abs() < 1e-9 * exact, "{v}");
    assert!(s.display_errors().is_empty(), "{:?}", s.display_errors());
    // A square to a circle has no exact surface yet: refused, nothing changes.
    let before = s.doc.clone();
    let undo = s.undo.len();
    assert!(
        s.execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 1, "x": 0, "y": 0}, {"sketch": 3, "x": 0, "y": 0}]}),
        )
        .is_err()
    );
    assert!(
        s.execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 1, "x": 0, "y": 0}]}),
        )
        .is_err()
    );
    assert_eq!(s.doc, before);
    assert_eq!(s.undo.len(), undo);
    // The sketches are in use by the lofts.
    assert!(!s.doc.sketch_users(2).is_empty());
}

#[test]
fn lofting_rings_gives_a_hollow_frustum() {
    // Circles R = 10 at z = 0 and 5 at z = 10, each around a coaxial hole r = 2 (the
    // profile seed between the circles): π h (R0² + R0 R1 + R1²) / 3 - π r² h
    // = 1750π / 3 - 40π = 1630π / 3.
    let mut s = Session::new();
    for (offset, r) in [(0.0, 10.0), (10.0, 5.0)] {
        s.execute("sketch.create", &json!({"plane": "xy", "offset": offset}))
            .unwrap();
        s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": r}))
            .unwrap();
        s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 2}))
            .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
    }
    let r = s
        .execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 1, "x": 3, "y": 0}, {"sketch": 2, "x": 3, "y": 0}]}),
        )
        .unwrap();
    let v = r["solids"][0]["volume"].as_f64().unwrap();
    let exact = 1630.0 * std::f64::consts::PI / 3.0;
    assert!((v - exact).abs() < 1e-9 * exact, "{v}");
    // A ring lofted to a plain disc is refused, naming the hole counts.
    let disc = s
        .execute("sketch.create", &json!({"plane": "xy", "offset": 20}))
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    s.execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 4}))
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    let before = s.doc.clone();
    let e = s
        .execute(
            "solid.loft",
            &json!({"sections": [{"sketch": 2, "x": 3, "y": 0}, {"sketch": disc, "x": 0, "y": 0}]}),
        )
        .unwrap_err();
    assert!(e.to_string().contains("same number of holes"), "{e}");
    assert_eq!(s.doc, before);
}

#[test]
fn the_swept_pipe_and_loft_example_has_its_exact_volumes() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/swept-pipe-and-loft.json"
    ))
    .unwrap();
    let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    for step in steps.as_array().unwrap() {
        let params = step.get("params").cloned().unwrap_or(json!({}));
        s.execute(step["command"].as_str().unwrap(), &params)
            .unwrap();
    }
    let pi = std::f64::consts::PI;
    let v: Vec<f64> = s
        .solids()
        .iter()
        .map(|x| x.body.volume().unwrap())
        .collect();
    // Pipe: annulus π(6² - 4²) along 40 + 15π + 40 (Pappus for the bend of radius 30).
    // Squares 40 -> 20 over 30 and 20 -> 30 over 20: 30/3 (1600 + 400 + 800) +
    // 20/3 (400 + 900 + 600). Cone r 15 -> 8 over 25: π 25/3 (225 + 120 + 64).
    let exact = [
        20.0 * pi * (80.0 + 15.0 * pi),
        28_000.0 + 38_000.0 / 3.0,
        pi * 25.0 / 3.0 * 409.0,
    ];
    assert_eq!(v.len(), 3);
    for (a, b) in v.iter().zip(exact) {
        assert!((a - b).abs() < 1e-6 * b, "{a} vs {b}");
    }
    assert!(s.display_errors().is_empty());
}

fn only_body(s: &Session) -> &chamfera_render::SceneBody {
    assert_eq!(s.scene.bodies.len(), 1);
    &s.scene.bodies[0]
}

#[test]
fn a_solid_shows_its_exact_edges_and_keeps_its_appearance_through_save_and_open() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    // A box has 12 B-rep edges, each drawn as its own polyline.
    assert_eq!(only_body(&s).edges.len(), 12);
    assert_eq!(
        only_body(&s).appearance,
        chamfera_render::Appearance::default()
    );
    let body = only_body(&s).id;
    let r = s
        .execute("body.appearance", &json!({"body": body, "preset": "glass"}))
        .unwrap();
    assert_eq!(r["feature"], 2);
    assert_eq!(r["saved"], true);
    // The glass preset: (214, 236, 240) at 30% opacity, stored for feature 2.
    let a = only_body(&s).appearance;
    assert_eq!(a.colour, Some(chamfera_render::Rgba(214, 236, 240, 255)));
    assert!((a.opacity - 0.3).abs() < 1e-6);
    assert!(s.can_undo());
    // A colour on its own keeps the opacity.
    let body = only_body(&s).id;
    s.execute(
        "body.appearance",
        &json!({"body": body, "colour": "#ff8000"}),
    )
    .unwrap();
    let a = only_body(&s).appearance;
    assert_eq!(a.colour, Some(chamfera_render::Rgba(255, 128, 0, 255)));
    assert!((a.opacity - 0.3).abs() < 1e-6);

    let dir = std::env::temp_dir().join(format!("chamfera-look-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("glass.chamfera");
    s.execute("design.save", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    let mut t = Session::new();
    t.execute("design.open", &json!({"path": path.to_str().unwrap()}))
        .unwrap();
    assert_eq!(t.doc, s.doc);
    let a = only_body(&t).appearance;
    assert_eq!(a.colour, Some(chamfera_render::Rgba(255, 128, 0, 255)));
    assert!((a.opacity - 0.3).abs() < 1e-6);
    std::fs::remove_dir_all(&dir).unwrap();

    // Undo goes back to plain glass; default clears it from the design.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(
        only_body(&s).appearance.colour,
        Some(chamfera_render::Rgba(214, 236, 240, 255))
    );
    let body = only_body(&s).id;
    s.execute(
        "body.appearance",
        &json!({"body": body, "preset": "default"}),
    )
    .unwrap();
    assert!(s.doc.appearances.is_empty());
    assert_eq!(
        only_body(&s).appearance,
        chamfera_render::Appearance::default()
    );
}

#[test]
fn bad_appearances_are_refused_and_change_nothing() {
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let body = only_body(&s).id;
    for p in [
        json!({"body": body}),
        json!({"body": body, "opacity": 0.0}),
        json!({"body": body, "opacity": 1.5}),
        json!({"body": body, "colour": "red"}),
        json!({"body": body, "preset": "unobtainium"}),
        json!({"body": 999, "opacity": 0.5}),
    ] {
        assert!(s.execute("body.appearance", &p).is_err(), "{p}");
    }
    assert!(s.doc.appearances.is_empty());
    assert!(!s.can_undo() || s.doc.appearances.is_empty());
}

#[test]
fn a_mesh_body_takes_an_appearance_for_the_session_only() {
    let mut s = Session::new();
    let body = s.execute("body.sample", &json!({})).unwrap()["body"]
        .as_u64()
        .unwrap();
    let r = s
        .execute("body.appearance", &json!({"body": body, "opacity": 0.5}))
        .unwrap();
    assert_eq!(r["saved"], false);
    assert!(s.doc.appearances.is_empty());
    let b = s.scene.body(body as u32).unwrap();
    assert!((b.appearance.opacity - 0.5).abs() < 1e-6);
    assert_eq!(b.appearance.colour, None);
}

#[test]
fn a_section_on_a_face_cuts_the_view_at_its_offset() {
    // The plate is 50 x 30 x 10 from the origin. A section on its top face (z = 10,
    // normal +Z) moved 4 mm down sits at z = 6 and cuts away everything above.
    let mut s = Session::new();
    extruded_plate(&mut s, json!(10));
    let r = s
        .execute("view.section", &json!({"plane": "2:top", "offset": -4}))
        .unwrap();
    assert!(
        (r["section"]["origin"]["z"].as_f64().unwrap() - 6.0).abs() < 1e-9,
        "{r}"
    );
    let sec = s.scene.display.section.unwrap();
    assert!((sec.normal.z - 1.0).abs() < 1e-9);
    // Looking straight down at the middle, the top is gone: the first face hit is the
    // bottom (z = 0), seen from inside.
    let ray = chamfera_geom::Ray::new(
        chamfera_geom::Point3::new(25.0, 15.0, 100.0),
        -chamfera_geom::Vec3::Z,
    )
    .unwrap();
    assert!(s.scene.pick(&ray).unwrap().point.z.abs() < 1e-9);
    // The design is untouched: a section is a display state, not a feature.
    assert!(!s.can_undo() || s.doc.features.len() == 1);
    assert_eq!(s.doc.features.len(), 1);
    // Flipped, it keeps the top 4 mm instead: the pick lands on the top face.
    s.execute(
        "view.section",
        &json!({"plane": "2:top", "offset": -4, "flip": true}),
    )
    .unwrap();
    assert!((s.scene.pick(&ray).unwrap().point.z - 10.0).abs() < 1e-9);
    // An origin plane: xz is the front plane, its normal along Y.
    s.execute("view.section", &json!({"plane": "xz", "offset": 15}))
        .unwrap();
    let sec = s.scene.display.section.unwrap();
    assert!((sec.normal.y.abs() - 1.0).abs() < 1e-9);
    assert!((sec.origin.y.abs() - 15.0).abs() < 1e-9);
    for bad in [
        json!({"plane": "diagonal"}),
        json!({"plane": "9:nowhere"}),
        json!({"plane": "xy", "offset": 1e300}),
        json!({"plane": "xy", "flip": "yes"}),
    ] {
        assert!(s.execute("view.section", &bad).is_err(), "{bad}");
    }
    let r = s.execute("view.section_clear", &json!({})).unwrap();
    assert!(r["section"].is_null());
    assert!(s.scene.display.section.is_none());
}

#[test]
fn the_visual_style_is_set_by_name_and_reported_by_view_get() {
    let mut s = Session::new();
    for name in [
        "shaded",
        "shaded_edges",
        "shaded_hidden_edges",
        "wireframe",
        "wireframe_hidden",
    ] {
        s.execute("view.style", &json!({"style": name})).unwrap();
        assert_eq!(
            s.execute("view.get", &json!({})).unwrap()["display"]["style"],
            name
        );
    }
    assert_eq!(
        s.scene.display.style,
        chamfera_render::VisualStyle::WireframeHidden
    );
    assert!(
        s.execute("view.style", &json!({"style": "cartoon"}))
            .is_err()
    );
    assert_eq!(
        s.scene.display.style,
        chamfera_render::VisualStyle::WireframeHidden
    );
}

/// Two 40 x 40 squares overlapping by 20 x 20, drawn and extruded 10 without splitting:
/// the overlap is 20 x 20 x 10 = 4000 mm³ and the L-shaped rest of the first square
/// (1600 - 400) x 10 = 12000 mm³.
#[test]
fn overlapping_rectangles_extrude_their_regions_without_a_split() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    for (x1, y1, x2, y2) in [(0, 0, 40, 40), (20, 20, 60, 60)] {
        s.execute(
            "sketch.rectangle",
            &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2}),
        )
        .unwrap();
    }
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let mut areas: Vec<f64> = g["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["area"].as_f64().unwrap())
        .collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 3, "{areas:?}");
    for (a, want) in areas.iter().zip([400.0, 1200.0, 1200.0]) {
        assert!((a - want).abs() < 1e-6, "{areas:?}");
    }
    s.execute("sketch.finish", &json!({})).unwrap();
    for (x, y, want) in [(30.0, 30.0, 4000.0), (10.0, 10.0, 12000.0)] {
        let r = s
            .execute(
                "solid.extrude",
                &json!({"sketch": 1, "profiles": [{"x": x, "y": y}], "distance": 10}),
            )
            .unwrap();
        let v: f64 = r["solids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["volume"].as_f64().unwrap())
            .fold(0.0, f64::max);
        assert!((v - want).abs() < 1e-6, "{r}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn concentric_collinear_and_point_alignment_from_the_selection() {
    use crate::sketching::SketchItem;
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let c1 = s
        .execute("sketch.circle", &json!({"cx": 0, "cy": 0, "radius": 10}))
        .unwrap();
    let c2 = s
        .execute("sketch.circle", &json!({"cx": 6, "cy": 2, "radius": 4}))
        .unwrap();
    let id = |v: &serde_json::Value, k: &str| v[k].as_u64().unwrap() as u32;
    s.sketch_selection = vec![
        SketchItem::Curve(id(&c1, "curve")),
        SketchItem::Curve(id(&c2, "curve")),
    ];
    s.execute("sketch.constrain_selection", &json!({"kind": "concentric"}))
        .unwrap();
    let at = |s: &mut Session, p: u32| {
        let g = s.execute("sketch.get", &json!({})).unwrap();
        let q = g["points"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["point"] == p)
            .unwrap()
            .clone();
        (q["x"].as_f64().unwrap(), q["y"].as_f64().unwrap())
    };
    let (a, b) = (at(&mut s, id(&c1, "centre")), at(&mut s, id(&c2, "centre")));
    assert!((a.0 - b.0).abs() < 1e-7 && (a.1 - b.1).abs() < 1e-7);
    // Two lines made collinear: the second's ends land on the first's line.
    let l1 = s
        .execute(
            "sketch.line",
            &json!({"x1": 20, "y1": 0, "x2": 40, "y2": 0}),
        )
        .unwrap();
    let l2 = s
        .execute(
            "sketch.line",
            &json!({"x1": 50, "y1": 3, "x2": 70, "y2": 5}),
        )
        .unwrap();
    s.execute(
        "sketch.constrain",
        &json!({"constraint": {"kind": "horizontal", "line": id(&l1, "curve")}}),
    )
    .unwrap();
    s.sketch_selection = vec![
        SketchItem::Curve(id(&l1, "curve")),
        SketchItem::Curve(id(&l2, "curve")),
    ];
    s.execute("sketch.constrain_selection", &json!({"kind": "collinear"}))
        .unwrap();
    let y1 = at(&mut s, id(&l1, "start")).1;
    let y2 = at(&mut s, id(&l2, "start")).1;
    let y3 = at(&mut s, id(&l2, "end")).1;
    assert!(
        (y1 - y2).abs() < 1e-7 && (y1 - y3).abs() < 1e-7,
        "{y1} {y2} {y3}"
    );
    // Two points made level.
    let p = s
        .execute("sketch.point", &json!({"x": 80, "y": 0}))
        .unwrap();
    let q = s
        .execute("sketch.point", &json!({"x": 90, "y": 7}))
        .unwrap();
    s.sketch_selection = vec![
        SketchItem::Point(id(&p, "point")),
        SketchItem::Point(id(&q, "point")),
    ];
    s.execute("sketch.constrain_selection", &json!({"kind": "horizontal"}))
        .unwrap();
    let (pp, qq) = (at(&mut s, id(&p, "point")), at(&mut s, id(&q, "point")));
    assert!((pp.1 - qq.1).abs() < 1e-7);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert!(
        g["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["definition"]["kind"] == "horizontal_points")
    );
    // Concentric needs circles or arcs.
    s.sketch_selection = vec![
        SketchItem::Curve(id(&l1, "curve")),
        SketchItem::Curve(id(&c1, "curve")),
    ];
    assert!(
        s.execute("sketch.constrain_selection", &json!({"kind": "concentric"}))
            .is_err()
    );
}

/// Points (0, 0) and (30, 40): placed above them (inside their X span) a dimension
/// measures X, 30; to their right (inside the Y span) Y, 40; off a corner the distance, 50.
#[test]
fn a_dimension_measures_the_way_it_is_placed() {
    use crate::sketching::SketchItem;
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let l = s
        .execute(
            "sketch.line",
            &json!({"x1": 0, "y1": 0, "x2": 30, "y2": 40}),
        )
        .unwrap();
    let (a, b) = (
        l["start"].as_u64().unwrap() as u32,
        l["end"].as_u64().unwrap() as u32,
    );
    for (at, kind, want) in [
        (json!({"x": 15, "y": 50}), "horizontal_distance", 30.0),
        (json!({"x": 45, "y": 20}), "vertical_distance", 40.0),
        (json!({"x": 45, "y": 50}), "distance", 50.0),
        (json!({"direction": "vertical"}), "vertical_distance", 40.0),
    ] {
        s.sketch_selection = vec![SketchItem::Point(a), SketchItem::Point(b)];
        let d = s.execute("sketch.dimension_selection", &at).unwrap();
        assert_eq!(d["kind"], kind, "{d}");
        assert!((d["value"].as_f64().unwrap() - want).abs() < 1e-9, "{d}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
    // The line itself placed above it: its horizontal extent.
    s.sketch_selection = vec![SketchItem::Curve(l["curve"].as_u64().unwrap() as u32)];
    let d = s
        .execute("sketch.dimension_selection", &json!({"x": 10, "y": 60}))
        .unwrap();
    assert_eq!(d["kind"], "horizontal_distance");
    assert!((d["value"].as_f64().unwrap() - 30.0).abs() < 1e-9, "{d}");
    s.sketch_selection = vec![SketchItem::Point(a), SketchItem::Point(b)];
    assert!(
        s.execute("sketch.dimension_selection", &json!({"direction": "up"}))
            .is_err()
    );
}

/// The 50 x 30 rectangle is fully constrained, so a dimension across its diagonal
/// (√(2500 + 900) = √3400) is driven: it shows the size, and follows the width when it
/// changes to 40 (√(1600 + 900) = 50). Driving again, it over-constrains.
#[test]
fn a_dimension_that_would_over_constrain_becomes_driven() {
    use crate::sketching::SketchItem;
    let mut s = Session::new();
    let (pts, lines) = dimensioned_rectangle(&mut s);
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["status"], "FullyConstrained", "{g}");
    s.sketch_selection = vec![
        SketchItem::Point(pts[0] as u32),
        SketchItem::Point(pts[2] as u32),
    ];
    let d = s.execute("sketch.dimension_selection", &json!({})).unwrap();
    assert_eq!(d["driven"], true, "{d}");
    assert!((d["value"].as_f64().unwrap() - 3400f64.sqrt()).abs() < 1e-9);
    let diagonal = d["constraint"].as_u64().unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["status"], "FullyConstrained");
    let width = g["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["definition"]["kind"] == "length" && c["definition"]["line"] == lines[0])
        .unwrap()["constraint"]
        .clone();
    s.execute(
        "sketch.dimension",
        &json!({"constraint": width, "value": 40}),
    )
    .unwrap();
    let labels = s.dimension_labels();
    let l = labels
        .iter()
        .find(|l| u64::from(l.constraint) == diagonal)
        .unwrap();
    assert!(l.driven && (l.value - 50.0).abs() < 1e-7, "{l:?}");
    // A driven dimension takes no new value.
    assert!(
        s.execute(
            "sketch.dimension",
            &json!({"constraint": diagonal, "value": 10})
        )
        .is_err()
    );
    s.execute(
        "sketch.dimension.driven",
        &json!({"constraint": diagonal, "driven": false}),
    )
    .unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["status"], "OverConstrained");
    s.execute("edit.undo", &json!({})).unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert_eq!(g["status"], "FullyConstrained");
}

/// An arc of r = 10 from angle 0 to π/2 is 5π long.
#[test]
fn an_arc_can_be_dimensioned_by_its_length() {
    use crate::sketching::SketchItem;
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let a = s
        .execute(
            "sketch.arc",
            &json!({"cx": 0, "cy": 0, "x1": 10, "y1": 0, "x2": 0, "y2": 10}),
        )
        .unwrap();
    s.sketch_selection = vec![SketchItem::Curve(a["curve"].as_u64().unwrap() as u32)];
    let d = s
        .execute("sketch.dimension_selection", &json!({"arc_length": true}))
        .unwrap();
    assert_eq!(d["kind"], "arc_length");
    assert!((d["value"].as_f64().unwrap() - 5.0 * std::f64::consts::PI).abs() < 1e-9);
}

/// A 3-point rectangle with side (0, 0)-(30, 0) and height 20: area 600. A 2-point circle
/// across (0, 50)-(0, 70) has r = 10: area 100π. A 3-point circle through (100, 0),
/// (120, 0), (100, 20) has the right angle at (100, 0), so its hypotenuse (20√2) is a
/// diameter: r² = 200, area 200π.
#[test]
fn rectangles_and_circles_by_points() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle.three_point",
        &json!({"x1": 0, "y1": 0, "x2": 30, "y2": 0, "x3": 12, "y3": 20}),
    )
    .unwrap();
    s.execute(
        "sketch.circle.two_point",
        &json!({"x1": 0, "y1": 50, "x2": 0, "y2": 70}),
    )
    .unwrap();
    s.execute(
        "sketch.circle.three_point",
        &json!({"x1": 100, "y1": 0, "x2": 120, "y2": 0, "x3": 100, "y3": 20}),
    )
    .unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let mut areas: Vec<f64> = g["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["area"].as_f64().unwrap())
        .collect();
    areas.sort_by(f64::total_cmp);
    let pi = std::f64::consts::PI;
    assert_eq!(areas.len(), 3, "{areas:?}");
    for (a, want) in areas.iter().zip([100.0 * pi, 600.0, 200.0 * pi]) {
        assert!((a - want).abs() < 1e-6, "{areas:?}");
    }
    assert!(
        s.execute(
            "sketch.circle.three_point",
            &json!({"x1": 0, "y1": 0, "x2": 1, "y2": 1, "x3": 2, "y3": 2}),
        )
        .is_err()
    );
}

/// The corner of two lines along the axes, radius 5: the tangent circle's centre is at
/// (5, 5).
#[test]
fn a_tangent_circle_touches_the_curves_it_is_given() {
    let mut s = Session::new();
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    let x = s
        .execute("sketch.line", &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 0}))
        .unwrap();
    let y = s
        .execute("sketch.line", &json!({"x1": 0, "y1": 0, "x2": 0, "y2": 50}))
        .unwrap();
    for p in [&x["start"], &x["end"], &y["start"], &y["end"]] {
        let g = s.execute("sketch.get", &json!({})).unwrap();
        let q = g["points"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["point"] == *p)
            .unwrap()
            .clone();
        s.execute(
            "sketch.constrain",
            &json!({"constraint": {"kind": "fix", "point": p, "at": {"x": q["x"], "y": q["y"]}}}),
        )
        .unwrap();
    }
    let r = s
        .execute(
            "sketch.circle.tangent",
            &json!({"curves": [x["curve"], y["curve"]], "x": 4, "y": 7, "radius": 5}),
        )
        .unwrap();
    let circle = r["created_curves"][0].as_u64().unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let def = g["curves"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["curve"] == circle)
        .unwrap()["geometry"]
        .clone();
    let centre = g["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["point"] == def["centre"])
        .unwrap()
        .clone();
    assert!(
        (centre["x"].as_f64().unwrap() - 5.0).abs() < 1e-7,
        "{centre}"
    );
    assert!(
        (centre["y"].as_f64().unwrap() - 5.0).abs() < 1e-7,
        "{centre}"
    );
    assert!(
        s.execute(
            "sketch.circle.tangent",
            &json!({"curves": [x["curve"]], "x": 4, "y": 7, "radius": 5}),
        )
        .is_err()
    );
}

/// A 20 x 10 rectangle at the origin, selected: moved by (5, 5) its corners span (5, 5)
/// to (25, 15); copied 30 to the right there are two 200 mm² profiles; scaled by 3 about
/// the origin it is 60 x 30 = 1800; turned a quarter about the origin the box is
/// x -10..0, y 0..20. Each is one undo step.
#[test]
fn selected_sketch_geometry_moves_turns_scales_and_copies() {
    use crate::sketching::SketchItem;
    let new_plate = || {
        let mut s = Session::new();
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        let r = s
            .execute(
                "sketch.rectangle",
                &json!({"x1": 0, "y1": 0, "x2": 20, "y2": 10}),
            )
            .unwrap();
        s.sketch_selection = r["curves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| SketchItem::Curve(c.as_u64().unwrap() as u32))
            .collect();
        s
    };
    let bounds = |s: &mut Session| {
        let g = s.execute("sketch.get", &json!({})).unwrap();
        let pts = g["points"].as_array().unwrap().clone();
        let xs: Vec<f64> = pts.iter().map(|p| p["x"].as_f64().unwrap()).collect();
        let ys: Vec<f64> = pts.iter().map(|p| p["y"].as_f64().unwrap()).collect();
        let lo = |v: &[f64]| v.iter().copied().fold(f64::MAX, f64::min);
        let hi = |v: &[f64]| v.iter().copied().fold(f64::MIN, f64::max);
        [lo(&xs), lo(&ys), hi(&xs), hi(&ys)]
    };
    let close = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-7);
    let mut s = new_plate();
    s.execute("sketch.move", &json!({"dx": 5, "dy": 5}))
        .unwrap();
    assert!(close(bounds(&mut s), [5.0, 5.0, 25.0, 15.0]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&mut s), [0.0, 0.0, 20.0, 10.0]));

    let mut s = new_plate();
    s.execute("sketch.move", &json!({"dx": 30, "dy": 0, "copy": true}))
        .unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    let areas: Vec<f64> = g["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["area"].as_f64().unwrap())
        .collect();
    assert_eq!(areas.len(), 2);
    assert!(areas.iter().all(|a| (a - 200.0).abs() < 1e-6), "{areas:?}");

    let mut s = new_plate();
    s.execute("sketch.scale", &json!({"cx": 0, "cy": 0, "factor": 3}))
        .unwrap();
    let g = s.execute("sketch.get", &json!({})).unwrap();
    assert!((g["profiles"][0]["area"].as_f64().unwrap() - 1800.0).abs() < 1e-6);

    let mut s = new_plate();
    s.execute(
        "sketch.rotate",
        &json!({"cx": 0, "cy": 0, "angle": std::f64::consts::FRAC_PI_2}),
    )
    .unwrap();
    assert!(close(bounds(&mut s), [-10.0, 0.0, 0.0, 20.0]));

    let mut s = new_plate();
    s.sketch_selection.clear();
    assert!(
        s.execute("sketch.move", &json!({"dx": 1, "dy": 0}))
            .is_err()
    );
}

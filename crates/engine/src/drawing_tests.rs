//! Drawings version 2 through commands: associative dimensions, sections of the hole
//! plate, details, annotations, the parts list and balloons of the hinge, exports and
//! hostile input. Every expected value is derived by hand in the comment next to it.

use crate::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p)
        .unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn example(name: &str) -> Session {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(path).unwrap();
    let steps: Value = serde_json::from_str(&text).unwrap();
    let mut s = Session::new();
    crate::script::run_steps(&mut s, steps.as_array().unwrap()).unwrap();
    s
}

/// A box `width` (a parameter) x 30 x 10 from the origin, its sketch's bottom line held
/// at the length `width` and its first corner fixed at the origin.
fn parametric_box(s: &mut Session) {
    run(
        s,
        "param.set",
        json!({"name": "width", "expression": "50 mm"}),
    );
    let sk = run(s, "sketch.create", json!({"plane": "xy"}))["sketch"]
        .as_u64()
        .unwrap();
    let r = run(
        s,
        "sketch.rectangle",
        json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    );
    // The rectangle's first line runs from its first corner (0, 0) to (50, 0).
    let line = r["curves"][0].as_u64().unwrap();
    let corner = r["points"][0].as_u64().unwrap();
    run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "fix", "point": corner, "at": {"x": 0, "y": 0}}}),
    );
    let c = run(
        s,
        "sketch.constrain",
        json!({"constraint": {"kind": "length", "line": line, "value": 50}}),
    );
    let cid = c["constraint"]
        .as_u64()
        .or_else(|| c["constraints"][0].as_u64())
        .unwrap();
    run(
        s,
        "sketch.dimension",
        json!({"constraint": cid, "expression": "width"}),
    );
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sk, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
    );
}

fn dimension(s: &mut Session, id: u64) -> Value {
    let l = run(s, "drawing.list", json!({}));
    l["sheets"][0]["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"].as_u64() == Some(id))
        .cloned()
        .unwrap()
}

#[test]
fn a_dimension_of_a_50_wide_box_reads_60_after_the_box_is_edited_to_60() {
    let mut s = Session::new();
    parametric_box(&mut s);
    let v = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "front", "x": 150, "y": 150}),
    )["view"]
        .as_u64()
        .unwrap();
    // The front view puts the box's centre (25, 5) at (150, 150): its bottom corners are
    // at (125, 145) and (175, 145).
    let d = run(
        &mut s,
        "drawing.add_dimension",
        json!({"view": v, "kind": "horizontal", "x1": 125.3, "y1": 145.2, "x2": 174.8, "y2": 145.1, "offset": -10}),
    );
    assert!((d["value"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(d["associative"], true, "{d}");
    let id = d["dimension"].as_u64().unwrap();
    // The box made 60 wide: the corners are 60 apart, so the dimension reads 60.000.
    run(
        &mut s,
        "param.set",
        json!({"name": "width", "expression": "60 mm"}),
    );
    let now = dimension(&mut s, id);
    assert!(
        (now["value"].as_f64().unwrap() - 60.0).abs() < 1e-9,
        "{now}"
    );
    assert_eq!(format!("{:.3}", now["value"].as_f64().unwrap()), "60.000");
    assert_eq!(now["text"], "60");
    assert!(now["dangling"].is_null());
    // The design saves and reopens with its references.
    let dir = std::env::temp_dir().join(format!("chamfera-dims-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("box.chamfera");
    run(
        &mut s,
        "design.save",
        json!({"path": path.to_str().unwrap()}),
    );
    let mut t = Session::new();
    run(
        &mut t,
        "design.open",
        json!({"path": path.to_str().unwrap()}),
    );
    assert_eq!(t.doc.drawing, s.doc.drawing);
    assert_eq!(t.doc.version, chamfera_doc::FORMAT_VERSION);
    assert!(!t.doc.drawing.sheets[0].dimensions[0].refs.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Views, dimensions and annotations can be picked, moved and deleted (QA user guide
/// pass, drawings 9). The front view of the 50 x 30 x 10 box at (150, 150) spans
/// x 125..175 and y 145..155; a horizontal dimension of its bottom edge with offset -10
/// has its line at y = 145 - 10 = 135.
#[test]
fn drawing_items_are_picked_moved_and_deleted_through_commands() {
    let mut s = Session::new();
    parametric_box(&mut s);
    let front = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "front", "x": 150, "y": 150}),
    )["view"]
        .as_u64()
        .unwrap();
    // The top view 80 above, projected from the front view.
    let top = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 230}),
    )["view"]
        .as_u64()
        .unwrap();
    let d = run(
        &mut s,
        "drawing.add_dimension",
        json!({"view": front, "kind": "horizontal", "x1": 125.3, "y1": 145.2, "x2": 174.8, "y2": 145.1, "offset": -10}),
    )["dimension"]
        .as_u64()
        .unwrap();
    let note = run(
        &mut s,
        "drawing.add_note",
        json!({"x": 300, "y": 100, "text": "NOTE"}),
    )["annotation"]
        .as_u64()
        .unwrap();
    let pick = |s: &mut Session, x: f64, y: f64| run(s, "drawing.pick", json!({"x": x, "y": y}));
    // The dimension line at y = 135 (x 140 is between its arrows).
    let r = pick(&mut s, 140.0, 135.3);
    assert_eq!(
        (r["kind"].as_str(), r["id"].as_u64()),
        (Some("dimension"), Some(d))
    );
    // The note's text starts at (300, 100) and is 3.5 tall: (302, 101) is inside it.
    let r = pick(&mut s, 302.0, 101.0);
    assert_eq!(
        (r["kind"].as_str(), r["id"].as_u64()),
        (Some("annotation"), Some(note))
    );
    // Inside the front view's box, away from the dimension: the view.
    let r = pick(&mut s, 160.0, 150.0);
    assert_eq!(
        (r["kind"].as_str(), r["id"].as_u64()),
        (Some("view"), Some(front))
    );
    assert!(pick(&mut s, 60.0, 260.0)["kind"].is_null());
    // Dragging the dimension 5 further down: its line at y = 130, offset -15.
    let r = run(
        &mut s,
        "drawing.move",
        json!({"dimension": d, "dx": 0, "dy": -5}),
    );
    assert!((r["offset"].as_f64().unwrap() + 15.0).abs() < 1e-9, "{r}");
    // Dragged up across the edge (to y = 150, 5 above the measured points at y = 145):
    // the line flips above them, offset +5.
    let r = run(
        &mut s,
        "drawing.move",
        json!({"dimension": d, "dx": 0, "dy": 20}),
    );
    assert!((r["offset"].as_f64().unwrap() - 5.0).abs() < 1e-9, "{r}");
    // The note 10 right; the front view 20 right takes the projected top view along.
    run(
        &mut s,
        "drawing.move",
        json!({"annotation": note, "dx": 10, "dy": 0}),
    );
    run(
        &mut s,
        "drawing.move",
        json!({"view": front, "dx": 20, "dy": 0}),
    );
    let l = run(&mut s, "drawing.list", json!({}));
    let views = l["sheets"][0]["views"].as_array().unwrap();
    let at = |id: u64| {
        let v = views.iter().find(|v| v["id"].as_u64() == Some(id)).unwrap();
        (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap())
    };
    assert_eq!(at(front), (170.0, 150.0));
    // A view without a section hatches nothing: 0, not -0 (QA guide pass, drawings 12).
    let hatched = views[0]["hatched_area"].as_f64().unwrap();
    assert!(hatched == 0.0 && hatched.is_sign_positive(), "{hatched}");
    assert_eq!(at(top), (170.0, 230.0));
    let sh = &s.doc.drawing.sheets[0];
    assert!(matches!(
        sh.annotations[0].kind,
        chamfera_doc::drawing::AnnotationKind::Note { at, .. } if (at.x - 310.0).abs() < 1e-9
    ));
    // One undo puts the view back; moves of nothing are refused and change nothing.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.drawing.sheets[0].views[0].x, 150.0);
    let before = s.doc.clone();
    assert!(
        s.execute("drawing.move", &json!({"view": 99, "dx": 1, "dy": 0}))
            .is_err()
    );
    assert!(
        s.execute(
            "drawing.move",
            &json!({"view": front, "dx": f64::NAN, "dy": 0})
        )
        .is_err()
    );
    assert!(
        s.execute("drawing.move", &json!({"view": front, "dx": 1e9, "dy": 0}))
            .is_err()
    );
    assert_eq!(s.doc, before);
    // Delete by what the pick named.
    run(&mut s, "drawing.delete", json!({"dimension": d}));
    assert!(pick(&mut s, 140.0, 150.3)["kind"] != "dimension");
}

#[test]
fn a_dimension_whose_edge_is_gone_is_dangling_and_kept() {
    let mut s = Session::new();
    parametric_box(&mut s);
    let v = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "front", "x": 150, "y": 150}),
    )["view"]
        .as_u64()
        .unwrap();
    // The right edge of the front view, from (175, 145) to (175, 155): 10 high.
    let d = run(
        &mut s,
        "drawing.add_dimension",
        json!({"view": v, "kind": "vertical", "x1": 175.1, "y1": 145.2, "x2": 174.9, "y2": 154.8}),
    );
    assert!((d["value"].as_f64().unwrap() - 10.0).abs() < 1e-9);
    assert_eq!(d["associative"], true);
    let id = d["dimension"].as_u64().unwrap();
    // Rounding the four vertical edges takes away the corner edges and their ends that
    // the dimension measured.
    run(
        &mut s,
        "solid.fillet",
        json!({"edges": ["2:sweep 0", "2:sweep 1", "2:sweep 2", "2:sweep 3"], "radius": 3}),
    );
    let now = dimension(&mut s, id);
    assert!(now["dangling"].is_string(), "{now}");
    // It keeps its old points and value, drawn on the dangling layer.
    assert!((now["value"].as_f64().unwrap() - 10.0).abs() < 1e-9);
    let g = crate::cmd::drawing::render(&s, s.doc.drawing.sheets[0].id).unwrap();
    assert!(
        g.items
            .iter()
            .any(|i| i.layer == chamfera_drawing::Layer::Dangling)
    );
    assert_eq!(s.doc.drawing.sheets[0].dimensions.len(), 1);
    // Undoing the fillet brings the edge back and the dimension follows it again.
    run(&mut s, "edit.undo", json!({}));
    assert!(dimension(&mut s, id)["dangling"].is_null());
}

#[test]
fn a_section_through_the_middle_of_the_hole_plate_hatches_the_area_derived_by_hand() {
    let mut s = example("hole-plate.json");
    let top = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 200}),
    )["view"]
        .as_u64()
        .unwrap();
    // The 80 x 50 x 10 plate's centre (40, 25) is at (150, 200) on the top view, so the
    // middle line y = 25 is the sheet line y = 200 from x = 110 to 190. Drawn right to
    // left, the arrows point up the paper: the section looks along +Y like a front view.
    let r = run(
        &mut s,
        "drawing.add_section",
        json!({"view": top, "x1": 195, "y1": 200, "x2": 105, "y2": 200, "x": 150, "y": 90}),
    );
    assert_eq!(r["label"], "A");
    // The plane y = 25 cuts the 80 x 10 section (800 mm²) through the axes of three
    // holes (the counterbored ones at y = 10 and 40 reach only 15.61 and 34.39):
    // - the countersunk hole at x = 40: Ø6 through (6 x 10 = 60) and the 90° sink from
    //   Ø12 at the top to Ø6 at depth (12 - 6) / 2 = 3: a trapezoid (12 + 6) / 2 x 3 = 27,
    //   of which 6 x 3 = 18 is in the bore: 69 in all;
    // - the two blind holes Ø5, 6 deep, at x = 25 and 55: 5 x 6 = 30 each, plus the
    //   118° drill point, a triangle 5 wide and 2.5 / tan 59° deep: 6.25 / tan 59°.
    // 800 - 69 - 2 (30 + 6.25 / tan 59°) = 671 - 12.5 / tan 59°.
    let want = 671.0 - 12.5 / 59f64.to_radians().tan();
    let got = r["hatched_area"].as_f64().unwrap();
    assert!((got - want).abs() < 1e-6, "{got} vs {want}");
    // 80 wide and 10 high on the sheet at 1:1.
    let b: Vec<f64> = r["bounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    assert!((b[2] - b[0] - 80.0).abs() < 1e-6 && (b[3] - b[1] - 10.0).abs() < 1e-6);
    // The section is the front view's direction, and a second one takes the next label.
    let sheet = &s.doc.drawing.sheets[0];
    assert_eq!(
        sheet.views[1].direction,
        chamfera_doc::drawing::ViewDirection::Front
    );
    let r2 = run(
        &mut s,
        "drawing.add_section",
        json!({"view": top, "x1": 150, "y1": 230, "x2": 150, "y2": 170, "x": 300, "y": 200}),
    );
    assert_eq!(r2["label"], "B");
    // A cutting line beside the part leaves nothing to show: refused, nothing changed.
    let before = s.doc.clone();
    assert!(
        s.execute(
            "drawing.add_section",
            &json!({"view": top, "x1": 195, "y1": 260, "x2": 105, "y2": 260, "x": 150, "y": 40}),
        )
        .is_err()
    );
    assert_eq!(s.doc, before);
}

#[test]
fn a_detail_annotations_and_exports_of_the_hole_plate() {
    let mut s = example("hole-plate.json");
    let top = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 200}),
    )["view"]
        .as_u64()
        .unwrap();
    // The countersunk hole at (40, 25) is at the top view's centre (150, 200); its Ø12
    // mouth is a circle of radius 6. A detail of radius 10 about it at 2:1 draws that
    // circle at radius 12 (Ø24) about the detail's centre.
    let d = run(
        &mut s,
        "drawing.add_detail",
        json!({"view": top, "cx": 150, "cy": 200, "radius": 10, "x": 330, "y": 200}),
    );
    assert_eq!(d["scale"].as_f64(), Some(2.0));
    let det = d["view"].as_u64().unwrap();
    let g = crate::cmd::drawing::render(&s, s.doc.drawing.sheets[0].id).unwrap();
    assert!(g.items.iter().any(|i| i.layer == chamfera_drawing::Layer::Visible
        && matches!(i.shape,
            chamfera_drawing::Shape::Arc { centre, radius, .. } | chamfera_drawing::Shape::Circle { centre, radius }
            if centre.distance(chamfera_geom::Point2::new(330.0, 200.0)) < 1e-6 && (radius - 12.0).abs() < 1e-6)));
    // Annotations.
    let n = run(
        &mut s,
        "drawing.add_note",
        json!({"x": 30, "y": 60, "text": "BREAK SHARP EDGES 0.5"}),
    );
    assert!(n["annotation"].is_u64());
    run(
        &mut s,
        "drawing.add_leader",
        json!({"points": [{"x": 150, "y": 206}, {"x": 175, "y": 240}], "text": "Ø6 THRU"}),
    );
    run(
        &mut s,
        "drawing.add_surface_finish",
        json!({"x": 120, "y": 225, "text": "Ra 3.2"}),
    );
    run(
        &mut s,
        "drawing.add_gdt",
        json!({"characteristic": "position", "tolerance": 0.1, "diameter": true,
               "datums": ["A", "B"], "x": 200, "y": 120, "leader_x": 160, "leader_y": 200}),
    );
    run(
        &mut s,
        "drawing.add_gdt",
        json!({"characteristic": "flatness", "tolerance": 0.05, "x": 200, "y": 105}),
    );
    // A centre line between the two blind holes (x = 25 and 55, at (135, 200) and
    // (165, 200) on the sheet): 30 long.
    let c = run(
        &mut s,
        "drawing.add_centre_line",
        json!({"view": top, "x1": 137.5, "y1": 200, "x2": 167.5, "y2": 200}),
    );
    assert!((c["length"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{c}");
    assert_eq!(c["associative"], true);
    for bad in [
        json!({"characteristic": "flatness", "tolerance": 0.05, "x": 0, "y": 0, "datums": ["A"]}),
        json!({"characteristic": "perpendicularity", "tolerance": 0.05, "x": 0, "y": 0}),
        json!({"characteristic": "wobble", "tolerance": 0.05, "x": 0, "y": 0}),
        json!({"characteristic": "position", "tolerance": -1, "x": 0, "y": 0, "datums": ["A"]}),
        json!({"characteristic": "position", "tolerance": 1, "x": 0, "y": 0, "datums": ["a"]}),
    ] {
        assert!(s.execute("drawing.add_gdt", &bad).is_err(), "{bad}");
    }
    // Exports hold the new entities.
    let svg = run(&mut s, "drawing.export_bytes", json!({"format": "svg"}));
    let bytes = decode(svg["data"].as_str().unwrap());
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("BREAK SHARP EDGES 0.5") && text.contains("Ra 3.2"));
    assert!(text.contains("DETAIL A (2:1)"), "the first label is A");
    let dxf = run(&mut s, "drawing.export_bytes", json!({"format": "dxf"}));
    let text = String::from_utf8(decode(dxf["data"].as_str().unwrap())).unwrap();
    assert!(text.contains("\n%%c0.1\n") && text.contains("\nCENTER\n"));
    let pdf = run(&mut s, "drawing.export_bytes", json!({"format": "pdf"}));
    let bytes = decode(pdf["data"].as_str().unwrap());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("%PDF-1.4") && text.contains("(Ra 3.2) Tj"));
    // Deleting the top view takes its detail and its view annotations.
    run(&mut s, "drawing.delete", json!({"view": top}));
    let sh = &s.doc.drawing.sheets[0];
    assert!(sh.view(det as u32).is_none());
    assert!(sh.annotations.iter().all(|a| a.kind.view().is_none()));
    assert_eq!(sh.annotations.len(), 5);
}

fn decode(b64: &str) -> Vec<u8> {
    // RFC 4648 base64, as `drawing.export_bytes` writes it.
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let clean: Vec<u8> = b64.bytes().filter(|c| *c != b'=').collect();
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let v: Vec<u32> = chunk.iter().map(|c| u32::from(val(*c))).collect();
        let n = v.iter().fold(0u32, |a, x| (a << 6) | x) << (6 * (4 - v.len()));
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..v.len().saturating_sub(1)]);
    }
    out
}

#[test]
fn the_parts_list_of_the_hinge_has_two_rows_of_one_and_balloons() {
    let mut s = example("hinge-assembly.json");
    let v = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 160}),
    )["view"]
        .as_u64()
        .unwrap();
    let p = run(&mut s, "drawing.add_parts_list", json!({}));
    // Two components, Leaf A and Leaf B, each once.
    let rows = p["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{p}");
    assert_eq!(rows[0]["name"], "Leaf A");
    assert_eq!(rows[1]["name"], "Leaf B");
    assert!(rows.iter().all(|r| r["quantity"] == 1));
    assert_eq!(rows[0]["item"], 1);
    let b = run(&mut s, "drawing.add_balloons", json!({"view": v}));
    let balloons = b["balloons"].as_array().unwrap();
    assert_eq!(balloons.len(), 2);
    let l = run(&mut s, "drawing.list", json!({}));
    let sheet = &l["sheets"][0];
    assert_eq!(sheet["parts_list"]["rows"].as_array().unwrap().len(), 2);
    assert!(
        sheet["annotations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["dangling"].is_null())
    );
    // Again replaces the view's balloons instead of doubling them.
    run(&mut s, "drawing.add_balloons", json!({"view": v}));
    assert_eq!(s.doc.drawing.sheets[0].annotations.len(), 2);
    // The parts list sits on the title block's top edge at the frame's right margin:
    // (420 - 10, 10 + 30).
    let pl = s.doc.drawing.sheets[0].parts_list.clone().unwrap();
    assert_eq!((pl.x, pl.y), (410.0, 40.0));
    // A design without components has no parts list to make.
    let mut t = Session::new();
    run(&mut t, "sketch.create", json!({"plane": "xy"}));
    run(
        &mut t,
        "sketch.rectangle",
        json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
    );
    run(&mut t, "sketch.finish", json!({}));
    run(
        &mut t,
        "solid.extrude",
        json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
    );
    run(
        &mut t,
        "drawing.add_view",
        json!({"view": "front", "x": 150, "y": 150}),
    );
    assert!(t.execute("drawing.add_parts_list", &json!({})).is_err());
    assert!(
        t.execute("drawing.add_balloons", &json!({"view": 2}))
            .is_err()
    );
}

#[test]
fn drawing_v2_commands_refuse_hostile_input_and_change_nothing() {
    let mut s = Session::new();
    parametric_box(&mut s);
    run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 200}),
    );
    let huge = "x".repeat(10_000);
    let cases = [
        json!({"view": 2, "x1": f64::NAN, "y1": 0, "x2": 1, "y2": 1, "x": 0, "y": 0}),
        json!({"view": 2, "x1": 150, "y1": 200, "x2": 150, "y2": 200.5, "x": 0, "y": 0}),
        json!({"view": 2, "x1": 0, "y1": 0, "x2": 100, "y2": 0, "x": 1e9, "y": 0}),
        json!({"view": 99, "x1": 0, "y1": 0, "x2": 100, "y2": 0, "x": 1, "y": 1}),
        json!({"view": 2, "cx": 150, "cy": 200, "radius": 0, "x": 1, "y": 1}),
        json!({"view": 2, "cx": 150, "cy": 200, "radius": 1e300, "x": 1, "y": 1}),
        json!({"view": 2, "cx": 150, "cy": 200, "radius": 5, "x": 1, "y": 1, "scale": 1e9}),
        json!({"view": 2, "cx": 150, "cy": 200, "radius": 5, "x": 1, "y": 1, "label": "a1"}),
        json!({"view": 2, "x1": 150, "y1": 200, "x2": 190, "y2": 200, "x": 1, "y": 1, "label": "AAAA"}),
        json!({"x": 0, "y": 0, "text": huge}),
        json!({"x": 0, "y": 0, "text": "\u{0}"}),
        json!({"x": 0, "y": 0, "text": "ok", "height": 1e9}),
        json!({"points": [{"x": 0, "y": 0}], "text": "one point"}),
        json!({"points": vec![json!({"x": 0, "y": 0}); 99], "text": "many"}),
        json!({"points": [{"x": "a", "y": 0}, {"x": 1, "y": 1}], "text": "bad"}),
        json!({"points": "nope", "text": "bad"}),
        json!({"characteristic": "position", "tolerance": 1, "x": 0, "y": 0, "datums": "A"}),
        json!({"characteristic": "position", "tolerance": 1, "x": 0, "y": 0, "datums": [1, 2]}),
        json!({"characteristic": "position", "tolerance": 1, "x": 0, "y": 0, "datums": ["A", "B", "C", "D"]}),
        json!({"characteristic": "position", "tolerance": 1, "x": 0, "y": 0, "datums": ["A"], "leader_x": 5}),
        json!({"view": 2, "x1": 0, "y1": 0, "x2": 5, "y2": 5}),
        json!({"view": 2, "flip": "yes", "x1": 0, "y1": 0, "x2": 5, "y2": 5, "x": 1, "y": 1}),
        json!({"x": f64::INFINITY, "y": 0}),
        json!({"annotation": 4_000_000_000u64}),
    ];
    let ids = [
        "drawing.add_section",
        "drawing.add_detail",
        "drawing.add_note",
        "drawing.add_leader",
        "drawing.add_gdt",
        "drawing.add_surface_finish",
        "drawing.add_centre_line",
        "drawing.add_parts_list",
        "drawing.add_balloons",
        "drawing.delete",
        "drawing.move",
        "drawing.pick",
    ];
    for p in &cases {
        for id in ids {
            let before = s.doc.clone();
            if s.execute(id, p).is_err() {
                assert_eq!(s.doc, before, "{id} {p}");
            }
            s.doc = before;
        }
    }
}

/// The values of a reply's `dimensions` list.
fn values(r: &Value) -> Vec<f64> {
    r["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["value"].as_f64().unwrap())
        .collect()
}

fn offsets(s: &Session, r: &Value) -> Vec<f64> {
    r["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let id = u32::try_from(d["dimension"].as_u64().unwrap()).unwrap();
            s.doc.drawing.sheets[0]
                .dimensions
                .iter()
                .find(|x| x.id == id)
                .unwrap()
                .offset
        })
        .collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// The top view of the hole plate (80 x 50 x 10, `examples/hole-plate.json`) at
/// (150, 200), 1:1: the plate's centre (40, 25) is there, so model (x, y) is sheet
/// (110 + x, 175 + y). Its holes: counterbored at (10, 10), (70, 10), (70, 40), (10, 40);
/// countersunk at (40, 25); blind at (25, 25) and (55, 25).
fn hole_plate_top() -> (Session, u64) {
    let mut s = example("hole-plate.json");
    let top = run(
        &mut s,
        "drawing.add_view",
        json!({"view": "top", "x": 150, "y": 200}),
    )["view"]
        .as_u64()
        .unwrap();
    (s, top)
}

#[test]
fn ordinate_baseline_and_chain_dimensions_of_the_hole_plate_read_the_hole_positions() {
    let (mut s, top) = hole_plate_top();
    // Ordinates along x from the lower-left corner (110, 175) to the hole centres at
    // x = 10, 25, 40, 55, 70: 0 at the corner, then each centre's x.
    let r = run(
        &mut s,
        "drawing.add_ordinate",
        json!({"view": top, "axis": "x", "x0": 110.4, "y0": 175.3, "points": [
            {"x": 120, "y": 185}, {"x": 135, "y": 200}, {"x": 150, "y": 200},
            {"x": 165, "y": 200}, {"x": 180, "y": 185}]}),
    );
    assert!(
        close(&values(&r), &[0.0, 10.0, 25.0, 40.0, 55.0, 70.0]),
        "{r}"
    );
    assert!(
        r["dimensions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["associative"] == true)
    );
    // And along y: the counterbored centres at y = 10 and 40.
    let r = run(
        &mut s,
        "drawing.add_ordinate",
        json!({"view": top, "axis": "y", "x0": 110, "y0": 175, "offset": -10,
               "points": [{"x": 120, "y": 185}, {"x": 120, "y": 215}]}),
    );
    assert!(close(&values(&r), &[0.0, 10.0, 40.0]));
    // One command, one undo step for the whole set.
    let before = s.doc.drawing.sheets[0].dimensions.len();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.drawing.sheets[0].dimensions.len(), before - 3);

    // Baseline from the corner to centres at x = 10, 40, 70 and the right corner at 80,
    // given out of order: shortest first. Their lines stack 7 mm apart beyond the
    // highest point (the centres at y = 200): y = 210, 217, 224, 231. A dimension's line
    // is `offset` beyond the higher of its own two points, so the offsets are
    // 210 - 185 = 25, 217 - 200 = 17, 224 - 185 = 39 and 231 - 175 = 56.
    let r = run(
        &mut s,
        "drawing.add_baseline",
        json!({"view": top, "kind": "horizontal", "points": [
            {"x": 110, "y": 175}, {"x": 190, "y": 175}, {"x": 150, "y": 200}, {"x": 120, "y": 185},
            {"x": 180, "y": 185}]}),
    );
    assert!(close(&values(&r), &[10.0, 40.0, 70.0, 80.0]), "{r}");
    assert!(
        close(&offsets(&s, &r), &[25.0, 17.0, 39.0, 56.0]),
        "{:?}",
        offsets(&s, &r)
    );

    // A vertical chain on the left, given out of order: corner (110, 175), the centres
    // (120, 185) and (120, 215), the corner (110, 225): 10, 30, 10, all on the line
    // x = 100, 10 mm left of the leftmost point. Offsets: -10, -10 - (120 - 110) = -20,
    // -10.
    let r = run(
        &mut s,
        "drawing.add_chain",
        json!({"view": top, "kind": "vertical", "offset": -10, "points": [
            {"x": 120, "y": 215}, {"x": 110, "y": 175}, {"x": 110, "y": 225}, {"x": 120, "y": 185}]}),
    );
    assert!(close(&values(&r), &[10.0, 30.0, 10.0]), "{r}");
    assert!(close(&offsets(&s, &r), &[-10.0, -20.0, -10.0]));

    // Refused without changing anything: a point off the part, too few points, a
    // wrong axis.
    for (id, p) in [
        (
            "drawing.add_baseline",
            json!({"view": top, "kind": "horizontal", "points": [{"x": 110, "y": 175}, {"x": 5, "y": 5}]}),
        ),
        (
            "drawing.add_chain",
            json!({"view": top, "kind": "vertical", "points": [{"x": 110, "y": 175}]}),
        ),
        (
            "drawing.add_chain",
            json!({"view": top, "kind": "aligned", "points": [{"x": 110, "y": 175}, {"x": 190, "y": 175}]}),
        ),
        (
            "drawing.add_ordinate",
            json!({"view": top, "axis": "z", "x0": 110, "y0": 175}),
        ),
        (
            "drawing.add_ordinate",
            json!({"view": top, "axis": "x", "x0": 5, "y0": 5}),
        ),
        (
            "drawing.add_baseline",
            json!({"view": top, "kind": "horizontal", "spacing": 1, "points": [{"x": 110, "y": 175}, {"x": 190, "y": 175}]}),
        ),
    ] {
        let before = s.doc.clone();
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
        assert_eq!(s.doc, before);
    }
}

#[test]
fn hole_callouts_of_the_hole_plate_say_what_each_hole_feature_drills_and_follow_it() {
    let (mut s, top) = hole_plate_top();
    // The counterbored holes: Ø6.6 (`bolt`) through, counterbore Ø6.6 x 1.7 = 11.22 and
    // 10 x 0.4 = 4 deep, at four sketch points. Picked on the rim of the one at
    // (10, 10), sheet (120, 185): the counterbore's radius 5.61 to the right.
    let cb = run(
        &mut s,
        "drawing.add_hole_callout",
        json!({"view": top, "x1": 125.61, "y1": 185, "x": 95, "y": 245}),
    );
    assert_eq!(
        cb["text"],
        json!(["4 x Ø6.6 THRU", "CBORE Ø11.22 x 4 DEEP"]),
        "{cb}"
    );
    assert_eq!(cb["associative"], true);
    // The countersunk hole at (40, 25), sheet (150, 200): Ø6 through, Ø12 x 90°.
    let cs = run(
        &mut s,
        "drawing.add_hole_callout",
        json!({"view": top, "x1": 156, "y1": 200, "x": 200, "y": 250}),
    );
    assert_eq!(cs["text"], json!(["Ø6 THRU", "CSK Ø12 x 90°"]));
    // The two blind holes Ø5, 6 deep: picked at the one at (25, 25), sheet (135, 200).
    let blind = run(
        &mut s,
        "drawing.add_hole_callout",
        json!({"view": top, "x1": 137.5, "y1": 200, "x": 60, "y": 140}),
    );
    assert_eq!(blind["text"], json!(["2 x Ø5 x 6 DEEP"]));
    assert_ne!(cb["feature"], cs["feature"]);
    // The plate's own outline is no hole: refused, nothing changed.
    let before = s.doc.clone();
    assert!(
        s.execute(
            "drawing.add_hole_callout",
            &json!({"view": top, "x1": 110, "y1": 200, "x": 60, "y": 140}),
        )
        .is_err()
    );
    assert_eq!(s.doc, before);

    // The bolt made 8 mm: the callout follows the feature, Ø8 and a Ø13.6 counterbore.
    run(
        &mut s,
        "param.set",
        json!({"name": "bolt", "expression": "8 mm"}),
    );
    let l = run(&mut s, "drawing.list", json!({}));
    let ann = l["sheets"][0]["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == cb["annotation"])
        .cloned()
        .unwrap();
    assert_eq!(ann["text"], json!(["4 x Ø8 THRU", "CBORE Ø13.6 x 4 DEEP"]));
    assert!(ann["dangling"].is_null(), "{ann}");
    // Suppressing the countersunk hole leaves its callout dangling, not dropped.
    let feature = cs["feature"].as_u64().unwrap();
    run(
        &mut s,
        "feature.edit",
        json!({"feature": feature, "suppressed": true}),
    );
    let l = run(&mut s, "drawing.list", json!({}));
    let ann = l["sheets"][0]["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == cs["annotation"])
        .cloned()
        .unwrap();
    assert!(ann["dangling"].is_string(), "{ann}");
    // A callout moves with drawing.move and is saved with the design.
    let id = cb["annotation"].as_u64().unwrap();
    let m = run(
        &mut s,
        "drawing.move",
        json!({"annotation": id, "dx": 5, "dy": -5}),
    );
    assert_eq!(
        (m["x"].as_f64(), m["y"].as_f64()),
        (Some(100.0), Some(240.0))
    );
    let text = s.doc.to_json().unwrap();
    let back = chamfera_doc::Document::from_json(&text).unwrap();
    assert_eq!(back.drawing, s.doc.drawing);
}

#[test]
fn sheet_settings_set_every_title_block_field_and_refuse_bad_ones() {
    let mut s = Session::new();
    parametric_box(&mut s);
    run(
        &mut s,
        "drawing.add_view",
        json!({"view": "front", "x": 150, "y": 150}),
    );
    let r = run(
        &mut s,
        "drawing.edit_sheet",
        json!({"paper": "a4", "orientation": "portrait", "scale": 0.5, "projection": "first",
               "name": "Detail sheet", "part_name": "Bracket", "drawing_number": "CH-0042",
               "revision": "C", "material": "AlMg3", "drawn_by": "DB", "checked_by": "QA",
               "date": "2026-11-07"}),
    );
    // A4 portrait is 210 x 297 (ISO 216).
    assert_eq!(
        (r["width"].as_f64(), r["height"].as_f64()),
        (Some(210.0), Some(297.0))
    );
    assert_eq!(r["paper"], "a4");
    assert_eq!(r["scale"], 0.5);
    assert_eq!(r["projection"], "first");
    let t = s.doc.drawing.sheets[0].title.clone();
    assert_eq!(
        t.fields(),
        ["Bracket", "DB", "2026-11-07", "CH-0042", "C", "AlMg3", "QA"]
    );
    // The scale shows as 1:2 in the title block; the fields round-trip through the file.
    let l = run(&mut s, "drawing.list", json!({}));
    assert_eq!(l["sheets"][0]["title"]["drawing_number"], "CH-0042");
    let back = chamfera_doc::Document::from_json(&s.doc.to_json().unwrap()).unwrap();
    assert_eq!(back.drawing.sheets[0].title, t);
    // Too long, control characters, an unknown paper or scale: refused, nothing changed.
    for p in [
        json!({"material": "x".repeat(65)}),
        json!({"revision": "a\nb"}),
        json!({"checked_by": 5}),
        json!({"paper": "a0"}),
        json!({"scale": 0}),
    ] {
        let before = s.doc.clone();
        assert!(s.execute("drawing.edit_sheet", &p).is_err(), "{p}");
        assert_eq!(s.doc, before);
    }
    // A design from before format 39 (no new fields) still opens.
    let old: chamfera_doc::drawing::TitleBlock =
        serde_json::from_str(r#"{"part_name": "P", "drawn_by": "", "date": ""}"#).unwrap();
    assert!(old.drawing_number.is_empty() && old.checked_by.is_empty());
}

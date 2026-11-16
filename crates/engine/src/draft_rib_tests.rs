//! Tests of `solid.draft` and `solid.rib`, each against volumes worked out by hand.

use super::*;
use serde_json::json;

fn volume(s: &Session) -> f64 {
    s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
}

fn close(what: &str, got: f64, want: f64) {
    assert!(
        (got - want).abs() <= 1e-6 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

/// The 40 x 20 x 10 block (feature 2): 8000 mm³.
fn block(s: &mut Session) {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    s.execute(
        "sketch.rectangle",
        &json!({"x1": 0, "y1": 0, "x2": 40, "y2": 20}),
    )
    .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 20, "y": 10}], "distance": 10}),
    )
    .unwrap();
}

/// The names of the block's four side faces.
fn sides(s: &Session) -> Vec<String> {
    let b = &s.solids()[0].body;
    b.faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).ok().flatten().map(str::to_owned))
        .filter(|n| n.starts_with("2:side"))
        .collect()
}

/// The 40 x 20 block of height 10 with its four sides drafted by `deg` degrees from
/// the bottom: the cross-section at height z is (40 - 2 z t)(20 - 2 z t), t = tan a, so
/// V = ∫₀¹⁰ (800 - 120 t z + 4 t² z²) dz = 8000 - 6000 t + (4000/3) t².
fn frustum(deg: f64) -> f64 {
    let t = deg.to_radians().tan();
    8000.0 - 6000.0 * t + 4000.0 / 3.0 * t * t
}

#[test]
fn drafting_the_four_sides_of_a_block_from_its_bottom_gives_the_frustum_volume() {
    let mut s = Session::new();
    block(&mut s);
    let faces = sides(&s);
    assert_eq!(faces.len(), 4, "{faces:?}");
    // The preview shows the result and changes nothing.
    let p = json!({"faces": faces, "neutral": "2:bottom", "angle": 5, "preview": true});
    let r = s.execute("solid.draft", &p).unwrap();
    close("preview", r["volume"].as_f64().unwrap(), frustum(5.0));
    assert_eq!(s.scene.previews.len(), 1);
    assert!(s.doc.features.len() == 1);
    s.execute("solid.draft", &json!({"clear": true})).unwrap();
    assert!(s.scene.previews.is_empty());
    let r = s
        .execute(
            "solid.draft",
            &json!({"faces": faces, "neutral": "2:bottom", "angle": "5 deg"}),
        )
        .unwrap();
    assert_eq!(r["name"], "Draft1");
    close("draft", volume(&s), frustum(5.0));
    let id = r["feature"].as_u64().unwrap();
    // The angle is a parameter: 10°, then undo back to 5°.
    s.execute("feature.edit", &json!({"feature": id, "angle": 10}))
        .unwrap();
    close("10 deg", volume(&s), frustum(10.0));
    s.execute("edit.undo", &json!({})).unwrap();
    close("undone", volume(&s), frustum(5.0));
    // A taller block rebuilds with the draft in place: height 12 gives
    // ∫₀¹² = 800·12 - 60 t 144 + (4/3) t² 1728.
    s.execute("feature.edit", &json!({"feature": 2, "distance": 12}))
        .unwrap();
    let t = 5f64.to_radians().tan();
    close("taller", volume(&s), 9600.0 - 8640.0 * t + 2304.0 * t * t);
}

#[test]
fn an_origin_plane_is_a_neutral_plane_and_flip_reverses_the_pull() {
    // The XY plane is the block's bottom with the pull up: the same frustum.
    let mut s = Session::new();
    block(&mut s);
    let faces = sides(&s);
    s.execute(
        "solid.draft",
        &json!({"faces": faces, "neutral": "xy", "angle": 5}),
    )
    .unwrap();
    close("xy", volume(&s), frustum(5.0));
    // Pulled down, the walls lean out going up: t → -t.
    let mut s = Session::new();
    block(&mut s);
    s.execute(
        "solid.draft",
        &json!({"faces": faces, "neutral": "xy", "angle": 5, "flip": true}),
    )
    .unwrap();
    close("flipped", volume(&s), frustum(-5.0));
}

#[test]
fn bad_drafts_are_refused_and_change_nothing() {
    let mut s = Session::new();
    block(&mut s);
    let faces = sides(&s);
    for p in [
        json!({"faces": faces, "neutral": "2:bottom", "angle": 0}),
        json!({"faces": faces, "neutral": "2:bottom", "angle": 85}),
        json!({"faces": faces, "neutral": "9:nowhere", "angle": 5}),
        json!({"faces": faces, "neutral": "2:bottom"}),
        json!({"faces": ["2:top"], "neutral": "2:bottom", "angle": 5}),
        json!({"faces": faces, "angle": 5}),
    ] {
        assert!(s.execute("solid.draft", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc.features.len(), 1);
    close("unchanged", volume(&s), 8000.0);
}

/// A closed polyline in the active sketch, joined point to point.
fn polyline(s: &mut Session, pts: &[(f64, f64)]) -> Vec<u64> {
    let mut curves = Vec::new();
    let mut first: Option<u64> = None;
    let mut prev: Option<u64> = None;
    for (k, a) in pts.iter().enumerate() {
        let b = pts[(k + 1) % pts.len()];
        let mut p = json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1});
        if let Some(id) = prev {
            p["start"] = json!(id);
        }
        if k + 1 == pts.len()
            && let Some(id) = first
        {
            p["end"] = json!(id);
        }
        let r = s.execute("sketch.line", &p).unwrap();
        first.get_or_insert(r["start"].as_u64().unwrap());
        prev = r["end"].as_u64();
        curves.push(r["curve"].as_u64().unwrap());
    }
    curves
}

/// An L-bracket 40 x 40 with 5 mm legs, 20 deep (feature 2):
/// (40·5 + 35·5)·20 = 7500 mm³. Then a sketch on the plane z = 10 holding the line
/// (5,25) → (25,5) across the inside corner; returns (sketch, line).
fn bracket_with_rib_line(s: &mut Session) -> (u64, u64) {
    s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
    polyline(
        s,
        &[
            (0.0, 0.0),
            (40.0, 0.0),
            (40.0, 5.0),
            (5.0, 5.0),
            (5.0, 40.0),
            (0.0, 40.0),
        ],
    );
    s.execute("sketch.finish", &json!({})).unwrap();
    s.execute(
        "solid.extrude",
        &json!({"sketch": 1, "profiles": [{"x": 2, "y": 2}], "distance": 20}),
    )
    .unwrap();
    close("bracket", volume(s), 7500.0);
    let plane = s
        .execute(
            "construct.plane_offset",
            &json!({"base": "xy", "distance": 10}),
        )
        .unwrap()["feature"]
        .as_u64()
        .unwrap();
    let sketch = s
        .execute("sketch.create", &json!({"plane": plane}))
        .unwrap()["sketch"]
        .as_u64()
        .unwrap();
    let line = s
        .execute(
            "sketch.line",
            &json!({"x1": 5, "y1": 25, "x2": 25, "y2": 5}),
        )
        .unwrap()["curve"]
        .as_u64()
        .unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    (sketch, line)
}

#[test]
fn a_rib_across_the_inside_corner_of_an_l_bracket_adds_the_triangle() {
    // The line and the inner faces x = 5, y = 5 bound a right triangle with legs 20:
    // area 200, 4 thick (z 8..12): 800 mm³.
    let mut s = Session::new();
    let (sketch, line) = bracket_with_rib_line(&mut s);
    let before = s.doc.features.len();
    let p = json!({"sketch": sketch, "curves": [line], "thickness": 4, "preview": true});
    let r = s.execute("solid.rib", &p).unwrap();
    close("preview", r["volume"].as_f64().unwrap(), 8300.0);
    assert_eq!(s.doc.features.len(), before);
    close("unchanged", volume(&s), 7500.0);
    let r = s
        .execute(
            "solid.rib",
            &json!({"sketch": sketch, "curves": [line], "thickness": "4 mm"}),
        )
        .unwrap();
    assert_eq!(r["name"], "Rib1");
    assert_eq!(s.solids().len(), 1);
    close("rib", volume(&s), 8300.0);
    let rib = r["feature"].as_u64().unwrap();
    // The rib's faces are named after it like an extrude's.
    let top = format!("{rib}:top");
    let b = &s.solids()[0].body;
    let names: Vec<String> = b
        .faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).ok().flatten().map(str::to_owned))
        .collect();
    assert!(names.contains(&top), "{names:?}");
    assert!(names.contains(&format!("{rib}:side 0")), "{names:?}");
    // A later feature finds the rib's top by that name, before and after the rib
    // changes: raised 1 mm the rib is 8..13 (5 thick, +1000), and with the rib 2 thick
    // (9..11) raised to 9..12 (+600).
    s.execute("solid.offset_face", &json!({"faces": [top], "distance": 1}))
        .unwrap();
    close("raised", volume(&s), 7500.0 + 200.0 * 5.0);
    s.execute("feature.edit", &json!({"feature": rib, "thickness": 2}))
        .unwrap();
    close("thinner", volume(&s), 7500.0 + 200.0 * 3.0);
    assert!(s.feature_errors().is_empty(), "{:?}", s.feature_errors());
    // A web through the corner adds the same triangle: past it is the bracket itself.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("feature.edit", &json!({"feature": rib, "web": true}))
        .unwrap();
    close("web", volume(&s), 8300.0);
    assert_eq!(s.doc.feature(rib as u32).unwrap().name, "Rib1");
}

#[test]
fn bad_ribs_are_refused_and_change_nothing() {
    let mut s = Session::new();
    let (sketch, line) = bracket_with_rib_line(&mut s);
    let before = s.doc.features.len();
    for p in [
        json!({"sketch": sketch, "curves": [line], "thickness": 0}),
        json!({"sketch": sketch, "curves": [], "thickness": 4}),
        json!({"sketch": sketch, "curves": [999], "thickness": 4}),
        json!({"sketch": sketch, "curves": [line, line], "thickness": 4}),
        json!({"sketch": sketch, "curves": [line], "thickness": 4, "body": "nobody"}),
        // Turned away from the bracket: nothing to meet.
        json!({"sketch": sketch, "curves": [line], "thickness": 4, "flip": true}),
        json!({"sketch": 999, "curves": [line], "thickness": 4}),
        json!({"sketch": sketch, "curves": [line]}),
    ] {
        let e = s.execute("solid.rib", &p);
        assert!(e.is_err(), "{p}");
    }
    assert_eq!(s.doc.features.len(), before);
    close("unchanged", volume(&s), 7500.0);
}

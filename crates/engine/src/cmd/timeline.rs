//! `timeline.*`: the build order of sketches and features, the rollback marker and groups.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::ItemKind;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "timeline.group",
        label: "Group",
        params: "items: list of integers (required) - adjacent sketch and feature ids\n\
                 name: string (optional) - the group's name, default GroupN",
        mutates: true,
        run: group,
    },
    CommandSpec {
        id: "timeline.list",
        label: "List Timeline",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "timeline.move",
        label: "Move in Timeline",
        params: "item: integer (required) - a sketch, feature or group id\n\
                 to: integer (required) - its index in the new order, from 0",
        mutates: true,
        run: move_item,
    },
    CommandSpec {
        id: "timeline.rename",
        label: "Rename",
        params: "item: integer (required) - a sketch, feature or group id\n\
                 name: string (required) - the new name",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "timeline.roll_to",
        label: "Roll Timeline To",
        params: "position: integer (required) - how many items stay built, from 0 to the \
                 number of items",
        mutates: true,
        run: roll_to,
    },
    CommandSpec {
        id: "timeline.roll_to_end",
        label: "Roll Timeline to End",
        params: "(none)",
        mutates: true,
        run: roll_to_end,
    },
    CommandSpec {
        id: "timeline.ungroup",
        label: "Ungroup",
        params: "group: integer (required) - a group id",
        mutates: true,
        run: ungroup,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// Largest index a timeline command accepts (well above the item limits).
const MAX_INDEX: f64 = 1e6;

/// A whole number from 0 to [`MAX_INDEX`].
fn index(p: &Value, key: &str) -> Result<usize, CommandError> {
    let v = params::req_f64(p, key)?;
    if v.fract() != 0.0 || !(0.0..=MAX_INDEX).contains(&v) {
        return Err(bad(format!("`{key}` must be a whole number from 0")));
    }
    Ok(v as usize)
}

/// The marker and order cannot change under a sketch being edited.
fn not_sketching(s: &Session) -> Result<(), CommandError> {
    if s.active_sketch.is_some() {
        return Err(CommandError::Unavailable(
            "finish the sketch before changing the timeline".into(),
        ));
    }
    Ok(())
}

/// The timeline as the user sees it: items in order, the marker, groups.
pub(crate) fn timeline_reply(s: &Session) -> Value {
    let doc = &s.doc;
    let built = doc.built_items();
    let items: Vec<Value> = doc
        .timeline_order()
        .into_iter()
        .map(|id| {
            let kind = match doc.item_kind(id) {
                Some(ItemKind::Sketch) => "sketch",
                _ => "feature",
            };
            json!({
                "id": id,
                "kind": kind,
                "name": doc.item_name(id),
                "built": built.contains(&id),
                "suppressed": doc.feature(id).map(|f| doc.is_suppressed(f)).unwrap_or(false),
                "group": doc.group_of(id).map(|g| g.id),
                "error": s.feature_errors().get(&id).or_else(|| s.sketch_errors().get(&id)),
                "broken_reference": s.broken_references().get(&id),
            })
        })
        .collect();
    let groups: Vec<Value> = doc
        .timeline
        .groups
        .iter()
        .map(|g| json!({ "id": g.id, "name": g.name, "items": g.items }))
        .collect();
    json!({
        "items": items,
        "marker": doc.marker(),
        "rolled_back": doc.rolled_back(),
        "groups": groups,
    })
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(timeline_reply(s))
}

fn roll_to(s: &mut Session, p: &Value) -> CmdResult {
    let position = index(p, "position")?;
    not_sketching(s)?;
    s.doc.roll_to(position)?;
    s.rebuild();
    Ok(timeline_reply(s))
}

fn roll_to_end(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    not_sketching(s)?;
    let n = s.doc.timeline_order().len();
    s.doc.roll_to(n)?;
    s.rebuild();
    Ok(timeline_reply(s))
}

fn move_item(s: &mut Session, p: &Value) -> CmdResult {
    let item = params::req_id(p, "item")?;
    let to = index(p, "to")?;
    not_sketching(s)?;
    // Features that build now must still build after the move: a cut moved before the
    // body it cuts has nothing left to cut.
    let built = s.doc.built_items();
    let ok_before: Vec<u32> = s
        .doc
        .features
        .iter()
        .filter(|f| built.contains(&f.id) && !s.doc.is_suppressed(f))
        .map(|f| f.id)
        .filter(|id| !s.feature_errors().contains_key(id))
        .collect();
    s.doc.move_item(item, to)?;
    s.rebuild();
    let name = s.doc.item_name(item).unwrap_or_default();
    if let Some((id, e)) = s
        .feature_errors()
        .iter()
        .find(|(id, _)| ok_before.contains(id))
    {
        let broken = s.doc.item_name(*id).unwrap_or_default();
        return Err(CommandError::Unavailable(format!(
            "moving {name} there breaks {broken}: {e}"
        )));
    }
    Ok(timeline_reply(s))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let item = params::req_id(p, "item")?;
    let name = params::req_str(p, "name")?;
    s.doc.rename_item(item, name)?;
    s.rebuild();
    Ok(json!({ "item": item, "name": s.doc.item_name(item) }))
}

fn group(s: &mut Session, p: &Value) -> CmdResult {
    let list = p
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`items` must be a list of sketch and feature ids"))?;
    if list.len() > chamfera_doc::feature::MAX_FEATURES * 2 {
        return Err(bad("too many items"));
    }
    let items = list
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("`items` must be a list of whole-number ids"))
        })
        .collect::<Result<Vec<u32>, CommandError>>()?;
    let name = params::opt_str(p, "name")?;
    let id = s.doc.group_items(&items, name)?;
    Ok(json!({ "group": id, "name": s.doc.item_name(id), "items": items }))
}

fn ungroup(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "group")?;
    let g = s.doc.ungroup(id)?;
    Ok(json!({ "ungrouped": id, "items": g.items }))
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::{Value, json};
    use std::f64::consts::PI;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        match s.execute(id, &p) {
            Ok(v) => v,
            Err(e) => panic!("{id} {p}: {e}"),
        }
    }

    fn volume(s: &Session) -> f64 {
        s.solids().iter().map(|x| x.body.volume().unwrap()).sum()
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    /// Plate 50 x 30 x 10 = 15000 (sketch 1, extrude 2), then a circle r = 3 at (25, 15)
    /// (sketch 3: on the plate's top face, or on the XY plane) cut through (extrude 4):
    /// a hole of 9π * 10 = 90π.
    fn plate_with_hole(on_face: bool) -> Session {
        let mut s = Session::new();
        run(&mut s, "sketch.create", json!({"plane": "xy"}));
        run(
            &mut s,
            "sketch.rectangle",
            json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        );
        run(&mut s, "sketch.finish", json!({}));
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 10}),
        );
        let cut = if on_face {
            run(
                &mut s,
                "sketch.create_on_face",
                json!({"feature": 2, "face": "top"}),
            );
            json!({"sketch": 3, "profiles": [{"x": 25, "y": 15}], "distance": 10,
                   "direction": "reversed", "operation": "cut"})
        } else {
            run(&mut s, "sketch.create", json!({"plane": "xy"}));
            // 15 up from z = 0: through the plate, and through a 5 tall boss on top if
            // one is built before the cut.
            json!({"sketch": 3, "profiles": [{"x": 25, "y": 15}], "distance": 15,
                   "operation": "cut"})
        };
        run(
            &mut s,
            "sketch.circle",
            json!({"cx": 25, "cy": 15, "radius": 3}),
        );
        run(&mut s, "sketch.finish", json!({}));
        run(&mut s, "solid.extrude", cut);
        close(volume(&s), 15_000.0 - 90.0 * PI);
        s
    }

    #[test]
    fn rolling_back_before_a_cut_shows_the_uncut_plate_and_rolling_forward_cuts_it() {
        let mut s = plate_with_hole(true);
        let t = run(&mut s, "timeline.roll_to", json!({"position": 2}));
        close(volume(&s), 15_000.0);
        assert_eq!(t["marker"], 2);
        assert_eq!(t["rolled_back"], true);
        let built: Vec<bool> = t["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["built"].as_bool().unwrap())
            .collect();
        assert_eq!(built, [true, true, false, false]);
        // A feature after the marker is not built, and is not an error.
        assert!(s.feature_errors().is_empty());
        let t = run(&mut s, "timeline.roll_to_end", json!({}));
        assert_eq!(t["marker"], 4);
        close(volume(&s), 15_000.0 - 90.0 * PI);
        // Undo walks back through the marker moves.
        run(&mut s, "edit.undo", json!({}));
        close(volume(&s), 15_000.0);
        assert_eq!(s.doc.marker(), 2);
        run(&mut s, "edit.undo", json!({}));
        close(volume(&s), 15_000.0 - 90.0 * PI);
        run(&mut s, "edit.redo", json!({}));
        close(volume(&s), 15_000.0);
        // Out of range, and not while a sketch is being edited.
        assert!(
            s.execute("timeline.roll_to", &json!({"position": 5}))
                .is_err()
        );
        assert!(
            s.execute("timeline.roll_to", &json!({"position": 1.5}))
                .is_err()
        );
        run(&mut s, "sketch.edit", json!({"sketch": 1}));
        assert!(s.execute("timeline.roll_to_end", &json!({})).is_err());
    }

    #[test]
    fn a_feature_made_at_the_marker_builds_in_the_new_order() {
        let mut s = plate_with_hole(false);
        run(&mut s, "timeline.roll_to", json!({"position": 2}));
        // A 10 x 10 boss, 5 tall, on the plate's top: made while rolled back, it goes in
        // before the hole.
        let sk = run(
            &mut s,
            "sketch.create_on_face",
            json!({"feature": 2, "face": "top"}),
        )["sketch"]
            .as_u64()
            .unwrap() as u32;
        run(
            &mut s,
            "sketch.rectangle",
            json!({"x1": 20, "y1": 10, "x2": 30, "y2": 20}),
        );
        run(&mut s, "sketch.finish", json!({}));
        let boss = run(
            &mut s,
            "solid.extrude",
            json!({"sketch": sk, "profiles": [{"x": 25, "y": 15}], "distance": 5,
                   "operation": "join"}),
        )["feature"]
            .as_u64()
            .unwrap() as u32;
        assert_eq!(s.doc.timeline_order(), [1, 2, sk, boss, 3, 4]);
        assert_eq!(s.doc.marker(), 4);
        close(volume(&s), 15_000.0 + 500.0);
        // Rolled forward, the hole (r 3, 15 up from z 0) now goes through plate and boss:
        // 15500 - 9π * 15. Had the boss come after the hole it would be 15500 - 90π.
        run(&mut s, "timeline.roll_to_end", json!({}));
        close(volume(&s), 15_500.0 - 135.0 * PI);
        run(&mut s, "edit.undo", json!({}));
        close(volume(&s), 15_500.0);
        run(&mut s, "edit.undo", json!({}));
        close(volume(&s), 15_000.0);
        assert_eq!(s.doc.timeline_order(), [1, 2, sk, 3, 4]);
    }

    #[test]
    fn moving_a_cut_before_the_extrude_it_cuts_is_refused() {
        // Drawn on the plate's face, the hole depends on the plate.
        let mut s = plate_with_hole(true);
        let before = s.doc.clone();
        for (item, to) in [(4, 1), (3, 1), (2, 0)] {
            let r = s.execute("timeline.move", &json!({"item": item, "to": to}));
            assert!(r.is_err(), "{item} to {to}");
        }
        assert_eq!(s.doc, before);
        // On the XY plane it uses nothing of the plate's, but has nothing to cut there.
        let mut s = plate_with_hole(false);
        let t = run(&mut s, "timeline.move", json!({"item": 3, "to": 1}));
        let ids: Vec<u64> = t["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, [1, 3, 2, 4]);
        close(volume(&s), 15_000.0 - 90.0 * PI);
        let before = s.doc.clone();
        let e = s
            .execute("timeline.move", &json!({"item": 4, "to": 2}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("nothing to cut"), "{e}");
        assert_eq!(s.doc, before);
        close(volume(&s), 15_000.0 - 90.0 * PI);
        // Undo puts the sketch back.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.timeline_order(), [1, 2, 3, 4]);
        assert!(
            s.execute("timeline.move", &json!({"item": 99, "to": 0}))
                .is_err()
        );
        assert!(
            s.execute("timeline.move", &json!({"item": 1, "to": 4}))
                .is_err()
        );
    }

    #[test]
    fn features_and_sketches_are_renamed_and_undo_restores_the_names() {
        let mut s = plate_with_hole(true);
        let r = run(
            &mut s,
            "feature.rename",
            json!({"feature": 2, "name": " Plate "}),
        );
        assert_eq!(r["name"], "Plate");
        assert!(
            s.solids().iter().any(|x| x.name == "Plate"),
            "bodies follow their feature"
        );
        run(
            &mut s,
            "sketch.rename",
            json!({"sketch": 3, "name": "Hole sketch"}),
        );
        assert_eq!(s.doc.sketch(3).unwrap().name, "Hole sketch");
        for bad in ["", "   ", "a\u{0}b"] {
            let r = s.execute("feature.rename", &json!({"feature": 2, "name": bad}));
            assert!(r.is_err(), "{bad:?}");
        }
        let long = "x".repeat(257);
        let r = s.execute("sketch.rename", &json!({"sketch": 3, "name": long}));
        assert!(r.is_err());
        assert!(
            s.execute("sketch.rename", &json!({"sketch": 2, "name": "x"}))
                .is_err()
        );
        assert!(
            s.execute("feature.rename", &json!({"feature": 1, "name": "x"}))
                .is_err()
        );
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.sketch(3).unwrap().name, "Sketch2");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.feature(2).unwrap().name, "Extrude1");
        assert!(s.solids().iter().any(|x| x.name == "Extrude1"));
    }

    #[test]
    fn groups_rename_and_the_marker_survive_save_and_open() {
        let mut s = plate_with_hole(true);
        let g = run(
            &mut s,
            "timeline.group",
            json!({"items": [3, 4], "name": "Hole"}),
        );
        let gid = g["group"].as_u64().unwrap();
        assert!(
            s.execute("timeline.group", &json!({"items": [1, 3]}))
                .is_err()
        );
        assert!(
            s.execute("timeline.group", &json!({"items": [1, 2, 3]}))
                .is_err()
        );
        run(
            &mut s,
            "timeline.rename",
            json!({"item": gid, "name": "Bore"}),
        );
        run(&mut s, "timeline.roll_to", json!({"position": 2}));
        let dir = std::env::temp_dir().join(format!("chamfera-timeline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rolled.chamfera");
        let p = path.to_str().unwrap();
        run(&mut s, "design.save", json!({"path": p}));
        run(&mut s, "design.new", json!({}));
        run(&mut s, "design.open", json!({"path": p}));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(s.doc.marker(), 2);
        close(volume(&s), 15_000.0);
        let t = run(&mut s, "timeline.list", json!({}));
        assert_eq!(t["groups"][0]["name"], "Bore");
        assert_eq!(t["groups"][0]["items"], json!([3, 4]));
        assert_eq!(t["items"][3]["group"], gid);
        run(&mut s, "timeline.roll_to_end", json!({}));
        close(volume(&s), 15_000.0 - 90.0 * PI);
        let u = run(&mut s, "timeline.ungroup", json!({"group": gid}));
        assert_eq!(u["items"], json!([3, 4]));
        assert!(s.doc.timeline.groups.is_empty());
        assert!(
            s.execute("timeline.ungroup", &json!({"group": gid}))
                .is_err()
        );
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.timeline.groups.len(), 1);
    }

    #[test]
    fn a_feature_that_fails_upstream_names_the_reference_that_broke() {
        let mut s = plate_with_hole(true);
        // Suppressing the plate takes away the face the hole sketch is drawn on.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": 2, "suppressed": true}),
        );
        let t = run(&mut s, "timeline.list", json!({}));
        let hole = &t["items"][3];
        assert!(
            hole["error"].as_str().unwrap().contains("is gone"),
            "{hole}"
        );
        assert_eq!(hole["broken_reference"], "face \"top\" of Extrude1");
        assert!(
            t["items"][2]["error"].as_str().is_some(),
            "the sketch says why"
        );
        run(&mut s, "edit.undo", json!({}));
        assert!(s.broken_references().is_empty());
        assert!(s.feature_errors().is_empty());
    }
}

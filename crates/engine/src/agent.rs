//! What every agent front end shares: the MCP server (`chamfera-mcp`) and the in-app
//! assistant (`chamfera-assistant`) both drive the session through commands, and both
//! need to know which commands touch files and how to describe the design compactly.

use crate::Session;
use serde_json::{Value, json};

/// Whether a command reads an existing file or writes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Every command that takes a `path`, and what it does with it.
pub const PATH_COMMANDS: &[(&str, Access)] = &[
    ("body.import_3mf", Access::Read),
    ("body.import_obj", Access::Read),
    ("body.import_step", Access::Read),
    ("body.import_stl", Access::Read),
    ("cam.post", Access::Write),
    // Its `dir` is resolved like a written path: the folder or its parent must be inside.
    ("config.export_all", Access::Write),
    ("design.open", Access::Read),
    ("design.export_3mf", Access::Write),
    ("design.export_glb", Access::Write),
    // Also writes the .mtl file of the same name beside it, in the same allowed folder.
    ("design.export_obj", Access::Write),
    ("design.export_step", Access::Write),
    ("design.export_stl", Access::Write),
    ("design.save", Access::Write),
    ("param.export_csv", Access::Write),
    ("param.import_csv", Access::Read),
    ("sheet.export_flat", Access::Write),
    ("drawing.export", Access::Write),
    ("sketch.export_dxf", Access::Write),
    ("sketch.import_dxf", Access::Read),
];

/// True when a command's parameter docs name a path parameter.
pub fn takes_path(params_doc: &str) -> bool {
    params_doc.lines().any(|l| {
        let name = l.split(':').next().unwrap_or_default();
        name.split(',').any(|n| {
            let n = n.trim();
            n == "path" || n == "file" || n == "folder" || n == "dir" || n.ends_with("_path")
        })
    })
}

/// True for commands that move whole files in or out: those with a path, and the
/// `*_bytes` variants that carry a file's contents in their parameters or reply.
pub fn is_file_command(id: &str) -> bool {
    id.ends_with("_bytes")
        || PATH_COMMANDS.iter().any(|(c, _)| *c == id)
        || crate::cmd::find(id).is_some_and(|c| takes_path(c.params))
}

/// A compact overview of the design: parameters; sketches with point, curve and
/// constraint counts, degrees of freedom, status and profile areas; bodies with volumes
/// and bounds; the timeline's features.
pub fn summary(session: &mut Session) -> Value {
    let ids: Vec<u32> = session.doc.sketches.iter().map(|f| f.id).collect();
    let sketches: Vec<Value> = ids
        .into_iter()
        .map(
            |id| match session.execute("sketch.get", &json!({ "sketch": id })) {
                Ok(r) => {
                    let count = |k: &str| r.get(k).and_then(Value::as_array).map_or(0, Vec::len);
                    json!({
                        "sketch": id,
                        "name": r.get("name"),
                        "points": count("points"),
                        "curves": count("curves"),
                        "constraints": count("constraints"),
                        "dof": r.get("dof"),
                        "status": r.get("status"),
                        "profiles": r.get("profiles"),
                    })
                }
                Err(e) => json!({ "sketch": id, "error": e.to_string() }),
            },
        )
        .collect();
    let bodies: Vec<Value> = session
        .execute("body.list", &json!({}))
        .ok()
        .and_then(|r| r.get("bodies").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|b| {
            // A body the timeline builds reports its exact B-rep volume and area; others
            // (imported meshes) their mesh volume.
            let id = b.get("body").and_then(Value::as_u64);
            let solid = session
                .solids()
                .iter()
                .find(|s| s.scene_id.map(u64::from) == id);
            let (volume, area, exact) = match solid {
                Some(s) => (
                    s.body.volume().ok().map_or(Value::Null, |v| json!(v)),
                    s.body.area().ok().map_or(Value::Null, |a| json!(a)),
                    true,
                ),
                None => (
                    b.get("volume").cloned().unwrap_or(Value::Null),
                    Value::Null,
                    false,
                ),
            };
            json!({
                "body": b.get("body"),
                "name": b.get("name"),
                "visible": b.get("visible"),
                "volume": volume,
                "area": area,
                "exact": exact,
                "feature": solid.map(|s| s.feature),
                "bounds": b.get("bounds"),
            })
        })
        .collect();
    let features = session
        .execute("feature.list", &json!({}))
        .ok()
        .and_then(|r| r.get("features").cloned())
        .unwrap_or(Value::Null);
    json!({
        "parameters": session.doc.parameters,
        "active_sketch": session.active_sketch,
        "sketches": sketches,
        "bodies": bodies,
        "features": features,
        "can_undo": session.can_undo(),
        "can_redo": session.can_redo(),
    })
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 (RFC 4648, standard alphabet, padded), for image content. Encoding only; a few
/// lines are simpler than a dependency.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextets = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        // 1 input byte gives 2 output characters, 2 give 3, 3 give 4; the rest is padding.
        for (i, s) in sextets.iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(BASE64[*s as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test vectors of RFC 4648, section 10.
    #[test]
    fn base64_encodes_the_rfc_4648_test_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded, "{plain:?}");
        }
        // 0xFB 0xFF uses the last two alphabet characters: 111110 111111 1111(00) -> "+/8=".
        assert_eq!(base64(&[0xFB, 0xFF]), "+/8=");
    }

    #[test]
    fn every_command_that_takes_a_path_is_listed_and_counts_as_a_file_command() {
        for c in crate::registry() {
            let listed = PATH_COMMANDS.iter().any(|(id, _)| *id == c.id);
            assert_eq!(takes_path(c.params), listed, "{}", c.id);
            if listed || c.id.ends_with("_bytes") {
                assert!(is_file_command(c.id), "{}", c.id);
            }
        }
        assert!(!is_file_command("solid.extrude"));
        assert!(is_file_command("design.save_bytes"));
    }

    #[test]
    fn grouped_steps_undo_together_and_a_rewound_history_is_not_grouped() {
        let mut s = Session::new();
        s.execute("param.set", &json!({ "name": "a", "value": 1 }))
            .unwrap();
        let mark = s.undo_mark();
        for (name, v) in [("b", 2), ("c", 3), ("d", 4)] {
            s.execute("param.set", &json!({ "name": name, "value": v }))
                .unwrap();
        }
        assert_eq!(s.group_undo_since(mark), 3);
        // One undo takes away b, c and d together, leaving a.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.parameters.len(), 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc.parameters.is_empty());
        assert!(!s.can_undo());
        // Redo brings a back, then b, c and d as one step.
        s.execute("edit.redo", &json!({})).unwrap();
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc.parameters.len(), 4);
        // An undo between the mark and the grouping leaves the history alone.
        let mark = s.undo_mark();
        s.execute("param.set", &json!({ "name": "e", "value": 5 }))
            .unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("param.set", &json!({ "name": "f", "value": 6 }))
            .unwrap();
        assert_eq!(s.group_undo_since(mark), 0);
        // A single step needs no merging but still counts.
        let mark = s.undo_mark();
        s.execute("param.set", &json!({ "name": "g", "value": 7 }))
            .unwrap();
        assert_eq!(s.group_undo_since(mark), 1);
    }

    #[test]
    fn the_summary_of_a_box_reports_its_volume_and_feature() {
        let mut s = Session::new();
        s.execute("sketch.create", &json!({ "plane": "xy" }))
            .unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        )
        .unwrap();
        s.execute(
            "solid.extrude",
            &json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        )
        .unwrap();
        let v = summary(&mut s);
        // 50 x 30 x 10 = 15000 mm³.
        let volume = v["bodies"][0]["volume"].as_f64().unwrap();
        assert!((volume - 15000.0).abs() < 1e-6, "{volume}");
        assert!(
            v["features"].as_array().is_some_and(|f| !f.is_empty()),
            "{v}"
        );
    }
}

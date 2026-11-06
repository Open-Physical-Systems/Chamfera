//! `version.*`: the open design's version history (`crate::versions`). None of these take
//! a path: they work on the history folder beside the design's own file.

use super::CommandSpec;
use crate::versions::{self, Side};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_io::history::{self, Kind};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "version.compare",
        label: "Compare Versions",
        params: "a: integer or string (optional) - the first version's id, or \"current\" \
                 (default) for the open design\nb: integer or string (optional) - the second, \
                 the same way (default \"current\")\nshow: boolean (optional) - show both \
                 versions' bodies over each other in the viewport, A shaded and B see-through \
                 (default true)\nclear: boolean (optional) - true takes a comparison out of \
                 the viewport and compares nothing\nReplies with the parameters changed, the \
                 timeline items added, removed and changed, each side's volume and \
                 `volume_delta` (B minus A)",
        mutates: false,
        run: compare,
    },
    CommandSpec {
        id: "version.delete",
        label: "Delete Version",
        params: "id: integer (required) - the version to delete from the history",
        mutates: false,
        run: delete,
    },
    CommandSpec {
        id: "version.list",
        label: "Version History",
        params: "(none) - replies with the open design's kept versions, oldest first: id, \
                 time, note, kind (save or note) and summary (volume, feature count); a \
                 design never saved has none",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "version.restore",
        label: "Restore Version",
        params: "id: integer (required) - the version whose design replaces the open one, as \
                 one undoable step (the file is not written until it is saved)",
        mutates: true,
        run: restore,
    },
    CommandSpec {
        id: "version.save",
        label: "Save Version",
        params: "note: string (optional) - what this version is, up to 500 characters\nSaves \
                 the design to its file and keeps a version with the note in its history",
        mutates: false,
        run: save,
    },
];

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(CommandError::BadParams(format!(
            "`{key}` must be true or false"
        ))),
    }
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let Ok(dir) = versions::folder(s) else {
        return Ok(json!({ "versions": [], "skipped": [], "folder": null, "saved": false }));
    };
    let listing = history::list(&dir)?;
    let versions: Vec<Value> = listing
        .versions
        .iter()
        .map(history::Version::to_json)
        .collect();
    Ok(json!({
        "versions": versions,
        "skipped": listing.skipped,
        "folder": dir,
        "saved": true,
        "limits": { "count": s.history.limits.max_count, "bytes": s.history.limits.max_bytes },
    }))
}

fn save(s: &mut Session, p: &Value) -> CmdResult {
    let note = params::opt_str(p, "note")?.unwrap_or("");
    if note.chars().count() > history::MAX_NOTE_CHARS {
        return Err(CommandError::BadParams(format!(
            "`note` is at most {} characters",
            history::MAX_NOTE_CHARS
        )));
    }
    let path = s.path.clone().ok_or_else(|| {
        CommandError::Unavailable(
            "save the design to a file first (`design.save` with a path)".into(),
        )
    })?;
    chamfera_io::save_design(&s.doc, &path)?;
    s.mark_saved(path.clone());
    let added = versions::keep(s, Kind::Note, note)?;
    Ok(json!({ "path": path, "version": added.version.to_json(), "pruned": added.pruned }))
}

fn restore(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "id")?;
    let dir = versions::folder(s)?;
    let doc = history::read(&dir, id)?;
    s.doc = doc;
    s.active_sketch = None;
    s.sketch_selection.clear();
    s.selected_edges.clear();
    s.scene.compare.clear();
    Ok(json!({ "id": id, "features": s.doc.features.len(), "parameters": s.doc.parameters.len() }))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "id")?;
    let dir = versions::folder(s)?;
    history::delete(&dir, id)?;
    Ok(json!({ "deleted": id }))
}

fn compare(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    if opt_bool(p, "clear")? == Some(true) {
        s.scene.compare.clear();
        return Ok(json!({ "cleared": true }));
    }
    let a = Side::parse(p.get("a"), "a")?;
    let b = Side::parse(p.get("b"), "b")?;
    let show = opt_bool(p, "show")?.unwrap_or(true);
    versions::compare(s, a, b, show)
}

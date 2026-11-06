//! `edit.*`: undo and redo.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "edit.redo",
        label: "Redo",
        params: "(none)",
        mutates: false,
        run: redo,
    },
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        params: "(none)",
        mutates: false,
        run: undo,
    },
];

// Undo and redo manage the history themselves, so they are not `mutates` (which would
// snapshot the design and push another undo step).
fn undo(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    if !s.undo() {
        return Err(CommandError::Unavailable("nothing to undo".into()));
    }
    Ok(json!({ "can_undo": s.can_undo(), "can_redo": s.can_redo() }))
}

fn redo(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    if !s.redo() {
        return Err(CommandError::Unavailable("nothing to redo".into()));
    }
    Ok(json!({ "can_undo": s.can_undo(), "can_redo": s.can_redo() }))
}

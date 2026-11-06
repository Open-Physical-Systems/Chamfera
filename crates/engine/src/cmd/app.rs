//! `app.*`: facts about this build.

use super::CommandSpec;
use crate::{CmdResult, Session, params};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "app.about",
        label: "About Chamfera",
        params: "(none)",
        mutates: false,
        run: about,
    },
    CommandSpec {
        id: "app.commands",
        label: "List Commands",
        params: "(none)",
        mutates: false,
        run: commands,
    },
];

fn about(_: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(json!({
        "name": "Chamfera",
        "version": env!("CARGO_PKG_VERSION"),
        "design_format": chamfera_doc::FORMAT_VERSION,
        "units": { "length": "mm", "angle": "rad" },
    }))
}

fn commands(_: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let list: Vec<Value> = super::registry()
        .iter()
        .map(|c| json!({ "id": c.id, "label": c.label, "params": c.params, "mutates": c.mutates }))
        .collect();
    Ok(json!({ "commands": list }))
}

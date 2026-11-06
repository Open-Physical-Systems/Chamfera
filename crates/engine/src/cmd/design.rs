//! `design.*`: new, open and save.
//!
//! Path access: the CLI runs as the user, so any path the user names is allowed. The MCP
//! server (M6) will restrict these to allowed folders before calling them.

use super::CommandSpec;
use crate::{CmdResult, Session, params};
use chamfera_doc::Document;
use serde_json::{Value, json};
use std::path::Path;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "design.new",
        label: "New Design",
        params: "(none)",
        mutates: false,
        run: new,
    },
    CommandSpec {
        id: "design.open",
        label: "Open Design",
        params: "path: string (required) - a .chamfera file",
        mutates: false,
        run: open,
    },
    CommandSpec {
        id: "design.save",
        label: "Save Design",
        params: "path: string (required) - where to write the .chamfera file",
        mutates: false,
        run: save,
    },
];

fn new(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.replace_document(Document::default());
    Ok(json!({}))
}

fn open(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let doc = chamfera_io::open_design(Path::new(path))?;
    let count = doc.parameters.len();
    s.replace_document(doc);
    Ok(json!({ "path": path, "parameters": count }))
}

fn save(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    chamfera_io::save_design(&s.doc, Path::new(path))?;
    Ok(json!({ "path": path }))
}

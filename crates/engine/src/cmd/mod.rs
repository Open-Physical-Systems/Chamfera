//! The command registry. Each area has its own file; `registry()` joins them.

mod app;
mod body;
mod design;
mod edit;
mod feature;
mod param;
mod select;
mod sketch;
mod timeline;
mod view;

use crate::{CmdResult, Session};
use serde_json::Value;

/// One command: its id, what the user sees, and what it does.
pub struct CommandSpec {
    /// Our own dotted, lower-case id: `<area>.<thing>[.<variant>]`.
    pub id: &'static str,
    /// The label a user sees (English for now; moves to message catalogs with the UI).
    pub label: &'static str,
    /// One line per parameter: `name: type (required|optional) - meaning`.
    pub params: &'static str,
    /// True when the command can change the design (undo snapshot, roll back on error).
    pub mutates: bool,
    pub run: fn(&mut Session, &Value) -> CmdResult,
}

/// Every command, in a stable order.
pub fn registry() -> Vec<&'static CommandSpec> {
    let mut all: Vec<&'static CommandSpec> = Vec::new();
    for area in [
        app::COMMANDS,
        body::COMMANDS,
        design::COMMANDS,
        edit::COMMANDS,
        feature::COMMANDS,
        param::COMMANDS,
        select::COMMANDS,
        sketch::COMMANDS,
        timeline::COMMANDS,
        view::COMMANDS,
    ] {
        all.extend(area.iter());
    }
    all
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    registry().into_iter().find(|c| c.id == id)
}

/// True for script names like `rect` or `axis2` (the same rule as parameter names).
pub fn is_valid_name(s: &str) -> bool {
    chamfera_doc::is_valid_name(s)
}

/// True for ids like `param.set` or `sketch.rectangle.two_point`.
pub fn is_valid_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|p| {
            let mut chars = p.chars();
            chars.next().is_some_and(|c| c.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

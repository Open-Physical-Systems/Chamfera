//! `param.*`: named values that dimensions and features will refer to.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::Unit;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "param.list",
        label: "List Parameters",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "param.remove",
        label: "Remove Parameter",
        params: "name: string (required) - the parameter to remove",
        mutates: true,
        run: remove,
    },
    CommandSpec {
        id: "param.set",
        label: "Set Parameter",
        params: "name: string (required) - letters, digits, _; not starting with a digit\n\
                 value: number (required) - in mm for length, radians for angle\n\
                 unit: string (optional) - length (default), angle or unitless",
        mutates: true,
        run: set,
    },
];

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(json!({ "parameters": s.doc.parameters }))
}

fn set(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    let value = params::req_f64(p, "value")?;
    let unit = match params::opt_str(p, "unit")? {
        None => Unit::Length,
        Some(u) => Unit::parse(u).ok_or_else(|| {
            CommandError::BadParams(format!("unknown unit {u:?}: use length, angle or unitless"))
        })?,
    };
    s.doc.set_parameter(name, value, unit)?;
    Ok(json!({ "name": name, "value": value, "unit": unit }))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    s.doc.remove_parameter(name)?;
    Ok(json!({ "removed": name }))
}

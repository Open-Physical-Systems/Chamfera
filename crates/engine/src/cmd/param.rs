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
                 expression: string (optional) - such as \"80 mm\", \"width / 2\" or \"30 deg\"\n\
                 value: number (required without expression) - mm for length, radians for angle\n\
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
    let unit = match params::opt_str(p, "unit")? {
        None => Unit::Length,
        Some(u) => Unit::parse(u).ok_or_else(|| {
            CommandError::BadParams(format!("unknown unit {u:?}: use length, angle or unitless"))
        })?,
    };
    match params::opt_str(p, "expression")? {
        Some(e) => s.doc.set_parameter_expression(name, e, unit)?,
        None => {
            let value = params::req_f64(p, "value")?;
            s.doc.set_parameter(name, value, unit)?;
        }
    }
    let param = s.doc.parameters.get(name);
    Ok(json!({
        "name": name,
        "value": param.map(|p| p.value),
        "unit": unit,
        "expression": param.and_then(|p| p.expression.clone()),
    }))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    s.doc.remove_parameter(name)?;
    Ok(json!({ "removed": name }))
}

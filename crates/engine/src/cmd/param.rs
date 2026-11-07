//! `param.*`: user parameters (named values and expressions) and model parameters (the
//! value of every feature and sketch dimension, `Extrude1.distance`), in one table.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{Document, Unit, expr};
use serde_json::{Value, json};
use std::path::Path;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "param.annotate",
        label: "Annotate Parameter",
        params: "name: string (required) - a user or model parameter\n\
                 comment: string (optional) - a note, at most 512 characters\n\
                 favourite: boolean (optional) - pin it at the top of the table",
        mutates: true,
        run: annotate,
    },
    CommandSpec {
        id: "param.export_csv",
        label: "Export Parameters (CSV)",
        params: "path: string (required) - where to write the user parameters as CSV \
                 (name, expression, unit, comment)",
        mutates: false,
        run: export_csv,
    },
    CommandSpec {
        id: "param.import_csv",
        label: "Import Parameters (CSV)",
        params: "path: string (required) - a CSV file with the columns name, expression, \
                 unit and comment; each row creates or replaces a user parameter",
        mutates: true,
        run: import_csv,
    },
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
        id: "param.rename",
        label: "Rename Parameter",
        params: "name: string (required) - a user or model parameter\n\
                 new_name: string (required) - letters, digits, _; every expression \
                 that used the old name follows",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "param.set",
        label: "Set Parameter",
        params: "name: string (required) - letters, digits, _; not starting with a digit; \
                 or a model parameter such as Extrude1.distance\n\
                 expression: string (optional) - such as \"80 mm\", \"width / 2\" or \"30 deg\"\n\
                 value: number (required without expression) - mm for length, radians for angle\n\
                 unit: string (optional) - length (default), angle or unitless (user parameters)\n\
                 comment: string (optional) - a note\n\
                 favourite: boolean (optional) - pin it at the top of the table",
        mutates: true,
        run: set,
    },
];

fn bad(m: impl Into<String>) -> CommandError {
    CommandError::BadParams(m.into())
}

fn unit_name(d: expr::Dim) -> &'static str {
    if d == expr::Dim::LENGTH {
        "length"
    } else if d == expr::Dim::ANGLE {
        "angle"
    } else {
        "unitless"
    }
}

/// Every parameter as the table shows it: user parameters, then model parameters, each
/// with its value or the error that keeps it from having one.
pub fn table(doc: &Document) -> Vec<Value> {
    let mut out = Vec::new();
    for (name, p) in &doc.parameters {
        let effective = doc.effective_expression(name).map(str::to_owned);
        out.push(json!({
            "name": name,
            "kind": "user",
            "expression": effective,
            "base_expression": p.expression,
            "overridden": doc.configurations.active_override(name).is_some(),
            "value": p.value,
            "unit": p.unit,
            "comment": p.comment,
            "favourite": p.favourite,
            "used_by": doc.parameter_users(name),
            "error": Value::Null,
        }));
    }
    for m in doc.model_parameter_list() {
        let (value, error) = match doc.parameter_quantity(&m.name) {
            Ok(q) => (json!(q.value), Value::Null),
            Err(e) => (Value::Null, json!(e.to_string())),
        };
        out.push(json!({
            "name": m.name,
            "kind": "model",
            "key": m.key,
            "owner": m.owner,
            "owner_name": m.owner_name,
            "field": m.field,
            "expression": m.expression,
            "value": value,
            "unit": unit_name(m.dim),
            "comment": m.comment,
            "favourite": m.favourite,
            "used_by": doc.expression_users(&m.name),
            "error": error,
        }));
    }
    out
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(json!({
        "parameters": s.doc.parameters,
        "table": table(&s.doc),
        "configuration": s.doc.configurations.active,
    }))
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_bool(p, key).map(Some),
    }
}

fn set(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    let comment = params::opt_str(p, "comment")?;
    let favourite = opt_bool(p, "favourite")?;
    let expression = params::opt_str(p, "expression")?;
    // A model parameter: its feature or dimension takes the expression.
    if !s.doc.parameters.contains_key(name) && s.doc.model_parameter(name).is_some() {
        if let Some(e) = expression {
            s.doc.set_model_expression(name, e)?;
        } else if comment.is_none() && favourite.is_none() {
            return Err(bad(format!(
                "{name} is a model parameter: give it an `expression`"
            )));
        }
        s.doc.annotate_parameter(name, comment, favourite)?;
        let q = s.doc.parameter_quantity(name)?;
        return Ok(json!({
            "name": name,
            "kind": "model",
            "value": q.value,
            "unit": unit_name(q.dim),
            "expression": s.doc.model_parameter(name).and_then(|m| m.expression),
        }));
    }
    let unit = match params::opt_str(p, "unit")? {
        None => s.doc.parameters.get(name).map_or(Unit::Length, |p| p.unit),
        Some(u) => Unit::parse(u)
            .ok_or_else(|| bad(format!("unknown unit {u:?}: use length, angle or unitless")))?,
    };
    let only_notes = expression.is_none()
        && p.get("value").is_none_or(Value::is_null)
        && (comment.is_some() || favourite.is_some())
        && s.doc.parameters.contains_key(name);
    if !only_notes {
        // While a configuration overrides this parameter, the edit goes to that override.
        let active = s.doc.configurations.active.clone();
        let overridden = s.doc.configurations.active_override(name).is_some();
        match (expression, active) {
            (Some(e), Some(c)) if overridden => s.doc.config_set(&c, name, Some(e))?,
            (Some(e), _) => s.doc.set_parameter_expression(name, e, unit)?,
            (None, _) => {
                let value = params::req_f64(p, "value")?;
                s.doc.set_parameter(name, value, unit)?;
            }
        }
    }
    s.doc.annotate_parameter(name, comment, favourite)?;
    let param = s.doc.parameters.get(name);
    Ok(json!({
        "name": name,
        "kind": "user",
        "value": param.map(|p| p.value),
        "unit": unit,
        "expression": s.doc.effective_expression(name),
    }))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    s.doc.remove_parameter(name)?;
    Ok(json!({ "removed": name }))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    let new_name = params::req_str(p, "new_name")?.trim();
    s.doc.rename_parameter(name, new_name)?;
    Ok(json!({ "name": new_name, "was": name }))
}

fn annotate(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    let comment = params::opt_str(p, "comment")?;
    let favourite = opt_bool(p, "favourite")?;
    s.doc.annotate_parameter(name, comment, favourite)?;
    Ok(json!({ "name": name, "comment": comment, "favourite": favourite }))
}

fn export_csv(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let text = chamfera_io::csv::write_parameters(&s.doc);
    std::fs::write(path, text.as_bytes()).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "parameters": s.doc.parameters.len() }))
}

fn import_csv(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bytes = chamfera_io::read_limited(Path::new(path), chamfera_io::csv::MAX_CSV_BYTES)?;
    let text = String::from_utf8(bytes).map_err(|_| bad("the CSV file is not UTF-8 text"))?;
    let rows = chamfera_io::csv::read_parameters(&text)?;
    let rows: Vec<(String, String, Unit, String)> = rows
        .into_iter()
        .map(|r| (r.name, r.expression, r.unit, r.comment))
        .collect();
    s.doc.set_parameters(&rows)?;
    let names: Vec<&String> = rows.iter().map(|r| &r.0).collect();
    Ok(json!({ "path": path, "imported": names }))
}

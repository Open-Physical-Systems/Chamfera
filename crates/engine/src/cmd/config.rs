//! `config.*`: configurations, a design table of named variants (`Small`, `Large`), each
//! overriding user parameters and feature suppression. Activating one rebuilds.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "config.activate",
        label: "Activate Configuration",
        params: "config: string (optional) - the configuration to build; null or left out \
                 builds the base design",
        mutates: true,
        run: activate,
    },
    CommandSpec {
        id: "config.add",
        label: "Add Configuration",
        params: "name: string (required) - 1 to 64 characters\n\
                 from: string (optional) - a configuration to copy the overrides of",
        mutates: true,
        run: add,
    },
    CommandSpec {
        id: "config.delete",
        label: "Delete Configuration",
        params: "config: string (required) - the configuration to delete",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "config.export_all",
        label: "Export Every Configuration",
        params: "dir: string (required) - the folder to write one file per configuration \
                 into (made if missing)\n\
                 format: string (optional) - step (default) or stl",
        mutates: false,
        run: export_all,
    },
    CommandSpec {
        id: "config.list",
        label: "List Configurations",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "config.set",
        label: "Set Configuration Parameter",
        params: "config: string (required) - the configuration\n\
                 parameter: string (required) - a user parameter\n\
                 expression: string (optional) - its value in this configuration; null or \
                 left out removes the override",
        mutates: true,
        run: set,
    },
    CommandSpec {
        id: "config.suppress",
        label: "Set Configuration Suppression",
        params: "config: string (required) - the configuration\n\
                 feature: integer (required) - a feature id\n\
                 suppressed: boolean (optional) - whether it is suppressed in this \
                 configuration; null or left out follows the feature's own state",
        mutates: true,
        run: suppress,
    },
];

fn bad(m: impl Into<String>) -> CommandError {
    CommandError::BadParams(m.into())
}

/// The total volume of the built solids (mm³), and how many there are.
fn solids_volume(s: &Session) -> (f64, usize) {
    let v: f64 = s.solids().iter().filter_map(|x| x.body.volume().ok()).sum();
    // An empty sum is -0; it reads 0.
    (if v == 0.0 { 0.0 } else { v }, s.solids().len())
}

/// The design table: each configuration's overrides, plus the columns.
pub fn table(s: &Session) -> Value {
    let c = &s.doc.configurations;
    let rows: Vec<Value> = c
        .list
        .iter()
        .map(|k| {
            let suppressed: serde_json::Map<String, Value> = k
                .suppressed
                .iter()
                .map(|(id, v)| (id.to_string(), json!(v)))
                .collect();
            json!({
                "name": k.name,
                "parameters": k.parameters,
                "suppressed": suppressed,
                "active": c.active.as_deref() == Some(k.name.as_str()),
            })
        })
        .collect();
    let features: Vec<Value> = c
        .feature_columns()
        .into_iter()
        .map(|id| {
            json!({
                "id": id,
                "name": s.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "configurations": rows,
        "active": c.active,
        "columns": c.columns(),
        "feature_columns": features,
    })
}

fn reply(s: &Session) -> Value {
    let (volume, solids) = solids_volume(s);
    let mut t = table(s);
    if let Some(o) = t.as_object_mut() {
        o.insert("volume".into(), json!(volume));
        o.insert("solids".into(), json!(solids));
    }
    t
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(reply(s))
}

fn add(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?;
    let from = params::opt_str(p, "from")?;
    let name = s.doc.config_add(name, from)?;
    let mut r = table(s);
    if let Some(o) = r.as_object_mut() {
        o.insert("added".into(), json!(name));
    }
    Ok(r)
}

fn set(s: &mut Session, p: &Value) -> CmdResult {
    let config = params::req_str(p, "config")?;
    let parameter = params::req_str(p, "parameter")?;
    let expression = params::opt_str(p, "expression")?;
    s.doc.config_set(config, parameter, expression)?;
    Ok(table(s))
}

fn suppress(s: &mut Session, p: &Value) -> CmdResult {
    let config = params::req_str(p, "config")?;
    let feature = params::req_id(p, "feature")?;
    let suppressed = match p.get("suppressed") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "suppressed")?),
    };
    s.doc.config_suppress(config, feature, suppressed)?;
    Ok(table(s))
}

fn activate(s: &mut Session, p: &Value) -> CmdResult {
    let config = params::opt_str(p, "config")?;
    s.doc.config_activate(config)?;
    // The session rebuilds after a design change; the reply needs the new solids now.
    s.rebuild();
    Ok(reply(s))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let config = params::req_str(p, "config")?;
    s.doc.config_delete(config)?;
    Ok(table(s))
}

/// A configuration's name as a file name: letters, digits, `-` and `_`.
fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.trim_matches('_').is_empty() {
        "configuration".to_owned()
    } else {
        s
    }
}

fn export_all(s: &mut Session, p: &Value) -> CmdResult {
    let dir = PathBuf::from(params::req_str(p, "dir")?);
    let format = params::opt_str(p, "format")?.unwrap_or("step");
    let (ext, write): (&str, fn(&mut Session, &Value) -> CmdResult) = match format {
        "step" | "stp" => ("step", super::design::export_step),
        "stl" => ("stl", super::design::export_stl),
        f => return Err(bad(format!("unknown format {f:?}: use step or stl"))),
    };
    let names: Vec<String> = s
        .doc
        .configurations
        .list
        .iter()
        .map(|c| c.name.clone())
        .collect();
    if names.is_empty() {
        return Err(CommandError::Unavailable(
            "the design has no configurations: add one with config.add".into(),
        ));
    }
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir).map_err(chamfera_io::IoError::from)?;
    }
    let before = s.doc.clone();
    let mut files = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    let mut result = Ok(());
    for name in &names {
        let mut stem = file_stem(name);
        while !used.insert(stem.clone()) {
            stem.push('_');
        }
        let path = dir.join(format!("{stem}.{ext}"));
        let step = (|| -> Result<Value, CommandError> {
            s.doc.config_activate(Some(name))?;
            s.rebuild();
            if let Some(e) = s.feature_errors().values().next() {
                return Err(CommandError::Unavailable(format!("{name}: {e}")));
            }
            let (volume, _) = solids_volume(s);
            let text = path_text(&path)?;
            write(s, &json!({ "path": text }))?;
            Ok(json!({ "configuration": name, "path": text, "volume": volume }))
        })();
        match step {
            Ok(v) => files.push(v),
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    // Back to the configuration that was active, whatever happened.
    s.doc = before;
    s.rebuild();
    result?;
    Ok(json!({ "dir": path_text(&dir)?, "format": ext, "files": files }))
}

fn path_text(p: &Path) -> Result<String, CommandError> {
    p.to_str()
        .map(str::to_owned)
        .ok_or_else(|| bad("the folder's path is not UTF-8"))
}

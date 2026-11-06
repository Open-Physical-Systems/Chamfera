//! `design.*`: new, open and save.
//!
//! Path access: the CLI runs as the user, so any path the user names is allowed. The MCP
//! server (M6) will restrict these to allowed folders before calling them.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::Document;
use serde_json::{Value, json};
use std::path::Path;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "design.export_step",
        label: "Export STEP",
        params: "path: string (required) - where to write the .step file (all solids)",
        mutates: false,
        run: export_step,
    },
    CommandSpec {
        id: "design.export_stl",
        label: "Export STL",
        params: "path: string (required) - where to write the binary .stl file (all shown solids)",
        mutates: false,
        run: export_stl,
    },
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
        params: "path: string (optional) - where to write the .chamfera file; default the file it \
                 was opened from or last saved to",
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
    s.path = Some(Path::new(path).to_path_buf());
    Ok(json!({ "path": path, "parameters": count }))
}

fn save(s: &mut Session, p: &Value) -> CmdResult {
    let path = match params::opt_str(p, "path")? {
        Some(path) => Path::new(path).to_path_buf(),
        None => s.path.clone().ok_or_else(|| {
            CommandError::BadParams("`path` is required: the design has not been saved yet".into())
        })?,
    };
    chamfera_io::save_design(&s.doc, &path)?;
    s.mark_saved(path.clone());
    Ok(json!({ "path": path }))
}

fn export_step(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    if s.solids().is_empty() {
        return Err(CommandError::Unavailable(
            "there are no solids to export".into(),
        ));
    }
    let bodies: Vec<(&str, &chamfera_kernel::Body)> = s
        .solids()
        .iter()
        .map(|x| (x.name.as_str(), &x.body))
        .collect();
    let product = s.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
        || "Chamfera design".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let options = chamfera_io::step::StepOptions {
        product_name: product,
        ..Default::default()
    };
    chamfera_io::step::write_file(Path::new(path), &bodies, &options)?;
    Ok(json!({ "path": path, "solids": bodies.len() }))
}

fn export_stl(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let mut mesh = chamfera_geom::TriMesh::default();
    for solid in s.solids() {
        if let Some(b) = solid.scene_id.and_then(|id| s.scene.body(id)) {
            mesh.append(&b.mesh);
        }
    }
    if mesh.triangles.is_empty() {
        return Err(CommandError::Unavailable(
            "there are no shown solids to export".into(),
        ));
    }
    let bytes = chamfera_io::stl::write_binary(&mesh)?;
    std::fs::write(path, bytes).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "triangles": mesh.triangles.len() }))
}

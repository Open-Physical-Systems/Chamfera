//! `body.*`: bodies in the scene. Imported meshes (STL, OBJ, 3MF) and solids (STEP) are
//! part of the design: saved with it and undone like any change. The sample part is
//! shown for the session only.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::{Point3, TriMesh};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "body.appearance",
        label: "Appearance",
        params: "body: integer or string (required) - the body id, or its name (Extrude1)\n\
                 preset: string (optional) - a material of the library (render.materials \
                 lists them: steel, aluminium, brass, copper, chrome, gold, glass, rubber, \
                 plastic_white, wood_oak, paint_red, ...); or default for the theme's colour\n\
                 colour: string (optional) - #rrggbb, or default for the theme's colour\n\
                 opacity: number (optional) - 0.05 to 1 (1 is opaque)\n\
                 metallic: number (optional) - 0 (plastic, paint) to 1 (metal)\n\
                 roughness: number (optional) - 0 (mirror) to 1 (matte)\n\
                 transmission: number (optional) - 0 (opaque) to 1 (glass)\n\
                 ior: number (optional) - index of refraction, 1 to 3 (glass 1.5)\n\
                 Give at least one. A solid's appearance is saved with the design (for \
                 every body of its feature); a mesh body's lasts for the session.",
        mutates: true,
        run: crate::display::appearance,
    },
    CommandSpec {
        id: "body.import_3mf",
        label: "Insert Mesh (3MF)",
        params: "path: string (required) - a .3mf file; each build item becomes a mesh body, \
                 in mm",
        mutates: true,
        run: import_3mf,
    },
    CommandSpec {
        id: "body.import_3mf_bytes",
        label: "Insert Mesh (3MF)",
        params: "data: string (required) - the .3mf file's contents, base64",
        mutates: true,
        run: import_3mf_bytes,
    },
    CommandSpec {
        id: "body.import_obj",
        label: "Insert Mesh (OBJ)",
        params: "path: string (required) - an .obj file, in mm; its faces become one mesh body",
        mutates: true,
        run: import_obj,
    },
    CommandSpec {
        id: "body.import_obj_bytes",
        label: "Insert Mesh (OBJ)",
        params: "data: string (required) - the .obj file's contents, base64\n\
                 name: string (optional) - the body's name; default \"Mesh\"",
        mutates: true,
        run: import_obj_bytes,
    },
    CommandSpec {
        id: "body.import_step",
        label: "Insert STEP Solids",
        params: "path: string (required) - a .step/.stp file (AP203, AP214, AP242); its B-rep solids become exact bodies, solids it cannot import yet are listed in `skipped`; a name a body already has gets a number (`Extrude1 (2)`)",
        mutates: true,
        run: import_step,
    },
    CommandSpec {
        id: "body.import_stl",
        label: "Insert Mesh (STL)",
        params: "path: string (required) - an .stl file, binary or ASCII, in mm",
        mutates: true,
        run: import_stl,
    },
    CommandSpec {
        id: "body.import_stl_bytes",
        label: "Insert Mesh (STL)",
        params: "data: string (required) - the .stl file's contents (binary or ASCII, in mm), \
                 base64\nname: string (optional) - the body's name; default \"Mesh\"",
        mutates: true,
        run: import_stl_bytes,
    },
    CommandSpec {
        id: "body.list",
        label: "List Bodies",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "body.remove",
        label: "Remove Body",
        params: "body: integer (required) - the body id; an imported mesh leaves the design \
                 (undoable), other bodies are hidden from the scene until the next rebuild",
        mutates: true,
        run: remove,
    },
    CommandSpec {
        id: "body.rename",
        label: "Rename Body",
        params: "body: integer (required) - the body id; a solid takes its feature's name, \
                 so rename that feature instead\n\
                 name: string (required) - the new name, 1 to 80 characters",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "body.sample",
        label: "Insert Sample Part",
        params: "(none)",
        mutates: false,
        run: sample,
    },
    CommandSpec {
        id: "body.visibility",
        label: "Show or Hide Body",
        params: "body: integer (required) - the body id\nvisible: boolean (required)",
        mutates: false,
        run: visibility,
    },
];

fn summary(s: &Session, id: u32) -> Value {
    match s.scene.body(id) {
        Some(b) => {
            let bounds = b.mesh.bounds();
            json!({
                "body": b.id,
                "name": b.name,
                "visible": b.visible,
                "triangles": b.mesh.triangles.len(),
                "area": b.mesh.area(),
                // A surface (an open shell) encloses nothing.
                "volume": (!b.surface).then(|| b.mesh.signed_volume()),
                "bounds": bounds,
                "appearance": b.appearance,
            })
        }
        None => Value::Null,
    }
}

/// Adds a session-only mesh body (the sample).
fn add(s: &mut Session, name: &str, mesh: TriMesh) -> CmdResult {
    let id = s
        .scene
        .add(name, mesh)
        .ok_or_else(|| CommandError::Unavailable("the scene holds no more bodies".into()))?;
    Ok(summary(s, id))
}

/// Adds an imported mesh to the design (saved and undone with it) under a name no
/// other body has, and shows it.
fn add_mesh(s: &mut Session, name: &str, mesh: TriMesh) -> CmdResult {
    if s.scene.bodies.len() >= chamfera_render::MAX_BODIES {
        return Err(CommandError::Unavailable(
            "the scene holds no more bodies".into(),
        ));
    }
    let name = s.doc.unique_body_name(name);
    let id = s.doc.add_imported_mesh(&name, mesh)?;
    if !s.sync_meshes() {
        return Err(CommandError::Unavailable(
            "the scene holds no more bodies".into(),
        ));
    }
    let scene = s
        .mesh_scene
        .get(&id)
        .copied()
        .ok_or_else(|| CommandError::Unavailable("the mesh could not be shown".into()))?;
    Ok(summary(s, scene))
}

fn import_stl(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let file = std::path::Path::new(path);
    let bytes = chamfera_io::read_limited(file, chamfera_io::stl::MAX_STL_BYTES)?;
    let mesh = chamfera_io::stl::read(&bytes)?;
    let name = file
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("Mesh")
        .chars()
        .take(64)
        .collect::<String>();
    add_mesh(s, &name, mesh)
}

fn import_stl_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes =
        super::design::decode_base64(params::req_str(p, "data")?, chamfera_io::stl::MAX_STL_BYTES)?;
    let mesh = chamfera_io::stl::read(&bytes)?;
    let name = params::opt_str(p, "name")?
        .map(|n| n.trim().trim_end_matches(".stl").trim_end_matches(".STL"))
        .filter(|n| !n.is_empty())
        .unwrap_or("Mesh");
    let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    add_mesh(s, &name, mesh)
}

/// A body name from a file name: no extension, no control characters, at most 64
/// characters, never empty.
fn body_name(name: Option<&str>, extension: &str) -> String {
    let name = name.map(str::trim).unwrap_or("");
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case(extension) => stem,
        _ => name,
    };
    let clean: String = stem.chars().filter(|c| !c.is_control()).take(64).collect();
    if clean.trim().is_empty() {
        "Mesh".to_owned()
    } else {
        clean
    }
}

fn import_obj(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let file = std::path::Path::new(path);
    let bytes = chamfera_io::read_limited(file, chamfera_io::obj::MAX_OBJ_BYTES)?;
    let mesh = chamfera_io::obj::read(&bytes)?;
    let name = body_name(file.file_name().and_then(|n| n.to_str()), "obj");
    add_mesh(s, &name, mesh)
}

fn import_obj_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes =
        super::design::decode_base64(params::req_str(p, "data")?, chamfera_io::obj::MAX_OBJ_BYTES)?;
    let mesh = chamfera_io::obj::read(&bytes)?;
    let name = body_name(params::opt_str(p, "name")?, "obj");
    add_mesh(s, &name, mesh)
}

/// Adds each build item of a 3MF file as a mesh body; all or none.
fn add_3mf(s: &mut Session, bytes: &[u8]) -> CmdResult {
    let items = chamfera_io::threemf::read(bytes)?;
    if s.scene.bodies.len() + items.len() > chamfera_render::MAX_BODIES {
        return Err(CommandError::Unavailable(format!(
            "the scene holds at most {} bodies",
            chamfera_render::MAX_BODIES
        )));
    }
    let mut bodies = Vec::new();
    let mut triangles = 0;
    for item in items {
        triangles += item.mesh.triangles.len();
        let name = body_name(Some(&item.name), "3mf");
        bodies.push(add_mesh(s, &name, item.mesh)?);
    }
    let name = bodies
        .first()
        .map(|b| b["name"].clone())
        .unwrap_or_default();
    Ok(json!({ "bodies": bodies, "name": name, "triangles": triangles }))
}

fn import_3mf(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bytes = chamfera_io::read_limited(
        std::path::Path::new(path),
        chamfera_io::threemf::MAX_3MF_BYTES,
    )?;
    add_3mf(s, &bytes)
}

fn import_3mf_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes = super::design::decode_base64(
        params::req_str(p, "data")?,
        chamfera_io::threemf::MAX_3MF_BYTES,
    )?;
    add_3mf(s, &bytes)
}

/// Imports the complete solids of a STEP file into the design (imported bodies are
/// saved with it and can be undone), shows them, and reports their exact measures and
/// the solids left out. Fails, changing nothing, when no solid can be imported.
fn import_step(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let report = chamfera_io::step::import_file(std::path::Path::new(path))?;
    let skipped: Vec<Value> = report
        .failed
        .iter()
        .map(|f| {
            json!({
                "entity": f.entity,
                "name": f.name,
                "kind": match f.kind {
                    chamfera_io::step::FailureKind::Unsupported => "unsupported",
                    chamfera_io::step::FailureKind::Invalid => "invalid",
                },
                "reasons": f.reasons,
            })
        })
        .collect();
    if report.solids.is_empty() {
        let why: Vec<String> = report.failed.iter().map(ToString::to_string).collect();
        return Err(CommandError::Unavailable(format!(
            "no solid of {path} can be imported: {}",
            why.join(" | ")
        )));
    }
    // A name a body already has gets a number, so names keep telling bodies apart.
    let mut ids = Vec::new();
    for (name, body) in report.solids {
        let name = s.doc.unique_body_name(&name);
        ids.push(s.doc.add_imported(&name, body)?);
    }
    s.rebuild();
    let solids: Vec<Value> = ids
        .iter()
        .filter_map(|id| s.solids().iter().find(|x| x.feature == *id))
        .map(|x| {
            json!({
                "id": x.feature,
                "name": x.name,
                "body": x.scene_id,
                "volume": x.body.volume().ok(),
                "area": x.body.area().ok(),
            })
        })
        .collect();
    Ok(json!({ "path": path, "solids": solids, "skipped": skipped }))
}

/// The sample: an L-bracket, 80 x 50 x 8 mm base, 40 mm upright, a 20 mm boss.
/// Volumes are exact boxes plus a 64-sided prism, so tests can check them.
pub(crate) fn sample_mesh() -> TriMesh {
    let mut m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(80.0, 50.0, 8.0));
    m.append(&TriMesh::cuboid(
        Point3::new(0.0, 42.0, 8.0),
        Point3::new(80.0, 50.0, 48.0),
    ));
    m.append(&TriMesh::cylinder(
        Point3::new(40.0, 21.0, 8.0),
        10.0,
        12.0,
        64,
    ));
    m
}

fn sample(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    add(s, "Bracket", sample_mesh())
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let ids: Vec<u32> = s.scene.bodies.iter().map(|b| b.id).collect();
    let bodies: Vec<Value> = ids.into_iter().map(|id| summary(s, id)).collect();
    Ok(json!({ "bodies": bodies }))
}

fn body_id(s: &Session, p: &Value) -> Result<u32, CommandError> {
    let id = params::req_id(p, "body")?;
    if s.scene.body(id).is_none() {
        return Err(CommandError::BadParams(format!("no body {id}")));
    }
    Ok(id)
}

fn visibility(s: &mut Session, p: &Value) -> CmdResult {
    let id = body_id(s, p)?;
    let visible = params::req_bool(p, "visible")?;
    if let Some(b) = s.scene.body_mut(id) {
        b.visible = visible;
    }
    if !visible {
        s.scene.deselect(|b| b == id);
    }
    Ok(summary(s, id))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let id = body_id(s, p)?;
    let name = params::req_display_name(p, "name")?;
    if let Some(solid) = s.solids().iter().find(|x| x.scene_id == Some(id)) {
        return Err(CommandError::Unavailable(format!(
            "{} is built by feature {}: rename the feature (feature.rename)",
            solid.name, solid.feature
        )));
    }
    if let Some(mesh) = s.mesh_of_scene(id) {
        s.doc.rename_imported_mesh(mesh, &name)?;
        s.sync_meshes();
    } else if let Some(b) = s.scene.body_mut(id) {
        b.name = name;
    }
    Ok(summary(s, id))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let id = body_id(s, p)?;
    if let Some(mesh) = s.mesh_of_scene(id) {
        s.doc.remove_imported_mesh(mesh)?;
        s.sync_meshes();
    }
    s.scene.bodies.retain(|b| b.id != id);
    s.scene.deselect(|b| b == id);
    Ok(json!({ "removed": id }))
}

//! `body.*`: bodies in the scene. In M1 these are mesh bodies (imported STL or a sample).

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::{Point3, TriMesh};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "body.import_stl",
        label: "Insert Mesh (STL)",
        params: "path: string (required) - an .stl file, binary or ASCII, in mm",
        mutates: false,
        run: import_stl,
    },
    CommandSpec {
        id: "body.import_stl_bytes",
        label: "Insert Mesh (STL)",
        params: "data: string (required) - the .stl file's contents (binary or ASCII, in mm), \
                 base64\nname: string (optional) - the body's name; default \"Mesh\"",
        mutates: false,
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
        params: "body: integer (required) - the body id",
        mutates: false,
        run: remove,
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
                "volume": b.mesh.signed_volume(),
                "bounds": bounds,
            })
        }
        None => Value::Null,
    }
}

fn add(s: &mut Session, name: &str, mesh: TriMesh) -> CmdResult {
    let id = s
        .scene
        .add(name, mesh)
        .ok_or_else(|| CommandError::Unavailable("the scene holds no more bodies".into()))?;
    Ok(summary(s, id))
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
    add(s, &name, mesh)
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
    add(s, &name, mesh)
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
    if !visible && s.scene.selection.is_some_and(|h| h.body == id) {
        s.scene.selection = None;
    }
    Ok(summary(s, id))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let id = body_id(s, p)?;
    s.scene.bodies.retain(|b| b.id != id);
    if s.scene.selection.is_some_and(|h| h.body == id) {
        s.scene.selection = None;
    }
    Ok(json!({ "removed": id }))
}

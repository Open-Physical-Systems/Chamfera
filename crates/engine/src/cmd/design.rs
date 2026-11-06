//! `design.*`: new, open and save, and exports (STEP, STL, 3MF, OBJ, glTF binary).
//!
//! Path access: the CLI runs as the user, so any path the user names is allowed. The MCP
//! server (M6) will restrict these to allowed folders before calling them.
//!
//! The `*_bytes` variants carry file contents as base64 text in the parameters and replies
//! instead of touching the file system: the browser app reads and writes files that way
//! (its file picker gives bytes, and it saves by offering a download).

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::Document;
use serde_json::{Value, json};
use std::path::Path;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "design.export_3mf",
        label: "Export 3MF",
        params: "path: string (required) - where to write the .3mf file (the selected bodies, \
                 else all shown bodies, with their colours, in mm)",
        mutates: false,
        run: export_3mf,
    },
    CommandSpec {
        id: "design.export_glb",
        label: "Export glTF Binary",
        params: "path: string (required) - where to write the .glb file (the selected bodies, \
                 else all shown bodies, with their colours; glTF is in metres, Y up)",
        mutates: false,
        run: export_glb,
    },
    CommandSpec {
        id: "design.export_obj",
        label: "Export OBJ",
        params: "path: string (required) - where to write the .obj file (the selected bodies, \
                 else all shown bodies, in mm); their colours go in a .mtl file of the same \
                 name next to it",
        mutates: false,
        run: export_obj,
    },
    CommandSpec {
        id: "design.export_step",
        label: "Export STEP",
        params: "path: string (required) - where to write the .step file (all solids)",
        mutates: false,
        run: export_step,
    },
    CommandSpec {
        id: "design.export_bytes",
        label: "Export",
        params: "format: string (required) - \"stl\" (binary, all shown solids), \"step\" (all \
                 solids), or \"3mf\", \"obj\" or \"glb\" (the selected bodies, else all shown \
                 bodies; OBJ without materials)\ntimestamp: string (optional) - the STEP header time, \
                 YYYY-MM-DDThh:mm:ss\nReplies with the file as base64 `data`; writes no file",
        mutates: false,
        run: export_bytes,
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
        params: "length: string (optional) - the new design's length unit: mm (default), cm, m, \
                 in or ft",
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
        id: "design.open_bytes",
        label: "Open Design",
        params: "data: string (required) - the .chamfera file's contents, base64",
        mutates: false,
        run: open_bytes,
    },
    CommandSpec {
        id: "design.save",
        label: "Save Design",
        params: "path: string (optional) - where to write the .chamfera file; default the file it \
                 was opened from or last saved to",
        mutates: false,
        run: save,
    },
    CommandSpec {
        id: "design.save_bytes",
        label: "Save Design",
        params: "(none) - replies with the .chamfera file as base64 `data` and counts the design \
                 as saved; writes no file",
        mutates: false,
        run: save_bytes,
    },
    CommandSpec {
        id: "design.units",
        label: "Design Units",
        params: "length: string (optional) - mm, cm, m, in or ft: the unit values are shown in \
                 and a plain typed number means\nprecision: integer (optional) - decimal places \
                 for lengths, 0 to 8\nangle_precision: integer (optional) - decimal places for \
                 angles, 0 to 8\nThe model keeps its size: stored expressions without a unit \
                 get the old one written in. Replies with the units",
        mutates: true,
        run: units,
    },
];

fn length_unit(p: &Value) -> Result<Option<chamfera_doc::LengthUnit>, CommandError> {
    params::opt_str(p, "length")?
        .map(|u| {
            chamfera_doc::LengthUnit::parse(u).ok_or_else(|| {
                CommandError::BadParams(format!(
                    "unknown length unit {u:?}: use mm, cm, m, in or ft"
                ))
            })
        })
        .transpose()
}

fn decimals(p: &Value, key: &str) -> Result<Option<u8>, CommandError> {
    match params::opt_f64(p, key)? {
        None => Ok(None),
        Some(v)
            if v.fract() == 0.0
                && (0.0..=f64::from(chamfera_doc::units::MAX_DECIMALS)).contains(&v) =>
        {
            Ok(Some(v as u8))
        }
        Some(_) => Err(CommandError::BadParams(format!(
            "`{key}` must be a whole number from 0 to {}",
            chamfera_doc::units::MAX_DECIMALS
        ))),
    }
}

fn units_reply(s: &Session) -> Value {
    let u = s.doc.units;
    json!({ "length": u.length.symbol(), "precision": u.length_decimals,
            "angle_precision": u.angle_decimals })
}

fn units(s: &mut Session, p: &Value) -> CmdResult {
    let mut u = s.doc.units;
    if let Some(l) = length_unit(p)? {
        u.length = l;
    }
    if let Some(d) = decimals(p, "precision")? {
        u.length_decimals = d;
    }
    if let Some(d) = decimals(p, "angle_precision")? {
        u.angle_decimals = d;
    }
    s.doc.set_units(u)?;
    // The sizes are unchanged; evaluating checks every expression still reads.
    s.doc.evaluate()?;
    Ok(units_reply(s))
}

fn new(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let mut doc = Document::default();
    if p.is_object()
        && let Some(l) = length_unit(p)?
    {
        doc.units.length = l;
    }
    s.replace_document(doc);
    Ok(units_reply(s))
}

fn open(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let doc = chamfera_io::open_design(Path::new(path))?;
    let count = doc.parameters.len();
    s.replace_document(doc);
    s.path = Some(Path::new(path).to_path_buf());
    Ok(json!({ "path": path, "parameters": count }))
}

fn open_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let bytes = decode_base64(params::req_str(p, "data")?, chamfera_io::MAX_DESIGN_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| chamfera_io::IoError::Format("a design file is UTF-8 text".into()))?;
    let doc = Document::from_json(&text)?;
    let count = doc.parameters.len();
    s.replace_document(doc);
    Ok(json!({ "parameters": count, "bytes": text.len() }))
}

fn save_bytes(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let text = s
        .doc
        .to_json()
        .map_err(|e| chamfera_io::IoError::Format(e.to_string()))?;
    s.saved = s.doc.clone();
    Ok(json!({ "data": encode_base64(text.as_bytes()), "bytes": text.len() }))
}

fn export_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let (bytes, key, count) = match params::req_str(p, "format")? {
        "stl" => {
            let mesh = shown_solids_mesh(s)?;
            let bytes = chamfera_io::stl::write_binary(&mesh)?;
            (bytes, "triangles", mesh.triangles.len())
        }
        "step" => {
            let timestamp = params::opt_str(p, "timestamp")?;
            if timestamp.is_some_and(|t| !is_timestamp(t)) {
                return Err(CommandError::BadParams(
                    "`timestamp` is YYYY-MM-DDThh:mm:ss".into(),
                ));
            }
            let bodies = solid_bodies(s)?;
            let options = chamfera_io::step::StepOptions {
                // The header names the file the user will get: the design's name.
                file_name: s
                    .path
                    .as_ref()
                    .map(|_| format!("{}.step", product_name(s)))
                    .unwrap_or_default(),
                product_name: product_name(s),
                timestamp: timestamp.map(str::to_owned),
                ..Default::default()
            };
            let text = chamfera_io::step::write(&bodies, &options)?;
            (text.into_bytes(), "solids", bodies.len())
        }
        format @ ("3mf" | "obj" | "glb") => {
            let bodies = mesh_bodies(s)?;
            let list = as_mesh_bodies(&bodies);
            let bytes = match format {
                "3mf" => chamfera_io::threemf::write(&list)?,
                "obj" => chamfera_io::obj::write(&list, None)?.0.into_bytes(),
                _ => chamfera_io::gltf::write_glb(&list)?,
            };
            (bytes, "bodies", bodies.len())
        }
        other => {
            return Err(CommandError::BadParams(format!(
                "`format` is \"stl\", \"step\", \"3mf\", \"obj\" or \"glb\", not {other:?}"
            )));
        }
    };
    Ok(json!({ "data": encode_base64(&bytes), "bytes": bytes.len(), key: count }))
}

/// True for `YYYY-MM-DDThh:mm:ss` (digits in place; ranges are not checked).
fn is_timestamp(t: &str) -> bool {
    let pattern = b"dddd-dd-ddTdd:dd:dd";
    t.len() == pattern.len()
        && t.bytes().zip(pattern.iter()).all(|(c, &p)| match p {
            b'd' => c.is_ascii_digit(),
            _ => c == p,
        })
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
    let bodies = solid_bodies(s)?;
    let options = chamfera_io::step::StepOptions {
        product_name: product_name(s),
        ..Default::default()
    };
    chamfera_io::step::write_file(Path::new(path), &bodies, &options)?;
    Ok(json!({ "path": path, "solids": bodies.len() }))
}

/// Every solid with its name, for STEP.
fn solid_bodies(s: &Session) -> Result<Vec<(&str, &chamfera_kernel::Body)>, CommandError> {
    if s.solids().is_empty() {
        return Err(CommandError::Unavailable(
            "there are no solids to export".into(),
        ));
    }
    Ok(s.solids()
        .iter()
        .map(|x| (x.name.as_str(), &x.body))
        .collect())
}

/// The STEP product name: the design's file name, if it has one.
fn product_name(s: &Session) -> String {
    s.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
        || "Chamfera design".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The meshes of every shown solid, as one, for STL.
fn shown_solids_mesh(s: &Session) -> Result<chamfera_geom::TriMesh, CommandError> {
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
    Ok(mesh)
}

fn export_stl(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let mesh = shown_solids_mesh(s)?;
    let bytes = chamfera_io::stl::write_binary(&mesh)?;
    std::fs::write(path, bytes).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "triangles": mesh.triangles.len() }))
}

/// The bodies a mesh export writes: the selected bodies, else every shown body, each with
/// its name, mesh and colour (`body.appearance`).
fn mesh_bodies(
    s: &Session,
) -> Result<Vec<(String, chamfera_geom::TriMesh, [u8; 3])>, CommandError> {
    let chosen: Vec<&chamfera_render::SceneBody> = if s.scene.selected.is_empty() {
        s.scene.bodies.iter().filter(|b| b.visible).collect()
    } else {
        s.scene
            .selected
            .iter()
            .filter_map(|id| s.scene.body(*id))
            .collect()
    };
    // A body's own appearance colour, else the light theme's body colour (with its
    // component tint), as the viewport shows it.
    let theme = chamfera_render::Theme::light();
    let out: Vec<_> = chosen
        .into_iter()
        .filter(|b| !b.mesh.triangles.is_empty())
        .map(|b| {
            let c = b
                .appearance
                .colour
                .unwrap_or_else(|| theme.body_colour(b.tint));
            (b.name.clone(), b.mesh.clone(), [c.0, c.1, c.2])
        })
        .collect();
    if out.is_empty() {
        return Err(CommandError::Unavailable(
            "there are no shown bodies to export".into(),
        ));
    }
    Ok(out)
}

fn as_mesh_bodies(
    bodies: &[(String, chamfera_geom::TriMesh, [u8; 3])],
) -> Vec<chamfera_io::mesh::MeshBody<'_>> {
    bodies
        .iter()
        .map(|(name, mesh, colour)| chamfera_io::mesh::MeshBody {
            name,
            mesh,
            colour: *colour,
        })
        .collect()
}

fn export_3mf(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bodies = mesh_bodies(s)?;
    let bytes = chamfera_io::threemf::write(&as_mesh_bodies(&bodies))?;
    std::fs::write(path, bytes).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "bodies": bodies.len() }))
}

fn export_glb(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let bodies = mesh_bodies(s)?;
    let bytes = chamfera_io::gltf::write_glb(&as_mesh_bodies(&bodies))?;
    std::fs::write(path, bytes).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "bodies": bodies.len() }))
}

fn export_obj(s: &mut Session, p: &Value) -> CmdResult {
    let path = Path::new(params::req_str(p, "path")?);
    let bodies = mesh_bodies(s)?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mtl"))
    {
        return Err(CommandError::BadParams(
            "`path` is the .obj file; its materials go in a .mtl file beside it".into(),
        ));
    }
    let mtl_path = path.with_extension("mtl");
    // The path was checked (over MCP, canonicalized into an allowed folder); its sibling
    // was not, so it must not lead elsewhere through a link.
    if let Ok(meta) = std::fs::symlink_metadata(&mtl_path)
        && !meta.is_file()
    {
        return Err(CommandError::Unavailable(format!(
            "{} exists and is not a plain file, so the materials cannot be written there",
            mtl_path.display()
        )));
    }
    let mtl_name = mtl_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| CommandError::BadParams("`path` must name a file".into()))?;
    let (obj, mtl) = chamfera_io::obj::write(&as_mesh_bodies(&bodies), Some(&mtl_name))?;
    std::fs::write(path, obj).map_err(chamfera_io::IoError::from)?;
    std::fs::write(&mtl_path, mtl).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "materials": mtl_path, "bodies": bodies.len() }))
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648, section 4) with `=` padding.
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(BASE64[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes standard base64 (RFC 4648, section 4). ASCII whitespace is skipped; padding is
/// optional but, if present, must be at the end. Refuses anything else, and output longer
/// than `limit` bytes.
pub(crate) fn decode_base64(text: &str, limit: u64) -> Result<Vec<u8>, CommandError> {
    let bad = |why: &str| CommandError::BadParams(format!("`data` is not base64: {why}"));
    let approx = (text.len() as u64 / 4).saturating_mul(3);
    if approx > limit.saturating_add(3) {
        return Err(CommandError::BadParams(format!(
            "`data` is larger than the limit of {limit} bytes"
        )));
    }
    let mut out = Vec::with_capacity(usize::try_from(approx).unwrap_or(0));
    let (mut acc, mut bits, mut pad) = (0u32, 0u32, 0usize);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return Err(bad("a character outside the alphabet")),
        };
        if pad > 0 {
            return Err(bad("data after the padding"));
        }
        acc = ((acc << 6) | u32::from(v)) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // A last group of one character holds less than a byte; padding is at most two.
    if bits >= 6 || pad > 2 {
        return Err(bad("the length is wrong"));
    }
    if out.len() as u64 > limit {
        return Err(CommandError::BadParams(format!(
            "`data` is larger than the limit of {limit} bytes"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, coded) in vectors {
            assert_eq!(encode_base64(plain.as_bytes()), coded);
            assert_eq!(decode_base64(coded, 100).unwrap(), plain.as_bytes());
            // Padding is optional, line breaks are skipped.
            let bare = coded.trim_end_matches('=');
            assert_eq!(decode_base64(bare, 100).unwrap(), plain.as_bytes());
        }
        assert_eq!(decode_base64("Zm9v\r\nYmFy", 100).unwrap(), b"foobar");
        // Every byte value survives a round trip.
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_base64(&encode_base64(&all), 256).unwrap(), all);
    }

    #[test]
    fn broken_base64_is_refused() {
        for bad in ["Z", "Zm9vY", "Zg===", "Zg==Zg", "Zm9v!", "Zm9v\u{e9}"] {
            assert!(decode_base64(bad, 100).is_err(), "{bad:?}");
        }
        // Over the limit, whether decoded or estimated from the length.
        assert!(decode_base64("Zm9vYmFy", 5).is_err());
        assert!(decode_base64(&"A".repeat(4000), 100).is_err());
    }

    /// A 50 x 30 rectangle extruded 10: one box solid of volume 15000.
    fn boxed(s: &mut Session) {
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 25, "y": 15}], "distance": 10}),
        )
        .unwrap();
    }

    #[test]
    fn a_design_round_trips_through_bytes() {
        let mut s = Session::new();
        boxed(&mut s);
        s.execute("param.set", &json!({"name": "w", "value": 12}))
            .unwrap();
        assert!(s.has_unsaved_changes());
        let r = s.execute("design.save_bytes", &json!({})).unwrap();
        assert!(!s.has_unsaved_changes(), "a download counts as a save");
        assert_eq!(s.path, None, "no file was named");
        let data = r["data"].as_str().unwrap();
        let text = String::from_utf8(decode_base64(data, u64::MAX).unwrap()).unwrap();
        assert_eq!(r["bytes"], text.len());
        assert_eq!(text, s.doc.to_json().unwrap());

        let mut t = Session::new();
        let r = t
            .execute("design.open_bytes", &json!({"data": data}))
            .unwrap();
        assert_eq!(r["parameters"], 1);
        assert_eq!(t.doc, s.doc);
        assert!(!t.has_unsaved_changes());
        assert_eq!(t.solids().len(), 1, "opening rebuilds the solids");
    }

    #[test]
    fn exports_come_back_as_bytes() {
        let mut s = Session::new();
        assert!(
            s.execute("design.export_bytes", &json!({"format": "stl"}))
                .is_err(),
            "nothing to export"
        );
        boxed(&mut s);
        // STL: the 12 triangles of the box; read back, volume 50 * 30 * 10.
        let r = s
            .execute("design.export_bytes", &json!({"format": "stl"}))
            .unwrap();
        assert_eq!(r["triangles"], 12);
        let bytes = decode_base64(r["data"].as_str().unwrap(), u64::MAX).unwrap();
        assert_eq!(
            bytes.len(),
            84 + 12 * 50,
            "binary STL: header and count, then 50 bytes per triangle"
        );
        let mesh = chamfera_io::stl::read(&bytes).unwrap();
        assert!((mesh.signed_volume() - 15_000.0).abs() < 1e-3);
        // STEP: a Part 21 file with the six faces of the box and the time given.
        let r = s
            .execute(
                "design.export_bytes",
                &json!({"format": "step", "timestamp": "2026-11-06T12:00:00"}),
            )
            .unwrap();
        assert_eq!(r["solids"], 1);
        let text = String::from_utf8(decode_base64(r["data"].as_str().unwrap(), u64::MAX).unwrap())
            .unwrap();
        assert!(text.starts_with("ISO-10303-21;"));
        assert!(text.contains("2026-11-06T12:00:00"));
        assert_eq!(text.matches("ADVANCED_FACE(").count(), 6);
        // A design with a name (the browser sets one when it opens a file) names the file
        // and the product after it.
        s.path = Some("plate.chamfera".into());
        let r = s
            .execute("design.export_bytes", &json!({"format": "step"}))
            .unwrap();
        let text = String::from_utf8(decode_base64(r["data"].as_str().unwrap(), u64::MAX).unwrap())
            .unwrap();
        assert!(text.contains("FILE_NAME('plate.step'"), "{text}");
        assert!(text.contains("PRODUCT('plate'"));
        // Bad formats and parameters are refused.
        for p in [
            json!({}),
            json!({"format": "dwg"}),
            json!({"format": 1}),
            json!({"format": "step", "timestamp": "x".repeat(100)}),
            json!({"format": "step", "timestamp": "\u{0}"}),
        ] {
            assert!(s.execute("design.export_bytes", &p).is_err(), "{p}");
        }
    }

    #[test]
    fn hostile_design_bytes_are_refused_and_change_nothing() {
        let mut s = Session::new();
        boxed(&mut s);
        let before = s.doc.clone();
        let not_utf8 = encode_base64(&[0xff, 0xfe, 0x00, 0x80]);
        let not_json = encode_base64(b"hello");
        let wrong_json = encode_base64(br#"{"format": "something else", "version": 999}"#);
        let deep = encode_base64("[".repeat(100_000).as_bytes());
        for p in [
            json!({}),
            json!({"data": null}),
            json!({"data": 5}),
            json!({"data": ["Zm9v"]}),
            json!({"data": "not base64!"}),
            json!({"data": not_utf8}),
            json!({"data": not_json}),
            json!({"data": wrong_json}),
            json!({"data": deep}),
        ] {
            assert!(s.execute("design.open_bytes", &p).is_err(), "{p}");
            assert_eq!(s.doc, before);
            assert_eq!(s.solids().len(), 1);
        }
    }

    #[test]
    fn an_stl_is_inserted_from_bytes() {
        // A 10 x 20 x 30 box: 12 triangles, volume 6000, area 2 * (200 + 600 + 300).
        let mesh = chamfera_geom::TriMesh::cuboid(
            chamfera_geom::Point3::ORIGIN,
            chamfera_geom::Point3::new(10.0, 20.0, 30.0),
        );
        let data = encode_base64(&chamfera_io::stl::write_binary(&mesh).unwrap());
        let mut s = Session::new();
        let r = s
            .execute(
                "body.import_stl_bytes",
                &json!({"data": data, "name": "block.stl"}),
            )
            .unwrap();
        assert_eq!(r["name"], "block");
        assert_eq!(r["triangles"], 12);
        assert!((r["volume"].as_f64().unwrap() - 6000.0).abs() < 1e-9);
        assert!((r["area"].as_f64().unwrap() - 2200.0).abs() < 1e-9);
        // ASCII works too, and a missing name becomes "Mesh".
        let ascii = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\n\
                     vertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let r = s
            .execute(
                "body.import_stl_bytes",
                &json!({"data": encode_base64(ascii.as_bytes())}),
            )
            .unwrap();
        assert_eq!(r["name"], "Mesh");
        assert_eq!(r["triangles"], 1);
        assert_eq!(s.scene.bodies.len(), 2);
    }

    #[test]
    fn hostile_stl_bytes_are_refused() {
        let mut s = Session::new();
        // A binary header that claims a billion triangles but has none.
        let mut lying = vec![0u8; 84];
        lying[80..84].copy_from_slice(&1_000_000_000u32.to_le_bytes());
        for p in [
            json!({}),
            json!({"data": 7}),
            json!({"data": "@@@@"}),
            json!({"data": ""}),
            json!({"data": encode_base64(b"just some text")}),
            json!({"data": encode_base64(&lying)}),
            json!({"data": encode_base64(&[0u8; 84]), "name": 5}),
        ] {
            assert!(s.execute("body.import_stl_bytes", &p).is_err(), "{p}");
        }
        assert!(s.scene.bodies.is_empty());
        // A hostile name is cut short and loses control characters.
        let mesh = chamfera_geom::TriMesh::cuboid(
            chamfera_geom::Point3::ORIGIN,
            chamfera_geom::Point3::new(1.0, 1.0, 1.0),
        );
        let data = encode_base64(&chamfera_io::stl::write_binary(&mesh).unwrap());
        let name = format!("a\u{0}b{}", "x".repeat(1000));
        let r = s
            .execute(
                "body.import_stl_bytes",
                &json!({"data": data, "name": name}),
            )
            .unwrap();
        let got = r["name"].as_str().unwrap();
        assert!(got.starts_with("abx") && got.chars().count() == 64, "{got}");
    }

    #[test]
    fn mesh_exports_read_back_as_the_box() {
        let mut s = Session::new();
        for format in ["3mf", "obj", "glb"] {
            assert!(
                s.execute("design.export_bytes", &json!({ "format": format }))
                    .is_err(),
                "nothing to export as {format}"
            );
        }
        boxed(&mut s);
        // The body's appearance colour goes into the file.
        let body = s.execute("body.list", &json!({})).unwrap()["bodies"][0]["body"].clone();
        s.execute(
            "body.appearance",
            &json!({ "body": body, "colour": "#ff2000" }),
        )
        .unwrap();
        // 3MF: inserted again as a mesh body of 12 triangles and volume 50 * 30 * 10.
        let data = bytes_of(&mut s, "3mf");
        let zip_bytes = decode_base64(&data, u64::MAX).unwrap();
        let zip = chamfera_io::zip::Archive::parse(&zip_bytes).unwrap();
        let model = zip
            .read(zip.find("3D/3dmodel.model").unwrap(), 1 << 20)
            .unwrap();
        assert!(
            String::from_utf8(model)
                .unwrap()
                .contains("displaycolor=\"#FF2000\"")
        );
        let r = s
            .execute("body.import_3mf_bytes", &json!({ "data": data }))
            .unwrap();
        assert_eq!(r["triangles"], 12);
        assert!((r["bodies"][0]["volume"].as_f64().unwrap() - 15_000.0).abs() < 1e-6);
        // OBJ: the same.
        let data = bytes_of(&mut s, "obj");
        let r = s
            .execute(
                "body.import_obj_bytes",
                &json!({ "data": data, "name": "copy.obj" }),
            )
            .unwrap();
        assert_eq!(r["name"], "copy");
        assert_eq!(r["triangles"], 24, "the box and the 3MF copy, both shown");
        assert!((r["volume"].as_f64().unwrap() - 30_000.0).abs() < 1e-6);
        // glTF binary: starts with its magic.
        let glb = decode_base64(&bytes_of(&mut s, "glb"), u64::MAX).unwrap();
        assert_eq!(&glb[0..4], b"glTF");
    }

    fn bytes_of(s: &mut Session, format: &str) -> String {
        let r = s
            .execute("design.export_bytes", &json!({ "format": format }))
            .unwrap();
        r["data"].as_str().unwrap().to_owned()
    }

    #[test]
    fn mesh_files_are_written_and_read_by_path() {
        let mut s = Session::new();
        boxed(&mut s);
        let dir = std::env::temp_dir().join(format!("chamfera-formats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = |n: &str| dir.join(n).to_string_lossy().into_owned();
        s.execute("design.export_3mf", &json!({ "path": path("b.3mf") }))
            .unwrap();
        s.execute("design.export_glb", &json!({ "path": path("b.glb") }))
            .unwrap();
        let r = s
            .execute("design.export_obj", &json!({ "path": path("b.obj") }))
            .unwrap();
        assert!(
            std::fs::read_to_string(dir.join("b.mtl"))
                .unwrap()
                .contains("newmtl")
        );
        assert!(r["materials"].as_str().unwrap().ends_with("b.mtl"));
        assert!(
            s.execute("design.export_obj", &json!({ "path": path("b.mtl") }))
                .is_err()
        );
        let r = s
            .execute("body.import_3mf", &json!({ "path": path("b.3mf") }))
            .unwrap();
        assert_eq!(r["triangles"], 12);
        let r = s
            .execute("body.import_obj", &json!({ "path": path("b.obj") }))
            .unwrap();
        assert_eq!(r["name"], "b");
        // The OBJ was written before the 3MF copy came in: the box's 12 triangles.
        assert_eq!(r["triangles"], 12);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hostile_mesh_files_are_refused() {
        let mut s = Session::new();
        let zip =
            chamfera_io::zip::write_stored(&[("3D/3dmodel.model", b"<model><build/></model>")])
                .unwrap();
        for (cmd, data) in [
            ("body.import_3mf_bytes", encode_base64(&zip)),
            (
                "body.import_3mf_bytes",
                encode_base64(&zip[..zip.len() / 2]),
            ),
            ("body.import_3mf_bytes", encode_base64(b"PK")),
            (
                "body.import_obj_bytes",
                encode_base64(b"v nan 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"),
            ),
            ("body.import_obj_bytes", encode_base64(b"f 1 2 3\n")),
        ] {
            assert!(s.execute(cmd, &json!({ "data": data })).is_err(), "{cmd}");
        }
        assert!(s.scene.bodies.is_empty());
    }
}

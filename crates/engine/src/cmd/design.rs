//! `design.*`: new, open and save.
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
        id: "design.export_step",
        label: "Export STEP",
        params: "path: string (required) - where to write the .step file (all solids)",
        mutates: false,
        run: export_step,
    },
    CommandSpec {
        id: "design.export_bytes",
        label: "Export",
        params: "format: string (required) - \"stl\" (binary, all shown solids) or \"step\" (all \
                 solids)\ntimestamp: string (optional) - the STEP header time, \
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
        other => {
            return Err(CommandError::BadParams(format!(
                "`format` is \"stl\" or \"step\", not {other:?}"
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
            json!({"format": "obj"}),
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
}

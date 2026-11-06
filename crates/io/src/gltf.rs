//! glTF 2.0 binary (`.glb`) export, from the public Khronos glTF 2.0 specification.
//!
//! One node and mesh per body, each with positions, normals (smooth across gentle bends,
//! sharp at creases), 32-bit indices and a material with the body's colour. glTF is in
//! metres with Y up, so millimetres are scaled by 0.001 and Chamfera's Z-up axes are
//! turned: (x, y, z) becomes (x, z, -y).

use crate::IoError;
use crate::mesh::{MeshBody, normals};
use serde_json::{Value, json};

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("glTF: {why}"))
}

/// `GLB_MAGIC` is "glTF" read as a little-endian u32.
pub const GLB_MAGIC: u32 = 0x4654_6C67;
/// Chunk type "JSON".
pub const CHUNK_JSON: u32 = 0x4E4F_534A;
/// Chunk type "BIN\0".
pub const CHUNK_BIN: u32 = 0x004E_4942;

/// sRGB 0..255 to linear 0..1 (the sRGB transfer function, IEC 61966-2-1).
fn linear(c: u8) -> f64 {
    let s = f64::from(c) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Writes a binary glTF.
pub fn write_glb(bodies: &[MeshBody]) -> Result<Vec<u8>, IoError> {
    if bodies.is_empty() {
        return Err(bad("there is nothing to write"));
    }
    let mut bin: Vec<u8> = Vec::new();
    let (mut views, mut accessors, mut meshes, mut nodes, mut materials) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for b in bodies {
        b.mesh
            .validate()
            .map_err(|e| bad(format!("{}: {e}", b.name)))?;
        let nm = normals(b.mesh);
        if nm.corners.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let pos_at = bin.len();
        for (v, _) in &nm.corners {
            let p = b
                .mesh
                .positions
                .get(*v as usize)
                .ok_or_else(|| bad("a corner has no vertex"))?;
            let q = [
                (p.x * 0.001) as f32,
                (p.z * 0.001) as f32,
                (-p.y * 0.001) as f32,
            ];
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
                bin.extend_from_slice(&q[k].to_le_bytes());
            }
        }
        let nrm_at = bin.len();
        for (_, n) in &nm.corners {
            for v in [n.x, n.z, -n.y] {
                bin.extend_from_slice(&(v as f32).to_le_bytes());
            }
        }
        let idx_at = bin.len();
        for t in &nm.triangles {
            for c in t {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        let end = bin.len();
        let v0 = views.len();
        views.push(json!({"buffer": 0, "byteOffset": pos_at, "byteLength": nrm_at - pos_at, "target": 34962}));
        views.push(json!({"buffer": 0, "byteOffset": nrm_at, "byteLength": idx_at - nrm_at, "target": 34962}));
        views.push(
            json!({"buffer": 0, "byteOffset": idx_at, "byteLength": end - idx_at, "target": 34963}),
        );
        let a0 = accessors.len();
        let count = nm.corners.len();
        accessors.push(
            json!({"bufferView": v0, "componentType": 5126, "count": count, "type": "VEC3",
            "min": lo, "max": hi}),
        );
        accessors.push(
            json!({"bufferView": v0 + 1, "componentType": 5126, "count": count, "type": "VEC3"}),
        );
        accessors.push(json!({"bufferView": v0 + 2, "componentType": 5125,
            "count": nm.triangles.len() * 3, "type": "SCALAR"}));
        let [r, g, bl] = b.colour.map(linear);
        materials.push(json!({
            "name": b.name,
            "pbrMetallicRoughness": {"baseColorFactor": [r, g, bl, 1.0],
                "metallicFactor": 0.0, "roughnessFactor": 0.6},
        }));
        meshes.push(json!({"name": b.name, "primitives": [{
            "attributes": {"POSITION": a0, "NORMAL": a0 + 1},
            "indices": a0 + 2, "material": materials.len() - 1, "mode": 4}]}));
        nodes.push(json!({"name": b.name, "mesh": meshes.len() - 1}));
    }
    if meshes.is_empty() {
        return Err(bad("the bodies have no triangles"));
    }
    let node_ids: Vec<usize> = (0..nodes.len()).collect();
    let doc: Value = json!({
        "asset": {"version": "2.0", "generator": "Chamfera"},
        "scene": 0,
        "scenes": [{"nodes": node_ids}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
    });
    let mut text = serde_json::to_vec(&doc).map_err(|e| bad(e.to_string()))?;
    // Chunks are 4-byte aligned: JSON padded with spaces, binary with zeros.
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let to32 = |n: usize| u32::try_from(n).map_err(|_| bad("the file would be over 4 GB"));
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&to32(total)?.to_le_bytes());
    out.extend_from_slice(&to32(text.len())?.to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&text);
    out.extend_from_slice(&to32(bin.len())?.to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(&bin);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::{Point3, TriMesh};

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn the_header_and_chunks_are_valid_and_the_json_parses() {
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(50.0, 30.0, 10.0));
        let glb = write_glb(&[MeshBody {
            name: "Box",
            mesh: &m,
            colour: [255, 0, 0],
        }])
        .unwrap();
        // Header: "glTF", version 2, the whole length.
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32_at(&glb, 4), 2);
        assert_eq!(u32_at(&glb, 8) as usize, glb.len());
        let json_len = u32_at(&glb, 12) as usize;
        assert_eq!(&glb[16..20], b"JSON");
        assert_eq!(json_len % 4, 0);
        let doc: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
        let bin_at = 20 + json_len;
        let bin_len = u32_at(&glb, bin_at) as usize;
        assert_eq!(&glb[bin_at + 4..bin_at + 8], b"BIN\0");
        assert_eq!(bin_at + 8 + bin_len, glb.len());
        assert_eq!(doc["asset"]["version"], "2.0");
        // A box: 24 corners (3 normals at each of 8 corners), 12 triangles.
        // Positions 24 * 12 bytes, normals 24 * 12, indices 36 * 4: 720 bytes.
        assert_eq!(doc["accessors"][0]["count"], 24);
        assert_eq!(doc["accessors"][2]["count"], 36);
        assert_eq!(doc["buffers"][0]["byteLength"], 720);
        assert_eq!(bin_len, 720);
        // 50 mm along x is 0.05 m; 30 mm along chamfera y is -0.03 m along glTF z;
        // 10 mm up is 0.01 m along glTF y.
        let max = &doc["accessors"][0]["max"];
        let min = &doc["accessors"][0]["min"];
        assert!((max[0].as_f64().unwrap() - 0.05).abs() < 1e-7);
        assert!((max[1].as_f64().unwrap() - 0.01).abs() < 1e-7);
        assert!((min[2].as_f64().unwrap() + 0.03).abs() < 1e-7);
        // Pure red stays pure red in linear light.
        assert_eq!(
            doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"][0],
            1.0
        );
        assert_eq!(
            doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"][1],
            0.0
        );
    }

    #[test]
    fn nothing_to_write_is_an_error() {
        assert!(write_glb(&[]).is_err());
        let empty = TriMesh::default();
        assert!(
            write_glb(&[MeshBody {
                name: "e",
                mesh: &empty,
                colour: [0; 3]
            }])
            .is_err()
        );
    }
}

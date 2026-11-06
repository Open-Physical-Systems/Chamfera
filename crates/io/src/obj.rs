//! Wavefront OBJ import and export (the public OBJ and MTL format descriptions).
//!
//! Export writes one `o` group per body: its vertices, vertex normals that are smooth
//! across gentle bends and sharp at creases (`crate::mesh::normals`), and its faces, with
//! a material per body in a companion `.mtl` text for its colour. Import reads `v` and `f`
//! lines (any of `v`, `v/vt`, `v//vn`, `v/vt/vn`, negative indices counted from the end),
//! fans polygons into triangles, and ignores everything else. All counts are capped and
//! every coordinate and index is checked.

use crate::IoError;
use crate::mesh::{MeshBody, normals};
use chamfera_geom::{MAX_TRIANGLES, Point3, TriMesh};
use std::fmt::Write as _;

/// Largest OBJ file accepted (bytes).
pub const MAX_OBJ_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Most corners one face may have.
const MAX_FACE_CORNERS: usize = 4096;

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("OBJ: {why}"))
}

/// A material name: letters, digits and `_` only, so it needs no quoting.
fn material_name(i: usize, name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    format!("m{i}_{clean}")
}

/// Writes the OBJ text, and the MTL text it names as `mtl_name` (pass the file name the
/// MTL text will be saved as, next to the OBJ; `None` writes no materials).
pub fn write(bodies: &[MeshBody], mtl_name: Option<&str>) -> Result<(String, String), IoError> {
    if bodies.is_empty() {
        return Err(bad("there is nothing to write"));
    }
    let mut obj = String::from("# Chamfera OBJ export, millimetres, Z up\n");
    let mut mtl = String::from("# Chamfera materials\n");
    if let Some(m) = mtl_name {
        let m: String = m.chars().filter(|c| !c.is_control()).collect();
        let _ = writeln!(obj, "mtllib {m}");
    }
    let (mut v_base, mut n_base) = (1usize, 1usize);
    for (i, b) in bodies.iter().enumerate() {
        b.mesh
            .validate()
            .map_err(|e| bad(format!("{}: {e}", b.name)))?;
        let name: String = b.name.chars().filter(|c| !c.is_control()).collect();
        let _ = writeln!(obj, "o {name}");
        for p in &b.mesh.positions {
            let _ = writeln!(obj, "v {} {} {}", p.x, p.y, p.z);
        }
        let nm = normals(b.mesh);
        for (_, n) in &nm.corners {
            let _ = writeln!(obj, "vn {} {} {}", n.x, n.y, n.z);
        }
        if mtl_name.is_some() {
            let m = material_name(i, &name);
            let _ = writeln!(obj, "usemtl {m}");
            let [r, g, bl] = b.colour.map(|c| f64::from(c) / 255.0);
            let _ = writeln!(
                mtl,
                "newmtl {m}\nKd {r:.4} {g:.4} {bl:.4}\nKa 0 0 0\nd 1\nillum 1"
            );
        }
        for tri in &nm.triangles {
            obj.push('f');
            for &c in tri {
                let Some((v, _)) = nm.corners.get(c as usize) else {
                    continue;
                };
                let _ = write!(obj, " {}//{}", *v as usize + v_base, c as usize + n_base);
            }
            obj.push('\n');
        }
        v_base += b.mesh.positions.len();
        n_base += nm.corners.len();
    }
    Ok((obj, mtl))
}

/// Resolves an OBJ index (1-based, or negative from the end) against `count` vertices.
fn index(token: &str, count: usize) -> Result<u32, IoError> {
    let first = token.split('/').next().unwrap_or("");
    let i: i64 = first
        .parse()
        .map_err(|_| bad(format!("face corner {token:?} is not a vertex index")))?;
    let at = if i > 0 {
        i - 1
    } else if i < 0 {
        count as i64 + i
    } else {
        return Err(bad("vertex index 0 does not exist (indices start at 1)"));
    };
    if at < 0 || at >= count as i64 {
        return Err(bad(format!(
            "face corner {token:?} refers to a vertex that is not defined yet"
        )));
    }
    u32::try_from(at).map_err(|_| bad("vertex index too large"))
}

/// Reads an OBJ file's vertices and faces into one validated mesh.
pub fn read(bytes: &[u8]) -> Result<TriMesh, IoError> {
    if bytes.len() as u64 > MAX_OBJ_BYTES {
        return Err(bad(format!(
            "the file is larger than {MAX_OBJ_BYTES} bytes"
        )));
    }
    let mut mesh = TriMesh::default();
    let mut corners: Vec<u32> = Vec::new();
    for (n, raw) in bytes.split(|&b| b == b'\n').enumerate() {
        let line = String::from_utf8_lossy(raw);
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tokens = line.split_ascii_whitespace();
        match tokens.next() {
            Some("v") => {
                if mesh.positions.len() >= MAX_TRIANGLES * 3 {
                    return Err(bad(format!("more than {} vertices", MAX_TRIANGLES * 3)));
                }
                let mut c = [0f64; 3];
                for v in &mut c {
                    *v = tokens
                        .next()
                        .and_then(|t| t.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| {
                            bad(format!(
                                "line {}: a vertex needs three finite numbers",
                                n + 1
                            ))
                        })?;
                }
                mesh.positions.push(Point3::new(c[0], c[1], c[2]));
            }
            Some("f") => {
                corners.clear();
                for t in tokens {
                    if corners.len() >= MAX_FACE_CORNERS {
                        return Err(bad(format!("line {}: a face has too many corners", n + 1)));
                    }
                    corners.push(index(t, mesh.positions.len())?);
                }
                if corners.len() < 3 {
                    return Err(bad(format!("line {}: a face needs three corners", n + 1)));
                }
                if mesh.triangles.len() + corners.len() - 2 > MAX_TRIANGLES {
                    return Err(bad(format!("more than {MAX_TRIANGLES} triangles")));
                }
                let first = corners[0];
                for w in corners[1..].windows(2) {
                    mesh.triangles.push([first, w[0], w[1]]);
                }
            }
            _ => {}
        }
    }
    mesh.validate().map_err(|e| bad(e.to_string()))?;
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::DEFAULT_COLOUR;

    #[test]
    fn a_box_round_trips_its_vertex_and_face_counts() {
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(50.0, 30.0, 10.0));
        let body = MeshBody {
            name: "Box 1",
            mesh: &m,
            colour: DEFAULT_COLOUR,
        };
        let (obj, mtl) = write(&[body, body], Some("box.mtl")).unwrap();
        // Two boxes of 8 vertices and 12 triangles; each box corner meets 3 faces at
        // right angles, so 24 normals a box.
        assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 16);
        assert_eq!(obj.lines().filter(|l| l.starts_with("vn ")).count(), 48);
        assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 24);
        assert!(obj.contains("mtllib box.mtl") && mtl.contains("Kd 0.7059 0.7059 0.7059"));
        let back = read(obj.as_bytes()).unwrap();
        assert_eq!(back.positions.len(), 16);
        assert_eq!(back.triangles.len(), 24);
        // 2 * 50 * 30 * 10.
        assert!((back.signed_volume() - 30000.0).abs() < 1e-9);
    }

    #[test]
    fn polygons_and_negative_indices_are_read() {
        let text = "v 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nvt 0 0\nf -4/1 -3/1 -2/1 -1/1 # a square\n";
        let m = read(text.as_bytes()).unwrap();
        assert_eq!(m.triangles.len(), 2);
        assert!((m.area() - 4.0).abs() < 1e-12);
    }

    #[test]
    fn hostile_files_are_refused_without_panicking() {
        let cases = [
            "",
            "f 1 2 3\n",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2\n",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -9 1 2\n",
            "v nan 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
            "v 1e300 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
            "v 1 2\n",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999\n",
        ];
        for c in cases {
            assert!(read(c.as_bytes()).is_err(), "{c:?}");
        }
        let _ = read(&[0xff, 0xfe, b'\n', b'v']);
    }
}

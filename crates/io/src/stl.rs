//! STL import and export (binary and ASCII), from the public format description.
//!
//! STL files are hostile input: sizes, counts and every coordinate are checked.

use crate::IoError;
use chamfera_geom::{MAX_TRIANGLES, Point3, TriMesh};

/// Largest STL file accepted (bytes).
pub const MAX_STL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Reads an STL file's bytes into a welded, validated mesh.
pub fn read(bytes: &[u8]) -> Result<TriMesh, IoError> {
    if bytes.len() as u64 > MAX_STL_BYTES {
        return Err(IoError::Format(format!(
            "the STL file is larger than {MAX_STL_BYTES} bytes"
        )));
    }
    let mut mesh = if is_binary(bytes) {
        read_binary(bytes)?
    } else {
        read_ascii(bytes)?
    };
    mesh.weld_exact();
    mesh.validate()
        .map_err(|e| IoError::Format(format!("STL: {e}")))?;
    Ok(mesh)
}

/// A binary STL is exactly 84 + 50 n bytes. ASCII files that happen to match are rare,
/// and an ASCII file must start with "solid", so check the size first.
fn is_binary(bytes: &[u8]) -> bool {
    let Some(count) = bytes.get(80..84) else {
        return false;
    };
    let n = u64::from(u32::from_le_bytes([count[0], count[1], count[2], count[3]]));
    84 + 50 * n == bytes.len() as u64
}

fn f32_at(bytes: &[u8], at: usize) -> Option<f64> {
    let b = bytes.get(at..at + 4)?;
    Some(f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
}

fn read_binary(bytes: &[u8]) -> Result<TriMesh, IoError> {
    let n = (bytes.len() - 84) / 50;
    if n > MAX_TRIANGLES {
        return Err(IoError::Format(format!(
            "STL has {n} triangles, the limit is {MAX_TRIANGLES}"
        )));
    }
    let mut mesh = TriMesh {
        positions: Vec::with_capacity(n * 3),
        triangles: Vec::with_capacity(n),
    };
    for i in 0..n {
        // Skip the stored normal (12 bytes); normals are recomputed from the winding.
        let base = 84 + i * 50 + 12;
        let first = mesh.positions.len() as u32;
        for v in 0..3 {
            let at = base + v * 12;
            let p = match (
                f32_at(bytes, at),
                f32_at(bytes, at + 4),
                f32_at(bytes, at + 8),
            ) {
                (Some(x), Some(y), Some(z)) => Point3::new(x, y, z),
                _ => return Err(IoError::Format(format!("STL triangle {i} is cut short"))),
            };
            mesh.positions.push(p);
        }
        mesh.triangles.push([first, first + 1, first + 2]);
    }
    Ok(mesh)
}

fn read_ascii(bytes: &[u8]) -> Result<TriMesh, IoError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| IoError::Format("not a binary STL, and not ASCII STL text".into()))?;
    if !text.trim_start().starts_with("solid") {
        return Err(IoError::Format("not an STL file".into()));
    }
    let mut mesh = TriMesh::default();
    let mut tokens = text.split_ascii_whitespace();
    while let Some(tok) = tokens.next() {
        if tok != "vertex" {
            continue;
        }
        let mut c = [0f64; 3];
        for v in &mut c {
            *v = tokens
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| IoError::Format("STL vertex needs three numbers".into()))?;
        }
        mesh.positions.push(Point3::new(c[0], c[1], c[2]));
        if mesh.positions.len() > MAX_TRIANGLES * 3 {
            return Err(IoError::Format(format!(
                "STL has more than {MAX_TRIANGLES} triangles"
            )));
        }
    }
    if mesh.positions.len() % 3 != 0 {
        return Err(IoError::Format("STL vertices do not come in threes".into()));
    }
    mesh.triangles = (0..mesh.positions.len() as u32 / 3)
        .map(|i| [3 * i, 3 * i + 1, 3 * i + 2])
        .collect();
    Ok(mesh)
}

/// Writes a binary STL.
pub fn write_binary(mesh: &TriMesh) -> Result<Vec<u8>, IoError> {
    mesh.validate()
        .map_err(|e| IoError::Format(format!("STL: {e}")))?;
    let n = u32::try_from(mesh.triangles.len())
        .map_err(|_| IoError::Format("too many triangles for STL".into()))?;
    let mut out = Vec::with_capacity(84 + 50 * mesh.triangles.len());
    let mut header = [0u8; 80];
    let tag = b"Chamfera binary STL";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);
    out.extend_from_slice(&n.to_le_bytes());
    for i in 0..mesh.triangles.len() {
        let Some(corners) = mesh.triangle(i) else {
            continue;
        };
        let normal = mesh.triangle_normal(i).unwrap_or_default();
        for v in [normal.x, normal.y, normal.z] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        for p in corners {
            for v in [p.x, p.y, p.z] {
                out.extend_from_slice(&(v as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> TriMesh {
        TriMesh::cuboid(Point3::ORIGIN, Point3::new(20.0, 10.0, 5.0))
    }

    #[test]
    fn binary_round_trip_keeps_the_shape() {
        let bytes = write_binary(&cube()).unwrap();
        assert_eq!(bytes.len(), 84 + 50 * 12);
        let back = read(&bytes).unwrap();
        assert_eq!(back.triangles.len(), 12);
        assert_eq!(back.positions.len(), 8, "corners are welded");
        assert!((back.signed_volume() - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn ascii_stl_is_read() {
        let text = "solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid t\n";
        let m = read(text.as_bytes()).unwrap();
        assert_eq!(m.triangles.len(), 1);
        assert!((m.area() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn hostile_files_are_refused_without_panicking() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0; 10],
            vec![0; 84],
            b"solid x\n vertex 1 2\n".to_vec(),
            b"solid x\n vertex 1 2 3\n vertex 1 2 3\n".to_vec(),
            b"solid x\n vertex nan 0 0\n vertex 0 0 0\n vertex 1 1 1\n".to_vec(),
            b"solid x\n vertex 1e300 0 0\n vertex 0 0 0\n vertex 1 1 1\n".to_vec(),
            b"hello".to_vec(),
            vec![0xff; 200],
        ];
        for c in cases {
            assert!(read(&c).is_err());
        }
        // A binary header that claims more triangles than the file holds is not binary,
        // and as text it is not STL.
        let mut lying = vec![0u8; 84];
        lying[80..84].copy_from_slice(&1000u32.to_le_bytes());
        assert!(read(&lying).is_err());
    }
}

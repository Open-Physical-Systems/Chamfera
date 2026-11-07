//! What the mesh exporters (3MF, OBJ, glTF) share: a named, coloured mesh body and vertex
//! normals that are smooth across gentle bends and sharp at creases.

use chamfera_geom::{TriMesh, Vec3};
use std::collections::HashMap;

/// A neutral light grey (sRGB) for callers that have no colour to give (the engine passes
/// each body's appearance or theme colour).
pub const DEFAULT_COLOUR: [u8; 3] = [0xB4, 0xB4, 0xB4];

/// A mesh to export, with its name and sRGB colour.
#[derive(Debug, Clone, Copy)]
pub struct MeshBody<'a> {
    pub name: &'a str,
    pub mesh: &'a TriMesh,
    pub colour: [u8; 3],
}

/// Faces meeting at more than this angle (degrees) keep a sharp edge.
pub const CREASE_DEGREES: f64 = 30.0;
/// A vertex shared by more triangles than this gets each triangle's own normal instead of
/// a smoothed one, so a hostile fan of millions of triangles cannot make this quadratic.
const MAX_SMOOTHED_VALENCE: usize = 256;

/// Vertices with normals: each corner of each triangle refers to one of `corners`, a
/// `(position index, unit normal)` pair; corners with the same position and normal are
/// shared.
#[derive(Debug, Clone, Default)]
pub struct NormalMesh {
    pub corners: Vec<(u32, Vec3)>,
    pub triangles: Vec<[u32; 3]>,
}

/// Normals for `mesh`: each corner averages (area-weighted) the triangles around its
/// vertex that face within [`CREASE_DEGREES`] of its own triangle. Degenerate triangles
/// are left out.
pub fn normals(mesh: &TriMesh) -> NormalMesh {
    let n = mesh.positions.len();
    // Area-weighted (unnormalised) and unit face normals.
    let mut weighted = Vec::with_capacity(mesh.triangles.len());
    let mut unit = Vec::with_capacity(mesh.triangles.len());
    for i in 0..mesh.triangles.len() {
        let Some([a, b, c]) = mesh.triangle(i) else {
            weighted.push(Vec3::ZERO);
            unit.push(None);
            continue;
        };
        let w = (b - a).cross(c - a);
        weighted.push(w);
        unit.push(w.normalized());
    }
    // Triangles around each vertex.
    let mut around: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        if unit.get(t).copied().flatten().is_none() {
            continue;
        }
        for &v in tri {
            if let Some(list) = around.get_mut(v as usize) {
                list.push(t as u32);
            }
        }
    }
    let cos_crease = CREASE_DEGREES.to_radians().cos();
    let mut out = NormalMesh::default();
    let mut index: HashMap<(u32, [u64; 3]), u32> = HashMap::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let Some(own) = unit.get(t).copied().flatten() else {
            continue;
        };
        let mut corners = [0u32; 3];
        for (k, &v) in tri.iter().enumerate() {
            let list = around.get(v as usize).map_or(&[][..], Vec::as_slice);
            let normal = if list.len() > MAX_SMOOTHED_VALENCE {
                own
            } else {
                let mut sum = Vec3::ZERO;
                for &o in list {
                    let o = o as usize;
                    if let (Some(Some(u)), Some(w)) = (unit.get(o), weighted.get(o))
                        && u.dot(own) >= cos_crease
                    {
                        sum = sum + *w;
                    }
                }
                sum.normalized().unwrap_or(own)
            };
            let key = (
                v,
                [normal.x.to_bits(), normal.y.to_bits(), normal.z.to_bits()],
            );
            let next = out.corners.len() as u32;
            let id = *index.entry(key).or_insert(next);
            if id == next {
                out.corners.push((v, normal));
            }
            corners[k] = id;
        }
        out.triangles.push(corners);
    }
    out
}

/// sRGB `#RRGGBB`.
pub fn hex_colour(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Point3;

    #[test]
    fn a_box_has_sharp_corners() {
        // Each of a box's 8 corners touches 3 faces at right angles: 3 normals each.
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 2.0, 3.0));
        let nm = normals(&m);
        assert_eq!(nm.triangles.len(), 12);
        assert_eq!(nm.corners.len(), 24);
        for (_, n) in &nm.corners {
            let axis = n.x.abs().max(n.y.abs()).max(n.z.abs());
            assert!((axis - 1.0).abs() < 1e-12, "{n:?} is along an axis");
        }
    }

    #[test]
    fn a_cylinder_side_is_smooth() {
        // 64 sides: neighbouring side faces differ by 360/64 = 5.6 degrees, under the
        // crease, so the side shares one normal per vertex: 2 * 64 side corners, plus
        // the caps' own (the rim is a right angle).
        let m = TriMesh::cylinder(Point3::ORIGIN, 1.0, 1.0, 64);
        let nm = normals(&m);
        let side = nm.corners.iter().filter(|(_, n)| n.z.abs() < 1e-9).count();
        assert_eq!(side, 128);
    }

    #[test]
    fn the_colour_is_hex() {
        assert_eq!(hex_colour([0xB4, 0x0A, 0xFF]), "#B40AFF");
    }
}

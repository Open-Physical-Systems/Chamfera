//! Smooth shading normals for display meshes: each triangle corner gets the angle-weighted
//! average normal (Thürmer and Wüthrich 1998, so how a face is split into triangles does not
//! change it) of the triangles around that point whose own normals are within a
//! crease angle of its triangle's. Curved faces then shade smoothly while sharp edges stay
//! sharp. Display only: model decisions never read these.

use chamfera_geom::{TriMesh, Vec3};
use std::collections::HashMap;

/// Most triangles averaged around one point; points shared more widely (a cone apex)
/// keep the face normal.
const MAX_FAN: usize = 256;

/// The shading normal of every corner, triangle by triangle (`[corner 0, 1, 2]`).
/// Degenerate triangles get a zero normal at every corner.
pub fn corner_normals(mesh: &TriMesh, crease: f64) -> Vec<[Vec3; 3]> {
    let cos_crease = crease.cos();
    // Corners that share a position share a key, even when the tessellator repeats a
    // vertex on both sides of a face boundary.
    let size = mesh.bounds().map_or(1.0, |b| b.radius().max(1e-9));
    let quantum = size * 1e-7;
    let key = |i: u32| -> Option<(i64, i64, i64)> {
        let p = mesh.positions.get(i as usize)?;
        let q = |v: f64| (v / quantum).round() as i64;
        Some((q(p.x), q(p.y), q(p.z)))
    };
    // Unit face normals and the angle at each corner.
    let faces: Vec<(Option<Vec3>, [f64; 3])> = (0..mesh.triangles.len())
        .map(|i| match mesh.triangle(i) {
            Some(p) => {
                let angle = |k: usize| {
                    let o = p[k];
                    (p[(k + 1) % 3] - o)
                        .angle_to(p[(k + 2) % 3] - o)
                        .unwrap_or(0.0)
                };
                let n = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
                (n, [angle(0), angle(1), angle(2)])
            }
            None => (None, [0.0; 3]),
        })
        .collect();
    let mut around: HashMap<(i64, i64, i64), Vec<(u32, u8)>> = HashMap::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        for (c, &v) in tri.iter().enumerate() {
            if let Some(k) = key(v) {
                let fan = around.entry(k).or_default();
                if fan.len() < MAX_FAN {
                    fan.push((t as u32, c as u8));
                }
            }
        }
    }
    mesh.triangles
        .iter()
        .enumerate()
        .map(|(t, tri)| {
            let Some((Some(own), _)) = faces.get(t).copied() else {
                return [Vec3::ZERO; 3];
            };
            tri.map(|v| {
                let Some(fan) = key(v).and_then(|k| around.get(&k)) else {
                    return own;
                };
                let mut sum = Vec3::ZERO;
                for &(o, c) in fan {
                    if let Some((Some(unit), angles)) = faces.get(o as usize).copied()
                        && unit.dot(own) >= cos_crease
                    {
                        sum = sum + unit * angles.get(c as usize).copied().unwrap_or(0.0);
                    }
                }
                sum.normalized().unwrap_or(own)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Point3;

    #[test]
    fn a_box_keeps_its_face_normals_and_a_cylinder_side_is_smooth() {
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 2.0, 3.0));
        let n = corner_normals(&m, 30f64.to_radians());
        for (t, corners) in n.iter().enumerate() {
            let f = m.triangle_normal(t).unwrap();
            for c in corners {
                assert!((*c - f).length() < 1e-9);
            }
        }
        // A 64-sided cylinder of radius 5 about Z at the origin: on the side, each corner's
        // normal points straight out from the axis through that corner: by symmetry the
        // two neighbouring facets meet it at equal angles (90 degrees each, however the
        // quads are split), so their normals average to the radial direction.
        let c = TriMesh::cylinder(Point3::ORIGIN, 5.0, 10.0, 64);
        let n = corner_normals(&c, 30f64.to_radians());
        let mut side = 0;
        for (t, corners) in n.iter().enumerate() {
            let f = c.triangle_normal(t).unwrap();
            if f.z.abs() > 0.5 {
                continue;
            }
            side += 1;
            let tri = c.triangle(t).unwrap();
            for (p, nn) in tri.iter().zip(corners) {
                let radial = Vec3::new(p.x, p.y, 0.0).normalized().unwrap();
                assert!((*nn - radial).length() < 1e-9, "{nn:?} {radial:?}");
            }
        }
        assert!(side > 0);
    }
}

//! Triangle meshes, boxes and rays.
//!
//! Meshes are for display, export, picking a displayed body and mesh bodies (imported STL).
//! Model decisions on B-rep bodies use exact geometry, never these meshes.

use crate::{LINEAR_TOLERANCE, Point3, Vec3};
use serde::{Deserialize, Serialize};

/// Most triangles one mesh may hold (hostile-input limit).
pub const MAX_TRIANGLES: usize = 50_000_000;

/// An axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Point3,
    pub max: Point3,
}

impl Aabb {
    /// The smallest box holding all points, or `None` for no points.
    pub fn from_points<I: IntoIterator<Item = Point3>>(points: I) -> Option<Aabb> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut b = Aabb {
            min: first,
            max: first,
        };
        for p in it {
            b.include(p);
        }
        Some(b)
    }

    pub fn include(&mut self, p: Point3) {
        self.min = Point3::new(
            self.min.x.min(p.x),
            self.min.y.min(p.y),
            self.min.z.min(p.z),
        );
        self.max = Point3::new(
            self.max.x.max(p.x),
            self.max.y.max(p.y),
            self.max.z.max(p.z),
        );
    }

    pub fn union(self, o: Aabb) -> Aabb {
        let mut b = self;
        b.include(o.min);
        b.include(o.max);
        b
    }

    pub fn center(&self) -> Point3 {
        Point3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Radius of the sphere around the box's centre that holds the box.
    pub fn radius(&self) -> f64 {
        self.size().length() * 0.5
    }
}

/// A ray: `origin + t * direction` for `t >= 0`. `direction` is a unit vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Point3,
    pub direction: Vec3,
}

impl Ray {
    /// A ray, or `None` when the direction has no length or a value is not finite.
    pub fn new(origin: Point3, direction: Vec3) -> Option<Ray> {
        if !(origin - Point3::ORIGIN).is_finite() {
            return None;
        }
        Some(Ray {
            origin,
            direction: direction.normalized()?,
        })
    }

    pub fn at(&self, t: f64) -> Point3 {
        self.origin + self.direction * t
    }

    /// Distance along the ray to triangle `a b c`, both sides counted, or `None` for a miss.
    /// Moller-Trumbore (1997), "Fast, Minimum Storage Ray/Triangle Intersection".
    pub fn hit_triangle(&self, a: Point3, b: Point3, c: Point3) -> Option<f64> {
        let e1 = b - a;
        let e2 = c - a;
        let p = self.direction.cross(e2);
        let det = e1.dot(p);
        // A triangle seen edge-on, or a degenerate one, is not hit.
        if det.abs() < 1e-12 * e1.length().max(1.0) * e2.length().max(1.0) {
            return None;
        }
        let inv = 1.0 / det;
        let s = self.origin - a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = self.direction.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t >= 0.0 && t.is_finite()).then_some(t)
    }
}

/// An indexed triangle mesh. Triangles wind counter-clockwise seen from outside.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
}

/// Why a mesh is not usable.
#[derive(Debug, Clone, PartialEq)]
pub enum MeshError {
    Empty,
    TooManyTriangles(usize),
    IndexOutOfRange { triangle: usize },
    BadCoordinate { vertex: usize },
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::Empty => write!(f, "the mesh has no triangles"),
            MeshError::TooManyTriangles(n) => {
                write!(
                    f,
                    "the mesh has {n} triangles, the limit is {MAX_TRIANGLES}"
                )
            }
            MeshError::IndexOutOfRange { triangle } => {
                write!(
                    f,
                    "triangle {triangle} refers to a vertex that does not exist"
                )
            }
            MeshError::BadCoordinate { vertex } => {
                write!(
                    f,
                    "vertex {vertex} is not finite or is outside the working range"
                )
            }
        }
    }
}

impl std::error::Error for MeshError {}

impl TriMesh {
    /// Checks indices, coordinates and size. Every mesh from outside goes through this.
    pub fn validate(&self) -> Result<(), MeshError> {
        if self.triangles.is_empty() {
            return Err(MeshError::Empty);
        }
        if self.triangles.len() > MAX_TRIANGLES {
            return Err(MeshError::TooManyTriangles(self.triangles.len()));
        }
        if let Some(i) = self.positions.iter().position(|p| !p.is_valid()) {
            return Err(MeshError::BadCoordinate { vertex: i });
        }
        let n = self.positions.len();
        if let Some(i) = self
            .triangles
            .iter()
            .position(|t| t.iter().any(|&v| v as usize >= n))
        {
            return Err(MeshError::IndexOutOfRange { triangle: i });
        }
        Ok(())
    }

    /// The three corners of triangle `i`, or `None` if `i` or an index is out of range.
    pub fn triangle(&self, i: usize) -> Option<[Point3; 3]> {
        let t = self.triangles.get(i)?;
        Some([
            *self.positions.get(t[0] as usize)?,
            *self.positions.get(t[1] as usize)?,
            *self.positions.get(t[2] as usize)?,
        ])
    }

    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::from_points(self.positions.iter().copied())
    }

    /// Total triangle area (mm^2).
    pub fn area(&self) -> f64 {
        (0..self.triangles.len())
            .filter_map(|i| self.triangle(i))
            .map(|[a, b, c]| (b - a).cross(c - a).length() * 0.5)
            .sum()
    }

    /// Enclosed volume (mm^3) by the divergence theorem. Meaningful only for a closed,
    /// consistently oriented mesh; negative when the mesh is inside out.
    pub fn signed_volume(&self) -> f64 {
        (0..self.triangles.len())
            .filter_map(|i| self.triangle(i))
            .map(|[a, b, c]| {
                let (a, b, c) = (a - Point3::ORIGIN, b - Point3::ORIGIN, c - Point3::ORIGIN);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// The unit normal of triangle `i` (right-hand rule), or `None` for a degenerate one.
    pub fn triangle_normal(&self, i: usize) -> Option<Vec3> {
        let [a, b, c] = self.triangle(i)?;
        (b - a).cross(c - a).normalized()
    }

    /// The nearest triangle the ray hits: `(triangle index, distance)`.
    pub fn raycast(&self, ray: &Ray) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.triangles.len() {
            let Some([a, b, c]) = self.triangle(i) else {
                continue;
            };
            if let Some(t) = ray.hit_triangle(a, b, c)
                && best.is_none_or(|(_, bt)| t < bt)
            {
                best = Some((i, t));
            }
        }
        best
    }

    /// Joins vertices that are exactly equal, so faces share their corners.
    pub fn weld_exact(&mut self) {
        let mut map: std::collections::HashMap<[u64; 3], u32> = std::collections::HashMap::new();
        let mut positions = Vec::new();
        let mut remap = Vec::with_capacity(self.positions.len());
        for p in &self.positions {
            // +0.0 and -0.0 are the same point.
            let key = [
                (p.x + 0.0).to_bits(),
                (p.y + 0.0).to_bits(),
                (p.z + 0.0).to_bits(),
            ];
            let idx = *map.entry(key).or_insert_with(|| {
                positions.push(*p);
                (positions.len() - 1) as u32
            });
            remap.push(idx);
        }
        for t in &mut self.triangles {
            for v in t.iter_mut() {
                *v = remap.get(*v as usize).copied().unwrap_or(*v);
            }
        }
        self.positions = positions;
    }

    /// A box from `min` to `max` (12 triangles, outward normals).
    pub fn cuboid(min: Point3, max: Point3) -> TriMesh {
        let p = |x: bool, y: bool, z: bool| {
            Point3::new(
                if x { max.x } else { min.x },
                if y { max.y } else { min.y },
                if z { max.z } else { min.z },
            )
        };
        let positions = vec![
            p(false, false, false),
            p(true, false, false),
            p(true, true, false),
            p(false, true, false),
            p(false, false, true),
            p(true, false, true),
            p(true, true, true),
            p(false, true, true),
        ];
        let quads: [[u32; 4]; 6] = [
            [0, 3, 2, 1], // bottom, -Z
            [4, 5, 6, 7], // top, +Z
            [0, 1, 5, 4], // front, -Y
            [2, 3, 7, 6], // back, +Y
            [1, 2, 6, 5], // right, +X
            [3, 0, 4, 7], // left, -X
        ];
        let triangles = quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect();
        TriMesh {
            positions,
            triangles,
        }
    }

    /// A closed cylinder standing on `base` along +Z, with `segments` sides (at least 3).
    pub fn cylinder(base: Point3, radius: f64, height: f64, segments: u32) -> TriMesh {
        let n = segments.max(3);
        let mut positions = Vec::with_capacity(2 * n as usize + 2);
        for k in 0..n {
            let a = std::f64::consts::TAU * f64::from(k) / f64::from(n);
            let off = Vec3::new(radius * a.cos(), radius * a.sin(), 0.0);
            positions.push(base + off);
            positions.push(base + off + Vec3::new(0.0, 0.0, height));
        }
        let bottom_c = positions.len() as u32;
        positions.push(base);
        let top_c = positions.len() as u32;
        positions.push(base + Vec3::new(0.0, 0.0, height));
        let mut triangles = Vec::with_capacity(4 * n as usize);
        for k in 0..n {
            let (b0, t0) = (2 * k, 2 * k + 1);
            let (b1, t1) = (2 * ((k + 1) % n), 2 * ((k + 1) % n) + 1);
            triangles.push([b0, b1, t1]);
            triangles.push([b0, t1, t0]);
            triangles.push([bottom_c, b1, b0]);
            triangles.push([top_c, t0, t1]);
        }
        TriMesh {
            positions,
            triangles,
        }
    }

    /// Appends another mesh (no welding).
    pub fn append(&mut self, other: &TriMesh) {
        let off = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.triangles.extend(
            other
                .triangles
                .iter()
                .map(|t| [t[0] + off, t[1] + off, t[2] + off]),
        );
    }
}

/// True when two lengths are equal within [`LINEAR_TOLERANCE`].
pub fn same_length(a: f64, b: f64) -> bool {
    (a - b).abs() <= LINEAR_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn box_measures_match_closed_forms() {
        // 50 x 30 x 10: volume 15000, area 2(50*30 + 50*10 + 30*10) = 4600.
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(50.0, 30.0, 10.0));
        m.validate().unwrap();
        assert!((m.signed_volume() - 15_000.0).abs() < 1e-9);
        assert!((m.area() - 4_600.0).abs() < 1e-9);
        let b = m.bounds().unwrap();
        assert_eq!(b.center(), Point3::new(25.0, 15.0, 5.0));
    }

    #[test]
    fn cylinder_volume_converges_to_pi_r2_h() {
        // An inscribed n-gon prism has volume (n/2) r^2 sin(2pi/n) h, below pi r^2 h.
        let (r, h) = (5.0, 10.0);
        let n = 256;
        let m = TriMesh::cylinder(Point3::ORIGIN, r, h, n);
        m.validate().unwrap();
        let exact_prism = f64::from(n) / 2.0 * r * r * (2.0 * PI / f64::from(n)).sin() * h;
        assert!((m.signed_volume() - exact_prism).abs() < 1e-9);
        assert!((m.signed_volume() - PI * r * r * h).abs() / (PI * r * r * h) < 1e-3);
    }

    #[test]
    fn rays_hit_the_nearest_face_from_either_side() {
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0));
        let ray = Ray::new(Point3::new(5.0, 5.0, 100.0), -Vec3::Z).unwrap();
        let (tri, t) = m.raycast(&ray).unwrap();
        assert!((t - 90.0).abs() < 1e-12);
        assert!((m.triangle_normal(tri).unwrap().z - 1.0).abs() < 1e-12);
        // From inside, the ray still finds the wall.
        let inside = Ray::new(Point3::new(5.0, 5.0, 5.0), Vec3::X).unwrap();
        assert!((m.raycast(&inside).unwrap().1 - 5.0).abs() < 1e-12);
        // A miss.
        let miss = Ray::new(Point3::new(50.0, 50.0, 100.0), -Vec3::Z).unwrap();
        assert!(m.raycast(&miss).is_none());
    }

    #[test]
    fn rays_need_a_direction() {
        assert!(Ray::new(Point3::ORIGIN, Vec3::ZERO).is_none());
        assert!(Ray::new(Point3::new(f64::NAN, 0.0, 0.0), Vec3::X).is_none());
    }

    #[test]
    fn bad_meshes_are_refused() {
        assert_eq!(TriMesh::default().validate(), Err(MeshError::Empty));
        let mut m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        m.triangles.push([0, 1, 99]);
        assert!(matches!(
            m.validate(),
            Err(MeshError::IndexOutOfRange { .. })
        ));
        let mut m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        m.positions[3].x = f64::NAN;
        assert!(matches!(
            m.validate(),
            Err(MeshError::BadCoordinate { vertex: 3 })
        ));
    }

    #[test]
    fn welding_joins_equal_corners() {
        let mut m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        let copy = m.clone();
        m.append(&copy);
        assert_eq!(m.positions.len(), 16);
        m.weld_exact();
        assert_eq!(m.positions.len(), 8);
        assert!((m.signed_volume() - 2.0).abs() < 1e-12);
    }
}

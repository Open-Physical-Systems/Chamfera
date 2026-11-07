//! Layer 0: vectors, points, tolerances, frames, transforms, robust predicates and NURBS
//! curves (`nurbs`).
//!
//! Conventions used everywhere in Chamfera:
//! - lengths are millimetres, angles are radians;
//! - the world is right-handed and Z is up;
//! - geometry is exact (f64); meshes are only for display and export.

mod frame;
mod mesh;
mod nurbs;
mod predicates;
mod transform;
mod vec2;

pub use frame::Frame;
pub use mesh::{Aabb, MAX_TRIANGLES, MeshError, Ray, TriMesh, same_length};
pub use nurbs::{
    BSplineCurve2, BSplineCurve3, CurveProjection, DEFAULT_SPLINE_DEGREE, KNOT_TOLERANCE,
    LENGTH_RELATIVE_TOLERANCE, MAX_CONTROL_POINTS, MAX_DEGREE, MAX_FIT_POINTS, MAX_KNOT,
    MAX_WEIGHT, MIN_WEIGHT, NurbsError,
};
pub use predicates::{
    PREDICATE_MAX_MAGNITUDE, PREDICATE_MIN_MAGNITUDE, Sign, incircle, orient2d, orient3d,
};
pub use transform::Transform;
pub use vec2::{Point2, Vec2};

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

/// Two lengths closer than this (mm) are the same length.
pub const LINEAR_TOLERANCE: f64 = 1e-7;
/// Two directions closer than this (radians) are the same direction.
pub const ANGULAR_TOLERANCE: f64 = 1e-10;
/// Largest coordinate magnitude a model may use (mm): 1 km. Larger inputs are rejected.
pub const MAX_COORDINATE: f64 = 1e6;

/// True when `v` is a finite number inside the model's working range.
pub fn is_valid_coordinate(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// A displacement in 3D space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A location in 3D space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// The unit vector in this direction, or `None` for a (near) zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec3> {
        let len = self.length();
        if !len.is_finite() || len < LINEAR_TOLERANCE {
            return None;
        }
        Some(self * (1.0 / len))
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Angle between two vectors in radians (0..=π), or `None` if either is zero.
    pub fn angle_to(self, o: Vec3) -> Option<f64> {
        let a = self.normalized()?;
        let b = o.normalized()?;
        // atan2 of |a×b| and a·b is accurate near 0 and π, unlike acos.
        Some(a.cross(b).length().atan2(a.dot(b)))
    }
}

impl Point3 {
    pub const ORIGIN: Point3 = Point3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance(self, o: Point3) -> f64 {
        (self - o).length()
    }

    /// True when the two points are within [`LINEAR_TOLERANCE`].
    pub fn coincident(self, o: Point3) -> bool {
        self.distance(o) <= LINEAR_TOLERANCE
    }

    pub fn is_valid(self) -> bool {
        is_valid_coordinate(self.x) && is_valid_coordinate(self.y) && is_valid_coordinate(self.z)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Add<Vec3> for Point3 {
    type Output = Point3;
    fn add(self, v: Vec3) -> Point3 {
        Point3::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}

impl Sub for Point3 {
    type Output = Vec3;
    fn sub(self, o: Point3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn cross_of_x_and_y_is_z() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::X), -Vec3::Z);
    }

    #[test]
    fn length_of_3_4_12_is_13() {
        assert!((Vec3::new(3.0, 4.0, 12.0).length() - 13.0).abs() < 1e-12);
    }

    #[test]
    fn zero_and_non_finite_vectors_have_no_direction() {
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        assert!(Vec3::new(f64::INFINITY, 0.0, 0.0).normalized().is_none());
        assert!(Vec3::new(1e-9, 0.0, 0.0).normalized().is_none());
    }

    #[test]
    fn angles_are_exact_at_the_ends() {
        let right = Vec3::X.angle_to(Vec3::Y).unwrap();
        assert!((right - FRAC_PI_2).abs() < 1e-15);
        let opposite = Vec3::X.angle_to(-Vec3::X).unwrap();
        assert!((opposite - PI).abs() < 1e-15);
        assert_eq!(Vec3::X.angle_to(Vec3::X * 5.0).unwrap(), 0.0);
        assert!(Vec3::X.angle_to(Vec3::ZERO).is_none());
    }

    #[test]
    fn points_within_tolerance_coincide() {
        let a = Point3::new(10.0, 0.0, 0.0);
        assert!(a.coincident(a + Vec3::new(LINEAR_TOLERANCE / 2.0, 0.0, 0.0)));
        assert!(!a.coincident(a + Vec3::new(LINEAR_TOLERANCE * 2.0, 0.0, 0.0)));
    }

    #[test]
    fn coordinates_outside_the_working_range_are_invalid() {
        assert!(Point3::new(1.0, 2.0, 3.0).is_valid());
        assert!(!Point3::new(MAX_COORDINATE * 2.0, 0.0, 0.0).is_valid());
        assert!(!Point3::new(f64::NAN, 0.0, 0.0).is_valid());
    }
}

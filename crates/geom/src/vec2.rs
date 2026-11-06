//! 2D vectors and points: sketch planes, profiles, planar curve intersection.

use crate::LINEAR_TOLERANCE;
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

/// A displacement in a plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

/// A location in a plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2::new(0.0, 0.0);
    pub const X: Vec2 = Vec2::new(1.0, 0.0);
    pub const Y: Vec2 = Vec2::new(0.0, 1.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// The z component of the 3D cross product: positive when `o` is counter-clockwise
    /// from `self`.
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The unit vector in this direction, or `None` for a (near) zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec2> {
        let len = self.length();
        if !len.is_finite() || len < LINEAR_TOLERANCE {
            return None;
        }
        Some(self * (1.0 / len))
    }

    /// Rotated a quarter turn counter-clockwise.
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    /// Direction angle from +X in radians, -π..=π.
    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Point2 {
    pub const ORIGIN: Point2 = Point2::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance(self, o: Point2) -> f64 {
        (self - o).length()
    }

    /// True when the two points are within [`LINEAR_TOLERANCE`].
    pub fn coincident(self, o: Point2) -> bool {
        self.distance(o) <= LINEAR_TOLERANCE
    }

    pub fn is_valid(self) -> bool {
        crate::is_valid_coordinate(self.x) && crate::is_valid_coordinate(self.y)
    }

    pub fn lerp(self, o: Point2, t: f64) -> Point2 {
        self + (o - self) * t
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Add<Vec2> for Point2 {
    type Output = Point2;
    fn add(self, v: Vec2) -> Point2 {
        Point2::new(self.x + v.x, self.y + v.y)
    }
}

impl Sub<Vec2> for Point2 {
    type Output = Point2;
    fn sub(self, v: Vec2) -> Point2 {
        Point2::new(self.x - v.x, self.y - v.y)
    }
}

impl Sub for Point2 {
    type Output = Vec2;
    fn sub(self, o: Point2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_sign_says_which_way_it_turns() {
        assert_eq!(Vec2::X.cross(Vec2::Y), 1.0);
        assert_eq!(Vec2::Y.cross(Vec2::X), -1.0);
        assert_eq!(Vec2::X.perp(), Vec2::Y);
    }

    #[test]
    fn lengths_and_directions() {
        assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);
        assert!(Vec2::ZERO.normalized().is_none());
        assert!(Vec2::new(f64::NAN, 1.0).normalized().is_none());
        assert!((Vec2::new(-1.0, 0.0).angle() - std::f64::consts::PI).abs() < 1e-15);
        assert_eq!(
            Point2::new(0.0, 0.0).lerp(Point2::new(10.0, 4.0), 0.5),
            Point2::new(5.0, 2.0)
        );
    }
}

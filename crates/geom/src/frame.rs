//! Frames: an origin and a right-handed orthonormal basis. A frame's x/y plane is where
//! sketches and planar curves live; its z axis is the plane's normal.

use crate::{ANGULAR_TOLERANCE, Point2, Point3, Transform, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// An origin with unit axes `x`, `y`, `z`, mutually perpendicular and right-handed
/// (`x × y = z`). Constructors guarantee this; the fields are private to keep it so, and
/// deserializing re-validates (it stores origin, normal and x; y is derived).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "FrameData", into = "FrameData")]
pub struct Frame {
    origin: Point3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

/// The serialized form of a [`Frame`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct FrameData {
    origin: Point3,
    normal: Vec3,
    x: Vec3,
}

impl From<Frame> for FrameData {
    fn from(f: Frame) -> Self {
        FrameData {
            origin: f.origin,
            normal: f.z,
            x: f.x,
        }
    }
}

impl TryFrom<FrameData> for Frame {
    type Error = &'static str;
    fn try_from(d: FrameData) -> Result<Self, Self::Error> {
        Frame::from_normal_and_x(d.origin, d.normal, d.x)
            .ok_or("a frame needs a valid origin and finite, non-zero, non-parallel normal and x")
    }
}

impl Frame {
    /// The world frame; its plane is the XY (top) plane: x = X, y = Y, normal Z (up).
    pub const XY: Frame = Frame::raw(Vec3::X, Vec3::Y, Vec3::Z);
    /// The XZ (front) plane: x = X, y = Z (up), normal -Y, i.e. seen from the front
    /// (from -Y looking toward +Y) x points right and y points up.
    pub const XZ: Frame = Frame::raw(Vec3::X, Vec3::Z, Vec3::new(0.0, -1.0, 0.0));
    /// The YZ (right) plane: x = Y, y = Z (up), normal X, i.e. seen from the right
    /// (from +X looking toward -X) x points right and y points up.
    pub const YZ: Frame = Frame::raw(Vec3::Y, Vec3::Z, Vec3::X);

    const fn raw(x: Vec3, y: Vec3, z: Vec3) -> Frame {
        Frame {
            origin: Point3::ORIGIN,
            x,
            y,
            z,
        }
    }

    /// A frame at `origin` whose z axis is `normal`, with x chosen deterministically:
    /// the world axis least aligned with the normal (first of X, Y, Z on ties), projected
    /// into the plane. This reproduces [`Frame::XY`], [`Frame::XZ`] and [`Frame::YZ`] for
    /// normals Z, -Y and X. `None` for a zero or non-finite normal or an invalid origin.
    pub fn from_normal(origin: Point3, normal: Vec3) -> Option<Frame> {
        let z = normal.normalized()?;
        let (ax, ay, az) = (z.x.abs(), z.y.abs(), z.z.abs());
        let hint = if ax <= ay && ax <= az {
            Vec3::X
        } else if ay <= az {
            Vec3::Y
        } else {
            Vec3::Z
        };
        Frame::from_normal_and_x(origin, z, hint)
    }

    /// A frame at `origin` with z axis `normal` and x axis the part of `x_hint`
    /// perpendicular to it. `None` when either vector is zero or not finite, the hint is
    /// parallel to the normal (within [`ANGULAR_TOLERANCE`]), or the origin is invalid.
    pub fn from_normal_and_x(origin: Point3, normal: Vec3, x_hint: Vec3) -> Option<Frame> {
        if !origin.is_valid() {
            return None;
        }
        let z = normal.normalized()?;
        let hint = x_hint.normalized()?;
        let in_plane = hint - z * hint.dot(z);
        // |in_plane| = sin(angle between hint and normal).
        if in_plane.length() <= ANGULAR_TOLERANCE {
            return None;
        }
        let x = in_plane.normalized()?;
        let y = z.cross(x);
        Some(Frame { origin, x, y, z })
    }

    /// The same frame moved to `origin`, or `None` for an invalid origin.
    pub fn with_origin(&self, origin: Point3) -> Option<Frame> {
        origin.is_valid().then_some(Frame { origin, ..*self })
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }
    pub fn x(&self) -> Vec3 {
        self.x
    }
    pub fn y(&self) -> Vec3 {
        self.y
    }
    /// The z axis: the normal of the frame's plane.
    pub fn z(&self) -> Vec3 {
        self.z
    }

    /// The frame turned over about its x axis: axes x, -y, -z. Still right-handed; a
    /// curve `c(t)` in the plane becomes `c(-t)`.
    pub fn flipped(&self) -> Frame {
        Frame {
            origin: self.origin,
            x: self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// World coordinates of `p` expressed in this frame.
    pub fn to_local(&self, p: Point3) -> Point3 {
        let v = self.vector_to_local(p - self.origin);
        Point3::new(v.x, v.y, v.z)
    }

    /// World point from coordinates in this frame.
    pub fn to_world(&self, p: Point3) -> Point3 {
        self.origin + self.vector_to_world(Vec3::new(p.x, p.y, p.z))
    }

    pub fn vector_to_local(&self, v: Vec3) -> Vec3 {
        Vec3::new(v.dot(self.x), v.dot(self.y), v.dot(self.z))
    }

    pub fn vector_to_world(&self, v: Vec3) -> Vec3 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }

    /// `p` projected onto the frame's plane, in plane coordinates (local z dropped).
    pub fn to_plane(&self, p: Point3) -> Point2 {
        let l = self.to_local(p);
        Point2::new(l.x, l.y)
    }

    /// World point of plane coordinates `p`.
    pub fn from_plane(&self, p: Point2) -> Point3 {
        self.origin + self.x * p.x + self.y * p.y
    }

    /// World vector of a plane vector.
    pub fn vector_from_plane(&self, v: Vec2) -> Vec3 {
        self.x * v.x + self.y * v.y
    }

    /// The rigid transform taking local coordinates to world coordinates.
    pub fn local_to_world(&self) -> Transform {
        // The axes are finite unit vectors and the origin is valid, so this always exists.
        Transform::from_basis(self.origin, self.x, self.y, self.z).unwrap_or_default()
    }

    /// The frame moved by a rigid transform, or `None` if `t` is not rigid or moves the
    /// origin out of range.
    pub fn transformed(&self, t: &Transform) -> Option<Frame> {
        if !t.is_rigid() {
            return None;
        }
        let origin = t.apply_point(self.origin);
        let z = t.apply_vector(self.z);
        let x = t.apply_vector(self.x);
        Frame::from_normal_and_x(origin, z, x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    fn right_handed_orthonormal(f: &Frame) -> bool {
        let unit = |v: Vec3| (v.length() - 1.0).abs() < 1e-12;
        unit(f.x)
            && unit(f.y)
            && unit(f.z)
            && f.x.dot(f.y).abs() < 1e-12
            && f.y.dot(f.z).abs() < 1e-12
            && near(f.x.cross(f.y), f.z)
    }

    #[test]
    fn origin_planes_follow_the_documented_conventions() {
        for f in [Frame::XY, Frame::XZ, Frame::YZ] {
            assert!(right_handed_orthonormal(&f));
        }
        assert_eq!(Frame::XY.z(), Vec3::Z);
        assert_eq!(Frame::XZ.y(), Vec3::Z);
        assert_eq!(Frame::XZ.z(), Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(Frame::YZ.x(), Vec3::Y);
        assert_eq!(Frame::YZ.z(), Vec3::X);
        // from_normal reproduces them.
        for (f, n) in [
            (Frame::XY, Vec3::Z),
            (Frame::XZ, -Vec3::Y),
            (Frame::YZ, Vec3::X),
        ] {
            let g = Frame::from_normal(Point3::ORIGIN, n * 3.0).unwrap();
            assert!(near(g.x(), f.x()) && near(g.y(), f.y()) && near(g.z(), f.z()));
        }
    }

    #[test]
    fn a_tilted_frame_is_orthonormal_and_honours_the_hint() {
        let f = Frame::from_normal_and_x(
            Point3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(1.0, 1.0, 5.0),
        )
        .unwrap();
        // The hint's in-plane part is (1, 1, 0)/sqrt 2.
        let s = 0.5_f64.sqrt();
        assert!(near(f.x(), Vec3::new(s, s, 0.0)));
        assert!(near(f.y(), Vec3::new(-s, s, 0.0)));
        let g = Frame::from_normal(Point3::ORIGIN, Vec3::new(1.0, -2.0, 0.5)).unwrap();
        assert!(right_handed_orthonormal(&g));
    }

    #[test]
    fn local_and_world_round_trip() {
        let f = Frame::from_normal_and_x(
            Point3::new(10.0, -5.0, 2.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(1.0, -1.0, 0.0),
        )
        .unwrap();
        let p = Point3::new(3.5, 7.25, -11.0);
        assert!(f.to_world(f.to_local(p)).distance(p) < 1e-12);
        assert!(f.to_local(f.to_world(p)).distance(p) < 1e-12);
        let v = Vec3::new(-2.0, 0.5, 9.0);
        assert!(near(f.vector_to_world(f.vector_to_local(v)), v));
        // The origin is local (0,0,0); origin + 2 z is local (0, 0, 2).
        assert!(f.to_local(f.origin()).distance(Point3::ORIGIN) < 1e-12);
        let up = f.origin() + f.z() * 2.0;
        assert!(f.to_local(up).distance(Point3::new(0.0, 0.0, 2.0)) < 1e-12);
        // Plane coordinates round trip for points in the plane.
        let q = Point2::new(4.0, -3.0);
        assert!(f.to_plane(f.from_plane(q)).distance(q) < 1e-12);
        // The transform agrees with to_world.
        let t = f.local_to_world();
        assert!(t.is_rigid());
        assert!(t.apply_point(p).distance(f.to_world(p)) < 1e-12);
        assert!(
            t.inverse_rigid()
                .unwrap()
                .apply_point(p)
                .distance(f.to_local(p))
                < 1e-12
        );
    }

    #[test]
    fn flipped_and_transformed_frames() {
        let f = Frame::XY.flipped();
        assert!(right_handed_orthonormal(&f));
        assert_eq!(f.z(), -Vec3::Z);
        // A quarter turn about X takes the XY frame to one with normal -Y: the XZ frame.
        let r = Transform::rotation(Vec3::X, FRAC_PI_2).unwrap();
        let g = Frame::XY.transformed(&r).unwrap();
        assert!(near(g.x(), Vec3::X) && near(g.y(), Vec3::Z) && near(g.z(), -Vec3::Y));
        assert!(
            Frame::XY
                .transformed(&Transform::scale(2.0).unwrap())
                .is_none()
        );
    }

    #[test]
    fn serialized_frames_round_trip_and_bad_ones_are_rejected() {
        let f = Frame::from_normal(Point3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 1.0, 1.0)).unwrap();
        let json = serde_json::to_string(&f).unwrap();
        let back = serde_json::from_str::<Frame>(&json).unwrap();
        assert!(near(back.x(), f.x()) && near(back.y(), f.y()) && near(back.z(), f.z()));
        let bad = r#"{"origin":{"x":0,"y":0,"z":0},"normal":{"x":0,"y":0,"z":1},"x":{"x":0,"y":0,"z":2}}"#;
        assert!(serde_json::from_str::<Frame>(bad).is_err());
    }

    #[test]
    fn hostile_input_gives_none() {
        let o = Point3::ORIGIN;
        assert!(Frame::from_normal(o, Vec3::ZERO).is_none());
        assert!(Frame::from_normal(o, Vec3::new(f64::NAN, 0.0, 1.0)).is_none());
        assert!(Frame::from_normal(Point3::new(f64::INFINITY, 0.0, 0.0), Vec3::Z).is_none());
        assert!(Frame::from_normal(Point3::new(1e300, 0.0, 0.0), Vec3::Z).is_none());
        assert!(Frame::from_normal_and_x(o, Vec3::Z, Vec3::Z * 4.0).is_none());
        assert!(Frame::from_normal_and_x(o, Vec3::Z, -Vec3::Z).is_none());
        assert!(Frame::from_normal_and_x(o, Vec3::Z, Vec3::ZERO).is_none());
        assert!(
            Frame::XY
                .with_origin(Point3::new(0.0, f64::NAN, 0.0))
                .is_none()
        );
    }
}

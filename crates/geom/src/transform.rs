//! Affine and rigid transforms: a 3x3 matrix and a translation, `p -> M p + t`.

use crate::{ANGULAR_TOLERANCE, Point3, Vec3};
use serde::{Deserialize, Serialize};

/// An affine map `p -> M p + t`; vectors map as `v -> M v`. Rows of `M` are stored.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    m: [[f64; 3]; 3],
    t: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: Vec3::ZERO,
    };

    /// A transform from a row-major matrix and a translation, or `None` if any value is
    /// not finite.
    pub fn from_parts(m: [[f64; 3]; 3], t: Vec3) -> Option<Transform> {
        let finite = m.iter().flatten().all(|v| v.is_finite()) && t.is_finite();
        finite.then_some(Transform { m, t })
    }

    /// A transform whose matrix columns are `x`, `y`, `z` and whose translation is
    /// `origin`: it maps local coordinates of that basis to world coordinates.
    pub fn from_basis(origin: Point3, x: Vec3, y: Vec3, z: Vec3) -> Option<Transform> {
        Transform::from_parts(
            [[x.x, y.x, z.x], [x.y, y.y, z.y], [x.z, y.z, z.z]],
            origin - Point3::ORIGIN,
        )
    }

    /// A translation by `v`, or `None` if `v` is not finite.
    pub fn translation(v: Vec3) -> Option<Transform> {
        Transform::from_parts(Transform::IDENTITY.m, v)
    }

    /// A uniform scale about the world origin, or `None` unless `s` is finite and non-zero.
    pub fn scale(s: f64) -> Option<Transform> {
        if !s.is_finite() || s == 0.0 {
            return None;
        }
        Transform::from_parts([[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s]], Vec3::ZERO)
    }

    /// A right-handed rotation by `angle` radians about `axis` through the world origin
    /// (counter-clockwise when the axis points at the viewer), or `None` for a zero or
    /// non-finite axis or a non-finite angle. Rodrigues' formula:
    /// `R = cos(a) I + sin(a) [k]x + (1 - cos(a)) k k^T`.
    pub fn rotation(axis: Vec3, angle: f64) -> Option<Transform> {
        if !angle.is_finite() {
            return None;
        }
        let k = axis.normalized()?;
        let (s, c) = angle.sin_cos();
        let v = 1.0 - c;
        let m = [
            [
                c + k.x * k.x * v,
                k.x * k.y * v - k.z * s,
                k.x * k.z * v + k.y * s,
            ],
            [
                k.y * k.x * v + k.z * s,
                c + k.y * k.y * v,
                k.y * k.z * v - k.x * s,
            ],
            [
                k.z * k.x * v - k.y * s,
                k.z * k.y * v + k.x * s,
                c + k.z * k.z * v,
            ],
        ];
        Transform::from_parts(m, Vec3::ZERO)
    }

    /// A rotation by `angle` about the axis through `point` in direction `axis`.
    pub fn rotation_about(point: Point3, axis: Vec3, angle: f64) -> Option<Transform> {
        let to_origin = Transform::translation(Point3::ORIGIN - point)?;
        let back = Transform::translation(point - Point3::ORIGIN)?;
        Some(
            to_origin
                .then(&Transform::rotation(axis, angle)?)
                .then(&back),
        )
    }

    /// The matrix, row-major.
    pub fn matrix(&self) -> [[f64; 3]; 3] {
        self.m
    }

    /// The translation part.
    pub fn translation_part(&self) -> Vec3 {
        self.t
    }

    pub fn apply_vector(&self, v: Vec3) -> Vec3 {
        let r = |row: [f64; 3]| row[0] * v.x + row[1] * v.y + row[2] * v.z;
        Vec3::new(r(self.m[0]), r(self.m[1]), r(self.m[2]))
    }

    pub fn apply_point(&self, p: Point3) -> Point3 {
        Point3::ORIGIN + self.apply_vector(p - Point3::ORIGIN) + self.t
    }

    /// The transform that applies `self` first, then `next`.
    pub fn then(&self, next: &Transform) -> Transform {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| next.m[i][k] * self.m[k][j]).sum();
            }
        }
        Transform {
            m,
            t: next.apply_vector(self.t) + next.t,
        }
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// True when the matrix is a proper rotation: `M^T M = I` entry-wise within
    /// [`ANGULAR_TOLERANCE`] (a dimensionless deviation, like an angle) and `det M > 0`.
    pub fn is_rigid(&self) -> bool {
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| self.m[k][i] * self.m[k][j]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                let within = (dot - want).abs() <= ANGULAR_TOLERANCE;
                if !within {
                    return false;
                }
            }
        }
        self.determinant() > 0.0
    }

    /// The inverse of a rigid transform (`M^T`, `-M^T t`), or `None` if not rigid.
    pub fn inverse_rigid(&self) -> Option<Transform> {
        if !self.is_rigid() {
            return None;
        }
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.m[j][i];
            }
        }
        let rt = Transform { m, t: Vec3::ZERO };
        let t = -rt.apply_vector(self.t);
        Transform::from_parts(m, t)
    }

    /// The inverse of an affine transform via the adjugate, or `None` when the matrix is
    /// singular (|det| below `ANGULAR_TOLERANCE` times the cube of its largest entry) or the
    /// result is not finite.
    pub fn inverse(&self) -> Option<Transform> {
        let m = &self.m;
        let det = self.determinant();
        let scale = m.iter().flatten().fold(0.0_f64, |a, v| a.max(v.abs()));
        let invertible = det.is_finite() && det.abs() > ANGULAR_TOLERANCE * scale * scale * scale;
        if !invertible {
            return None;
        }
        let inv_det = 1.0 / det;
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let inv = [
            [
                c(1, 1, 2, 2) * inv_det,
                -c(0, 1, 2, 2) * inv_det,
                c(0, 1, 1, 2) * inv_det,
            ],
            [
                -c(1, 0, 2, 2) * inv_det,
                c(0, 0, 2, 2) * inv_det,
                -c(0, 0, 1, 2) * inv_det,
            ],
            [
                c(1, 0, 2, 1) * inv_det,
                -c(0, 0, 2, 1) * inv_det,
                c(0, 0, 1, 1) * inv_det,
            ],
        ];
        let rt = Transform {
            m: inv,
            t: Vec3::ZERO,
        };
        Transform::from_parts(inv, -rt.apply_vector(self.t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn close(a: Point3, b: Point3) -> bool {
        a.distance(b) < 1e-12
    }

    #[test]
    fn quarter_turn_about_z_maps_x_to_y() {
        let r = Transform::rotation(Vec3::Z, FRAC_PI_2).unwrap();
        assert!(close(
            r.apply_point(Point3::new(1.0, 0.0, 0.0)),
            Point3::new(0.0, 1.0, 0.0)
        ));
        assert!(close(
            r.apply_point(Point3::new(0.0, 1.0, 5.0)),
            Point3::new(-1.0, 0.0, 5.0)
        ));
        assert!(r.is_rigid());
        assert!((r.determinant() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn a_third_turn_about_the_diagonal_cycles_the_axes() {
        // Rotating 120 degrees about (1,1,1) maps x -> y -> z -> x.
        let r = Transform::rotation(Vec3::new(1.0, 1.0, 1.0), 2.0 * PI / 3.0).unwrap();
        assert!(close(
            r.apply_point(Point3::new(1.0, 0.0, 0.0)),
            Point3::new(0.0, 1.0, 0.0)
        ));
        assert!(close(
            r.apply_point(Point3::new(0.0, 0.0, 1.0)),
            Point3::new(1.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn rotation_about_an_offset_axis_keeps_the_axis_point_fixed() {
        // Half turn about the vertical line through (1, 0, 0): the origin goes to (2, 0, 0).
        let r = Transform::rotation_about(Point3::new(1.0, 0.0, 0.0), Vec3::Z, PI).unwrap();
        assert!(close(
            r.apply_point(Point3::new(1.0, 0.0, 7.0)),
            Point3::new(1.0, 0.0, 7.0)
        ));
        assert!(close(
            r.apply_point(Point3::ORIGIN),
            Point3::new(2.0, 0.0, 0.0)
        ));
        // Vectors ignore the translation.
        let v = r.apply_vector(Vec3::X);
        assert!((v - (-Vec3::X)).length() < 1e-12);
    }

    #[test]
    fn compose_then_inverse_round_trips() {
        let a = Transform::rotation(Vec3::new(0.3, -1.0, 2.0), 0.7).unwrap();
        let b = Transform::translation(Vec3::new(10.0, -4.0, 2.5)).unwrap();
        let c = Transform::rotation_about(Point3::new(1.0, 2.0, 3.0), Vec3::Y, -1.1).unwrap();
        let abc = a.then(&b).then(&c);
        let p = Point3::new(3.0, -7.0, 11.0);
        // then() applies in order.
        let step = c.apply_point(b.apply_point(a.apply_point(p)));
        assert!(close(abc.apply_point(p), step));
        let inv = abc.inverse_rigid().unwrap();
        assert!(close(inv.apply_point(abc.apply_point(p)), p));
        assert!(close(abc.then(&inv).apply_point(p), p));
        let general = abc.inverse().unwrap();
        assert!(close(general.apply_point(abc.apply_point(p)), p));
    }

    #[test]
    fn affine_inverse_undoes_a_scale_and_rigid_inverse_refuses_it() {
        let s = Transform::scale(4.0)
            .unwrap()
            .then(&Transform::translation(Vec3::X).unwrap());
        assert!(!s.is_rigid());
        assert!(s.inverse_rigid().is_none());
        let p = Point3::new(1.0, 2.0, 3.0);
        // 4 p + (1,0,0) = (5, 8, 12).
        assert!(close(s.apply_point(p), Point3::new(5.0, 8.0, 12.0)));
        assert!(close(
            s.inverse()
                .unwrap()
                .apply_point(Point3::new(5.0, 8.0, 12.0)),
            p
        ));
    }

    #[test]
    fn hostile_input_gives_none() {
        assert!(Transform::rotation(Vec3::ZERO, 1.0).is_none());
        assert!(Transform::rotation(Vec3::Z, f64::NAN).is_none());
        assert!(Transform::rotation(Vec3::new(f64::INFINITY, 0.0, 0.0), 1.0).is_none());
        assert!(Transform::translation(Vec3::new(0.0, f64::NAN, 0.0)).is_none());
        assert!(Transform::scale(0.0).is_none());
        assert!(Transform::scale(f64::INFINITY).is_none());
        let singular = Transform::from_parts(
            [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]],
            Vec3::ZERO,
        )
        .unwrap();
        assert!(singular.inverse().is_none());
        assert!(singular.inverse_rigid().is_none());
        assert!(Transform::from_parts([[f64::NAN; 3]; 3], Vec3::ZERO).is_none());
        // A mirror is orthonormal but not a proper rotation.
        let mirror = Transform::from_parts(
            [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            Vec3::ZERO,
        )
        .unwrap();
        assert!(!mirror.is_rigid());
    }
}

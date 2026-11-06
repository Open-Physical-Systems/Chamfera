//! Analytic surfaces (rung K3): plane, cylinder, cone, sphere and torus.
//!
//! Every surface lives in a [`Frame`] and is parametrised by `(u, v)`. Formulas follow the
//! usual differential geometry of surfaces of revolution (do Carmo, *Differential Geometry
//! of Curves and Surfaces*, section 2-3; Piegl and Tiller, *The NURBS Book*, chapter 8 for
//! the same surfaces as rational patches). With `o, x, y, z` the frame,
//! `e(u) = cos u x + sin u y` and `t(u) = -sin u x + cos u y`:
//!
//! | Surface | `S(u, v)` | `u` | `v` |
//! |---|---|---|---|
//! | Plane | `o + u x + v y` | ℝ | ℝ |
//! | Cylinder (radius `r`) | `o + r e(u) + v z` | angle, period 2π | height, ℝ |
//! | Cone (radius `r` at `v = 0`, half-angle `α`) | `o + (r + v sin α) e(u) + v cos α z` | angle, period 2π | distance along the generator, `r + v sin α >= 0` |
//! | Sphere (radius `r`) | `o + r cos v e(u) + r sin v z` | longitude, period 2π | latitude, `[-π/2, π/2]` |
//! | Torus (major `R`, minor `a`) | `o + (R + a cos v) e(u) + a sin v z` | angle, period 2π | tube angle, period 2π |
//!
//! The four curved surfaces are surfaces of revolution about the frame's z axis with
//! meridian `(ρ(v), h(v))` (distance from the axis, height): `S = o + ρ e(u) + h z`, so
//! `S_u = ρ t(u)`, `S_v = ρ' e(u) + h' z`, and `S_u × S_v = ρ (h' e(u) - ρ' z)`. The
//! parametrisation is orthogonal (`S_u · S_v = 0`) and the surface normal
//! ([`Surface::normal_at`]) is the direction of `S_u × S_v`: away from the axis for a
//! cylinder and cone, away from the centre for a sphere and away from the tube's centre
//! circle for a torus. A face's `same_sense` flag says whether its outward normal is this
//! normal or its opposite.
//!
//! Seams and singular points: `u` is periodic on every curved surface (the seam is the
//! meridian `u = 0`, through the frame's x axis) and `v` is periodic on a torus. Points on
//! the axis (the sphere's poles, the cone's apex, a spindle torus's tips) have no defined
//! `u`; [`SurfaceParameters::singular`] says so. The torus's major radius may be smaller
//! than its minor radius (a spindle torus) or negative (the inner, lemon-shaped part of a
//! spindle torus: the tube's centre circle then lies on the far side of the axis), which
//! revolving an arc whose centre is near or beyond the axis needs.

use crate::check;
use crate::{KernelError, KernelResult};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point3, Vec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, TAU};

/// A circular cylinder about its frame's z axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cylinder {
    frame: Frame,
    radius: f64,
}

/// A circular cone about its frame's z axis: radius `radius` in the frame's x/y plane,
/// widening by `sin(half_angle)` per unit length along a generator (a negative half-angle
/// narrows toward +z). Only the nappe through the reference circle is the surface; the
/// apex is at `v = -radius / sin(half_angle)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cone {
    frame: Frame,
    radius: f64,
    half_angle: f64,
}

/// A sphere centred on its frame's origin; the poles lie on the frame's z axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    frame: Frame,
    radius: f64,
}

/// A torus about its frame's z axis: a circle of radius `minor_radius` in a plane through
/// the axis, centred `major_radius` from it, swept around the axis. See the module
/// documentation for spindle and negative major radii.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Torus {
    frame: Frame,
    major_radius: f64,
    minor_radius: f64,
}

impl Cylinder {
    pub fn new(frame: Frame, radius: f64) -> KernelResult<Cylinder> {
        Ok(Cylinder {
            frame,
            radius: check::length("radius", radius)?,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
}

impl Cone {
    /// `radius >= 0` (zero puts the apex at the frame's origin) and
    /// `0 < |half_angle| < π/2`.
    pub fn new(frame: Frame, radius: f64, half_angle: f64) -> KernelResult<Cone> {
        if !(radius.is_finite() && (0.0..=MAX_COORDINATE).contains(&radius)) {
            return Err(KernelError::InvalidInput(format!(
                "`radius` of a cone must be a finite length between 0 and {MAX_COORDINATE} mm"
            )));
        }
        let ok = half_angle.is_finite()
            && half_angle.abs() > ANGULAR_TOLERANCE
            && half_angle.abs() < FRAC_PI_2 - ANGULAR_TOLERANCE;
        if !ok {
            return Err(KernelError::InvalidInput(
                "`half_angle` of a cone must be a finite angle with 0 < |half_angle| < π/2 radians (use a cylinder or a plane otherwise)".into(),
            ));
        }
        Ok(Cone {
            frame,
            radius,
            half_angle,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    /// The radius at `v = 0` (in the frame's x/y plane).
    pub fn radius(&self) -> f64 {
        self.radius
    }
    pub fn half_angle(&self) -> f64 {
        self.half_angle
    }
    /// The `v` of the apex.
    pub fn apex_v(&self) -> f64 {
        -self.radius / self.half_angle.sin()
    }
    pub fn apex(&self) -> Point3 {
        self.frame.origin() + self.frame.z() * (self.apex_v() * self.half_angle.cos())
    }
}

impl Sphere {
    pub fn new(frame: Frame, radius: f64) -> KernelResult<Sphere> {
        Ok(Sphere {
            frame,
            radius: check::length("radius", radius)?,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
}

impl Torus {
    /// `minor_radius > 0`; `major_radius` finite with `|major_radius|` more than
    /// [`LINEAR_TOLERANCE`] (zero is a sphere) and `major_radius + minor_radius > 0`.
    pub fn new(frame: Frame, major_radius: f64, minor_radius: f64) -> KernelResult<Torus> {
        let minor_radius = check::length("minor_radius", minor_radius)?;
        let ok = major_radius.is_finite()
            && major_radius.abs() > LINEAR_TOLERANCE
            && major_radius.abs() <= MAX_COORDINATE
            && major_radius + minor_radius > LINEAR_TOLERANCE;
        if !ok {
            return Err(KernelError::InvalidInput(format!(
                "`major_radius` of a torus must be finite, non-zero, at most {MAX_COORDINATE} mm in size and more than -`minor_radius`"
            )));
        }
        Ok(Torus {
            frame,
            major_radius,
            minor_radius,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn major_radius(&self) -> f64 {
        self.major_radius
    }
    pub fn minor_radius(&self) -> f64 {
        self.minor_radius
    }
}

/// The surface a face lies on. A face also has an orientation flag saying whether its
/// outward normal is the surface normal or its opposite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SurfaceData", into = "SurfaceData")]
pub enum Surface {
    /// The plane through the frame's origin; its normal is the frame's z axis.
    Plane(Frame),
    Cylinder(Cylinder),
    Cone(Cone),
    Sphere(Sphere),
    Torus(Torus),
}

/// The serialized form of a [`Surface`]: the constructor arguments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum SurfaceData {
    Plane(Frame),
    Cylinder {
        frame: Frame,
        radius: f64,
    },
    Cone {
        frame: Frame,
        radius: f64,
        half_angle: f64,
    },
    Sphere {
        frame: Frame,
        radius: f64,
    },
    Torus {
        frame: Frame,
        major_radius: f64,
        minor_radius: f64,
    },
}

impl From<Surface> for SurfaceData {
    fn from(s: Surface) -> Self {
        match s {
            Surface::Plane(f) => SurfaceData::Plane(f),
            Surface::Cylinder(c) => SurfaceData::Cylinder {
                frame: c.frame,
                radius: c.radius,
            },
            Surface::Cone(c) => SurfaceData::Cone {
                frame: c.frame,
                radius: c.radius,
                half_angle: c.half_angle,
            },
            Surface::Sphere(s) => SurfaceData::Sphere {
                frame: s.frame,
                radius: s.radius,
            },
            Surface::Torus(t) => SurfaceData::Torus {
                frame: t.frame,
                major_radius: t.major_radius,
                minor_radius: t.minor_radius,
            },
        }
    }
}

impl TryFrom<SurfaceData> for Surface {
    type Error = KernelError;
    fn try_from(d: SurfaceData) -> KernelResult<Surface> {
        Ok(match d {
            SurfaceData::Plane(f) => Surface::Plane(f),
            SurfaceData::Cylinder { frame, radius } => {
                Surface::Cylinder(Cylinder::new(frame, radius)?)
            }
            SurfaceData::Cone {
                frame,
                radius,
                half_angle,
            } => Surface::Cone(Cone::new(frame, radius, half_angle)?),
            SurfaceData::Sphere { frame, radius } => Surface::Sphere(Sphere::new(frame, radius)?),
            SurfaceData::Torus {
                frame,
                major_radius,
                minor_radius,
            } => Surface::Torus(Torus::new(frame, major_radius, minor_radius)?),
        })
    }
}

/// The parameters of a point projected onto a surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceParameters {
    pub u: f64,
    pub v: f64,
    /// True when the point is on the surface's axis (within [`LINEAR_TOLERANCE`]), where
    /// `u` is not defined (it is then 0) — a pole, an apex or a torus tip.
    pub singular: bool,
}

/// The point of a surface nearest to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub u: f64,
    pub v: f64,
    pub point: Point3,
    pub distance: f64,
    /// False when other surface points are as close (e.g. a point on a cylinder's axis or
    /// a sphere's centre); `point` is then one of them.
    pub unique: bool,
}

/// The meridian `(ρ(v), h(v))` of a surface of revolution, with the antiderivatives the
/// exact measures need (see `topology::measure`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Meridian {
    /// `ρ = r + s v`, `h = c v` with `s = sin α`, `c = cos α` (a cylinder has `s = 0`,
    /// `c = 1`).
    Line { r: f64, s: f64, c: f64 },
    /// `ρ = r cos v`, `h = r sin v`.
    Sphere { r: f64 },
    /// `ρ = big + a cos v`, `h = a sin v`.
    Torus { big: f64, a: f64 },
}

/// `x` moved by whole turns into `[0, 2π)` (`rem_euclid` alone can round up to 2π).
fn turn(x: f64) -> f64 {
    let y = x.rem_euclid(TAU);
    if y >= TAU { 0.0 } else { y }
}

/// `∫ cos² v dv = v/2 + sin(2v)/4`.
fn cos2_integral(v: f64) -> f64 {
    v * 0.5 + (2.0 * v).sin() * 0.25
}

impl Meridian {
    pub(crate) fn rho(&self, v: f64) -> f64 {
        match *self {
            Meridian::Line { r, s, .. } => r + s * v,
            Meridian::Sphere { r } => r * v.cos(),
            Meridian::Torus { big, a } => big + a * v.cos(),
        }
    }
    pub(crate) fn height(&self, v: f64) -> f64 {
        match *self {
            Meridian::Line { c, .. } => c * v,
            Meridian::Sphere { r } => r * v.sin(),
            Meridian::Torus { a, .. } => a * v.sin(),
        }
    }
    /// `(ρ'(v), h'(v))`.
    pub(crate) fn tangent(&self, v: f64) -> (f64, f64) {
        match *self {
            Meridian::Line { s, c, .. } => (s, c),
            Meridian::Sphere { r } => (-r * v.sin(), r * v.cos()),
            Meridian::Torus { a, .. } => (-a * v.sin(), a * v.cos()),
        }
    }
    /// `G(v) = ∫ ρ |(ρ', h')| dv`: `|S_u × S_v| = G'(v)`, so a face's area is
    /// `∬ G'(v) du dv`.
    pub(crate) fn area_antiderivative(&self, v: f64) -> f64 {
        match *self {
            Meridian::Line { r, s, .. } => r * v + 0.5 * s * v * v,
            Meridian::Sphere { r } => r * r * v.sin(),
            Meridian::Torus { big, a } => a * (big * v + a * v.sin()),
        }
    }
    /// `(A(v), B(v), C(v))` with `A' = ρ² h' - ρ ρ' h`, `B' = ρ h'`, `C' = ρ ρ'`. For a point
    /// `c` with local coordinates `(cx, cy, cz)`,
    /// `(S - c) · (S_u × S_v) = A' - (cx cos u + cy sin u) B' + cz C'`.
    ///
    /// Derivations: a line meridian has `A' = ρ c (ρ - s v) = c r ρ`, `B' = c ρ`. A sphere
    /// has `A' = r³ cos v (cos² v + sin² v) = r³ cos v`, `B' = r² cos² v`. A torus has
    /// `A' = a ρ (ρ cos v + a sin² v) = a ((R² + a²) cos v + a R + a R cos² v)` and
    /// `B' = a (R cos v + a cos² v)`. Every `C = ρ² / 2`.
    pub(crate) fn volume_antiderivatives(&self, v: f64) -> (f64, f64, f64) {
        let rho = self.rho(v);
        let c_term = 0.5 * rho * rho;
        match *self {
            Meridian::Line { r, s, c } => {
                let g = r * v + 0.5 * s * v * v;
                (c * r * g, c * g, c_term)
            }
            Meridian::Sphere { r } => (r * r * r * v.sin(), r * r * cos2_integral(v), c_term),
            Meridian::Torus { big, a } => (
                a * ((big * big + a * a) * v.sin() + a * big * v + a * big * cos2_integral(v)),
                a * (big * v.sin() + a * cos2_integral(v)),
                c_term,
            ),
        }
    }
}

impl Surface {
    /// A cylinder (see [`Cylinder::new`]).
    pub fn cylinder(frame: Frame, radius: f64) -> KernelResult<Surface> {
        Ok(Surface::Cylinder(Cylinder::new(frame, radius)?))
    }
    /// A cone (see [`Cone::new`]).
    pub fn cone(frame: Frame, radius: f64, half_angle: f64) -> KernelResult<Surface> {
        Ok(Surface::Cone(Cone::new(frame, radius, half_angle)?))
    }
    /// A sphere (see [`Sphere::new`]).
    pub fn sphere(frame: Frame, radius: f64) -> KernelResult<Surface> {
        Ok(Surface::Sphere(Sphere::new(frame, radius)?))
    }
    /// A torus (see [`Torus::new`]).
    pub fn torus(frame: Frame, major_radius: f64, minor_radius: f64) -> KernelResult<Surface> {
        Ok(Surface::Torus(Torus::new(
            frame,
            major_radius,
            minor_radius,
        )?))
    }

    pub fn frame(&self) -> &Frame {
        match self {
            Surface::Plane(f) => f,
            Surface::Cylinder(c) => &c.frame,
            Surface::Cone(c) => &c.frame,
            Surface::Sphere(s) => &s.frame,
            Surface::Torus(t) => &t.frame,
        }
    }

    /// The meridian of a surface of revolution (`None` for a plane).
    pub(crate) fn meridian(&self) -> Option<Meridian> {
        match *self {
            Surface::Plane(_) => None,
            Surface::Cylinder(c) => Some(Meridian::Line {
                r: c.radius,
                s: 0.0,
                c: 1.0,
            }),
            Surface::Cone(c) => {
                let (s, co) = c.half_angle.sin_cos();
                Some(Meridian::Line {
                    r: c.radius,
                    s,
                    c: co,
                })
            }
            Surface::Sphere(s) => Some(Meridian::Sphere { r: s.radius }),
            Surface::Torus(t) => Some(Meridian::Torus {
                big: t.major_radius,
                a: t.minor_radius,
            }),
        }
    }

    /// The period of `u` (2π on every curved surface; none on a plane).
    pub fn period_u(&self) -> Option<f64> {
        match self {
            Surface::Plane(_) => None,
            _ => Some(TAU),
        }
    }
    /// The period of `v` (2π on a torus; none otherwise).
    pub fn period_v(&self) -> Option<f64> {
        match self {
            Surface::Torus(_) => Some(TAU),
            _ => None,
        }
    }
    /// The range of `v` (one period `[0, 2π)` on a torus).
    pub fn v_range(&self) -> (f64, f64) {
        match self {
            Surface::Plane(_) | Surface::Cylinder(_) => (f64::NEG_INFINITY, f64::INFINITY),
            Surface::Cone(c) => {
                if c.half_angle > 0.0 {
                    (c.apex_v(), f64::INFINITY)
                } else {
                    (f64::NEG_INFINITY, c.apex_v())
                }
            }
            Surface::Sphere(_) => (-FRAC_PI_2, FRAC_PI_2),
            Surface::Torus(_) => (0.0, TAU),
        }
    }

    /// `S(u, v)`.
    pub fn point_at(&self, u: f64, v: f64) -> Point3 {
        match (self, self.meridian()) {
            (Surface::Plane(f), _) => f.origin() + f.x() * u + f.y() * v,
            (_, Some(m)) => {
                let f = self.frame();
                let (s, c) = u.sin_cos();
                f.origin() + (f.x() * c + f.y() * s) * m.rho(v) + f.z() * m.height(v)
            }
            (_, None) => self.frame().origin(),
        }
    }

    /// The partial derivatives `(S_u, S_v)` at `(u, v)`.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3) {
        let f = self.frame();
        match self.meridian() {
            None => (f.x(), f.y()),
            Some(m) => {
                let (s, c) = u.sin_cos();
                let e = f.x() * c + f.y() * s;
                let t = f.y() * c - f.x() * s;
                let (dr, dh) = m.tangent(v);
                (t * m.rho(v), e * dr + f.z() * dh)
            }
        }
    }

    /// The unit normal at `(u, v)`: the direction of `S_u × S_v`. Where that vanishes
    /// (on the axis) it is the limit from the side where `ρ > 0` along the meridian at
    /// `u`: `±z` at a sphere's pole, the generator's normal at a cone's apex.
    pub fn normal_at(&self, u: f64, v: f64) -> Vec3 {
        let f = self.frame();
        match self.meridian() {
            None => f.z(),
            Some(m) => {
                let (s, c) = u.sin_cos();
                let e = f.x() * c + f.y() * s;
                let (dr, dh) = m.tangent(v);
                let speed = (dr * dr + dh * dh).sqrt();
                let sign = if m.rho(v) < 0.0 { -1.0 } else { 1.0 };
                if speed > 0.0 {
                    (e * dh - f.z() * dr) * (sign / speed)
                } else {
                    e
                }
            }
        }
    }

    /// The parameters of the point of the surface nearest to `p` (`u` in `[0, 2π)` on a
    /// curved surface, `v` in [`Surface::v_range`]). `p` must be valid.
    pub fn parameters(&self, p: Point3) -> SurfaceParameters {
        let l = self.frame().to_local(p);
        let rho_p = l.x.hypot(l.y);
        let singular = rho_p <= LINEAR_TOLERANCE;
        let u = if singular { 0.0 } else { turn(l.y.atan2(l.x)) };
        let (u, v) = match self {
            Surface::Plane(_) => {
                return SurfaceParameters {
                    u: l.x,
                    v: l.y,
                    singular: false,
                };
            }
            Surface::Cylinder(_) => (u, l.z),
            Surface::Cone(c) => {
                let (s, co) = c.half_angle.sin_cos();
                // Projection onto the generator in the half-plane of `u`; the nearest point
                // of the nappe is there, clamped at the apex (the generator in the
                // opposite half-plane is never nearer for a point with ρ >= 0).
                let v = (rho_p - c.radius) * s + l.z * co;
                let v = if c.radius + s * v < 0.0 {
                    c.apex_v()
                } else {
                    v
                };
                (u, v)
            }
            Surface::Sphere(_) => (u, l.z.atan2(rho_p)),
            Surface::Torus(t) => {
                // The tube circle in the half-plane of `u` (centre R) and in the opposite
                // half-plane (centre R seen from the other side, at -R): take the nearer.
                let (big, a) = (t.major_radius, t.minor_radius);
                let d1 = ((rho_p - big).hypot(l.z) - a).abs();
                let d2 = ((-rho_p - big).hypot(l.z) - a).abs();
                if d2 < d1 && !singular {
                    let v = turn(l.z.atan2(-rho_p - big));
                    (turn(u + std::f64::consts::PI), v)
                } else {
                    (u, turn(l.z.atan2(rho_p - big)))
                }
            }
        };
        SurfaceParameters { u, v, singular }
    }

    /// The point of the surface nearest to `p`.
    pub fn closest_point(&self, p: Point3) -> KernelResult<SurfacePoint> {
        check::point3("point", p)?;
        let q = self.parameters(p);
        let point = self.point_at(q.u, q.v);
        let unique = match self {
            Surface::Plane(_) => true,
            Surface::Sphere(s) => s.frame.origin().distance(p) > LINEAR_TOLERANCE,
            Surface::Torus(t) => {
                // On the axis every tube circle is equally near.
                !q.singular || t.frame.to_local(p).z.abs() > t.minor_radius + LINEAR_TOLERANCE
            }
            Surface::Cylinder(_) => !q.singular,
            Surface::Cone(_) => {
                // On the axis beyond the apex, inside the cone, every generator is as near.
                !q.singular || point.distance(p) <= LINEAR_TOLERANCE
            }
        };
        Ok(SurfacePoint {
            u: q.u,
            v: q.v,
            point,
            distance: point.distance(p),
            unique,
        })
    }

    /// The distance from `p` to the surface (infinite for an invalid point).
    pub fn distance(&self, p: Point3) -> f64 {
        self.closest_point(p)
            .map(|c| c.distance)
            .unwrap_or(f64::INFINITY)
    }

    /// The surface normal at a point of a plane (constant).
    pub fn plane_normal(&self) -> Option<Vec3> {
        match self {
            Surface::Plane(f) => Some(f.z()),
            _ => None,
        }
    }

    /// Signed distance of `p` from the surface along its normal, for a plane.
    pub fn plane_distance(&self, p: Point3) -> Option<f64> {
        match self {
            Surface::Plane(f) => Some((p - f.origin()).dot(f.z())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn near(a: Point3, b: Point3, tol: f64) -> bool {
        a.distance(b) <= tol
    }
    fn vnear(a: Vec3, b: Vec3, tol: f64) -> bool {
        (a - b).length() <= tol
    }

    fn tilted() -> Frame {
        Frame::from_normal_and_x(
            Point3::new(3.0, -2.0, 7.0),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(2.0, -1.0, 0.0),
        )
        .unwrap()
    }

    fn surfaces() -> Vec<Surface> {
        let f = tilted();
        vec![
            Surface::Plane(f),
            Surface::cylinder(f, 4.0).unwrap(),
            Surface::cone(f, 3.0, 0.4).unwrap(),
            Surface::cone(f, 3.0, -0.6).unwrap(),
            Surface::sphere(f, 5.0).unwrap(),
            Surface::torus(f, 6.0, 2.0).unwrap(),
            Surface::torus(f, 1.0, 2.0).unwrap(),
            Surface::torus(f, -1.0, 2.0).unwrap(),
        ]
    }

    #[test]
    fn points_follow_the_documented_formulas() {
        let f = Frame::XY;
        // Cylinder r = 2 at u = π/2, v = 3: (0, 2, 3).
        let c = Surface::cylinder(f, 2.0).unwrap();
        assert!(near(
            c.point_at(FRAC_PI_2, 3.0),
            Point3::new(0.0, 2.0, 3.0),
            1e-12
        ));
        // Cone r = 1, α = π/6 at u = 0, v = 2: ρ = 1 + 2 sin(π/6) = 2, h = 2 cos(π/6) = √3.
        let k = Surface::cone(f, 1.0, PI / 6.0).unwrap();
        assert!(near(
            k.point_at(0.0, 2.0),
            Point3::new(2.0, 0.0, 3f64.sqrt()),
            1e-12
        ));
        // Its apex is at v = -1 / sin(π/6) = -2, height -2 cos(π/6) = -√3.
        let Surface::Cone(cone) = k else { panic!() };
        assert!(near(
            cone.apex(),
            Point3::new(0.0, 0.0, -3f64.sqrt()),
            1e-12
        ));
        // Sphere r = 2 at latitude π/2: the north pole.
        let s = Surface::sphere(f, 2.0).unwrap();
        assert!(near(
            s.point_at(1.0, FRAC_PI_2),
            Point3::new(0.0, 0.0, 2.0),
            1e-12
        ));
        // Torus R = 5, a = 1 at u = π, v = 0: (-6, 0, 0); at v = π/2: (0, -5, 1) for u = 3π/2.
        let t = Surface::torus(f, 5.0, 1.0).unwrap();
        assert!(near(
            t.point_at(PI, 0.0),
            Point3::new(-6.0, 0.0, 0.0),
            1e-12
        ));
        assert!(near(
            t.point_at(1.5 * PI, FRAC_PI_2),
            Point3::new(0.0, -5.0, 1.0),
            1e-12
        ));
    }

    #[test]
    fn derivatives_match_finite_differences_and_normals_are_their_cross_product() {
        for s in surfaces() {
            for (u, v) in [(0.3, 0.2f64), (2.0, -0.4), (4.5, 1.1)] {
                let v = v.clamp(s.v_range().0 + 0.01, s.v_range().1 - 0.01);
                let h = 1e-6;
                let fu = (s.point_at(u + h, v) - s.point_at(u - h, v)) * (0.5 / h);
                let fv = (s.point_at(u, v + h) - s.point_at(u, v - h)) * (0.5 / h);
                let (du, dv) = s.derivatives(u, v);
                assert!(vnear(du, fu, 1e-6), "{s:?} S_u at {u}, {v}");
                assert!(vnear(dv, fv, 1e-6), "{s:?} S_v at {u}, {v}");
                let n = s.normal_at(u, v);
                assert!((n.length() - 1.0).abs() < 1e-12);
                let cross = du.cross(dv).normalized().unwrap();
                assert!(vnear(n, cross, 1e-9), "{s:?} normal at {u}, {v}");
            }
        }
    }

    #[test]
    fn normals_point_away_from_the_axis_or_centre() {
        let f = Frame::XY;
        let c = Surface::cylinder(f, 2.0).unwrap();
        assert!(vnear(c.normal_at(0.0, 5.0), Vec3::X, 1e-15));
        let s = Surface::sphere(f, 2.0).unwrap();
        assert!(vnear(s.normal_at(0.7, FRAC_PI_2), Vec3::Z, 1e-15));
        assert!(vnear(s.normal_at(0.7, -FRAC_PI_2), -Vec3::Z, 1e-15));
        // A cone opening upward at 45°: normal (1, 0, -1)/√2 at u = 0.
        let k = Surface::cone(f, 1.0, PI / 4.0).unwrap();
        let r = 0.5f64.sqrt();
        assert!(vnear(k.normal_at(0.0, 1.0), Vec3::new(r, 0.0, -r), 1e-15));
        // The torus's outer equator faces +x, its top faces +z.
        let t = Surface::torus(f, 5.0, 1.0).unwrap();
        assert!(vnear(t.normal_at(0.0, 0.0), Vec3::X, 1e-15));
        assert!(vnear(t.normal_at(0.0, FRAC_PI_2), Vec3::Z, 1e-15));
        assert!(vnear(t.normal_at(0.0, PI), -Vec3::X, 1e-15));
    }

    #[test]
    fn parameters_invert_point_at_across_the_seams() {
        for s in surfaces() {
            let (lo, hi) = s.v_range();
            for u in [0.0, 1e-9, 1.0, PI, TAU - 1e-9] {
                for v in [-1.2, -0.3, 0.0, 0.5, 1.4, 2.5, 6.0] {
                    if v <= lo + 1e-3 || v >= hi - 1e-3 {
                        continue;
                    }
                    let p = s.point_at(u, v);
                    if let Some(m) = s.meridian()
                        && m.rho(v) < 1e-3
                    {
                        continue;
                    }
                    let q = s.parameters(p);
                    assert!(!q.singular);
                    assert!(near(s.point_at(q.u, q.v), p, 1e-9), "{s:?} at {u}, {v}");
                    let c = s.closest_point(p).unwrap();
                    assert!(c.distance < 1e-9 && c.unique);
                    if s.period_u().is_some() {
                        assert!((0.0..TAU).contains(&q.u));
                        let du = (q.u - u).rem_euclid(TAU);
                        assert!(du < 1e-8 || du > TAU - 1e-8, "{s:?} u {u} -> {}", q.u);
                    }
                }
            }
        }
    }

    #[test]
    fn closest_points_off_the_surface_are_at_the_analytic_distance() {
        let f = Frame::XY;
        // Cylinder r = 2: a point at radius 5 is 3 away; the axis is 2 away (not unique).
        let c = Surface::cylinder(f, 2.0).unwrap();
        let cp = c.closest_point(Point3::new(3.0, 4.0, 1.0)).unwrap();
        assert!((cp.distance - 3.0).abs() < 1e-12 && cp.unique);
        assert!(near(cp.point, Point3::new(1.2, 1.6, 1.0), 1e-12));
        let axis = c.closest_point(Point3::new(0.0, 0.0, 9.0)).unwrap();
        assert!((axis.distance - 2.0).abs() < 1e-12 && !axis.unique);
        // Sphere r = 2: (0, 0, 5) is 3 from the north pole.
        let s = Surface::sphere(f, 2.0).unwrap();
        let sp = s.closest_point(Point3::new(0.0, 0.0, 5.0)).unwrap();
        assert!(
            (sp.distance - 3.0).abs() < 1e-12 && near(sp.point, Point3::new(0.0, 0.0, 2.0), 1e-12)
        );
        assert!(!s.closest_point(Point3::ORIGIN).unwrap().unique);
        // Cone r = 0 (apex at origin), α = 45°: the point (0, 0, -3) below the apex is
        // nearest to the apex, 3 away. (2, 0, 0) is 2 sin 45° = √2 from the generator.
        let k = Surface::cone(f, 0.0, PI / 4.0).unwrap();
        let below = k.closest_point(Point3::new(0.0, 0.0, -3.0)).unwrap();
        assert!((below.distance - 3.0).abs() < 1e-12);
        let side = k.closest_point(Point3::new(2.0, 0.0, 0.0)).unwrap();
        assert!((side.distance - 2f64.sqrt()).abs() < 1e-12);
        assert!(near(side.point, Point3::new(1.0, 0.0, 1.0), 1e-12));
        // Torus R = 5, a = 1: the centre of the tube (5, 0, 0) is 1 away from every tube
        // point; (8, 0, 0) is 2 from (6, 0, 0); (-8, 0, 0) is 2 from (-6, 0, 0).
        let t = Surface::torus(f, 5.0, 1.0).unwrap();
        let out = t.closest_point(Point3::new(-8.0, 0.0, 0.0)).unwrap();
        assert!(
            (out.distance - 2.0).abs() < 1e-12
                && near(out.point, Point3::new(-6.0, 0.0, 0.0), 1e-12)
        );
        // A spindle torus R = 1, a = 2: its tube circle, centred (1, 0) in the meridian
        // plane, meets the axis at heights ±√(2² - 1²) = ±√3.
        let spindle = Surface::torus(f, 1.0, 2.0).unwrap();
        let tip = Point3::new(0.0, 0.0, 3f64.sqrt());
        assert!(spindle.distance(tip) < 1e-12);
        // Hostile input.
        assert!(c.closest_point(Point3::new(f64::NAN, 0.0, 0.0)).is_err());
        assert_eq!(
            c.distance(Point3::new(f64::INFINITY, 0.0, 0.0)),
            f64::INFINITY
        );
    }

    #[test]
    fn poles_and_apexes_are_singular() {
        let f = Frame::XY;
        let s = Surface::sphere(f, 2.0).unwrap();
        let q = s.parameters(Point3::new(0.0, 0.0, -2.0));
        assert!(q.singular && (q.v + FRAC_PI_2).abs() < 1e-15);
        let k = Surface::cone(f, 2.0, -PI / 4.0).unwrap();
        let Surface::Cone(cone) = k else { panic!() };
        let q = k.parameters(cone.apex());
        assert!(q.singular && (q.v - cone.apex_v()).abs() < 1e-12);
    }

    #[test]
    fn constructors_refuse_bad_parameters() {
        let f = Frame::XY;
        assert!(Surface::cylinder(f, 0.0).is_err());
        assert!(Surface::cylinder(f, f64::NAN).is_err());
        assert!(Surface::cone(f, -1.0, 0.3).is_err());
        assert!(Surface::cone(f, 1.0, 0.0).is_err());
        assert!(Surface::cone(f, 1.0, FRAC_PI_2).is_err());
        assert!(Surface::cone(f, 1.0, f64::INFINITY).is_err());
        assert!(Surface::sphere(f, -2.0).is_err());
        assert!(Surface::torus(f, 0.0, 1.0).is_err());
        assert!(Surface::torus(f, -3.0, 1.0).is_err());
        assert!(Surface::torus(f, 3.0, 0.0).is_err());
        assert!(Surface::torus(f, f64::NAN, 1.0).is_err());
    }

    #[test]
    fn surfaces_round_trip_through_json_and_bad_ones_are_refused() {
        for s in surfaces() {
            let json = serde_json::to_string(&s).unwrap();
            let back: Surface = serde_json::from_str(&json).unwrap();
            let p = s.point_at(0.4, 0.3);
            assert!(near(back.point_at(0.4, 0.3), p, 1e-12));
        }
        let frame = serde_json::to_value(Frame::XY).unwrap();
        let bad = serde_json::json!({"cylinder": {"frame": frame, "radius": -1.0}});
        assert!(serde_json::from_value::<Surface>(bad).is_err());
        let bad = serde_json::json!({"cone": {"frame": frame, "radius": 1.0, "half_angle": 2.0}});
        assert!(serde_json::from_value::<Surface>(bad).is_err());
        let extra = serde_json::json!({"sphere": {"frame": frame, "radius": 1.0, "x": 1}});
        assert!(serde_json::from_value::<Surface>(extra).is_err());
    }
}

//! The geometry topology refers to: curves under edges and surfaces under faces.
//!
//! [`Curve`] wraps the K1 curves in space. An edge uses a curve over a parameter range
//! `[t0, t1]` (an [`EdgeCurve`]); its first half-edge runs from `c(t0)` to `c(t1)`.
//! The carrier of a face is a [`Surface`](crate::Surface) (rung K3, in `surface.rs`).
//!
//! Both serialize through plain data and re-validate on load with the curve constructors.

use crate::{Arc3, Circle3, Ellipse3, EllipticalArc3, KernelError, KernelResult, Line3, Segment3};
use chamfera_geom::{ANGULAR_TOLERANCE, Aabb, Frame, Point3, Vec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// A curve in space that an edge can lie on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CurveData", into = "CurveData")]
pub enum Curve {
    Line(Line3),
    Segment(Segment3),
    Circle(Circle3),
    Arc(Arc3),
    Ellipse(Ellipse3),
    EllipticalArc(EllipticalArc3),
}

/// The serialized form of a [`Curve`]: the constructor arguments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CurveData {
    Line {
        origin: Point3,
        direction: Vec3,
    },
    Segment {
        start: Point3,
        end: Point3,
    },
    Circle {
        frame: Frame,
        radius: f64,
    },
    Arc {
        frame: Frame,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        frame: Frame,
        rx: f64,
        ry: f64,
    },
    EllipticalArc {
        frame: Frame,
        rx: f64,
        ry: f64,
        start: f64,
        sweep: f64,
    },
}

impl From<Curve> for CurveData {
    fn from(c: Curve) -> Self {
        match c {
            Curve::Line(l) => CurveData::Line {
                origin: l.origin(),
                direction: l.direction(),
            },
            Curve::Segment(s) => CurveData::Segment {
                start: s.start(),
                end: s.end(),
            },
            Curve::Circle(c) => CurveData::Circle {
                frame: *c.frame(),
                radius: c.radius(),
            },
            Curve::Arc(a) => CurveData::Arc {
                frame: *a.circle().frame(),
                radius: a.circle().radius(),
                start: a.start_angle(),
                sweep: a.sweep(),
            },
            Curve::Ellipse(e) => CurveData::Ellipse {
                frame: *e.frame(),
                rx: e.rx(),
                ry: e.ry(),
            },
            Curve::EllipticalArc(a) => {
                let (lo, hi) = a.range();
                CurveData::EllipticalArc {
                    frame: *a.ellipse().frame(),
                    rx: a.ellipse().rx(),
                    ry: a.ellipse().ry(),
                    start: lo,
                    sweep: hi - lo,
                }
            }
        }
    }
}

impl TryFrom<CurveData> for Curve {
    type Error = KernelError;
    fn try_from(d: CurveData) -> KernelResult<Curve> {
        Ok(match d {
            CurveData::Line { origin, direction } => Curve::Line(Line3::new(origin, direction)?),
            CurveData::Segment { start, end } => Curve::Segment(Segment3::new(start, end)?),
            CurveData::Circle { frame, radius } => Curve::Circle(Circle3::new(frame, radius)?),
            CurveData::Arc {
                frame,
                radius,
                start,
                sweep,
            } => Curve::Arc(Arc3::new(frame, radius, start, sweep)?),
            CurveData::Ellipse { frame, rx, ry } => Curve::Ellipse(Ellipse3::new(frame, rx, ry)?),
            CurveData::EllipticalArc {
                frame,
                rx,
                ry,
                start,
                sweep,
            } => Curve::EllipticalArc(EllipticalArc3::new(frame, rx, ry, start, sweep)?),
        })
    }
}

impl Curve {
    pub fn point_at(&self, t: f64) -> Point3 {
        match self {
            Curve::Line(c) => c.point_at(t),
            Curve::Segment(c) => c.point_at(t),
            Curve::Circle(c) => c.point_at(t),
            Curve::Arc(c) => c.point_at(t),
            Curve::Ellipse(c) => c.point_at(t),
            Curve::EllipticalArc(c) => c.point_at(t),
        }
    }

    /// The first derivative `c'(t)`.
    pub fn d1(&self, t: f64) -> Vec3 {
        match self {
            Curve::Line(c) => c.d1(t),
            Curve::Segment(c) => c.d1(t),
            Curve::Circle(c) => c.d1(t),
            Curve::Arc(c) => c.d1(t),
            Curve::Ellipse(c) => c.d1(t),
            Curve::EllipticalArc(c) => c.d1(t),
        }
    }

    /// The distance from `p` to the part of the curve with parameter in `[t0, t1]` (a
    /// valid edge range).
    pub fn distance_on(&self, t0: f64, t1: f64, p: Point3) -> KernelResult<f64> {
        let (a, b) = (self.point_at(t0), self.point_at(t1));
        let d = match self {
            Curve::Line(_) | Curve::Segment(_) => {
                if a.coincident(b) {
                    a.distance(p)
                } else {
                    Segment3::new(a, b)?.closest_point(p)?.distance
                }
            }
            Curve::Circle(c) => {
                Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?
                    .closest_point(p)?
                    .distance
            }
            Curve::Arc(a) => {
                let c = a.circle();
                Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?
                    .closest_point(p)?
                    .distance
            }
            Curve::Ellipse(e) => {
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?
                    .closest_point(p)?
                    .distance
            }
            Curve::EllipticalArc(a) => {
                let e = a.ellipse();
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?
                    .closest_point(p)?
                    .distance
            }
        };
        Ok(d)
    }

    /// The same points run the other way, with the matching range: the curve `c'` and
    /// `[t0', t1']` with `c'(t0') = c(t1)` and `c'(t1') = c(t0)`.
    pub fn reversed_on(&self, t0: f64, t1: f64) -> KernelResult<(Curve, f64, f64)> {
        Ok(match self {
            Curve::Line(l) => (Curve::Line(l.reversed()), -t1, -t0),
            Curve::Segment(s) => (Curve::Segment(s.reversed()), 1.0 - t1, 1.0 - t0),
            Curve::Circle(c) => (Curve::Circle(c.reversed()), -t1, -t0),
            Curve::Arc(a) => {
                let r = a.reversed();
                // The reversed arc's parameter u covers the point c(t) at u = -t, moved by
                // whole turns into its own range.
                let (lo, _) = r.range();
                let u0 = crate::check::angle_from(-t1, lo);
                (Curve::Arc(r), u0, u0 + (t1 - t0))
            }
            Curve::Ellipse(e) => (Curve::Ellipse(e.reversed()), -t1, -t0),
            Curve::EllipticalArc(_) => {
                return Err(KernelError::Unsupported(
                    "reversing an elliptical arc edge".into(),
                ));
            }
        })
    }

    /// The curve's own parameter range (infinite for a line, one turn for a circle or
    /// ellipse).
    pub fn range(&self) -> (f64, f64) {
        match self {
            Curve::Line(c) => c.range(),
            Curve::Segment(c) => c.range(),
            Curve::Circle(c) => c.range(),
            Curve::Arc(c) => c.range(),
            Curve::Ellipse(c) => c.range(),
            Curve::EllipticalArc(c) => c.range(),
        }
    }

    /// True for closed curves (circle, ellipse), whose parameter may run past `range()`
    /// by whole turns.
    pub fn is_periodic(&self) -> bool {
        matches!(self, Curve::Circle(_) | Curve::Ellipse(_))
    }

    /// Checks that `[t0, t1]` is a usable edge range on this curve: finite, increasing,
    /// inside the curve's range (or at most one turn for a closed curve), with endpoints
    /// inside the model's working range.
    pub fn check_range(&self, t0: f64, t1: f64) -> KernelResult<()> {
        if !(t0.is_finite() && t1.is_finite() && t0 < t1) {
            return Err(KernelError::InvalidInput(
                "an edge range must be finite with `t0 < t1`".into(),
            ));
        }
        if self.is_periodic() {
            if t1 - t0 > TAU + ANGULAR_TOLERANCE {
                return Err(KernelError::InvalidInput(
                    "an edge on a circle or ellipse may span at most one turn (2π)".into(),
                ));
            }
        } else {
            let (lo, hi) = self.range();
            if t0 < lo || t1 > hi {
                return Err(KernelError::InvalidInput(format!(
                    "an edge range must lie inside the curve's range [{lo}, {hi}]"
                )));
            }
        }
        if !(self.point_at(t0).is_valid() && self.point_at(t1).is_valid()) {
            return Err(KernelError::InvalidInput(
                "an edge's end points must be finite and inside the model's working range".into(),
            ));
        }
        Ok(())
    }

    /// `∫ (c(t) - r) × c'(t) dt` over `[t0, t1]`, in closed form. Half of it summed around
    /// a closed loop is the loop's vector area (Stokes / Green); it does not depend on `r`
    /// for a closed loop, and `r` near the loop keeps the sum accurate.
    ///
    /// Derivation: a straight piece `c(t) = o + t d` gives `(o - r) × d (t1 - t0)`, which is
    /// `(c(t0) - r) × (c(t1) - r)`. A conic `c(t) = m + a cos t x + b sin t y` (circle with
    /// `a = b`) has `(c - m) × c' = a b z`, so the integral is
    /// `(m - r) × (c(t1) - c(t0)) + a b (t1 - t0) z`.
    pub fn area_moment(&self, t0: f64, t1: f64, r: Point3) -> Vec3 {
        let p0 = self.point_at(t0);
        let p1 = self.point_at(t1);
        let conic = |frame: &Frame, ab: f64| {
            (frame.origin() - r).cross(p1 - p0) + frame.z() * (ab * (t1 - t0))
        };
        match self {
            Curve::Line(_) | Curve::Segment(_) => (p0 - r).cross(p1 - r),
            Curve::Circle(c) => conic(c.frame(), c.radius() * c.radius()),
            Curve::Arc(a) => {
                let c = a.circle();
                conic(c.frame(), c.radius() * c.radius())
            }
            Curve::Ellipse(e) => conic(e.frame(), e.rx() * e.ry()),
            Curve::EllipticalArc(a) => {
                let e = a.ellipse();
                conic(e.frame(), e.rx() * e.ry())
            }
        }
    }

    /// The box around the curve's points with parameter in `[t0, t1]` (a valid range).
    pub fn bounding_box_on(&self, t0: f64, t1: f64) -> KernelResult<Aabb> {
        let ends = [self.point_at(t0), self.point_at(t1)];
        let straight = Aabb::from_points(ends)
            .ok_or_else(|| KernelError::InvalidInput("an edge range needs two end points".into()));
        match self {
            Curve::Line(_) | Curve::Segment(_) => straight,
            Curve::Circle(c) => Ok(Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?.bounding_box()),
            Curve::Arc(a) => {
                let c = a.circle();
                Ok(Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?.bounding_box())
            }
            Curve::Ellipse(e) => {
                Ok(EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?.bounding_box())
            }
            Curve::EllipticalArc(a) => {
                let e = a.ellipse();
                Ok(EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?.bounding_box())
            }
        }
    }
}

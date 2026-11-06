//! Rational B-spline (NURBS) curves in the plane and in space (kernel rung K10, curves).
//!
//! Written from Piegl and Tiller, *The NURBS Book*, 2nd ed. (Springer, 1997); section and
//! algorithm numbers refer to it:
//! - basis functions and their derivatives: A2.1-A2.3 (`basis.rs`);
//! - points and derivatives of rational curves: A4.1, A4.2 with A3.2 on the homogeneous
//!   curve (eq. 4.8);
//! - knot insertion: A5.1; splitting and segments by inserting a knot `p` times (§5.2);
//! - point inversion (closest point): Newton iteration (§6.1, eq. 6.3) from sampled
//!   seeds, with a subdivision fallback that prunes pieces by their control-point boxes
//!   (the convex hull property, §4.2);
//! - global interpolation through fit points with chord-length parameters, averaged
//!   knots and optional end derivatives: A9.1 and §9.2.2 (`fit.rs`);
//! - arc length by adaptive Gauss-Legendre quadrature of `|C'(u)|` per knot span.
//!
//! A curve is checked when it is made and when it is read from a file: degree
//! `1..=`[`MAX_DEGREE`], at most [`MAX_CONTROL_POINTS`] control points inside the working
//! range, finite weights in `[`[`MIN_WEIGHT`]`, `[`MAX_WEIGHT`]`]`, and a valid knot
//! vector (see [`BSplineCurve2::new`]). A curve with all weights equal is an ordinary
//! (non-rational) B-spline.

mod basis;
mod core;
mod fit;
mod planar;
#[cfg(test)]
mod tests;

pub use self::core::LENGTH_RELATIVE_TOLERANCE;
pub use fit::DEFAULT_SPLINE_DEGREE;

use self::core::{Arr, Nurbs};
use crate::{Frame, Point2, Point3, Transform, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Highest degree a curve may have.
pub const MAX_DEGREE: usize = 7;
/// Most control points one curve may have.
pub const MAX_CONTROL_POINTS: usize = 1024;
/// Most fit points one interpolated curve may have (the interpolation system is dense).
pub const MAX_FIT_POINTS: usize = 256;
/// Smallest weight a control point may have.
pub const MIN_WEIGHT: f64 = 1e-6;
/// Largest weight a control point may have.
pub const MAX_WEIGHT: f64 = 1e6;
/// Largest knot magnitude.
pub const MAX_KNOT: f64 = 1e9;
/// Knots closer than this (relative to the domain) are the same knot when splitting.
pub const KNOT_TOLERANCE: f64 = 1e-12;

/// Why a curve could not be made or a query answered.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum NurbsError {
    #[error("invalid curve: {0}")]
    InvalidInput(String),
    #[error("{0}")]
    Singular(String),
    #[error("a numeric method did not converge: {0}")]
    NoConvergence(String),
}

/// The point of a curve nearest to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveProjection<P> {
    pub u: f64,
    pub point: P,
    pub distance: f64,
}

/// The serialized form of a curve: its defining numbers, re-validated on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CurveData<P> {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<P>,
    weights: Vec<f64>,
}

type CurveData2 = CurveData<Point2>;
type CurveData3 = CurveData<Point3>;

fn p2(p: Point2) -> Arr<2> {
    [p.x, p.y]
}
fn a2(a: Arr<2>) -> Point2 {
    Point2::new(a[0], a[1])
}
fn v2(a: Arr<2>) -> Vec2 {
    Vec2::new(a[0], a[1])
}
fn p3(p: Point3) -> Arr<3> {
    [p.x, p.y, p.z]
}
fn a3(a: Arr<3>) -> Point3 {
    Point3::new(a[0], a[1], a[2])
}
fn v3(a: Arr<3>) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

macro_rules! curve_type {
    ($name:ident, $d:literal, $point:ty, $vector:ty, $to:ident, $pt:ident, $vec:ident, $what:literal, $data:literal) => {
        #[doc = concat!("A rational B-spline curve ", $what, " (see the module documentation).")]
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(try_from = $data, into = $data)]
        pub struct $name {
            inner: Nurbs<$d>,
        }

        impl TryFrom<CurveData<$point>> for $name {
            type Error = NurbsError;
            fn try_from(d: CurveData<$point>) -> Result<Self, NurbsError> {
                $name::new(d.degree, d.knots, d.points, Some(d.weights))
            }
        }

        impl From<$name> for CurveData<$point> {
            fn from(c: $name) -> Self {
                CurveData {
                    degree: c.degree(),
                    knots: c.inner.knots.clone(),
                    points: c.control_points(),
                    weights: c.inner.weights.clone(),
                }
            }
        }

        impl $name {
            /// A curve from its defining numbers: `degree` in `1..=MAX_DEGREE`, `n + 1`
            /// control points (`degree + 1 ..= MAX_CONTROL_POINTS`), `n + degree + 2`
            /// non-decreasing finite knots with a non-empty domain `[U_p, U_{n+1}]` and
            /// no interior knot repeated more than `degree` times, and one weight per
            /// point (`None`: all 1, a non-rational curve).
            pub fn new(
                degree: usize,
                knots: Vec<f64>,
                points: Vec<$point>,
                weights: Option<Vec<f64>>,
            ) -> Result<Self, NurbsError> {
                let weights = weights.unwrap_or_else(|| vec![1.0; points.len()]);
                let points = points.into_iter().map($to).collect();
                Ok($name {
                    inner: Nurbs::new(degree, knots, points, weights)?,
                })
            }

            /// The cubic (or lower, for few points) curve through `points` in order
            /// (global interpolation, §9.2), with optional start and end tangent
            /// directions (any length; scaled by the total chord length). The domain is
            /// `[0, 1]`.
            pub fn interpolate(
                points: &[$point],
                start_tangent: Option<$vector>,
                end_tangent: Option<$vector>,
            ) -> Result<Self, NurbsError> {
                let q: Vec<Arr<$d>> = points.iter().copied().map($to).collect();
                let t = |v: $vector| -> Arr<$d> { $to(<$point>::ORIGIN + v) };
                Ok($name {
                    inner: fit::interpolate(
                        &q,
                        DEFAULT_SPLINE_DEGREE,
                        start_tangent.map(t),
                        end_tangent.map(t),
                    )?,
                })
            }

            /// The closed cubic curve through `points` (the first point is not
            /// repeated), with a continuous tangent where it closes.
            pub fn interpolate_closed(points: &[$point]) -> Result<Self, NurbsError> {
                let q: Vec<Arr<$d>> = points.iter().copied().map($to).collect();
                Ok($name {
                    inner: fit::interpolate_closed(&q, DEFAULT_SPLINE_DEGREE)?,
                })
            }

            /// The cubic (or lower) spline with `points` as its control polygon:
            /// clamped uniform knots when open (it starts and ends at the end points),
            /// uniform periodic knots when `closed`.
            pub fn from_control_points(
                points: &[$point],
                closed: bool,
            ) -> Result<Self, NurbsError> {
                let q: Vec<Arr<$d>> = points.iter().copied().map($to).collect();
                let inner = if closed {
                    fit::control_closed(&q, DEFAULT_SPLINE_DEGREE)?
                } else {
                    fit::control_open(&q, DEFAULT_SPLINE_DEGREE)?
                };
                Ok($name { inner })
            }

            pub fn degree(&self) -> usize {
                self.inner.degree
            }
            pub fn knots(&self) -> &[f64] {
                &self.inner.knots
            }
            pub fn weights(&self) -> &[f64] {
                &self.inner.weights
            }
            pub fn control_points(&self) -> Vec<$point> {
                self.inner.points.iter().map(|a| $pt(*a)).collect()
            }
            /// True when the weights are not all equal.
            pub fn is_rational(&self) -> bool {
                self.inner.is_rational()
            }
            /// The parameter domain `[U_p, U_{n+1}]`.
            pub fn domain(&self) -> (f64, f64) {
                self.inner.domain()
            }
            /// The distinct knots of the domain, ends included: the curve is one rational
            /// polynomial piece between consecutive values.
            pub fn breaks(&self) -> Vec<f64> {
                self.inner.breaks()
            }

            /// `C(u)` (A4.1); `u` is clamped to the domain.
            pub fn point_at(&self, u: f64) -> $point {
                $pt(self.inner.point(u))
            }
            /// `C'(u)`.
            pub fn d1(&self, u: f64) -> $vector {
                self.derivatives(u, 1)
                    .get(1)
                    .copied()
                    .unwrap_or(<$vector>::ZERO)
            }
            /// `C''(u)`.
            pub fn d2(&self, u: f64) -> $vector {
                self.derivatives(u, 2)
                    .get(2)
                    .copied()
                    .unwrap_or(<$vector>::ZERO)
            }
            /// `C(u) - origin, C'(u), .., C^(k)(u)` (A4.2), `k <= MAX_DEGREE`.
            pub fn derivatives(&self, u: f64, k: usize) -> Vec<$vector> {
                self.inner
                    .derivs(u, k.min(MAX_DEGREE))
                    .into_iter()
                    .map($vec)
                    .collect()
            }
            /// True when the curve ends where it starts.
            pub fn is_closed(&self) -> bool {
                let (a, b) = self.domain();
                self.point_at(a).distance(self.point_at(b)) <= crate::LINEAR_TOLERANCE
            }

            /// The curve with the knot `u` inserted `times` more times (A5.1): the same
            /// curve with more control points.
            pub fn insert_knot(&self, u: f64, times: usize) -> Result<Self, NurbsError> {
                Ok($name {
                    inner: self.inner.insert_knot(u, times)?,
                })
            }
            /// The parts before and after `u` (strictly inside the domain).
            pub fn split_at(&self, u: f64) -> Result<(Self, Self), NurbsError> {
                let (a, b) = self.inner.split(u)?;
                Ok(($name { inner: a }, $name { inner: b }))
            }
            /// The part on `[a, b]`, keeping the parameterisation.
            pub fn segment(&self, a: f64, b: f64) -> Result<Self, NurbsError> {
                Ok($name {
                    inner: self.inner.segment(a, b)?,
                })
            }
            /// The same points run backwards; see [`Self::reversed_parameter`].
            pub fn reversed(&self) -> Self {
                $name {
                    inner: self.inner.reversed(),
                }
            }
            /// The parameter on [`Self::reversed`] of the point at `u` here.
            pub fn reversed_parameter(&self, u: f64) -> f64 {
                self.inner.reversed_parameter(u)
            }

            /// A box around the whole curve: the box of its control points (convex hull
            /// property).
            pub fn hull_box(&self) -> ($point, $point) {
                let (lo, hi) = self.inner.hull_box();
                ($pt(lo), $pt(hi))
            }
            /// A box around the curve's points on `[a, b]`: the control box of that
            /// segment.
            pub fn bounding_box_on(&self, a: f64, b: f64) -> Result<($point, $point), NurbsError> {
                Ok(self.segment(a, b)?.hull_box())
            }
            /// The point of the curve nearest to `p` (§6.1).
            pub fn closest_point(&self, p: $point) -> CurveProjection<$point> {
                let (u, distance) = self.inner.closest($to(p));
                CurveProjection {
                    u,
                    point: self.point_at(u),
                    distance,
                }
            }
            /// The arc length of the whole curve.
            pub fn length(&self) -> Result<f64, NurbsError> {
                let (a, b) = self.domain();
                self.inner.length(a, b)
            }
            /// The arc length from `a` to `b` (negative when `b < a`).
            pub fn length_between(&self, a: f64, b: f64) -> Result<f64, NurbsError> {
                self.inner.length(a, b)
            }
        }
    };
}

curve_type!(
    BSplineCurve2,
    2,
    Point2,
    Vec2,
    p2,
    a2,
    v2,
    "in the plane",
    "CurveData2"
);
curve_type!(
    BSplineCurve3,
    3,
    Point3,
    Vec3,
    p3,
    a3,
    v3,
    "in space",
    "CurveData3"
);

impl BSplineCurve2 {
    /// The same curve placed in the plane of `frame` (plane coordinates to world).
    pub fn to_3d(&self, frame: &Frame) -> Result<BSplineCurve3, NurbsError> {
        Ok(BSplineCurve3 {
            inner: self.inner.map(|a| p3(frame.from_plane(a2(a))))?,
        })
    }
    /// The curve mirrored in the x axis (`y -> -y`).
    pub fn mirrored_y(&self) -> Result<BSplineCurve2, NurbsError> {
        Ok(BSplineCurve2 {
            inner: self.inner.map(|a| [a[0], -a[1]])?,
        })
    }
    /// `½ ∫ (x dy - y dx)` over `[a, b]` (Green's theorem; summed around a closed loop it
    /// is the signed area), by adaptive Gauss-Legendre quadrature on each knot span. A
    /// non-rational span of degree `p` has a polynomial integrand of degree `2p - 1`,
    /// which the 5-point rule integrates exactly for `p <= 5`.
    pub fn area_term(&self, a: f64, b: f64) -> Result<f64, NurbsError> {
        self.moment_about(a, b, Point2::ORIGIN)
    }
    /// `½ ∫ ((x - r_x) dy - (y - r_y) dx)` over `[a, b]`.
    pub fn moment_about(&self, a: f64, b: f64, r: Point2) -> Result<f64, NurbsError> {
        let f = |u: f64| {
            let d = self.derivatives(u, 1);
            match (d.first(), d.get(1)) {
                (Some(c), Some(t)) => 0.5 * ((c.x - r.x) * t.y - (c.y - r.y) * t.x),
                _ => 0.0,
            }
        };
        let (lo, hi) = (self.inner.clamp(a.min(b)), self.inner.clamp(a.max(b)));
        let mut total = 0.0;
        let floor = 1e-15 * self.inner.size() * self.inner.size();
        for w in self.breaks().windows(2) {
            let (s, e) = (w[0].max(lo), w[1].min(hi));
            if e > s {
                total += self::core::integrate(&f, s, e, 1e-13, floor).ok_or_else(|| {
                    NurbsError::NoConvergence("the area integral did not converge".into())
                })?;
            }
        }
        Ok(if a <= b { total } else { -total })
    }
}

impl BSplineCurve3 {
    /// The curve moved by `t` (affine invariance: the control points are mapped).
    pub fn transformed(&self, t: &Transform) -> Result<BSplineCurve3, NurbsError> {
        Ok(BSplineCurve3 {
            inner: self.inner.map(|a| p3(t.apply_point(a3(a))))?,
        })
    }
    /// The curve in the plane coordinates of `frame` (the z coordinate dropped).
    pub fn to_plane(&self, frame: &Frame) -> Result<BSplineCurve2, NurbsError> {
        Ok(BSplineCurve2 {
            inner: self.inner.map(|a| p2(frame.to_plane(a3(a))))?,
        })
    }
    /// `∫ (C(u) - r) × C'(u) du` over `[a, b]` by adaptive Gauss-Legendre quadrature per
    /// knot span (half of it summed around a closed loop is the loop's vector area).
    pub fn area_moment(&self, a: f64, b: f64, r: Point3) -> Result<Vec3, NurbsError> {
        let comp = |k: usize| {
            move |u: f64| {
                let d = self.derivatives(u, 1);
                match (d.first(), d.get(1)) {
                    (Some(c), Some(t)) => {
                        let m = (Point3::ORIGIN + *c - r).cross(*t);
                        [m.x, m.y, m.z][k]
                    }
                    _ => 0.0,
                }
            }
        };
        let (lo, hi) = (self.inner.clamp(a.min(b)), self.inner.clamp(a.max(b)));
        let mut out = [0.0; 3];
        let floor = 1e-15 * self.inner.size() * self.inner.size();
        for (k, slot) in out.iter_mut().enumerate() {
            let f = comp(k);
            for w in self.breaks().windows(2) {
                let (s, e) = (w[0].max(lo), w[1].min(hi));
                if e > s {
                    *slot += self::core::integrate(&f, s, e, 1e-13, floor).ok_or_else(|| {
                        NurbsError::NoConvergence("the area integral did not converge".into())
                    })?;
                }
            }
        }
        let v = Vec3::new(out[0], out[1], out[2]);
        Ok(if a <= b { v } else { -v })
    }
}

//! Edge discretisation: each edge curve becomes one polyline, shared by both faces.
//!
//! Points are spaced uniformly in the curve parameter. The number of segments `n` is the
//! smallest that keeps both the chord error (the largest distance between the curve
//! and its chord) within `chord_tolerance` and the turn of the tangent per segment within
//! `angle_tolerance`:
//!
//! - **Line, segment**: one segment (the chord is the curve).
//! - **Circle, arc** of radius `r` over the angle `θ = t1 - t0`: a chord over the angle
//!   `φ` has sagitta `r (1 - cos(φ/2))`, so `φ ≤ 2 acos(1 - tol / r)`, evaluated as the
//!   equal but cancellation-free `4 asin(sqrt(tol / (2r)))` (because
//!   `1 - cos(φ/2) = 2 sin²(φ/4)`). The tangent turns by `φ` over the chord, so also
//!   `φ ≤ angle_tolerance`. Then `n = ceil(θ / φ)`.
//! - **Ellipse, elliptical arc** `c(t) = m + a cos t x + b sin t y`: linear
//!   interpolation over a parameter step `h` deviates from the curve by at most
//!   `h² max|c''| / 8`, and `|c''| ≤ max(a, b)`, so `h ≤ sqrt(8 tol / max(a, b))`. The
//!   tangent turns at a rate `κ |c'| = a b / (a² sin² t + b² cos² t) ≤ max / min`, so also
//!   `h ≤ angle_tolerance · min(a, b) / max(a, b)`. Then `n = ceil((t1 - t0) / h)`.
//!
//! Curved edges get at least two segments, so no loop of curved edges collapses to a
//! two-point polygon, and every segment spans at most a third of a turn (a full circle
//! has at least three).

use super::TessOptions;
use crate::{Curve, KernelError, KernelResult};
use chamfera_geom::Point3;
use std::f64::consts::{PI, TAU};

/// Largest parameter span (radians of the conic's angle) one curved segment may cover:
/// a third of a turn, so a full circle is at least a triangle.
const MAX_CURVED_STEP: f64 = TAU / 3.0;
/// Fewest segments a curved edge gets.
const MIN_CURVED_SEGMENTS: usize = 2;

/// The largest angle step of a circle of radius `r` that keeps the sagitta within
/// `tol` and the tangent turn within `angle`.
pub(super) fn circle_step(r: f64, tol: f64, angle: f64) -> f64 {
    let chord = if tol >= r {
        PI
    } else {
        4.0 * (tol / (2.0 * r)).sqrt().asin()
    };
    chord.min(angle).min(MAX_CURVED_STEP)
}

/// The largest parameter step of an ellipse with semi-axes `a`, `b` (see the module
/// documentation for the bound).
fn ellipse_step(a: f64, b: f64, tol: f64, angle: f64) -> f64 {
    let (big, small) = (a.max(b), a.min(b));
    let chord = (8.0 * tol / big).sqrt();
    chord.min(angle * small / big).min(MAX_CURVED_STEP)
}

/// How many segments the edge on `curve` over `[t0, t1]` needs. Refuses an edge that
/// alone would need more segments than the triangle budget.
pub(super) fn segment_count(
    curve: &Curve,
    t0: f64,
    t1: f64,
    options: &TessOptions,
) -> KernelResult<usize> {
    let span = t1 - t0;
    let (tol, angle) = (options.chord_tolerance, options.angle_tolerance);
    let step = match curve {
        Curve::Line(_) | Curve::Segment(_) => return Ok(1),
        Curve::Circle(c) => circle_step(c.radius(), tol, angle),
        Curve::Arc(a) => circle_step(a.circle().radius(), tol, angle),
        Curve::Ellipse(e) => ellipse_step(e.rx(), e.ry(), tol, angle),
        Curve::EllipticalArc(a) => {
            let e = a.ellipse();
            ellipse_step(e.rx(), e.ry(), tol, angle)
        }
        Curve::BSpline(c) => crate::extrusion::spline_step(c, t0, t1, tol, angle),
    };
    let n = (span / step).ceil();
    if !(n.is_finite() && step > 0.0) || n > options.max_triangles as f64 {
        return Err(KernelError::InvalidInput(format!(
            "an edge would need more than `max_triangles` = {} segments at this tolerance; raise `chord_tolerance` or `angle_tolerance`",
            options.max_triangles
        )));
    }
    // `n` is a finite whole number no larger than `max_triangles`, so it fits.
    Ok((n as usize).max(MIN_CURVED_SEGMENTS))
}

/// The `n + 1` points of the edge on `curve` over `[t0, t1]`: `start` and `end` are the
/// edge's vertex points (used as they are, so every edge at a vertex meets there
/// bit-identically), the others `curve(t0 + i (t1 - t0) / n)`.
pub(super) fn discretise(
    curve: &Curve,
    t0: f64,
    t1: f64,
    n: usize,
    start: Point3,
    end: Point3,
) -> Vec<Point3> {
    let mut out = Vec::with_capacity(n + 1);
    out.push(start);
    for i in 1..n {
        let t = t0 + (t1 - t0) * (i as f64 / n as f64);
        out.push(curve.point_at(t));
    }
    out.push(end);
    out
}

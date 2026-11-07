//! Crossings that involve ellipses and splines, for profile detection.
//!
//! Lines, arcs and circles meet in closed form (`intersect.rs`). An ellipse or a spline
//! meets another curve where `A(s) = B(t)`, two equations in the two curve parameters.
//! Each curve is sampled into a fine polyline (the parameter evenly spaced on each knot
//! span or quarter turn); every pair of polyline pieces that come close is a start, and
//! from each start Levenberg-Marquardt on `F(s, t) = A(s) - B(t)` with the exact
//! derivatives `[A'(s), -B'(t)]` converges to the crossing (Madsen, Nielsen and Tingleff,
//! *Methods for Non-Linear Least Squares Problems*, §3.2). A transversal crossing
//! converges quadratically to the last bits of the coordinates; a tangent touch, where the
//! Jacobian is singular, linearly, and is accepted once the two curves are within
//! `LINEAR_TOLERANCE`. The parameters are kept inside each curve's domain (wrapped for
//! closed curves), so an open curve's end can be a crossing (a T-junction) but a point
//! past it cannot.

use crate::ellipse::EllipseArc;
use crate::intersect::Shape;
use chamfera_geom::{BSplineCurve2, LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::Arc;

/// Polyline samples per quarter turn of an arc, circle or ellipse.
const SAMPLES_PER_QUARTER: usize = 16;
/// Polyline samples per knot span of a spline.
const SAMPLES_PER_SPAN: usize = 16;
/// Most samples of one curve (a spline with many spans gets fewer per span).
const MAX_SAMPLES: usize = 2048;
/// Iterations of the crossing search from one start.
const MAX_STEPS: usize = 80;
/// A crossing is accepted when the two curve points are this close (mm).
const ACCEPT: f64 = LINEAR_TOLERANCE;

/// A curve as a function of one parameter, with its derivative.
#[derive(Debug, Clone)]
pub(crate) enum Path {
    /// A line, arc or circle, with `Shape`'s parameter.
    Shape(Shape),
    /// An ellipse or elliptical arc; the parameter runs from 0 at its start through its
    /// sweep (`EllipseArc::point_at(t0 + t)`).
    Ellipse(EllipseArc),
    /// A spline with its own parameter.
    Spline {
        curve: Arc<BSplineCurve2>,
        closed: bool,
    },
}

impl Path {
    /// The parameter range.
    pub(crate) fn domain(&self) -> (f64, f64) {
        match self {
            Path::Shape(Shape::Segment { .. }) => (0.0, 1.0),
            Path::Shape(Shape::Arc { sweep, .. }) => (0.0, *sweep),
            Path::Shape(Shape::Circle { .. }) => (0.0, TAU),
            Path::Ellipse(e) => (0.0, e.sweep),
            Path::Spline { curve, .. } => curve.domain(),
        }
    }

    /// True for a closed curve (circle, whole ellipse, closed spline), whose parameter
    /// wraps around.
    pub(crate) fn periodic(&self) -> bool {
        match self {
            Path::Shape(Shape::Circle { .. }) => true,
            Path::Shape(_) => false,
            Path::Ellipse(e) => e.is_full(),
            Path::Spline { closed, .. } => *closed,
        }
    }

    /// The parameter brought into the domain: wrapped for a closed curve, clamped for an
    /// open one.
    fn fold(&self, t: f64) -> f64 {
        let (lo, hi) = self.domain();
        if !t.is_finite() {
            return lo;
        }
        if self.periodic() && hi > lo {
            lo + (t - lo).rem_euclid(hi - lo)
        } else {
            t.clamp(lo, hi)
        }
    }

    pub(crate) fn point_at(&self, t: f64) -> Point2 {
        match self {
            Path::Shape(s) => s.point_at(t),
            Path::Ellipse(e) => e.point_at(e.t0 + t),
            Path::Spline { curve, .. } => curve.point_at(t),
        }
    }

    fn d1(&self, t: f64) -> Vec2 {
        match self {
            Path::Shape(Shape::Segment { a, b }) => *b - *a,
            Path::Shape(Shape::Arc { r, a0, .. }) => {
                Vec2::new(-(a0 + t).sin(), (a0 + t).cos()) * *r
            }
            Path::Shape(Shape::Circle { r, .. }) => Vec2::new(-t.sin(), t.cos()) * *r,
            Path::Ellipse(e) => e.tangent_at(e.t0 + t),
            Path::Spline { curve, .. } => curve.d1(t),
        }
    }

    /// The end points of an open curve (none for a closed one).
    fn ends(&self) -> Option<(Point2, Point2)> {
        if self.periodic() {
            return None;
        }
        let (lo, hi) = self.domain();
        Some((self.point_at(lo), self.point_at(hi)))
    }

    /// True when `p` (on the curve) is inside it, farther than the tolerance from its
    /// ends. Every point of a closed curve is inside.
    pub(crate) fn interior(&self, p: Point2) -> bool {
        match self.ends() {
            None => true,
            Some((a, b)) => !a.coincident(p) && !b.coincident(p),
        }
    }

    /// The sample parameters, evenly spaced on each quarter turn or knot span, from the
    /// start to the end.
    fn samples(&self) -> Vec<f64> {
        let (lo, hi) = self.domain();
        let breaks: Vec<f64> = match self {
            Path::Shape(Shape::Segment { .. }) => vec![lo, hi],
            Path::Spline { curve, .. } => curve.breaks(),
            _ => {
                let quarters = ((hi - lo) / FRAC_PI_2).ceil().clamp(1.0, 4.0) as usize;
                (0..=quarters)
                    .map(|k| lo + (hi - lo) * k as f64 / quarters as f64)
                    .collect()
            }
        };
        let spans = breaks.len().saturating_sub(1).max(1);
        let per = match self {
            Path::Shape(Shape::Segment { .. }) => 1,
            Path::Spline { .. } => SAMPLES_PER_SPAN.min((MAX_SAMPLES / spans).max(2)),
            _ => SAMPLES_PER_QUARTER,
        };
        let mut out = Vec::with_capacity(spans * per + 1);
        for w in breaks.windows(2) {
            let (s, e) = (w[0], w[1]);
            for k in 0..per {
                out.push(s + (e - s) * k as f64 / per as f64);
            }
        }
        out.push(hi);
        out
    }

    /// A box around the curve (the control hull for a spline, which contains it).
    pub(crate) fn bounds(&self) -> (Point2, Point2) {
        let t = LINEAR_TOLERANCE;
        let (lo, hi) = match self {
            Path::Shape(Shape::Segment { a, b }) => (
                Point2::new(a.x.min(b.x), a.y.min(b.y)),
                Point2::new(a.x.max(b.x), a.y.max(b.y)),
            ),
            Path::Shape(Shape::Arc { c, r, .. } | Shape::Circle { c, r }) => {
                (Point2::new(c.x - r, c.y - r), Point2::new(c.x + r, c.y + r))
            }
            Path::Ellipse(e) => {
                // Half-widths of the ellipse's box: |a u_x| + |b v_x| bounds the reach in x.
                let (u, v) = (e.axis, e.minor_axis());
                let hx = (e.a * u.x).abs() + (e.b * v.x).abs();
                let hy = (e.a * u.y).abs() + (e.b * v.y).abs();
                (
                    Point2::new(e.centre.x - hx, e.centre.y - hy),
                    Point2::new(e.centre.x + hx, e.centre.y + hy),
                )
            }
            Path::Spline { curve, .. } => curve.hull_box(),
        };
        (
            Point2::new(lo.x - t, lo.y - t),
            Point2::new(hi.x + t, hi.y + t),
        )
    }
}

/// The parameters of the crossing nearest a start `(s, t)`, or `None` when the search
/// does not reach one.
fn converge(a: &Path, b: &Path, mut s: f64, mut t: f64) -> Option<(f64, f64)> {
    let mut f = a.point_at(s) - b.point_at(t);
    let mut lambda = 1e-12;
    for _ in 0..MAX_STEPS {
        let err = f.length();
        if !err.is_finite() {
            return None;
        }
        if err <= f64::EPSILON * 16.0 * (1.0 + a.point_at(s).distance(Point2::ORIGIN)) {
            break;
        }
        let (da, db) = (a.d1(s), -b.d1(t));
        // Normal equations (JᵀJ + λ diag) δ = -Jᵀ f of the 2 x 2 system.
        let (j11, j12, j22) = (da.dot(da), da.dot(db), db.dot(db));
        let (g1, g2) = (da.dot(f), db.dot(f));
        let (m11, m22) = (j11 * (1.0 + lambda), j22 * (1.0 + lambda));
        let det = m11 * m22 - j12 * j12;
        if !(det.is_finite() && det.abs() > f64::MIN_POSITIVE) {
            return None;
        }
        let ds = -(m22 * g1 - j12 * g2) / det;
        let dt = -(m11 * g2 - j12 * g1) / det;
        let (ns, nt) = (a.fold(s + ds), b.fold(t + dt));
        let nf = a.point_at(ns) - b.point_at(nt);
        if nf.length() < err {
            (s, t, f) = (ns, nt, nf);
            lambda = (lambda * 0.1).max(1e-15);
        } else {
            lambda *= 10.0;
            if lambda > 1e12 {
                break;
            }
        }
    }
    (f.length() <= ACCEPT).then_some((s, t))
}

/// Where two curves meet, ends included: each place once, with the parameter on `a`, the
/// parameter on `b` and the point (the middle of the two curve points, which agree to
/// the last bits for a crossing).
pub(crate) fn crossings(a: &Path, b: &Path) -> Vec<(f64, f64, Point2)> {
    let (ba, bb) = (a.bounds(), b.bounds());
    if ba.0.x > bb.1.x || bb.0.x > ba.1.x || ba.0.y > bb.1.y || bb.0.y > ba.1.y {
        return Vec::new();
    }
    let sample = |p: &Path| -> Vec<(f64, Point2)> {
        p.samples()
            .into_iter()
            .map(|t| (t, p.point_at(t)))
            .collect()
    };
    let (sa, sb) = (sample(a), sample(b));
    let mut out: Vec<(f64, f64, Point2)> = Vec::new();
    for wa in sa.windows(2) {
        let ((s0, p0), (s1, p1)) = (wa[0], wa[1]);
        // Pieces of the polyline lie within a little of the curve: let a close pair
        // through, with a margin of a quarter of the longer piece.
        let la = p0.distance(p1);
        for wb in sb.windows(2) {
            let ((t0, q0), (t1, q1)) = (wb[0], wb[1]);
            let m = 0.25 * la.max(q0.distance(q1)) + LINEAR_TOLERANCE;
            if p0.x.min(p1.x) - m > q0.x.max(q1.x)
                || q0.x.min(q1.x) - m > p0.x.max(p1.x)
                || p0.y.min(p1.y) - m > q0.y.max(q1.y)
                || q0.y.min(q1.y) - m > p0.y.max(p1.y)
            {
                continue;
            }
            if segment_distance(p0, p1, q0, q1) > m {
                continue;
            }
            let Some((mut s, mut t)) = converge(a, b, 0.5 * (s0 + s1), 0.5 * (t0 + t1)) else {
                continue;
            };
            if tangent_sine(a, b, s, t).0.abs() < TOUCH_SINE {
                let (rs, rt) = refine_touch(a, b, s, t);
                if a.point_at(rs).distance(b.point_at(rt)) <= ACCEPT {
                    (s, t) = (rs, rt);
                }
            }
            let p = a.point_at(s).lerp(b.point_at(t), 0.5);
            if !out.iter().any(|(_, _, q)| q.coincident(p)) {
                out.push((s, t, p));
            }
        }
    }
    out
}

/// Below this sine of the angle between the two tangents a crossing is a touch.
const TOUCH_SINE: f64 = 1e-3;
/// How far (mm) a touch found by the least-squares search may lie from the true one: the
/// gap grows only with the square of the distance along a touch, so the search can stop
/// up to about √(gap / curvature) away.
const TOUCH_WINDOW: f64 = 1e-3;
/// Bisection steps for a touch (to the last bits of the window).
const TOUCH_STEPS: usize = 64;

/// The parameter on `b` of the point nearest `p`, from a start `t` (Newton on
/// `(B(t) - p) · B'(t) = 0`, in Gauss-Newton form).
fn foot(b: &Path, p: Point2, mut t: f64) -> f64 {
    for _ in 0..12 {
        let d = b.d1(t);
        let l2 = d.dot(d);
        if !(l2.is_finite() && l2 > f64::MIN_POSITIVE) {
            break;
        }
        let step = (b.point_at(t) - p).dot(d) / l2;
        t = b.fold(t - step);
        if step.abs() < f64::EPSILON {
            break;
        }
    }
    t
}

/// The sine of the angle from `B'` to `A'` at `s` and the foot of `A(s)` on `b`, with
/// that foot's parameter.
fn tangent_sine(a: &Path, b: &Path, s: f64, t: f64) -> (f64, f64) {
    let t = foot(b, a.point_at(s), t);
    let (da, db) = (a.d1(s), b.d1(t));
    let n = da.length() * db.length();
    let sine = if n > f64::MIN_POSITIVE {
        da.cross(db) / n
    } else {
        0.0
    };
    (sine, t)
}

/// Where two curves touch, from a least-squares answer `(s, t)` near it: at the touch
/// the tangents are parallel, so the sine of the angle between them changes sign, and
/// bisection on it within [`TOUCH_WINDOW`] finds the touch to the last bits. Returns the
/// start unchanged when the sine keeps its sign (two crossings very close together).
fn refine_touch(a: &Path, b: &Path, s: f64, t: f64) -> (f64, f64) {
    let speed = a.d1(s).length();
    if !(speed.is_finite() && speed > f64::MIN_POSITIVE) {
        return (s, t);
    }
    let w = TOUCH_WINDOW / speed;
    let (mut lo, mut hi) = (s - w, s + w);
    let (lo_sine, _) = tangent_sine(a, b, a.fold(lo), t);
    let (hi_sine, _) = tangent_sine(a, b, a.fold(hi), t);
    if !(lo_sine.is_finite() && hi_sine.is_finite()) || (lo_sine > 0.0) == (hi_sine > 0.0) {
        return (s, t);
    }
    let lo_positive = lo_sine > 0.0;
    for _ in 0..TOUCH_STEPS {
        let m = 0.5 * (lo + hi);
        if m <= lo || m >= hi {
            break;
        }
        let (sine, _) = tangent_sine(a, b, a.fold(m), t);
        if (sine > 0.0) == lo_positive {
            lo = m;
        } else {
            hi = m;
        }
    }
    let s = a.fold(0.5 * (lo + hi));
    (s, foot(b, a.point_at(s), t))
}

/// The shortest distance between segments `p0 p1` and `q0 q1`.
fn segment_distance(p0: Point2, p1: Point2, q0: Point2, q1: Point2) -> f64 {
    let (d1, d2) = (p1 - p0, q1 - q0);
    let den = d1.cross(d2);
    if den.abs() > f64::MIN_POSITIVE {
        let s = (q0 - p0).cross(d2) / den;
        let t = (q0 - p0).cross(d1) / den;
        if (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t) {
            return 0.0;
        }
    }
    let to_seg = |p: Point2, a: Point2, b: Point2| {
        let d = b - a;
        let l2 = d.dot(d);
        let k = if l2 > 0.0 {
            ((p - a).dot(d) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(a + d * k)
    };
    to_seg(p0, q0, q1)
        .min(to_seg(p1, q0, q1))
        .min(to_seg(q0, p0, p1))
        .min(to_seg(q1, p0, p1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spline::{SplineForm, spline_geometry};

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn ellipse(c: Point2, a: f64, b: f64) -> Path {
        Path::Ellipse(EllipseArc {
            centre: c,
            axis: Vec2::X,
            a,
            b,
            t0: 0.0,
            sweep: TAU,
        })
    }

    fn has(points: &[(f64, f64, Point2)], q: Point2, tol: f64) -> bool {
        points.iter().any(|(_, _, x)| x.distance(q) < tol)
    }

    #[test]
    fn a_line_meets_an_ellipse_where_the_equations_say() {
        // x²/100 + y²/25 = 1 and y = 3: x² = 100 (1 - 9/25) = 64, x = ±8.
        let e = ellipse(p(0.0, 0.0), 10.0, 5.0);
        let line = Path::Shape(Shape::Segment {
            a: p(-20.0, 3.0),
            b: p(20.0, 3.0),
        });
        let x = crossings(&line, &e);
        assert_eq!(x.len(), 2, "{x:?}");
        assert!(has(&x, p(8.0, 3.0), 1e-12) && has(&x, p(-8.0, 3.0), 1e-12));
        // The diagonal y = x: x² (1/100 + 1/25) = 1, x = ±√20.
        let diag = Path::Shape(Shape::Segment {
            a: p(-20.0, -20.0),
            b: p(20.0, 20.0),
        });
        let r = 20f64.sqrt();
        let x = crossings(&diag, &e);
        assert_eq!(x.len(), 2);
        assert!(has(&x, p(r, r), 1e-12) && has(&x, p(-r, -r), 1e-12));
        // The tangent y = 5 touches once, at (0, 5); y = 5.1 misses.
        let top = Path::Shape(Shape::Segment {
            a: p(-20.0, 5.0),
            b: p(20.0, 5.0),
        });
        let x = crossings(&top, &e);
        assert_eq!(x.len(), 1, "{x:?}");
        assert!(has(&x, p(0.0, 5.0), 1e-9));
        let miss = Path::Shape(Shape::Segment {
            a: p(-20.0, 5.1),
            b: p(20.0, 5.1),
        });
        assert!(crossings(&miss, &e).is_empty());
        // A segment that stops short of the ellipse does not meet it.
        let short = Path::Shape(Shape::Segment {
            a: p(-20.0, 3.0),
            b: p(-9.0, 3.0),
        });
        assert!(crossings(&short, &e).is_empty());
    }

    #[test]
    fn a_circle_meets_an_ellipse_at_four_known_points() {
        // x²/100 + y²/25 = 1 gives x² = 100 - 4 y²; with x² + y² = 49 that is
        // 100 - 3 y² = 49, so y² = 17 and x² = 32.
        let e = ellipse(p(0.0, 0.0), 10.0, 5.0);
        let c = Path::Shape(Shape::Circle {
            c: p(0.0, 0.0),
            r: 7.0,
        });
        let x = crossings(&c, &e);
        assert_eq!(x.len(), 4, "{x:?}");
        let (px, py) = (32f64.sqrt(), 17f64.sqrt());
        for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            assert!(has(&x, p(sx * px, sy * py), 1e-12));
        }
    }

    #[test]
    fn two_ellipses_cross_on_the_diagonals() {
        // x²/100 + y²/25 = 1 and x²/25 + y²/100 = 1 meet where x² = y² = 20.
        let a = ellipse(p(0.0, 0.0), 10.0, 5.0);
        let b = ellipse(p(0.0, 0.0), 5.0, 10.0);
        let x = crossings(&a, &b);
        assert_eq!(x.len(), 4, "{x:?}");
        let r = 20f64.sqrt();
        assert!(has(&x, p(r, -r), 1e-12) && has(&x, p(-r, r), 1e-12));
    }

    #[test]
    fn a_line_meets_a_spline_parabola_where_the_parabola_says() {
        // The fit spline through (0, 0), (10, 10), (20, 0) is the parabola
        // y = x (20 - x) / 10 (see `spline.rs`); y = 5 meets it where x² - 20x + 50 = 0,
        // x = 10 ± √50.
        let curve = spline_geometry(
            &[p(0.0, 0.0), p(10.0, 10.0), p(20.0, 0.0)],
            SplineForm::Fit,
            false,
        )
        .unwrap();
        let sp = Path::Spline {
            curve: Arc::new(curve),
            closed: false,
        };
        let line = Path::Shape(Shape::Segment {
            a: p(-5.0, 5.0),
            b: p(25.0, 5.0),
        });
        let x = crossings(&sp, &line);
        assert_eq!(x.len(), 2, "{x:?}");
        let r = 50f64.sqrt();
        assert!(has(&x, p(10.0 - r, 5.0), 1e-9) && has(&x, p(10.0 + r, 5.0), 1e-9));
        // Ends count: the base line y = 0 meets it at both of its ends, which are not inside.
        let base = Path::Shape(Shape::Segment {
            a: p(-5.0, 0.0),
            b: p(25.0, 0.0),
        });
        let x = crossings(&sp, &base);
        assert_eq!(x.len(), 2, "{x:?}");
        assert!(x.iter().all(|(_, _, q)| !sp.interior(*q)));
    }

    #[test]
    fn hostile_curves_give_no_points() {
        let e = ellipse(p(f64::NAN, 0.0), 10.0, 5.0);
        let line = Path::Shape(Shape::Segment {
            a: p(-20.0, 3.0),
            b: p(20.0, 3.0),
        });
        assert!(crossings(&line, &e).is_empty());
        let dot = Path::Shape(Shape::Segment {
            a: p(1.0, 1.0),
            b: p(1.0, 1.0),
        });
        let good = ellipse(p(0.0, 0.0), 10.0, 5.0);
        assert!(crossings(&dot, &good).is_empty());
    }
}

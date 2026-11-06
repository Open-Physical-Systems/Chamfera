//! Numerics for spline intersections (rung K10 on K6): the roots of a distance field
//! along a B-spline, and the common points of two plane curves, by subdivision with
//! convex-hull boxes and Newton refinement.
//!
//! # Method
//! A NURBS curve with positive weights lies in the convex hull of its control points, and
//! so does every piece of it once the piece is cut out by knot insertion (The NURBS Book,
//! §4.2 and §5.2). That gives a box around every parameter interval that is never too
//! small, so an interval whose box misses the other object holds no common point and is
//! dropped. The rest are halved until they are small, then refined by Newton's method;
//! this is the subdivision scheme of Lane and Riesenfeld ("A theoretical development for
//! the computer generation and display of piecewise polynomial surfaces", 1980) as
//! Patrikalakis and Maekawa (*Shape Interrogation for Computer Aided Design and
//! Manufacturing*, §5.6 to §5.8) use it for curve-surface and curve-curve intersection:
//! subdivision to isolate the roots, Newton to make them exact to rounding.
//!
//! - **A field along a curve** ([`field_roots`]): `g(p)` is a signed distance (to a plane,
//!   a cylinder, a sphere, any surface by its closest point), so `|g|` grows by at most
//!   the distance moved and a box of radius `ρ` about a point with `|g| > ρ` holds no
//!   root. Newton steps `t ← t - g / (∇g · c')`.
//! - **Two plane curves** ([`pair_roots`]): two boxes that do not overlap (by more than
//!   the tolerance) hold no common point; otherwise the larger one is halved. Newton
//!   solves `a(s) - b(t) = 0` with the 2 × 2 Jacobian `[a', -b']`.
//!
//! An interval becomes a leaf when it can hold only one root: its box is within
//! [`SMALL_FRACTION`] of the whole problem's size and its sampled tangents are apart from
//! the other piece's (the tangent-cone test of Sederberg and Meyers, "Loop detection in
//! surface patch intersections", 1988; for a field, `g` is monotone along it), or the
//! pieces are flat within [`LINEAR_TOLERANCE`] (two straight pieces cross once or run
//! together within the tolerance). In a leaf Newton runs from the middle; a root it
//! converges to within the interval (or next to it) is kept. Where Newton fails (the
//! Jacobian is singular at a tangent contact) the interval's samples decide: the nearest
//! sample within [`LINEAR_TOLERANCE`] is a touching point, which is then placed exactly
//! by bisection on where the tangents align. Candidates within
//! the tolerance of each other, or joined by a stretch that stays within it, are one
//! point; a point where the curves (or the curve and the field's level set) share a
//! tangent direction within [`ANGULAR_TOLERANCE`], or a cluster that spreads beyond the
//! tolerance, is a tangent contact.
//!
//! # Bounds
//! Work is bounded: at most [`MAX_VISITS`] intervals (or interval pairs) per call, a
//! depth cap of [`MAX_DEPTH`] halvings and [`NEWTON_STEPS`] Newton steps per start. A
//! problem that needs more (two curves that run together within the tolerance over a
//! long stretch without being the same curve) is refused with
//! [`KernelError::Unsupported`], never answered by a guess.

use crate::{Curve, KernelError, KernelResult};
use chamfera_geom::{
    ANGULAR_TOLERANCE, BSplineCurve2, BSplineCurve3, Frame, LINEAR_TOLERANCE, Point2, Point3, Vec2,
    Vec3,
};

/// Most intervals (or interval pairs) one call may look at.
pub(crate) const MAX_VISITS: usize = 200_000;
/// Most halvings of one interval (of both, for a pair).
pub(crate) const MAX_DEPTH: usize = 96;
/// An interval whose box is within this fraction of the problem's size, and whose
/// tangents show it can cross only once, is refined by Newton instead of split.
pub(crate) const SMALL_FRACTION: f64 = 1e-3;
/// Most Newton steps from one start.
pub(crate) const NEWTON_STEPS: usize = 40;
/// Newton has converged when the residual is within this (mm): far inside the
/// tolerance, so vertices made from different intersections agree.
const NEWTON_RESIDUAL: f64 = 1e-3 * LINEAR_TOLERANCE;
/// Samples of a small interval that decide a touching point where Newton fails.
const TOUCH_SAMPLES: usize = 9;
/// Bisection steps placing a touching point (to the last bit of the bracket).
const BISECTION_STEPS: usize = 200;

fn too_much(what: &str) -> KernelError {
    KernelError::Unsupported(format!(
        "{what} run within the tolerance of each other over a stretch; their common points cannot be separated"
    ))
}

/// A root of a field along a curve: its parameter and whether the curve only touches
/// the level set there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FieldRoot {
    pub t: f64,
    pub tangent: bool,
}

/// A signed distance field: its value at a point and its unit gradient there.
pub(crate) type Field<'a> = &'a dyn Fn(Point3) -> (f64, Vec3);

fn box_of3(lo: Point3, hi: Point3) -> (Point3, f64) {
    let c = Point3::new(
        0.5 * (lo.x + hi.x),
        0.5 * (lo.y + hi.y),
        0.5 * (lo.z + hi.z),
    );
    (c, 0.5 * lo.distance(hi))
}

/// The parameters in `[t0, t1]` where `field` vanishes along `c` (see the module
/// documentation). `field` must be a signed distance (`|g|` at most the distance to the
/// zero set), whose zero set bends with radius at least `level_radius` (infinite for a
/// plane, 0 when unknown). The range must lie in the curve's domain.
pub(crate) fn field_roots(
    c: &BSplineCurve3,
    t0: f64,
    t1: f64,
    field: Field<'_>,
    level_radius: f64,
) -> KernelResult<Vec<FieldRoot>> {
    let (lo, hi) = c.domain();
    let (t0, t1) = (t0.max(lo), t1.min(hi));
    if t0.is_nan() || t1.is_nan() || t0 >= t1 {
        return Ok(Vec::new());
    }
    let whole = c.segment(t0, t1)?;
    let (blo, bhi) = whole.hull_box();
    let scale = blo.distance(bhi).max(LINEAR_TOLERANCE);
    let small = (SMALL_FRACTION * scale).max(LINEAR_TOLERANCE);
    let mut stack = vec![(whole, t0, t1, 0usize)];
    // (parameter, found by sampling a touch rather than by Newton)
    let mut found: Vec<(f64, bool)> = Vec::new();
    let mut visits = 0usize;
    while let Some((piece, a, b, depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_VISITS {
            return Err(too_much("a spline and a surface"));
        }
        let (plo, phi) = piece.hull_box();
        let (centre, radius) = box_of3(plo, phi);
        let (g, _) = field(centre);
        if g.is_nan() || g.abs() > radius + LINEAR_TOLERANCE {
            continue;
        }
        let m = 0.5 * (a + b);
        // A leaf holds at most one root: small with `g` monotone along it, or flat (the
        // piece within the tolerance of its chord and the zero set within it over the
        // box), where a second root would be within the tolerance of touching.
        let monotone = 2.0 * radius <= small && field_monotone(c, field, a, b);
        let flat = flatness3(&piece) <= LINEAR_TOLERANCE
            && (2.0 * radius).powi(2) <= 8.0 * LINEAR_TOLERANCE * level_radius;
        if monotone || flat || depth >= MAX_DEPTH || !(m > a && m < b) {
            if let Some(t) = field_newton(c, field, a, b) {
                found.push((t, false));
            } else if let Some(t) = field_touch(c, field, a, b) {
                found.push((t, true));
            }
            continue;
        }
        let (l, r) = (piece.segment(a, m)?, piece.segment(m, b)?);
        stack.push((r, m, b, depth + 1));
        stack.push((l, a, m, depth + 1));
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let g_at = |t: f64| field(c.point_at(t)).0.abs();
    // Clusters of candidates are one root: (first, last, best, any touch).
    let mut out: Vec<(f64, f64, f64, bool)> = Vec::new();
    for (t, touch) in found {
        match out.last_mut() {
            Some(last)
                if c.point_at(last.1).distance(c.point_at(t)) <= LINEAR_TOLERANCE
                    || g_at(0.5 * (last.1 + t)) <= LINEAR_TOLERANCE =>
            {
                last.1 = t;
                last.3 |= touch;
                if g_at(t) < g_at(last.2) {
                    last.2 = t;
                }
            }
            _ => out.push((t, t, t, touch)),
        }
    }
    Ok(out
        .into_iter()
        .map(|(first, last, best, touch)| {
            let d = c.d1(best);
            let (_, n) = field(c.point_at(best));
            let spread = c.point_at(first).distance(c.point_at(last)) > LINEAR_TOLERANCE;
            let tangent = touch || spread || n.dot(d).abs() <= ANGULAR_TOLERANCE * d.length();
            let t = if tangent {
                let w = (last - first) + SMALL_FRACTION * (t1 - t0);
                field_tangent(c, field, (first - w).max(t0), (last + w).min(t1), best)
            } else {
                best
            };
            FieldRoot { t, tangent }
        })
        .collect())
}

/// The touching point of a tangent cluster within `[lo, hi]`: where the curve runs along
/// the level set, `∇g · c' = 0`, found by bisection (exact to rounding, where the
/// minimum of `|g|` itself is too flat to place). Falls back to the nearest of `best`
/// and the bracket's ends (a touch at the end of the curve).
fn field_tangent(c: &BSplineCurve3, field: Field<'_>, lo: f64, hi: f64, best: f64) -> f64 {
    let g_at = |t: f64| field(c.point_at(t)).0.abs();
    let h = |t: f64| {
        let d = c.d1(t);
        field(c.point_at(t)).1.dot(d) / d.length().max(f64::MIN_POSITIVE)
    };
    let mut pick = best;
    for t in [lo, hi] {
        if g_at(t) < g_at(pick) {
            pick = t;
        }
    }
    let (mut a, mut b) = (lo, hi);
    let (ha, hb) = (h(a), h(b));
    if !(ha.is_finite() && hb.is_finite()) || (ha > 0.0) == (hb > 0.0) {
        return pick;
    }
    for _ in 0..BISECTION_STEPS {
        let m = 0.5 * (a + b);
        if !(m > a && m < b) {
            break;
        }
        if (h(m) > 0.0) == (ha > 0.0) {
            a = m;
        } else {
            b = m;
        }
    }
    let t = 0.5 * (a + b);
    if g_at(t) <= LINEAR_TOLERANCE { t } else { pick }
}

/// The farthest control point of a piece from the chord between its ends (the piece
/// lies in the hull of its control points, so within this of the chord).
fn flatness3(piece: &BSplineCurve3) -> f64 {
    let pts = piece.control_points();
    let (Some(&p0), Some(&p1)) = (pts.first(), pts.last()) else {
        return 0.0;
    };
    let d = p1 - p0;
    let len = d.length();
    pts.iter()
        .map(|q| {
            let r = *q - p0;
            if len > 0.0 {
                r.cross(d).length() / len
            } else {
                r.length()
            }
        })
        .fold(0.0, f64::max)
}

/// The farthest control point of a plane piece from its chord.
fn flatness2(piece: &BSplineCurve2) -> f64 {
    let pts = piece.control_points();
    let (Some(&p0), Some(&p1)) = (pts.first(), pts.last()) else {
        return 0.0;
    };
    let d = p1 - p0;
    let len = d.length();
    pts.iter()
        .map(|q| {
            let r = *q - p0;
            if len > 0.0 {
                r.cross(d).abs() / len
            } else {
                r.length()
            }
        })
        .fold(0.0, f64::max)
}

/// Samples of an interval when checking that `g` is monotone along it, or that two
/// pieces cross transversally.
const CONE_SAMPLES: usize = 6;

/// True when `∇g · c'` keeps one sign on samples of `[a, b]`: `g` is monotone there, so
/// it has at most one root (the field's analogue of disjoint tangent cones, Sederberg and
/// Meyers, "Loop detection in surface patch intersections", 1988).
fn field_monotone(c: &BSplineCurve3, field: Field<'_>, a: f64, b: f64) -> bool {
    let mut sign = 0.0;
    for i in 0..=CONE_SAMPLES {
        let t = a + (b - a) * i as f64 / CONE_SAMPLES as f64;
        let d = c.d1(t);
        let len = d.length();
        if len.is_nan() || len <= 0.0 {
            return false;
        }
        let h = field(c.point_at(t)).1.dot(d) / len;
        if h.is_nan() || h.abs() <= 1e-9 || (sign != 0.0 && h.signum() != sign) {
            return false;
        }
        sign = h.signum();
    }
    true
}

/// Newton's method for `g(c(t)) = 0` from the middle of `[a, b]`: the root if it
/// converges within the interval or one interval width beside it.
fn field_newton(c: &BSplineCurve3, field: Field<'_>, a: f64, b: f64) -> Option<f64> {
    let (lo, hi) = c.domain();
    let w = b - a;
    let mut t = 0.5 * (a + b);
    for _ in 0..NEWTON_STEPS {
        let (g, n) = field(c.point_at(t));
        if !g.is_finite() {
            return None;
        }
        if g.abs() <= NEWTON_RESIDUAL {
            return (t >= a - w && t <= b + w).then_some(t);
        }
        let d = n.dot(c.d1(t));
        if d.is_nan() || d.abs() <= f64::MIN_POSITIVE {
            return None;
        }
        let next = (t - g / d).clamp(lo, hi);
        if !next.is_finite() {
            return None;
        }
        t = next;
    }
    None
}

/// The sample of `[a, b]` nearest the zero set, if it is within the tolerance.
fn field_touch(c: &BSplineCurve3, field: Field<'_>, a: f64, b: f64) -> Option<f64> {
    let mut best: Option<(f64, f64)> = None;
    for i in 0..=TOUCH_SAMPLES {
        let t = a + (b - a) * i as f64 / TOUCH_SAMPLES as f64;
        let g = field(c.point_at(t)).0.abs();
        if g <= LINEAR_TOLERANCE && best.is_none_or(|x| g < x.0) {
            best = Some((g, t));
        }
    }
    best.map(|x| x.1)
}

/// A plane curve for [`pair_roots`]: a B-spline in the plane, or a kernel curve seen in
/// the plane of a frame (its points projected along the frame's z, an affine map).
#[derive(Debug, Clone)]
pub(crate) enum Track<'a> {
    Nurbs(BSplineCurve2),
    Projected { curve: &'a Curve, frame: &'a Frame },
}

impl Track<'_> {
    fn point(&self, t: f64) -> Point2 {
        match self {
            Track::Nurbs(c) => c.point_at(t),
            Track::Projected { curve, frame } => frame.to_plane(curve.point_at(t)),
        }
    }
    fn d1(&self, t: f64) -> Vec2 {
        match self {
            Track::Nurbs(c) => c.d1(t),
            Track::Projected { curve, frame } => {
                let v = frame.vector_to_local(curve.d1(t));
                Vec2::new(v.x, v.y)
            }
        }
    }
    /// The track as a piece over `[a, b]` (a B-spline is cut out, so its box is tight).
    fn piece(&self, a: f64, b: f64) -> KernelResult<Self> {
        Ok(match self {
            Track::Nurbs(c) => Track::Nurbs(c.segment(a, b)?),
            Track::Projected { curve, frame } => Track::Projected { curve, frame },
        })
    }
    /// How far the piece over `[a, b]` strays from its chord: exact for a B-spline (its
    /// control points) and a line; sampled, with a margin, for a projected conic.
    fn flatness(&self, a: f64, b: f64) -> f64 {
        match self {
            Track::Nurbs(c) => flatness2(c),
            Track::Projected { curve, .. } => {
                if matches!(curve, Curve::Line(_) | Curve::Segment(_)) {
                    return 0.0;
                }
                let (p0, p1) = (self.point(a), self.point(b));
                let d = p1 - p0;
                let len = d.length();
                let mut worst: f64 = 0.0;
                for i in 1..CONE_SAMPLES {
                    let q = self.point(a + (b - a) * i as f64 / CONE_SAMPLES as f64) - p0;
                    worst = worst.max(if len > 0.0 {
                        q.cross(d).abs() / len
                    } else {
                        q.length()
                    });
                }
                2.0 * worst
            }
        }
    }
    /// A box around the piece's points on `[a, b]`.
    fn bbox(&self, a: f64, b: f64) -> KernelResult<(Point2, Point2)> {
        match self {
            Track::Nurbs(c) => Ok(c.hull_box()),
            Track::Projected { curve, frame } => {
                let bb = curve.bounding_box_on(a, b)?;
                let mut lo = Point2::new(f64::INFINITY, f64::INFINITY);
                let mut hi = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                for i in 0..8 {
                    let pick = |bit: usize, x: f64, y: f64| if i & bit == 0 { x } else { y };
                    let q = frame.to_plane(Point3::new(
                        pick(1, bb.min.x, bb.max.x),
                        pick(2, bb.min.y, bb.max.y),
                        pick(4, bb.min.z, bb.max.z),
                    ));
                    lo = Point2::new(lo.x.min(q.x), lo.y.min(q.y));
                    hi = Point2::new(hi.x.max(q.x), hi.y.max(q.y));
                }
                Ok((lo, hi))
            }
        }
    }
}

/// A common point of two plane curves: parameters on each, and whether they touch there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PairRoot {
    pub s: f64,
    pub t: f64,
    pub tangent: bool,
}

fn apart(a: (Point2, Point2), b: (Point2, Point2)) -> bool {
    let t = LINEAR_TOLERANCE;
    a.1.x + t < b.0.x || b.1.x + t < a.0.x || a.1.y + t < b.0.y || b.1.y + t < a.0.y
}

/// The common points of `a` over `ar` and `b` over `br` (see the module documentation),
/// sorted by `s`. Both ranges must be finite and inside the curves' ranges.
pub(crate) fn pair_roots(
    a: &Track<'_>,
    ar: (f64, f64),
    b: &Track<'_>,
    br: (f64, f64),
) -> KernelResult<Vec<PairRoot>> {
    if !(ar.0 < ar.1 && br.0 < br.1 && ar.1.is_finite() && br.1.is_finite()) {
        return Ok(Vec::new());
    }
    let pa = a.piece(ar.0, ar.1)?;
    let pb = b.piece(br.0, br.1)?;
    let (ba, bb) = (pa.bbox(ar.0, ar.1)?, pb.bbox(br.0, br.1)?);
    let scale = (ba.0.distance(ba.1) + bb.0.distance(bb.1)).max(LINEAR_TOLERANCE);
    let small = (SMALL_FRACTION * scale).max(LINEAR_TOLERANCE);
    let mut stack = vec![(pa, ar, pb, br, 0usize)];
    let mut found: Vec<(f64, f64, bool)> = Vec::new();
    let mut visits = 0usize;
    while let Some((pa, (a0, a1), pb, (b0, b1), depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_VISITS {
            return Err(too_much("two curves"));
        }
        let (ba, bb) = (pa.bbox(a0, a1)?, pb.bbox(b0, b1)?);
        if apart(ba, bb) {
            continue;
        }
        let (sa, sb) = (ba.0.distance(ba.1), bb.0.distance(bb.1));
        let (ma, mb) = (0.5 * (a0 + a1), 0.5 * (b0 + b1));
        let a_splits = ma > a0 && ma < a1;
        let b_splits = mb > b0 && mb < b1;
        // A leaf pair meets at most once: small with disjoint tangent cones (sampled), or
        // both pieces flat within the tolerance (straight, so once or touching).
        let transversal = sa.max(sb) <= small && cones_apart(a, b, (a0, a1), (b0, b1));
        let flat =
            pa.flatness(a0, a1) <= LINEAR_TOLERANCE && pb.flatness(b0, b1) <= LINEAR_TOLERANCE;
        if transversal || flat || depth >= MAX_DEPTH || !(a_splits || b_splits) {
            if let Some(r) = pair_newton(a, b, (a0, a1), (b0, b1)) {
                found.push((r.0, r.1, false));
            } else if let Some(r) = pair_touch(a, b, (a0, a1), (b0, b1)) {
                found.push((r.0, r.1, true));
            }
            continue;
        }
        if (sa >= sb && a_splits) || !b_splits {
            let (l, r) = (pa.piece(a0, ma)?, pa.piece(ma, a1)?);
            stack.push((r, (ma, a1), pb.clone(), (b0, b1), depth + 1));
            stack.push((l, (a0, ma), pb, (b0, b1), depth + 1));
        } else {
            let (l, r) = (pb.piece(b0, mb)?, pb.piece(mb, b1)?);
            stack.push((pa.clone(), (a0, a1), r, (mb, b1), depth + 1));
            stack.push((pa, (a0, a1), l, (b0, mb), depth + 1));
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let gap = |s: f64, t: f64| a.point(s).distance(b.point(t));
    // Clusters: (first, last, best, any touch, t range).
    type Cluster = ((f64, f64), (f64, f64), (f64, f64), bool, (f64, f64));
    let mut out: Vec<Cluster> = Vec::new();
    for (s, t, touch) in found {
        let r = (s, t);
        match out.last_mut() {
            Some(last)
                if a.point(last.1.0).distance(a.point(s)) <= LINEAR_TOLERANCE
                    || gap(0.5 * (last.1.0 + s), 0.5 * (last.1.1 + t)) <= LINEAR_TOLERANCE =>
            {
                last.1 = r;
                last.3 |= touch;
                last.4 = (last.4.0.min(t), last.4.1.max(t));
                if gap(s, t) < gap(last.2.0, last.2.1) {
                    last.2 = r;
                }
            }
            _ => out.push((r, r, r, touch, (t, t))),
        }
    }
    Ok(out
        .into_iter()
        .map(|(first, last, best, touch, (tlo, thi))| {
            let (da, db) = (a.d1(best.0), b.d1(best.1));
            let spread = a.point(first.0).distance(a.point(last.0)) > LINEAR_TOLERANCE;
            let tangent = touch
                || spread
                || da.cross(db).abs() <= ANGULAR_TOLERANCE * da.length() * db.length();
            let (s, t) = if tangent {
                let ws = (last.0 - first.0) + SMALL_FRACTION * (ar.1 - ar.0);
                let wt = (thi - tlo) + SMALL_FRACTION * (br.1 - br.0);
                pair_tangent(
                    a,
                    b,
                    ((first.0 - ws).max(ar.0), (last.0 + ws).min(ar.1)),
                    ((tlo - wt).max(br.0), (thi + wt).min(br.1)),
                    best,
                )
            } else {
                best
            };
            PairRoot { s, t, tangent }
        })
        .collect())
}

/// The ranges `a` and `b` may be evaluated on: their whole domains (a projected curve's
/// own range, or the requested one for an infinite line).
fn domain(track: &Track<'_>, own: (f64, f64)) -> (f64, f64) {
    match track {
        Track::Nurbs(c) => c.domain(),
        Track::Projected { curve, .. } => {
            let (lo, hi) = curve.range();
            if curve.is_periodic() {
                (f64::NEG_INFINITY, f64::INFINITY)
            } else {
                (
                    lo.max(own.0 - (own.1 - own.0)),
                    hi.min(own.1 + (own.1 - own.0)),
                )
            }
        }
    }
}

/// True when no sampled tangent of `a` over `ar` is parallel to one of `b` over `br`:
/// the tangent cones are apart, so the pieces cross at most once (Sederberg and Meyers,
/// 1988).
fn cones_apart(a: &Track<'_>, b: &Track<'_>, ar: (f64, f64), br: (f64, f64)) -> bool {
    let dirs = |t: &Track<'_>, (lo, hi): (f64, f64)| -> Option<Vec<Vec2>> {
        (0..=CONE_SAMPLES)
            .map(|i| {
                t.d1(lo + (hi - lo) * i as f64 / CONE_SAMPLES as f64)
                    .normalized()
            })
            .collect()
    };
    let (Some(da), Some(db)) = (dirs(a, ar), dirs(b, br)) else {
        return false;
    };
    let mut sign = 0.0;
    for x in &da {
        for y in &db {
            let c = x.cross(*y);
            if c.is_nan() || c.abs() <= 1e-9 || (sign != 0.0 && c.signum() != sign) {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}

/// Newton's method for `a(s) = b(t)` from the middle of the two intervals.
fn pair_newton(
    a: &Track<'_>,
    b: &Track<'_>,
    (a0, a1): (f64, f64),
    (b0, b1): (f64, f64),
) -> Option<(f64, f64)> {
    let (da_lo, da_hi) = domain(a, (a0, a1));
    let (db_lo, db_hi) = domain(b, (b0, b1));
    let (wa, wb) = (a1 - a0, b1 - b0);
    let (mut s, mut t) = (0.5 * (a0 + a1), 0.5 * (b0 + b1));
    for _ in 0..NEWTON_STEPS {
        let f = a.point(s) - b.point(t);
        if !(f.x.is_finite() && f.y.is_finite()) {
            return None;
        }
        if f.length() <= NEWTON_RESIDUAL {
            let inside = s >= a0 - wa && s <= a1 + wa && t >= b0 - wb && t <= b1 + wb;
            return inside.then_some((s, t));
        }
        let (da, db) = (a.d1(s), b.d1(t));
        // [da, -db] (ds, dt) = -f.
        let det = db.x * da.y - da.x * db.y;
        if det.is_nan() || det.abs() <= 1e-300 || det.abs() <= 1e-14 * da.length() * db.length() {
            return None;
        }
        let (rx, ry) = (-f.x, -f.y);
        let ds = (rx * (-db.y) + db.x * ry) / det;
        let dt = (da.x * ry - da.y * rx) / det;
        let (ns, nt) = ((s + ds).clamp(da_lo, da_hi), (t + dt).clamp(db_lo, db_hi));
        if !(ns.is_finite() && nt.is_finite()) {
            return None;
        }
        s = ns;
        t = nt;
    }
    None
}

/// The parameter on `b` within `[b0, b1]` of the point nearest to `p`, by Gauss-Newton
/// from the middle.
fn foot(b: &Track<'_>, p: Point2, (b0, b1): (f64, f64)) -> f64 {
    let mut t = 0.5 * (b0 + b1);
    for _ in 0..NEWTON_STEPS {
        let d = b.d1(t);
        let dd = d.dot(d);
        if dd.is_nan() || dd <= f64::MIN_POSITIVE {
            break;
        }
        let next = (t - (b.point(t) - p).dot(d) / dd).clamp(b0, b1);
        if !next.is_finite() || (next - t).abs() <= 1e-15 * (b1 - b0).abs().max(1.0) {
            t = if next.is_finite() { next } else { t };
            break;
        }
        t = next;
    }
    t
}

/// The touching point of a tangent cluster: where the tangents are parallel,
/// `a'(s) × b'(t(s)) = 0` with `t(s)` the foot of `a(s)` on `b` within `tr`, found by
/// bisection over `sr`; else the nearest of `best` and the bracket's ends.
fn pair_tangent(
    a: &Track<'_>,
    b: &Track<'_>,
    sr: (f64, f64),
    tr: (f64, f64),
    best: (f64, f64),
) -> (f64, f64) {
    let at = |s: f64| {
        let t = foot(b, a.point(s), tr);
        (t, a.point(s).distance(b.point(t)))
    };
    let h = |s: f64| {
        let (t, _) = at(s);
        let (da, db) = (a.d1(s), b.d1(t));
        da.cross(db) / (da.length() * db.length()).max(f64::MIN_POSITIVE)
    };
    let mut pick = (best, a.point(best.0).distance(b.point(best.1)));
    for s in [sr.0, sr.1] {
        let (t, d) = at(s);
        if d < pick.1 {
            pick = ((s, t), d);
        }
    }
    let (mut lo, mut hi) = sr;
    let (hl, hh) = (h(lo), h(hi));
    if !(hl.is_finite() && hh.is_finite()) || (hl > 0.0) == (hh > 0.0) {
        return pick.0;
    }
    for _ in 0..BISECTION_STEPS {
        let m = 0.5 * (lo + hi);
        if !(m > lo && m < hi) {
            break;
        }
        if (h(m) > 0.0) == (hl > 0.0) {
            lo = m;
        } else {
            hi = m;
        }
    }
    let s = 0.5 * (lo + hi);
    let (t, d) = at(s);
    if d <= LINEAR_TOLERANCE {
        (s, t)
    } else {
        pick.0
    }
}

/// The sample pair of the two intervals nearest to each other, if within the tolerance.
fn pair_touch(
    a: &Track<'_>,
    b: &Track<'_>,
    (a0, a1): (f64, f64),
    (b0, b1): (f64, f64),
) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64, f64)> = None;
    for i in 0..=TOUCH_SAMPLES {
        let s = a0 + (a1 - a0) * i as f64 / TOUCH_SAMPLES as f64;
        let p = a.point(s);
        let t = foot(b, p, (b0, b1));
        let d = p.distance(b.point(t));
        if d <= LINEAR_TOLERANCE && best.is_none_or(|x| d < x.0) {
            best = Some((d, s, t));
        }
    }
    best.map(|x| (x.1, x.2))
}

/// True when every one of `n + 1` evenly spaced samples of `[t0, t1]` passes `on`.
pub(crate) fn all_samples(t0: f64, t1: f64, n: usize, on: impl Fn(f64) -> bool) -> bool {
    (0..=n).all(|i| on(t0 + (t1 - t0) * i as f64 / n as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    /// The unit circle as a rational quadratic (The NURBS Book, §7.3, nine points).
    fn circle(c: Point2, r: f64) -> BSplineCurve2 {
        let w = FRAC_1_SQRT_2;
        let corners = [
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (1.0, 0.0),
        ];
        BSplineCurve2::new(
            2,
            vec![
                0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
            ],
            corners
                .iter()
                .map(|(x, y)| Point2::new(c.x + r * x, c.y + r * y))
                .collect(),
            Some(vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]),
        )
        .unwrap()
    }

    #[test]
    fn a_line_field_meets_a_circle_at_its_two_chord_ends() {
        // The circle r = 2 about the origin against x = 1: the points (1, ±√3).
        let c = circle(Point2::ORIGIN, 2.0).to_3d(&Frame::XY).unwrap();
        let f = |p: Point3| (p.x - 1.0, Vec3::X);
        let (lo, hi) = c.domain();
        let r = field_roots(&c, lo, hi, &f, f64::INFINITY).unwrap();
        assert_eq!(r.len(), 2, "{r:?}");
        let mut ys: Vec<f64> = r.iter().map(|x| c.point_at(x.t).y).collect();
        ys.sort_by(f64::total_cmp);
        assert!((ys[0] + 3f64.sqrt()).abs() < 1e-9 && (ys[1] - 3f64.sqrt()).abs() < 1e-9);
        assert!(r.iter().all(|x| !x.tangent));
        // x = 2 touches it once, at (2, 0); x = 2.001 misses.
        let f = |p: Point3| (p.x - 2.0, Vec3::X);
        let r = field_roots(&c, lo, hi, &f, f64::INFINITY).unwrap();
        assert!(!r.is_empty() && r.iter().all(|x| x.tangent), "{r:?}");
        // The touching point is placed where the tangent runs along the line, not anywhere
        // in the band |x - 2| <= 1e-7 (where the circle runs |y| <= √(4 · 1e-7) ≈ 6e-4).
        assert!(
            r.iter()
                .all(|x| c.point_at(x.t).distance(Point3::new(2.0, 0.0, 0.0)) < 1e-9)
        );
        let f = |p: Point3| (p.x - 2.001, Vec3::X);
        assert!(
            field_roots(&c, lo, hi, &f, f64::INFINITY)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn two_circles_cross_where_the_radical_line_meets_them() {
        // Circles r = 2 about (0, 0) and (2, 0): x = 1, y = ±√3.
        let a = Track::Nurbs(circle(Point2::ORIGIN, 2.0));
        let b = Track::Nurbs(circle(Point2::new(2.0, 0.0), 2.0));
        let r = pair_roots(&a, (0.0, 1.0), &b, (0.0, 1.0)).unwrap();
        assert_eq!(r.len(), 2, "{r:?}");
        for x in &r {
            let p = a.point(x.s);
            assert!((p.x - 1.0).abs() < 1e-9 && (p.y.abs() - 3f64.sqrt()).abs() < 1e-9);
            assert!(p.distance(b.point(x.t)) < 1e-9);
            assert!(!x.tangent);
        }
        // Circles r = 1 about (0, 0) and (2, 0) touch at (1, 0).
        let a = Track::Nurbs(circle(Point2::ORIGIN, 1.0));
        let b = Track::Nurbs(circle(Point2::new(2.0, 0.0), 1.0));
        let r = pair_roots(&a, (0.0, 1.0), &b, (0.0, 1.0)).unwrap();
        assert!(!r.is_empty() && r.iter().all(|x| x.tangent), "{r:?}");
        // Apart: none.
        let b = Track::Nurbs(circle(Point2::new(2.1, 0.0), 1.0));
        assert!(
            pair_roots(&a, (0.0, 1.0), &b, (0.0, 1.0))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_curve_against_itself_is_refused_not_looped_on() {
        let a = Track::Nurbs(circle(Point2::ORIGIN, 1.0));
        let r = pair_roots(&a, (0.0, 1.0), &a, (0.0, 1.0));
        // Either refused, or one tangent cluster: never a hang or a crash.
        if let Ok(r) = r {
            assert!(r.iter().all(|x| x.tangent));
        }
    }
}

//! Lines and segments against free-form (NURBS) surfaces, for point classification (rays)
//! and picking.
//!
//! # Method
//! Subdivision with convex-hull boxes, then Newton (the approach of Nishita, Sederberg
//! and Kakimoto, "Ray tracing trimmed rational surface patches", SIGGRAPH 1990, with
//! boxes in place of their Bezier clipping; Patrikalakis and Maekawa, section 5.7):
//! the surface is cut into its Bezier patches (one per pair of knot spans, by knot
//! insertion), and each patch whose control-point box (which holds it, with positive
//! weights) the line passes within [`LINEAR_TOLERANCE`] of is quartered, recursively,
//! down to [`LEAF_DEPTH`] levels. Each leaf the line still meets seeds Newton's method on
//! `S(u, v) - L(t) = 0` (three equations in `u`, `v`, `t`, Jacobian `[S_u, S_v, -L']`).
//! Roots in the domain and the line's range are kept and merged when they coincide. A
//! root where the line is tangent to the surface (`|L' · N| <= TANGENT_COSINE |L'|`), or
//! a leaf where Newton fails but the line passes within [`LINEAR_TOLERANCE`] of the
//! surface, is a [`Contact::Tangent`]: the classification then casts another ray.
//!
//! Work is bounded: at most [`MAX_CELLS`] boxes are tested per call, beyond which the
//! intersection fails with [`KernelError::NoConvergence`] instead of guessing.
//!
//! Other curves (circles, ellipses, splines) against free-form surfaces are not supported
//! yet: they arrive with surface-surface marching.

use super::curve::{CurveSurfaceIntersection, CurveSurfacePoint};
use crate::nurbs::NurbsSurface;
use crate::{Contact, Curve, KernelError, KernelResult};
use chamfera_geom::{BSplineSurface3, LINEAR_TOLERANCE, MAX_COORDINATE, Point3, Vec3};

/// Levels of quartering below each Bezier patch.
const LEAF_DEPTH: usize = 5;
/// Most boxes one intersection tests (hostile-input limit).
const MAX_CELLS: usize = 200_000;
/// Newton steps from one leaf.
const NEWTON_STEPS: usize = 40;
/// Below this cosine between the line and the surface's tangent plane the contact is
/// tangent.
const TANGENT_COSINE: f64 = 1e-9;

/// The part `[lo, hi]` of `[t0, t1]` where `origin + t dir` is inside the box grown by
/// `grow`, if any.
fn clip(
    origin: Point3,
    dir: Vec3,
    t0: f64,
    t1: f64,
    min: Point3,
    max: Point3,
    grow: f64,
) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (t0, t1);
    for (o, d, a, b) in [
        (origin.x, dir.x, min.x, max.x),
        (origin.y, dir.y, min.y, max.y),
        (origin.z, dir.z, min.z, max.z),
    ] {
        let (a, b) = (a - grow, b + grow);
        if d == 0.0 {
            if o < a || o > b {
                return None;
            }
            continue;
        }
        let (x, y) = ((a - o) / d, (b - o) / d);
        lo = lo.max(x.min(y));
        hi = hi.min(x.max(y));
    }
    (lo.is_finite() && hi.is_finite() && lo <= hi).then_some((lo, hi))
}

/// Solves the 3x3 system with columns `a`, `b`, `c` for `r` by Cramer's rule.
fn solve(a: Vec3, b: Vec3, c: Vec3, r: Vec3) -> Option<(f64, f64, f64)> {
    let det = a.dot(b.cross(c));
    let scale = a.length() * b.length() * c.length();
    if !(det.is_finite() && det.abs() > 1e-14 * scale && scale > 0.0) {
        return None;
    }
    Some((
        r.dot(b.cross(c)) / det,
        a.dot(r.cross(c)) / det,
        a.dot(b.cross(r)) / det,
    ))
}

/// Where the line or segment `curve` over `[t0, t1]` meets the free-form surface `n`.
pub(crate) fn curve_nurbs(
    curve: &Curve,
    t0: f64,
    t1: f64,
    n: &NurbsSurface,
) -> KernelResult<CurveSurfaceIntersection> {
    let (origin, dir) = match curve {
        Curve::Line(l) => (l.origin(), l.direction()),
        Curve::Segment(s) => (s.start(), s.end() - s.start()),
        _ => {
            return Err(KernelError::Unsupported(
                "intersecting a circle, ellipse or spline with a free-form surface (K10: needs marching)".into(),
            ));
        }
    };
    if t0.is_nan() || t1.is_nan() || t0 > t1 || t0 == f64::INFINITY || t1 == f64::NEG_INFINITY {
        return Err(KernelError::InvalidInput(
            "the curve range `t0..t1` must be ordered, with t0 <= t1".into(),
        ));
    }
    let speed = dir.length();
    if !(speed.is_finite() && speed > 0.0) {
        return Err(KernelError::InvalidInput(
            "the line has no direction".into(),
        ));
    }
    let world = Point3::new(MAX_COORDINATE, MAX_COORDINATE, MAX_COORDINATE);
    let Some((t0, t1)) = clip(
        origin,
        dir,
        t0,
        t1,
        Point3::new(-MAX_COORDINATE, -MAX_COORDINATE, -MAX_COORDINATE),
        world,
        0.0,
    ) else {
        return Ok(CurveSurfaceIntersection::Points(vec![]));
    };
    let whole = n.bounding_box();
    let Some((lo, hi)) = clip(origin, dir, t0, t1, whole.min, whole.max, LINEAR_TOLERANCE) else {
        return Ok(CurveSurfaceIntersection::Points(vec![]));
    };
    let at = |t: f64| origin + dir * t;

    // Leaves the line meets: (u, v, t) seeds.
    let mut seeds: Vec<(f64, f64, f64, [f64; 4])> = Vec::new();
    let mut cells = 0usize;
    for (patch, range) in n.bezier_patches()? {
        let mut stack: Vec<(BSplineSurface3, [f64; 4], usize)> = vec![(patch.clone(), *range, 0)];
        while let Some((p, r, depth)) = stack.pop() {
            cells += 1;
            if cells > MAX_CELLS {
                return Err(KernelError::NoConvergence(
                    "a line against a free-form surface needs more subdivision than allowed".into(),
                ));
            }
            let (bmin, bmax) = p.hull_box();
            let Some((a, b)) = clip(origin, dir, lo, hi, bmin, bmax, LINEAR_TOLERANCE) else {
                continue;
            };
            let [u0, u1, v0, v1] = r;
            if depth >= LEAF_DEPTH {
                seeds.push((0.5 * (u0 + u1), 0.5 * (v0 + v1), 0.5 * (a + b), r));
                continue;
            }
            let (um, vm) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
            for (a0, a1, b0, b1) in [
                (u0, um, v0, vm),
                (um, u1, v0, vm),
                (u0, um, vm, v1),
                (um, u1, vm, v1),
            ] {
                let q = p.sub_patch(a0, a1, b0, b1)?;
                stack.push((q, [a0, a1, b0, b1], depth + 1));
            }
        }
    }

    let (ul, uh) = n.domain_u();
    let (vl, vh) = n.domain_v();
    let eps_t = LINEAR_TOLERANCE / speed;
    let mut hits: Vec<CurveSurfacePoint> = Vec::new();
    for (u, v, t, r) in seeds {
        let hit = newton(n, origin, dir, u, v, t).or_else(|| near_miss(n, origin, dir, r, t));
        let Some((u, v, t, tangent)) = hit else {
            continue;
        };
        let inside = (n.period_u().is_some() || (ul..=uh).contains(&u))
            && (n.period_v().is_some() || (vl..=vh).contains(&v));
        if !inside || t < t0 - eps_t || t > t1 + eps_t {
            continue;
        }
        let normal = n.normal_at(u, v);
        let tangent = tangent || (dir.dot(normal) / speed).abs() <= TANGENT_COSINE;
        hits.push(CurveSurfacePoint {
            t: t.clamp(t0, t1),
            point: at(t.clamp(t0, t1)),
            contact: if tangent {
                Contact::Tangent
            } else {
                Contact::Crossing
            },
        });
    }
    hits.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut out: Vec<CurveSurfacePoint> = Vec::with_capacity(hits.len());
    for h in hits {
        match out.last_mut() {
            Some(last) if last.point.distance(h.point) <= LINEAR_TOLERANCE => {
                if h.contact == Contact::Tangent {
                    last.contact = Contact::Tangent;
                }
            }
            _ => out.push(h),
        }
    }
    Ok(CurveSurfaceIntersection::Points(out))
}

/// Newton's method on `S(u, v) - (origin + t dir) = 0` from `(u, v, t)`: the root and
/// whether the line is tangent there (never, here), or `None`.
fn newton(
    n: &NurbsSurface,
    origin: Point3,
    dir: Vec3,
    u: f64,
    v: f64,
    t: f64,
) -> Option<(f64, f64, f64, bool)> {
    let (ul, uh) = n.domain_u();
    let (vl, vh) = n.domain_v();
    let (mut u, mut v, mut t) = (u, v, t);
    let tol = LINEAR_TOLERANCE * 1e-3;
    for _ in 0..NEWTON_STEPS {
        let g = n.point_at(u, v) - (origin + dir * t);
        if g.length() <= tol {
            return Some((u, v, t, false));
        }
        let (su, sv) = n.derivatives(u, v);
        let (du, dv, dt) = solve(su, sv, -dir, -g)?;
        u += du;
        v += dv;
        t += dt;
        if n.period_u().is_none() {
            u = u.clamp(ul, uh);
        }
        if n.period_v().is_none() {
            v = v.clamp(vl, vh);
        }
        if !(u.is_finite() && v.is_finite() && t.is_finite()) {
            return None;
        }
    }
    let g = n.point_at(u, v) - (origin + dir * t);
    (g.length() <= LINEAR_TOLERANCE).then_some((u, v, t, false))
}

/// Where Newton failed in a leaf: the line's closest approach to the surface from the
/// leaf, by damped Gauss-Newton (Levenberg-Marquardt) on `|S(u, v) - L(t)|²`, which
/// converges where the line only touches (Newton's Jacobian is singular there); a
/// tangent contact when within [`LINEAR_TOLERANCE`].
fn near_miss(
    n: &NurbsSurface,
    origin: Point3,
    dir: Vec3,
    r: [f64; 4],
    t: f64,
) -> Option<(f64, f64, f64, bool)> {
    let [u0, u1, v0, v1] = r;
    let (ul, uh) = n.domain_u();
    let (vl, vh) = n.domain_v();
    let (mut u, mut v, mut t) = (0.5 * (u0 + u1), 0.5 * (v0 + v1), t);
    let residual = |u: f64, v: f64, t: f64| n.point_at(u, v) - (origin + dir * t);
    let mut g = residual(u, v, t);
    let mut lambda = 1e-3;
    for _ in 0..NEWTON_STEPS {
        let (su, sv) = n.derivatives(u, v);
        let cols = [su, sv, -dir];
        let mut a = [[0.0f64; 3]; 3];
        for (i, ci) in cols.iter().enumerate() {
            for (j, cj) in cols.iter().enumerate() {
                a[i][j] = ci.dot(*cj);
            }
        }
        let b = Vec3::new(-cols[0].dot(g), -cols[1].dot(g), -cols[2].dot(g));
        let mut improved = false;
        for _ in 0..8 {
            let m = |i: usize| a[i][i] * (1.0 + lambda) + f64::MIN_POSITIVE;
            let c0 = Vec3::new(m(0), a[1][0], a[2][0]);
            let c1 = Vec3::new(a[0][1], m(1), a[2][1]);
            let c2 = Vec3::new(a[0][2], a[1][2], m(2));
            let Some((du, dv, dt)) = solve(c0, c1, c2, b) else {
                lambda *= 10.0;
                continue;
            };
            let mut un = u + du;
            let mut vn = v + dv;
            if n.period_u().is_none() {
                un = un.clamp(ul, uh);
            }
            if n.period_v().is_none() {
                vn = vn.clamp(vl, vh);
            }
            let tn = t + dt;
            let gn = residual(un, vn, tn);
            if gn.length() < g.length() {
                (u, v, t, g) = (un, vn, tn, gn);
                lambda = (lambda * 0.3).max(1e-12);
                improved = true;
                break;
            }
            lambda *= 10.0;
        }
        if !improved || g.length() <= LINEAR_TOLERANCE * 1e-3 {
            break;
        }
    }
    (g.length() <= LINEAR_TOLERANCE).then_some((u, v, t, true))
}

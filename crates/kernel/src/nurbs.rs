//! Free-form faces (rung K10, surfaces): a rational B-spline surface
//! ([`chamfera_geom::BSplineSurface3`]) as a kernel [`Surface`].
//!
//! # Parameterisation
//! `S(u, v)` is the surface's own: `u` and `v` run over its knot domains and the normal
//! ([`Surface::normal_at`]) is the direction of `S_u × S_v`. Two conventions make it fit
//! the parameter-plane machinery the analytic surfaces use (`topology/uv.rs`,
//! `tessellate`):
//! - **Closed directions are periodic.** When the boundary curves `S(u_lo, v)` and
//!   `S(u_hi, v)` coincide (a full revolution, a tube), `u` is periodic with the domain's
//!   width as period: evaluation wraps `u` into the domain, projection returns `u` in
//!   `[u_lo, u_hi)`, and a seam edge's two uses unwrap a period apart. Likewise `v`.
//! - **Collapsed boundaries are poles in `v`.** A boundary curve that collapses to a
//!   point (the pole of a surface of revolution, the tip of a triangular patch) is a
//!   singular line: on it `u` is undefined, which the loop images and tessellation treat
//!   as they treat a sphere's poles. A surface whose collapsed boundary is a `u` boundary
//!   is turned into one whose collapsed boundary is a `v` boundary when it is made:
//!   `S'(s, t) = S(u_lo + u_hi - t, s)` (parameters swapped, the new `u` reversed), which
//!   keeps the normal (`S'_s × S'_t = S_u × S_v`). Surfaces with collapsed boundaries in
//!   both directions are refused.
//!
//! # Point inversion
//! [`NurbsSurface::parameters`] is Piegl and Tiller's point inversion (*The NURBS Book*,
//! §6.1): Newton's method on `f = (S - P) · S_u = 0`, `g = (S - P) · S_v = 0` (eq. 6.6-6.8,
//! with the second derivatives of eq. 4.20), started from the nearest of a grid of
//! surface samples (a few per knot span, at most [`MAX_SEEDS`]). The safeguards: each
//! step is shortened until the distance to `P` does not grow (falling back to the
//! scaled gradient step when the Newton direction does not descend), parameters are
//! clamped to the domain in open directions and wrapped in periodic ones (eq. 6.9, 6.10),
//! iteration stops on point coincidence, zero cosine or a vanishing step, and is bounded
//! by [`MAX_NEWTON_STEPS`]. Two further seeds are tried when the first ends farther from
//! the point than a thousandth of the surface's size. A point on a pole projects to the pole with `singular` set.
//!
//! # Boxes
//! With positive weights a surface lies in the convex hull of its control points
//! (§4.4), so the box of a sub-patch's control points bounds that part of the surface
//! ([`NurbsSurface::bounding_box_on`]).

use crate::check;
use crate::{KernelError, KernelResult, SurfaceParameters, SurfacePoint};
use chamfera_geom::{
    Aabb, BSplineSurface3, Frame, LINEAR_TOLERANCE, Point3, SurfaceDerivatives, Transform, Vec3,
};
use std::sync::{Arc, OnceLock};

/// Most samples the inversion seed grid holds (64 x 64).
pub const MAX_SEEDS: usize = 4096;
/// Most Newton steps one point inversion takes from one seed.
pub const MAX_NEWTON_STEPS: usize = 64;
/// Seed samples per knot span in each direction (fewer when the grid would exceed
/// [`MAX_SEEDS`]).
const SEEDS_PER_SPAN: usize = 4;
/// Most intervals between seed samples along one direction (64 samples).
const MAX_SEEDS_PER_DIRECTION: usize = 63;
/// Samples per knot span when testing whether a boundary closes or collapses.
const CLOSURE_SAMPLES_PER_SPAN: usize = 8;
/// Most such samples along one boundary.
const MAX_CLOSURE_SAMPLES: usize = 1024;
/// Step halvings before a Newton step is given up.
const MAX_HALVINGS: usize = 40;
/// The zero-cosine test of §6.1 (eq. 6.8): `|S_u · (S - P)| / (|S_u| |S - P|)` below this
/// means `S - P` is perpendicular to the surface.
const ZERO_COSINE: f64 = 1e-12;
/// A point within this fraction of the surface's size of the point Newton finds from
/// the nearest seed is settled by it; farther points try two more seeds.
const NEAR_FRACTION: f64 = 1e-3;
/// Relative step (to the surface's size) at which Newton has converged: below rounding of
/// coordinates of that size.
const STEP_RELATIVE: f64 = 1e-15;
/// Gauss panels per knot span when integrating along `v` (`topology/freeform.rs`,
/// `mass.rs`): the integrands are rational (with a square root for areas) on each span,
/// analytic on it, so eight 5-point panels leave an error far below the measures'
/// tolerance for the spans of real surfaces.
pub(crate) const V_PANELS_PER_SPAN: usize = 8;
/// Most Bezier patches a surface is cut into for ray intersection.
pub(crate) const MAX_BEZIER_PATCHES: usize = 4096;

/// A free-form surface (see the module documentation). Cheap to clone: the surface and
/// its derived data are shared.
#[derive(Debug, Clone, PartialEq)]
pub struct NurbsSurface(Arc<Inner>);

#[derive(Debug, PartialEq)]
struct Inner {
    /// The world frame (the surface has no placement of its own).
    frame: Frame,
    surface: BSplineSurface3,
    closed_u: bool,
    closed_v: bool,
    /// The point the boundary `v = v_lo` collapses to, if it does.
    pole_lo: Option<Point3>,
    /// The point the boundary `v = v_hi` collapses to, if it does.
    pole_hi: Option<Point3>,
    /// `(u, v, S(u, v))` samples for the inversion's starting points.
    seeds: Vec<(f64, f64, Point3)>,
    /// The diagonal of the control-point box (at least 1 mm), the scale of relative
    /// tolerances.
    size: f64,
    /// The surface cut into Bezier patches with their parameter ranges, made on first use.
    bezier: OnceLock<Vec<BezierPatch>>,
}

/// One Bezier patch of a surface: the patch and its `(u0, u1, v0, v1)`.
pub(crate) type BezierPatch = (BSplineSurface3, [f64; 4]);

/// `n` + 1 parameters over `[a, b]` per span of `breaks`, or `cap` + 1 evenly spread over
/// the whole range when that would be more than `cap`.
fn samples(breaks: &[f64], per_span: usize, cap: usize) -> Vec<f64> {
    let spans = breaks.len().saturating_sub(1).max(1);
    let (lo, hi) = (
        breaks.first().copied().unwrap_or(0.0),
        breaks.last().copied().unwrap_or(1.0),
    );
    if spans.saturating_mul(per_span) > cap {
        return (0..=cap)
            .map(|i| lo + (hi - lo) * i as f64 / cap as f64)
            .collect();
    }
    let mut out = Vec::with_capacity(spans * per_span + 1);
    for w in breaks.windows(2) {
        for k in 0..per_span {
            out.push(w[0] + (w[1] - w[0]) * k as f64 / per_span as f64);
        }
    }
    out.push(hi);
    out
}

/// True when every point is within [`LINEAR_TOLERANCE`] of the first.
fn collapsed(points: &[Point3]) -> bool {
    points
        .first()
        .is_some_and(|a| points.iter().all(|p| p.distance(*a) <= LINEAR_TOLERANCE))
}

/// The boundary flags of a surface: `(u_lo collapses, u_hi collapses, v_lo collapses,
/// v_hi collapses, closed in u, closed in v)`.
fn boundaries(s: &BSplineSurface3) -> [bool; 6] {
    let (ul, uh) = s.domain_u();
    let (vl, vh) = s.domain_v();
    let us = samples(&s.breaks_u(), CLOSURE_SAMPLES_PER_SPAN, MAX_CLOSURE_SAMPLES);
    let vs = samples(&s.breaks_v(), CLOSURE_SAMPLES_PER_SPAN, MAX_CLOSURE_SAMPLES);
    let along_v = |u: f64| -> Vec<Point3> { vs.iter().map(|v| s.point_at(u, *v)).collect() };
    let along_u = |v: f64| -> Vec<Point3> { us.iter().map(|u| s.point_at(*u, v)).collect() };
    let (a, b) = (along_v(ul), along_v(uh));
    let (c, d) = (along_u(vl), along_u(vh));
    let same = |x: &[Point3], y: &[Point3]| {
        x.iter()
            .zip(y)
            .all(|(p, q)| p.distance(*q) <= LINEAR_TOLERANCE)
    };
    let (cu0, cu1, cv0, cv1) = (collapsed(&a), collapsed(&b), collapsed(&c), collapsed(&d));
    // A collapsed pair of boundaries trivially "coincide"; that is not closure.
    let closed_u = !(cu0 && cu1) && same(&a, &b);
    let closed_v = !(cv0 && cv1) && same(&c, &d);
    [cu0, cu1, cv0, cv1, closed_u, closed_v]
}

impl NurbsSurface {
    /// The kernel surface of `s` (see the module documentation for the conventions; a
    /// surface collapsed in `u` is reparameterised so its normal is kept). Refuses
    /// surfaces collapsed in both directions.
    pub fn new(s: BSplineSurface3) -> KernelResult<NurbsSurface> {
        let mut s = s;
        let mut flags = boundaries(&s);
        if (flags[0] || flags[1]) && !(flags[2] || flags[3]) {
            // Parameters swapped (normal flips), then the new u reversed (flips back).
            s = s.transposed().reversed_u();
            flags = boundaries(&s);
        }
        let [cu0, cu1, cv0, cv1, closed_u, closed_v] = flags;
        if cu0 || cu1 {
            return Err(KernelError::Unsupported(
                "a free-form surface with boundaries that collapse to points in both parameter directions".into(),
            ));
        }
        let (ul, _) = s.domain_u();
        let (vl, vh) = s.domain_v();
        let pole_lo = cv0.then(|| s.point_at(ul, vl));
        let pole_hi = cv1.then(|| s.point_at(ul, vh));
        let us = samples(&s.breaks_u(), SEEDS_PER_SPAN, MAX_SEEDS_PER_DIRECTION);
        let vs = samples(&s.breaks_v(), SEEDS_PER_SPAN, MAX_SEEDS_PER_DIRECTION);
        let mut seeds = Vec::with_capacity(us.len() * vs.len());
        for u in &us {
            for v in &vs {
                seeds.push((*u, *v, s.point_at(*u, *v)));
            }
        }
        let (lo, hi) = s.hull_box();
        let size = lo.distance(hi).max(1.0);
        if !size.is_finite() {
            return Err(KernelError::InvalidInput(
                "a free-form surface must lie inside the model's working range".into(),
            ));
        }
        Ok(NurbsSurface(Arc::new(Inner {
            frame: Frame::XY,
            surface: s,
            closed_u,
            closed_v: closed_v && !(cv0 || cv1),
            pole_lo,
            pole_hi,
            seeds,
            size,
            bezier: OnceLock::new(),
        })))
    }

    /// The world frame (free-form surfaces carry their placement in their control points).
    pub fn frame(&self) -> &Frame {
        &self.0.frame
    }
    /// The B-spline surface, in the kernel's parameterisation.
    pub fn surface(&self) -> &BSplineSurface3 {
        &self.0.surface
    }
    pub fn domain_u(&self) -> (f64, f64) {
        self.0.surface.domain_u()
    }
    pub fn domain_v(&self) -> (f64, f64) {
        self.0.surface.domain_v()
    }
    /// The period of `u`: the domain's width when the surface closes in `u`.
    pub fn period_u(&self) -> Option<f64> {
        self.0.closed_u.then(|| {
            let (a, b) = self.domain_u();
            b - a
        })
    }
    /// The period of `v`: the domain's width when the surface closes in `v`.
    pub fn period_v(&self) -> Option<f64> {
        self.0.closed_v.then(|| {
            let (a, b) = self.domain_v();
            b - a
        })
    }
    /// The poles: the points the boundaries `v = v_lo` and `v = v_hi` collapse to.
    pub fn poles(&self) -> (Option<Point3>, Option<Point3>) {
        (self.0.pole_lo, self.0.pole_hi)
    }
    /// The diagonal of the control-point box (at least 1 mm).
    pub fn size(&self) -> f64 {
        self.0.size
    }

    /// `x` moved by whole periods into `[lo, hi)` when `period` is set, else clamped.
    fn wrap(x: f64, (lo, hi): (f64, f64), closed: bool) -> f64 {
        if x.is_nan() {
            return lo;
        }
        if closed {
            let w = hi - lo;
            let y = lo + (x - lo).rem_euclid(w);
            if y >= hi { lo } else { y }
        } else {
            x.clamp(lo, hi)
        }
    }
    fn wrap_u(&self, u: f64) -> f64 {
        Self::wrap(u, self.domain_u(), self.0.closed_u)
    }
    fn wrap_v(&self, v: f64) -> f64 {
        Self::wrap(v, self.domain_v(), self.0.closed_v)
    }

    pub fn point_at(&self, u: f64, v: f64) -> Point3 {
        self.0.surface.point_at(self.wrap_u(u), self.wrap_v(v))
    }

    /// `(S_u, S_v)`.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3) {
        let (_, su, sv) = self.0.surface.derivatives(self.wrap_u(u), self.wrap_v(v));
        (su, sv)
    }

    /// `S` with its first and second partial derivatives.
    pub fn derivatives2(&self, u: f64, v: f64) -> SurfaceDerivatives {
        self.0.surface.derivatives2(self.wrap_u(u), self.wrap_v(v))
    }

    /// The unit normal, the direction of `S_u × S_v`. Where that vanishes (a pole, a
    /// degenerate point) it is the limit from inside the domain: the cross product a
    /// little way in, toward the middle of the domain.
    pub fn normal_at(&self, u: f64, v: f64) -> Vec3 {
        let (su, sv) = self.derivatives(u, v);
        if let Some(n) = su.cross(sv).normalized() {
            return n;
        }
        let (ul, uh) = self.domain_u();
        let (vl, vh) = self.domain_v();
        let (u, v) = (self.wrap_u(u), self.wrap_v(v));
        for frac in [1e-9, 1e-7, 1e-5, 1e-3] {
            let du = (0.5 * (ul + uh) - u) * frac;
            let dv = (0.5 * (vl + vh) - v) * frac;
            for (a, b) in [(u, v + dv), (u + du, v), (u + du, v + dv)] {
                let (su, sv) = self.derivatives(a, b);
                if let Some(n) = su.cross(sv).normalized() {
                    return n;
                }
            }
        }
        Vec3::Z
    }

    /// True when `p` is within [`LINEAR_TOLERANCE`] of a pole.
    pub fn is_pole(&self, p: Point3) -> bool {
        [self.0.pole_lo, self.0.pole_hi]
            .into_iter()
            .flatten()
            .any(|q| q.distance(p) <= LINEAR_TOLERANCE)
    }

    /// The parameters of the surface point nearest to `p` (see the module documentation):
    /// `u` (and `v`) in `[lo, hi)` in a periodic direction; at a pole `u` is the domain's
    /// start and `singular` is set.
    pub fn parameters(&self, p: Point3) -> SurfaceParameters {
        let (ul, _) = self.domain_u();
        let (vl, vh) = self.domain_v();
        if let Some(q) = self.0.pole_lo
            && q.distance(p) <= LINEAR_TOLERANCE
        {
            return SurfaceParameters {
                u: ul,
                v: vl,
                singular: true,
            };
        }
        if let Some(q) = self.0.pole_hi
            && q.distance(p) <= LINEAR_TOLERANCE
        {
            return SurfaceParameters {
                u: ul,
                v: vh,
                singular: true,
            };
        }
        let (u, v, _) = self.invert(p);
        SurfaceParameters {
            u: self.wrap_u(u),
            v: self.wrap_v(v),
            singular: false,
        }
    }

    /// The surface point nearest to `p`. `unique` is always true: whether another point
    /// is as near is not decided for free-form surfaces.
    pub fn closest_point(&self, p: Point3) -> KernelResult<SurfacePoint> {
        check::point3("point", p)?;
        let q = self.parameters(p);
        let point = self.point_at(q.u, q.v);
        Ok(SurfacePoint {
            u: q.u,
            v: q.v,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }

    /// Point inversion: `(u, v, distance)` of the nearest point found.
    fn invert(&self, p: Point3) -> (f64, f64, f64) {
        if !p.is_valid() {
            let (ul, _) = self.domain_u();
            let (vl, _) = self.domain_v();
            return (ul, vl, f64::INFINITY);
        }
        // The three nearest seeds, nearest first.
        let mut best: [(f64, f64, f64); 3] = [(0.0, 0.0, f64::INFINITY); 3];
        for (u, v, q) in &self.0.seeds {
            let d = q.distance(p);
            if d < best[2].2 {
                best[2] = (*u, *v, d);
                best.sort_by(|a, b| a.2.total_cmp(&b.2));
            }
        }
        let mut out = (best[0].0, best[0].1, best[0].2);
        for (k, (u0, v0, d0)) in best.into_iter().enumerate() {
            if !d0.is_finite() {
                break;
            }
            let r = self.newton(p, u0, v0);
            if r.2 < out.2 {
                out = r;
            }
            // A point near the surface (the usual query: an edge point, a mesh point)
            // is settled by its nearest seed; farther ones try the next seeds too.
            if out.2 <= NEAR_FRACTION * self.0.size || k >= 2 {
                break;
            }
        }
        out
    }

    /// Keeps a parameter pair in the domain: clamped in open directions, free in periodic
    /// ones (evaluation wraps).
    fn constrain(&self, u: f64, v: f64) -> (f64, f64) {
        let (ul, uh) = self.domain_u();
        let (vl, vh) = self.domain_v();
        let u = if self.0.closed_u { u } else { u.clamp(ul, uh) };
        let v = if self.0.closed_v { v } else { v.clamp(vl, vh) };
        (u, v)
    }

    /// Newton's method from `(u, v)` (see the module documentation).
    fn newton(&self, p: Point3, u0: f64, v0: f64) -> (f64, f64, f64) {
        let (mut u, mut v) = (u0, v0);
        let mut dist = self.point_at(u, v).distance(p);
        let tiny = STEP_RELATIVE * self.0.size;
        for _ in 0..MAX_NEWTON_STEPS {
            if dist <= tiny {
                break;
            }
            let d = self.derivatives2(u, v);
            let r = d.point - p;
            let f = r.dot(d.su);
            let g = r.dot(d.sv);
            let (lu, lv) = (d.su.length(), d.sv.length());
            let rl = r.length();
            if f.abs() <= ZERO_COSINE * lu * rl && g.abs() <= ZERO_COSINE * lv * rl {
                break;
            }
            let j11 = d.su.dot(d.su) + r.dot(d.suu);
            let j12 = d.su.dot(d.sv) + r.dot(d.suv);
            let j22 = d.sv.dot(d.sv) + r.dot(d.svv);
            let det = j11 * j22 - j12 * j12;
            let mut directions = Vec::with_capacity(2);
            if det.is_finite() && det.abs() > 1e-300 && j11 > 0.0 && det > 0.0 {
                directions.push(((g * j12 - f * j22) / det, (f * j12 - g * j11) / det));
            }
            // The gradient step scaled by the metric's diagonal.
            let (a, b) = (d.su.dot(d.su), d.sv.dot(d.sv));
            if a > 0.0 && b > 0.0 {
                directions.push((-f / a, -g / b));
            } else if a > 0.0 {
                directions.push((-f / a, 0.0));
            } else if b > 0.0 {
                directions.push((0.0, -g / b));
            }
            let mut moved = None;
            'dirs: for (du, dv) in directions {
                if !(du.is_finite() && dv.is_finite()) {
                    continue;
                }
                let mut t = 1.0;
                for _ in 0..MAX_HALVINGS {
                    let (un, vn) = self.constrain(u + t * du, v + t * dv);
                    let q = self.point_at(un, vn);
                    let dn = q.distance(p);
                    if dn <= dist {
                        moved = Some((un, vn, dn, q.distance(d.point)));
                        break 'dirs;
                    }
                    t *= 0.5;
                }
            }
            let Some((un, vn, dn, step)) = moved else {
                break;
            };
            u = un;
            v = vn;
            dist = dn;
            if step <= tiny {
                break;
            }
        }
        (u, v, dist)
    }

    /// The rates `(du/dt, dv/dt)` of the parameters along a curve on the surface through
    /// `S(u, v)` with tangent `d`: the least-squares solution of `S_u u' + S_v v' = d`.
    /// `None` where `S_u` and `S_v` are parallel (a pole).
    pub(crate) fn parameter_rates(&self, u: f64, v: f64, d: Vec3) -> Option<(f64, f64)> {
        let (su, sv) = self.derivatives(u, v);
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let det = a * c - b * b;
        if !(det.is_finite() && det > 1e-24 * (a * c).max(f64::MIN_POSITIVE)) {
            return None;
        }
        let (x, y) = (su.dot(d), sv.dot(d));
        Some(((x * c - y * b) / det, (y * a - x * b) / det))
    }

    /// The panel ends for integrating along `v` from `a` to `b` (either order): every
    /// knot break between them (repeated by whole periods in a periodic `v`), each span
    /// cut into [`V_PANELS_PER_SPAN`] panels. Ordered from `a` to `b`. Bounded: `None`
    /// when that would be more than `max` panels.
    pub(crate) fn v_panels(&self, a: f64, b: f64, max: usize) -> Option<Vec<f64>> {
        let (lo, hi) = (a.min(b), a.max(b));
        let breaks = self.0.surface.breaks_v();
        let mut cuts = vec![lo];
        let shifts: Vec<f64> = match self.period_v() {
            Some(p) if p > 0.0 => {
                let k0 = ((lo - breaks.first().copied().unwrap_or(lo)) / p).floor();
                let k1 = ((hi - breaks.first().copied().unwrap_or(lo)) / p).ceil();
                if !(k0.is_finite() && k1.is_finite()) || k1 - k0 > max as f64 {
                    return None;
                }
                let mut s = Vec::new();
                let mut k = k0;
                while k <= k1 {
                    s.push(k * p);
                    k += 1.0;
                }
                s
            }
            _ => vec![0.0],
        };
        for s in shifts {
            for k in &breaks {
                let x = k + s;
                if x > lo && x < hi {
                    cuts.push(x);
                }
            }
        }
        cuts.push(hi);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let mut out = Vec::with_capacity(cuts.len() * V_PANELS_PER_SPAN);
        for w in cuts.windows(2) {
            for k in 0..V_PANELS_PER_SPAN {
                out.push(w[0] + (w[1] - w[0]) * k as f64 / V_PANELS_PER_SPAN as f64);
            }
        }
        out.push(hi);
        if out.len() > max.saturating_add(1) {
            return None;
        }
        if a > b {
            out.reverse();
        }
        Some(out)
    }

    /// The box of the whole surface (its control points).
    pub fn bounding_box(&self) -> Aabb {
        let (min, max) = self.0.surface.hull_box();
        Aabb { min, max }
    }

    /// A box around the part of the surface over `[u0, u1] x [v0, v1]`: the control
    /// points of that sub-patch. Ranges reaching past a periodic direction's domain give
    /// the whole surface's box.
    pub fn bounding_box_on(&self, u0: f64, u1: f64, v0: f64, v1: f64) -> KernelResult<Aabb> {
        let (ul, uh) = self.domain_u();
        let (vl, vh) = self.domain_v();
        let inside = |a: f64, b: f64, lo: f64, hi: f64| {
            a.is_finite() && b.is_finite() && a < b && a >= lo && b <= hi
        };
        let (u0, u1) = (u0.max(ul), u1.min(uh));
        let (v0, v1) = (v0.max(vl), v1.min(vh));
        if !(inside(u0, u1, ul, uh) && inside(v0, v1, vl, vh)) {
            return Ok(self.bounding_box());
        }
        let sub = self.0.surface.sub_patch(u0, u1, v0, v1)?;
        let (min, max) = sub.hull_box();
        Ok(Aabb { min, max })
    }

    /// The surface cut into Bezier patches (one per pair of knot spans), with their
    /// parameter ranges. Refused for surfaces with more than [`MAX_BEZIER_PATCHES`].
    pub(crate) fn bezier_patches(&self) -> KernelResult<&[BezierPatch]> {
        if let Some(b) = self.0.bezier.get() {
            return Ok(b);
        }
        let bu = self.0.surface.breaks_u();
        let bv = self.0.surface.breaks_v();
        let count = bu
            .len()
            .saturating_sub(1)
            .saturating_mul(bv.len().saturating_sub(1));
        if count > MAX_BEZIER_PATCHES {
            return Err(KernelError::Unsupported(format!(
                "a free-form surface with more than {MAX_BEZIER_PATCHES} knot-span patches"
            )));
        }
        let mut out = Vec::with_capacity(count);
        for a in bu.windows(2) {
            for b in bv.windows(2) {
                let patch = self.0.surface.sub_patch(a[0], a[1], b[0], b[1])?;
                out.push((patch, [a[0], a[1], b[0], b[1]]));
            }
        }
        Ok(self.0.bezier.get_or_init(|| out))
    }

    /// The same surface moved by the isometry `t`. Under a mirror (`mirror`) `u` is
    /// reversed as well, so the normal is the mirror image of the original's:
    /// `(R S_u) × (R S_v) = det R · R (S_u × S_v)`.
    pub fn transformed(&self, t: &Transform, mirror: bool) -> KernelResult<NurbsSurface> {
        let s = self.0.surface.transformed(t)?;
        NurbsSurface::new(if mirror { s.reversed_u() } else { s })
    }
}

//! Area and volume of faces on free-form (NURBS) surfaces (rung K10, `nurbs.rs`).
//!
//! # Green's theorem in `(u, v)`, with the inner integral by quadrature
//! A face's area is `∬_D |S_u × S_v| du dv` and its share of the volume (divergence
//! theorem, as in `measure.rs`) `∬_D (S - c) · (S_u × S_v) du dv`, over its domain `D` in
//! the parameter plane. With `f` either integrand and
//! `F(u, v) = ∫_{v_lo}^{v} f(u, s) ds` (`v_lo` the start of the knot domain),
//! `∂F/∂v = f`, so Green's theorem gives `∬_D f du dv = ∮_∂D -F du` around the face's
//! loop images (`uv.rs`), counter-clockwise when the outward normal is `S_u × S_v`.
//!
//! `F` is evaluated by Gauss-Legendre quadrature along `v`: `f` is rational on each knot
//! span (with a square root for the area), analytic there, so each span is cut into
//! [`crate::nurbs::V_PANELS_PER_SPAN`] 5-point panels. The boundary integral is taken
//! edge by edge: an edge at constant `u` (a `v` boundary of the patch, a seam) adds
//! nothing; one at constant `v` (a `u` boundary, a step along a pole) adds
//! `-∫ F(u, v₀) du`, by adaptive Gauss-Legendre quadrature on each `u` knot span; any
//! other edge adds `-∫ F(u(t), v(t)) u'(t) dt` along its curve, with `(u, v)` the edge
//! point's projection and `(u', v')` the least-squares solution of
//! `S_u u' + S_v v' = C'(t)` ([`crate::NurbsSurface`]'s parameter rates).
//!
//! A loop that goes once around a periodic `u` without a seam is fine (a virtual seam at
//! constant `u` adds nothing, and `F` is periodic in `u`); one that goes around a
//! periodic `v` is refused, as on a torus.

use super::measure::{GAUSS5, integrate};
use super::uv::{EdgeImage, LoopImage};
use super::{Body, Curve, FaceId, Surface};
use crate::nurbs::NurbsSurface;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Aabb, Point3};

/// Image coordinates within this (relative) are the same: projections of an edge along a
/// parameter line differ only by rounding and the point inversion's accuracy.
const STRAIGHT_IMAGE_TOLERANCE: f64 = 1e-9;
/// Most Gauss panels one evaluation of `F` may use (hostile-input limit: a face that
/// winds around a periodic `v` many times).
const MAX_V_PANELS: usize = 65_536;

/// What a free-form face's integral measures.
#[derive(Debug, Clone, Copy)]
pub(super) enum FreeformIntegrand {
    Area,
    /// `(S - c) · (S_u × S_v)` about the world point `c`.
    Volume(Point3),
}

/// `f(u, v)`.
fn density(n: &NurbsSurface, kind: FreeformIntegrand, u: f64, v: f64) -> f64 {
    let (su, sv) = n.derivatives(u, v);
    let cross = su.cross(sv);
    match kind {
        FreeformIntegrand::Area => cross.length(),
        FreeformIntegrand::Volume(c) => (n.point_at(u, v) - c).dot(cross),
    }
}

/// `F(u, v) = ∫_{v_lo}^{v} f(u, s) ds` by Gauss-Legendre panels per knot span.
fn potential(n: &NurbsSurface, kind: FreeformIntegrand, u: f64, v: f64) -> KernelResult<f64> {
    let (vl, _) = n.domain_v();
    if !(u.is_finite() && v.is_finite()) {
        return Err(KernelError::InvalidInput(
            "a free-form face's parameters are not finite".into(),
        ));
    }
    if v == vl {
        return Ok(0.0);
    }
    let cuts = n.v_panels(vl, v, MAX_V_PANELS).ok_or_else(|| {
        KernelError::InvalidInput("a free-form face winds around its surface too many times".into())
    })?;
    let mut total = 0.0;
    for w in cuts.windows(2) {
        let (m, h) = (0.5 * (w[0] + w[1]), 0.5 * (w[1] - w[0]));
        for (x, wt) in GAUSS5 {
            total += wt * h * density(n, kind, u, m + h * x);
        }
    }
    Ok(total)
}

/// `-∫_{u0}^{u1} F(u, v) du` at constant `v`, panel by panel between `u` knots.
fn horizontal(
    n: &NurbsSurface,
    kind: FreeformIntegrand,
    v: f64,
    u0: f64,
    u1: f64,
) -> KernelResult<f64> {
    let (lo, hi) = (u0.min(u1), u0.max(u1));
    let mut cuts = vec![lo, hi];
    let breaks = n.surface().breaks_u();
    let shift = |k: f64| -> Vec<f64> {
        match n.period_u() {
            Some(p) if p > 0.0 => {
                let first = breaks.first().copied().unwrap_or(0.0);
                let k0 = ((lo - first) / p).floor();
                let k1 = ((hi - first) / p).ceil();
                if !(k0.is_finite() && k1.is_finite()) || k1 - k0 > 64.0 {
                    return Vec::new();
                }
                let mut out = Vec::new();
                let mut j = k0;
                while j <= k1 {
                    out.push(k + j * p);
                    j += 1.0;
                }
                out
            }
            _ => vec![k],
        }
    };
    for k in &breaks {
        cuts.extend(shift(*k).into_iter().filter(|x| *x > lo && *x < hi));
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut total = 0.0;
    for w in cuts.windows(2) {
        total += integrate(&|u| potential(n, kind, u, v), w[0], w[1])?;
    }
    Ok(if u0 <= u1 { -total } else { total })
}

/// `-∫ F(u(t), v(t)) u'(t) dt` along an edge image that is neither along nor across the
/// parameter lines, in the loop's direction.
fn oblique(n: &NurbsSurface, kind: FreeformIntegrand, e: &EdgeImage) -> KernelResult<f64> {
    let samples = e.uv.len().saturating_sub(1).max(1);
    let pv = n.period_v();
    let f = |t: f64| -> KernelResult<f64> {
        let p = e.curve.point_at(t);
        let q = n.parameters(p);
        if q.singular {
            // At a pole the image's u is undefined but F du there is a pole step's.
            return Ok(0.0);
        }
        let mut v = q.v;
        if let Some(period) = pv {
            // The unwrapped v of the nearest sample picks the representative.
            let s = ((t - e.t0) / (e.t1 - e.t0)).clamp(0.0, 1.0);
            let k = if e.forward { s } else { 1.0 - s };
            let idx = ((k * samples as f64).round() as usize).min(samples);
            if let Some(&(_, vs)) = e.uv.get(idx) {
                v += ((vs - v) / period).round() * period;
            }
        }
        let Some((du, _)) = n.parameter_rates(q.u, q.v, e.curve.d1(t)) else {
            return Ok(0.0);
        };
        Ok(-potential(n, kind, q.u, v)? * du)
    };
    let (lo, hi) = (e.t0.min(e.t1), e.t0.max(e.t1));
    let mut cuts = vec![lo, hi];
    if let Curve::BSpline(s) = &e.curve {
        cuts.extend(s.breaks().into_iter().filter(|k| *k > lo && *k < hi));
    }
    cuts.sort_by(f64::total_cmp);
    let mut along = 0.0;
    for w in cuts.windows(2) {
        along += integrate(&f, w[0], w[1])?;
    }
    let along = if e.t0 <= e.t1 { along } else { -along };
    Ok(if e.forward { along } else { -along })
}

/// One edge image's term: zero, at constant `v`, or along its curve.
fn edge_term(n: &NurbsSurface, kind: FreeformIntegrand, e: &EdgeImage) -> KernelResult<f64> {
    let (Some(&(u0, v0)), Some(&(u1, _))) = (e.uv.first(), e.uv.last()) else {
        return Ok(0.0);
    };
    let tol = |x: f64| STRAIGHT_IMAGE_TOLERANCE * x.abs().max(1.0);
    if e.uv.iter().all(|q| (q.0 - u0).abs() <= tol(u0)) {
        return Ok(0.0);
    }
    if e.uv.iter().all(|q| (q.1 - v0).abs() <= tol(v0)) {
        return horizontal(n, kind, v0, u0, u1);
    }
    oblique(n, kind, e)
}

impl Body {
    /// True when any face lies on a free-form (NURBS) surface.
    pub fn has_freeform_faces(&self) -> bool {
        self.faces()
            .into_iter()
            .any(|f| matches!(self.face_surface(f), Ok(Some((Surface::Nurbs(_), _)))))
    }

    /// The box of a face on a free-form surface: the control points of the sub-patch
    /// over the range its loop images cover (the whole surface's box when the images
    /// cannot be made or reach around a periodic direction). `None` for other faces.
    pub(super) fn freeform_face_box(&self, face: FaceId) -> Option<Aabb> {
        let Ok(Some((surface, same))) = self.face_surface(face) else {
            return None;
        };
        let Surface::Nurbs(n) = &surface else {
            return None;
        };
        let orient = if same { 1.0 } else { -1.0 };
        let mut range: Option<[f64; 4]> = None;
        for l in self.face_loops(face).ok()? {
            let Ok(img) = self.loop_image(l, &surface, orient) else {
                return Some(n.bounding_box());
            };
            for (u, v) in img.points() {
                range = Some(match range {
                    None => [u, u, v, v],
                    Some([a, b, c, d]) => [a.min(u), b.max(u), c.min(v), d.max(v)],
                });
            }
        }
        let [u0, u1, v0, v1] = range?;
        Some(
            n.bounding_box_on(u0, u1, v0, v1)
                .unwrap_or_else(|_| n.bounding_box()),
        )
    }

    /// `∮ -F du` over the loops of a face on a free-form surface (see the module
    /// documentation); `orient` is the face's orientation relative to `S_u × S_v`. The
    /// loops' direction carries the orientation, so the result is the face's signed
    /// integral `∬ f dA` taken about its outward normal times `orient`.
    pub(super) fn freeform_integral(
        &self,
        face: FaceId,
        surface: &Surface,
        n: &NurbsSurface,
        orient: f64,
        kind: FreeformIntegrand,
    ) -> KernelResult<f64> {
        let mut total = 0.0;
        for l in self.face_loops(face)? {
            let img: LoopImage = self.loop_image(l, surface, orient)?;
            if img.wraps_v {
                return Err(KernelError::Unsupported(format!(
                    "{l} goes around a closed free-form surface's v direction without a seam"
                )));
            }
            for e in &img.edges {
                total += edge_term(n, kind, e)?;
            }
            for s in img.pole_steps() {
                total += horizontal(n, kind, s.v, s.u0, s.u1)?;
            }
        }
        Ok(total)
    }
}

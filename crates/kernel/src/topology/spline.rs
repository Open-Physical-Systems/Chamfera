//! Bodies with spline geometry (rung K10): which operations they reach, and the area and
//! volume of faces on surfaces of linear extrusion.
//!
//! # Measures on an extrusion (Green's theorem in `(u, v)`)
//! On `S(u, v) = o + c(u) + v z` (`extrusion.rs`) both integrands of `measure.rs` depend
//! on `u` only: `|S_u × S_v| = |c'(u)|`, and for a point `q` with local coordinates
//! `(q_x, q_y, q_z)`, `(S - q) · (S_u × S_v) = (c_x - q_x) c_y' - (c_y - q_y) c_x'`. With
//! `f(u)` either one, `F = v f(u)` has `∂F/∂v = f`, so the face integral is `∮ -F du`
//! around the face's loop images. An edge along the extrusion (constant `u`) contributes
//! nothing; an edge at constant height `v₀` from `u₀` to `u₁` contributes
//! `-v₀ ∫ f du`: `-v₀` times the arc length (area), or `-v₀` times twice the curve's
//! area moment about `q` (volume), both by adaptive Gauss-Legendre quadrature on the
//! curve. Extrude makes only these two kinds of edge on its side faces; a boolean's
//! oblique plane section is integrated along its own parameter (`oblique_term`).

use super::uv::{EdgeImage, LoopImage};
use super::{Body, Curve, FaceId, Surface};
use crate::{Extrusion, KernelResult};
use chamfera_geom::{Point2, Point3};

/// Image coordinates within this (relative) are the same: projections of an edge along
/// or across the extrusion differ only by rounding and the point inversion's accuracy.
const STRAIGHT_IMAGE_TOLERANCE: f64 = 1e-9;

/// `-∫ v f(u) du` along an edge whose image is neither along nor across the extrusion
/// (a booleans' oblique section), in the edge's own parameter `t` in the loop's direction:
/// `u(t)` is the curve parameter of the edge point's projection, `v(t)` its height, and
/// `u'(t) = c'(u) · p'(t) / |c'(u)|²` (the parameterisation is orthogonal, so the
/// component of `p'` along `S_u` is `u' |c'|²`). Adaptive Gauss-Legendre quadrature.
fn oblique_term(e: &Extrusion, edge: &EdgeImage, centre: Option<Point3>) -> KernelResult<f64> {
    let (frame, curve) = (e.frame(), e.curve());
    let (ta, tb) = if edge.forward {
        (edge.t0, edge.t1)
    } else {
        (edge.t1, edge.t0)
    };
    let integrand = |t: f64| -> KernelResult<f64> {
        let p = frame.to_local(edge.curve.point_at(t));
        let dp = frame.vector_to_local(edge.curve.d1(t));
        let u = curve.closest_point(Point2::new(p.x, p.y)).u;
        let d = curve.derivatives(u, 1);
        let (Some(c), Some(cd)) = (d.first(), d.get(1)) else {
            return Ok(0.0);
        };
        let speed2 = cd.dot(*cd);
        if speed2.is_nan() || speed2 <= 0.0 {
            return Ok(0.0);
        }
        let du = (cd.x * dp.x + cd.y * dp.y) / speed2;
        let f = match centre {
            None => speed2.sqrt(),
            Some(q) => (c.x - q.x) * cd.y - (c.y - q.y) * cd.x,
        };
        Ok(-p.z * f * du)
    };
    let (lo, hi) = (ta.min(tb), ta.max(tb));
    let mut total = 0.0;
    // Panels at the edge curve's own breaks (a spline's knots) and the extruded curve's.
    let mut cuts = vec![lo, hi];
    if let super::Curve::BSpline(s) = &edge.curve {
        cuts.extend(s.breaks().into_iter().filter(|k| *k > lo && *k < hi));
    }
    cuts.sort_by(f64::total_cmp);
    for w in cuts.windows(2) {
        total += super::measure::integrate(&integrand, w[0], w[1])?;
    }
    Ok(if ta <= tb { total } else { -total })
}

impl Body {
    /// True when any face lies on a spline surface or any edge on a spline curve.
    pub fn has_spline_geometry(&self) -> bool {
        self.surfaces
            .live()
            .any(|(_, s)| matches!(s, Surface::Extrusion(_)))
            || self
                .curves
                .live()
                .any(|(_, c)| matches!(c, Curve::BSpline(_)))
    }

    /// `∮ -F du` over the loops of a face on an extrusion (see the module documentation):
    /// the area integrand when `centre` is `None`, else the volume integrand about
    /// `centre` (in the surface's local coordinates).
    pub(super) fn extrusion_integral(
        &self,
        face: FaceId,
        surface: &Surface,
        e: &Extrusion,
        orient: f64,
        centre: Option<Point3>,
    ) -> KernelResult<f64> {
        let curve = e.curve();
        let mut total = 0.0;
        for l in self.face_loops(face)? {
            let img: LoopImage = self.loop_image(l, surface, orient)?;
            for edge in &img.edges {
                let (Some(&(u0, v0)), Some(&(u1, _))) = (edge.uv.first(), edge.uv.last()) else {
                    continue;
                };
                let tol = |x: f64| STRAIGHT_IMAGE_TOLERANCE * x.abs().max(1.0);
                if edge.uv.iter().all(|q| (q.0 - u0).abs() <= tol(u0)) {
                    continue;
                }
                if !edge.uv.iter().all(|q| (q.1 - v0).abs() <= tol(v0)) {
                    total += oblique_term(e, edge, centre)?;
                    continue;
                }
                let integral = match centre {
                    None => curve.length_between(u0, u1)?,
                    Some(q) => 2.0 * curve.moment_about(u0, u1, Point2::new(q.x, q.y))?,
                };
                total -= v0 * integral;
            }
        }
        Ok(total)
    }
}

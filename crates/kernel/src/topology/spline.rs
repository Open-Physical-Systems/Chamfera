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
//! curve. Extrude makes only these two kinds of edge on its side faces; any other edge
//! is refused (`Unsupported`), as booleans do not reach spline faces yet.

use super::uv::LoopImage;
use super::{Body, Curve, FaceId, Surface};
use crate::{Extrusion, KernelError, KernelResult};
use chamfera_geom::{Point2, Point3};

/// Image coordinates within this (relative) are the same: projections of an edge along
/// or across the extrusion differ only by rounding and the point inversion's accuracy.
const STRAIGHT_IMAGE_TOLERANCE: f64 = 1e-9;

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
                    return Err(KernelError::Unsupported(format!(
                        "{} crosses a spline face obliquely; only extruded spline faces are measured",
                        edge.edge
                    )));
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

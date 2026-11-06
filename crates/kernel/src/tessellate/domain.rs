//! The (u, v) parameter domain a face is triangulated in.
//!
//! Every face goes through [`SurfaceDomain`]: its boundary points are mapped to (u, v)
//! with [`SurfaceDomain::surface_uv`], triangulated there, and points the refinement
//! adds inside the face are lifted back with [`SurfaceDomain::surface_point`]. A plane
//! is the first implementation (u, v are the plane frame's x, y coordinates); K3 adds
//! one per curved surface in [`domain_for`].

use crate::{KernelError, KernelResult, Surface};
use chamfera_geom::{Frame, Point2, Point3, Vec3};

/// A surface seen through its parameterisation.
///
/// Contract for implementations:
/// - `surface_point(surface_uv(p)) == p` for points on the surface (up to rounding),
///   and `surface_uv` is continuous except across the seam of a periodic direction;
/// - `∂S/∂u × ∂S/∂v` points along the surface normal ([`SurfaceDomain::surface_normal`]),
///   so a loop counter-clockwise in (u, v) is counter-clockwise about the normal;
/// - (u, v) is roughly isometric (e.g. `u = r · angle` on a cylinder of radius `r`), so
///   Delaunay triangles in (u, v) are well shaped on the surface.
pub(super) trait SurfaceDomain {
    /// The surface point at `uv`.
    fn surface_point(&self, uv: Point2) -> Point3;
    /// The parameters of a point on the surface. In a periodic direction any
    /// representative will do; loops are unwrapped by the caller ([`unwrap_loop`]).
    fn surface_uv(&self, p: Point3) -> Point2;
    /// The unit surface normal at `uv` (not the face's outward normal, which may be its
    /// opposite).
    fn surface_normal(&self, uv: Point2) -> Vec3;
    /// The period of u, if the surface closes on itself in u.
    fn u_period(&self) -> Option<f64> {
        None
    }
    /// The period of v, if the surface closes on itself in v.
    fn v_period(&self) -> Option<f64> {
        None
    }
    /// True when every flat triangle between points of the surface lies on it (a
    /// plane): no refinement inside the face is needed.
    fn is_flat(&self) -> bool {
        false
    }
}

/// A plane: u, v are the coordinates along its frame's x and y axes.
pub(super) struct PlaneDomain {
    frame: Frame,
}

impl SurfaceDomain for PlaneDomain {
    fn surface_point(&self, uv: Point2) -> Point3 {
        self.frame.from_plane(uv)
    }
    fn surface_uv(&self, p: Point3) -> Point2 {
        self.frame.to_plane(p)
    }
    fn surface_normal(&self, _uv: Point2) -> Vec3 {
        self.frame.z()
    }
    fn is_flat(&self) -> bool {
        true
    }
}

/// The domain of a surface. K3 adds one arm per curved surface here; until a surface
/// has an arm, its faces are refused as unsupported.
#[allow(unreachable_patterns)] // the catch-all arm is for surfaces K3 adds
pub(super) fn domain_for(surface: &Surface) -> KernelResult<Box<dyn SurfaceDomain>> {
    match surface {
        Surface::Plane(frame) => Ok(Box::new(PlaneDomain { frame: *frame })),
        _ => Err(KernelError::Unsupported(
            "tessellating this kind of surface is not implemented yet".into(),
        )),
    }
}

/// Makes a loop's (u, v) points continuous across the seams of periodic directions:
/// each point is moved by whole periods to lie within half a period of the one before
/// it. Edges have at most a third of a turn per segment, so consecutive points of a
/// loop are never more than half a period apart on the surface. Fails if the loop does
/// not close in (u, v), i.e. it winds around the surface; such a face must be cut along
/// a seam edge first.
pub(super) fn unwrap_loop(
    uv: &mut [Point2],
    u_period: Option<f64>,
    v_period: Option<f64>,
) -> KernelResult<()> {
    let shift = |prev: f64, cur: f64, period: Option<f64>| match period {
        Some(p) if p > 0.0 && p.is_finite() => cur - ((cur - prev) / p).round() * p,
        _ => cur,
    };
    for i in 1..uv.len() {
        let (prev, cur) = (uv[i - 1], uv[i]);
        uv[i] = Point2::new(
            shift(prev.x, cur.x, u_period),
            shift(prev.y, cur.y, v_period),
        );
    }
    // The closing step from the last point back to the first must need no shift either.
    if let (Some(first), Some(last)) = (uv.first().copied(), uv.last().copied()) {
        let closes = |a: f64, b: f64, period: Option<f64>| match period {
            Some(p) if p > 0.0 && p.is_finite() => ((a - b) / p).round() == 0.0,
            _ => true,
        };
        if !(closes(first.x, last.x, u_period) && closes(first.y, last.y, v_period)) {
            return Err(KernelError::Unsupported(
                "a face loop winds around a periodic surface without a seam edge".into(),
            ));
        }
    }
    Ok(())
}

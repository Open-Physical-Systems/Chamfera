//! The (u, v) parameter domain a face is triangulated in.
//!
//! Every face goes through [`SurfaceDomain`]: its boundary points are mapped to (u, v)
//! with [`SurfaceDomain::surface_uv`], triangulated there, and points the refinement
//! adds inside the face are lifted back with [`SurfaceDomain::surface_point`].
//!
//! # The domains
//! A plane uses its frame's x, y coordinates. The four curved surfaces are surfaces of
//! revolution `S(θ, w) = o + ρ(w) e(θ) + h(w) z` (see `surface.rs`), and their domain is
//! the kernel parameterisation scaled to lengths, `(u, v) = (s_u θ, s_v w)`:
//!
//! | Surface | `s_u` | `s_v` | periods | singular points |
//! |---|---|---|---|---|
//! | Cylinder, radius `r` | `r` (isometric) | 1 (`w` is the height) | u: `2π r` | none |
//! | Cone | the largest distance of the face's boundary from the axis | 1 (`w` is the length along a generator) | u: `2π s_u` | the apex |
//! | Sphere, radius `r` | `r` | `r` (isometric at the equator) | u: `2π r` | the two poles |
//! | Torus, `R`, `a` | the tube's mean distance from the axis, `(ρ_max + max(ρ_min, 0)) / 2` | `a` (isometric) | u: `2π s_u`, v: `2π a` | a spindle torus's tips |
//!
//! Scaling by positive constants keeps `∂S/∂u × ∂S/∂v` along the kernel's normal
//! ([`Surface::normal_at`]), so the orientation contract holds. The u scale is exact on a
//! cylinder and at the widest parallel elsewhere; nearer the axis Delaunay triangles in
//! (u, v) are wide in u, which the refinement in space corrects.
//!
//! - **Seams.** `surface_uv` returns `θ` in `[0, 2π)`; loops are made continuous by whole
//!   periods ([`unwrap_loop`]).
//! - **Singular points** (on the axis, where `θ` is undefined: the poles, the apex, a
//!   spindle torus's tips) are reported by [`SurfaceDomain::is_singular`]; the face
//!   tessellation (`mod.rs`) replaces each by a run of copies along its singular line.
//! - **Spans.** [`SurfaceDomain::max_span`] bounds the extent of a triangle's interior
//!   sides in u (and in v on a sphere or torus) to a quarter turn around the axis (and
//!   along the meridian circle). Away from the axis the chord and normal tests stop far
//!   shorter sides anyway; near a pole they do not, and a triangle spanning half a turn
//!   there would fold over.

use crate::{KernelError, KernelResult, Surface};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Point3, Vec3};
use std::f64::consts::FRAC_PI_2;

/// The largest turn about the axis (and along a circular meridian) an interior side of
/// a triangle may span on a surface of revolution: a quarter turn. A triangle fanning
/// into a pole folds over once its sides span half a turn; away from the axis the chord
/// and normal tests stop far shorter sides anyway, so this only acts near a pole and on
/// the first, coarse triangulation.
const FOLD_SPAN: f64 = FRAC_PI_2;

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
    /// True when `p` is a singular point of the parameterisation, where u is undefined
    /// and only the v of `surface_uv` means anything.
    fn is_singular(&self, _p: Point3) -> bool {
        false
    }
    /// The largest extent in u and in v an interior side of a triangle may have
    /// (infinite where nothing limits it).
    fn max_span(&self) -> (f64, f64) {
        (f64::INFINITY, f64::INFINITY)
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

/// A surface of revolution (cylinder, cone, sphere, torus) in the scaled kernel
/// parameters `(u, v) = (s_u θ, s_v w)`; see the module documentation.
pub(super) struct RevolutionDomain {
    surface: Surface,
    su: f64,
    sv: f64,
    /// The meridian is a circle (sphere, torus), so v is limited by the angle too.
    curved_meridian: bool,
}

impl SurfaceDomain for RevolutionDomain {
    fn surface_point(&self, uv: Point2) -> Point3 {
        self.surface.point_at(uv.x / self.su, uv.y / self.sv)
    }
    fn surface_uv(&self, p: Point3) -> Point2 {
        let q = self.surface.parameters(p);
        Point2::new(q.u * self.su, q.v * self.sv)
    }
    fn surface_normal(&self, uv: Point2) -> Vec3 {
        self.surface.normal_at(uv.x / self.su, uv.y / self.sv)
    }
    fn u_period(&self) -> Option<f64> {
        self.surface.period_u().map(|p| p * self.su)
    }
    fn v_period(&self) -> Option<f64> {
        self.surface.period_v().map(|p| p * self.sv)
    }
    fn is_singular(&self, p: Point3) -> bool {
        self.surface.parameters(p).singular
    }
    fn max_span(&self) -> (f64, f64) {
        let v = if self.curved_meridian {
            self.sv * FOLD_SPAN
        } else {
            f64::INFINITY
        };
        (self.su * FOLD_SPAN, v)
    }
}

/// The domain of a face on `surface`; `boundary` holds the face's boundary points (a
/// cone's u scale is taken from them).
pub(super) fn domain_for(
    surface: &Surface,
    boundary: &[Point3],
) -> KernelResult<Box<dyn SurfaceDomain>> {
    let revolution = |su: f64, sv: f64, curved_meridian: bool| {
        if !(su.is_finite() && sv.is_finite() && su > 0.0 && sv > 0.0) {
            return Err(KernelError::InvalidInput(
                "a curved face is too small to tessellate".into(),
            ));
        }
        let domain: Box<dyn SurfaceDomain> = Box::new(RevolutionDomain {
            surface: *surface,
            su,
            sv,
            curved_meridian,
        });
        Ok(domain)
    };
    match surface {
        Surface::Plane(frame) => Ok(Box::new(PlaneDomain { frame: *frame })),
        Surface::Cylinder(c) => revolution(c.radius(), 1.0, false),
        Surface::Cone(c) => {
            let widest = boundary
                .iter()
                .map(|p| {
                    let l = c.frame().to_local(*p);
                    l.x.hypot(l.y)
                })
                .fold(0.0, f64::max);
            revolution(widest.max(LINEAR_TOLERANCE), 1.0, false)
        }
        Surface::Sphere(s) => revolution(s.radius(), s.radius(), true),
        Surface::Torus(t) => {
            let (big, a) = (t.major_radius(), t.minor_radius());
            let mean = 0.5 * ((big + a) + (big - a).max(0.0));
            revolution(mean.max(LINEAR_TOLERANCE), a, true)
        }
    }
}

/// `step + k p` for the integer `k` that puts it in `(0, p]` (`dir > 0`) or `[-p, 0)`
/// (`dir < 0`), steps within `tiny` of zero counting as a whole period: the step a loop
/// takes along a singular line, which always moves (`mod.rs`).
pub(super) fn directed_step(step: f64, dir: f64, p: f64, tiny: f64) -> f64 {
    if dir > 0.0 {
        let k = ((tiny - step) / p).floor() + 1.0;
        step + k * p
    } else {
        let k = ((-tiny - step) / p).ceil() - 1.0;
        step + k * p
    }
}

/// Makes a loop's (u, v) points continuous across the seams of periodic directions:
/// each point is moved by whole periods to lie within half a period of the one before
/// it. Edges have at most a third of a turn per segment, so consecutive points of a
/// loop are never more than half a period apart on the surface. Fails if the loop does
/// not close in (u, v), i.e. it winds around the surface; such a face must be cut along
/// a seam edge first.
#[cfg_attr(not(test), allow(dead_code))] // the face path unwraps in `mod.rs`, with poles
pub(super) fn unwrap_loop(
    uv: &mut [Point2],
    u_period: Option<f64>,
    v_period: Option<f64>,
) -> KernelResult<()> {
    for i in 1..uv.len() {
        let (prev, cur) = (uv[i - 1], uv[i]);
        uv[i] = Point2::new(
            nearest(prev.x, cur.x, u_period),
            nearest(prev.y, cur.y, v_period),
        );
    }
    check_closed(uv, u_period, v_period)
}

/// `cur` moved by whole periods to within half a period of `prev`.
pub(super) fn nearest(prev: f64, cur: f64, period: Option<f64>) -> f64 {
    match period {
        Some(p) if p > 0.0 && p.is_finite() => cur - ((cur - prev) / p).round() * p,
        _ => cur,
    }
}

/// The closing step from the last point back to the first must need no shift.
pub(super) fn check_closed(
    uv: &[Point2],
    u_period: Option<f64>,
    v_period: Option<f64>,
) -> KernelResult<()> {
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

//! Two cylinders.
//!
//! - Parallel axes: every cross-section perpendicular to the axes shows two circles, so
//!   the common points are lines along the axes through the common points of those
//!   circles (coaxial and equal: coincident; coaxial and unequal: empty).
//! - Axes meeting at `C` with equal radii `r`: with unit axes `a`, `b` and `q = p - C`,
//!   the cylinders are `|q|² - (q · a)² = r²` and `|q|² - (q · b)² = r²`. Subtracting,
//!   `(q · a)² = (q · b)²`, i.e. `q · (a - b) = 0` or `q · (a + b) = 0`: the common points
//!   lie in the two planes through `C` bisecting the axes, and each plane cuts the first
//!   cylinder in an ellipse (see `plane::plane_cylinder`). For orthogonal axes the
//!   ellipses have semi-axes `r √2` (along `(a ± b)/√2`) and `r` (along `a × b`), and they
//!   cross at the two points `C ± r (a × b)/|a × b|`, where the cylinders touch.
//! - Axes farther apart than `r1 + r2`: empty (a common point would be within `r1` of one
//!   axis and `r2` of the other).
//! - Otherwise the intersection is a quartic space curve: unsupported until K10.

use super::revolution::{Common, circle_circle};
use super::{
    SurfaceIntersection, UnsupportedIntersection, beyond_range, line_curve, parallel, plane,
};
use crate::{Cylinder, KernelResult};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2};

pub(crate) fn cylinder_cylinder(a: &Cylinder, b: &Cylinder) -> KernelResult<SurfaceIntersection> {
    let (fa, fb) = (a.frame(), b.frame());
    let (za, zb) = (fa.z(), fb.z());
    let (ra, rb) = (a.radius(), b.radius());
    if parallel(za, zb) {
        let q = fa.to_local(fb.origin());
        return match circle_circle(Point2::ORIGIN, ra, Point2::new(q.x, q.y), rb) {
            Common::Overlap => Ok(SurfaceIntersection::Coincident),
            Common::Points(points) => {
                let mut curves = Vec::with_capacity(points.len());
                for (p, contact) in points {
                    curves.push(line_curve(fa.from_plane(p), za, false, contact)?);
                }
                Ok(SurfaceIntersection::from_parts(curves, vec![]))
            }
        };
    }
    // Closest points of the axes oa + s za and ob + t zb (Ericson, Real-Time Collision
    // Detection, 5.1.8): with w = oa - ob, k = za · zb, d = za · w, e = zb · w,
    // s = (k e - d) / (1 - k²), t = (e - k d) / (1 - k²).
    let w = fa.origin() - fb.origin();
    let k = za.dot(zb);
    let (d, e) = (za.dot(w), zb.dot(w));
    let denom = za.cross(zb).length().powi(2);
    let s = (k * e - d) / denom;
    let t = (e - k * d) / denom;
    let pa = fa.origin() + za * s;
    let pb = fb.origin() + zb * t;
    let gap = pa.distance(pb);
    if gap > ra + rb + LINEAR_TOLERANCE {
        return Ok(SurfaceIntersection::Empty);
    }
    if gap <= LINEAR_TOLERANCE && (ra - rb).abs() <= LINEAR_TOLERANCE {
        let centre = pa + (pb - pa) * 0.5;
        let mut curves = Vec::new();
        for n in [za - zb, za + zb] {
            let len = n.length();
            let frame = Frame::from_normal(centre, n * (1.0 / len))
                .ok_or_else(|| beyond_range("ellipse"))?;
            curves.extend_from_slice(plane::plane_cylinder(&frame, a)?.curves());
        }
        return Ok(SurfaceIntersection::from_parts(curves, vec![]));
    }
    Ok(SurfaceIntersection::Unsupported(
        UnsupportedIntersection::CylinderQuartic,
    ))
}

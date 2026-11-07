//! Tight boxes of sphere and torus faces, from the face's parameter ranges.
//!
//! A face on a sphere or torus `S(u, v) = o + ρ(v) e(u) + h(v) z` lies inside the image of
//! the rectangle(s) of `(u, v)` it covers, and the box of such an image is exact to work
//! out (below). The covered ranges come from the face's loop images:
//!
//! 1. Every pair of consecutive boundary samples spans an interval of `u` (and of `v`).
//!    The images of the edges the kernel builds by sweeping are straight, so the interval
//!    holds the boundary between the two samples; an edge whose image is not straight
//!    (an intersection curve) has its intervals grown by their own length on each side.
//! 2. Between those intervals are *gaps*: ranges of `u` no boundary point has. The strip
//!    of the surface over a gap (`u` in the gap, every `v`) is connected and meets no
//!    boundary, so it lies wholly inside the face or wholly outside it. One point decides
//!    which ([`Body::classify_point_on_face`]); a covered gap joins its neighbours.
//!
//! The same holds for `v` (periodic on a torus, `[-π/2, π/2]` on a sphere, where the strip
//! over a gap is a band of parallels).
//!
//! # The box of a rectangle's image
//! Along world axis `i` a surface point is `o_i + ρ(v) g_i(u) + h(v) z_i` with
//! `g_i(u) = x_i cos u + y_i sin u = A cos(u - φ)`, `A = hypot(x_i, y_i)`,
//! `φ = atan2(y_i, x_i)`. Over `u ∈ [u0, u1]` the range of `cos` is its end values widened
//! to ±1 where a multiple of π lies inside; `ρ` and `h` are `R + a cos v`, `a sin v` on a
//! torus and `r cos v`, `r sin v` on a sphere. The product `ρ g_i` of two ranges reaches
//! its extremes at the corners, so the sum of the three ranges holds the image.

use crate::surface::Meridian;
use crate::{Body, FaceId, KernelResult, PointOnFace, Surface};
use chamfera_geom::{Aabb, Point3};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Rounding slack added to every sampled interval (radians).
const SLACK: f64 = 1e-9;

/// The tight box of a sphere or torus face, or `None` when it cannot be worked out (the
/// caller then uses the box of the whole surface).
pub(super) fn revolution_face_box(body: &Body, f: FaceId, surface: &Surface) -> Option<Aabb> {
    let meridian = match surface.meridian() {
        Some(m @ (Meridian::Sphere { .. } | Meridian::Torus { .. })) => m,
        _ => return None,
    };
    let (u, v) = face_ranges(body, f, surface).ok()??;
    let mut out: Option<Aabb> = None;
    for &ur in &u {
        for &vr in &v {
            let b = rectangle_box(surface, meridian, ur, vr);
            out = Some(out.map_or(b, |o| o.union(b)));
        }
    }
    out
}

/// The `u` and `v` ranges a face covers (`None` if the face has no usable image).
type Ranges = Vec<(f64, f64)>;

fn face_ranges(
    body: &Body,
    f: FaceId,
    surface: &Surface,
) -> KernelResult<Option<(Ranges, Ranges)>> {
    let (_, orient) = body.face_orientation(f)?;
    let mut us = Vec::new();
    let mut vs = Vec::new();
    for lp in body.face_loops(f)? {
        let image = body.loop_image(lp, surface, orient)?;
        for e in &image.edges {
            let grow = if straight(&e.uv) { 0.0 } else { 1.0 };
            for w in e.uv.windows(2) {
                let [a, b] = w else { continue };
                us.push(padded(a.0, b.0, grow));
                vs.push(padded(a.1, b.1, grow));
            }
        }
    }
    if us.is_empty()
        || us
            .iter()
            .chain(&vs)
            .any(|r| !r.0.is_finite() || !r.1.is_finite())
    {
        return Ok(None);
    }
    // A strip is inside the face when one of its points is (or is on the boundary, which
    // keeps the strip: conservative).
    let inside = |p: Point3| -> KernelResult<bool> {
        Ok(body.classify_point_on_face(f, p)? != PointOnFace::Outside)
    };
    let u = cover(us, Some(TAU), |um| inside(surface.point_at(um, 0.0)))?;
    let v = match surface {
        Surface::Torus(_) => cover(vs, Some(TAU), |vm| inside(surface.point_at(0.0, vm)))?,
        _ => cover(
            vs.into_iter()
                .map(|(a, b)| (a.max(-FRAC_PI_2), b.min(FRAC_PI_2)))
                .filter(|(a, b)| a <= b)
                .collect(),
            None,
            |vm| inside(surface.point_at(0.0, vm)),
        )?,
    };
    Ok(Some((u, v)))
}

/// The interval between two samples, grown by `grow` times its length on each side
/// (and by [`SLACK`]).
fn padded(a: f64, b: f64, grow: f64) -> (f64, f64) {
    let (lo, hi) = (a.min(b), a.max(b));
    let d = (hi - lo) * grow;
    (lo - d - SLACK, hi + d + SLACK)
}

/// True when the samples lie on the chord from the first to the last (within [`SLACK`]):
/// the image of an edge the kernel builds by sweeping (a line along or through the axis,
/// a circle about it, a profile curve through it), which is exact between samples.
fn straight(uv: &[(f64, f64)]) -> bool {
    let (Some(&a), Some(&b)) = (uv.first(), uv.last()) else {
        return true;
    };
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len <= SLACK {
        return uv.iter().all(|p| (p.0 - a.0).hypot(p.1 - a.1) <= SLACK);
    }
    uv.iter()
        .all(|p| ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len <= SLACK)
}

/// The ranges covered by the boundary intervals `raw` and by the gaps between them for
/// which `covered(mid)` holds. With a `period`, ranges live on a circle; without one, in
/// `[-π/2, π/2]` (a sphere's `v`).
fn cover(
    raw: Vec<(f64, f64)>,
    period: Option<f64>,
    mut covered: impl FnMut(f64) -> KernelResult<bool>,
) -> KernelResult<Vec<(f64, f64)>> {
    let Some(p) = period else {
        return cover_line(raw, covered);
    };
    let mut iv = Vec::with_capacity(raw.len());
    for (lo, hi) in raw {
        if hi - lo >= p {
            return Ok(vec![(0.0, p)]);
        }
        let k = (lo / p).floor();
        iv.push((lo - k * p, hi - k * p));
    }
    let mut iv = merge(iv);
    // Close the circle: the last interval may run past the first one's start.
    while iv.len() > 1 {
        let (Some(&first), Some(last)) = (iv.first(), iv.last()) else {
            break;
        };
        if last.1 < first.0 + p {
            break;
        }
        let hi = last.1.max(first.1 + p);
        iv.remove(0);
        if let Some(l) = iv.last_mut() {
            l.1 = hi;
        }
    }
    if iv.iter().any(|(lo, hi)| hi - lo >= p) {
        return Ok(vec![(0.0, p)]);
    }
    // Gap i runs from the end of interval i to the start of the next one (a period on
    // for the last).
    let n = iv.len();
    let mut gap_in = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(a), Some(b)) = (iv.get(i), iv.get((i + 1) % n)) else {
            continue;
        };
        let end = if i + 1 == n { b.0 + p } else { b.0 };
        gap_in.push(covered(0.5 * (a.1 + end))?);
    }
    let Some(start) = gap_in.iter().position(|c| !c) else {
        return Ok(vec![(0.0, p)]);
    };
    // Walk the circle from the interval after the first uncovered gap, a period on once
    // the index wraps, joining intervals across covered gaps.
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut open = false;
    for k in start + 1..=start + n {
        let i = k % n;
        let shift = if k >= n { p } else { 0.0 };
        let Some(&(lo, hi)) = iv.get(i) else { continue };
        let (lo, hi) = (lo + shift, hi + shift);
        match out.last_mut() {
            Some(l) if open => l.1 = hi,
            _ => out.push((lo, hi)),
        }
        open = gap_in.get(i).copied().unwrap_or(false);
    }
    if out.first().is_some_and(|f| f.0 >= p) {
        for r in &mut out {
            r.0 -= p;
            r.1 -= p;
        }
    }
    Ok(out)
}

/// [`cover`] on the segment `[-π/2, π/2]`.
fn cover_line(
    raw: Vec<(f64, f64)>,
    mut covered: impl FnMut(f64) -> KernelResult<bool>,
) -> KernelResult<Vec<(f64, f64)>> {
    let (dlo, dhi) = (-FRAC_PI_2, FRAC_PI_2);
    let iv = merge(raw);
    let (Some(&first), Some(&last)) = (iv.first(), iv.last()) else {
        return Ok(vec![(dlo, dhi)]);
    };
    let mut out: Vec<(f64, f64)> = Vec::new();
    let lo = if first.0 > dlo && covered(0.5 * (dlo + first.0))? {
        dlo
    } else {
        first.0
    };
    out.push((lo, first.1));
    for &(a, b) in iv.iter().skip(1) {
        let Some(l) = out.last_mut() else { break };
        if covered(0.5 * (l.1 + a))? {
            l.1 = b;
        } else {
            out.push((a, b));
        }
    }
    if last.1 < dhi
        && covered(0.5 * (last.1 + dhi))?
        && let Some(l) = out.last_mut()
    {
        l.1 = dhi;
    }
    Ok(out)
}

/// Sorts intervals and joins the overlapping ones.
fn merge(mut iv: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(iv.len());
    for (lo, hi) in iv {
        match out.last_mut() {
            Some(l) if lo <= l.1 => l.1 = l.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

/// The range of `cos` over `[a, b]` (`a <= b`).
fn cos_range(a: f64, b: f64) -> (f64, f64) {
    if b - a >= TAU {
        return (-1.0, 1.0);
    }
    let (ca, cb) = (a.cos(), b.cos());
    let (mut lo, mut hi) = (ca.min(cb), ca.max(cb));
    if (a / TAU).ceil() * TAU <= b {
        hi = 1.0;
    }
    if ((a - PI) / TAU).ceil() * TAU + PI <= b {
        lo = -1.0;
    }
    (lo, hi)
}

/// The range of the product of two ranges.
fn product((a0, a1): (f64, f64), (b0, b1): (f64, f64)) -> (f64, f64) {
    let c = [a0 * b0, a0 * b1, a1 * b0, a1 * b1];
    let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo, hi)
}

/// The box of `S([u0, u1] × [v0, v1])` (see the module documentation).
fn rectangle_box(
    surface: &Surface,
    meridian: Meridian,
    (u0, u1): (f64, f64),
    (v0, v1): (f64, f64),
) -> Aabb {
    let c = cos_range(v0, v1);
    let s = cos_range(v0 - FRAC_PI_2, v1 - FRAC_PI_2);
    let (rho, h) = match meridian {
        Meridian::Torus { big, a } => ((big + a * c.0, big + a * c.1), (a * s.0, a * s.1)),
        Meridian::Sphere { r } => ((r * c.0, r * c.1), (r * s.0, r * s.1)),
        Meridian::Line { .. } => ((0.0, 0.0), (0.0, 0.0)),
    };
    let f = surface.frame();
    let (o, x, y, z) = (f.origin(), f.x(), f.y(), f.z());
    let axis = |oi: f64, xi: f64, yi: f64, zi: f64| -> (f64, f64) {
        let amp = xi.hypot(yi);
        let phi = yi.atan2(xi);
        let g = cos_range(u0 - phi, u1 - phi);
        let g = (amp * g.0, amp * g.1);
        let radial = product(rho, g);
        let height = product(h, (zi, zi));
        (oi + radial.0 + height.0, oi + radial.1 + height.1)
    };
    let ax = axis(o.x, x.x, y.x, z.x);
    let ay = axis(o.y, x.y, y.y, z.y);
    let az = axis(o.z, x.z, y.z, z.z);
    Aabb {
        min: Point3::new(ax.0, ay.0, az.0),
        max: Point3::new(ax.1, ay.1, az.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_range_widens_where_a_multiple_of_pi_lies_inside() {
        // [0.1, 1]: cos falls from cos 0.1 to cos 1, no extreme inside.
        let (lo, hi) = cos_range(0.1, 1.0);
        assert!((lo - 1f64.cos()).abs() < 1e-15 && (hi - 0.1f64.cos()).abs() < 1e-15);
        // [-1, 1] holds 0: the top is 1. [3, 4] holds π: the bottom is -1.
        assert_eq!(cos_range(-1.0, 1.0).1, 1.0);
        assert_eq!(cos_range(3.0, 4.0).0, -1.0);
        assert_eq!(cos_range(0.0, 7.0), (-1.0, 1.0));
    }

    #[test]
    fn a_covered_gap_joins_its_neighbours_on_the_circle() {
        // Boundary at [0.5, 1] and [3, 3.5]: the gap (1, 3) is inside, the gap (3.5, 6.78)
        // is not, so the face covers [0.5, 3.5].
        let r = cover(vec![(0.5, 1.0), (3.0, 3.5)], Some(TAU), |m| Ok(m < 3.0)).unwrap();
        assert_eq!(r, vec![(0.5, 3.5)]);
        // The other way round: the face covers [3, 2π + 1] across the seam.
        let r = cover(vec![(0.5, 1.0), (3.0, 3.5)], Some(TAU), |m| Ok(m > 3.0)).unwrap();
        assert_eq!(r, vec![(3.0, TAU + 1.0)]);
        // Both gaps inside: the whole turn.
        let r = cover(vec![(0.5, 1.0), (3.0, 3.5)], Some(TAU), |_| Ok(true)).unwrap();
        assert_eq!(r, vec![(0.0, TAU)]);
    }

    /// The faces of `b` on a torus or sphere, each with its tight box.
    fn curved_boxes(b: &Body) -> Vec<Aabb> {
        b.faces()
            .into_iter()
            .filter_map(|f| {
                let (s, _) = b.face_surface(f).unwrap()?;
                matches!(s, Surface::Torus(_) | Surface::Sphere(_))
                    .then(|| revolution_face_box(b, f, &s).unwrap())
            })
            .collect()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < chamfera_geom::LINEAR_TOLERANCE
    }

    #[test]
    fn a_quarter_torus_face_is_bounded_by_its_quarter_not_the_whole_torus() {
        use crate::{Line2, Profile, revolve};
        use chamfera_geom::{Frame, Point2, Vec2};
        // A circle r = 2 at x = 10 in the XZ plane, turned a quarter about Z: the tube
        // face has ρ in [8, 12] and h in [-2, 2] over a quarter turn from +x, so its box
        // is x in [0, 12], z in [-2, 2] and y over [0, 12] on the side it turned to. The
        // whole torus box would be ±12 in x and y.
        let p = Profile::circle(Point2::new(10.0, 0.0), 2.0);
        let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
        let b = revolve(&Frame::XZ, &p, &axis, FRAC_PI_2).unwrap();
        let boxes = curved_boxes(&b);
        assert_eq!(boxes.len(), 1);
        let bx = boxes[0];
        assert!(near(bx.min.x, 0.0) && near(bx.max.x, 12.0), "{bx:?}");
        assert!(near(bx.min.z, -2.0) && near(bx.max.z, 2.0), "{bx:?}");
        assert!(near(bx.max.y - bx.min.y, 12.0), "{bx:?}");
        assert!(near(bx.min.y, 0.0) || near(bx.max.y, 0.0), "{bx:?}");
    }

    #[test]
    fn a_half_sphere_face_is_bounded_by_its_half() {
        use crate::{Line2, Profile, ProfileEdge, revolve};
        use chamfera_geom::{Frame, Point2, Vec2};
        // A half disc r = 5 turned half a turn about Z: its meridian runs pole to pole,
        // so the face is half the sphere, x in [-5, 5], z in [-5, 5] and y over [0, 5] on one side.
        let p = Profile::new(
            vec![
                ProfileEdge::Arc {
                    centre: Point2::ORIGIN,
                    radius: 5.0,
                    start_angle: -FRAC_PI_2,
                    sweep: PI,
                },
                ProfileEdge::line(Point2::new(0.0, 5.0), Point2::new(0.0, -5.0)),
            ],
            vec![],
        );
        let axis = Line2::new(Point2::ORIGIN, Vec2::Y).unwrap();
        let b = revolve(&Frame::XZ, &p, &axis, PI).unwrap();
        let boxes = curved_boxes(&b);
        assert_eq!(boxes.len(), 1);
        let bx = boxes[0];
        assert!(near(bx.min.x, -5.0) && near(bx.max.x, 5.0), "{bx:?}");
        assert!(near(bx.min.z, -5.0) && near(bx.max.z, 5.0), "{bx:?}");
        assert!(near(bx.max.y - bx.min.y, 5.0), "{bx:?}");
    }
}

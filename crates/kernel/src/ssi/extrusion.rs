//! Intersections with surfaces of linear extrusion and with B-spline curves (rung K10 on
//! K6).
//!
//! The extrusion `S(u, v) = o + c(u) + v z` (`extrusion.rs`) is a cylinder over the plane
//! curve `c`: a point lies on it exactly when its projection along `z` into the frame's
//! plane lies on `c`. So every question about it with a partner that is itself invariant
//! along `z` is a question in the plane (Patrikalakis and Maekawa, *Shape Interrogation
//! for Computer Aided Design and Manufacturing*, §5.8, reduce intersections to lower
//! dimension the same way):
//!
//! | Pair | Result |
//! |---|---|
//! | plane parallel to `z` | rulings `c(u_i) + v z` at the roots of the plane's trace line against `c` (tangent where `c` touches the line); coincident when `c` lies on the line |
//! | plane perpendicular to `z` | the curve itself at the plane's height (the same B-spline, moved) |
//! | oblique plane | the section `c(u) + v(u) z` with `v(u) = (k - n₂ · c(u)) / (n · z)`, affine in `c(u)`; an affine image of a NURBS curve is the NURBS curve of the mapped control points with the same weights and knots (The NURBS Book, §4.2, affine invariance), so the section is exact |
//! | cylinder with its axis along `z` | rulings at the crossings of `c` with the circle the cylinder projects to; coincident when `c` lies on it |
//! | extrusion along the same direction | rulings at the crossings of the two plane curves (subdivision and Newton, `spline.rs`); coincident when one curve lies on the other |
//! | sphere or torus clear of the extrusion | empty (bounding sphere) |
//! | anything else (a cone, a cylinder or extrusion along another direction, a sphere or torus that reaches it) | [`UnsupportedIntersection::Spline`]: a space curve that needs surface marching |
//!
//! Curve-surface intersection against an extrusion projects the curve into the plane too:
//! a line becomes a line (a ruling when it runs along `z`, which lies on the surface or
//! misses it), a circle or ellipse an ellipse, a spline a spline, and the plane curves
//! meet by `spline.rs`. The curve touches the surface exactly where the plane curves
//! touch: the tangent plane holds `z`, so a tangent lies in it when its projection is
//! along `c'`. B-spline curves against the analytic surfaces are roots of the surface's
//! signed distance along the curve (`spline.rs`).

use super::spline::{Track, all_samples, field_roots, pair_roots};
use super::{
    CurveSurfaceIntersection, CurveSurfacePoint, IntersectionCurve, ON_SURFACE_SAMPLES,
    SurfaceIntersection, UnsupportedIntersection, beyond_range, bounding_spheres_apart, parallel,
    perpendicular,
};
use crate::{Contact, Curve, Cylinder, Extrusion, KernelError, KernelResult, Surface};
use chamfera_geom::{
    ANGULAR_TOLERANCE, BSplineCurve2, BSplineCurve3, Frame, LINEAR_TOLERANCE, Point2, Point3, Vec2,
    Vec3,
};

fn contact(tangent: bool) -> Contact {
    if tangent {
        Contact::Tangent
    } else {
        Contact::Crossing
    }
}

/// The ruling of `e` at curve parameter `u`, a whole line along the extrusion.
fn ruling(e: &Extrusion, u: f64, tangent: bool) -> KernelResult<IntersectionCurve> {
    super::line_curve(e.point_at(u, 0.0), e.frame().z(), false, contact(tangent))
}

/// An extrusion against any surface (see the module documentation).
pub(crate) fn extrusion_surface(
    e: &Extrusion,
    other: &Surface,
) -> KernelResult<SurfaceIntersection> {
    let z = e.frame().z();
    match other {
        Surface::Plane(p) => plane_extrusion(p, e),
        Surface::Cylinder(c) if parallel(c.frame().z(), z) => cylinder_extrusion(c, e),
        Surface::Extrusion(f) if parallel(f.frame().z(), z) => extrusion_extrusion(e, f),
        Surface::Sphere(_) | Surface::Torus(_)
            if bounding_spheres_apart(other, &Surface::Extrusion(e.clone())) =>
        {
            Ok(SurfaceIntersection::Empty)
        }
        _ => Ok(SurfaceIntersection::Unsupported(
            UnsupportedIntersection::Spline,
        )),
    }
}

/// A plane against an extrusion.
pub(crate) fn plane_extrusion(plane: &Frame, e: &Extrusion) -> KernelResult<SurfaceIntersection> {
    let f = e.frame();
    let (n, z) = (plane.z(), f.z());
    let c = e.curve();
    if parallel(n, z) {
        // The curve at the plane's height.
        let h = (plane.origin() - f.origin()).dot(z);
        let moved = f
            .with_origin(f.origin() + z * h)
            .ok_or_else(|| beyond_range("curve"))?;
        let curve = c.to_3d(&moved)?;
        let range = curve.domain();
        return Ok(SurfaceIntersection::Curves {
            curves: vec![IntersectionCurve {
                curve: Curve::BSpline(curve),
                range,
                contact: Contact::Crossing,
            }],
            points: vec![],
        });
    }
    let n2 = Vec2::new(n.dot(f.x()), n.dot(f.y()));
    let k = n.dot(plane.origin() - f.origin());
    if perpendicular(n, z) {
        // The trace line n₂ · p = k in the curve's plane; |n₂| = 1 up to the angular
        // tolerance, so n₂ · p - k is a signed distance.
        let len = n2.length();
        let Some(m) = n2.normalized() else {
            return Ok(SurfaceIntersection::Empty);
        };
        let kk = k / len;
        let sd = |p: Point2| m.dot(p - Point2::ORIGIN) - kk;
        let (lo, hi) = c.domain();
        if all_samples(lo, hi, 4 * ON_SURFACE_SAMPLES, |u| {
            sd(c.point_at(u)).abs() <= LINEAR_TOLERANCE
        }) {
            return Ok(SurfaceIntersection::Coincident);
        }
        let g = Vec3::new(m.x, m.y, 0.0);
        let field = move |p: Point3| (g.dot(p - Point3::ORIGIN) - kk, g);
        let c3 = c.to_3d(&Frame::XY)?;
        let mut curves = Vec::new();
        for r in field_roots(&c3, lo, hi, &field, f64::INFINITY)? {
            curves.push(ruling(e, r.t, r.tangent)?);
        }
        return Ok(SurfaceIntersection::from_parts(curves, vec![]));
    }
    // Oblique: map every control point P to o + P_x x + P_y y + v(P) z.
    let nz = n.dot(z);
    let pts: Vec<Point3> = c
        .control_points()
        .into_iter()
        .map(|p| {
            let v = (k - n2.dot(p - Point2::ORIGIN)) / nz;
            f.from_plane(p) + z * v
        })
        .collect();
    if pts.iter().any(|p| !p.is_valid()) {
        return Err(beyond_range("curve"));
    }
    let curve = BSplineCurve3::new(
        c.degree(),
        c.knots().to_vec(),
        pts,
        Some(c.weights().to_vec()),
    )?;
    let range = curve.domain();
    Ok(SurfaceIntersection::Curves {
        curves: vec![IntersectionCurve {
            curve: Curve::BSpline(curve),
            range,
            contact: Contact::Crossing,
        }],
        points: vec![],
    })
}

/// A cylinder whose axis runs along the extrusion: rulings where the curve crosses the
/// cylinder's circle.
fn cylinder_extrusion(cyl: &Cylinder, e: &Extrusion) -> KernelResult<SurfaceIntersection> {
    let f = e.frame();
    let m = f.to_plane(cyl.frame().origin());
    let r = cyl.radius();
    let c = e.curve();
    let (lo, hi) = c.domain();
    if all_samples(lo, hi, 4 * ON_SURFACE_SAMPLES, |u| {
        (c.point_at(u).distance(m) - r).abs() <= LINEAR_TOLERANCE
    }) {
        return Ok(SurfaceIntersection::Coincident);
    }
    let centre = Point3::new(m.x, m.y, 0.0);
    let field = move |p: Point3| {
        let d = Vec3::new(p.x - centre.x, p.y - centre.y, 0.0);
        (d.length() - r, d.normalized().unwrap_or(Vec3::X))
    };
    let c3 = c.to_3d(&Frame::XY)?;
    let mut curves = Vec::new();
    for root in field_roots(&c3, lo, hi, &field, r)? {
        curves.push(ruling(e, root.t, root.tangent)?);
    }
    Ok(SurfaceIntersection::from_parts(curves, vec![]))
}

/// The curve of `g` in the plane coordinates of `e`'s frame (both extrude along the
/// same direction, so this is the projection of `g`'s surface into `e`'s plane).
fn curve_in(e: &Extrusion, g: &Extrusion) -> KernelResult<BSplineCurve2> {
    let (fe, fg) = (e.frame(), g.frame());
    let c = g.curve();
    let pts: Vec<Point2> = c
        .control_points()
        .into_iter()
        .map(|p| fe.to_plane(fg.from_plane(p)))
        .collect();
    Ok(BSplineCurve2::new(
        c.degree(),
        c.knots().to_vec(),
        pts,
        Some(c.weights().to_vec()),
    )?)
}

/// True when every sample of `a` lies on `b`.
fn lies_on(a: &BSplineCurve2, b: &BSplineCurve2) -> bool {
    let (lo, hi) = a.domain();
    all_samples(lo, hi, 4 * ON_SURFACE_SAMPLES, |u| {
        b.closest_point(a.point_at(u)).distance <= LINEAR_TOLERANCE
    })
}

/// Two extrusions along the same direction.
fn extrusion_extrusion(e: &Extrusion, g: &Extrusion) -> KernelResult<SurfaceIntersection> {
    let a = e.curve().clone();
    let b = curve_in(e, g)?;
    if lies_on(&a, &b) || lies_on(&b, &a) {
        return Ok(SurfaceIntersection::Coincident);
    }
    let (ad, bd) = (a.domain(), b.domain());
    let roots = pair_roots(&Track::Nurbs(a), ad, &Track::Nurbs(b), bd)?;
    let mut curves = Vec::new();
    for r in roots {
        curves.push(ruling(e, r.s, r.tangent)?);
    }
    Ok(SurfaceIntersection::from_parts(curves, vec![]))
}

// ------------------------------------------------------------ curve against surface

fn points(curve: &Curve, hits: impl IntoIterator<Item = (f64, bool)>) -> CurveSurfaceIntersection {
    let mut out: Vec<CurveSurfacePoint> = hits
        .into_iter()
        .map(|(t, tangent)| CurveSurfacePoint {
            t,
            point: curve.point_at(t),
            contact: contact(tangent),
        })
        .collect();
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    CurveSurfaceIntersection::Points(out)
}

/// The parameter range of the plane line `p + τ d` inside the box `lo..hi` grown by the
/// tolerance, within `[t0, t1]`.
fn clip_line(p: Point2, d: Vec2, lo: Point2, hi: Point2, t0: f64, t1: f64) -> Option<(f64, f64)> {
    let t = LINEAR_TOLERANCE;
    let (mut a, mut b) = (t0, t1);
    for (o, dd, l, h) in [
        (p.x, d.x, lo.x - t, hi.x + t),
        (p.y, d.y, lo.y - t, hi.y + t),
    ] {
        if dd == 0.0 {
            if o < l || o > h {
                return None;
            }
            continue;
        }
        let (x, y) = ((l - o) / dd, (h - o) / dd);
        a = a.max(x.min(y));
        b = b.min(x.max(y));
    }
    (a.is_finite() && b.is_finite() && a <= b).then_some((a, b))
}

/// Where `curve` over `[t0, t1]` meets the extrusion `e`.
pub(crate) fn curve_extrusion(
    curve: &Curve,
    t0: f64,
    t1: f64,
    e: &Extrusion,
) -> KernelResult<CurveSurfaceIntersection> {
    let f = e.frame();
    let c = e.curve();
    let on = |p: Point3| c.closest_point(f.to_plane(p)).distance <= LINEAR_TOLERANCE;
    let target = Track::Nurbs(c.clone());
    let cd = c.domain();
    match curve {
        Curve::Line(_) | Curve::Segment(_) => {
            // Affine in t: p(t) = p0 + t d.
            let (q0, q1) = (curve.point_at(0.0), curve.point_at(1.0));
            let d3 = q1 - q0;
            let p = f.to_plane(q0);
            let d = f.to_plane(q1) - p;
            if d.length() <= ANGULAR_TOLERANCE * d3.length() {
                // Along the extrusion: a ruling or nothing.
                return Ok(if on(q0) {
                    CurveSurfaceIntersection::OnSurface { t0, t1 }
                } else {
                    CurveSurfaceIntersection::Points(vec![])
                });
            }
            let (lo, hi) = c.hull_box();
            let Some((a, b)) = clip_line(p, d, lo, hi, t0, t1) else {
                return Ok(CurveSurfaceIntersection::Points(vec![]));
            };
            let track = Track::Projected { curve, frame: f };
            let roots = pair_roots(&track, (a, b), &target, cd)?;
            Ok(points(
                curve,
                roots
                    .into_iter()
                    .filter(|r| r.s >= t0 && r.s <= t1)
                    .map(|r| (r.s, r.tangent)),
            ))
        }
        _ => {
            if !(t0.is_finite() && t1.is_finite() && t0 <= t1) {
                return Err(KernelError::InvalidInput(
                    "the curve range `t0..t1` must be finite, with t0 <= t1".into(),
                ));
            }
            if all_samples(t0, t1, ON_SURFACE_SAMPLES - 1, |t| on(curve.point_at(t))) {
                return Ok(CurveSurfaceIntersection::OnSurface { t0, t1 });
            }
            if t0 == t1 {
                return Ok(if on(curve.point_at(t0)) {
                    points(curve, [(t0, true)])
                } else {
                    CurveSurfaceIntersection::Points(vec![])
                });
            }
            let roots = match curve {
                Curve::BSpline(s) => {
                    let plane = Track::Nurbs(s.to_plane(f)?);
                    pair_roots(&plane, (t0, t1), &target, cd)?
                }
                _ => {
                    let track = Track::Projected { curve, frame: f };
                    pair_roots(&track, (t0, t1), &target, cd)?
                }
            };
            Ok(points(
                curve,
                roots
                    .into_iter()
                    .filter(|r| r.s >= t0 && r.s <= t1)
                    .map(|r| (r.s, r.tangent)),
            ))
        }
    }
}

/// Where the B-spline `c` over `[t0, t1]` meets an analytic surface (not an
/// extrusion): roots of the surface's signed distance along the curve.
pub(crate) fn spline_surface(
    c: &BSplineCurve3,
    t0: f64,
    t1: f64,
    surface: &Surface,
) -> KernelResult<CurveSurfaceIntersection> {
    let curve = Curve::BSpline(c.clone());
    let (lo, hi) = c.domain();
    if !(t0.is_finite() && t1.is_finite() && t0 <= t1) {
        return Err(KernelError::InvalidInput(
            "the curve range `t0..t1` must be finite, with t0 <= t1".into(),
        ));
    }
    let (t0, t1) = (t0.max(lo), t1.min(hi));
    if t0.is_nan() || t1.is_nan() || t0 > t1 {
        return Ok(CurveSurfaceIntersection::Points(vec![]));
    }
    if all_samples(t0, t1, ON_SURFACE_SAMPLES - 1, |t| {
        surface.distance(c.point_at(t)) <= LINEAR_TOLERANCE
    }) {
        return Ok(CurveSurfaceIntersection::OnSurface { t0, t1 });
    }
    if t0 == t1 {
        return Ok(CurveSurfaceIntersection::Points(vec![]));
    }
    let fr = *surface.frame();
    let field = move |p: Point3| -> (f64, Vec3) {
        match surface {
            Surface::Plane(_) => ((p - fr.origin()).dot(fr.z()), fr.z()),
            Surface::Cylinder(k) => {
                let l = fr.to_local(p);
                let radial = Vec3::new(l.x, l.y, 0.0);
                let n = fr.vector_to_world(radial.normalized().unwrap_or(Vec3::X));
                (radial.length() - k.radius(), n)
            }
            Surface::Sphere(s) => {
                let d = p - fr.origin();
                (d.length() - s.radius(), d.normalized().unwrap_or(Vec3::Z))
            }
            _ => match surface.closest_point(p) {
                Ok(q) => {
                    let n = surface.normal_at(q.u, q.v);
                    let d = p - q.point;
                    let s = if d.dot(n) < 0.0 { -1.0 } else { 1.0 };
                    (s * q.distance, n)
                }
                Err(_) => (f64::INFINITY, Vec3::Z),
            },
        }
    };
    // The smallest radius of curvature of the surface (0: unknown, never flat).
    let level_radius = match surface {
        Surface::Plane(_) => f64::INFINITY,
        Surface::Cylinder(k) => k.radius(),
        Surface::Sphere(s) => s.radius(),
        Surface::Torus(t) => t.minor_radius(),
        Surface::Cone(_) | Surface::Extrusion(_) => 0.0,
    };
    let roots = field_roots(c, t0, t1, &field, level_radius)?;
    Ok(points(&curve, roots.into_iter().map(|r| (r.t, r.tangent))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Line3, intersect_curve_surface, intersect_surfaces};
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2};

    /// The ellipse x²/a² + y²/b² = 1, turned by `angle`, as one closed rational quadratic
    /// (the nine-point circle of The NURBS Book §7.3 with its points scaled and turned),
    /// extruded along z from the XY plane.
    fn elliptic(a: f64, b: f64, angle: f64) -> Extrusion {
        let w = FRAC_1_SQRT_2;
        let (s, c) = angle.sin_cos();
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
        let curve = BSplineCurve2::new(
            2,
            vec![
                0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
            ],
            corners
                .iter()
                .map(|(x, y)| {
                    let (x, y) = (a * x, b * y);
                    Point2::new(c * x - s * y, s * x + c * y)
                })
                .collect(),
            Some(vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]),
        )
        .unwrap();
        Extrusion::new(Frame::XY, curve).unwrap()
    }

    fn on_ellipse(p: Point3, a: f64, b: f64) -> bool {
        (p.x * p.x / (a * a) + p.y * p.y / (b * b) - 1.0).abs() < 1e-9
    }

    /// The rulings of a result as their (x, y), sorted.
    fn feet(r: &SurfaceIntersection) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = r
            .curves()
            .iter()
            .map(|c| match &c.curve {
                Curve::Line(l) => {
                    assert!(l.direction().cross(Vec3::Z).length() < 1e-12);
                    (l.origin().x, l.origin().y)
                }
                other => panic!("a ruling expected, got {other:?}"),
            })
            .collect();
        out.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.total_cmp(&q.1)));
        out
    }

    #[test]
    fn a_plane_along_the_extrusion_cuts_rulings_at_the_curves_roots() {
        // x = 6 against x²/100 + y²/25 = 1: y = ±5 √(1 - 0.36) = ±4.
        let e = Surface::Extrusion(elliptic(10.0, 5.0, 0.0));
        let plane = Surface::Plane(Frame::YZ.with_origin(Point3::new(6.0, 0.0, 0.0)).unwrap());
        let r = intersect_surfaces(&plane, &e).unwrap();
        let f = feet(&r);
        assert_eq!(f.len(), 2, "{r:?}");
        assert!((f[0].0 - 6.0).abs() < 1e-9 && (f[0].1 + 4.0).abs() < 1e-9);
        assert!((f[1].0 - 6.0).abs() < 1e-9 && (f[1].1 - 4.0).abs() < 1e-9);
        // y = 5 touches it along the ruling through (0, 5); y = 5.1 misses.
        let touch = Surface::Plane(Frame::XZ.with_origin(Point3::new(0.0, 5.0, 0.0)).unwrap());
        let r = intersect_surfaces(&e, &touch).unwrap();
        assert!(matches!(r, SurfaceIntersection::Tangent { .. }), "{r:?}");
        assert!(
            feet(&r)
                .iter()
                .all(|p| p.0.abs() < 1e-6 && (p.1 - 5.0).abs() < 1e-9)
        );
        let miss = Surface::Plane(Frame::XZ.with_origin(Point3::new(0.0, 5.1, 0.0)).unwrap());
        assert_eq!(
            intersect_surfaces(&e, &miss).unwrap(),
            SurfaceIntersection::Empty
        );
    }

    #[test]
    fn a_plane_across_the_extrusion_cuts_the_curve_and_an_oblique_one_an_exact_section() {
        let e = elliptic(10.0, 5.0, 0.0);
        // z = 3: the ellipse at height 3.
        let r = plane_extrusion(
            &Frame::XY.with_origin(Point3::new(0.0, 0.0, 3.0)).unwrap(),
            &e,
        )
        .unwrap();
        let [c] = r.curves() else { panic!("{r:?}") };
        for i in 0..=20 {
            let p = c.curve.point_at(i as f64 / 20.0);
            assert!(
                on_ellipse(p, 10.0, 5.0) && (p.z - 3.0).abs() < 1e-12,
                "{p:?}"
            );
        }
        // z = 5 + x/4 (normal (-1/4, 0, 1)): every point of the section is on the ellipse
        // and on the plane, exactly (an affine image of the NURBS).
        let n = Vec3::new(-0.25, 0.0, 1.0);
        let plane = Frame::from_normal(Point3::new(0.0, 0.0, 5.0), n).unwrap();
        let r = plane_extrusion(&plane, &e).unwrap();
        let [c] = r.curves() else { panic!("{r:?}") };
        assert!(matches!(c.curve, Curve::BSpline(_)));
        for i in 0..=40 {
            let p = c.curve.point_at(i as f64 / 40.0);
            assert!(on_ellipse(p, 10.0, 5.0), "{p:?}");
            assert!((p.z - (5.0 + p.x / 4.0)).abs() < 1e-12, "{p:?}");
        }
    }

    #[test]
    fn a_parallel_cylinder_and_extrusion_meet_in_rulings() {
        // r = 6 about the z axis against a = 10, b = 5: substituting y² = 36 - x² into the
        // ellipse, x²/100 + (36 - x²)/25 = 1, so
        // x² (1/100 - 1/25) = 1 - 36/25 and x² = 44/3, y² = 36 - 44/3 = 64/3.
        let e = Surface::Extrusion(elliptic(10.0, 5.0, 0.0));
        let cyl = Surface::cylinder(Frame::XY, 6.0).unwrap();
        let r = intersect_surfaces(&cyl, &e).unwrap();
        let f = feet(&r);
        assert_eq!(f.len(), 4, "{r:?}");
        for (x, y) in f {
            assert!((x * x - 44.0 / 3.0).abs() < 1e-9 && (y * y - 64.0 / 3.0).abs() < 1e-9);
        }
        // r = 5 touches it along the minor axis ends (0, ±5).
        let r = intersect_surfaces(&Surface::cylinder(Frame::XY, 5.0).unwrap(), &e).unwrap();
        assert!(matches!(r, SurfaceIntersection::Tangent { .. }), "{r:?}");
        // A cone, or a cylinder along another axis: a space curve, refused.
        let cone = Surface::cone(Frame::XY, 3.0, 0.3).unwrap();
        assert!(matches!(
            intersect_surfaces(&cone, &e).unwrap(),
            SurfaceIntersection::Unsupported(UnsupportedIntersection::Spline)
        ));
        let across = Surface::cylinder(Frame::YZ, 2.0).unwrap();
        assert!(matches!(
            intersect_surfaces(&e, &across).unwrap(),
            SurfaceIntersection::Unsupported(UnsupportedIntersection::Spline)
        ));
    }

    #[test]
    fn crossed_ellipses_meet_in_four_rulings_and_one_meets_itself_coincident() {
        // x²/100 + y²/25 = 1 and x²/25 + y²/100 = 1: subtracting, x² = y², so
        // x² (1/100 + 1/25) = 1 and x² = y² = 20.
        let a = Surface::Extrusion(elliptic(10.0, 5.0, 0.0));
        let b = Surface::Extrusion(elliptic(10.0, 5.0, FRAC_PI_2));
        let f = feet(&intersect_surfaces(&a, &b).unwrap());
        assert_eq!(f.len(), 4, "{f:?}");
        for (x, y) in f {
            assert!((x * x - 20.0).abs() < 1e-9 && (y * y - 20.0).abs() < 1e-9);
        }
        assert_eq!(
            intersect_surfaces(&a, &a).unwrap(),
            SurfaceIntersection::Coincident
        );
    }

    #[test]
    fn curves_meet_an_extrusion_where_their_projections_meet_its_curve() {
        let e = Surface::Extrusion(elliptic(10.0, 5.0, 0.0));
        // The line y = 3, z = 1 along x: x = ±10 √(1 - 9/25) = ±8.
        let l = Curve::Line(Line3::new(Point3::new(0.0, 3.0, 1.0), Vec3::X).unwrap());
        let r = intersect_curve_surface(&l, &e).unwrap();
        let ts: Vec<f64> = r.points().iter().map(|p| p.t).collect();
        assert_eq!(ts.len(), 2, "{r:?}");
        assert!((ts[0] + 8.0).abs() < 1e-9 && (ts[1] - 8.0).abs() < 1e-9);
        // A ruling lies on it; a vertical line inside misses it.
        let ruling = Curve::Line(Line3::new(Point3::new(10.0, 0.0, 0.0), Vec3::Z).unwrap());
        assert!(matches!(
            intersect_curve_surface(&ruling, &e).unwrap(),
            CurveSurfaceIntersection::OnSurface { .. }
        ));
        let inside = Curve::Line(Line3::new(Point3::new(1.0, 0.0, 0.0), Vec3::Z).unwrap());
        assert!(
            intersect_curve_surface(&inside, &e)
                .unwrap()
                .points()
                .is_empty()
        );
        // The circle r = 6 about the z axis at z = 2 crosses it four times (as above).
        let circle = Curve::Circle(
            crate::Circle3::new(
                Frame::XY.with_origin(Point3::new(0.0, 0.0, 2.0)).unwrap(),
                6.0,
            )
            .unwrap(),
        );
        let r = intersect_curve_surface(&circle, &e).unwrap();
        assert_eq!(r.points().len(), 4, "{r:?}");
        for p in r.points() {
            assert!(on_ellipse(p.point, 10.0, 5.0) && (p.point.z - 2.0).abs() < 1e-12);
        }
        // A spline against a plane: the ellipse at z = 0 against x = 6 (y = ±4).
        let s = elliptic(10.0, 5.0, 0.0).curve().to_3d(&Frame::XY).unwrap();
        let plane = Surface::Plane(Frame::YZ.with_origin(Point3::new(6.0, 0.0, 0.0)).unwrap());
        let r = intersect_curve_surface(&Curve::BSpline(s), &plane).unwrap();
        let mut ys: Vec<f64> = r.points().iter().map(|p| p.point.y).collect();
        ys.sort_by(f64::total_cmp);
        assert_eq!(ys.len(), 2, "{r:?}");
        assert!((ys[0] + 4.0).abs() < 1e-9 && (ys[1] - 4.0).abs() < 1e-9);
    }
}

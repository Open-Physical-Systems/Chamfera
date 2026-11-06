//! Tests for K6. Expected values are closed forms, derived in the comments. Every test
//! that gets curves or points back checks that they lie on both surfaces within
//! `LINEAR_TOLERANCE` (`on_both`), in both argument orders.

use super::*;
use crate::{
    Arc3, Body, Circle3, Cone, Ellipse3, Line2, Profile, ProfileEdge, Segment3, extrude, revolve,
};
use chamfera_geom::{MAX_COORDINATE, Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, PI};

const TOL: f64 = LINEAR_TOLERANCE;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn assert_close(what: &str, got: f64, want: f64) {
    assert!(close(got, want), "{what}: got {got}, want {want}");
}

fn near(p: Point3, q: Point3) -> bool {
    p.distance(q) <= 1e-9
}

fn plane(o: Point3, n: Vec3) -> Surface {
    Surface::Plane(Frame::from_normal(o, n).unwrap())
}

fn z_plane(h: f64) -> Surface {
    plane(Point3::new(0.0, 0.0, h), Vec3::Z)
}

fn frame_at(o: Point3, z: Vec3) -> Frame {
    Frame::from_normal(o, z).unwrap()
}

fn cylinder(o: Point3, z: Vec3, r: f64) -> Surface {
    Surface::cylinder(frame_at(o, z), r).unwrap()
}

fn sphere(o: Point3, r: f64) -> Surface {
    Surface::sphere(Frame::XY.with_origin(o).unwrap(), r).unwrap()
}

fn cone(o: Point3, z: Vec3, r: f64, half_angle: f64) -> Surface {
    Surface::cone(frame_at(o, z), r, half_angle).unwrap()
}

fn torus(big: f64, a: f64) -> Surface {
    Surface::torus(Frame::XY, big, a).unwrap()
}

/// Points along a returned curve: 25 over its range, an infinite end cut at ±30.
fn samples(c: &IntersectionCurve) -> Vec<Point3> {
    let lo = if c.range.0.is_finite() {
        c.range.0
    } else {
        -30.0
    };
    let hi = if c.range.1.is_finite() {
        c.range.1
    } else {
        30.0
    };
    (0..=24)
        .map(|k| c.curve.point_at(lo + (hi - lo) * k as f64 / 24.0))
        .collect()
}

fn on_both(r: &SurfaceIntersection, a: &Surface, b: &Surface) {
    let pts = r
        .curves()
        .iter()
        .flat_map(samples)
        .chain(r.points().iter().copied());
    for p in pts {
        let (da, db) = (a.distance(p), b.distance(p));
        assert!(
            da <= TOL && db <= TOL,
            "{p:?} is {da} from the first and {db} from the second surface"
        );
    }
}

fn kind(r: &SurfaceIntersection) -> u8 {
    match r {
        SurfaceIntersection::Empty => 0,
        SurfaceIntersection::Coincident => 1,
        SurfaceIntersection::Curves { .. } => 2,
        SurfaceIntersection::Tangent { .. } => 3,
        SurfaceIntersection::Unsupported(_) => 4,
    }
}

/// The intersection, checked in both orders: same kind, same number of parts, every
/// point on both surfaces.
fn both(a: &Surface, b: &Surface) -> SurfaceIntersection {
    let r = intersect_surfaces(a, b).unwrap();
    let s = intersect_surfaces(b, a).unwrap();
    on_both(&r, a, b);
    on_both(&s, a, b);
    assert_eq!(kind(&r), kind(&s), "{r:?} vs {s:?}");
    assert_eq!(r.curves().len(), s.curves().len());
    assert_eq!(r.points().len(), s.points().len());
    r
}

fn circle_of(c: &IntersectionCurve) -> Circle3 {
    match c.curve {
        Curve::Circle(c) => c,
        other => panic!("expected a circle, got {other:?}"),
    }
}

fn ellipse_of(c: &IntersectionCurve) -> Ellipse3 {
    match c.curve {
        Curve::Ellipse(e) => e,
        other => panic!("expected an ellipse, got {other:?}"),
    }
}

fn line_of(c: &IntersectionCurve) -> Line3 {
    match c.curve {
        Curve::Line(l) => l,
        other => panic!("expected a line, got {other:?}"),
    }
}

fn assert_circle(c: &IntersectionCurve, centre: Point3, axis: Vec3, r: f64) {
    let k = circle_of(c);
    assert!(near(k.center(), centre), "centre {:?}", k.center());
    assert!(
        k.normal().cross(axis).length() < 1e-12,
        "normal {:?}",
        k.normal()
    );
    assert_close("radius", k.radius(), r);
}

// ---------------------------------------------------------------- plane / plane

#[test]
fn two_planes_meet_in_a_line_or_coincide_or_miss() {
    // z = 0 and x = 2 meet in the line x = 2, z = 0 along y.
    let r = both(&z_plane(0.0), &plane(Point3::new(2.0, 5.0, 1.0), Vec3::X));
    let l = line_of(&r.curves()[0]);
    assert!(l.direction().cross(Vec3::Y).length() < 1e-12);
    assert!(close(l.origin().x, 2.0) && l.origin().z.abs() < 1e-12);
    assert_eq!(r.curves()[0].contact, Contact::Crossing);
    // An oblique pair: z = 0 and x + z = 4 meet along y through (4, 0, 0).
    let r = both(
        &z_plane(0.0),
        &plane(Point3::new(4.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0)),
    );
    let l = line_of(&r.curves()[0]);
    assert!(close(l.origin().x, 4.0) && l.origin().z.abs() < 1e-12);
    // The same plane with the normal flipped, and a parallel one.
    assert_eq!(
        both(&z_plane(3.0), &plane(Point3::new(7.0, 1.0, 3.0), -Vec3::Z)),
        SurfaceIntersection::Coincident
    );
    assert_eq!(
        both(&z_plane(3.0), &z_plane(3.0 + 2.0 * TOL)),
        SurfaceIntersection::Empty
    );
    assert_eq!(
        both(&z_plane(3.0), &z_plane(3.0 + 0.5 * TOL)),
        SurfaceIntersection::Coincident
    );
}

// ---------------------------------------------------------------- plane / sphere

#[test]
fn plane_z_3_cuts_a_sphere_of_radius_5_in_a_circle_of_radius_4() {
    // r² = 5² - 3² = 16.
    let s = sphere(Point3::ORIGIN, 5.0);
    let r = both(&z_plane(3.0), &s);
    assert_eq!(r.curves().len(), 1);
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, 3.0), Vec3::Z, 4.0);
    // Through the centre: the great circle.
    let r = both(
        &plane(Point3::new(1.0, 2.0, 3.0), Vec3::X),
        &sphere(Point3::new(1.0, 0.0, 0.0), 2.0),
    );
    assert_circle(&r.curves()[0], Point3::new(1.0, 0.0, 0.0), Vec3::X, 2.0);
}

#[test]
fn a_plane_at_the_sphere_radius_flips_consistently_between_tangent_circle_and_empty() {
    let s = sphere(Point3::ORIGIN, 5.0);
    for h in [5.0, 5.0 + 0.5 * TOL, 5.0 - 0.5 * TOL, -5.0] {
        match both(&z_plane(h), &s) {
            SurfaceIntersection::Tangent { curves, points } => {
                assert!(curves.is_empty());
                assert!(near(points[0], Point3::new(0.0, 0.0, h)), "{points:?}");
            }
            other => panic!("h = {h}: {other:?}"),
        }
    }
    assert_eq!(
        both(&z_plane(5.0 + 2.0 * TOL), &s),
        SurfaceIntersection::Empty
    );
    // 2 tol inside: a small circle, radius sqrt((r - d)(r + d)).
    // (dρ/dd = d/ρ ≈ 3500 here, so the rounding of h shows at 1e-12.)
    let h = 5.0 - 2.0 * TOL;
    let r = both(&z_plane(h), &s);
    assert!(matches!(r, SurfaceIntersection::Curves { .. }));
    let want = ((5.0 - h) * (5.0 + h)).sqrt();
    assert!((circle_of(&r.curves()[0]).radius() - want).abs() < 1e-10);
}

// ---------------------------------------------------------------- plane / cylinder

#[test]
fn a_plane_through_the_axis_cuts_two_lines_at_plus_minus_r() {
    let c = cylinder(Point3::ORIGIN, Vec3::Z, 2.0);
    let r = both(&plane(Point3::ORIGIN, Vec3::X), &c);
    assert_eq!(r.curves().len(), 2);
    let mut ys: Vec<f64> = r
        .curves()
        .iter()
        .map(|k| {
            let l = line_of(k);
            assert!(l.direction().cross(Vec3::Z).length() < 1e-12);
            assert!(l.origin().x.abs() < 1e-12);
            l.origin().y
        })
        .collect();
    ys.sort_by(f64::total_cmp);
    assert!(close(ys[0], -2.0) && close(ys[1], 2.0), "{ys:?}");
}

#[test]
fn a_plane_parallel_to_the_axis_at_r_is_tangent_and_flips_at_r_plus_minus_tolerance() {
    let c = cylinder(Point3::ORIGIN, Vec3::Z, 2.0);
    let at = |x: f64| both(&plane(Point3::new(x, 0.0, 0.0), Vec3::X), &c);
    for x in [2.0, 2.0 + 0.5 * TOL, 2.0 - 0.5 * TOL, -2.0] {
        match at(x) {
            SurfaceIntersection::Tangent { curves, .. } => {
                assert_eq!(curves.len(), 1);
                assert_eq!(curves[0].contact, Contact::Tangent);
                assert!((line_of(&curves[0]).origin().x - x).abs() < 1e-12);
            }
            other => panic!("x = {x}: {other:?}"),
        }
    }
    assert_eq!(at(2.0 + 2.0 * TOL), SurfaceIntersection::Empty);
    let r = at(2.0 - 2.0 * TOL);
    assert!(matches!(r, SurfaceIntersection::Curves { .. }));
    assert_eq!(r.curves().len(), 2);
}

#[test]
fn an_oblique_plane_cuts_an_ellipse_with_semi_axes_r_and_r_over_sin_theta() {
    // A plane through the origin at θ to the axis has n · z = sin θ.
    let r0 = 2.0;
    let c = cylinder(Point3::new(0.0, 0.0, 1.0), Vec3::Z, r0);
    for theta in [FRAC_PI_6, FRAC_PI_4, 1.2, 0.05] {
        let n = Vec3::new(0.0, -theta.cos(), theta.sin());
        let r = both(&plane(Point3::ORIGIN, n), &c);
        let e = ellipse_of(&r.curves()[0]);
        assert_close("rx", e.rx(), r0 / theta.sin());
        assert_close("ry", e.ry(), r0);
        assert!(near(e.center(), Point3::ORIGIN));
    }
    // Perpendicular: a circle of radius r.
    let r = both(&z_plane(7.0), &c);
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, 7.0), Vec3::Z, r0);
}

// ---------------------------------------------------------------- plane / cone

#[test]
fn planes_perpendicular_to_a_cone_cut_circles_or_the_apex() {
    // Apex at the origin, half-angle 30°, opening up: ρ = z tan 30°.
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_6);
    let r = both(&z_plane(3.0), &k);
    assert_circle(
        &r.curves()[0],
        Point3::new(0.0, 0.0, 3.0),
        Vec3::Z,
        3.0 * FRAC_PI_6.tan(),
    );
    let r = both(&z_plane(0.0), &k);
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert!(near(r.points()[0], Point3::ORIGIN));
    assert_eq!(both(&z_plane(-1.0), &k), SurfaceIntersection::Empty);
    // A cone narrowing upward: radius 2 at z = 0, half-angle -30°, apex at
    // v = -2 / sin(-30°) = 4, height 4 cos 30° = 2√3. The nappe lies below the apex.
    let k = cone(Point3::ORIGIN, Vec3::Z, 2.0, -FRAC_PI_6);
    assert_circle(
        &both(&z_plane(0.0), &k).curves()[0],
        Point3::ORIGIN,
        Vec3::Z,
        2.0,
    );
    assert_eq!(
        both(&z_plane(2.0 * 3f64.sqrt() + 1.0), &k),
        SurfaceIntersection::Empty
    );
}

#[test]
fn planes_through_the_apex_cut_two_generators_one_or_only_the_apex() {
    let a = FRAC_PI_6;
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, a);
    // y = 0 contains the axis: rays from the apex along (±sin a, 0, cos a).
    let r = both(&plane(Point3::ORIGIN, Vec3::Y), &k);
    assert_eq!(r.curves().len(), 2);
    for c in r.curves() {
        assert_eq!(c.range, (0.0, f64::INFINITY));
        let l = line_of(c);
        assert!(near(l.origin(), Point3::ORIGIN));
        let d = l.direction();
        assert!(close(d.z, a.cos()) && close(d.x.abs(), a.sin()) && d.y.abs() < 1e-12);
    }
    // The plane spanned by the generator (sin a, 0, cos a) and y touches along it:
    // normal (sin a, 0, cos a) × y = (-cos a, 0, sin a).
    let r = both(
        &plane(Point3::ORIGIN, Vec3::new(-a.cos(), 0.0, a.sin())),
        &k,
    );
    match &r {
        SurfaceIntersection::Tangent { curves, .. } => {
            let d = line_of(&curves[0]).direction();
            assert!((d - Vec3::new(a.sin(), 0.0, a.cos())).length() < 1e-12);
        }
        other => panic!("{other:?}"),
    }
    // A shallow plane z = 0.1 x through the apex meets only the apex.
    let r = both(&plane(Point3::ORIGIN, Vec3::new(-0.1, 0.0, 1.0)), &k);
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert!(near(r.points()[0], Point3::ORIGIN));
}

#[test]
fn an_oblique_plane_cuts_a_cone_in_the_ellipse_found_by_hand() {
    // Cone x² + y² = z² (apex at the origin, 45°), plane z = 2 + x/2. In y = 0 the cone's
    // lines x = z and x = -z meet the plane at (4, 0, 4) and (-4/3, 0, 4/3): the major
    // axis, half-length (1/2)|(16/3, 0, 8/3)| = (4/3)√5, centre (4/3, 0, 8/3). Across it,
    // at x = 4/3, z = 8/3: y² = z² - x² = 48/9, so the semi-minor axis is 4/√3.
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_4);
    let p = plane(Point3::new(0.0, 0.0, 2.0), Vec3::new(-0.5, 0.0, 1.0));
    let r = both(&p, &k);
    let e = ellipse_of(&r.curves()[0]);
    assert_close("rx", e.rx(), 4.0 / 3.0 * 5f64.sqrt());
    assert_close("ry", e.ry(), 4.0 / 3f64.sqrt());
    assert!(near(e.center(), Point3::new(4.0 / 3.0, 0.0, 8.0 / 3.0)));
    // The same plane on the other side of the apex (z = -2 - x/2) meets the other nappe.
    let q = plane(Point3::new(0.0, 0.0, -2.0), Vec3::new(-0.5, 0.0, 1.0));
    assert_eq!(both(&q, &k), SurfaceIntersection::Empty);
}

#[test]
fn parabolas_and_hyperbolas_of_a_cone_are_unsupported() {
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_4);
    // z = 2 + x is parallel to the generator (1, 0, 1).
    assert_eq!(
        both(
            &plane(Point3::new(0.0, 0.0, 2.0), Vec3::new(-1.0, 0.0, 1.0)),
            &k
        ),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::ConeParabola)
    );
    // z = x - 2: y² = 4 - 4x, z = -1 - y²/4 < 0, on the other nappe only.
    assert_eq!(
        both(
            &plane(Point3::new(0.0, 0.0, -2.0), Vec3::new(-1.0, 0.0, 1.0)),
            &k
        ),
        SurfaceIntersection::Empty
    );
    assert_eq!(
        both(&plane(Point3::new(1.0, 0.0, 0.0), Vec3::X), &k),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::ConeHyperbola)
    );
}

// ---------------------------------------------------------------- plane / torus

#[test]
fn planes_perpendicular_to_a_torus_cut_concentric_circles() {
    // R = 5, a = 1: at height h the tube gives ρ = 5 ± sqrt(1 - h²).
    let t = torus(5.0, 1.0);
    let r = both(&z_plane(0.0), &t);
    let radii: Vec<f64> = r.curves().iter().map(|c| circle_of(c).radius()).collect();
    assert_eq!(radii.len(), 2);
    assert!(close(radii[0].min(radii[1]), 4.0) && close(radii[0].max(radii[1]), 6.0));
    let r = both(&z_plane(0.6), &t);
    let mut radii: Vec<f64> = r.curves().iter().map(|c| circle_of(c).radius()).collect();
    radii.sort_by(f64::total_cmp);
    assert!(close(radii[0], 4.2) && close(radii[1], 5.8), "{radii:?}");
    // The top of the tube: one tangent circle of radius R.
    let r = both(&z_plane(1.0), &t);
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, 1.0), Vec3::Z, 5.0);
    assert_eq!(r.curves()[0].contact, Contact::Tangent);
    assert_eq!(
        both(&z_plane(1.0 + 2.0 * TOL), &t),
        SurfaceIntersection::Empty
    );
    // A spindle torus R = 1, a = 2 at h = 0: ρ = 1 + 2 = 3 and ρ = 2 - 1 = 1.
    let s = torus(1.0, 2.0);
    let mut radii: Vec<f64> = both(&z_plane(0.0), &s)
        .curves()
        .iter()
        .map(|c| circle_of(c).radius())
        .collect();
    radii.sort_by(f64::total_cmp);
    assert!(close(radii[0], 1.0) && close(radii[1], 3.0), "{radii:?}");
}

#[test]
fn a_plane_through_the_torus_axis_cuts_two_meridian_circles() {
    let t = torus(5.0, 1.0);
    let r = both(&plane(Point3::ORIGIN, Vec3::Y), &t);
    assert_eq!(r.curves().len(), 2);
    let mut xs: Vec<f64> = r
        .curves()
        .iter()
        .map(|c| {
            let k = circle_of(c);
            assert_close("radius", k.radius(), 1.0);
            k.center().x
        })
        .collect();
    xs.sort_by(f64::total_cmp);
    assert!(close(xs[0], -5.0) && close(xs[1], 5.0));
    // Other planes: unsupported, unless they miss the bounding sphere (radius 6).
    assert_eq!(
        both(&plane(Point3::ORIGIN, Vec3::new(-0.1, 0.0, 1.0)), &t),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::TorusSection)
    );
    assert_eq!(
        both(
            &plane(Point3::new(10.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 1.0)),
            &t
        ),
        SurfaceIntersection::Empty
    );
}

// ---------------------------------------------------------------- quadric pairs

#[test]
fn two_spheres_of_radius_5_with_centres_8_apart_meet_in_a_circle_of_radius_3_at_x_4() {
    // x² + ρ² = 25 and (x - 8)² + ρ² = 25 give x = 4, ρ = 3.
    let a = sphere(Point3::ORIGIN, 5.0);
    let b = sphere(Point3::new(8.0, 0.0, 0.0), 5.0);
    let r = both(&a, &b);
    assert_circle(&r.curves()[0], Point3::new(4.0, 0.0, 0.0), Vec3::X, 3.0);
    // Touching outside (centres 10 apart) and inside (radii 5 and 3, centres 2 apart).
    for (c, rb) in [(10.0, 5.0), (2.0, 3.0)] {
        let r = both(&a, &sphere(Point3::new(c, 0.0, 0.0), rb));
        assert!(matches!(r, SurfaceIntersection::Tangent { .. }), "{r:?}");
        assert!(near(r.points()[0], Point3::new(5.0, 0.0, 0.0)));
    }
    assert_eq!(
        both(&a, &sphere(Point3::new(10.0 + 2.0 * TOL, 0.0, 0.0), 5.0)),
        SurfaceIntersection::Empty
    );
    assert_eq!(
        both(&a, &sphere(Point3::ORIGIN, 5.0)),
        SurfaceIntersection::Coincident
    );
    assert_eq!(
        both(&a, &sphere(Point3::ORIGIN, 4.0)),
        SurfaceIntersection::Empty
    );
}

#[test]
fn equal_orthogonal_cylinders_meet_in_two_ellipses_through_the_four_points() {
    // Cylinders y² + z² = r² (axis x) and x² + y² = r² (axis z) meet in the planes x = ±z;
    // each ellipse has semi-axes r√2 (to (±r, 0, ±r)) and r (to (0, ±r, 0)).
    let r0 = 3.0;
    let a = cylinder(Point3::ORIGIN, Vec3::X, r0);
    let b = cylinder(Point3::ORIGIN, Vec3::Z, r0);
    let r = both(&a, &b);
    assert_eq!(r.curves().len(), 2);
    let mut ends = Vec::new();
    for c in r.curves() {
        let e = ellipse_of(c);
        assert_close("rx", e.rx(), r0 * 2f64.sqrt());
        assert_close("ry", e.ry(), r0);
        assert!(near(e.center(), Point3::ORIGIN));
        ends.push(e.point_at(0.0));
        ends.push(e.point_at(PI));
        let side = e.point_at(FRAC_PI_2);
        assert!(close(side.y.abs(), r0) && side.x.abs() < 1e-9 && side.z.abs() < 1e-9);
    }
    for want in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let p = Point3::new(want.0 * r0, 0.0, want.1 * r0);
        assert!(ends.iter().any(|q| near(*q, p)), "{p:?} not in {ends:?}");
    }
    // Oblique axes with equal radii meeting at (1, 2, 3): still two ellipses.
    let o = Point3::new(1.0, 2.0, 3.0);
    let r = both(
        &cylinder(o, Vec3::new(1.0, 0.3, 0.2), 1.5),
        &cylinder(o, Vec3::new(-0.2, 1.0, 0.5), 1.5),
    );
    assert_eq!(r.curves().len(), 2);
}

#[test]
fn parallel_cylinders_meet_in_zero_one_or_two_lines() {
    // r = 5 about the z axis and about x = 8: x = 4, y = ±3.
    let a = cylinder(Point3::ORIGIN, Vec3::Z, 5.0);
    let r = both(&a, &cylinder(Point3::new(8.0, 0.0, -3.0), -Vec3::Z, 5.0));
    let mut pts: Vec<(f64, f64)> = r
        .curves()
        .iter()
        .map(|c| {
            let l = line_of(c);
            assert!(l.direction().cross(Vec3::Z).length() < 1e-12);
            (l.origin().x, l.origin().y)
        })
        .collect();
    pts.sort_by(|p, q| p.1.total_cmp(&q.1));
    assert!(close(pts[0].0, 4.0) && close(pts[0].1, -3.0));
    assert!(close(pts[1].0, 4.0) && close(pts[1].1, 3.0));
    let r = both(&a, &cylinder(Point3::new(10.0, 0.0, 0.0), Vec3::Z, 5.0));
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert!(close(line_of(&r.curves()[0]).origin().x, 5.0));
    assert_eq!(
        both(
            &a,
            &cylinder(Point3::new(10.0 + 2.0 * TOL, 0.0, 0.0), Vec3::Z, 5.0)
        ),
        SurfaceIntersection::Empty
    );
    // Coaxial: the same surface, or concentric and apart.
    assert_eq!(
        both(&a, &cylinder(Point3::new(0.0, 0.0, 9.0), -Vec3::Z, 5.0)),
        SurfaceIntersection::Coincident
    );
    assert_eq!(
        both(&a, &cylinder(Point3::ORIGIN, Vec3::Z, 4.0)),
        SurfaceIntersection::Empty
    );
}

#[test]
fn skew_cylinders_are_empty_when_far_apart_and_unsupported_otherwise() {
    // The z axis and the line (t, 5, 0) are 5 apart, more than the radii 1 + 1.
    let a = cylinder(Point3::ORIGIN, Vec3::Z, 1.0);
    // The line (t, 0, 5) meets the z axis at (0, 0, 5).
    let b = cylinder(Point3::new(0.0, 0.0, 5.0), Vec3::X, 1.0);
    let b_skew = cylinder(Point3::new(0.0, 5.0, 0.0), Vec3::X, 1.0);
    assert_eq!(both(&a, &b_skew), SurfaceIntersection::Empty);
    assert_eq!(
        both(&a, &cylinder(Point3::new(0.0, 1.5, 0.0), Vec3::X, 1.0)),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::CylinderQuartic)
    );
    // Axes meeting but radii unequal: a quartic too.
    assert_eq!(
        both(&a, &cylinder(Point3::ORIGIN, Vec3::X, 0.5)),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::CylinderQuartic)
    );
    // b's axis meets a's: equal radii, two ellipses.
    assert_eq!(both(&a, &b).curves().len(), 2);
}

#[test]
fn coaxial_surfaces_of_revolution_meet_in_circles() {
    // Sphere r = 5 and coaxial cylinder r = 3: circles of radius 3 at z = ±4.
    let s = sphere(Point3::ORIGIN, 5.0);
    let r = both(&s, &cylinder(Point3::new(0.0, 0.0, 2.0), Vec3::Z, 3.0));
    assert_eq!(r.curves().len(), 2);
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, -4.0), Vec3::Z, 3.0);
    assert_circle(&r.curves()[1], Point3::new(0.0, 0.0, 4.0), Vec3::Z, 3.0);
    // Radius 5: tangent along the equator; a little more: nothing.
    let r = both(&s, &cylinder(Point3::ORIGIN, Vec3::Z, 5.0));
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert_circle(&r.curves()[0], Point3::ORIGIN, Vec3::Z, 5.0);
    assert_eq!(
        both(&s, &cylinder(Point3::ORIGIN, Vec3::Z, 5.0 + 2.0 * TOL)),
        SurfaceIntersection::Empty
    );
    // Cylinder r = 2 inside the 45° cone ρ = z: the circle at z = 2.
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_4);
    let r = both(&cylinder(Point3::ORIGIN, -Vec3::Z, 2.0), &k);
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, 2.0), Vec3::Z, 2.0);
    // The sphere about (0, 0, 1) through the apex: meridian ray (t/√2, t/√2) meets the
    // circle ρ² + (h - 1)² = 1 where t² - √2 t = 0: the apex and (1, 1).
    let r = both(&sphere(Point3::new(0.0, 0.0, 1.0), 1.0), &k);
    assert!(matches!(r, SurfaceIntersection::Curves { .. }));
    assert_circle(&r.curves()[0], Point3::new(0.0, 0.0, 1.0), Vec3::Z, 1.0);
    assert!(near(r.points()[0], Point3::ORIGIN));
    // Two cones with the same apex: only the apex.
    let r = both(&k, &cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_6));
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert!(near(r.points()[0], Point3::ORIGIN));
    // The same cone from a different reference circle: coincident.
    let shifted = cone(Point3::new(0.0, 0.0, 2.0), Vec3::Z, 2.0, FRAC_PI_4);
    assert_eq!(both(&k, &shifted), SurfaceIntersection::Coincident);
    // A sphere of radius R + a about a torus's centre touches its outer equator.
    let r = both(&sphere(Point3::ORIGIN, 6.0), &torus(5.0, 1.0));
    assert!(matches!(r, SurfaceIntersection::Tangent { .. }));
    assert_circle(&r.curves()[0], Point3::ORIGIN, Vec3::Z, 6.0);
}

#[test]
fn non_coaxial_curved_pairs_are_empty_when_provably_apart_or_unsupported() {
    let s = sphere(Point3::new(20.0, 0.0, 0.0), 2.0);
    let c = cylinder(Point3::ORIGIN, Vec3::Z, 3.0);
    assert_eq!(both(&s, &c), SurfaceIntersection::Empty);
    assert_eq!(
        both(&sphere(Point3::new(2.0, 0.0, 0.0), 2.0), &c),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::NonCoaxial)
    );
    assert_eq!(
        both(
            &torus(5.0, 1.0),
            &cone(Point3::new(1.0, 0.0, 0.0), Vec3::Z, 1.0, 0.5)
        ),
        SurfaceIntersection::Unsupported(UnsupportedIntersection::NonCoaxial)
    );
}

// ---------------------------------------------------------------- curve / surface

fn line(o: Point3, d: Vec3) -> Curve {
    Curve::Line(Line3::new(o, d).unwrap())
}

fn hits(c: &Curve, s: &Surface) -> Vec<CurveSurfacePoint> {
    match intersect_curve_surface(c, s).unwrap() {
        CurveSurfaceIntersection::Points(p) => {
            for h in &p {
                assert!(s.distance(h.point) <= TOL, "{h:?} is off the surface");
                assert!(
                    h.point.distance(c.point_at(h.t)) < 1e-9,
                    "{h:?} is off the curve"
                );
            }
            p
        }
        other => panic!("expected points, got {other:?}"),
    }
}

fn ts(h: &[CurveSurfacePoint]) -> Vec<f64> {
    h.iter().map(|p| p.t).collect()
}

fn assert_ts(got: &[CurveSurfacePoint], want: &[f64], contact: Contact) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.t - w).abs() < 1e-7, "t = {} vs {w}: {got:?}", g.t);
        assert_eq!(g.contact, contact, "{got:?}");
    }
}

#[test]
fn a_line_meets_a_torus_four_times_through_the_tube_and_never_along_the_axis() {
    // R = 5, a = 1: along x through the centre, at x = ±4 and ±6.
    let t = torus(5.0, 1.0);
    assert!(hits(&line(Point3::new(0.0, 0.0, -3.0), Vec3::Z), &t).is_empty());
    let h = hits(&line(Point3::ORIGIN, Vec3::X), &t);
    assert_ts(&h, &[-6.0, -4.0, 4.0, 6.0], Contact::Crossing);
    // Shifted so t = x - 1: at -7, -5, 3, 5.
    let h = hits(&line(Point3::new(1.0, 0.0, 0.0), Vec3::X), &t);
    assert_ts(&h, &[-7.0, -5.0, 3.0, 5.0], Contact::Crossing);
    // Along the top of the tube (z = 1): tangent at x = ±5.
    let h = hits(&line(Point3::new(0.0, 0.0, 1.0), Vec3::X), &t);
    assert_ts(&h, &[-5.0, 5.0], Contact::Tangent);
    assert!(hits(&line(Point3::new(0.0, 0.0, 1.0 + 2.0 * TOL), Vec3::X), &t).is_empty());
}

#[test]
fn a_line_at_the_cylinder_radius_is_tangent_and_flips_at_plus_minus_tolerance() {
    let c = cylinder(Point3::ORIGIN, Vec3::Z, 2.0);
    // Along x at height 1: t = ±sqrt(4 - y²).
    let at = |y: f64| hits(&line(Point3::new(0.0, y, 1.0), Vec3::X), &c);
    assert_ts(&at(0.0), &[-2.0, 2.0], Contact::Crossing);
    for y in [2.0, 2.0 + 0.5 * TOL, 2.0 - 0.5 * TOL] {
        assert_ts(&at(y), &[0.0], Contact::Tangent);
    }
    assert!(at(2.0 + 2.0 * TOL).is_empty());
    let h = at(2.0 - 2.0 * TOL);
    assert_eq!(h.len(), 2);
    assert!(h.iter().all(|p| p.contact == Contact::Crossing));
    // A line parallel to the axis on the cylinder lies on it.
    assert!(matches!(
        intersect_curve_surface(&line(Point3::new(2.0, 0.0, 0.0), Vec3::Z), &c).unwrap(),
        CurveSurfaceIntersection::OnSurface { .. }
    ));
    assert!(hits(&line(Point3::new(3.0, 0.0, 0.0), Vec3::Z), &c).is_empty());
}

#[test]
fn lines_and_segments_meet_planes_spheres_and_cones() {
    // Sphere r = 5, line along x through (0, 3, 0): t = ±4.
    let s = sphere(Point3::ORIGIN, 5.0);
    assert_ts(
        &hits(&line(Point3::new(0.0, 3.0, 0.0), Vec3::X), &s),
        &[-4.0, 4.0],
        Contact::Crossing,
    );
    // Segment from z = -1 to z = 3 meets z = 0 at a quarter.
    let seg = Curve::Segment(
        Segment3::new(Point3::new(1.0, 1.0, -1.0), Point3::new(1.0, 1.0, 3.0)).unwrap(),
    );
    assert_ts(&hits(&seg, &z_plane(0.0)), &[0.25], Contact::Crossing);
    assert!(hits(&seg, &z_plane(5.0)).is_empty());
    let flat = Curve::Segment(
        Segment3::new(Point3::new(1.0, 1.0, 2.0), Point3::new(4.0, -1.0, 2.0)).unwrap(),
    );
    assert_eq!(
        intersect_curve_surface(&flat, &z_plane(2.0)).unwrap(),
        CurveSurfaceIntersection::OnSurface { t0: 0.0, t1: 1.0 }
    );
    // 45° cone x² + y² = z², z >= 0: the line (t, 0, 1) at t = ±1; (t, 0, -1) meets only
    // the other nappe.
    let k = cone(Point3::ORIGIN, Vec3::Z, 0.0, FRAC_PI_4);
    assert_ts(
        &hits(&line(Point3::new(0.0, 0.0, 1.0), Vec3::X), &k),
        &[-1.0, 1.0],
        Contact::Crossing,
    );
    assert!(hits(&line(Point3::new(0.0, 0.0, -1.0), Vec3::X), &k).is_empty());
    // A generator through the apex lies on the nappe from the apex on.
    let g = line(Point3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0));
    match intersect_curve_surface(&g, &k).unwrap() {
        CurveSurfaceIntersection::OnSurface { t0, t1 } => {
            assert!((t0 - 2f64.sqrt()).abs() < 1e-9, "{t0}");
            assert!(t1 > 1e5);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn circles_arcs_and_ellipses_meet_surfaces_at_known_angles() {
    // The circle (5 cos t, 0, 5 sin t) in the XZ plane.
    let c = Curve::Circle(Circle3::new(Frame::XZ, 5.0).unwrap());
    let a = (0.6f64).asin();
    // z = 3: sin t = 3/5.
    assert_ts(&hits(&c, &z_plane(3.0)), &[a, PI - a], Contact::Crossing);
    // z = 5: tangent at the top, t = π/2, and the flips around it.
    assert_ts(&hits(&c, &z_plane(5.0)), &[FRAC_PI_2], Contact::Tangent);
    assert_ts(
        &hits(&c, &z_plane(5.0 + 0.5 * TOL)),
        &[FRAC_PI_2],
        Contact::Tangent,
    );
    assert!(hits(&c, &z_plane(5.0 + 2.0 * TOL)).is_empty());
    assert_eq!(hits(&c, &z_plane(5.0 - 2.0 * TOL)).len(), 2);
    // The cylinder ρ = 3 about z: |cos t| = 3/5.
    let b = (0.6f64).acos();
    assert_ts(
        &hits(&c, &cylinder(Point3::ORIGIN, Vec3::Z, 3.0)),
        &[b, PI - b, PI + b, 2.0 * PI - b],
        Contact::Crossing,
    );
    // The torus R = 5, a = 1: (5|cos t| - 5)² + 25 sin² t = 1 gives |cos t| = 0.98.
    let g = (0.98f64).acos();
    assert_ts(
        &hits(&c, &torus(5.0, 1.0)),
        &[g, PI - g, PI + g, 2.0 * PI - g],
        Contact::Crossing,
    );
    // A horizontal circle about (3, 0, 0) of radius 5 and the sphere r = 5 at the
    // origin: 9 + 25 + 30 cos t = 25, cos t = -0.3.
    let f = Frame::XY.with_origin(Point3::new(3.0, 0.0, 0.0)).unwrap();
    let s = sphere(Point3::ORIGIN, 5.0);
    let h = (-0.3f64).acos();
    let full = Curve::Circle(Circle3::new(f, 5.0).unwrap());
    assert_ts(&hits(&full, &s), &[h, 2.0 * PI - h], Contact::Crossing);
    // The upper half of it (t in [0, π]) meets the sphere once.
    let arc = Curve::Arc(Arc3::new(f, 5.0, 0.0, PI).unwrap());
    assert_ts(&hits(&arc, &s), &[h], Contact::Crossing);
    // A circle of radius 1 about (6, 0, 0) touches the sphere at (5, 0, 0), t = π.
    let touch = |x: f64| {
        let f = Frame::XY.with_origin(Point3::new(x, 0.0, 0.0)).unwrap();
        hits(&Curve::Circle(Circle3::new(f, 1.0).unwrap()), &s)
    };
    assert_ts(&touch(6.0), &[PI], Contact::Tangent);
    assert!(touch(6.0 + 2.0 * TOL).is_empty());
    assert_eq!(touch(6.0 - 2.0 * TOL).len(), 2);
    // The torus's outer equator lies on it.
    let eq = Curve::Circle(Circle3::new(Frame::XY, 6.0).unwrap());
    assert!(matches!(
        intersect_curve_surface(&eq, &torus(5.0, 1.0)).unwrap(),
        CurveSurfaceIntersection::OnSurface { .. }
    ));
    // The ellipse (4 cos t, 2 sin t, 0) and the plane x = 2: cos t = 1/2.
    let e = Curve::Ellipse(Ellipse3::new(Frame::XY, 4.0, 2.0).unwrap());
    assert_ts(
        &hits(&e, &plane(Point3::new(2.0, 0.0, 0.0), Vec3::X)),
        &[FRAC_PI_3, 2.0 * PI - FRAC_PI_3],
        Contact::Crossing,
    );
}

#[test]
fn bad_curve_ranges_are_errors() {
    let l = line(Point3::ORIGIN, Vec3::X);
    let s = sphere(Point3::ORIGIN, 1.0);
    assert!(intersect_curve_surface_on(&l, f64::NAN, 1.0, &s).is_err());
    assert!(intersect_curve_surface_on(&l, 2.0, 1.0, &s).is_err());
    let c = Curve::Circle(Circle3::new(Frame::XY, 1.0).unwrap());
    assert!(intersect_curve_surface_on(&c, 0.0, 7.0, &s).is_err());
    assert!(intersect_curve_surface_on(&c, 0.0, f64::INFINITY, &s).is_err());
    let seg = Curve::Segment(Segment3::new(Point3::ORIGIN, Point3::new(1.0, 0.0, 0.0)).unwrap());
    assert!(intersect_curve_surface_on(&seg, 0.0, f64::INFINITY, &s).is_err());
}

// ---------------------------------------------------------------- point in solid

#[test]
fn points_are_classified_against_a_box() {
    let b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 20.0, 30.0)).unwrap();
    let at = |x, y, z| classify_point(&b, Point3::new(x, y, z)).unwrap();
    assert_eq!(at(5.0, 10.0, 15.0), PointInSolid::Inside);
    assert_eq!(at(0.5, 19.5, 29.5), PointInSolid::Inside);
    assert_eq!(at(15.0, 10.0, 15.0), PointInSolid::Outside);
    assert_eq!(at(-1.0, -1.0, -1.0), PointInSolid::Outside);
    assert_eq!(at(5.0, 10.0, 30.0 + 2.0 * TOL), PointInSolid::Outside);
    assert_eq!(at(5.0, 10.0, 30.0 - 2.0 * TOL), PointInSolid::Inside);
    // On a face, an edge and a vertex.
    assert_eq!(at(5.0, 10.0, 0.0), PointInSolid::OnBoundary);
    assert_eq!(at(0.0, 0.0, 12.0), PointInSolid::OnBoundary);
    assert_eq!(at(10.0, 20.0, 30.0), PointInSolid::OnBoundary);
    // In a face's plane but outside the face.
    assert_eq!(at(5.0, 25.0, 0.0), PointInSolid::Outside);
}

#[test]
fn points_are_classified_against_a_cylinder() {
    // r = 5, h = 10 from extruding a circle on XY.
    let b = extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap();
    let at = |x, y, z| classify_point(&b, Point3::new(x, y, z)).unwrap();
    assert_eq!(at(0.0, 0.0, 5.0), PointInSolid::Inside);
    assert_eq!(at(3.0, -2.0, 9.0), PointInSolid::Inside);
    assert_eq!(at(6.0, 0.0, 5.0), PointInSolid::Outside);
    assert_eq!(at(0.0, 0.0, 11.0), PointInSolid::Outside);
    assert_eq!(at(3.6, 3.6, 5.0), PointInSolid::Outside); // ρ = 5.09
    assert_eq!(at(0.0, -5.0, 5.0), PointInSolid::OnBoundary);
    assert_eq!(at(1.0, 1.0, 10.0), PointInSolid::OnBoundary);
    // On the rim, on the seam, at a seam vertex.
    assert_eq!(at(0.0, 5.0, 10.0), PointInSolid::OnBoundary);
    assert_eq!(at(5.0, 0.0, 4.0), PointInSolid::OnBoundary);
    assert_eq!(at(5.0, 0.0, 0.0), PointInSolid::OnBoundary);
}

#[test]
fn points_are_classified_against_a_sphere() {
    // A half disc of radius 3 revolved about the sketch's y axis: a ball about the
    // origin with poles (0, ±3, 0).
    let r = 3.0;
    let p = Profile::new(
        vec![
            ProfileEdge::Arc {
                centre: Point2::ORIGIN,
                radius: r,
                start_angle: -FRAC_PI_2,
                sweep: PI,
            },
            ProfileEdge::line(Point2::new(0.0, r), Point2::new(0.0, -r)),
        ],
        vec![],
    );
    let b = revolve(
        &Frame::XY,
        &p,
        &Line2::new(Point2::ORIGIN, Vec2::Y).unwrap(),
        2.0 * PI,
    )
    .unwrap();
    let at = |x, y, z| classify_point(&b, Point3::new(x, y, z)).unwrap();
    assert_eq!(at(0.0, 0.0, 0.0), PointInSolid::Inside);
    assert_eq!(at(1.0, 1.0, 1.0), PointInSolid::Inside);
    assert_eq!(at(0.0, 2.9, 0.0), PointInSolid::Inside);
    assert_eq!(at(2.0, 2.0, 2.0), PointInSolid::Outside); // |p| = 2√3 > 3
    assert_eq!(at(0.0, 4.0, 0.0), PointInSolid::Outside);
    assert_eq!(at(-3.0, 0.0, 0.0), PointInSolid::OnBoundary);
    assert_eq!(at(0.0, 0.0, 3.0), PointInSolid::OnBoundary);
    // The poles are vertices.
    assert_eq!(at(0.0, 3.0, 0.0), PointInSolid::OnBoundary);
    assert_eq!(at(0.0, -3.0, 0.0), PointInSolid::OnBoundary);
    assert_eq!(at(0.0, 0.0, 3.0 + 2.0 * TOL), PointInSolid::Outside);
}

// ---------------------------------------------------------------- hostile input

#[test]
fn hostile_input_is_an_error_never_a_panic() {
    assert!(Surface::sphere(Frame::XY, f64::NAN).is_err());
    assert!(Surface::cylinder(Frame::XY, 1e300).is_err());
    assert!(Surface::cone(Frame::XY, 1.0, FRAC_PI_2).is_err());
    assert!(Frame::from_normal(Point3::ORIGIN, Vec3::new(f64::NAN, 0.0, 1.0)).is_none());
    assert!(Frame::from_normal(Point3::ORIGIN, Vec3::ZERO).is_none());
    // A plane 1e-9 rad from parallel to a cylinder's axis cuts an ellipse 2e9 mm long:
    // beyond the working range, an error.
    let c = cylinder(Point3::ORIGIN, Vec3::Z, 2.0);
    let n = Vec3::new(0.0, -(1e-9f64).cos(), (1e-9f64).sin());
    assert!(intersect_surfaces(&plane(Point3::ORIGIN, n), &c).is_err());
    // Huge but valid surfaces at the edge of the range answer.
    let big = sphere(Point3::new(MAX_COORDINATE, 0.0, 0.0), MAX_COORDINATE);
    let r = intersect_surfaces(&big, &z_plane(0.0)).unwrap();
    assert!(matches!(r, SurfaceIntersection::Curves { .. }));
    // It passes through the origin, so it cuts the unit sphere there: by the two-circle
    // formula, a = (D² + r1² - r2²) / 2D = 1e6 - 1/(2e6) from its centre, ρ ≈ 1.
    let unit = sphere(Point3::ORIGIN, 1.0);
    let r = intersect_surfaces(&big, &unit).unwrap();
    on_both(&r, &big, &unit);
    let k = circle_of(&r.curves()[0]);
    assert!((k.center().x - 0.5e-6).abs() < 1e-9 && (k.radius() - 1.0).abs() < 1e-6);
    // A cone with its apex far beyond the range: still answers or errors.
    let far = Surface::Cone(Cone::new(Frame::XY, 1e6, 1e-9).unwrap());
    let _ = intersect_surfaces(&far, &plane(Point3::ORIGIN, Vec3::new(1.0, 0.0, 1.0)));
    let _ = intersect_surfaces(&far, &z_plane(5.0));
    // Classification refuses bad points and empty bodies.
    let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
    assert!(classify_point(&b, Point3::new(f64::NAN, 0.0, 0.0)).is_err());
    assert!(classify_point(&b, Point3::new(1e300, 0.0, 0.0)).is_err());
    assert!(classify_point(&Body::new(), Point3::ORIGIN).is_err());
}

/// A deterministic pseudo-random sequence in [0, 1) (64-bit LCG, Knuth's MMIX constants).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
    fn unit(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
            );
            if let Some(u) = v.normalized() {
                return u;
            }
        }
    }
}

fn random_surface(g: &mut Lcg, axis: Option<(Point3, Vec3)>) -> Surface {
    let (o, z) = axis.unwrap_or_else(|| {
        (
            Point3::new(g.range(-5.0, 5.0), g.range(-5.0, 5.0), g.range(-5.0, 5.0)),
            g.unit(),
        )
    });
    // Slide along the shared axis so coaxial pairs have different origins.
    let o = o + z * g.range(-3.0, 3.0);
    let f = frame_at(o, z);
    match (g.next() * 5.0) as u32 {
        0 => Surface::Plane(f),
        1 => Surface::cylinder(f, g.range(0.5, 5.0)).unwrap(),
        2 => Surface::cone(
            f,
            g.range(0.0, 3.0),
            g.range(0.2, 1.3) * if g.next() < 0.5 { -1.0 } else { 1.0 },
        )
        .unwrap(),
        3 => Surface::sphere(f, g.range(0.5, 5.0)).unwrap(),
        _ => Surface::torus(f, g.range(1.0, 6.0), g.range(0.3, 2.0)).unwrap(),
    }
}

#[test]
fn random_surface_pairs_give_points_on_both_surfaces() {
    let mut g = Lcg(0x5eed);
    let mut answered = 0;
    for _ in 0..4000 {
        let a = random_surface(&mut g, None);
        let axis = (g.next() < 0.5).then(|| (a.frame().origin(), a.frame().z()));
        let b = random_surface(&mut g, axis);
        if let Ok(r) = intersect_surfaces(&a, &b) {
            on_both(&r, &a, &b);
            if matches!(
                r,
                SurfaceIntersection::Curves { .. } | SurfaceIntersection::Tangent { .. }
            ) {
                answered += 1;
            }
        }
    }
    // Most pairs with a shared axis, and many planar ones, have exact answers.
    assert!(answered > 1000, "{answered}");
}

/// The implicit function of a surface (no cone: its double-nappe form has spurious
/// sign changes on the other nappe).
fn implicit_value(s: &Surface, p: Point3) -> Option<f64> {
    let l = s.frame().to_local(p);
    let rho2 = l.x * l.x + l.y * l.y;
    let all = rho2 + l.z * l.z;
    match s {
        Surface::Plane(_) => Some(l.z),
        Surface::Cylinder(c) => Some(rho2 - c.radius() * c.radius()),
        Surface::Sphere(c) => Some(all - c.radius() * c.radius()),
        Surface::Torus(t) => {
            let (big, a) = (t.major_radius(), t.minor_radius());
            let q = all + big * big - a * a;
            Some(q * q - 4.0 * big * big * rho2)
        }
        Surface::Cone(_) => None,
    }
}

#[test]
fn random_curves_give_points_on_the_surface_and_miss_no_crossing() {
    let mut g = Lcg(42);
    for _ in 0..3000 {
        let s = random_surface(&mut g, None);
        let o = Point3::new(g.range(-6.0, 6.0), g.range(-6.0, 6.0), g.range(-6.0, 6.0));
        let c = match (g.next() * 4.0) as u32 {
            0 => line(o, g.unit()),
            1 => Curve::Segment(Segment3::new(o, o + g.unit() * g.range(0.5, 15.0)).unwrap()),
            2 => Curve::Circle(Circle3::new(frame_at(o, g.unit()), g.range(0.5, 6.0)).unwrap()),
            _ => Curve::Ellipse(
                Ellipse3::new(frame_at(o, g.unit()), g.range(0.5, 6.0), g.range(0.5, 6.0)).unwrap(),
            ),
        };
        match intersect_curve_surface(&c, &s).unwrap() {
            CurveSurfaceIntersection::Points(p) => {
                for h in &p {
                    assert!(s.distance(h.point) <= TOL, "{h:?} off {s:?}");
                }
                assert!(ts(&p).windows(2).all(|w| w[0] <= w[1]));
                // Completeness: every sign change of the implicit function between dense
                // samples (over a line's first ±40 mm) has a reported point within it.
                let (lo, hi) = match c {
                    Curve::Line(_) => (-40.0, 40.0),
                    _ => c.range(),
                };
                let n = 2000;
                let t_at = |k: usize| lo + (hi - lo) * k as f64 / n as f64;
                for k in 0..n {
                    let (ta, tb) = (t_at(k), t_at(k + 1));
                    let (Some(fa), Some(fb)) = (
                        implicit_value(&s, c.point_at(ta)),
                        implicit_value(&s, c.point_at(tb)),
                    ) else {
                        continue;
                    };
                    if fa * fb < 0.0 {
                        assert!(
                            p.iter().any(|h| h.t >= ta - 1e-6 && h.t <= tb + 1e-6),
                            "missed a crossing in [{ta}, {tb}] of {c:?} with {s:?}: {p:?}"
                        );
                    }
                }
            }
            CurveSurfaceIntersection::OnSurface { .. } => {}
        }
    }
}

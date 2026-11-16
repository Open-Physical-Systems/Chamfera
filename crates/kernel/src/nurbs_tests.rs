//! Free-form (NURBS) faces (rung K10, surfaces): evaluation, inversion, measures,
//! classification and tessellation against values derived by hand.

use crate::{
    Body, Curve, KernelError, NurbsSurface, PointInSolid, PointOnFace, Segment3, Surface,
    TessOptions, classify_point, intersect_curve_surface, tessellate,
};
use crate::{Line3, union};
use chamfera_geom::{BSplineCurve3, BSplineSurface3, Frame, Point3, Transform, Vec3};
use std::f64::consts::{FRAC_1_SQRT_2, PI};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1.0)
}

/// A bicubic Bezier patch over `[0, 1]²` with `x = 30 u`, `y = 30 v` (control x, y on an
/// even grid: linear precision) and heights `base + h` at the four inner control points,
/// `base` at the border ones.
fn bicubic(base: f64, h: f64) -> BSplineSurface3 {
    let k = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let net = (0..4)
        .map(|i| {
            (0..4)
                .map(|j| {
                    let inner = (1..=2).contains(&i) && (1..=2).contains(&j);
                    p(
                        10.0 * i as f64,
                        10.0 * j as f64,
                        base + if inner { h } else { 0.0 },
                    )
                })
                .collect()
        })
        .collect();
    BSplineSurface3::new(3, 3, k.clone(), k, net, None).unwrap()
}

/// The rational quarter cylinder `x² + y² = 100`, `0 <= z <= 5`, from (10, 0) to (0, 10)
/// along u (the quarter circle with weights 1, √2/2, 1), z along v.
fn quarter_cylinder() -> BSplineSurface3 {
    quarter_cylinder_to(5.0)
}

/// The same quarter cylinder up to height `h`.
fn quarter_cylinder_to(h: f64) -> BSplineSurface3 {
    let ring = [p(10.0, 0.0, 0.0), p(10.0, 10.0, 0.0), p(0.0, 10.0, 0.0)];
    let net = ring.iter().map(|c| vec![*c, p(c.x, c.y, h)]).collect();
    let w = FRAC_1_SQRT_2;
    let weights = vec![vec![1.0, 1.0], vec![w, w], vec![1.0, 1.0]];
    BSplineSurface3::new(
        2,
        1,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![0.0, 0.0, 1.0, 1.0],
        net,
        Some(weights),
    )
    .unwrap()
}

/// The nine-point rational full circle of radius 1 (The NURBS Book, ex. 7.2): control
/// points on the square's corners and mid-sides, weights 1 and √2/2.
fn unit_circle() -> (Vec<(f64, f64)>, Vec<f64>, Vec<f64>) {
    let pts = vec![
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
    let w = FRAC_1_SQRT_2;
    let weights = vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0];
    let knots = vec![
        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
    ];
    (pts, weights, knots)
}

/// The meridian half circle of radius `r` from the south pole to the north pole, as
/// `(ρ, z)` control points with weights (two rational quarter circles).
fn meridian(r: f64) -> (Vec<(f64, f64)>, Vec<f64>, Vec<f64>) {
    let w = FRAC_1_SQRT_2;
    (
        vec![(0.0, -r), (r, -r), (r, 0.0), (r, r), (0.0, r)],
        vec![1.0, w, 1.0, w, 1.0],
        vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
    )
}

/// The sphere of radius `r` about the origin as a surface of revolution (The NURBS Book
/// §8.5): `P_ij = (ρ_j X_i, ρ_j Y_i, z_j)`, `w_ij = w_i w_j`. Closed in u, poles at both
/// ends of v; `S_u × S_v` points outward.
fn sphere_surface(r: f64) -> BSplineSurface3 {
    let (circle, cw, ku) = unit_circle();
    let (mer, mw, kv) = meridian(r);
    let net = circle
        .iter()
        .map(|(x, y)| mer.iter().map(|(rho, z)| p(rho * x, rho * y, *z)).collect())
        .collect();
    let weights = cw
        .iter()
        .map(|a| mw.iter().map(|b| a * b).collect())
        .collect();
    BSplineSurface3::new(2, 2, ku, kv, net, Some(weights)).unwrap()
}

/// The closed solid sphere of radius `r`: one free-form face, a seam meridian (the
/// surface's `u = 0` column, in the x/z plane) used twice, two pole vertices.
fn nurbs_sphere(r: f64) -> Body {
    let (mer, mw, kv) = meridian(r);
    let seam = BSplineCurve3::new(
        2,
        kv,
        mer.iter().map(|(rho, z)| p(*rho, 0.0, *z)).collect(),
        Some(mw),
    )
    .unwrap();
    let surface = Surface::nurbs(sphere_surface(r)).unwrap();
    Body::from_boundary(
        &[p(0.0, 0.0, -r), p(0.0, 0.0, r)],
        &[(0, 1, Curve::BSpline(seam), 0.0, 1.0)],
        &[(surface, true, vec![vec![(0, true), (0, false)]])],
    )
    .unwrap()
}

fn seg(a: Point3, b: Point3) -> Curve {
    Curve::Segment(Segment3::new(a, b).unwrap())
}

fn plane(origin: Point3, normal: Vec3) -> Surface {
    Surface::Plane(Frame::from_normal(origin, normal).unwrap())
}

/// The quarter-cylinder wedge `x, y >= 0`, `x² + y² <= 100`, `0 <= z <= 5`: the curved
/// side free-form, the rest planar; arcs as rational splines.
fn wedge() -> Body {
    let (a, b, c) = (p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0), p(0.0, 10.0, 0.0));
    let (d, e, f) = (p(0.0, 0.0, 5.0), p(10.0, 0.0, 5.0), p(0.0, 10.0, 5.0));
    let arc = |z: f64| {
        Curve::BSpline(
            BSplineCurve3::new(
                2,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![p(10.0, 0.0, z), p(10.0, 10.0, z), p(0.0, 10.0, z)],
                Some(vec![1.0, FRAC_1_SQRT_2, 1.0]),
            )
            .unwrap(),
        )
    };
    let edges = vec![
        (0, 1, seg(a, b), 0.0, 1.0),
        (1, 2, arc(0.0), 0.0, 1.0),
        (2, 0, seg(c, a), 0.0, 1.0),
        (3, 4, seg(d, e), 0.0, 1.0),
        (4, 5, arc(5.0), 0.0, 1.0),
        (5, 3, seg(f, d), 0.0, 1.0),
        (0, 3, seg(a, d), 0.0, 1.0),
        (1, 4, seg(b, e), 0.0, 1.0),
        (2, 5, seg(c, f), 0.0, 1.0),
    ];
    let faces = vec![
        (
            Surface::Plane(Frame::XY),
            false,
            vec![vec![(2, false), (1, false), (0, false)]],
        ),
        (
            plane(d, Vec3::Z),
            true,
            vec![vec![(3, true), (4, true), (5, true)]],
        ),
        (
            plane(a, -Vec3::Y),
            true,
            vec![vec![(0, true), (7, true), (3, false), (6, false)]],
        ),
        (
            plane(a, -Vec3::X),
            true,
            vec![vec![(6, true), (5, false), (8, false), (2, true)]],
        ),
        (
            Surface::nurbs(quarter_cylinder()).unwrap(),
            true,
            vec![vec![(1, true), (8, true), (4, false), (7, false)]],
        ),
    ];
    Body::from_boundary(&[a, b, c, d, e, f], &edges, &faces).unwrap()
}

/// A 30 x 30 box from z = 0 up to the bicubic top `bicubic(10, h)`, with
/// the top's border at z = 10 (its border control points are), sides planar.
fn pillow(h: f64) -> Body {
    let corners = [
        p(0.0, 0.0, 0.0),
        p(30.0, 0.0, 0.0),
        p(30.0, 30.0, 0.0),
        p(0.0, 30.0, 0.0),
    ];
    let up = |q: Point3| p(q.x, q.y, 10.0);
    let mut pts: Vec<Point3> = corners.to_vec();
    pts.extend(corners.iter().map(|q| up(*q)));
    // Edges: bottom 0-3, top 4-7, verticals 8-11.
    let mut edges = Vec::new();
    for i in 0..4 {
        edges.push((i, (i + 1) % 4, seg(pts[i], pts[(i + 1) % 4]), 0.0, 1.0));
    }
    for i in 0..4 {
        edges.push((
            4 + i,
            4 + (i + 1) % 4,
            seg(pts[4 + i], pts[4 + (i + 1) % 4]),
            0.0,
            1.0,
        ));
    }
    for i in 0..4 {
        edges.push((i, 4 + i, seg(pts[i], pts[4 + i]), 0.0, 1.0));
    }
    // The top: u along x, v along y, so S_u × S_v = +z (outward) and the loop around
    // the corners counter-clockwise seen from above.
    let top = Surface::nurbs(bicubic(10.0, h)).unwrap();
    let mut faces = vec![
        (
            Surface::Plane(Frame::XY),
            false,
            vec![vec![(3, false), (2, false), (1, false), (0, false)]],
        ),
        (
            top,
            true,
            vec![vec![(4, true), (5, true), (6, true), (7, true)]],
        ),
    ];
    // Side i between corner i and i + 1, outward normals -y, +x, +y, -x.
    let normals = [-Vec3::Y, Vec3::X, Vec3::Y, -Vec3::X];
    for i in 0..4 {
        faces.push((
            plane(pts[i], normals[i]),
            true,
            vec![vec![
                (i, true),
                (8 + (i + 1) % 4, true),
                (4 + i, false),
                (8 + i, false),
            ]],
        ));
    }
    Body::from_boundary(&pts, &edges, &faces).unwrap()
}

#[test]
fn a_bicubic_patch_evaluates_to_hand_derived_points() {
    // Cubic Bernstein: the two inner weights sum to 3t(1 - t), so z = base + h (3u(1-u))
    // (3v(1-v)). At (½, ½): 3/4 · 3/4 = 9/16, z = 2 + 8 · 9/16 = 6.5. At (¼, ½):
    // 3·¼·¾ = 9/16 and 3/4: z = 2 + 8 · 27/64 = 5.375, x = 7.5, y = 15. The control x
    // and y are evenly spaced, so x = 30u, y = 30v.
    let s = Surface::nurbs(bicubic(2.0, 8.0)).unwrap();
    assert!(s.point_at(0.5, 0.5).distance(p(15.0, 15.0, 6.5)) < 1e-12);
    assert!(s.point_at(0.25, 0.5).distance(p(7.5, 15.0, 5.375)) < 1e-12);
    // d/du 3u(1-u) = 3 - 6u = 0 at u = ½: S_u = (30, 0, 0), S_v = (0, 30, 0), normal +z.
    let (su, sv) = s.derivatives(0.5, 0.5);
    assert!((su - Vec3::new(30.0, 0.0, 0.0)).length() < 1e-12);
    assert!((sv - Vec3::new(0.0, 30.0, 0.0)).length() < 1e-12);
    assert!((s.normal_at(0.5, 0.5) - Vec3::Z).length() < 1e-12);
    // At (¼, ½): z_u = 8 (3 - 1.5)(3/4) = 9, so S_u = (30, 0, 9).
    let (su, _) = s.derivatives(0.25, 0.5);
    assert!((su - Vec3::new(30.0, 0.0, 9.0)).length() < 1e-12, "{su:?}");
    // Inversion recovers the parameters of surface points, and the point above the
    // middle projects straight down: (15, 15, 20) is 13.5 above (15, 15, 6.5), where
    // the tangent plane is horizontal.
    for (u, v) in [
        (0.1, 0.2),
        (0.5, 0.5),
        (0.9, 0.35),
        (0.0, 1.0),
        (0.77, 0.03),
    ] {
        let q = s.parameters(s.point_at(u, v));
        assert!(!q.singular);
        assert!(
            (q.u - u).abs() < 1e-9 && (q.v - v).abs() < 1e-9,
            "{u} {v}: {q:?}"
        );
    }
    let c = s.closest_point(p(15.0, 15.0, 20.0)).unwrap();
    assert!((c.distance - 13.5).abs() < 1e-9 && c.point.distance(p(15.0, 15.0, 6.5)) < 1e-9);
    // Beside the patch the nearest point is on its border: (40, 15, 2) is 10 from
    // (30, 15, 2) (the border at x = 30 is the straight line z = 2).
    let c = s.closest_point(p(40.0, 15.0, 2.0)).unwrap();
    assert!((c.distance - 10.0).abs() < 1e-9, "{c:?}");
    assert!(s.closest_point(p(f64::NAN, 0.0, 0.0)).is_err());
}

#[test]
fn a_rational_sphere_is_exact_and_inverts_across_its_seam_and_poles() {
    let Surface::Nurbs(n) = Surface::nurbs(sphere_surface(5.0)).unwrap() else {
        panic!()
    };
    assert_eq!(n.period_u(), Some(1.0));
    assert_eq!(n.period_v(), None);
    let (south, north) = n.poles();
    assert!(south.unwrap().distance(p(0.0, 0.0, -5.0)) < 1e-12);
    assert!(north.unwrap().distance(p(0.0, 0.0, 5.0)) < 1e-12);
    for i in 0..=12 {
        for j in 0..=12 {
            let (u, v) = (i as f64 / 12.0, j as f64 / 12.0);
            let q = n.point_at(u, v);
            assert!((q.distance(Point3::ORIGIN) - 5.0).abs() < 1e-12);
            // Outward normal everywhere, also at the poles (the limit).
            let out = (q - Point3::ORIGIN) * 0.2;
            assert!((n.normal_at(u, v) - out).length() < 1e-6, "{u} {v}");
        }
    }
    // u wraps: u = 1.25 is u = 0.25 (a quarter turn: the +y meridian).
    assert!(n.point_at(1.25, 0.5).distance(p(0.0, 5.0, 0.0)) < 1e-12);
    // Inversion: on the seam u comes back as 0, near it either side; poles are singular.
    let q = n.parameters(p(5.0, 0.0, 0.0));
    assert!(q.u.abs() < 1e-9 && (q.v - 0.5).abs() < 1e-9 && !q.singular);
    let q = n.parameters(p(3.0, -1e-9, 4.0));
    assert!(q.u > 0.99 && q.u < 1.0, "{q:?}");
    let q = n.parameters(p(0.0, 0.0, 5.0));
    assert!(q.singular && q.v == 1.0);
    // A point outside projects radially: (0, 0, 9) is 4 from the north pole, (3, 4, 12)
    // is 13 - 5 = 8 from the sphere.
    let c = n.closest_point(p(3.0, 4.0, 12.0)).unwrap();
    assert!((c.distance - 8.0).abs() < 1e-9, "{c:?}");
    let k = 5.0 / 13.0;
    assert!(c.point.distance(p(3.0 * k, 4.0 * k, 12.0 * k)) < 1e-8);
    // Its box is the control net's: [-5, 5]³.
    let b = n.bounding_box();
    assert!(
        b.min.distance(p(-5.0, -5.0, -5.0)) < 1e-12 && b.max.distance(p(5.0, 5.0, 5.0)) < 1e-12
    );
}

#[test]
fn a_surface_collapsed_in_u_is_turned_so_its_poles_are_in_v() {
    // The sphere with its parameters swapped (normal inward) collapses at u = 0 and 1.
    // The kernel turns it back (swap, then reverse the new u) and keeps that inward
    // normal.
    let s = sphere_surface(5.0).transposed();
    let Surface::Nurbs(n) = Surface::nurbs(s.clone()).unwrap() else {
        panic!()
    };
    assert!(n.poles().0.is_some() && n.poles().1.is_some());
    assert!(n.period_u().is_some());
    let (_, su, sv) = s.derivatives(0.3, 0.6);
    let theirs = su.cross(sv).normalized().unwrap();
    let pt = s.point_at(0.3, 0.6);
    let q = n.parameters(pt);
    assert!(n.normal_at(q.u, q.v).dot(theirs) > 0.999_999);
    assert!(n.normal_at(q.u, q.v).dot(pt - Point3::ORIGIN) < 0.0);
    // A patch collapsed at a u end and a v end (a triangle with two degenerate sides)
    // is refused.
    let k = vec![0.0, 0.0, 1.0, 1.0];
    let tri = BSplineSurface3::new(
        1,
        1,
        k.clone(),
        k,
        vec![
            vec![p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0)],
            vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 0.0)],
        ],
        None,
    )
    .unwrap();
    assert!(matches!(
        Surface::nurbs(tri),
        Err(KernelError::Unsupported(_))
    ));
}

#[test]
fn a_wedge_with_a_free_form_side_has_the_exact_area_and_volume() {
    // r = 10, h = 5, a quarter: V = π r² h / 4 = 125π; the curved side (π/2) r h = 25π;
    // two quarter discs 25π each and two 10 x 5 rectangles: 75π + 100 in all.
    let b = wedge();
    let side = b
        .faces()
        .into_iter()
        .find(|f| matches!(b.face_surface(*f), Ok(Some((Surface::Nurbs(_), _)))))
        .unwrap();
    let a = b.face_area(side).unwrap();
    assert!(close(a, 25.0 * PI, 1e-9), "{a} vs {}", 25.0 * PI);
    let v = b.volume().unwrap();
    assert!(close(v, 125.0 * PI, 1e-9), "{v} vs {}", 125.0 * PI);
    assert!(close(b.area().unwrap(), 75.0 * PI + 100.0, 1e-9));
    // The centre of mass of a quarter disc's sector is 4r/(3π) from each straight side:
    // x = y = 40 / (3π), z = 2.5.
    let m = b.mass_properties(1.0).unwrap();
    let k = 40.0 / (3.0 * PI);
    assert!(
        m.centre_of_mass.distance(p(k, k, 2.5)) < 1e-8,
        "{:?}",
        m.centre_of_mass
    );
    // The face box: the quarter cylinder's control points span [0, 10]² x [0, 5].
    let bb = b.bounding_box().unwrap();
    assert!(bb.max.distance(p(10.0, 10.0, 5.0)) < 1e-9);
}

/// The wedge of radius 10 under the slanted plane `z = 5 + x / 5`: the free-form quarter
/// cylinder (up to z = 10) is trimmed by the ellipse `(10 cos t, 10 sin t, 5 + 2 cos t)`,
/// an edge oblique to its parameter lines.
fn slanted_wedge() -> Body {
    let (a, b, c) = (p(0.0, 0.0, 0.0), p(10.0, 0.0, 0.0), p(0.0, 10.0, 0.0));
    let (d, e, f) = (p(0.0, 0.0, 5.0), p(10.0, 0.0, 7.0), p(0.0, 10.0, 5.0));
    let arc = Curve::BSpline(
        BSplineCurve3::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![p(10.0, 0.0, 0.0), p(10.0, 10.0, 0.0), p(0.0, 10.0, 0.0)],
            Some(vec![1.0, FRAC_1_SQRT_2, 1.0]),
        )
        .unwrap(),
    );
    // The ellipse's frame: centre (0, 0, 5), x along (10, 0, 2) (rx = √104), y along
    // (0, 1, 0) (ry = 10), so its normal is (-2, 0, 10) / |…|: the top plane's.
    let frame =
        Frame::from_normal_and_x(d, Vec3::new(-2.0, 0.0, 10.0), Vec3::new(10.0, 0.0, 2.0)).unwrap();
    let top_arc = Curve::EllipticalArc(
        crate::EllipticalArc3::new(frame, 104f64.sqrt(), 10.0, 0.0, PI / 2.0).unwrap(),
    );
    let edges = vec![
        (0, 1, seg(a, b), 0.0, 1.0),
        (1, 2, arc, 0.0, 1.0),
        (2, 0, seg(c, a), 0.0, 1.0),
        (3, 4, seg(d, e), 0.0, 1.0),
        (4, 5, top_arc, 0.0, PI / 2.0),
        (5, 3, seg(f, d), 0.0, 1.0),
        (0, 3, seg(a, d), 0.0, 1.0),
        (1, 4, seg(b, e), 0.0, 1.0),
        (2, 5, seg(c, f), 0.0, 1.0),
    ];
    let faces = vec![
        (
            Surface::Plane(Frame::XY),
            false,
            vec![vec![(2, false), (1, false), (0, false)]],
        ),
        (
            Surface::Plane(frame),
            true,
            vec![vec![(3, true), (4, true), (5, true)]],
        ),
        (
            plane(a, -Vec3::Y),
            true,
            vec![vec![(0, true), (7, true), (3, false), (6, false)]],
        ),
        (
            plane(a, -Vec3::X),
            true,
            vec![vec![(6, true), (5, false), (8, false), (2, true)]],
        ),
        (
            Surface::nurbs(quarter_cylinder_to(10.0)).unwrap(),
            true,
            vec![vec![(1, true), (8, true), (4, false), (7, false)]],
        ),
    ];
    Body::from_boundary(&[a, b, c, d, e, f], &edges, &faces).unwrap()
}

#[test]
fn an_edge_oblique_to_the_parameter_lines_is_integrated_along_its_curve() {
    // V = ∬_quarter disc (5 + x/5) dA = 5 · 25π + (1/5) ∫∫ x dA, ∫∫ x dA = r³/3 = 1000/3:
    // V = 125π + 200/3. The round side: ∫_0^{π/2} 10 (5 + 2 cos θ) dθ = 25π + 20.
    let b = slanted_wedge();
    let v = b.volume().unwrap();
    let want = 125.0 * PI + 200.0 / 3.0;
    assert!(close(v, want, 1e-9), "{v} vs {want}");
    let side = b
        .faces()
        .into_iter()
        .find(|f| matches!(b.face_surface(*f), Ok(Some((Surface::Nurbs(_), _)))))
        .unwrap();
    let a = b.face_area(side).unwrap();
    assert!(close(a, 25.0 * PI + 20.0, 1e-9), "{a}");
    // ∫ z dV = ∬ (5 + x/5)² / 2 dA = ½ (25 · 25π + 2 · 1000/3 + (1/25) ∬ x² dA) with
    // ∬ x² dA = π r⁴ / 16 = 625π: ½ (625π + 2000/3 + 25π) = 325π + 1000/3.
    let m = b.mass_properties(1.0).unwrap();
    let z = (325.0 * PI + 1000.0 / 3.0) / want;
    assert!(
        (m.centre_of_mass.z - z).abs() < 1e-8,
        "{:?} vs {z}",
        m.centre_of_mass
    );
    let mesh = tessellate(&b, &TessOptions::default()).unwrap();
    assert!(
        (mesh.mesh.signed_volume() - want).abs() < 2.0,
        "{}",
        mesh.mesh.signed_volume()
    );
}

#[test]
fn a_closed_nurbs_sphere_measures_classifies_and_tessellates_like_a_sphere() {
    // r = 5: V = 4/3 π r³ = 500π/3, A = 4π r² = 100π, and about its centre
    // I = 2/5 m r² = 2/5 · 500π/3 · 25 = 5000π/3 for unit density.
    let b = nurbs_sphere(5.0);
    let s = b.summary();
    assert_eq!((s.vertices, s.edges, s.faces), (2, 1, 1));
    let v = b.volume().unwrap();
    assert!(close(v, 500.0 * PI / 3.0, 1e-9), "{v}");
    let a = b.area().unwrap();
    assert!(close(a, 100.0 * PI, 1e-9), "{a}");
    let m = b.mass_properties(1.0).unwrap();
    assert!(m.centre_of_mass.distance(Point3::ORIGIN) < 1e-9);
    for k in 0..3 {
        assert!(
            close(m.principal_moments[k], 5000.0 * PI / 3.0, 1e-8),
            "{m:?}"
        );
    }
    // Points in, out and on.
    assert_eq!(
        classify_point(&b, p(1.0, 2.0, -1.5)).unwrap(),
        PointInSolid::Inside
    );
    assert_eq!(
        classify_point(&b, Point3::ORIGIN).unwrap(),
        PointInSolid::Inside
    );
    assert_eq!(
        classify_point(&b, p(4.0, 3.0, 0.1)).unwrap(),
        PointInSolid::Outside
    );
    assert_eq!(
        classify_point(&b, p(0.0, 0.0, -5.2)).unwrap(),
        PointInSolid::Outside
    );
    assert_eq!(
        classify_point(&b, p(3.0, 0.0, 4.0)).unwrap(),
        PointInSolid::OnBoundary
    );
    assert_eq!(
        classify_point(&b, p(0.0, 3.0, -4.0)).unwrap(),
        PointInSolid::OnBoundary
    );
    let face = b.faces()[0];
    assert_eq!(
        b.classify_point_on_face(face, p(0.0, -4.0, 3.0)).unwrap(),
        PointOnFace::Inside
    );
    assert_eq!(
        b.classify_point_on_face(face, p(0.0, -4.0, 2.0)).unwrap(),
        PointOnFace::Outside
    );
    // A watertight mesh whose volume approaches the sphere's from below: inscribed
    // triangles within the chord tolerance c lose at most the area times c.
    let opts = TessOptions {
        chord_tolerance: 0.02,
        ..TessOptions::default()
    };
    let mesh = tessellate(&b, &opts).unwrap();
    let mv = mesh.mesh.signed_volume();
    let exact = 500.0 * PI / 3.0;
    assert!(
        mv <= exact + 1e-6 && exact - mv <= 100.0 * PI * 0.02,
        "{mv} vs {exact}"
    );
    // Every mesh vertex is on the sphere.
    for q in &mesh.mesh.positions {
        assert!((q.distance(Point3::ORIGIN) - 5.0).abs() < 1e-9);
    }
}

#[test]
fn a_box_with_a_bicubic_top_has_the_volume_under_its_bernstein_heights() {
    // Over [0, 30]² the top is z = 10 + h (3u(1-u))(3v(1-v)) (border control points at
    // 10, the four inner at 10 + h), so V = 9000 + 900 h (∫ 3u(1-u) du)² = 9000 + 900 h
    // / 4. With h = 4: 9900. ∫ z dV = ½ ∬ z² dx dy = 450 (100 + 20h/4 + h² 81/900),
    // so the centroid's height is 450 · 121.44 / 9900 = 5.52; x = y = 15 by symmetry.
    let b = pillow(4.0);
    let v = b.volume().unwrap();
    assert!(close(v, 9900.0, 1e-10), "{v}");
    let m = b.mass_properties(1.0).unwrap();
    assert!(
        m.centre_of_mass.distance(p(15.0, 15.0, 5.52)) < 1e-9,
        "{:?}",
        m.centre_of_mass
    );
    // The top reaches 10 + 4 · 9/16 = 12.25 in the middle; the box includes that bulge.
    let bb = b.bounding_box().unwrap();
    assert!(bb.max.z >= 12.25 - 1e-9 && bb.max.z <= 14.0 + 1e-9);
    assert_eq!(
        classify_point(&b, p(15.0, 15.0, 12.2)).unwrap(),
        PointInSolid::Inside
    );
    assert_eq!(
        classify_point(&b, p(15.0, 15.0, 12.3)).unwrap(),
        PointInSolid::Outside
    );
    assert_eq!(
        classify_point(&b, p(15.0, 15.0, 12.25)).unwrap(),
        PointInSolid::OnBoundary
    );
    // Near the corner (u = 1/30, v = 29/30) the top is at 10 + 4 (3 · 29/900)² = 10.0374.
    assert_eq!(
        classify_point(&b, p(1.0, 29.0, 10.03)).unwrap(),
        PointInSolid::Inside
    );
    assert_eq!(
        classify_point(&b, p(1.0, 29.0, 10.045)).unwrap(),
        PointInSolid::Outside
    );
    let mesh = tessellate(&b, &TessOptions::default()).unwrap();
    let mv = mesh.mesh.signed_volume();
    assert!((mv - 9900.0).abs() < 900.0 * 0.01 * 2.0, "{mv}");
    // Moving and mirroring keep the volume and validity.
    let t = Transform::rotation_about(p(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 0.0), 0.7).unwrap();
    assert!(close(
        b.transformed(&t).unwrap().volume().unwrap(),
        9900.0,
        1e-9
    ));
    let mirrored = b
        .mirrored(p(0.0, 0.0, 0.0), Vec3::new(0.3, 1.0, 0.2))
        .unwrap();
    assert!(close(mirrored.volume().unwrap(), 9900.0, 1e-9));
    // It survives a JSON round trip.
    let json = serde_json::to_string(&b).unwrap();
    let back: Body = serde_json::from_str(&json).unwrap();
    assert!(close(back.volume().unwrap(), 9900.0, 1e-10));
}

#[test]
fn rays_meet_a_free_form_surface_where_the_algebra_says() {
    // The vertical line through (12, 12) meets the bicubic top (base 10, h 4) at
    // u = v = 0.4: z = 10 + 4 (3 · 0.4 · 0.6)² = 10 + 4 · 0.5184 = 12.0736.
    let s = Surface::nurbs(bicubic(10.0, 4.0)).unwrap();
    let line = Curve::Line(Line3::new(p(12.0, 12.0, -50.0), Vec3::Z).unwrap());
    let hits = intersect_curve_surface(&line, &s).unwrap();
    let pts = hits.points();
    assert_eq!(pts.len(), 1, "{hits:?}");
    assert!(pts[0].point.distance(p(12.0, 12.0, 12.0736)) < 1e-9);
    // A line through the sphere's centre meets it twice, 5 either side.
    let sph = Surface::nurbs(sphere_surface(5.0)).unwrap();
    let d = Vec3::new(1.0, 2.0, 2.0).normalized().unwrap();
    let line = Curve::Line(Line3::new(Point3::ORIGIN, d).unwrap());
    let hits = intersect_curve_surface(&line, &sph).unwrap();
    let pts = hits.points();
    assert_eq!(pts.len(), 2, "{hits:?}");
    assert!((pts[0].t + 5.0).abs() < 1e-9 && (pts[1].t - 5.0).abs() < 1e-9);
    // A line 6 from the centre misses; a circle is not supported yet.
    let miss = Curve::Line(Line3::new(p(6.0, 0.0, 0.0), Vec3::Z).unwrap());
    assert!(
        intersect_curve_surface(&miss, &sph)
            .unwrap()
            .points()
            .is_empty()
    );
}

#[test]
fn booleans_on_free_form_solids_are_refused_clearly() {
    let a = nurbs_sphere(5.0);
    let b = pillow(4.0);
    match union(&a, &b) {
        Err(KernelError::Unsupported(m)) => assert!(m.contains("free-form"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        crate::intersect_surfaces(
            &Surface::nurbs(sphere_surface(5.0)).unwrap(),
            &Surface::Plane(Frame::XY)
        ),
        Ok(crate::SurfaceIntersection::Unsupported(
            crate::UnsupportedIntersection::FreeForm
        ))
    ));
}

#[test]
fn hostile_free_form_input_never_panics() {
    let Surface::Nurbs(n) = Surface::nurbs(bicubic(0.0, 3.0)).unwrap() else {
        panic!()
    };
    for q in [
        p(f64::NAN, 0.0, 0.0),
        p(f64::INFINITY, 1.0, 2.0),
        p(1e300, -1e300, 0.0),
        p(0.0, 0.0, 0.0),
    ] {
        let _ = n.parameters(q);
        let _ = n.closest_point(q);
        let _ = n.normal_at(q.x, q.y);
        let _ = n.point_at(q.x, q.y);
    }
    let _ = n.bounding_box_on(f64::NAN, 1.0, 0.0, 1.0).unwrap();
    let _ = n.bounding_box_on(0.5, 0.2, 0.0, 1.0).unwrap();
    // Lines with no direction or bad ranges are refused, not looped on.
    let s = Surface::Nurbs(n.clone());
    let l = Curve::Line(Line3::new(p(1.0, 1.0, 1.0), Vec3::Z).unwrap());
    assert!(crate::intersect_curve_surface_on(&l, 2.0, 1.0, &s).is_err());
    assert!(crate::intersect_curve_surface_on(&l, f64::NAN, 1.0, &s).is_err());
    assert!(crate::intersect_curve_surface_on(&l, f64::NEG_INFINITY, f64::INFINITY, &s).is_ok());
    // Bad serialized surfaces are refused.
    let bad = serde_json::json!({"nurbs": {"degree_u": 1, "degree_v": 1,
        "knots_u": [0.0, 0.0, 1.0, 1.0], "knots_v": [0.0, 0.0, 1.0, 1.0],
        "points": [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], [[0.0, 1.0, 0.0]]],
        "weights": [[1.0, 1.0], [1.0]]}});
    assert!(serde_json::from_value::<Surface>(bad).is_err());
    let s2 = serde_json::to_value(Surface::Nurbs(n)).unwrap();
    let back: Surface = serde_json::from_value(s2).unwrap();
    assert!(
        back.point_at(0.5, 0.5)
            .distance(p(15.0, 15.0, 3.0 * 0.5625))
            < 1e-12
    );
    let _ = NurbsSurface::new(bicubic(0.0, 0.0)).unwrap();
}

#[test]
fn a_free_form_sphere_scales_non_uniformly_into_an_exact_ellipsoid() {
    // Scaling x by 2 about the centre maps the sphere r = 5 to the ellipsoid with
    // semi-axes 10, 5, 5: V = 4/3 π · 10 · 5 · 5 = 1000π/3 (affine invariance keeps it
    // an exact NURBS surface).
    let b = nurbs_sphere(5.0)
        .scaled(Point3::ORIGIN, [2.0, 1.0, 1.0])
        .unwrap();
    let v = b.volume().unwrap();
    assert!(close(v, 1000.0 * PI / 3.0, 1e-9), "{v}");
    assert_eq!(
        classify_point(&b, p(9.0, 0.5, 0.5)).unwrap(),
        PointInSolid::Inside
    );
}

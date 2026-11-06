use super::*;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI};

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

/// The rational quadratic quarter circle (The NURBS Book, §7.3): control
/// points (1,0), (1,1), (0,1) with weights 1, √2/2, 1 is exactly the unit circle's first
/// quadrant.
fn quarter() -> BSplineCurve2 {
    BSplineCurve2::new(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![pt(1.0, 0.0), pt(1.0, 1.0), pt(0.0, 1.0)],
        Some(vec![1.0, FRAC_1_SQRT_2, 1.0]),
    )
    .unwrap()
}

/// The whole unit circle as four rational quadratic quarters (nine points, double
/// interior knots): the corners of the square have weight √2/2.
fn circle(r: f64, c: Point2) -> BSplineCurve2 {
    let w = FRAC_1_SQRT_2;
    let pts = [
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (-1.0, 1.0),
        (-1.0, 0.0),
        (-1.0, -1.0),
        (0.0, -1.0),
        (1.0, -1.0),
        (1.0, 0.0),
    ]
    .iter()
    .map(|(x, y)| pt(c.x + r * x, c.y + r * y))
    .collect();
    BSplineCurve2::new(
        2,
        vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ],
        pts,
        Some(vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]),
    )
    .unwrap()
}

#[test]
fn the_rational_quadratic_is_exactly_a_quarter_circle() {
    let q = quarter();
    assert!(q.is_rational());
    for i in 0..=1000 {
        let u = i as f64 / 1000.0;
        let p = q.point_at(u);
        let r = (p.x * p.x + p.y * p.y).sqrt();
        assert!((r - 1.0).abs() <= 1e-12, "u = {u}: r = {r}");
        assert!(p.x >= -1e-15 && p.y >= -1e-15);
    }
    assert!(q.point_at(0.0).distance(pt(1.0, 0.0)) < 1e-15);
    assert!(q.point_at(1.0).distance(pt(0.0, 1.0)) < 1e-15);
    // By symmetry the middle of the parameter range is at 45°.
    assert!(q.point_at(0.5).distance(pt(FRAC_1_SQRT_2, FRAC_1_SQRT_2)) < 1e-15);
}

#[test]
fn rational_derivatives_match_the_end_formula_and_the_circle() {
    let q = quarter();
    // Eq. 4.9: C'(0) = p / u_{p+1} · (w1 / w0) (P1 - P0) = 2 · (√2/2) · (0, 1).
    let d = q.d1(0.0);
    assert!((d.x).abs() < 1e-14 && (d.y - 2.0 * FRAC_1_SQRT_2).abs() < 1e-14);
    // On a circle the tangent is perpendicular to the radius everywhere, and the
    // curvature |C' × C''| / |C'|³ is 1.
    for i in 0..=20 {
        let u = i as f64 / 20.0;
        let ders = q.derivatives(u, 2);
        let (c, d1, d2) = (ders[0], ders[1], ders[2]);
        assert!(c.dot(d1).abs() < 1e-12, "u = {u}");
        let k = d1.cross(d2).abs() / d1.length().powi(3);
        assert!((k - 1.0).abs() < 1e-10, "u = {u}: curvature {k}");
    }
}

#[test]
fn a_degree_one_spline_is_its_polyline() {
    let pts = vec![pt(0.0, 0.0), pt(3.0, 0.0), pt(3.0, 4.0), pt(-1.0, 4.0)];
    // Knots at the cumulative chord lengths 0, 3, 7, 11 make the parameter arc length.
    let s = BSplineCurve2::new(1, vec![0.0, 0.0, 3.0, 7.0, 11.0, 11.0], pts.clone(), None).unwrap();
    for (u, want) in [
        (0.0, pt(0.0, 0.0)),
        (1.5, pt(1.5, 0.0)),
        (3.0, pt(3.0, 0.0)),
        (5.0, pt(3.0, 2.0)),
        (9.0, pt(1.0, 4.0)),
        (11.0, pt(-1.0, 4.0)),
    ] {
        assert!(s.point_at(u).distance(want) < 1e-14, "u = {u}");
    }
    // Length 3 + 4 + 4 = 11.
    assert!((s.length().unwrap() - 11.0).abs() < 1e-12);
    // The polyline from control points (uniform knots) is the same set of points.
    let c = BSplineCurve2::new(1, vec![0.0, 0.0, 1.0, 2.0, 3.0, 3.0], pts, None).unwrap();
    assert!(c.point_at(1.5).distance(pt(3.0, 2.0)) < 1e-14);
}

#[test]
fn the_quarter_circle_has_length_pi_over_two_and_the_circle_area_pi() {
    let q = quarter();
    assert!((q.length().unwrap() - FRAC_PI_2).abs() < 1e-11);
    let c = circle(2.0, pt(1.0, -3.0));
    assert!(c.is_closed());
    // Circumference 2π r and area π r² with r = 2, wherever the centre is.
    assert!((c.length().unwrap() - 4.0 * PI).abs() < 1e-10);
    let (a, b) = c.domain();
    assert!((c.area_term(a, b).unwrap() - 4.0 * PI).abs() < 1e-10);
    assert!((c.reversed().area_term(a, b).unwrap() + 4.0 * PI).abs() < 1e-10);
}

#[test]
fn knot_insertion_leaves_the_curve_unchanged() {
    let c = circle(1.0, pt(0.0, 0.0));
    let samples: Vec<f64> = (0..=40).map(|i| i as f64 / 40.0).collect();
    for (u, times) in [(0.1, 1), (0.1, 2), (0.3, 1), (0.6, 2)] {
        let k = c.insert_knot(u, times).unwrap();
        assert_eq!(k.control_points().len(), 9 + times);
        for s in &samples {
            assert!(
                k.point_at(*s).distance(c.point_at(*s)) < 1e-14,
                "u = {u}, s = {s}"
            );
        }
    }
    // A knot may not be raised past the degree, nor inserted outside the domain.
    assert!(c.insert_knot(0.25, 1).is_err());
    assert!(c.insert_knot(0.0, 1).is_err(), "the clamped start is full");
    assert!(c.insert_knot(1.5, 1).is_err());
    assert!(c.insert_knot(f64::NAN, 1).is_err());
}

#[test]
fn splitting_gives_two_curves_on_the_same_points() {
    let fit = [
        pt(0.0, 0.0),
        pt(10.0, 5.0),
        pt(20.0, -3.0),
        pt(30.0, 8.0),
        pt(35.0, 0.0),
    ];
    let s = BSplineCurve2::interpolate(&fit, None, None).unwrap();
    let (left, right) = s.split_at(0.37).unwrap();
    assert_eq!(left.domain(), (0.0, 0.37));
    assert_eq!(right.domain(), (0.37, 1.0));
    for i in 0..=30 {
        let u = 0.37 * i as f64 / 30.0;
        assert!(left.point_at(u).distance(s.point_at(u)) < 1e-12);
        let v = 0.37 + 0.63 * i as f64 / 30.0;
        assert!(right.point_at(v).distance(s.point_at(v)) < 1e-12);
    }
    // The lengths add up.
    let total = s.length().unwrap();
    let parts = left.length().unwrap() + right.length().unwrap();
    assert!((total - parts).abs() < 1e-9 * total);
    // The ends cannot be split off.
    assert!(s.split_at(0.0).is_err() && s.split_at(1.0).is_err());
    // Segments of a closed rational curve keep it a circle.
    let seg = circle(1.0, pt(0.0, 0.0)).segment(0.1, 0.8).unwrap();
    for i in 0..=20 {
        let u = 0.1 + 0.7 * i as f64 / 20.0;
        let p = seg.point_at(u);
        assert!((p.x.hypot(p.y) - 1.0).abs() < 1e-12);
    }
}

#[test]
fn interpolation_passes_through_its_fit_points() {
    let fit = [
        pt(0.0, 0.0),
        pt(10.0, 5.0),
        pt(20.0, -3.0),
        pt(30.0, 8.0),
        pt(35.0, 0.0),
        pt(36.0, -10.0),
    ];
    let s = BSplineCurve2::interpolate(&fit, None, None).unwrap();
    assert_eq!(s.degree(), 3);
    for q in fit {
        let c = s.closest_point(q);
        assert!(c.distance < 1e-9, "{q:?} is {} away", c.distance);
    }
    // The parameters are the chord lengths: the fit points are hit at ū_k exactly.
    let chords: Vec<f64> = fit.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f64 = chords.iter().sum();
    let mut acc = 0.0;
    for (k, q) in fit.iter().enumerate() {
        assert!(s.point_at(acc / total).distance(*q) < 1e-9, "point {k}");
        if let Some(c) = chords.get(k) {
            acc += c;
        }
    }
    // With end tangents the curve leaves and arrives along them.
    let t = BSplineCurve2::interpolate(&fit, Some(Vec2::Y), Some(Vec2::new(1.0, 1.0))).unwrap();
    for (k, q) in fit.iter().enumerate() {
        let u = fit[..=k]
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum::<f64>()
            / total;
        assert!(
            t.point_at(u).distance(*q) < 1e-9,
            "{k}: {:?} vs {q:?}",
            t.point_at(u)
        );
    }
    let d0 = t.d1(0.0);
    assert!(d0.x.abs() < 1e-9 && d0.y > 0.0);
    assert!((d0.length() - total).abs() < 1e-9 * total);
    let d1 = t.d1(1.0);
    assert!((d1.x - d1.y).abs() < 1e-9 && d1.x > 0.0);
    // Two points give the straight line; three a parabola-like quadratic.
    let line = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(4.0, 2.0)], None, None).unwrap();
    assert_eq!(line.degree(), 1);
    assert!(line.point_at(0.5).distance(pt(2.0, 1.0)) < 1e-14);
    let q = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(1.0, 1.0), pt(2.0, 0.0)], None, None)
        .unwrap();
    assert_eq!(q.degree(), 2);
    assert!(q.point_at(0.5).distance(pt(1.0, 1.0)) < 1e-12);
}

#[test]
fn a_closed_fit_spline_closes_smoothly() {
    let fit = [pt(10.0, 0.0), pt(0.0, 10.0), pt(-10.0, 0.0), pt(0.0, -10.0)];
    let s = BSplineCurve2::interpolate_closed(&fit).unwrap();
    assert!(s.is_closed());
    for q in fit {
        assert!(s.closest_point(q).distance < 1e-9);
    }
    let (a, b) = s.domain();
    let (t0, t1) = (s.d1(a), s.d1(b));
    assert!((t0 - t1).length() < 1e-9 * t0.length());
    // Through the corners of a square of diagonal 20, bulging outward: more than the
    // inscribed square's area 200, less than the circumscribed square's 400.
    let area = s.area_term(a, b).unwrap();
    assert!(area > 200.0 && area < 400.0, "{area}");
}

#[test]
fn control_point_splines_start_and_end_at_their_end_points() {
    let ctrl = [
        pt(0.0, 0.0),
        pt(5.0, 10.0),
        pt(15.0, 10.0),
        pt(20.0, 0.0),
        pt(30.0, 5.0),
    ];
    let s = BSplineCurve2::from_control_points(&ctrl, false).unwrap();
    assert_eq!(s.degree(), 3);
    assert_eq!(s.domain(), (0.0, 1.0));
    assert!(s.point_at(0.0).distance(ctrl[0]) < 1e-14);
    assert!(s.point_at(1.0).distance(ctrl[4]) < 1e-14);
    // Tangent along the first leg (eq. 3.7: C'(0) = p / u_{p+1} (P1 - P0)); here two
    // spans, u_{p+1} = 1/2, so C'(0) = 6 (P1 - P0).
    let d = s.d1(0.0);
    assert!((d - (ctrl[1] - ctrl[0]) * 6.0).length() < 1e-12);
    let closed = BSplineCurve2::from_control_points(&ctrl, true).unwrap();
    assert!(closed.is_closed());
    let (a, b) = closed.domain();
    let (t0, t1) = (closed.d1(a), closed.d1(b));
    assert!((t0 - t1).length() < 1e-9 * t0.length());
    let (a2, b2) = (closed.d2(a), closed.d2(b));
    assert!((a2 - b2).length() < 1e-8 * a2.length().max(1.0));
}

#[test]
fn closest_points_on_the_quarter_circle_are_radial() {
    let q = quarter();
    // From (2, 2) the nearest point is at 45°, at distance 2√2 - 1.
    let c = q.closest_point(pt(2.0, 2.0));
    assert!(c.point.distance(pt(FRAC_1_SQRT_2, FRAC_1_SQRT_2)) < 1e-10);
    assert!((c.distance - (2.0 * 2f64.sqrt() - 1.0)).abs() < 1e-12);
    // From a point inside, along the ray at 30°.
    let a = PI / 6.0;
    let c = q.closest_point(pt(0.3 * a.cos(), 0.3 * a.sin()));
    assert!(c.point.distance(pt(a.cos(), a.sin())) < 1e-10);
    assert!((c.distance - 0.7).abs() < 1e-12);
    // Beyond the end the end point is nearest.
    let c = q.closest_point(pt(3.0, -1.0));
    assert!(c.point.distance(pt(1.0, 0.0)) < 1e-12 && c.u.abs() < 1e-12);
    // From the centre every point is as near: any one at distance 1.
    let c = q.closest_point(pt(0.0, 0.0));
    assert!((c.distance - 1.0).abs() < 1e-12);
}

#[test]
fn boxes_contain_the_curve() {
    let c = circle(3.0, pt(1.0, 1.0));
    let (lo, hi) = c.hull_box();
    assert_eq!((lo, hi), (pt(-2.0, -2.0), pt(4.0, 4.0)));
    let (lo, hi) = c.bounding_box_on(0.0, 0.25).unwrap();
    // The first quarter: x in [1, 4], y in [1, 4].
    assert!(lo.x >= 1.0 - 1e-12 && lo.y >= 1.0 - 1e-12 && hi.x <= 4.0 + 1e-12);
    for i in 0..=10 {
        let p = c.point_at(0.025 * i as f64);
        assert!(p.x >= lo.x - 1e-12 && p.x <= hi.x + 1e-12);
        assert!(p.y >= lo.y - 1e-12 && p.y <= hi.y + 1e-12);
    }
}

#[test]
fn space_curves_lift_reverse_and_move() {
    let f = Frame::from_normal(Point3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 1.0, 1.0)).unwrap();
    let c3 = quarter().to_3d(&f).unwrap();
    for i in 0..=10 {
        let u = i as f64 / 10.0;
        let p = c3.point_at(u);
        assert!((p.distance(f.origin()) - 1.0).abs() < 1e-12);
        assert!((p - f.origin()).dot(f.z()).abs() < 1e-12);
    }
    let r = c3.reversed();
    assert!(
        r.point_at(r.reversed_parameter(0.3))
            .distance(c3.point_at(0.3))
            < 1e-14
    );
    let t = Transform::translation(Vec3::new(1.0, 2.0, 3.0)).unwrap();
    let m = c3.transformed(&t).unwrap();
    assert!(
        m.point_at(0.4)
            .distance(c3.point_at(0.4) + Vec3::new(1.0, 2.0, 3.0))
            < 1e-14
    );
    // The vector area of the closed circle lifted into a plane is π r² along its normal.
    let disc = circle(2.0, pt(0.0, 0.0)).to_3d(&f).unwrap();
    let a = disc.area_moment(0.0, 1.0, f.origin()).unwrap() * 0.5;
    assert!((a - f.z() * (4.0 * PI)).length() < 1e-9);
    assert!((quarter().to_3d(&f).unwrap().length().unwrap() - FRAC_PI_2).abs() < 1e-11);
}

#[test]
fn hostile_curves_are_refused() {
    let pts = vec![pt(0.0, 0.0), pt(1.0, 1.0), pt(2.0, 0.0)];
    let k = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let ok = BSplineCurve2::new(2, k.clone(), pts.clone(), None);
    assert!(ok.is_ok());
    // Degree 0 and above 7.
    assert!(BSplineCurve2::new(0, vec![0.0, 1.0, 2.0, 3.0], pts.clone(), None).is_err());
    let many: Vec<Point2> = (0..9).map(|i| pt(i as f64, 0.0)).collect();
    let knots9: Vec<f64> = (0..18).map(|i| i as f64).collect();
    assert!(BSplineCurve2::new(8, knots9, many.clone(), None).is_err());
    // Weights: zero, negative, NaN, infinite, wrong count.
    for w in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e9] {
        assert!(BSplineCurve2::new(2, k.clone(), pts.clone(), Some(vec![1.0, w, 1.0])).is_err());
    }
    assert!(BSplineCurve2::new(2, k.clone(), pts.clone(), Some(vec![1.0, 1.0])).is_err());
    // Knots: decreasing, NaN, wrong count, empty domain, too high a multiplicity.
    assert!(BSplineCurve2::new(2, vec![0.0, 0.0, 0.0, 1.0, 0.5, 1.0], pts.clone(), None).is_err());
    assert!(
        BSplineCurve2::new(
            2,
            vec![0.0, 0.0, f64::NAN, 1.0, 1.0, 1.0],
            pts.clone(),
            None
        )
        .is_err()
    );
    assert!(BSplineCurve2::new(2, vec![0.0, 0.0, 1.0, 1.0, 1.0], pts.clone(), None).is_err());
    assert!(BSplineCurve2::new(2, vec![1.0; 6], pts.clone(), None).is_err());
    let four = vec![pt(0.0, 0.0), pt(1.0, 1.0), pt(2.0, 0.0), pt(3.0, 1.0)];
    assert!(
        BSplineCurve2::new(1, vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0], four, None).is_err(),
        "an interior knot repeated more than the degree"
    );
    // Points: too few, non-finite, outside the working range, too many.
    assert!(BSplineCurve2::new(2, vec![0.0, 0.0, 1.0, 1.0], pts[..1].to_vec(), None).is_err());
    let bad = vec![pt(0.0, 0.0), pt(f64::NAN, 1.0), pt(2.0, 0.0)];
    assert!(BSplineCurve2::new(2, k.clone(), bad, None).is_err());
    let far = vec![pt(0.0, 0.0), pt(1e7, 1.0), pt(2.0, 0.0)];
    assert!(BSplineCurve2::new(2, k.clone(), far, None).is_err());
    let huge: Vec<Point2> = (0..MAX_CONTROL_POINTS + 1)
        .map(|i| pt(i as f64, 0.0))
        .collect();
    assert!(BSplineCurve2::from_control_points(&huge, false).is_err());
    // Interpolation: repeated points, too few, too many, zero tangents.
    assert!(BSplineCurve2::interpolate(&[pt(0.0, 0.0)], None, None).is_err());
    assert!(BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(0.0, 0.0)], None, None).is_err());
    let fit = [pt(0.0, 0.0), pt(1.0, 0.0)];
    assert!(BSplineCurve2::interpolate(&fit, Some(Vec2::ZERO), None).is_err());
    assert!(BSplineCurve2::interpolate(&fit, Some(Vec2::new(f64::NAN, 0.0)), None).is_err());
    let lots: Vec<Point2> = (0..MAX_FIT_POINTS + 1).map(|i| pt(i as f64, 0.0)).collect();
    assert!(BSplineCurve2::interpolate(&lots, None, None).is_err());
    assert!(BSplineCurve2::interpolate_closed(&fit).is_err());
    // Queries with hostile parameters clamp instead of failing or panicking.
    let c = ok.unwrap();
    assert!(c.point_at(f64::NAN).is_valid() && c.point_at(1e300).is_valid());
    assert!(c.segment(0.5, 0.2).is_err() && c.segment(-1.0, 0.5).is_err());
    let _ = c.closest_point(pt(f64::NAN, 0.0));
}

#[test]
fn curves_round_trip_through_json_and_loading_revalidates() {
    let q = quarter();
    let json = serde_json::to_string(&q).unwrap();
    let back: BSplineCurve2 = serde_json::from_str(&json).unwrap();
    assert_eq!(back, q);
    let bad = r#"{"degree":2,"knots":[0,0,0,1,1,1],"points":[{"x":0,"y":0},{"x":1,"y":1},{"x":2,"y":0}],"weights":[1,0,1]}"#;
    assert!(serde_json::from_str::<BSplineCurve2>(bad).is_err());
    let unknown = r#"{"degree":2,"knots":[0,0,0,1,1,1],"points":[{"x":0,"y":0},{"x":1,"y":1},{"x":2,"y":0}],"weights":[1,1,1],"extra":1}"#;
    assert!(serde_json::from_str::<BSplineCurve2>(unknown).is_err());
}

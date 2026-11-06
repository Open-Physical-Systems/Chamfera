use super::*;
use chamfera_geom::Point2;
use std::f64::consts::PI;

fn at(z: f64) -> Frame {
    Frame::XY.with_origin(Point3::new(0.0, 0.0, z)).unwrap()
}

fn square(side: f64) -> Profile {
    let h = side / 2.0;
    Profile::polygon(&[
        Point2::new(-h, -h),
        Point2::new(h, -h),
        Point2::new(h, h),
        Point2::new(-h, h),
    ])
}

fn circle(r: f64) -> Profile {
    Profile::circle(Point2::ORIGIN, r)
}

fn check_valid(b: &Body) {
    let r = b.validate().unwrap();
    let s = r.summary;
    assert_eq!(s.shells, 1);
    assert_eq!(
        s.vertices as i64 - s.edges as i64 + s.faces as i64,
        2,
        "{s:?}"
    );
}

#[test]
fn a_square_to_a_smaller_square_is_a_frustum() {
    let b = loft(&[(at(0.0), square(20.0)), (at(10.0), square(10.0))]).unwrap();
    check_valid(&b);
    // h/3 (A1 + A2 + √(A1 A2)) = 10/3 (400 + 100 + 200).
    let v = 10.0 / 3.0 * (400.0 + 100.0 + 200.0);
    assert!(
        (b.volume().unwrap() - v).abs() < 1e-9,
        "{}",
        b.volume().unwrap()
    );
    // Four trapezoids (20 + 10)/2 high √(10² + 5²), and the two squares.
    let a = 4.0 * 15.0 * 125f64.sqrt() + 400.0 + 100.0;
    assert!((b.area().unwrap() - a).abs() < 1e-9);
    let names: Vec<String> = b
        .faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).unwrap().map(str::to_owned))
        .collect();
    for n in ["start", "end", "band 0 side 0", "band 0 side 3"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
}

#[test]
fn a_circle_to_a_smaller_circle_is_a_cone_frustum() {
    let b = loft(&[(at(0.0), circle(10.0)), (at(10.0), circle(5.0))]).unwrap();
    check_valid(&b);
    // π h/3 (R² + R r + r²) = π 10/3 (100 + 50 + 25).
    let v = PI * 10.0 / 3.0 * (100.0 + 50.0 + 25.0);
    assert!(
        (b.volume().unwrap() - v).abs() < 1e-9 * v,
        "{}",
        b.volume().unwrap()
    );
    // Lateral π (R + r) s with slant s = √(10² + 5²), and the two discs.
    let a = PI * 15.0 * 125f64.sqrt() + PI * 125.0;
    assert!((b.area().unwrap() - a).abs() < 1e-9 * a);
    assert!(
        b.faces()
            .into_iter()
            .any(|f| matches!(b.face_surface(f).unwrap(), Some((Surface::Cone(_), _))))
    );
}

#[test]
fn equal_circles_loft_into_a_cylinder_and_three_sections_add_up() {
    let b = loft(&[(at(0.0), circle(4.0)), (at(5.0), circle(4.0))]).unwrap();
    check_valid(&b);
    assert!((b.volume().unwrap() - PI * 16.0 * 5.0).abs() < 1e-9);
    // Waist: 20 -> 10 -> 20, two frusta.
    let b = loft(&[
        (at(0.0), square(20.0)),
        (at(10.0), square(10.0)),
        (at(20.0), square(20.0)),
    ])
    .unwrap();
    check_valid(&b);
    assert!((b.volume().unwrap() - 2.0 * 7000.0 / 3.0).abs() < 1e-9);
}

#[test]
fn order_and_plane_orientation_do_not_matter() {
    let v = 7000.0 / 3.0;
    let down = loft(&[(at(10.0), square(10.0)), (at(0.0), square(20.0))]).unwrap();
    assert!((down.volume().unwrap() - v).abs() < 1e-9);
    // The top profile drawn on a plane facing down, with its vertices starting
    // elsewhere: the edges are still matched.
    let flipped = at(10.0).flipped();
    let top = Profile::polygon(&[
        Point2::new(5.0, 5.0),
        Point2::new(-5.0, 5.0),
        Point2::new(-5.0, -5.0),
        Point2::new(5.0, -5.0),
    ]);
    let b = loft(&[(at(0.0), square(20.0)), (flipped, top)]).unwrap();
    check_valid(&b);
    assert!((b.volume().unwrap() - v).abs() < 1e-9);
}

#[test]
fn a_rounded_slot_lofts_with_planes_and_cones() {
    // A slot: two half circles of radius r joined by lines of length 2, scaled from
    // r = 4 to r = 2 over 6. Its area is 4 r + π r²; the loft is a prismatoid whose
    // cross-section area is quadratic in z, so Simpson's rule is exact:
    // V = h/6 (A0 + 4 Am + A1) with Am at r = 3.
    let slot = |r: f64| {
        Profile::new(
            vec![
                ProfileEdge::line(Point2::new(-1.0, -r), Point2::new(1.0, -r)),
                ProfileEdge::Arc {
                    centre: Point2::new(1.0, 0.0),
                    radius: r,
                    start_angle: -PI / 2.0,
                    sweep: PI,
                },
                ProfileEdge::line(Point2::new(1.0, r), Point2::new(-1.0, r)),
                ProfileEdge::Arc {
                    centre: Point2::new(-1.0, 0.0),
                    radius: r,
                    start_angle: PI / 2.0,
                    sweep: PI,
                },
            ],
            Vec::new(),
        )
    };
    let b = loft(&[(at(0.0), slot(4.0)), (at(6.0), slot(2.0))]).unwrap();
    check_valid(&b);
    let area = |r: f64| 4.0 * r + PI * r * r;
    let v = 6.0 / 6.0 * (area(4.0) + 4.0 * area(3.0) + area(2.0));
    assert!(
        (b.volume().unwrap() - v).abs() < 1e-9 * v,
        "{}",
        b.volume().unwrap()
    );
}

#[test]
fn inexact_and_bad_lofts_are_refused() {
    // A square turned by 45°: skew sides need a hyperbolic paraboloid.
    let turned = Profile::polygon(&[
        Point2::new(0.0, -7.0),
        Point2::new(7.0, 0.0),
        Point2::new(0.0, 7.0),
        Point2::new(-7.0, 0.0),
    ]);
    assert!(matches!(
        loft(&[(at(0.0), square(20.0)), (at(10.0), turned)]),
        Err(KernelError::Unsupported(_))
    ));
    // Circles off each other's axis: an oblique cone.
    let off = Profile::circle(Point2::new(3.0, 0.0), 5.0);
    assert!(matches!(
        loft(&[(at(0.0), circle(10.0)), (at(10.0), off)]),
        Err(KernelError::Unsupported(_))
    ));
    // A square and a circle: different edge counts.
    assert!(loft(&[(at(0.0), square(20.0)), (at(10.0), circle(5.0))]).is_err());
    // A hole in one profile only, and holes of different edge counts.
    let ring = circle(10.0).with_hole(vec![ProfileEdge::circle(Point2::ORIGIN, 5.0)]);
    let e = loft(&[(at(0.0), ring.clone()), (at(10.0), circle(10.0))]).unwrap_err();
    assert!(e.to_string().contains("same number of holes"), "{e}");
    let square_hole = circle(10.0).with_hole(crate::polygon_edges(&[
        Point2::new(-2.0, -2.0),
        Point2::new(2.0, -2.0),
        Point2::new(2.0, 2.0),
        Point2::new(-2.0, 2.0),
    ]));
    assert!(loft(&[(at(0.0), ring), (at(10.0), square_hole)]).is_err());
    // One profile, the same plane twice, planes out of order, planes not parallel.
    assert!(loft(&[(at(0.0), square(20.0))]).is_err());
    assert!(loft(&[(at(0.0), square(20.0)), (at(0.0), square(10.0))]).is_err());
    assert!(
        loft(&[
            (at(0.0), square(20.0)),
            (at(10.0), square(10.0)),
            (at(5.0), square(10.0))
        ])
        .is_err()
    );
    assert!(loft(&[(at(0.0), square(20.0)), (Frame::XZ, square(10.0))]).is_err());
    // The same square listed clockwise is turned round and matched, not twisted.
    let crossed = Profile::polygon(&[
        Point2::new(10.0, -10.0),
        Point2::new(-10.0, -10.0),
        Point2::new(-10.0, 10.0),
        Point2::new(10.0, 10.0),
    ]);
    assert!(loft(&[(at(0.0), square(20.0)), (at(10.0), crossed)]).is_ok());
}

/// (V - E + F, genus) after a full validity check. With `R` inner loops over all faces,
/// Euler-Poincaré gives V - E + F - R = 2 (S - G).
fn euler(b: &Body) -> (i64, usize) {
    let r = b.validate().unwrap();
    let s = r.summary;
    assert_eq!(s.shells, 1);
    (s.vertices as i64 - s.edges as i64 + s.faces as i64, r.genus)
}

#[test]
fn a_frustum_with_a_coaxial_cylindrical_hole() {
    // Circles R = 10 at z = 0 and 5 at z = 10, each with a hole r = 2 about the same
    // axis: the cone frustum π h (R0² + R0 R1 + R1²) / 3 = 1750π / 3 less the cylinder
    // π r² h = 40π, so 1630π / 3. Genus 1 with a ring (inner loop) in each cap:
    // V - E + F - R = 2 - 2g gives V - E + F = 2 - 2 + 2 = 2.
    let holed = |r: f64| circle(r).with_hole(vec![ProfileEdge::circle(Point2::ORIGIN, 2.0)]);
    let b = loft(&[(at(0.0), holed(10.0)), (at(10.0), holed(5.0))]).unwrap();
    assert_eq!(euler(&b), (2, 1));
    let v = b.volume().unwrap();
    assert!((v - 1630.0 * PI / 3.0).abs() < 1e-9, "{v}");
    // Area: the cone's side π (R0 + R1) √(h² + (R0 - R1)²) = 15π √125, the hole's side
    // 2π r h = 40π, the caps π (100 - 4) and π (25 - 4).
    let a = 15.0 * PI * 125f64.sqrt() + 40.0 * PI + 96.0 * PI + 21.0 * PI;
    assert!(
        (b.area().unwrap() - a).abs() < 1e-9,
        "{}",
        b.area().unwrap()
    );
    let names: Vec<String> = b
        .faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).unwrap().map(str::to_owned))
        .collect();
    for n in ["start", "end", "band 0 side 0", "band 0 hole 0 side 0"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
}

#[test]
fn square_tubes_with_two_holes_match_their_holes_by_position() {
    // A 40 x 40 square to a 20 x 20 one, 10 apart, each with two 4 x 4 square holes
    // centred at x = ±10 (bottom) and x = ±5 (top), the second profile listing its holes
    // the other way round. Each hole is a frustum of squares too (the holes move in but
    // keep their size: a prism leaning in, volume A h = 160). The outer frustum is
    // h/3 (A0 + A1 + √(A0 A1)) = 10/3 (1600 + 400 + 800) = 28000/3; less two holes of
    // 160 each: 28000/3 - 320. Genus 2 with four rings: V - E + F = 2 - 4 + 4 = 2.
    let hole = |cx: f64| {
        vec![
            ProfileEdge::line(Point2::new(cx - 2.0, -2.0), Point2::new(cx + 2.0, -2.0)),
            ProfileEdge::line(Point2::new(cx + 2.0, -2.0), Point2::new(cx + 2.0, 2.0)),
            ProfileEdge::line(Point2::new(cx + 2.0, 2.0), Point2::new(cx - 2.0, 2.0)),
            ProfileEdge::line(Point2::new(cx - 2.0, 2.0), Point2::new(cx - 2.0, -2.0)),
        ]
    };
    let bottom = square(40.0).with_hole(hole(-10.0)).with_hole(hole(10.0));
    let top = square(20.0).with_hole(hole(5.0)).with_hole(hole(-5.0));
    let b = loft(&[(at(0.0), bottom), (at(10.0), top)]).unwrap();
    assert_eq!(euler(&b), (2, 2));
    let v = b.volume().unwrap();
    assert!((v - (28_000.0 / 3.0 - 320.0)).abs() < 1e-9, "{v}");
}

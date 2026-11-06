use super::*;
use crate::{KernelError, Surface};
use chamfera_geom::{Point2, Point3, Vec3};
use std::f64::consts::{FRAC_PI_2, PI};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

fn line(a: Point3, b: Point3) -> PathSegment {
    PathSegment::Line { start: a, end: b }
}

/// The quarter arc of radius 20 from (30, 0, 0), heading +x, to (50, 20, 0), heading +y.
fn quarter() -> PathSegment {
    PathSegment::arc_from_points(
        p(30.0, 20.0, 0.0),
        Vec3::Z,
        p(30.0, 0.0, 0.0),
        p(50.0, 20.0, 0.0),
    )
    .unwrap()
}

/// The L path: 30 along +x, the quarter arc, 30 along +y.
fn l_path() -> Vec<PathSegment> {
    vec![
        line(p(0.0, 0.0, 0.0), p(30.0, 0.0, 0.0)),
        quarter(),
        line(p(50.0, 20.0, 0.0), p(50.0, 50.0, 0.0)),
    ]
}

/// The YZ plane (normal +x) through the origin: the profile's plane.
fn yz() -> Frame {
    Frame::YZ
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

fn check_valid(b: &Body) {
    let r = b.validate().unwrap();
    // One shell, genus 0: V - E + F = 2.
    let s = r.summary;
    assert_eq!(s.shells, 1);
    assert_eq!(
        s.vertices as i64 - s.edges as i64 + s.faces as i64,
        2,
        "{s:?}"
    );
}

#[test]
fn a_square_along_a_line_is_a_box() {
    let b = sweep_path(
        &yz(),
        &square(4.0),
        &[line(p(0.0, 0.0, 0.0), p(30.0, 0.0, 0.0))],
    )
    .unwrap();
    check_valid(&b);
    // 4 × 4 × 30 = 480.
    assert!((b.volume().unwrap() - 480.0).abs() < 1e-9);
    // 2 · 16 + 4 · (4 · 30) = 512.
    assert!((b.area().unwrap() - 512.0).abs() < 1e-9);
    let names: Vec<String> = b
        .faces()
        .into_iter()
        .filter_map(|f| b.face_name(f).unwrap().map(str::to_owned))
        .collect();
    for n in ["start", "end", "segment 0 side 0", "segment 0 side 3"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
}

#[test]
fn a_circle_along_a_quarter_arc_obeys_pappus() {
    let frame = Frame::YZ.with_origin(p(30.0, 0.0, 0.0)).unwrap();
    let b = sweep_path(&frame, &Profile::circle(Point2::ORIGIN, 2.0), &[quarter()]).unwrap();
    check_valid(&b);
    // Pappus: area π·2² times the centroid's path, a quarter of 2π·20.
    let v = PI * 4.0 * (FRAC_PI_2 * 20.0);
    assert!(
        (b.volume().unwrap() - v).abs() < 1e-6 * v,
        "{}",
        b.volume().unwrap()
    );
    // Lateral area 2π·2 · 10π plus two discs of π·4.
    let a = 2.0 * PI * 2.0 * (FRAC_PI_2 * 20.0) + 2.0 * PI * 4.0;
    assert!((b.area().unwrap() - a).abs() < 1e-6 * a);
    // The tube is a torus piece.
    assert!(
        b.faces()
            .into_iter()
            .any(|f| matches!(b.face_surface(f).unwrap(), Some((Surface::Torus(_), _))))
    );
}

#[test]
fn an_l_path_equals_the_sum_of_its_parts() {
    for (profile, area) in [
        (Profile::circle(Point2::ORIGIN, 2.0), PI * 4.0),
        (square(4.0), 16.0),
    ] {
        let b = sweep_path(&yz(), &profile, &l_path()).unwrap();
        check_valid(&b);
        // Each part: the profile's area times its length along the centroid's path
        // (Pappus for the arc): 30 + 10π + 30.
        let v = area * (60.0 + 10.0 * PI);
        assert!(
            (b.volume().unwrap() - v).abs() < 1e-6 * v,
            "{}",
            b.volume().unwrap()
        );
        let edges: Vec<String> = b
            .edges()
            .into_iter()
            .filter_map(|e| b.edge_name(e).unwrap().map(str::to_owned))
            .collect();
        for n in ["start 0", "end 0", "joint 1 0", "joint 2 0"] {
            assert!(edges.iter().any(|x| x == n), "{n} in {edges:?}");
        }
    }
}

#[test]
fn the_path_may_come_in_any_order_and_direction() {
    let mut path = l_path();
    path.reverse();
    path[1] = path[1].reversed();
    let b = sweep_path(&yz(), &square(4.0), &path).unwrap();
    assert!((b.volume().unwrap() - 16.0 * (60.0 + 10.0 * PI)).abs() < 1e-6);
}

#[test]
fn a_profile_with_a_hole_sweeps_into_a_pipe() {
    let pipe = Profile::circle(Point2::ORIGIN, 3.0)
        .with_hole(vec![ProfileEdge::circle(Point2::ORIGIN, 2.0)]);
    let b = sweep_path(&yz(), &pipe, &l_path()).unwrap();
    let r = b.validate().unwrap();
    assert_eq!(r.summary.shells, 1);
    // Annulus π(9 - 4) along 60 + 10π.
    let v = 5.0 * PI * (60.0 + 10.0 * PI);
    assert!((b.volume().unwrap() - v).abs() < 1e-6 * v);
}

#[test]
fn a_flipped_profile_plane_still_sweeps_forward() {
    let b = sweep_path(&yz().flipped(), &square(4.0), &l_path()).unwrap();
    assert!((b.volume().unwrap() - 16.0 * (60.0 + 10.0 * PI)).abs() < 1e-6);
}

#[test]
fn a_half_turn_and_more_is_built_in_two_halves() {
    // 270° about (0, 10, 0) from the origin, after a lead-in along +x.
    let arc = PathSegment::Arc {
        centre: p(0.0, 10.0, 0.0),
        axis: Vec3::Z,
        start: p(0.0, 0.0, 0.0),
        angle: 1.5 * PI,
    };
    let path = [line(p(-30.0, 0.0, 0.0), p(0.0, 0.0, 0.0)), arc];
    let frame = Frame::YZ.with_origin(p(-30.0, 0.0, 0.0)).unwrap();
    let b = sweep_path(&frame, &Profile::circle(Point2::ORIGIN, 1.0), &path).unwrap();
    check_valid(&b);
    let v = PI * (30.0 + 1.5 * PI * 10.0);
    assert!((b.volume().unwrap() - v).abs() < 1e-6 * v);
}

#[test]
fn corners_large_profiles_and_crossings_are_refused() {
    let circle = Profile::circle(Point2::ORIGIN, 2.0);
    // A corner: +x then +y.
    let corner = [
        line(p(0.0, 0.0, 0.0), p(30.0, 0.0, 0.0)),
        line(p(30.0, 0.0, 0.0), p(30.0, 30.0, 0.0)),
    ];
    let e = sweep_path(&yz(), &circle, &corner).unwrap_err();
    assert!(e.to_string().contains("corner"), "{e}");
    // A profile reaching past the arc's radius (25 > 20).
    let e = sweep_path(&yz(), &Profile::circle(Point2::ORIGIN, 25.0), &l_path()).unwrap_err();
    assert!(e.to_string().contains("radius"), "{e}");
    // Exactly at the radius is refused too.
    assert!(sweep_path(&yz(), &Profile::circle(Point2::ORIGIN, 20.0), &l_path()).is_err());
    // A path that is not perpendicular to the profile.
    let slanted = [line(p(0.0, 0.0, 0.0), p(30.0, 30.0, 0.0))];
    assert!(sweep_path(&yz(), &circle, &slanted).is_err());
    // A path that does not start on the profile's plane.
    let away = [line(p(5.0, 0.0, 0.0), p(30.0, 0.0, 0.0))];
    assert!(sweep_path(&yz(), &circle, &away).is_err());
    // A loop that comes back across its own lead-in: 300° about (0, 10, 0), then a line
    // along the end tangent (cos 210°, ...) rotated: it crosses y = 0 at x ≈ -5.8.
    let arc = PathSegment::Arc {
        centre: p(0.0, 10.0, 0.0),
        axis: Vec3::Z,
        start: p(0.0, 0.0, 0.0),
        angle: 300f64.to_radians(),
    };
    let end = arc.end();
    let t = arc.tangent_at(1.0).unwrap();
    let crossing = [
        line(p(-30.0, 0.0, 0.0), p(0.0, 0.0, 0.0)),
        arc,
        line(end, end + t * 20.0),
    ];
    let frame = Frame::YZ.with_origin(p(-30.0, 0.0, 0.0)).unwrap();
    let e = sweep_path(&frame, &circle, &crossing).unwrap_err();
    assert!(e.to_string().contains("cross itself"), "{e}");
    // Branching and broken paths.
    let branch = [
        line(p(0.0, 0.0, 0.0), p(30.0, 0.0, 0.0)),
        line(p(30.0, 0.0, 0.0), p(60.0, 0.0, 0.0)),
        line(p(30.0, 0.0, 0.0), p(30.0, 0.0, 30.0)),
    ];
    assert!(sweep_path(&yz(), &circle, &branch).is_err());
    let broken = [
        line(p(0.0, 0.0, 0.0), p(30.0, 0.0, 0.0)),
        line(p(31.0, 0.0, 0.0), p(60.0, 0.0, 0.0)),
    ];
    assert!(sweep_path(&yz(), &circle, &broken).is_err());
    assert!(matches!(
        sweep_path(&yz(), &circle, &[]),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn hostile_paths_are_refused_without_panicking() {
    let circle = Profile::circle(Point2::ORIGIN, 1.0);
    for seg in [
        line(p(0.0, 0.0, 0.0), p(f64::NAN, 0.0, 0.0)),
        line(p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0)),
        line(p(0.0, 0.0, 0.0), p(1e300, 0.0, 0.0)),
        PathSegment::Arc {
            centre: p(0.0, 5.0, 0.0),
            axis: Vec3::new(0.0, 0.0, 2.0),
            start: Point3::ORIGIN,
            angle: 1.0,
        },
        PathSegment::Arc {
            centre: p(0.0, 5.0, 0.0),
            axis: Vec3::Z,
            start: Point3::ORIGIN,
            angle: f64::INFINITY,
        },
        PathSegment::Arc {
            centre: p(0.0, 5.0, 0.0),
            axis: Vec3::Z,
            start: Point3::ORIGIN,
            angle: 7.0,
        },
    ] {
        assert!(sweep_path(&yz(), &circle, &[seg]).is_err(), "{seg:?}");
    }
    let many = vec![line(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)); MAX_PATH_SEGMENTS + 1];
    assert!(sweep_path(&yz(), &circle, &many).is_err());
}

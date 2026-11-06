//! Tests for spline profiles and their extrusions (rung K10). Expected values are closed
//! forms derived in the comments.

use crate::{
    Body, Curve, KernelError, Line2, Profile, ProfileEdge, Surface, TessOptions, extrude,
    extrude_symmetric, fillet, revolve, tessellate, union,
};
use chamfera_geom::{
    BSplineCurve2, Frame, LINEAR_TOLERANCE, Point2, Point3, Transform, Vec2, Vec3,
};
use std::collections::HashMap;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

/// The circle of radius `r` about `c` as one rational quadratic B-spline: nine control
/// points on the square around it, the corners with weight √2/2 (The NURBS Book, §7.3).
fn rational_circle(c: Point2, r: f64) -> BSplineCurve2 {
    let w = FRAC_1_SQRT_2;
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
    BSplineCurve2::new(
        2,
        vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ],
        corners
            .iter()
            .map(|(x, y)| pt(c.x + r * x, c.y + r * y))
            .collect(),
        Some(vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0]),
    )
    .unwrap()
}

/// The rational circle as a profile of two half-circle splines.
fn spline_disc(c: Point2, r: f64) -> Profile {
    let (a, b) = rational_circle(c, r).split_at(0.5).unwrap();
    Profile::new(vec![ProfileEdge::Spline(a), ProfileEdge::Spline(b)], vec![])
}

/// The arch: the fit spline through (0, 0), (10, 10), (20, 0), closed by a line. With
/// equal chords the parameters are 0, ½, 1 and the quadratic through them is
/// x = 20 t, y = 40 t (1 - t): the parabola y = x (20 - x) / 10.
fn arch() -> Profile {
    let s = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)], None, None)
        .unwrap();
    Profile::new(
        vec![
            ProfileEdge::Spline(s),
            ProfileEdge::line(pt(20.0, 0.0), pt(0.0, 0.0)),
        ],
        vec![],
    )
}

/// Arc length of y = x (20 - x) / 10 over [0, 20]: with s = 2 - x/5, ds = -dx/5,
/// L = 5 ∫_{-2}^{2} √(1 + s²) ds = 10 [s √(1+s²) / 2 + asinh(s) / 2]_0^2
///   = 10 √5 + 5 asinh 2.
fn arch_length() -> f64 {
    10.0 * 5f64.sqrt() + 5.0 * 2f64.asinh()
}

/// Mesh checks: closed and consistently oriented, volume within the chord tolerance.
fn check_mesh(body: &Body, volume: f64) {
    let opts = TessOptions::default();
    let m = tessellate(body, &opts).unwrap();
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for t in &m.mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "edge {a}->{b} used {n} times");
        assert_eq!(directed.get(&(b, a)), Some(&1), "edge {a}->{b} unpaired");
    }
    // A mesh inscribed within the chord tolerance misses at most area · tolerance.
    let area = body.area().unwrap();
    let mv = m.mesh.signed_volume();
    assert!(
        (mv - volume).abs() <= area * opts.chord_tolerance,
        "mesh volume {mv} vs {volume}"
    );
}

#[test]
fn a_rational_circle_profile_has_area_pi_r_squared() {
    // r = 3: π r² = 9π.
    let p = spline_disc(pt(1.0, -2.0), 3.0);
    assert!(close(p.area().unwrap(), 9.0 * PI, 1e-11));
    // Unit circle: π.
    assert!(close(
        spline_disc(pt(0.0, 0.0), 1.0).area().unwrap(),
        PI,
        1e-11
    ));
}

#[test]
fn an_extruded_rational_circle_is_a_cylinder_by_its_measures() {
    // r = 3, h = 7: V = π r² h = 63π, A = 2π r² + 2π r h = 18π + 42π = 60π.
    let body = extrude(&Frame::XY, &spline_disc(pt(0.0, 0.0), 3.0), 7.0).unwrap();
    body.validate().unwrap();
    // Two caps and two side faces on extrusions; six edges, four vertices:
    // V - E + F = 4 - 6 + 4 = 2.
    assert_eq!(body.faces().len(), 4);
    assert_eq!(body.edges().len(), 6);
    let sides = body
        .faces()
        .into_iter()
        .filter(|f| matches!(body.face_surface(*f), Ok(Some((Surface::Extrusion(_), _)))))
        .count();
    assert_eq!(sides, 2);
    assert!(close(body.volume().unwrap(), 63.0 * PI, 1e-10));
    assert!(close(body.area().unwrap(), 60.0 * PI, 1e-10));
    check_mesh(&body, 63.0 * PI);
}

#[test]
fn an_extruded_spline_profile_has_volume_area_times_height() {
    // Archimedes: the parabolic segment has 2/3 of its base times height,
    // 2/3 · 20 · 10 = 400/3; h = 5 gives V = 2000/3.
    let p = arch();
    let a = p.area().unwrap();
    assert!(close(a, 400.0 / 3.0, 1e-12), "{a}");
    let h = 5.0;
    let body = extrude(&Frame::XY, &p, h).unwrap();
    body.validate().unwrap();
    assert!(close(body.volume().unwrap(), 2000.0 / 3.0, 1e-10));
    // Area: two caps plus the sides (arc length and the 20 mm base, times h).
    let area = 2.0 * 400.0 / 3.0 + h * (arch_length() + 20.0);
    assert!(
        close(body.area().unwrap(), area, 1e-10),
        "{}",
        body.area().unwrap()
    );
    check_mesh(&body, 2000.0 / 3.0);
    // Downwards, symmetric and on a tilted plane: the same volume.
    let down = extrude(&Frame::XY, &p, -h).unwrap();
    assert!(close(down.volume().unwrap(), 2000.0 / 3.0, 1e-10));
    let sym = extrude_symmetric(&Frame::XY, &p, h).unwrap();
    assert!(close(sym.volume().unwrap(), 2000.0 / 3.0, 1e-10));
    let tilted = Frame::from_normal_and_x(
        Point3::new(5.0, -3.0, 2.0),
        Vec3::new(1.0, 2.0, 2.0),
        Vec3::X,
    )
    .unwrap();
    let t = extrude(&tilted, &p, h).unwrap();
    t.validate().unwrap();
    assert!(close(t.volume().unwrap(), 2000.0 / 3.0, 1e-10));
}

#[test]
fn a_spline_profile_with_a_spline_hole_subtracts_its_area() {
    // A 40 x 40 square with the rational circle r = 5 as a hole: 1600 - 25π, extruded
    // by 2: 3200 - 50π.
    let square = crate::polygon_edges(&[
        pt(-20.0, -20.0),
        pt(20.0, -20.0),
        pt(20.0, 20.0),
        pt(-20.0, 20.0),
    ]);
    let hole = spline_disc(pt(0.0, 0.0), 5.0).outer;
    let p = Profile::new(square, vec![hole]);
    assert!(close(p.area().unwrap(), 1600.0 - 25.0 * PI, 1e-11));
    let body = extrude(&Frame::XY, &p, 2.0).unwrap();
    body.validate().unwrap();
    assert!(close(body.volume().unwrap(), 3200.0 - 50.0 * PI, 1e-10));
    check_mesh(&body, 3200.0 - 50.0 * PI);
}

#[test]
fn spline_bodies_move_and_round_trip() {
    let body = extrude(&Frame::XY, &arch(), 5.0).unwrap();
    let t = Transform::rotation_about(Point3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 0.0), 0.7)
        .unwrap();
    let moved = body.transformed(&t).unwrap();
    moved.validate().unwrap();
    assert!(close(moved.volume().unwrap(), 2000.0 / 3.0, 1e-10));
    let json = serde_json::to_string(&body).unwrap();
    let back: Body = serde_json::from_str(&json).unwrap();
    back.validate().unwrap();
    assert!(close(back.volume().unwrap(), 2000.0 / 3.0, 1e-10));
    assert!(
        back.edges()
            .iter()
            .any(|e| matches!(back.edge_curve(*e), Ok(Some((Curve::BSpline(_), _, _)))))
    );
}

#[test]
fn operations_that_do_not_handle_splines_yet_refuse_cleanly() {
    let body = extrude(&Frame::XY, &arch(), 5.0).unwrap();
    let block = extrude(
        &Frame::XY,
        &Profile::polygon(&[pt(0.0, 0.0), pt(5.0, 0.0), pt(5.0, 5.0), pt(0.0, 5.0)]),
        3.0,
    )
    .unwrap();
    assert!(matches!(
        union(&body, &block),
        Err(KernelError::Unsupported(_))
    ));
    assert!(matches!(
        union(&block, &body),
        Err(KernelError::Unsupported(_))
    ));
    let e = body.edges()[0];
    assert!(matches!(
        fillet(&body, &[e], 1.0),
        Err(KernelError::Unsupported(_))
    ));
    let axis = Line2::new(pt(0.0, -5.0), Vec2::X).unwrap();
    assert!(matches!(
        revolve(&Frame::XY, &arch(), &axis, 1.0),
        Err(KernelError::Unsupported(_))
    ));
}

#[test]
fn hostile_spline_profiles_are_refused() {
    // A closed spline as one edge must be given as two.
    let whole = Profile::new(
        vec![ProfileEdge::Spline(rational_circle(pt(0.0, 0.0), 2.0))],
        vec![],
    );
    assert!(whole.area().is_err());
    // A spline that does not close with the rest of the loop.
    let s = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)], None, None)
        .unwrap();
    let open = Profile::new(
        vec![
            ProfileEdge::Spline(s.clone()),
            ProfileEdge::line(pt(20.0, 0.0), pt(1.0, 0.0)),
        ],
        vec![],
    );
    assert!(open.area().is_err());
    // A line that crosses the spline: the loop is not simple.
    let crossing = Profile::new(
        vec![
            ProfileEdge::Spline(s),
            ProfileEdge::line(pt(20.0, 0.0), pt(10.0, 20.0)),
            ProfileEdge::line(pt(10.0, 20.0), pt(0.0, 0.0)),
        ],
        vec![],
    );
    // The two lines run outside the arch, from its ends up to (10, 20), and meet it only
    // at its ends: this one is valid (the arch's top is at 10 < 20).
    assert!(crossing.area().is_ok());
    let s = BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)], None, None)
        .unwrap();
    let through = Profile::new(
        vec![
            ProfileEdge::Spline(s),
            ProfileEdge::line(pt(20.0, 0.0), pt(10.0, 5.0)),
            ProfileEdge::line(pt(10.0, 5.0), pt(10.0, 15.0)),
            ProfileEdge::line(pt(10.0, 15.0), pt(0.0, 0.0)),
        ],
        vec![],
    );
    assert!(
        through.area().is_err(),
        "the line from (10, 5) to (10, 15) crosses the arch"
    );
    // A spline through (0,0), (10,10), (10,0), (0,10) crosses itself (the segment from
    // the second point to the fourth crosses the one from the first to the third).
    let bow = BSplineCurve2::interpolate(
        &[pt(0.0, 0.0), pt(10.0, 10.0), pt(10.0, 0.0), pt(0.0, 10.0)],
        None,
        None,
    )
    .unwrap();
    let bowtie = Profile::new(
        vec![
            ProfileEdge::Spline(bow),
            ProfileEdge::line(pt(0.0, 10.0), pt(0.0, 0.0)),
        ],
        vec![],
    );
    assert!(bowtie.area().is_err());
    // Tiny: a spline shorter than the tolerance cannot be built at all.
    assert!(
        BSplineCurve2::interpolate(&[pt(0.0, 0.0), pt(LINEAR_TOLERANCE * 0.5, 0.0)], None, None)
            .is_err()
    );
}

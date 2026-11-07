//! End-to-end solver scenarios with analytic expected values.

use crate::*;
use chamfera_geom::{LINEAR_TOLERANCE, Point2};
use std::f64::consts::{FRAC_PI_6, PI};
use std::time::Instant;

/// Agreement expected from a converged solve (residuals are at most `LINEAR_TOLERANCE`,
/// usually far less).
const TOL: f64 = 1e-7;

fn pt(s: &mut Sketch, x: f64, y: f64) -> PointId {
    s.add_point(Point2::new(x, y)).unwrap()
}

fn at(s: &Sketch, id: PointId) -> Point2 {
    s.point(id).unwrap()
}

fn assert_at(s: &Sketch, id: PointId, x: f64, y: f64) {
    let p = at(s, id);
    assert!(
        (p.x - x).abs() < TOL && (p.y - y).abs() < TOL,
        "{id} is at ({}, {}), expected ({x}, {y})",
        p.x,
        p.y
    );
}

fn add(s: &mut Sketch, c: Constraint) -> ConstraintId {
    s.add_constraint(c).unwrap()
}

fn fix_at(s: &mut Sketch, point: PointId, x: f64, y: f64) -> ConstraintId {
    add(
        s,
        Constraint::Fix {
            point,
            at: Point2::new(x, y),
        },
    )
}

/// Equal up to the last bit of a float: serde_json without its `float_roundtrip` feature
/// may read a printed f64 back one ulp off.
fn assert_same_sketch(a: &Sketch, b: &Sketch) {
    assert_eq!(a.points().count(), b.points().count());
    for ((ia, pa), (ib, pb)) in a.points().zip(b.points()) {
        assert_eq!(ia, ib);
        assert!(pa.distance(pb) <= 1e-12 * (1.0 + pa.x.abs() + pa.y.abs()));
    }
    assert_eq!(a.curves().count(), b.curves().count());
    for ((ia, ca), (ib, cb)) in a.curves().zip(b.curves()) {
        assert_eq!(ia, ib);
        assert_eq!(ca.points(), cb.points());
        assert_eq!(ca.is_construction(), cb.is_construction());
    }
    assert_eq!(
        a.constraints().map(|(i, _)| i).collect::<Vec<_>>(),
        b.constraints().map(|(i, _)| i).collect::<Vec<_>>()
    );
}

/// Signed distance of `p` from the line through `a` and `b`.
fn line_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    (b - a).cross(p - a) / (b - a).length()
}

/// Four separate lines joined by coincident constraints, roughly a 50 × 30 rectangle with
/// its corners a little off. Returns the 8 points (two per corner, in order bottom-left,
/// bottom-right ×2, top-right ×2, top-left ×2, bottom-left), the lines (bottom, right,
/// top, left) and the length constraints of the bottom and right sides.
struct Rect {
    p: Vec<PointId>,
    lines: Vec<CurveId>,
    width: ConstraintId,
}

fn rectangle(s: &mut Sketch, with_height: bool) -> Rect {
    let corners = [
        (0.4, -0.3),
        (48.0, 1.0),
        (49.0, 0.5),
        (51.0, 29.0),
        (50.5, 31.0),
        (-1.0, 31.0),
        (-0.5, 29.5),
        (0.3, 0.6),
    ];
    let p: Vec<PointId> = corners.iter().map(|(x, y)| pt(s, *x, *y)).collect();
    let lines: Vec<CurveId> = (0..4)
        .map(|k| s.add_line(p[2 * k], p[2 * k + 1]).unwrap())
        .collect();
    for (a, b) in [(1, 2), (3, 4), (5, 6), (7, 0)] {
        add(s, Constraint::Coincident { a: p[a], b: p[b] });
    }
    add(s, Constraint::Horizontal { line: lines[0] });
    add(s, Constraint::Vertical { line: lines[1] });
    add(s, Constraint::Horizontal { line: lines[2] });
    add(s, Constraint::Vertical { line: lines[3] });
    fix_at(s, p[0], 0.0, 0.0);
    let width = add(
        s,
        Constraint::Length {
            line: lines[0],
            value: 50.0,
        },
    );
    if with_height {
        add(
            s,
            Constraint::Length {
                line: lines[1],
                value: 30.0,
            },
        );
    }
    Rect { p, lines, width }
}

#[test]
fn a_dimensioned_rectangle_solves_to_exact_corners_with_no_freedom() {
    let mut s = Sketch::new();
    let r = rectangle(&mut s, true);
    let report = s.solve().unwrap();
    // 8 points = 16 unknowns; 4 coincident (8) + 4 H/V + fix (2) + 2 lengths = 16 equations.
    assert_eq!(report.dof.dof, 0);
    assert_eq!(report.dof.status, ConstraintStatus::FullyConstrained);
    assert!(report.dof.redundant.is_empty());
    let expected = [
        (0.0, 0.0),
        (50.0, 0.0),
        (50.0, 0.0),
        (50.0, 30.0),
        (50.0, 30.0),
        (0.0, 30.0),
        (0.0, 30.0),
        (0.0, 0.0),
    ];
    for (id, (x, y)) in r.p.iter().zip(expected) {
        assert_at(&s, *id, x, y);
        assert!(report.dof.is_point_determined(*id));
    }
    for l in &r.lines {
        assert!(report.dof.is_curve_determined(*l));
    }
    assert!(report.max_residual <= LINEAR_TOLERANCE);
    let regions = s.profiles();
    assert_eq!(regions.len(), 1);
    assert!((regions[0].area - 1500.0).abs() < 1e-6);
}

#[test]
fn changing_a_dimension_resolves_the_rectangle() {
    let mut s = Sketch::new();
    let r = rectangle(&mut s, true);
    s.solve().unwrap();
    s.try_set_dimension(r.width, 80.0).unwrap();
    assert_at(&s, r.p[3], 80.0, 30.0);
    assert!((s.profiles()[0].area - 2400.0).abs() < 1e-6);
    // A bad value is refused and changes nothing.
    let before = s.clone();
    assert!(s.try_set_dimension(r.width, f64::NAN).is_err());
    assert!(s.try_set_dimension(r.width, -5.0).is_err());
    assert_eq!(s, before);
    // Shrinking to almost nothing and back stays on the same branch (no flip).
    s.try_set_dimension(r.width, 0.5).unwrap();
    assert_at(&s, r.p[1], 0.5, 0.0);
    s.try_set_dimension(r.width, 50.0).unwrap();
    assert_at(&s, r.p[1], 50.0, 0.0);
}

#[test]
fn a_rectangle_without_its_height_has_one_freedom_and_drags_along_it() {
    let mut s = Sketch::new();
    let r = rectangle(&mut s, false);
    let report = s.solve().unwrap();
    assert_eq!(report.dof.dof, 1);
    assert_eq!(report.dof.status, ConstraintStatus::UnderConstrained);
    // The bottom corners are determined; the top corners can move up and down.
    for k in [0, 1, 2, 7] {
        assert!(report.dof.is_point_determined(r.p[k]), "point {k}");
    }
    for k in [3, 4, 5, 6] {
        assert!(!report.dof.is_point_determined(r.p[k]), "point {k}");
    }
    assert!(report.dof.is_curve_determined(r.lines[0]));
    assert!(!report.dof.is_curve_determined(r.lines[2]));
    // Drag the top-right corner up and to the right: the width is fixed, so it only rises.
    let drag = s.drag(r.p[3], Point2::new(70.0, 45.0)).unwrap();
    assert!((drag.reached.x - 50.0).abs() < TOL && (drag.reached.y - 45.0).abs() < TOL);
    assert!((drag.distance_to_target - 20.0).abs() < TOL);
    assert_at(&s, r.p[0], 0.0, 0.0);
    assert_at(&s, r.p[1], 50.0, 0.0);
    assert_at(&s, r.p[4], 50.0, 45.0);
    assert_at(&s, r.p[5], 0.0, 45.0);
    // A drag along the free direction reaches the target exactly.
    let drag = s.drag(r.p[6], Point2::new(0.0, 20.0)).unwrap();
    assert!(drag.distance_to_target < TOL);
    assert_at(&s, r.p[3], 50.0, 20.0);
    // Dragging a determined corner moves nothing.
    let before = s.clone();
    s.drag(r.p[1], Point2::new(60.0, 10.0)).unwrap();
    for (id, p) in before.points() {
        assert!(at(&s, id).distance(p) < TOL);
    }
}

#[test]
fn a_3_4_5_triangle_has_a_right_angle() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 3.2, 0.3);
    let c = pt(&mut s, 3.4, 3.8);
    let ab = s.add_line(a, b).unwrap();
    let bc = s.add_line(b, c).unwrap();
    let ca = s.add_line(c, a).unwrap();
    for (line, value) in [(ab, 3.0), (bc, 4.0), (ca, 5.0)] {
        add(&mut s, Constraint::Length { line, value });
    }
    let report = s.solve().unwrap();
    // 6 unknowns, 3 lengths: a rigid triangle still translates and rotates.
    assert_eq!(report.dof.dof, 3);
    // 3² + 4² = 5², so the angle at B is right: (A - B)·(C - B) = 0.
    let (pa, pb, pc) = (at(&s, a), at(&s, b), at(&s, c));
    assert!((pa - pb).dot(pc - pb).abs() < 1e-6);
    assert!((pa.distance(pc) - 5.0).abs() < TOL);
}

#[test]
fn a_triangle_that_cannot_close_has_no_solution_and_changes_nothing() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 3.0, 0.0);
    let c = pt(&mut s, 3.0, 4.0);
    let ab = s.add_line(a, b).unwrap();
    let bc = s.add_line(b, c).unwrap();
    let ca = s.add_line(c, a).unwrap();
    // 3 + 4 < 10: no triangle has these sides.
    for (line, value) in [(ab, 3.0), (bc, 4.0), (ca, 10.0)] {
        add(&mut s, Constraint::Length { line, value });
    }
    let before = s.clone();
    match s.solve() {
        Err(SolveError::NoSolution { unsatisfied, .. }) => assert!(!unsatisfied.is_empty()),
        other => panic!("expected no solution, got {other:?}"),
    }
    assert_eq!(s, before);
}

#[test]
fn a_line_tangent_to_a_circle_is_one_radius_from_its_centre() {
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let circle = s.add_circle(c, 8.0).unwrap();
    fix_at(&mut s, c, 0.0, 0.0);
    add(
        &mut s,
        Constraint::Radius {
            curve: circle,
            value: 10.0,
        },
    );
    let a = pt(&mut s, -20.0, 12.0);
    let b = pt(&mut s, 20.0, 11.0);
    let line = s.add_line(a, b).unwrap();
    add(&mut s, Constraint::Tangent { a: line, b: circle });
    s.solve().unwrap();
    let d = line_distance(at(&s, c), at(&s, a), at(&s, b));
    // The line runs left to right above the centre, so the centre is on its right: -10.
    assert!((d + 10.0).abs() < TOL, "distance {d}");
    assert!((s.radius(circle).unwrap() - 10.0).abs() < TOL);
}

#[test]
fn circles_touch_outside_or_inside_whichever_is_nearer() {
    let mut s = Sketch::new();
    let c1 = pt(&mut s, 0.0, 0.0);
    let k1 = s.add_circle(c1, 5.0).unwrap();
    fix_at(&mut s, c1, 0.0, 0.0);
    add(
        &mut s,
        Constraint::Radius {
            curve: k1,
            value: 5.0,
        },
    );
    let c2 = pt(&mut s, 12.0, 1.0);
    let k2 = s.add_circle(c2, 4.0).unwrap();
    add(
        &mut s,
        Constraint::Radius {
            curve: k2,
            value: 4.0,
        },
    );
    add(&mut s, Constraint::Tangent { a: k1, b: k2 });
    // Big circle r=10 around c1 and a small one r=3 near its centre: inside tangency.
    let k3 = s.add_circle(c1, 10.0).unwrap();
    add(
        &mut s,
        Constraint::Radius {
            curve: k3,
            value: 10.0,
        },
    );
    let c4 = pt(&mut s, 3.0, 0.0);
    let k4 = s.add_circle(c4, 3.0).unwrap();
    add(
        &mut s,
        Constraint::Radius {
            curve: k4,
            value: 3.0,
        },
    );
    add(&mut s, Constraint::Tangent { a: k3, b: k4 });
    s.solve().unwrap();
    // Outside: |c1 c2| = 5 + 4. Inside: |c1 c4| = 10 - 3.
    assert!((at(&s, c1).distance(at(&s, c2)) - 9.0).abs() < TOL);
    assert!((at(&s, c1).distance(at(&s, c4)) - 7.0).abs() < TOL);
}

/// A 10 × 10 square on shared corners, fully constrained by H/V, a fixed corner, and the
/// bottom and left lengths. Returns the corner points and lines (bottom, right, top, left).
fn fixed_square(s: &mut Sketch) -> (Vec<PointId>, Vec<CurveId>) {
    let p: Vec<PointId> = [(0.0, 0.0), (10.5, 0.2), (9.8, 10.3), (0.1, 9.6)]
        .iter()
        .map(|(x, y)| pt(s, *x, *y))
        .collect();
    let l: Vec<CurveId> = (0..4)
        .map(|k| s.add_line(p[k], p[(k + 1) % 4]).unwrap())
        .collect();
    add(s, Constraint::Horizontal { line: l[0] });
    add(s, Constraint::Vertical { line: l[1] });
    add(s, Constraint::Horizontal { line: l[2] });
    add(s, Constraint::Vertical { line: l[3] });
    fix_at(s, p[0], 0.0, 0.0);
    add(
        s,
        Constraint::Length {
            line: l[0],
            value: 10.0,
        },
    );
    add(
        s,
        Constraint::Length {
            line: l[3],
            value: 10.0,
        },
    );
    (p, l)
}

#[test]
fn a_conflicting_length_is_named_and_the_sketch_is_unchanged() {
    let mut s = Sketch::new();
    let (p, l) = fixed_square(&mut s);
    assert_eq!(s.solve().unwrap().dof.dof, 0);
    assert_at(&s, p[2], 10.0, 10.0);
    let before = s.clone();
    // The top is already 10 long; asking for 12 conflicts.
    let err = s
        .try_add_constraint(Constraint::Length {
            line: l[2],
            value: 12.0,
        })
        .unwrap_err();
    assert_eq!(s, before, "the sketch must be exactly as it was");
    let SolveError::Conflicting { conflicting, .. } = err else {
        panic!("expected a conflict, got {err:?}");
    };
    assert_eq!(conflicting.len(), 1);
    // Without try_: add then solve; the solve fails and leaves the geometry alone.
    let id = add(
        &mut s,
        Constraint::Length {
            line: l[2],
            value: 12.0,
        },
    );
    let geometry: Vec<_> = s.points().collect();
    match s.solve() {
        Err(SolveError::Conflicting { conflicting, .. }) => assert_eq!(conflicting, vec![id]),
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(s.points().collect::<Vec<_>>(), geometry);
}

#[test]
fn a_redundant_length_is_reported_redundant() {
    let mut s = Sketch::new();
    let (_, l) = fixed_square(&mut s);
    s.solve().unwrap();
    let (id, report) = s
        .try_add_constraint(Constraint::Length {
            line: l[2],
            value: 10.0,
        })
        .unwrap();
    assert_eq!(report.dof.redundant, vec![id]);
    assert_eq!(report.dof.status, ConstraintStatus::OverConstrained);
    assert_eq!(report.dof.dof, 0);
}

#[test]
fn free_entities_have_their_textbook_degrees_of_freedom() {
    // Point: x, y.
    let mut s = Sketch::new();
    pt(&mut s, 1.0, 2.0);
    assert_eq!(s.analyze().unwrap().dof, 2);
    // Line: two free points.
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 4.0, 1.0);
    s.add_line(a, b).unwrap();
    assert_eq!(s.analyze().unwrap().dof, 4);
    // Circle: centre and radius.
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    s.add_circle(c, 3.0).unwrap();
    assert_eq!(s.analyze().unwrap().dof, 3);
    // Arc: three points, less the end-on-the-start's-radius equation.
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let a = pt(&mut s, 5.0, 0.0);
    let b = pt(&mut s, 0.0, 5.0);
    s.add_arc(c, a, b).unwrap();
    let report = s.analyze().unwrap();
    assert_eq!(report.dof, 5);
    assert_eq!(report.status, ConstraintStatus::UnderConstrained);
    // Empty sketch: nothing can move, nothing is wrong.
    let mut s = Sketch::new();
    let report = s.solve().unwrap();
    assert_eq!(report.dof.dof, 0);
    assert_eq!(report.dof.status, ConstraintStatus::FullyConstrained);
    assert!(s.profiles().is_empty());
}

#[test]
fn an_arc_end_is_pulled_onto_the_arc_radius() {
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let a = pt(&mut s, 5.0, 0.0);
    let b = pt(&mut s, 0.0, 6.0);
    s.add_arc(c, a, b).unwrap();
    s.solve().unwrap();
    let r_start = at(&s, a).distance(at(&s, c));
    let r_end = at(&s, b).distance(at(&s, c));
    assert!((r_start - r_end).abs() < TOL);
    // The smallest move splits the 1 mm difference: both radii end near 5.5.
    assert!((r_start - 5.5).abs() < 0.01);
}

#[test]
fn every_constraint_kind_reaches_its_analytic_value() {
    let mut s = Sketch::new();
    // A fixed horizontal base line from (0,0) to (10,0).
    let o = pt(&mut s, 0.0, 0.0);
    let e = pt(&mut s, 10.0, 0.0);
    let base = s.add_line(o, e).unwrap();
    fix_at(&mut s, o, 0.0, 0.0);
    fix_at(&mut s, e, 10.0, 0.0);

    // Midpoint.
    let m = pt(&mut s, 4.0, 1.0);
    add(
        &mut s,
        Constraint::Midpoint {
            point: m,
            line: base,
        },
    );
    // Symmetric about the base: q mirrors p.
    let p = pt(&mut s, 3.0, 4.0);
    let q = pt(&mut s, 3.5, -3.0);
    fix_at(&mut s, p, 3.0, 4.0);
    add(
        &mut s,
        Constraint::Symmetric {
            a: p,
            b: q,
            line: base,
        },
    );
    // A line from o at 30° to the base, 8 long.
    let t = pt(&mut s, 7.0, 3.0);
    let slanted = s.add_line(o, t).unwrap();
    add(
        &mut s,
        Constraint::Angle {
            a: base,
            b: slanted,
            value: FRAC_PI_6,
        },
    );
    add(
        &mut s,
        Constraint::Length {
            line: slanted,
            value: 8.0,
        },
    );
    // A perpendicular to the slanted line through t, equal in length to the base.
    let u = pt(&mut s, 3.0, 12.0);
    let perp = s.add_line(t, u).unwrap();
    add(
        &mut s,
        Constraint::Perpendicular {
            a: slanted,
            b: perp,
        },
    );
    add(&mut s, Constraint::Equal { a: perp, b: base });
    // A line parallel to the base, 6 above it, from a point 5 from o.
    let v = pt(&mut s, 1.0, 5.0);
    let w = pt(&mut s, 9.0, 6.5);
    let par = s.add_line(v, w).unwrap();
    add(&mut s, Constraint::Parallel { a: base, b: par });
    add(
        &mut s,
        Constraint::PointLineDistance {
            point: v,
            line: base,
            value: 6.0,
        },
    );
    add(
        &mut s,
        Constraint::Distance {
            a: o,
            b: w,
            value: 10.0,
        },
    );
    // A point on the infinite line through the parallel line, 2 from its start along x.
    let on = pt(&mut s, 2.5, 7.0);
    add(
        &mut s,
        Constraint::PointOnLine {
            point: on,
            line: par,
        },
    );
    // Two circles: one by diameter, one equal to it; a point on the second.
    let c1 = pt(&mut s, 20.0, 0.0);
    let k1 = s.add_circle(c1, 2.0).unwrap();
    fix_at(&mut s, c1, 20.0, 0.0);
    add(
        &mut s,
        Constraint::Diameter {
            curve: k1,
            value: 6.0,
        },
    );
    let c2 = pt(&mut s, 30.0, 0.0);
    let k2 = s.add_circle(c2, 4.0).unwrap();
    fix_at(&mut s, c2, 30.0, 0.0);
    add(&mut s, Constraint::Equal { a: k1, b: k2 });
    let r = pt(&mut s, 34.0, 1.0);
    add(
        &mut s,
        Constraint::PointOnCircle {
            point: r,
            curve: k2,
        },
    );
    // An arc around (40,0) of radius 5 and a point on it.
    let ac = pt(&mut s, 40.0, 0.0);
    let a0 = pt(&mut s, 45.0, 0.0);
    let a1 = pt(&mut s, 40.0, 5.0);
    let arc = s.add_arc(ac, a0, a1).unwrap();
    fix_at(&mut s, ac, 40.0, 0.0);
    add(
        &mut s,
        Constraint::Radius {
            curve: arc,
            value: 5.0,
        },
    );
    let on_arc = pt(&mut s, 44.0, 4.0);
    add(
        &mut s,
        Constraint::PointOnCircle {
            point: on_arc,
            curve: arc,
        },
    );

    s.solve().unwrap();

    assert_at(&s, m, 5.0, 0.0);
    assert_at(&s, q, 3.0, -4.0);
    // t = 8 (cos 30°, sin 30°) = (4√3, 4).
    assert_at(&s, t, 4.0 * 3f64.sqrt(), 4.0);
    let (pt_t, pt_u) = (at(&s, t), at(&s, u));
    assert!((pt_t - Point2::ORIGIN).dot(pt_u - pt_t).abs() < 1e-6);
    assert!((pt_t.distance(pt_u) - 10.0).abs() < TOL);
    // Parallel line at y = 6 (v started above the base); w is 10 from o: w = (8, 6).
    assert!((at(&s, v).y - 6.0).abs() < TOL);
    assert_at(&s, w, 8.0, 6.0);
    assert!((at(&s, on).y - 6.0).abs() < TOL);
    assert!((s.radius(k1).unwrap() - 3.0).abs() < TOL);
    assert!((s.radius(k2).unwrap() - 3.0).abs() < TOL);
    assert!((at(&s, r).distance(at(&s, c2)) - 3.0).abs() < TOL);
    assert!((at(&s, on_arc).distance(at(&s, ac)) - 5.0).abs() < TOL);
    assert!((at(&s, a1).distance(at(&s, ac)) - 5.0).abs() < TOL);
}

#[test]
fn a_point_keeps_its_side_of_a_line() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 10.0, 0.0);
    let line = s.add_line(a, b).unwrap();
    fix_at(&mut s, a, 0.0, 0.0);
    fix_at(&mut s, b, 10.0, 0.0);
    let p = pt(&mut s, 5.0, -3.0);
    add(
        &mut s,
        Constraint::PointLineDistance {
            point: p,
            line,
            value: 7.0,
        },
    );
    s.solve().unwrap();
    // Below the line it started, below it stays: y = -7, not +7.
    assert!((at(&s, p).y + 7.0).abs() < TOL);
}

#[test]
fn dragging_a_point_on_a_fixed_circle_stops_at_the_nearest_point() {
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let k = s.add_circle(c, 10.0).unwrap();
    fix_at(&mut s, c, 0.0, 0.0);
    add(
        &mut s,
        Constraint::Radius {
            curve: k,
            value: 10.0,
        },
    );
    let p = pt(&mut s, 10.0, 0.0);
    add(&mut s, Constraint::PointOnCircle { point: p, curve: k });
    let report = s.drag(p, Point2::new(0.0, 30.0)).unwrap();
    // The nearest point of the circle to (0, 30) is (0, 10).
    assert!(report.reached.distance(Point2::new(0.0, 10.0)) < 1e-6);
    assert!((report.distance_to_target - 20.0).abs() < 1e-6);
    // A free point just moves.
    let f = pt(&mut s, 1.0, 1.0);
    s.drag(f, Point2::new(-3.0, 7.0)).unwrap();
    assert_eq!(at(&s, f), Point2::new(-3.0, 7.0));
}

#[test]
fn bad_drags_are_refused_and_change_nothing() {
    let mut s = Sketch::new();
    let p = pt(&mut s, 1.0, 1.0);
    let before = s.clone();
    assert!(s.drag(p, Point2::new(f64::NAN, 0.0)).is_err());
    assert!(s.drag(p, Point2::new(0.0, f64::INFINITY)).is_err());
    assert!(s.drag(p, Point2::new(1e9, 0.0)).is_err());
    assert!(s.drag(PointId(9999), Point2::new(0.0, 0.0)).is_err());
    assert_eq!(s, before);
}

#[test]
fn a_solved_sketch_round_trips_and_solves_to_the_same_place() {
    let mut s = Sketch::new();
    rectangle(&mut s, true);
    s.solve().unwrap();
    let json = serde_json::to_string(&s).unwrap();
    let mut back: Sketch = serde_json::from_str(&json).unwrap();
    assert_same_sketch(&back, &s);
    let report = back.solve().unwrap();
    assert_eq!(report.iterations, 0, "already solved");
    for (id, p) in s.points() {
        assert!(at(&back, id).distance(p) < 1e-12);
    }
}

#[test]
fn degenerate_geometry_never_panics_and_failures_change_nothing() {
    // Zero-length lines under length, perpendicular, parallel and angle constraints.
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 0.0, 0.0);
    let c = pt(&mut s, 0.0, 0.0);
    let l1 = s.add_line(a, b).unwrap();
    let l2 = s.add_line(b, c).unwrap();
    add(&mut s, Constraint::Perpendicular { a: l1, b: l2 });
    add(&mut s, Constraint::Parallel { a: l1, b: l2 });
    add(
        &mut s,
        Constraint::Angle {
            a: l1,
            b: l2,
            value: 1.0,
        },
    );
    add(
        &mut s,
        Constraint::Length {
            line: l1,
            value: 10.0,
        },
    );
    add(&mut s, Constraint::PointOnLine { point: c, line: l1 });
    let before = s.clone();
    if s.solve().is_err() {
        assert_eq!(s, before);
    }
    let _ = s.analyze();
    let _ = s.profiles();
    let _ = s.drag(a, Point2::new(5.0, 5.0));

    // Circles on the same centre asked to touch, with the same and with different points.
    let mut s = Sketch::new();
    let c1 = pt(&mut s, 0.0, 0.0);
    let c2 = pt(&mut s, 0.0, 0.0);
    let k1 = s.add_circle(c1, 5.0).unwrap();
    let k2 = s.add_circle(c2, 5.0).unwrap();
    let k3 = s.add_circle(c1, 5.0).unwrap();
    add(&mut s, Constraint::Tangent { a: k1, b: k2 });
    add(&mut s, Constraint::Tangent { a: k1, b: k3 });
    fix_at(&mut s, c1, 0.0, 0.0);
    fix_at(&mut s, c2, 0.0, 0.0);
    add(
        &mut s,
        Constraint::Radius {
            curve: k1,
            value: 5.0,
        },
    );
    add(
        &mut s,
        Constraint::Radius {
            curve: k2,
            value: 4.0,
        },
    );
    let before = s.clone();
    if s.solve().is_err() {
        assert_eq!(s, before);
    }
    let _ = s.analyze();

    // Huge coordinates and lengths near the working range.
    let mut s = Sketch::new();
    let a = pt(&mut s, -9e5, -9e5);
    let b = pt(&mut s, 9e5, 9e5);
    let l = s.add_line(a, b).unwrap();
    add(
        &mut s,
        Constraint::Length {
            line: l,
            value: 1e6,
        },
    );
    add(&mut s, Constraint::Horizontal { line: l });
    let before = s.clone();
    match s.solve() {
        Ok(_) => {
            for (_, p) in s.points() {
                assert!(p.is_valid());
            }
        }
        Err(_) => assert_eq!(s, before),
    }
    // A radius pushed negative by a conflicting tangency is refused, not stored.
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let k = s.add_circle(c, 1.0).unwrap();
    let a = pt(&mut s, -5.0, 0.0);
    let b = pt(&mut s, 5.0, 0.0);
    let l = s.add_line(a, b).unwrap();
    for p in [c, a, b] {
        s.fix_point(p).unwrap();
    }
    add(&mut s, Constraint::Tangent { a: l, b: k });
    let before = s.clone();
    if s.solve().is_err() {
        assert_eq!(s, before);
    } else {
        assert!(s.radius(k).unwrap() > LINEAR_TOLERANCE);
    }
}

#[test]
fn thousands_of_constraints_solve_in_bounded_time() {
    let mut s = Sketch::new();
    // 300 separate 10 × 10 squares, each fully constrained: 300 × 7 = 2100 constraints.
    let mut corners = Vec::new();
    for k in 0..300 {
        let ox = (k % 20) as f64 * 20.0;
        let oy = (k / 20) as f64 * 20.0;
        let p: Vec<PointId> = [(0.3, -0.2), (10.4, 0.1), (9.7, 10.2), (-0.1, 9.9)]
            .iter()
            .map(|(x, y)| pt(&mut s, ox + x, oy + y))
            .collect();
        let l: Vec<CurveId> = (0..4)
            .map(|i| s.add_line(p[i], p[(i + 1) % 4]).unwrap())
            .collect();
        add(&mut s, Constraint::Horizontal { line: l[0] });
        add(&mut s, Constraint::Vertical { line: l[1] });
        add(&mut s, Constraint::Horizontal { line: l[2] });
        add(&mut s, Constraint::Vertical { line: l[3] });
        fix_at(&mut s, p[0], ox, oy);
        add(
            &mut s,
            Constraint::Length {
                line: l[0],
                value: 10.0,
            },
        );
        add(
            &mut s,
            Constraint::Length {
                line: l[3],
                value: 10.0,
            },
        );
        corners.push((p[2], ox + 10.0, oy + 10.0));
    }
    // And 2000 copies of the same horizontal constraint on one more line: all redundant
    // but the first.
    let a = pt(&mut s, 1000.0, 0.0);
    let b = pt(&mut s, 1010.0, 0.5);
    let line = s.add_line(a, b).unwrap();
    for _ in 0..2000 {
        add(&mut s, Constraint::Horizontal { line });
    }
    let start = Instant::now();
    let report = s.solve().unwrap();
    let elapsed = start.elapsed();
    for (id, x, y) in corners {
        assert_at(&s, id, x, y);
    }
    assert_eq!(report.dof.redundant.len(), 1999);
    // Debug builds are slow; this is a hang detector, not a benchmark.
    assert!(elapsed.as_secs_f64() < 30.0, "took {elapsed:?}");
    assert_eq!(s.profiles().len(), 300);
}

#[test]
fn a_group_too_large_for_the_dense_solver_is_refused_quickly() {
    let mut s = Sketch::new();
    // A chain of 400 points (800 coupled unknowns) joined by distance constraints.
    let ids: Vec<PointId> = (0..400).map(|i| pt(&mut s, i as f64, 0.0)).collect();
    for w in ids.windows(2) {
        add(
            &mut s,
            Constraint::Distance {
                a: w[0],
                b: w[1],
                value: 1.0,
            },
        );
    }
    let before = s.clone();
    let start = Instant::now();
    assert!(matches!(s.solve(), Err(SolveError::TooLarge { .. })));
    assert!(start.elapsed().as_secs_f64() < 5.0);
    assert_eq!(s, before);
}

#[test]
fn a_hundred_entity_staircase_solves_quickly_and_exactly() {
    // 51 points and 50 lines: a staircase of alternating horizontal and vertical steps of
    // 10 mm from a fixed origin. 102 unknowns, 2 + 50 + 50 equations: fully constrained.
    // Point k is at (10·⌈k/2⌉, 10·⌊k/2⌋).
    let mut s = Sketch::new();
    let mut seed: u64 = 7;
    let mut jitter = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 2.0
    };
    let expected = |k: usize| (10.0 * k.div_ceil(2) as f64, 10.0 * (k / 2) as f64);
    let ids: Vec<PointId> = (0..51)
        .map(|k| {
            let (x, y) = expected(k);
            let (dx, dy) = (jitter(), jitter());
            pt(&mut s, x + dx, y + dy)
        })
        .collect();
    fix_at(&mut s, ids[0], 0.0, 0.0);
    for k in 0..50 {
        let line = s.add_line(ids[k], ids[k + 1]).unwrap();
        if k % 2 == 0 {
            add(&mut s, Constraint::Horizontal { line });
        } else {
            add(&mut s, Constraint::Vertical { line });
        }
        add(&mut s, Constraint::Length { line, value: 10.0 });
    }
    let start = Instant::now();
    let report = s.solve().unwrap();
    let elapsed = start.elapsed();
    println!(
        "100-entity staircase: solved in {elapsed:?}, {} iterations",
        report.iterations
    );
    assert_eq!(report.dof.dof, 0);
    for (k, id) in ids.iter().enumerate() {
        let (x, y) = expected(k);
        assert_at(&s, *id, x, y);
    }
    assert!(elapsed.as_secs_f64() < 10.0, "took {elapsed:?}");
}

#[test]
fn random_hostile_sketches_never_panic() {
    let mut seed: u64 = 12345;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };
    let specials = [
        0.0,
        -0.0,
        1e-12,
        1.0,
        -1.0,
        1e6,
        -1e6,
        1e7,
        f64::NAN,
        f64::INFINITY,
    ];
    for _round in 0..60 {
        let mut s = Sketch::new();
        let mut points = Vec::new();
        let mut curves = Vec::new();
        for _ in 0..(3 + next() % 6) {
            let x = (next() % 200) as f64 - 100.0;
            let y = (next() % 200) as f64 - 100.0;
            points.push(pt(&mut s, x, y));
        }
        let pick = |n: u64, list: &[PointId]| list[(n as usize) % list.len()];
        for _ in 0..(2 + next() % 4) {
            let (a, b, c) = (
                pick(next(), &points),
                pick(next(), &points),
                pick(next(), &points),
            );
            let made = match next() % 3 {
                0 => s.add_line(a, b),
                1 => s.add_circle(a, 1.0 + (next() % 20) as f64),
                _ => s.add_arc(a, b, c),
            };
            if let Ok(id) = made {
                curves.push(id);
            }
        }
        if curves.is_empty() {
            continue;
        }
        for _ in 0..(next() % 10) {
            let p1 = pick(next(), &points);
            let p2 = pick(next(), &points);
            let c1 = curves[(next() as usize) % curves.len()];
            let c2 = curves[(next() as usize) % curves.len()];
            let v = if next() % 4 == 0 {
                specials[(next() as usize) % specials.len()]
            } else {
                (next() % 50) as f64
            };
            let c = match next() % 18 {
                0 => Constraint::Coincident { a: p1, b: p2 },
                1 => Constraint::PointOnLine {
                    point: p1,
                    line: c1,
                },
                2 => Constraint::PointOnCircle {
                    point: p1,
                    curve: c1,
                },
                3 => Constraint::Horizontal { line: c1 },
                4 => Constraint::Vertical { line: c1 },
                5 => Constraint::Parallel { a: c1, b: c2 },
                6 => Constraint::Perpendicular { a: c1, b: c2 },
                7 => Constraint::Tangent { a: c1, b: c2 },
                8 => Constraint::Equal { a: c1, b: c2 },
                9 => Constraint::Fix {
                    point: p1,
                    at: Point2::new(v, -v),
                },
                10 => Constraint::Midpoint {
                    point: p1,
                    line: c1,
                },
                11 => Constraint::Symmetric {
                    a: p1,
                    b: p2,
                    line: c1,
                },
                12 => Constraint::Distance {
                    a: p1,
                    b: p2,
                    value: v,
                },
                13 => Constraint::PointLineDistance {
                    point: p1,
                    line: c1,
                    value: v,
                },
                14 => Constraint::Length { line: c1, value: v },
                15 => Constraint::Radius {
                    curve: c1,
                    value: v,
                },
                16 => Constraint::Diameter {
                    curve: c1,
                    value: v,
                },
                _ => Constraint::Angle {
                    a: c1,
                    b: c2,
                    value: v,
                },
            };
            let _ = s.add_constraint(c);
        }
        s.validate().unwrap();
        let before = s.clone();
        if s.solve().is_err() {
            assert_eq!(s, before);
        }
        let _ = s.analyze();
        let _ = s.profiles();
        let target = Point2::new((next() % 300) as f64 - 150.0, (next() % 300) as f64 - 150.0);
        let before = s.clone();
        if s.drag(pick(next(), &points), target).is_err() {
            assert_eq!(s, before);
        }
        s.validate().unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: Sketch = serde_json::from_str(&json).unwrap();
        assert_same_sketch(&back, &s);
    }
}

#[test]
fn a_full_circle_area_is_exact() {
    let mut s = Sketch::new();
    let c = pt(&mut s, 3.0, -2.0);
    s.add_circle(c, 2.0).unwrap();
    let regions = s.profiles();
    assert_eq!(regions.len(), 1);
    assert!((regions[0].area - 4.0 * PI).abs() < 1e-12);
}

/// Two circles about (0, 0) and (7, 3) made concentric: with the first centre fixed, the
/// second moves onto it; each keeps its radius.
#[test]
fn concentric_circles_share_a_centre() {
    let mut s = Sketch::new();
    let c1 = pt(&mut s, 0.0, 0.0);
    let c2 = pt(&mut s, 7.0, 3.0);
    let a = s.add_circle(c1, 10.0).unwrap();
    let b = s.add_circle(c2, 4.0).unwrap();
    fix_at(&mut s, c1, 0.0, 0.0);
    let before = s.analyze().unwrap().dof;
    add(&mut s, Constraint::Concentric { a, b });
    let report = s.solve().unwrap();
    assert_at(&s, c2, 0.0, 0.0);
    assert!((s.radius(b).unwrap() - 4.0).abs() < TOL);
    // Two equations: the second centre's x and y.
    assert_eq!(before - report.dof.dof, 2);
    // A line is not circular, and a circle is not concentric with itself.
    let p = pt(&mut s, 1.0, 1.0);
    let q = pt(&mut s, 2.0, 1.0);
    let l = s.add_line(p, q).unwrap();
    assert!(
        s.add_constraint(Constraint::Concentric { a, b: l })
            .is_err()
    );
    assert!(
        s.add_constraint(Constraint::Concentric { a, b: a })
            .is_err()
    );
}

/// A line from (0, 0) to (10, 0), fixed, and one from (12, 3) to (20, 4): collinear puts
/// both ends of the second on y = 0, two degrees of freedom fewer. Lines that share a point
/// need one equation only.
#[test]
fn collinear_lines_lie_on_one_line() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 10.0, 0.0);
    let c = pt(&mut s, 12.0, 3.0);
    let d = pt(&mut s, 20.0, 4.0);
    let l1 = s.add_line(a, b).unwrap();
    let l2 = s.add_line(c, d).unwrap();
    fix_at(&mut s, a, 0.0, 0.0);
    fix_at(&mut s, b, 10.0, 0.0);
    let before = s.analyze().unwrap().dof;
    add(&mut s, Constraint::Collinear { a: l2, b: l1 });
    let report = s.solve().unwrap();
    assert!(at(&s, c).y.abs() < TOL && at(&s, d).y.abs() < TOL);
    assert_eq!(before - report.dof.dof, 2);
    assert!(report.dof.redundant.is_empty());
    // A third line from b, sharing it: one equation, and no redundancy.
    let e = pt(&mut s, 15.0, -2.0);
    let l3 = s.add_line(b, e).unwrap();
    let before = s.analyze().unwrap().dof;
    add(&mut s, Constraint::Collinear { a: l3, b: l1 });
    let report = s.solve().unwrap();
    assert!(at(&s, e).y.abs() < TOL);
    assert_eq!(before - report.dof.dof, 1);
    assert!(report.dof.redundant.is_empty());
}

/// Points (0, 0) fixed and (8, 5): horizontal between them puts the second at y = 0,
/// vertical at x = 0. Together they would join the points.
#[test]
fn horizontal_and_vertical_between_two_points() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 8.0, 5.0);
    fix_at(&mut s, a, 0.0, 0.0);
    add(&mut s, Constraint::HorizontalPoints { a, b });
    s.solve().unwrap();
    assert!(at(&s, b).y.abs() < TOL && (at(&s, b).x - 8.0).abs() < 1.0);
    let mut s2 = Sketch::new();
    let a = pt(&mut s2, 0.0, 0.0);
    let b = pt(&mut s2, 8.0, 5.0);
    fix_at(&mut s2, a, 0.0, 0.0);
    add(&mut s2, Constraint::VerticalPoints { a, b });
    s2.solve().unwrap();
    assert!(at(&s2, b).x.abs() < TOL && (at(&s2, b).y - 5.0).abs() < 1.0);
    assert!(
        s2.add_constraint(Constraint::VerticalPoints { a, b: a })
            .is_err()
    );
}

/// From (0, 0) fixed to (30, 40): a horizontal dimension of 12 puts the second point at
/// x = 12, a vertical one of 9 at y = 9; the aligned distance is then √(144 + 81) = 15.
/// The second point stays on the side it started on.
#[test]
fn horizontal_and_vertical_dimensions_measure_along_the_axes() {
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, 30.0, 40.0);
    fix_at(&mut s, a, 0.0, 0.0);
    add(&mut s, Constraint::HorizontalDistance { a, b, value: 12.0 });
    add(&mut s, Constraint::VerticalDistance { a, b, value: 9.0 });
    let report = s.solve().unwrap();
    assert_at(&s, b, 12.0, 9.0);
    assert_eq!(report.dof.dof, 0);
    assert!((at(&s, a).distance(at(&s, b)) - 15.0).abs() < TOL);
    // From the left: a point at (-30, 40) goes to x = -12.
    let mut s = Sketch::new();
    let a = pt(&mut s, 0.0, 0.0);
    let b = pt(&mut s, -30.0, 40.0);
    fix_at(&mut s, a, 0.0, 0.0);
    add(&mut s, Constraint::HorizontalDistance { a, b, value: 12.0 });
    s.solve().unwrap();
    assert!((at(&s, b).x + 12.0).abs() < TOL);
    assert!(
        s.add_constraint(Constraint::HorizontalDistance { a, b, value: -1.0 })
            .is_err()
    );
}

/// A quarter arc about the fixed origin, start fixed at (10, 0): an arc length of 5π
/// (a quarter turn of r = 10 is 5π; asking 10π/3 gives a sweep of π/3) puts the end at
/// angle π/3: (5, 5√3).
#[test]
fn an_arc_length_dimension_sets_the_sweep() {
    let mut s = Sketch::new();
    let c = pt(&mut s, 0.0, 0.0);
    let st = pt(&mut s, 10.0, 0.0);
    let en = pt(&mut s, 0.0, 10.0);
    let arc = s.add_arc(c, st, en).unwrap();
    fix_at(&mut s, c, 0.0, 0.0);
    fix_at(&mut s, st, 10.0, 0.0);
    let len = add(
        &mut s,
        Constraint::ArcLength {
            curve: arc,
            value: 5.0 * PI,
        },
    );
    s.solve().unwrap();
    assert_at(&s, en, 0.0, 10.0);
    s.try_set_dimension(len, 10.0 * PI / 3.0).unwrap();
    assert_at(&s, en, 5.0, 5.0 * 3f64.sqrt());
    assert_eq!(s.analyze().unwrap().dof, 0);
    // Only arcs have an arc length.
    let o = pt(&mut s, 50.0, 50.0);
    let circle = s.add_circle(o, 3.0).unwrap();
    assert!(
        s.add_constraint(Constraint::ArcLength {
            curve: circle,
            value: 1.0
        })
        .is_err()
    );
}

/// A 40 x 20 rectangle fully dimensioned, with a driven distance across the diagonal: it
/// reads √(1600 + 400) = 20√5 without over-constraining, follows a width change to 30
/// (√(900 + 400) = 10√13), refuses a new value, survives a save and becomes driving again
/// on request.
#[test]
fn a_driven_dimension_shows_a_size_without_constraining() {
    let mut s = Sketch::new();
    let p0 = pt(&mut s, 0.0, 0.0);
    let p1 = pt(&mut s, 40.0, 0.0);
    let p2 = pt(&mut s, 40.0, 20.0);
    let p3 = pt(&mut s, 0.0, 20.0);
    let l: Vec<CurveId> = [(p0, p1), (p1, p2), (p2, p3), (p3, p0)]
        .iter()
        .map(|(a, b)| s.add_line(*a, *b).unwrap())
        .collect();
    fix_at(&mut s, p0, 0.0, 0.0);
    add(&mut s, Constraint::Horizontal { line: l[0] });
    add(&mut s, Constraint::Horizontal { line: l[2] });
    add(&mut s, Constraint::Vertical { line: l[1] });
    add(&mut s, Constraint::Vertical { line: l[3] });
    let width = add(
        &mut s,
        Constraint::Length {
            line: l[0],
            value: 40.0,
        },
    );
    add(
        &mut s,
        Constraint::Length {
            line: l[1],
            value: 20.0,
        },
    );
    assert_eq!(s.solve().unwrap().dof.dof, 0);
    let diagonal = add(
        &mut s,
        Constraint::Distance {
            a: p0,
            b: p2,
            value: 1.0,
        },
    );
    s.set_driven(diagonal, true).unwrap();
    assert!(s.is_driven(diagonal));
    let value = |s: &Sketch| s.constraint(diagonal).and_then(Constraint::value).unwrap();
    assert!((value(&s) - 20.0 * 5f64.sqrt()).abs() < TOL);
    let report = s.solve().unwrap();
    assert!(report.dof.redundant.is_empty());
    assert_eq!(report.dof.status, ConstraintStatus::FullyConstrained);
    s.try_set_dimension(width, 30.0).unwrap();
    assert!((value(&s) - 10.0 * 13f64.sqrt()).abs() < TOL);
    assert!(matches!(
        s.set_dimension(diagonal, 5.0),
        Err(SketchError::Driven(_))
    ));
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"driven\":true"));
    let back: Sketch = serde_json::from_str(&json).unwrap();
    assert!(back.is_driven(diagonal));
    // Driving again it over-constrains the rectangle.
    s.set_driven(diagonal, false).unwrap();
    assert!(!s.analyze().unwrap().redundant.is_empty());
    // Only dimensions can be driven.
    let h = s.constraints().find(|(_, c)| !c.is_dimension()).unwrap().0;
    assert!(s.set_driven(h, true).is_err());
    // Removing it forgets it.
    s.set_driven(diagonal, true).unwrap();
    s.remove_constraint(diagonal).unwrap();
    assert!(!s.is_driven(diagonal));
    s.validate().unwrap();
}

//! The sketch tools against hand-derived geometry.

use crate::*;
use chamfera_geom::Point2;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const TOL: f64 = 1e-7;

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn line(s: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> CurveId {
    let a = s.add_point(p(a.0, a.1)).unwrap();
    let b = s.add_point(p(b.0, b.1)).unwrap();
    s.add_line(a, b).unwrap()
}

fn circle(s: &mut Sketch, c: (f64, f64), r: f64) -> CurveId {
    let c = s.add_point(p(c.0, c.1)).unwrap();
    s.add_circle(c, r).unwrap()
}

/// A 50 x 30 rectangle with shared corners, horizontal and vertical sides: 4 DOF.
/// Returns the corner points (from (0, 0) counter-clockwise) and the sides.
fn rectangle(s: &mut Sketch, w: f64, h: f64) -> (Vec<PointId>, Vec<CurveId>) {
    let pts: Vec<PointId> = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
        .iter()
        .map(|(x, y)| s.add_point(p(*x, *y)).unwrap())
        .collect();
    let lines: Vec<CurveId> = (0..4)
        .map(|k| s.add_line(pts[k], pts[(k + 1) % 4]).unwrap())
        .collect();
    for (k, l) in lines.iter().enumerate() {
        s.add_constraint(if k % 2 == 0 {
            Constraint::Horizontal { line: *l }
        } else {
            Constraint::Vertical { line: *l }
        })
        .unwrap();
    }
    (pts, lines)
}

/// The ends of a line, in order.
fn ends(s: &Sketch, id: CurveId) -> (Point2, Point2) {
    match s.curve(id).unwrap() {
        Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => {
            (s.point(*start).unwrap(), s.point(*end).unwrap())
        }
        Curve::Circle { .. } => panic!("a circle has no ends"),
    }
}

fn near(a: Point2, x: f64, y: f64) -> bool {
    (a.x - x).abs() < TOL && (a.y - y).abs() < TOL
}

/// The segments of the sketch's non-construction lines, as unordered end pairs.
fn has_segment(s: &Sketch, a: (f64, f64), b: (f64, f64)) -> bool {
    s.curves().any(|(id, c)| {
        c.is_line() && !c.is_construction() && {
            let (u, v) = ends(s, id);
            (near(u, a.0, a.1) && near(v, b.0, b.1)) || (near(u, b.0, b.1) && near(v, a.0, a.1))
        }
    })
}

fn line_count(s: &Sketch) -> usize {
    s.curves()
        .filter(|(_, c)| c.is_line() && !c.is_construction())
        .count()
}

/// Degrees of freedom, checking that no constraint a tool added is redundant.
fn dof(s: &Sketch) -> usize {
    let r = s.analyze().unwrap();
    assert!(r.redundant.is_empty(), "redundant: {:?}", r.redundant);
    r.dof
}

fn areas(s: &Sketch) -> Vec<f64> {
    let mut a: Vec<f64> = s.profiles().iter().map(|r| r.area).collect();
    a.sort_by(f64::total_cmp);
    a
}

fn cross(s: &mut Sketch) -> (CurveId, CurveId) {
    let a = line(s, (-10.0, 0.0), (10.0, 0.0));
    let b = line(s, (0.0, -10.0), (0.0, 10.0));
    (a, b)
}

#[test]
fn splitting_a_cross_gives_four_pieces_meeting_at_the_crossing() {
    let mut s = Sketch::new();
    cross(&mut s);
    let before = dof(&s);
    let r = s.apply_tool(&ToolOp::Split { curve: None }).unwrap();
    assert_eq!(line_count(&s), 4);
    for end in [(-10.0, 0.0), (10.0, 0.0), (0.0, -10.0), (0.0, 10.0)] {
        assert!(has_segment(&s, end, (0.0, 0.0)), "{end:?}");
    }
    // One shared point at the crossing, and the pieces stay in line: same freedom as
    // two free lines (8).
    assert_eq!(r.created_points.len(), 1);
    assert_eq!(dof(&s), before);
    assert_eq!(before, 8);
    // Trimming one arm of the split cross leaves three.
    let arm = s
        .curves()
        .find(|(id, _)| {
            let (a, b) = ends(&s, *id);
            near(a, 10.0, 0.0) || near(b, 10.0, 0.0)
        })
        .unwrap()
        .0;
    s.apply_tool(&ToolOp::Trim {
        curve: arm,
        at: p(6.0, 0.0),
    })
    .unwrap();
    assert_eq!(line_count(&s), 3);
    assert!(!has_segment(&s, (10.0, 0.0), (0.0, 0.0)));
    assert!(has_segment(&s, (-10.0, 0.0), (0.0, 0.0)));
}

#[test]
fn trimming_an_unsplit_cross_cuts_back_to_the_crossing() {
    let mut s = Sketch::new();
    let (h, v) = cross(&mut s);
    s.apply_tool(&ToolOp::Trim {
        curve: h,
        at: p(5.0, 0.1),
    })
    .unwrap();
    assert_eq!(line_count(&s), 2);
    let (a, b) = ends(&s, h);
    assert!(near(a, -10.0, 0.0) && near(b, 0.0, 0.0));
    assert!(has_segment(&s, (0.0, -10.0), (0.0, 10.0)));
    // The cut end stays on the vertical line: dragging that line takes it along.
    let (_, vb) = ends(&s, v);
    let top = s
        .points()
        .find(|(_, q)| q.coincident(vb))
        .map(|(id, _)| id)
        .unwrap();
    s.drag(top, p(4.0, 10.0)).unwrap();
    let (_, cut) = ends(&s, h);
    let (v0, v1) = ends(&s, v);
    assert!(((v1 - v0).cross(cut - v0)).abs() < 1e-6);
    // The far end of the trimmed half is gone.
    assert!(s.points().all(|(_, q)| !q.coincident(p(10.0, 0.0))));
}

#[test]
fn trimming_a_side_between_two_circles_leaves_the_outer_parts() {
    let mut s = Sketch::new();
    let (_, sides) = rectangle(&mut s, 50.0, 30.0);
    // Circles on the bottom side cut it at x = 10, 20 and 30, 40.
    circle(&mut s, (15.0, 0.0), 5.0);
    circle(&mut s, (35.0, 0.0), 5.0);
    let before = dof(&s);
    s.apply_tool(&ToolOp::Trim {
        curve: sides[0],
        at: p(25.0, 0.0),
    })
    .unwrap();
    assert_eq!(line_count(&s), 5);
    assert!(has_segment(&s, (0.0, 0.0), (20.0, 0.0)));
    assert!(has_segment(&s, (30.0, 0.0), (50.0, 0.0)));
    assert!(!has_segment(&s, (0.0, 0.0), (50.0, 0.0)));
    // The two new ends ride on their circles and the kept pieces stay level.
    assert_eq!(dof(&s), before);
    assert_eq!(s.curves().filter(|(_, c)| c.is_circular()).count(), 2);
}

#[test]
fn trimming_a_circle_between_two_lines_leaves_an_arc() {
    let mut s = Sketch::new();
    let c = circle(&mut s, (0.0, 0.0), 5.0);
    // x = 3 cuts the circle at (3, ±4).
    line(&mut s, (3.0, -10.0), (3.0, 10.0));
    s.apply_tool(&ToolOp::Trim {
        curve: c,
        at: p(5.0, 0.0),
    })
    .unwrap();
    let (a, b) = ends(&s, c);
    // The left part remains: counter-clockwise from (3, 4) to (3, -4).
    assert!(near(a, 3.0, 4.0) && near(b, 3.0, -4.0), "{a:?} {b:?}");
    // A curve nothing cuts is removed whole.
    let lone = circle(&mut s, (100.0, 0.0), 2.0);
    s.apply_tool(&ToolOp::Trim {
        curve: lone,
        at: p(102.0, 0.0),
    })
    .unwrap();
    assert!(s.curve(lone).is_none());
}

#[test]
fn extending_to_a_circle_hits_the_exact_point() {
    let mut s = Sketch::new();
    let l = line(&mut s, (0.0, 0.0), (5.0, 0.0));
    // The circle about (20, 3) with r = 5 meets y = 0 at x = 20 ± 4: the near one is 16.
    circle(&mut s, (20.0, 3.0), 5.0);
    s.apply_tool(&ToolOp::Extend {
        curve: l,
        at: p(4.0, 0.0),
    })
    .unwrap();
    let (a, b) = ends(&s, l);
    assert!(near(a, 0.0, 0.0) && near(b, 16.0, 0.0), "{b:?}");
    // Nothing behind the start: refused, unchanged.
    let before = s.clone();
    assert!(
        s.apply_tool(&ToolOp::Extend {
            curve: l,
            at: p(0.5, 0.0),
        })
        .is_err()
    );
    assert_eq!(s, before);
}

#[test]
fn extending_an_arc_follows_its_circle_to_a_line() {
    let mut s = Sketch::new();
    let c = s.add_point(p(0.0, 0.0)).unwrap();
    let a = s.add_point(p(5.0, 0.0)).unwrap();
    let b = s.add_point(p(0.0, 5.0)).unwrap();
    let arc = s.add_arc(c, a, b).unwrap();
    // x = -3 meets the circle at (-3, ±4); going on counter-clockwise from (0, 5) the
    // first is (-3, 4).
    line(&mut s, (-3.0, -10.0), (-3.0, 10.0));
    s.apply_tool(&ToolOp::Extend {
        curve: arc,
        at: p(0.0, 5.0),
    })
    .unwrap();
    let (_, e) = ends(&s, arc);
    assert!(near(e, -3.0, 4.0), "{e:?}");
}

#[test]
fn a_fillet_of_a_right_angle_is_tangent_with_its_centre_r_from_both_lines() {
    let mut s = Sketch::new();
    let v = s.add_point(p(0.0, 0.0)).unwrap();
    let x = s.add_point(p(20.0, 0.0)).unwrap();
    let y = s.add_point(p(0.0, 20.0)).unwrap();
    let lx = s.add_line(v, x).unwrap();
    let ly = s.add_line(v, y).unwrap();
    s.fix_point(v).unwrap();
    s.add_constraint(Constraint::Horizontal { line: lx })
        .unwrap();
    s.add_constraint(Constraint::Vertical { line: ly }).unwrap();
    let before = dof(&s);
    let r = s
        .apply_tool(&ToolOp::Fillet {
            corner: v,
            radius: 5.0,
        })
        .unwrap();
    let arc = *r.created_curves.first().unwrap();
    let Curve::Arc { centre, .. } = s.curve(arc).unwrap() else {
        panic!("a fillet is an arc")
    };
    assert!(near(s.point(*centre).unwrap(), 5.0, 5.0));
    assert!((s.radius(arc).unwrap() - 5.0).abs() < TOL);
    // Tangent points 5 along each line; the lines now start there.
    assert!(near(ends(&s, lx).0, 5.0, 0.0));
    assert!(near(ends(&s, ly).0, 0.0, 5.0));
    // Tangent, radius and the corner kept on both lines: no freedom gained or lost.
    assert_eq!(dof(&s), before);
    let kinds: Vec<&str> = s.constraints().map(|(_, c)| c.kind_name()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "tangent").count(), 2);
    assert!(kinds.contains(&"radius"));
    // Changing the radius moves the tangent points along the lines.
    let dim = r
        .created_constraints
        .iter()
        .find(|c| s.constraint(**c).unwrap().kind_name() == "radius")
        .unwrap();
    s.try_set_dimension(*dim, 8.0).unwrap();
    assert!(near(ends(&s, lx).0, 8.0, 0.0));
}

#[test]
fn a_rectangle_filleted_at_every_corner_has_the_exact_area() {
    let mut s = Sketch::new();
    let (pts, _) = rectangle(&mut s, 50.0, 30.0);
    let before = dof(&s);
    for c in pts {
        s.apply_tool(&ToolOp::Fillet {
            corner: c,
            radius: 5.0,
        })
        .unwrap();
    }
    // Each corner loses the square r² minus the quarter disc π r² / 4.
    let expected = 1500.0 - (4.0 - PI) * 25.0;
    let a = areas(&s);
    assert_eq!(a.len(), 1);
    assert!((a[0] - expected).abs() < 1e-9, "{}", a[0]);
    assert_eq!(dof(&s), before);
}

#[test]
fn a_chamfer_cuts_a_right_triangle_off_the_corner() {
    let mut s = Sketch::new();
    let (pts, _) = rectangle(&mut s, 50.0, 30.0);
    let before = dof(&s);
    s.apply_tool(&ToolOp::Chamfer {
        corner: pts[2],
        distance: 5.0,
        distance2: Some(3.0),
    })
    .unwrap();
    // The top-right corner loses 5 · 3 / 2.
    let a = areas(&s);
    assert!((a[0] - (1500.0 - 7.5)).abs() < 1e-9, "{a:?}");
    assert_eq!(dof(&s), before);
    // 5 down the right side (the first line at the corner) and 3 along the top.
    assert!(has_segment(&s, (50.0, 25.0), (47.0, 30.0)));
}

#[test]
fn fillets_are_refused_where_they_do_not_fit() {
    let mut s = Sketch::new();
    let (pts, _) = rectangle(&mut s, 50.0, 30.0);
    let before = s.clone();
    for (corner, radius) in [
        (pts[0], 40.0),
        (pts[0], -1.0),
        (pts[0], f64::NAN),
        (pts[0], f64::INFINITY),
        (PointId(999), 5.0),
    ] {
        assert!(s.apply_tool(&ToolOp::Fillet { corner, radius }).is_err());
        assert_eq!(s, before);
    }
    // A free end is not a corner.
    let l = line(&mut s, (100.0, 0.0), (110.0, 0.0));
    let Curve::Line { start, .. } = s.curve(l).unwrap().clone() else {
        unreachable!()
    };
    assert!(
        s.apply_tool(&ToolOp::Fillet {
            corner: start,
            radius: 1.0
        })
        .is_err()
    );
}

fn offset_rectangle(corners: OffsetCorners, distance: f64) -> (Sketch, ToolReport, usize) {
    let mut s = Sketch::new();
    let (_, sides) = rectangle(&mut s, 50.0, 30.0);
    let before = dof(&s);
    let r = s
        .apply_tool(&ToolOp::Offset {
            curves: vec![sides[0]],
            distance,
            side: None,
            corners,
            dimension: true,
        })
        .unwrap();
    (s, r, before)
}

#[test]
fn a_sharp_outward_offset_of_a_rectangle_is_54_by_34() {
    let (s, r, before) = offset_rectangle(OffsetCorners::Sharp, 2.0);
    assert_eq!(r.created_curves.len(), 4);
    for (a, b) in [
        ((-2.0, -2.0), (52.0, -2.0)),
        ((52.0, -2.0), (52.0, 32.0)),
        ((52.0, 32.0), (-2.0, 32.0)),
        ((-2.0, 32.0), (-2.0, -2.0)),
    ] {
        assert!(has_segment(&s, a, b), "{a:?} {b:?}");
    }
    // The ring between them is 54 · 34 - 50 · 30; the copy is fully tied to the original.
    let a = areas(&s);
    assert!((a[0] - (1836.0 - 1500.0)).abs() < 1e-9, "{a:?}");
    assert!((a[1] - 1500.0).abs() < 1e-9);
    assert_eq!(dof(&s), before);
}

#[test]
fn a_round_outward_offset_has_quarter_circles_at_the_corners() {
    let (s, r, before) = offset_rectangle(OffsetCorners::Round, 2.0);
    // Four sides and four r = 2 arcs around the original corners.
    assert_eq!(r.created_curves.len(), 8);
    let arcs: Vec<CurveId> = r
        .created_curves
        .iter()
        .copied()
        .filter(|c| s.curve(*c).unwrap().is_circular())
        .collect();
    assert_eq!(
        arcs.len(),
        4,
        "{:?} {:?}",
        r.created_curves,
        s.curves().collect::<Vec<_>>()
    );
    for a in arcs {
        assert!((s.radius(a).unwrap() - 2.0).abs() < TOL);
    }
    // 50 · 30 plus the four side strips 2 · (50 + 30) · 2 plus a full circle π 2².
    let outer = 1500.0 + 320.0 + 4.0 * PI;
    let a = areas(&s);
    assert!((a[0] - (outer - 1500.0)).abs() < 1e-9, "{a:?}");
    assert_eq!(dof(&s), before);
}

#[test]
fn an_inward_offset_toward_a_point_is_46_by_26_and_too_far_is_refused() {
    let mut s = Sketch::new();
    let (_, sides) = rectangle(&mut s, 50.0, 30.0);
    s.apply_tool(&ToolOp::Offset {
        curves: sides.clone(),
        distance: 2.0,
        side: Some(p(25.0, 15.0)),
        corners: OffsetCorners::Round,
        dimension: false,
    })
    .unwrap();
    // Inside corners are cut back to meet, whatever the corner style.
    assert!(has_segment(&s, (2.0, 2.0), (48.0, 2.0)));
    // The inner 46 x 26 and the ring between it and the original.
    let a = areas(&s);
    assert!((a[1] - 46.0 * 26.0).abs() < 1e-9, "{a:?}");
    assert!((a[0] - (1500.0 - 46.0 * 26.0)).abs() < 1e-9);
    let before = s.clone();
    assert!(
        s.apply_tool(&ToolOp::Offset {
            curves: vec![sides[0]],
            distance: -20.0,
            side: None,
            corners: OffsetCorners::Sharp,
            dimension: true,
        })
        .is_err()
    );
    assert_eq!(s, before);
}

#[test]
fn offsetting_a_slot_keeps_its_arcs_concentric() {
    // A slot: lines y = ±5 from x = 0 to 20 and half circles r = 5 about (0, 0), (20, 0).
    let mut s = Sketch::new();
    let c0 = s.add_point(p(0.0, 0.0)).unwrap();
    let c1 = s.add_point(p(20.0, 0.0)).unwrap();
    let q: Vec<PointId> = [(0.0, -5.0), (20.0, -5.0), (20.0, 5.0), (0.0, 5.0)]
        .iter()
        .map(|(x, y)| s.add_point(p(*x, *y)).unwrap())
        .collect();
    let bottom = s.add_line(q[0], q[1]).unwrap();
    s.add_arc(c1, q[1], q[2]).unwrap();
    s.add_line(q[2], q[3]).unwrap();
    s.add_arc(c0, q[3], q[0]).unwrap();
    s.apply_tool(&ToolOp::Offset {
        curves: vec![bottom],
        distance: 1.0,
        side: None,
        corners: OffsetCorners::Sharp,
        dimension: true,
    })
    .unwrap();
    // Outer slot: 20 · 12 + π 6²; inner 20 · 10 + π 5².
    let a = areas(&s);
    assert!((a[1] - (200.0 + 25.0 * PI)).abs() < 1e-9, "{a:?}");
    assert!(
        (a[0] - (240.0 + 36.0 * PI - 200.0 - 25.0 * PI)).abs() < 1e-9,
        "{a:?}"
    );
    let s0 = s.analyze().unwrap();
    assert!(s0.redundant.is_empty(), "{:?}", s0.redundant);
}

#[test]
fn mirroring_a_line_an_arc_and_a_circle_in_the_y_axis() {
    let mut s = Sketch::new();
    let axis = line(&mut s, (0.0, -10.0), (0.0, 10.0));
    let l = line(&mut s, (1.0, 1.0), (4.0, 2.0));
    let c = circle(&mut s, (3.0, -3.0), 1.5);
    let ac = s.add_point(p(5.0, 5.0)).unwrap();
    let a0 = s.add_point(p(7.0, 5.0)).unwrap();
    let a1 = s.add_point(p(5.0, 7.0)).unwrap();
    let arc = s.add_arc(ac, a0, a1).unwrap();
    let r = s
        .apply_tool(&ToolOp::Mirror {
            curves: vec![l, c, arc],
            points: vec![],
            axis,
        })
        .unwrap();
    assert_eq!(r.created_curves.len(), 3);
    assert!(has_segment(&s, (-1.0, 1.0), (-4.0, 2.0)));
    let copies: Vec<&Curve> = r
        .created_curves
        .iter()
        .map(|k| s.curve(*k).unwrap())
        .collect();
    let circle_copy = copies
        .iter()
        .find_map(|c| match c {
            Curve::Circle { centre, radius, .. } => Some((s.point(*centre).unwrap(), *radius)),
            _ => None,
        })
        .unwrap();
    assert!(near(circle_copy.0, -3.0, -3.0) && (circle_copy.1 - 1.5).abs() < TOL);
    // The arc's image runs counter-clockwise from (-5, 7) to (-7, 5) about (-5, 5).
    let (st, en) = copies
        .iter()
        .find_map(|c| match c {
            Curve::Arc { start, end, .. } => {
                Some((s.point(*start).unwrap(), s.point(*end).unwrap()))
            }
            _ => None,
        })
        .unwrap();
    assert!(near(st, -5.0, 7.0) && near(en, -7.0, 5.0));
    // The copy follows: drag the original line's end and the image mirrors it.
    let Curve::Line { end, .. } = s.curve(l).unwrap().clone() else {
        unreachable!()
    };
    // With the mirror line held still.
    let Curve::Line {
        start: m0, end: m1, ..
    } = s.curve(axis).unwrap().clone()
    else {
        unreachable!()
    };
    s.fix_point(m0).unwrap();
    s.fix_point(m1).unwrap();
    s.drag(end, p(6.0, 3.0)).unwrap();
    assert!(has_segment(&s, (-1.0, 1.0), (-6.0, 3.0)));
}

#[test]
fn patterns_copy_to_the_right_places_and_keep_the_shape() {
    let mut s = Sketch::new();
    let c = circle(&mut s, (0.0, 0.0), 1.0);
    s.apply_tool(&ToolOp::RectangularPattern {
        curves: vec![c],
        points: vec![],
        columns: 3,
        rows: 2,
        dx: 10.0,
        dy: 5.0,
    })
    .unwrap();
    let mut centres: Vec<(i64, i64)> = s
        .curves()
        .filter_map(|(_, c)| match c {
            Curve::Circle { centre, .. } => {
                let q = s.point(*centre).unwrap();
                Some((q.x.round() as i64, q.y.round() as i64))
            }
            _ => None,
        })
        .collect();
    centres.sort();
    assert_eq!(
        centres,
        vec![(0, 0), (0, 5), (10, 0), (10, 5), (20, 0), (20, 5)]
    );

    // Around the origin, four instances: a horizontal line turns vertical at 90°.
    let mut s = Sketch::new();
    let l = line(&mut s, (10.0, 0.0), (12.0, 0.0));
    s.add_constraint(Constraint::Horizontal { line: l })
        .unwrap();
    s.apply_tool(&ToolOp::CircularPattern {
        curves: vec![l],
        points: vec![],
        centre: p(0.0, 0.0),
        count: 4,
        angle: TAU,
    })
    .unwrap();
    assert!(has_segment(&s, (0.0, 10.0), (0.0, 12.0)));
    assert!(has_segment(&s, (-10.0, 0.0), (-12.0, 0.0)));
    assert!(has_segment(&s, (0.0, -10.0), (0.0, -12.0)));
    let kinds: Vec<&str> = s.constraints().map(|(_, c)| c.kind_name()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "horizontal").count(), 2);
    assert_eq!(kinds.iter().filter(|k| **k == "vertical").count(), 2);
    // Three over a quarter turn: 0°, 45°, 90°.
    let mut s = Sketch::new();
    let l = line(&mut s, (10.0, 0.0), (12.0, 0.0));
    s.apply_tool(&ToolOp::CircularPattern {
        curves: vec![l],
        points: vec![],
        centre: p(0.0, 0.0),
        count: 3,
        angle: FRAC_PI_2,
    })
    .unwrap();
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!(has_segment(&s, (10.0 * h, 10.0 * h), (12.0 * h, 12.0 * h)));
    assert!(has_segment(&s, (0.0, 10.0), (0.0, 12.0)));
}

#[test]
fn splitting_two_overlapping_rectangles_finds_their_three_regions() {
    let mut s = Sketch::new();
    rectangle(&mut s, 20.0, 20.0);
    let pts: Vec<PointId> = [(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)]
        .iter()
        .map(|(x, y)| s.add_point(p(*x, *y)).unwrap())
        .collect();
    for k in 0..4 {
        s.add_line(pts[k], pts[(k + 1) % 4]).unwrap();
    }
    s.apply_tool(&ToolOp::Split { curve: None }).unwrap();
    // 400 - 100 on each side and the 10 x 10 overlap.
    let a = areas(&s);
    assert_eq!(a.len(), 3, "{a:?}");
    assert!((a[0] - 100.0).abs() < 1e-9 && (a[1] - 300.0).abs() < 1e-9);
    assert!((a[2] - 300.0).abs() < 1e-9);
}

#[test]
fn drawing_helpers_make_the_expected_shapes() {
    let mut s = Sketch::new();
    s.apply_tool(&ToolOp::CentreRectangle {
        centre: p(0.0, 0.0),
        corner: p(5.0, 3.0),
    })
    .unwrap();
    let a = areas(&s);
    assert!((a[0] - 60.0).abs() < 1e-9);
    // Four free rectangle freedoms; the centre follows the diagonal.
    assert_eq!(dof(&s), 4);

    let mut s = Sketch::new();
    let r = s
        .apply_tool(&ToolOp::ThreePointArc {
            start: p(5.0, 0.0),
            through: p(0.0, 5.0),
            end: p(-5.0, 0.0),
        })
        .unwrap();
    let arc = r.created_curves[0];
    let Curve::Arc { centre, .. } = s.curve(arc).unwrap() else {
        panic!()
    };
    assert!(near(s.point(*centre).unwrap(), 0.0, 0.0));
    assert!((s.radius(arc).unwrap() - 5.0).abs() < TOL);
    let (st, _) = ends(&s, arc);
    assert!(near(st, 5.0, 0.0));
    // The other way round passes below: the same circle, stored counter-clockwise.
    let r = s
        .apply_tool(&ToolOp::ThreePointArc {
            start: p(5.0, 0.0),
            through: p(0.0, -5.0),
            end: p(-5.0, 0.0),
        })
        .unwrap();
    let (st, en) = ends(&s, r.created_curves[0]);
    assert!(near(st, -5.0, 0.0) && near(en, 5.0, 0.0));
    assert!(
        s.apply_tool(&ToolOp::ThreePointArc {
            start: p(0.0, 0.0),
            through: p(1.0, 1.0),
            end: p(2.0, 2.0),
        })
        .is_err()
    );

    // Leaving (10, 0) along +X and ending at (10, 10): a half circle about (10, 5).
    let mut s = Sketch::new();
    let l = line(&mut s, (0.0, 0.0), (10.0, 0.0));
    let Curve::Line { end, .. } = s.curve(l).unwrap().clone() else {
        unreachable!()
    };
    let r = s
        .apply_tool(&ToolOp::TangentArc {
            from: end,
            to: p(10.0, 10.0),
        })
        .unwrap();
    let arc = *r
        .created_curves
        .iter()
        .find(|c| s.curve(**c).unwrap().is_circular())
        .unwrap();
    let Curve::Arc { centre, .. } = s.curve(arc).unwrap() else {
        panic!()
    };
    assert!(near(s.point(*centre).unwrap(), 10.0, 5.0));
    assert!((s.radius(arc).unwrap() - 5.0).abs() < TOL);
}

#[test]
fn hostile_tool_input_is_refused_without_changing_anything() {
    let mut s = Sketch::new();
    let (pts, sides) = rectangle(&mut s, 50.0, 30.0);
    let c = circle(&mut s, (25.0, 15.0), 5.0);
    let bad = [
        f64::NAN,
        f64::INFINITY,
        -f64::INFINITY,
        1e300,
        -1e300,
        0.0,
        -0.0,
    ];
    let before = s.clone();
    let mut ops = vec![
        ToolOp::Trim {
            curve: CurveId(999),
            at: p(0.0, 0.0),
        },
        ToolOp::Extend {
            curve: c,
            at: p(0.0, 0.0),
        },
        ToolOp::Extend {
            curve: sides[0],
            at: p(1.0, 0.0),
        },
        ToolOp::Split {
            curve: Some(CurveId(12345)),
        },
        ToolOp::Mirror {
            curves: sides.clone(),
            points: vec![],
            axis: c,
        },
        ToolOp::Mirror {
            curves: vec![],
            points: vec![],
            axis: sides[0],
        },
        ToolOp::Offset {
            curves: vec![],
            distance: 1.0,
            side: None,
            corners: OffsetCorners::Sharp,
            dimension: true,
        },
        ToolOp::RectangularPattern {
            curves: vec![c],
            points: vec![],
            columns: 100_000,
            rows: 100_000,
            dx: 1.0,
            dy: 1.0,
        },
        ToolOp::RectangularPattern {
            curves: vec![c],
            points: vec![],
            columns: 0,
            rows: 1,
            dx: 1.0,
            dy: 1.0,
        },
        ToolOp::CircularPattern {
            curves: vec![c],
            points: vec![],
            centre: p(0.0, 0.0),
            count: 1,
            angle: TAU,
        },
        ToolOp::TangentArc {
            from: pts[0],
            to: p(5.0, 5.0),
        },
        ToolOp::CentreRectangle {
            centre: p(0.0, 0.0),
            corner: p(0.0, 5.0),
        },
    ];
    for v in bad {
        ops.push(ToolOp::Trim {
            curve: sides[0],
            at: p(v, 0.0),
        });
        ops.push(ToolOp::Fillet {
            corner: pts[0],
            radius: v,
        });
        ops.push(ToolOp::Chamfer {
            corner: pts[1],
            distance: 1.0,
            distance2: Some(v),
        });
        ops.push(ToolOp::Offset {
            curves: vec![sides[1]],
            distance: v,
            side: Some(p(v, v)),
            corners: OffsetCorners::Round,
            dimension: true,
        });
        ops.push(ToolOp::RectangularPattern {
            curves: vec![c],
            points: vec![],
            columns: 2,
            rows: 1,
            dx: v,
            dy: 0.0,
        });
        ops.push(ToolOp::CircularPattern {
            curves: vec![c],
            points: vec![],
            centre: p(v, 0.0),
            count: 3,
            angle: v,
        });
        ops.push(ToolOp::ThreePointArc {
            start: p(v, 0.0),
            through: p(0.0, v),
            end: p(1.0, 1.0),
        });
    }
    for op in &ops {
        // Whatever the outcome, the preview never panics either.
        let _ = s.preview_tool(op);
        if s.apply_tool(op).is_ok() {
            s = before.clone();
        }
        assert_eq!(s, before, "{op:?}");
    }
}

#[test]
fn trimming_the_inner_arcs_of_two_circles_leaves_their_union() {
    let mut s = Sketch::new();
    let a = circle(&mut s, (0.0, 0.0), 10.0);
    let b = circle(&mut s, (14.0, 0.0), 10.0);
    s.apply_tool(&ToolOp::Trim {
        curve: a,
        at: p(9.0, 0.0),
    })
    .unwrap();
    s.apply_tool(&ToolOp::Trim {
        curve: b,
        at: p(5.0, 0.0),
    })
    .unwrap();
    // Two discs less the lens they share: the lens of circles r apart d is
    // 2 r² acos(d / 2r) - (d / 2) sqrt(4 r² - d²).
    let lens = 200.0 * (0.7f64).acos() - 7.0 * 204f64.sqrt();
    let a = areas(&s);
    assert_eq!(a.len(), 1, "{a:?}");
    assert!((a[0] - (200.0 * PI - lens)).abs() < 1e-9, "{a:?}");
    // The second trim reused the first one's cut points: nothing is redundant.
    assert_eq!(dof(&s), 6);
}

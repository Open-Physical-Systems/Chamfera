//! Intersections of planar curves (rung K1): segment, line, circle and arc pairs.
//!
//! Tolerance policy: two curves meet where they come within [`LINEAR_TOLERANCE`];
//! directions within [`ANGULAR_TOLERANCE`] are parallel. Results are explicit:
//! - isolated points, each a [`Contact::Crossing`] or a [`Contact::Tangent`] (the curves
//!   touch with the same tangent line: tangent circles, a line touching a circle,
//!   collinear segments meeting end to end);
//! - [`Overlap`]s where the curves coincide over a stretch (collinear segments,
//!   concentric equal circles and arcs).
//!
//! Inputs are valid by construction (the curve constructors check them), so these
//! functions always answer.

use crate::check;
use crate::planar::{Arc2, Circle2, Line2, Segment2};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::TAU;

/// How two curves meet at an isolated point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    /// The curves cross (or one ends on the other) at an angle.
    Crossing,
    /// The curves touch with a common tangent line.
    Tangent,
}

/// An isolated common point, with its parameter on each curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntersectionPoint {
    pub point: Point2,
    /// Parameter on the first curve.
    pub t_a: f64,
    /// Parameter on the second curve.
    pub t_b: f64,
    pub contact: Contact,
}

/// A stretch where the two curves coincide. `start` has parameters `a.0` and `b.0`,
/// `end` has `a.1` and `b.1`; `a.0 < a.1`. For arcs and circles the parameters are
/// angles and may run past 2π when the stretch wraps through angle 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlap {
    pub a: (f64, f64),
    pub b: (f64, f64),
    pub start: Point2,
    pub end: Point2,
    /// True when both curves run the same way along the stretch.
    pub same_direction: bool,
}

/// Everything two curves have in common.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Intersections {
    pub points: Vec<IntersectionPoint>,
    pub overlaps: Vec<Overlap>,
}

impl Intersections {
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.overlaps.is_empty()
    }

    /// The same result with the roles of the two curves swapped.
    pub fn swapped(mut self) -> Intersections {
        for p in &mut self.points {
            std::mem::swap(&mut p.t_a, &mut p.t_b);
        }
        for o in &mut self.overlaps {
            // Keep `a.0 < a.1` for the new first curve.
            let (a, b) = (o.a, o.b);
            if b.0 <= b.1 {
                o.a = b;
                o.b = a;
            } else {
                o.a = (b.1, b.0);
                o.b = (a.1, a.0);
                std::mem::swap(&mut o.start, &mut o.end);
            }
        }
        self
    }
}

/// Where a line `o + s d` (unit `d`) meets a circle.
enum LineCircle {
    None,
    Tangent { s: f64, point: Point2 },
    Two { s: [f64; 2] },
}

fn line_circle_raw(o: Point2, d: Vec2, center: Point2, r: f64) -> LineCircle {
    // Foot of the perpendicular from the centre, and the distance h to the line.
    let s0 = (center - o).dot(d);
    let foot = o + d * s0;
    let h = foot.distance(center);
    if h > r + LINEAR_TOLERANCE {
        LineCircle::None
    } else if (h - r).abs() <= LINEAR_TOLERANCE {
        // Report the touching point on the circle.
        let point = match (foot - center).normalized() {
            Some(u) => center + u * r,
            None => foot,
        };
        LineCircle::Tangent { s: s0, point }
    } else {
        // Half chord, sqrt(r^2 - h^2) written to avoid cancellation.
        let w = ((r - h) * (r + h)).sqrt();
        LineCircle::Two {
            s: [s0 - w, s0 + w],
        }
    }
}

fn angle_on(center: Point2, p: Point2) -> f64 {
    (p - center).angle().rem_euclid(TAU)
}

/// A line-like curve: origin, unit direction, parameter scale and accepted range.
#[derive(Clone, Copy)]
struct LineLike {
    origin: Point2,
    dir: Vec2,
    /// Curve parameter per unit of distance along `dir`.
    per_mm: f64,
    /// Accepted distance range along `dir` (with tolerance applied by `accept`).
    range: (f64, f64),
}

impl LineLike {
    fn of_segment(s: &Segment2) -> LineLike {
        let len = s.length();
        LineLike {
            origin: s.start(),
            dir: s.d1() * (1.0 / len),
            per_mm: 1.0 / len,
            range: (0.0, len),
        }
    }
    fn of_line(l: &Line2) -> LineLike {
        LineLike {
            origin: l.origin(),
            dir: l.direction(),
            per_mm: 1.0,
            range: (f64::NEG_INFINITY, f64::INFINITY),
        }
    }
    /// The curve parameter for distance `s`, or `None` when `s` is more than the linear
    /// tolerance outside the range (clamped onto it otherwise).
    fn accept(&self, s: f64) -> Option<f64> {
        let (lo, hi) = self.range;
        if s < lo - LINEAR_TOLERANCE || s > hi + LINEAR_TOLERANCE {
            return None;
        }
        Some(s.clamp(lo, hi) * self.per_mm)
    }
}

/// A circle-like curve: centre, radius and, for an arc, its accepted angles.
#[derive(Clone, Copy)]
struct CircleLike<'a> {
    center: Point2,
    radius: f64,
    arc: Option<&'a Arc2>,
}

impl CircleLike<'_> {
    fn accept(&self, theta: f64) -> Option<f64> {
        match self.arc {
            None => Some(theta.rem_euclid(TAU)),
            Some(a) => a.parameter_of_angle(theta),
        }
    }
    /// Angular interval `(lo, width)` with `width` in `(0, 2π]`.
    fn interval(&self) -> (f64, f64) {
        match self.arc {
            None => (0.0, TAU),
            Some(a) => {
                let (lo, hi) = a.range();
                (lo, hi - lo)
            }
        }
    }
    fn counter_clockwise(&self) -> bool {
        self.arc.is_none_or(|a| a.sweep() > 0.0)
    }
    fn point_at(&self, theta: f64) -> Point2 {
        let (s, c) = theta.sin_cos();
        self.center + Vec2::new(c, s) * self.radius
    }
}

fn line_vs_circle(l: LineLike, c: CircleLike<'_>) -> Intersections {
    let mut out = Intersections::default();
    let mut push = |s: f64, point: Point2, contact: Contact| {
        if let (Some(t_a), Some(t_b)) = (l.accept(s), c.accept(angle_on(c.center, point))) {
            out.points.push(IntersectionPoint {
                point,
                t_a,
                t_b,
                contact,
            });
        }
    };
    match line_circle_raw(l.origin, l.dir, c.center, c.radius) {
        LineCircle::None => {}
        LineCircle::Tangent { s, point } => push(s, point, Contact::Tangent),
        LineCircle::Two { s } => {
            for si in s {
                push(si, l.origin + l.dir * si, Contact::Crossing);
            }
        }
    }
    out
}

fn circle_vs_circle(a: CircleLike<'_>, b: CircleLike<'_>) -> Intersections {
    let mut out = Intersections::default();
    let delta = b.center - a.center;
    let d = delta.length();
    let (r1, r2) = (a.radius, b.radius);
    if d <= LINEAR_TOLERANCE {
        if (r1 - r2).abs() <= LINEAR_TOLERANCE {
            coincident_circles(a, b, &mut out);
        }
        return out;
    }
    if d > r1 + r2 + LINEAR_TOLERANCE || d < (r1 - r2).abs() - LINEAR_TOLERANCE {
        return out;
    }
    let u = delta * (1.0 / d);
    let mut candidates: Vec<(Point2, Contact)> = Vec::with_capacity(2);
    if (d - (r1 + r2)).abs() <= LINEAR_TOLERANCE {
        // External tangency: on the centre line, r1 from a's centre toward b.
        candidates.push((a.center + u * r1, Contact::Tangent));
    } else if (d - (r1 - r2).abs()).abs() <= LINEAR_TOLERANCE {
        // Internal tangency: on the centre line, on the far side of the smaller circle.
        let p = if r1 >= r2 {
            a.center + u * r1
        } else {
            a.center - u * r1
        };
        candidates.push((p, Contact::Tangent));
    } else {
        // The common chord is at distance x = (d^2 + r1^2 - r2^2) / (2d) from a's centre
        // along u, with half length sqrt(r1^2 - x^2).
        let x = (d * d + (r1 - r2) * (r1 + r2)) / (2.0 * d);
        let h = ((r1 - x) * (r1 + x)).max(0.0).sqrt();
        let m = a.center + u * x;
        candidates.push((m + u.perp() * h, Contact::Crossing));
        candidates.push((m - u.perp() * h, Contact::Crossing));
    }
    for (point, contact) in candidates {
        if let (Some(t_a), Some(t_b)) = (
            a.accept(angle_on(a.center, point)),
            b.accept(angle_on(b.center, point)),
        ) {
            out.points.push(IntersectionPoint {
                point,
                t_a,
                t_b,
                contact,
            });
        }
    }
    out
}

/// Two arcs (or circles) on the same circle: their common angular stretches.
///
/// Each curve covers the angles `[lo, lo + width]` of its own parameter range. A stretch
/// is found in the first curve's angles; the second curve's parameter of the same point
/// differs by a whole number of turns (`offset`).
fn coincident_circles(a: CircleLike<'_>, b: CircleLike<'_>, out: &mut Intersections) {
    let same_direction = a.counter_clockwise() == b.counter_clockwise();
    let (a0, wa) = a.interval();
    let (b0, wb) = b.interval();
    let slack = LINEAR_TOLERANCE / a.radius;
    let mut add = |lo: f64, hi: f64, offset: f64| {
        let width = hi - lo;
        if width < -slack {
            return;
        }
        if width <= slack {
            out.points.push(IntersectionPoint {
                point: a.point_at(lo),
                t_a: lo,
                t_b: lo - offset,
                contact: Contact::Tangent,
            });
        } else {
            out.overlaps.push(Overlap {
                a: (lo, hi),
                b: (lo - offset, hi - offset),
                start: a.point_at(lo),
                end: a.point_at(hi),
                same_direction,
            });
        }
    };
    if wb >= TAU {
        // b is a whole circle: all of a, at b's angle of a's start onward.
        add(a0, a0 + wa, a0 - check::angle_from(a0, b0));
    } else if wa >= TAU {
        // a is a whole circle: all of b, from a's angle of b's start.
        let lo = check::angle_from(b0, a0);
        add(lo, lo + wb, lo - b0);
    } else {
        // Copies of b one turn apart, the first starting in [a0, a0 + 2π). Each meets a in
        // at most one stretch, and the stretches are disjoint because both arcs are
        // shorter than a full turn.
        let first = check::angle_from(b0, a0);
        for k in [-1.0, 0.0] {
            let lo_b = first + k * TAU;
            add(a0.max(lo_b), (a0 + wa).min(lo_b + wb), lo_b - b0);
        }
    }
}

/// Segment and segment.
pub fn segment_segment(a: &Segment2, b: &Segment2) -> Intersections {
    line_like_pair(LineLike::of_segment(a), LineLike::of_segment(b))
}

/// Line and segment.
pub fn line_segment(a: &Line2, b: &Segment2) -> Intersections {
    line_like_pair(LineLike::of_line(a), LineLike::of_segment(b))
}

/// Line and line. Coincident lines give one overlap with infinite parameter bounds.
pub fn line_line(a: &Line2, b: &Line2) -> Intersections {
    line_like_pair(LineLike::of_line(a), LineLike::of_line(b))
}

fn line_like_pair(a: LineLike, b: LineLike) -> Intersections {
    let mut out = Intersections::default();
    let w = b.origin - a.origin;
    let denom = a.dir.cross(b.dir);
    if denom.abs() > ANGULAR_TOLERANCE {
        // a.origin + s a.dir = b.origin + u b.dir; cross both sides with b.dir and a.dir.
        let s = w.cross(b.dir) / denom;
        let u = w.cross(a.dir) / denom;
        if let (Some(t_a), Some(t_b)) = (a.accept(s), b.accept(u)) {
            let point = a.origin + a.dir * s.clamp(a.range.0, a.range.1);
            out.points.push(IntersectionPoint {
                point,
                t_a,
                t_b,
                contact: Contact::Crossing,
            });
        }
        return out;
    }
    // Parallel: apart, or on one line.
    if w.cross(a.dir).abs() > LINEAR_TOLERANCE {
        return out;
    }
    // b's range mapped to distances along a (b's direction is ±a's).
    let along = b.dir.dot(a.dir);
    let base = w.dot(a.dir);
    let map = |u: f64| {
        if u.is_infinite() {
            u * along
        } else {
            base + along * u
        }
    };
    let (e0, e1) = (map(b.range.0), map(b.range.1));
    let (lo, hi) = (a.range.0.max(e0.min(e1)), a.range.1.min(e0.max(e1)));
    if hi - lo < -LINEAR_TOLERANCE {
        return out;
    }
    let to_b = |s: f64| {
        if s.is_infinite() {
            s * along
        } else {
            (s - base) * along
        }
    };
    let b_param = |s: f64| {
        let u = to_b(s).clamp(b.range.0, b.range.1);
        u * b.per_mm
    };
    let point_at = |s: f64| a.origin + a.dir * s;
    if hi - lo <= LINEAR_TOLERANCE {
        let s = 0.5 * (lo + hi);
        out.points.push(IntersectionPoint {
            point: point_at(s),
            t_a: s * a.per_mm,
            t_b: b_param(s),
            contact: Contact::Tangent,
        });
    } else {
        out.overlaps.push(Overlap {
            a: (lo * a.per_mm, hi * a.per_mm),
            b: (b_param(lo), b_param(hi)),
            start: point_at(lo),
            end: point_at(hi),
            same_direction: along > 0.0,
        });
    }
    out
}

fn circle_like(c: &Circle2) -> CircleLike<'_> {
    CircleLike {
        center: c.center(),
        radius: c.radius(),
        arc: None,
    }
}

fn arc_like(a: &Arc2) -> CircleLike<'_> {
    CircleLike {
        center: a.center(),
        radius: a.radius(),
        arc: Some(a),
    }
}

/// Line and circle.
pub fn line_circle(a: &Line2, b: &Circle2) -> Intersections {
    line_vs_circle(LineLike::of_line(a), circle_like(b))
}

/// Line and arc.
pub fn line_arc(a: &Line2, b: &Arc2) -> Intersections {
    line_vs_circle(LineLike::of_line(a), arc_like(b))
}

/// Segment and circle.
pub fn segment_circle(a: &Segment2, b: &Circle2) -> Intersections {
    line_vs_circle(LineLike::of_segment(a), circle_like(b))
}

/// Segment and arc.
pub fn segment_arc(a: &Segment2, b: &Arc2) -> Intersections {
    line_vs_circle(LineLike::of_segment(a), arc_like(b))
}

/// Circle and circle.
pub fn circle_circle(a: &Circle2, b: &Circle2) -> Intersections {
    circle_vs_circle(circle_like(a), circle_like(b))
}

/// Circle and arc.
pub fn circle_arc(a: &Circle2, b: &Arc2) -> Intersections {
    circle_vs_circle(circle_like(a), arc_like(b))
}

/// Arc and arc.
pub fn arc_arc(a: &Arc2, b: &Arc2) -> Intersections {
    circle_vs_circle(arc_like(a), arc_like(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    const TOL: f64 = LINEAR_TOLERANCE;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }
    fn seg(a: (f64, f64), b: (f64, f64)) -> Segment2 {
        Segment2::new(p(a.0, a.1), p(b.0, b.1)).unwrap()
    }
    fn circle(x: f64, y: f64, r: f64) -> Circle2 {
        Circle2::new(p(x, y), r).unwrap()
    }
    fn arc(x: f64, y: f64, r: f64, start: f64, sweep: f64) -> Arc2 {
        Arc2::new(p(x, y), r, start, sweep).unwrap()
    }
    fn near(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-9
    }
    fn has_point(r: &Intersections, q: Point2) -> bool {
        r.points.iter().any(|i| near(i.point, q))
    }

    #[test]
    fn two_circles_of_radius_5_eight_apart_meet_at_4_plus_minus_3() {
        // x = (64 + 25 - 25) / 16 = 4, h = sqrt(25 - 16) = 3.
        let r = circle_circle(&circle(0.0, 0.0, 5.0), &circle(8.0, 0.0, 5.0));
        assert_eq!(r.points.len(), 2);
        assert!(r.overlaps.is_empty());
        assert!(has_point(&r, p(4.0, 3.0)) && has_point(&r, p(4.0, -3.0)));
        for i in &r.points {
            assert_eq!(i.contact, Contact::Crossing);
            // Parameters are the angles on each circle.
            let on_a = circle(0.0, 0.0, 5.0).point_at(i.t_a);
            let on_b = circle(8.0, 0.0, 5.0).point_at(i.t_b);
            assert!(near(on_a, i.point) && near(on_b, i.point));
        }
        let top = r.points.iter().find(|i| i.point.y > 0.0).unwrap();
        assert!((top.t_a - 3f64.atan2(4.0)).abs() < 1e-12);
        assert!((top.t_b - (PI - 3f64.atan2(4.0))).abs() < 1e-12);
    }

    #[test]
    fn tangent_circles_meet_at_one_point() {
        // External: r 3 and r 2 with centres 5 apart touch at (3, 0).
        let r = circle_circle(&circle(0.0, 0.0, 3.0), &circle(5.0, 0.0, 2.0));
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].contact, Contact::Tangent);
        assert!(near(r.points[0].point, p(3.0, 0.0)));
        // Internal: r 5 and r 2 with centres 3 apart touch at (5, 0); either order.
        let pairs = [
            (circle(0.0, 0.0, 5.0), circle(3.0, 0.0, 2.0)),
            (circle(3.0, 0.0, 2.0), circle(0.0, 0.0, 5.0)),
        ];
        for (a, b) in pairs {
            let r = circle_circle(&a, &b);
            assert_eq!(r.points.len(), 1);
            assert_eq!(r.points[0].contact, Contact::Tangent);
            assert!(near(r.points[0].point, p(5.0, 0.0)));
        }
        // Within tolerance of tangency it is still tangent; beyond, apart or crossing.
        let r = circle_circle(&circle(0.0, 0.0, 3.0), &circle(5.0 + 0.5 * TOL, 0.0, 2.0));
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].contact, Contact::Tangent);
        let apart = circle_circle(&circle(0.0, 0.0, 3.0), &circle(5.0 + 2.0 * TOL, 0.0, 2.0));
        assert!(apart.is_empty());
        let r = circle_circle(&circle(0.0, 0.0, 3.0), &circle(5.0 - 2.0 * TOL, 0.0, 2.0));
        assert_eq!(r.points.len(), 2);
        assert!(r.points.iter().all(|i| i.contact == Contact::Crossing));
    }

    #[test]
    fn separate_nested_and_concentric_circles_do_not_meet() {
        assert!(circle_circle(&circle(0.0, 0.0, 1.0), &circle(5.0, 0.0, 1.0)).is_empty());
        assert!(circle_circle(&circle(0.0, 0.0, 5.0), &circle(1.0, 0.0, 1.0)).is_empty());
        assert!(circle_circle(&circle(0.0, 0.0, 5.0), &circle(0.0, 0.0, 4.0)).is_empty());
        // Equal concentric circles coincide everywhere.
        let r = circle_circle(&circle(1.0, 1.0, 5.0), &circle(1.0, 1.0 + 0.5 * TOL, 5.0));
        assert!(r.points.is_empty());
        assert_eq!(r.overlaps.len(), 1);
        let o = r.overlaps[0];
        assert_eq!((o.a, o.b), ((0.0, TAU), (0.0, TAU)));
        assert!(o.same_direction);
    }

    #[test]
    fn crossing_and_touching_segments() {
        let r = segment_segment(&seg((0.0, 0.0), (2.0, 2.0)), &seg((0.0, 2.0), (2.0, 0.0)));
        assert_eq!(r.points.len(), 1);
        let i = r.points[0];
        assert!(near(i.point, p(1.0, 1.0)));
        assert!((i.t_a - 0.5).abs() < 1e-15 && (i.t_b - 0.5).abs() < 1e-15);
        assert_eq!(i.contact, Contact::Crossing);
        // A T junction: one segment ends on the other.
        let base = seg((0.0, 0.0), (2.0, 0.0));
        let r = segment_segment(&base, &seg((1.0, 0.0), (1.0, 5.0)));
        assert_eq!(r.points.len(), 1);
        assert_eq!((r.points[0].t_a, r.points[0].t_b), (0.5, 0.0));
        // Ending within tolerance of the other segment counts; twice the tolerance does not.
        let r = segment_segment(&base, &seg((1.0, 0.5 * TOL), (1.0, 5.0)));
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].t_b, 0.0);
        assert!(segment_segment(&base, &seg((1.0, 2.0 * TOL), (1.0, 5.0))).is_empty());
        // Lines that would cross beyond the segments' ends.
        let short = seg((0.0, 0.0), (1.0, 0.0));
        assert!(segment_segment(&short, &seg((2.0, -1.0), (2.0, 1.0))).is_empty());
    }

    #[test]
    fn parallel_and_collinear_segments() {
        let a = seg((0.0, 0.0), (4.0, 0.0));
        // Parallel, one apart.
        assert!(segment_segment(&a, &seg((0.0, 1.0), (4.0, 1.0))).is_empty());
        // Collinear, overlapping over [2, 4] x {0}: a's t in [0.5, 1], b's in [0, 0.5].
        let r = segment_segment(&a, &seg((2.0, 0.0), (6.0, 0.0)));
        assert!(r.points.is_empty());
        assert_eq!(r.overlaps.len(), 1);
        let o = r.overlaps[0];
        assert!((o.a.0 - 0.5).abs() < 1e-15 && (o.a.1 - 1.0).abs() < 1e-15);
        assert!(o.b.0.abs() < 1e-15 && (o.b.1 - 0.5).abs() < 1e-15);
        assert!(near(o.start, p(2.0, 0.0)) && near(o.end, p(4.0, 0.0)) && o.same_direction);
        // The same running the other way: b = (6,0) -> (2,0) has t 1 at (2,0), 0.5 at (4,0).
        let back = seg((6.0, 0.0), (2.0, 0.0));
        let o = segment_segment(&a, &back).overlaps[0];
        assert!((o.b.0 - 1.0).abs() < 1e-15 && (o.b.1 - 0.5).abs() < 1e-15 && !o.same_direction);
        // One inside the other.
        let o = segment_segment(&a, &seg((1.0, 0.0), (3.0, 0.0))).overlaps[0];
        assert!((o.a.0 - 0.25).abs() < 1e-15 && (o.a.1 - 0.75).abs() < 1e-15);
        // End to end: a single tangent point.
        let r = segment_segment(&a, &seg((4.0, 0.0), (9.0, 0.0)));
        assert!(r.overlaps.is_empty());
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].contact, Contact::Tangent);
        assert!(near(r.points[0].point, p(4.0, 0.0)));
        assert!((r.points[0].t_a - 1.0).abs() < 1e-12 && r.points[0].t_b.abs() < 1e-12);
        // Collinear with a gap.
        assert!(segment_segment(&a, &seg((5.0, 0.0), (9.0, 0.0))).is_empty());
        // Swapping the curves: b = (6,0)->(2,0) first, its t from 0.5 at (4,0) to 1 at (2,0).
        let o = segment_segment(&back, &a).overlaps[0];
        assert!(o.a.0 < o.a.1 && !o.same_direction);
        assert!((o.a.0 - 0.5).abs() < 1e-15 && (o.b.0 - 1.0).abs() < 1e-15);
        let sw = segment_segment(&a, &back).swapped();
        assert_eq!(sw.overlaps.len(), 1);
        assert!((sw.overlaps[0].a.0 - o.a.0).abs() < 1e-15);
        assert!((sw.overlaps[0].b.0 - o.b.0).abs() < 1e-15);
    }

    #[test]
    fn lines_meet_circles_in_two_one_or_no_points() {
        let c5 = circle(0.0, 0.0, 5.0);
        // y = 3 cuts x^2 + y^2 = 25 at x = ±4.
        let r = line_circle(&Line2::new(p(-10.0, 3.0), Vec2::X).unwrap(), &c5);
        assert_eq!(r.points.len(), 2);
        assert!(has_point(&r, p(-4.0, 3.0)) && has_point(&r, p(4.0, 3.0)));
        let left = r.points.iter().find(|i| i.point.x < 0.0).unwrap();
        // The line's parameter is the distance from (-10, 3): 6.
        assert!((left.t_a - 6.0).abs() < 1e-12);
        // y = 5 touches at (0, 5).
        let r = line_circle(&Line2::new(p(-10.0, 5.0), Vec2::X).unwrap(), &c5);
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].contact, Contact::Tangent);
        assert!(near(r.points[0].point, p(0.0, 5.0)));
        assert!((r.points[0].t_b - FRAC_PI_2).abs() < 1e-12);
        let above = Line2::new(p(0.0, 5.0 + 2.0 * TOL), Vec2::X).unwrap();
        assert!(line_circle(&above, &c5).is_empty());
        // Through the centre at 45 degrees: (±5/sqrt 2, ±5/sqrt 2).
        let diag = Line2::new(p(0.0, 0.0), Vec2::new(1.0, 1.0)).unwrap();
        let r = line_circle(&diag, &c5);
        let c = 5.0 / 2f64.sqrt();
        assert!(has_point(&r, p(c, c)) && has_point(&r, p(-c, -c)));
    }

    #[test]
    fn segments_and_arcs_keep_only_points_on_both() {
        let upper = arc(0.0, 0.0, 5.0, 0.0, PI);
        let lower = arc(0.0, 0.0, 5.0, PI, PI);
        let s = seg((-6.0, 3.0), (6.0, 3.0));
        let r = segment_arc(&s, &upper);
        assert_eq!(r.points.len(), 2);
        // (4, 3) is at t = 10/12 on the segment and angle atan2(3, 4) on the arc.
        let right = r.points.iter().find(|i| i.point.x > 0.0).unwrap();
        assert!((right.t_a - 10.0 / 12.0).abs() < 1e-12);
        assert!((right.t_b - 3f64.atan2(4.0)).abs() < 1e-12);
        assert!(segment_arc(&s, &lower).is_empty());
        // A short segment reaching only (4, 3).
        let r = segment_circle(&seg((0.0, 3.0), (6.0, 3.0)), &circle(0.0, 0.0, 5.0));
        assert_eq!(r.points.len(), 1);
        // Tangent at the arc's start point (5, 0).
        let r = segment_arc(
            &seg((5.0, -1.0), (5.0, 1.0)),
            &arc(0.0, 0.0, 5.0, 0.0, FRAC_PI_2),
        );
        assert_eq!(r.points.len(), 1);
        assert_eq!(r.points[0].contact, Contact::Tangent);
        assert!(r.points[0].t_b.abs() < 1e-12);
        // A clockwise arc reports angles in its range: from -π/2 (end) to 0 (start).
        let cw = arc(0.0, 0.0, 5.0, 0.0, -FRAC_PI_2);
        let r = segment_arc(&seg((0.0, -3.0), (6.0, -3.0)), &cw);
        assert_eq!(r.points.len(), 1);
        assert!((r.points[0].t_b - (-3f64).atan2(4.0)).abs() < 1e-12);
        let r = line_arc(&Line2::new(p(0.0, 3.0), Vec2::X).unwrap(), &upper);
        assert_eq!(r.points.len(), 2);
    }

    #[test]
    fn arcs_on_different_circles() {
        // Upper halves of the two r = 5 circles 8 apart share only (4, 3).
        let r = arc_arc(&arc(0.0, 0.0, 5.0, 0.0, PI), &arc(8.0, 0.0, 5.0, 0.0, PI));
        assert_eq!(r.points.len(), 1);
        assert!(near(r.points[0].point, p(4.0, 3.0)));
        // The lower half of the second circle meets the upper half of the first nowhere.
        assert!(arc_arc(&arc(0.0, 0.0, 5.0, 0.0, PI), &arc(8.0, 0.0, 5.0, PI, PI)).is_empty());
        let r = circle_arc(&circle(0.0, 0.0, 5.0), &arc(8.0, 0.0, 5.0, PI, PI));
        assert_eq!(r.points.len(), 1);
        assert!(near(r.points[0].point, p(4.0, -3.0)));
    }

    #[test]
    fn arcs_on_the_same_circle_overlap_or_touch() {
        let on5 = |start: f64, sweep: f64| arc(0.0, 0.0, 5.0, start, sweep);
        // [0, π] and [π/2, 3π/2] share [π/2, π].
        let r = arc_arc(&on5(0.0, PI), &on5(FRAC_PI_2, PI));
        assert!(r.points.is_empty());
        assert_eq!(r.overlaps.len(), 1);
        let o = r.overlaps[0];
        assert!((o.a.0 - FRAC_PI_2).abs() < 1e-12 && (o.a.1 - PI).abs() < 1e-12);
        assert!((o.b.0 - FRAC_PI_2).abs() < 1e-12 && (o.b.1 - PI).abs() < 1e-12);
        assert!(near(o.start, p(0.0, 5.0)) && near(o.end, p(-5.0, 0.0)) && o.same_direction);
        // Complementary halves touch at both ends, (-5, 0) and (5, 0).
        let r = arc_arc(&on5(0.0, PI), &on5(PI, PI));
        assert!(r.overlaps.is_empty());
        assert_eq!(r.points.len(), 2);
        assert!(has_point(&r, p(5.0, 0.0)) && has_point(&r, p(-5.0, 0.0)));
        assert!(r.points.iter().all(|i| i.contact == Contact::Tangent));
        // Wrapping through angle 0: [3π/2, 5π/2] and [0, π/2] share the first quadrant;
        // the first arc's angles there are 2π..5π/2.
        let r = arc_arc(&on5(1.5 * PI, PI), &on5(0.0, FRAC_PI_2));
        assert_eq!(r.overlaps.len(), 1);
        let o = r.overlaps[0];
        assert!((o.a.0 - TAU).abs() < 1e-12 && (o.a.1 - 2.5 * PI).abs() < 1e-12);
        assert!(o.b.0.abs() < 1e-12 && (o.b.1 - FRAC_PI_2).abs() < 1e-12);
        // Two 300-degree arcs, [0, 5π/3] and [π, 8π/3], overlap in two stretches:
        // [π, 5π/3] and [0, 2π/3] (the latter is b's [2π, 8π/3]).
        let r = arc_arc(&on5(0.0, 5.0 * PI / 3.0), &on5(PI, 5.0 * PI / 3.0));
        assert_eq!(r.overlaps.len(), 2);
        let mut s: Vec<_> = r.overlaps.iter().map(|o| (o.a, o.b)).collect();
        s.sort_by(|x, y| x.0.0.total_cmp(&y.0.0));
        assert!(s[0].0.0.abs() < 1e-12 && (s[0].0.1 - 2.0 * PI / 3.0).abs() < 1e-12);
        assert!((s[0].1.0 - TAU).abs() < 1e-12 && (s[0].1.1 - 8.0 * PI / 3.0).abs() < 1e-12);
        assert!((s[1].0.0 - PI).abs() < 1e-12 && (s[1].0.1 - 5.0 * PI / 3.0).abs() < 1e-12);
        // Opposite travel: a clockwise arc over the same points.
        let r = arc_arc(&on5(0.0, FRAC_PI_2), &on5(FRAC_PI_2, -FRAC_PI_2));
        assert_eq!(r.overlaps.len(), 1);
        assert!(!r.overlaps[0].same_direction);
        // A circle and an arc on it: the whole arc, at the circle's angles 3π/2..5π/2.
        let r = circle_arc(&circle(0.0, 0.0, 5.0), &on5(-FRAC_PI_2, PI));
        assert_eq!(r.overlaps.len(), 1);
        let o = r.overlaps[0];
        assert!((o.a.0 - 1.5 * PI).abs() < 1e-12 && (o.a.1 - 2.5 * PI).abs() < 1e-12);
        assert!((o.b.0 - 1.5 * PI).abs() < 1e-12);
        // Disjoint arcs on one circle.
        assert!(arc_arc(&on5(0.0, 1.0), &on5(2.0, 1.0)).is_empty());
    }

    #[test]
    fn extreme_but_valid_input_still_answers() {
        // Circles of r = 6e5 at (±5e5, 0): x = 0, h = sqrt(36e10 - 25e10) = sqrt(11e10).
        let r = circle_circle(&circle(-5e5, 0.0, 6e5), &circle(5e5, 0.0, 6e5));
        assert_eq!(r.points.len(), 2);
        let h = 11e10f64.sqrt();
        assert!(r.points.iter().any(|i| i.point.distance(p(0.0, h)) < 1e-6));
        // Tiny circles near the tolerance: r = 1e-6, centres 1.5e-6 apart, x = 7.5e-7.
        let r = circle_circle(&circle(0.0, 0.0, 1e-6), &circle(1.5e-6, 0.0, 1e-6));
        assert_eq!(r.points.len(), 2);
        assert!(r.points.iter().all(|i| (i.point.x - 7.5e-7).abs() < 1e-15));
        // Nearly parallel segments far from the origin cross at their middles.
        let flat = seg((-9e5, 9e5), (9e5, 9e5));
        let r = segment_segment(&flat, &seg((-9e5, 9e5 - 1.0), (9e5, 9e5 + 1.0)));
        assert_eq!(r.points.len(), 1);
        assert!(near(r.points[0].point, p(0.0, 9e5)));
        // Coincident infinite lines report an unbounded overlap rather than failing.
        let l = Line2::new(p(0.0, 0.0), Vec2::X).unwrap();
        let r = line_line(&l, &Line2::new(p(5.0, 0.0), -Vec2::X).unwrap());
        assert_eq!(r.overlaps.len(), 1);
        assert!(!r.overlaps[0].same_direction);
        let r = line_segment(&l, &seg((1.0, 0.0), (2.0, 0.0)));
        assert_eq!(r.overlaps[0].a, (1.0, 2.0));
        let r = line_line(&l, &Line2::new(p(3.0, -1.0), Vec2::new(1.0, 1.0)).unwrap());
        assert!(near(r.points[0].point, p(4.0, 0.0)));
    }
}

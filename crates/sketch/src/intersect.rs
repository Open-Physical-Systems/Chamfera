//! Planar curve geometry for the sketch tools: where lines, arcs and circles meet, and
//! where a point lies along a curve.
//!
//! Intersections are closed-form (no iteration): two lines by Cramer's rule, a line and a
//! circle through the foot of the perpendicular from the centre, two circles through the
//! radical line. Tangency is decided with [`LINEAR_TOLERANCE`]: a line whose distance from
//! the centre is within the tolerance of the radius touches at the foot point, and two
//! circles whose centre distance is within the tolerance of the sum or difference of the
//! radii touch at one point. Parallel lines and concentric circles have no isolated
//! intersection (coincident overlaps are not reported). The sketch crate is on the same
//! layer as the kernel, so it keeps this small helper of its own.

use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::TAU;

/// The geometry of one sketch curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Shape {
    /// A segment from `a` to `b`. Its parameter is the fraction along it (0 at `a`).
    Segment { a: Point2, b: Point2 },
    /// An arc counter-clockwise from angle `a0` through `sweep` (0 < sweep < 2π). Its
    /// parameter is the angle past `a0`.
    Arc {
        c: Point2,
        r: f64,
        a0: f64,
        sweep: f64,
    },
    /// A full circle. Its parameter is the angle from +X, 0..2π.
    Circle { c: Point2, r: f64 },
}

/// An angle wrapped into 0..2π.
pub(crate) fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The arc's start angle and counter-clockwise sweep from its centre, start and end.
pub(crate) fn arc_angles(c: Point2, s: Point2, e: Point2) -> Option<(f64, f64)> {
    let r = c.distance(s);
    if !(r.is_finite() && r > LINEAR_TOLERANCE) {
        return None;
    }
    let a0 = (s - c).angle();
    let mut sweep = (e - c).angle() - a0;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    Some((a0, sweep))
}

impl Shape {
    /// The arc through centre `c` from `s` to `e`, counter-clockwise.
    pub(crate) fn arc(c: Point2, s: Point2, e: Point2) -> Option<Shape> {
        let (a0, sweep) = arc_angles(c, s, e)?;
        Some(Shape::Arc {
            c,
            r: c.distance(s),
            a0,
            sweep,
        })
    }

    /// A length that turns linear tolerances into parameter tolerances.
    fn scale(&self) -> f64 {
        match *self {
            Shape::Segment { a, b } => a.distance(b),
            Shape::Arc { r, .. } | Shape::Circle { r, .. } => r,
        }
        .max(LINEAR_TOLERANCE)
    }

    /// [`LINEAR_TOLERANCE`] in parameter units.
    pub(crate) fn tolerance(&self) -> f64 {
        LINEAR_TOLERANCE / self.scale()
    }

    /// The parameter of the point of the curve's carrier (the infinite line, or the full
    /// circle) nearest `p`. Not clamped: a segment's may be below 0 or above 1, an arc's is
    /// in 0..2π and may be past the sweep.
    pub(crate) fn param(&self, p: Point2) -> f64 {
        match *self {
            Shape::Segment { a, b } => {
                let d = b - a;
                let len2 = d.dot(d);
                if len2 > 0.0 {
                    (p - a).dot(d) / len2
                } else {
                    0.0
                }
            }
            Shape::Arc { c, a0, .. } => wrap((p - c).angle() - a0),
            Shape::Circle { c, .. } => wrap((p - c).angle()),
        }
    }

    /// The point at parameter `t`.
    pub(crate) fn point_at(&self, t: f64) -> Point2 {
        match *self {
            Shape::Segment { a, b } => a.lerp(b, t),
            Shape::Arc { c, r, a0, .. } => c + Vec2::new((a0 + t).cos(), (a0 + t).sin()) * r,
            Shape::Circle { c, r } => c + Vec2::new(t.cos(), t.sin()) * r,
        }
    }

    /// True when parameter `t` is on the curve, ends included, within the tolerance.
    pub(crate) fn on(&self, t: f64) -> bool {
        let tol = self.tolerance();
        match *self {
            Shape::Segment { .. } => (-tol..=1.0 + tol).contains(&t),
            Shape::Arc { sweep, .. } => t <= sweep + tol || t >= TAU - tol,
            Shape::Circle { .. } => true,
        }
    }

    /// True when parameter `t` is inside the curve, away from its ends by more than the
    /// tolerance. Every point of a circle is inside.
    pub(crate) fn interior(&self, t: f64) -> bool {
        let tol = self.tolerance();
        match *self {
            Shape::Segment { .. } => t > tol && t < 1.0 - tol,
            Shape::Arc { sweep, .. } => t > tol && t < sweep - tol,
            Shape::Circle { .. } => true,
        }
    }

    /// The piece of the curve from parameter `t0` to `t1` as a polyline (for previews).
    pub(crate) fn polyline(&self, t0: f64, t1: f64) -> Vec<Point2> {
        match *self {
            Shape::Segment { .. } => vec![self.point_at(t0), self.point_at(t1)],
            Shape::Arc { .. } | Shape::Circle { .. } => {
                let span = t1 - t0;
                let n = ((span.abs() / TAU * 96.0).ceil() as usize).clamp(2, 96);
                (0..=n)
                    .map(|k| self.point_at(t0 + span * k as f64 / n as f64))
                    .collect()
            }
        }
    }
}

/// The infinite line or full circle that carries a curve.
#[derive(Clone, Copy)]
enum Carrier {
    Line { a: Point2, d: Vec2 },
    Circle { c: Point2, r: f64 },
}

fn carrier(s: &Shape) -> Carrier {
    match *s {
        Shape::Segment { a, b } => Carrier::Line { a, d: b - a },
        Shape::Arc { c, r, .. } | Shape::Circle { c, r } => Carrier::Circle { c, r },
    }
}

fn line_line(a1: Point2, d1: Vec2, a2: Point2, d2: Vec2) -> Vec<Point2> {
    let den = d1.cross(d2);
    if den.abs() <= ANGULAR_TOLERANCE * d1.length() * d2.length() || !den.is_finite() {
        return Vec::new();
    }
    let t = (a2 - a1).cross(d2) / den;
    vec![a1 + d1 * t]
}

fn line_circle(a: Point2, d: Vec2, c: Point2, r: f64) -> Vec<Point2> {
    let Some(u) = d.normalized() else {
        return Vec::new();
    };
    let foot = a + u * (c - a).dot(u);
    let h = c.distance(foot);
    if h > r + LINEAR_TOLERANCE {
        Vec::new()
    } else if (h - r).abs() <= LINEAR_TOLERANCE {
        vec![foot]
    } else {
        let k = (r * r - h * h).max(0.0).sqrt();
        vec![foot - u * k, foot + u * k]
    }
}

fn circle_circle(c1: Point2, r1: f64, c2: Point2, r2: f64) -> Vec<Point2> {
    let v = c2 - c1;
    let d = v.length();
    if d < LINEAR_TOLERANCE
        || d > r1 + r2 + LINEAR_TOLERANCE
        || d < (r1 - r2).abs() - LINEAR_TOLERANCE
    {
        return Vec::new();
    }
    let u = v * (1.0 / d);
    let a = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let base = c1 + u * a;
    let h2 = r1 * r1 - a * a;
    let touching = (d - (r1 + r2)).abs() <= LINEAR_TOLERANCE
        || (d - (r1 - r2).abs()).abs() <= LINEAR_TOLERANCE;
    if touching || h2 <= 0.0 {
        vec![base]
    } else {
        let h = h2.sqrt();
        vec![base + u.perp() * h, base - u.perp() * h]
    }
}

/// Where the carriers of two curves meet (lines are infinite, arcs are full circles).
pub(crate) fn carrier_intersections(s1: &Shape, s2: &Shape) -> Vec<Point2> {
    let pts = match (carrier(s1), carrier(s2)) {
        (Carrier::Line { a: a1, d: d1 }, Carrier::Line { a: a2, d: d2 }) => {
            line_line(a1, d1, a2, d2)
        }
        (Carrier::Line { a, d }, Carrier::Circle { c, r })
        | (Carrier::Circle { c, r }, Carrier::Line { a, d }) => line_circle(a, d, c, r),
        (Carrier::Circle { c: c1, r: r1 }, Carrier::Circle { c: c2, r: r2 }) => {
            circle_circle(c1, r1, c2, r2)
        }
    };
    pts.into_iter()
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .collect()
}

/// Where two curves meet, ends included.
pub(crate) fn intersections(s1: &Shape, s2: &Shape) -> Vec<Point2> {
    carrier_intersections(s1, s2)
        .into_iter()
        .filter(|p| s1.on(s1.param(*p)) && s2.on(s2.param(*p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn close(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-12
    }

    #[test]
    fn crossing_segments_meet_at_one_exact_point() {
        let a = Shape::Segment {
            a: p(-10.0, 0.0),
            b: p(10.0, 0.0),
        };
        let b = Shape::Segment {
            a: p(3.0, -5.0),
            b: p(3.0, 5.0),
        };
        let x = intersections(&a, &b);
        assert_eq!(x.len(), 1);
        assert!(close(x[0], p(3.0, 0.0)));
        // Short of each other: the carriers meet but the segments do not.
        let c = Shape::Segment {
            a: p(3.0, 1.0),
            b: p(3.0, 5.0),
        };
        assert!(intersections(&a, &c).is_empty());
        assert_eq!(carrier_intersections(&a, &c).len(), 1);
        // Parallel lines never meet.
        let d = Shape::Segment {
            a: p(-10.0, 1.0),
            b: p(10.0, 1.0),
        };
        assert!(carrier_intersections(&a, &d).is_empty());
    }

    #[test]
    fn a_line_cuts_a_circle_twice_and_a_tangent_touches_once() {
        let circle = Shape::Circle {
            c: p(0.0, 0.0),
            r: 5.0,
        };
        // y = 3 cuts x² = 25 - 9 at x = ±4.
        let cut = Shape::Segment {
            a: p(-10.0, 3.0),
            b: p(10.0, 3.0),
        };
        let x = intersections(&circle, &cut);
        assert_eq!(x.len(), 2);
        assert!(x.iter().any(|q| close(*q, p(-4.0, 3.0))));
        assert!(x.iter().any(|q| close(*q, p(4.0, 3.0))));
        // y = 5 touches at (0, 5); y = 5 + tolerance / 2 still touches.
        for y in [5.0, 5.0 + LINEAR_TOLERANCE * 0.5] {
            let tangent = Shape::Segment {
                a: p(-10.0, y),
                b: p(10.0, y),
            };
            let t = intersections(&circle, &tangent);
            assert_eq!(t.len(), 1, "{y}");
            assert!(t[0].distance(p(0.0, y)) < 1e-12);
        }
        let miss = Shape::Segment {
            a: p(-10.0, 5.0 + 1e-6),
            b: p(10.0, 5.0 + 1e-6),
        };
        assert!(intersections(&circle, &miss).is_empty());
    }

    #[test]
    fn two_circles_meet_on_their_radical_line() {
        // Centres 6 apart, radii 5: a = 3, h = 4.
        let a = Shape::Circle {
            c: p(0.0, 0.0),
            r: 5.0,
        };
        let b = Shape::Circle {
            c: p(6.0, 0.0),
            r: 5.0,
        };
        let x = intersections(&a, &b);
        assert_eq!(x.len(), 2);
        assert!(x.iter().any(|q| close(*q, p(3.0, 4.0))));
        assert!(x.iter().any(|q| close(*q, p(3.0, -4.0))));
        // Touching outside at (5, 0), concentric never.
        let c = Shape::Circle {
            c: p(10.0, 0.0),
            r: 5.0,
        };
        let t = intersections(&a, &c);
        assert_eq!(t.len(), 1);
        assert!(close(t[0], p(5.0, 0.0)));
        let d = Shape::Circle {
            c: p(0.0, 0.0),
            r: 2.0,
        };
        assert!(intersections(&a, &d).is_empty());
    }

    #[test]
    fn arcs_only_meet_inside_their_sweep() {
        // The upper half of the circle r = 5.
        let arc = Shape::Arc {
            c: p(0.0, 0.0),
            r: 5.0,
            a0: 0.0,
            sweep: PI,
        };
        let below = Shape::Segment {
            a: p(-10.0, -3.0),
            b: p(10.0, -3.0),
        };
        assert!(intersections(&arc, &below).is_empty());
        let above = Shape::Segment {
            a: p(-10.0, 3.0),
            b: p(10.0, 3.0),
        };
        assert_eq!(intersections(&arc, &above).len(), 2);
        // Parameters: (0, 5) is a quarter turn past the start.
        assert!((arc.param(p(0.0, 5.0)) - FRAC_PI_2).abs() < 1e-15);
        assert!(arc.interior(FRAC_PI_2) && !arc.interior(0.0) && !arc.interior(PI));
    }

    #[test]
    fn degenerate_input_gives_no_points_and_no_panic() {
        let dot = Shape::Segment {
            a: p(1.0, 1.0),
            b: p(1.0, 1.0),
        };
        let circle = Shape::Circle {
            c: p(0.0, 0.0),
            r: 5.0,
        };
        assert!(intersections(&dot, &circle).is_empty());
        let nan = Shape::Segment {
            a: p(f64::NAN, 0.0),
            b: p(1.0, 0.0),
        };
        assert!(intersections(&nan, &circle).is_empty());
        assert!(intersections(&nan, &dot).is_empty());
        assert_eq!(wrap(-1e-300), 0.0);
        assert!(wrap(-0.5) > 0.0 && wrap(TAU) == 0.0);
    }
}

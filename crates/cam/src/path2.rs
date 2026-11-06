//! Exact planar paths of lines and circular arcs, in the machine's XY plane.
//!
//! Toolpaths for 2.5D milling are made of lines and arcs only, so they post as G1, G2 and
//! G3 moves without being broken into tiny lines. Everything here is exact for lines and
//! arcs: offsets, intersections, closest points and areas are closed-form (circle and line
//! equations as in any analytic-geometry text; Held, "On the Computational Geometry of
//! Pocket Machining", 1991, ch. 2, uses the same primitives).

use chamfera_geom::{Point2, Vec2};
use std::f64::consts::{PI, TAU};

/// Two points closer than this (mm) are the same point when paths are joined. Larger
/// than `LINEAR_TOLERANCE` because offsets and intersections each add a little rounding.
pub const JOIN_TOLERANCE: f64 = 1e-6;

/// A line or a circular arc in the XY plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Line {
        a: Point2,
        b: Point2,
    },
    /// Centre `c`, radius `r`, starting at angle `start` (from +X) and turning by `sweep`
    /// radians: positive counter-clockwise, negative clockwise, `|sweep| <= 2π`.
    Arc {
        c: Point2,
        r: f64,
        start: f64,
        sweep: f64,
    },
}

fn polar(c: Point2, r: f64, angle: f64) -> Point2 {
    let (s, co) = angle.sin_cos();
    Point2::new(c.x + r * co, c.y + r * s)
}

/// `a` in `[0, 2π)`.
pub fn norm_angle(a: f64) -> f64 {
    let x = a.rem_euclid(TAU);
    if x >= TAU { 0.0 } else { x }
}

impl Seg {
    pub fn line(a: Point2, b: Point2) -> Seg {
        Seg::Line { a, b }
    }

    /// The arc about `c` from `from` to `to` (both taken at their own distance from `c`;
    /// the radius is `from`'s), counter-clockwise when `ccw`. Equal ends give a full turn.
    pub fn arc_between(c: Point2, from: Point2, to: Point2, ccw: bool) -> Seg {
        let r = from.distance(c);
        let a0 = (from - c).angle();
        let a1 = (to - c).angle();
        let mut sweep = norm_angle(if ccw { a1 - a0 } else { a0 - a1 });
        if sweep * r <= JOIN_TOLERANCE {
            sweep = TAU;
        }
        Seg::Arc {
            c,
            r,
            start: a0,
            sweep: if ccw { sweep } else { -sweep },
        }
    }

    pub fn start(&self) -> Point2 {
        match *self {
            Seg::Line { a, .. } => a,
            Seg::Arc { c, r, start, .. } => polar(c, r, start),
        }
    }

    pub fn end(&self) -> Point2 {
        match *self {
            Seg::Line { b, .. } => b,
            Seg::Arc { c, r, start, sweep } => polar(c, r, start + sweep),
        }
    }

    pub fn length(&self) -> f64 {
        match *self {
            Seg::Line { a, b } => a.distance(b),
            Seg::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }

    /// The point at `t` in `[0, 1]` along the segment.
    pub fn point_at(&self, t: f64) -> Point2 {
        match *self {
            Seg::Line { a, b } => a.lerp(b, t),
            Seg::Arc { c, r, start, sweep } => polar(c, r, start + sweep * t),
        }
    }

    /// The unit direction of travel at `t` (`+X` for a degenerate segment).
    pub fn tangent_at(&self, t: f64) -> Vec2 {
        match *self {
            Seg::Line { a, b } => (b - a).normalized().unwrap_or(Vec2::X),
            Seg::Arc { start, sweep, .. } => {
                let (s, c) = (start + sweep * t).sin_cos();
                let radial_left = Vec2::new(-s, c);
                if sweep >= 0.0 {
                    radial_left
                } else {
                    -radial_left
                }
            }
        }
    }

    pub fn reversed(&self) -> Seg {
        match *self {
            Seg::Line { a, b } => Seg::Line { a: b, b: a },
            Seg::Arc { c, r, start, sweep } => Seg::Arc {
                c,
                r,
                start: start + sweep,
                sweep: -sweep,
            },
        }
    }

    /// The part from `t0` to `t1` (fractions along the segment, `t0 < t1`).
    pub fn sub(&self, t0: f64, t1: f64) -> Seg {
        match *self {
            Seg::Line { .. } => Seg::Line {
                a: self.point_at(t0),
                b: self.point_at(t1),
            },
            Seg::Arc { c, r, start, sweep } => Seg::Arc {
                c,
                r,
                start: start + sweep * t0,
                sweep: sweep * (t1 - t0),
            },
        }
    }

    /// Where `p` lies along an arc, as a fraction, if its direction from the centre is
    /// within the sweep (allowing `slack` mm at the ends).
    fn arc_param(c: Point2, r: f64, start: f64, sweep: f64, p: Point2, slack: f64) -> Option<f64> {
        let a = (p - c).angle();
        let along = if sweep >= 0.0 {
            norm_angle(a - start)
        } else {
            norm_angle(start - a)
        };
        let total = sweep.abs();
        let slack_angle = if r > 0.0 { slack / r } else { 0.0 };
        if along <= total + slack_angle {
            Some((along / total.max(f64::MIN_POSITIVE)).min(1.0))
        } else if along >= TAU - slack_angle {
            Some(0.0)
        } else {
            None
        }
    }

    /// The fraction along the segment of the point nearest `p`.
    pub fn closest_param(&self, p: Point2) -> f64 {
        match *self {
            Seg::Line { a, b } => {
                let d = b - a;
                let len2 = d.dot(d);
                if len2 <= f64::MIN_POSITIVE {
                    0.0
                } else {
                    ((p - a).dot(d) / len2).clamp(0.0, 1.0)
                }
            }
            Seg::Arc { c, r, start, sweep } => {
                if p.distance(c) <= f64::MIN_POSITIVE {
                    return 0.0;
                }
                match Seg::arc_param(c, r, start, sweep, p, 0.0) {
                    Some(t) => t,
                    None => {
                        if p.distance(self.start()) <= p.distance(self.end()) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                }
            }
        }
    }

    /// The distance from `p` to the nearest point of the segment.
    pub fn distance(&self, p: Point2) -> f64 {
        p.distance(self.point_at(self.closest_param(p)))
    }

    /// The raw offset to the left of the direction of travel by `d > 0`: a line moves
    /// sideways; an arc changes radius. An arc whose radius would go negative becomes the
    /// "inverted" arc on the far side of its centre (Held 1991, §2.3: the raw offset of a
    /// circular arc), which the trimming step then removes. `None` when the offset arc
    /// shrinks to its centre.
    pub fn offset_left(&self, d: f64) -> Option<Seg> {
        match *self {
            Seg::Line { a, b } => {
                let n = (b - a).normalized()?.perp() * d;
                Some(Seg::Line { a: a + n, b: b + n })
            }
            Seg::Arc { c, r, start, sweep } => {
                // The left of a counter-clockwise arc is its centre.
                let r2 = if sweep >= 0.0 { r - d } else { r + d };
                if r2.abs() <= JOIN_TOLERANCE {
                    None
                } else if r2 > 0.0 {
                    Some(Seg::Arc {
                        c,
                        r: r2,
                        start,
                        sweep,
                    })
                } else {
                    Some(Seg::Arc {
                        c,
                        r: -r2,
                        start: start + PI,
                        sweep,
                    })
                }
            }
        }
    }

    /// An axis-aligned box around the segment: (min, max).
    pub fn bounds(&self) -> (Point2, Point2) {
        match *self {
            Seg::Line { a, b } => (
                Point2::new(a.x.min(b.x), a.y.min(b.y)),
                Point2::new(a.x.max(b.x), a.y.max(b.y)),
            ),
            Seg::Arc { c, r, .. } => (Point2::new(c.x - r, c.y - r), Point2::new(c.x + r, c.y + r)),
        }
    }

    /// `(x dy - y dx) / 2` integrated along the segment: summed round a closed loop it is
    /// the signed area (Green's theorem), positive for a counter-clockwise loop.
    pub fn area_term(&self) -> f64 {
        match *self {
            Seg::Line { a, b } => 0.5 * (a.x * b.y - a.y * b.x),
            Seg::Arc { c, r, start, sweep } => {
                let end = start + sweep;
                0.5 * (r * r * sweep + r * c.x * (end.sin() - start.sin())
                    - r * c.y * (end.cos() - start.cos()))
            }
        }
    }

    /// The angle the segment sweeps as seen from `p` (for winding numbers). Exact for
    /// arcs: from outside the circle the chord gives it; from inside, the direction to
    /// the arc turns the same way as the arc, by its sweep's sense.
    fn subtended(&self, p: Point2) -> f64 {
        let (s, e) = (self.start() - p, self.end() - p);
        let chord = s.cross(e).atan2(s.dot(e));
        match *self {
            Seg::Line { .. } => chord,
            Seg::Arc { c, r, sweep, .. } => {
                if p.distance(c) >= r {
                    chord
                } else if sweep >= 0.0 {
                    let a = norm_angle(chord);
                    if a <= 0.0 && sweep > PI { TAU } else { a }
                } else {
                    let a = -norm_angle(-chord);
                    if a >= 0.0 && sweep < -PI { -TAU } else { a }
                }
            }
        }
    }
}

/// Points where two segments cross or touch, as fractions `(t on a, u on b)`.
pub fn intersect(a: &Seg, b: &Seg) -> Vec<(f64, f64)> {
    let (amin, amax) = a.bounds();
    let (bmin, bmax) = b.bounds();
    let e = JOIN_TOLERANCE;
    if amin.x > bmax.x + e || bmin.x > amax.x + e || amin.y > bmax.y + e || bmin.y > amax.y + e {
        return Vec::new();
    }
    let mut out = Vec::new();
    match (*a, *b) {
        (Seg::Line { a: p0, b: p1 }, Seg::Line { a: q0, b: q1 }) => {
            let (d1, d2) = (p1 - p0, q1 - q0);
            let (l1, l2) = (d1.length(), d2.length());
            if l1 <= f64::MIN_POSITIVE || l2 <= f64::MIN_POSITIVE {
                return out;
            }
            let cross = d1.cross(d2);
            let w = q0 - p0;
            if cross.abs() <= 1e-12 * l1 * l2 {
                // Parallel: only collinear overlaps matter; split at the overlap's ends.
                if (w.cross(d1) / l1).abs() > e {
                    return out;
                }
                for (pt, on_a) in [(q0, true), (q1, true), (p0, false), (p1, false)] {
                    if on_a {
                        let t = (pt - p0).dot(d1) / (l1 * l1);
                        if (-e / l1..=1.0 + e / l1).contains(&t) {
                            out.push((t.clamp(0.0, 1.0), b.closest_param(pt)));
                        }
                    } else {
                        let u = (pt - q0).dot(d2) / (l2 * l2);
                        if (-e / l2..=1.0 + e / l2).contains(&u) {
                            out.push((a.closest_param(pt), u.clamp(0.0, 1.0)));
                        }
                    }
                }
                return out;
            }
            let t = w.cross(d2) / cross;
            let u = w.cross(d1) / cross;
            if (-e / l1..=1.0 + e / l1).contains(&t) && (-e / l2..=1.0 + e / l2).contains(&u) {
                out.push((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)));
            }
        }
        (Seg::Line { a: p0, b: p1 }, Seg::Arc { c, r, start, sweep }) => {
            for (t, pt) in line_circle(p0, p1, c, r) {
                if let Some(u) = Seg::arc_param(c, r, start, sweep, pt, e) {
                    out.push((t, u));
                }
            }
        }
        (Seg::Arc { .. }, Seg::Line { .. }) => {
            out = intersect(b, a).into_iter().map(|(u, t)| (t, u)).collect();
        }
        (
            Seg::Arc {
                c: c1,
                r: r1,
                start: s1,
                sweep: w1,
            },
            Seg::Arc {
                c: c2,
                r: r2,
                start: s2,
                sweep: w2,
            },
        ) => {
            let dist = c1.distance(c2);
            if dist <= e && (r1 - r2).abs() <= e {
                // The same circle: split at the other arc's ends where they overlap.
                for (pt, on_a) in [
                    (b.start(), true),
                    (b.end(), true),
                    (a.start(), false),
                    (a.end(), false),
                ] {
                    if on_a {
                        if let Some(t) = Seg::arc_param(c1, r1, s1, w1, pt, e) {
                            out.push((t, b.closest_param(pt)));
                        }
                    } else if let Some(u) = Seg::arc_param(c2, r2, s2, w2, pt, e) {
                        out.push((a.closest_param(pt), u));
                    }
                }
                return out;
            }
            for pt in circle_circle(c1, r1, c2, r2) {
                if let (Some(t), Some(u)) = (
                    Seg::arc_param(c1, r1, s1, w1, pt, e),
                    Seg::arc_param(c2, r2, s2, w2, pt, e),
                ) {
                    out.push((t, u));
                }
            }
        }
    }
    out
}

/// Where the segment `p0 p1` meets the circle: (fraction along the segment, point).
fn line_circle(p0: Point2, p1: Point2, c: Point2, r: f64) -> Vec<(f64, Point2)> {
    let d = p1 - p0;
    let len = d.length();
    if len <= f64::MIN_POSITIVE {
        return Vec::new();
    }
    let u = d * (1.0 / len);
    // Foot of the perpendicular from the centre, then half-chord by Pythagoras.
    let s = (c - p0).dot(u);
    let foot = p0 + u * s;
    let h2 = r * r - foot.distance(c).powi(2);
    let e = JOIN_TOLERANCE;
    let roots: Vec<f64> = if h2 < -2.0 * r * e {
        return Vec::new();
    } else if h2 <= 2.0 * r * e {
        vec![s]
    } else {
        let h = h2.sqrt();
        vec![s - h, s + h]
    };
    roots
        .into_iter()
        .filter(|s| (-e..=len + e).contains(s))
        .map(|s| ((s / len).clamp(0.0, 1.0), p0 + u * s))
        .collect()
}

/// Where two circles meet (none, one where they touch, or two).
fn circle_circle(c1: Point2, r1: f64, c2: Point2, r2: f64) -> Vec<Point2> {
    let d = c1.distance(c2);
    let e = JOIN_TOLERANCE;
    if d <= f64::MIN_POSITIVE || d > r1 + r2 + e || d < (r1 - r2).abs() - e {
        return Vec::new();
    }
    let u = (c2 - c1) * (1.0 / d);
    // Distance from c1 along the centre line to the chord (radical line).
    let a = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let mid = c1 + u * a;
    if h2 <= 2.0 * r1.max(r2) * e {
        return vec![mid];
    }
    let h = h2.max(0.0).sqrt();
    vec![mid + u.perp() * h, mid - u.perp() * h]
}

/// A closed loop: each segment starts where the one before ends.
pub type Loop = Vec<Seg>;

pub fn loop_length(l: &[Seg]) -> f64 {
    l.iter().map(Seg::length).sum()
}

/// Signed area: positive for a counter-clockwise loop.
pub fn signed_area(l: &[Seg]) -> f64 {
    l.iter().map(Seg::area_term).sum()
}

/// How many times the loop winds round `p` (counter-clockwise positive).
pub fn winding(l: &[Seg], p: Point2) -> i64 {
    let total: f64 = l.iter().map(|s| s.subtended(p)).sum();
    (total / TAU).round() as i64
}

pub fn reverse_loop(l: &[Seg]) -> Loop {
    l.iter().rev().map(Seg::reversed).collect()
}

/// The distance from `p` to the nearest point of any of the loops.
pub fn distance_to(loops: &[Loop], p: Point2) -> f64 {
    loops
        .iter()
        .flatten()
        .map(|s| s.distance(p))
        .fold(f64::INFINITY, f64::min)
}

/// True when every segment starts where the previous one ends and the last ends at the
/// first's start.
pub fn is_closed(l: &[Seg]) -> bool {
    let (Some(first), Some(last)) = (l.first(), l.last()) else {
        return false;
    };
    last.end().distance(first.start()) <= JOIN_TOLERANCE * 10.0
        && l.windows(2)
            .all(|w| w[0].end().distance(w[1].start()) <= JOIN_TOLERANCE * 10.0)
}

/// Merges neighbours that continue each other: collinear lines in the same direction,
/// arcs on the same circle turning the same way.
pub fn simplify(l: &[Seg]) -> Loop {
    let mut out: Loop = Vec::with_capacity(l.len());
    for s in l {
        if s.length() <= JOIN_TOLERANCE {
            continue;
        }
        if let Some(prev) = out.last_mut()
            && let Some(m) = merge(prev, s)
        {
            *prev = m;
            continue;
        }
        out.push(*s);
    }
    // The last may continue into the first.
    if out.len() > 1
        && let (Some(last), Some(first)) = (out.last(), out.first())
        && let Some(m) = merge(last, first)
    {
        out.pop();
        if let Some(f) = out.first_mut() {
            *f = m;
        }
    }
    out
}

fn merge(a: &Seg, b: &Seg) -> Option<Seg> {
    if a.end().distance(b.start()) > JOIN_TOLERANCE {
        return None;
    }
    match (*a, *b) {
        (Seg::Line { a: p0, b: p1 }, Seg::Line { a: _, b: p2 }) => {
            let (d1, d2) = ((p1 - p0).normalized()?, (p2 - p1).normalized()?);
            let off = (p2 - p0).cross(d1).abs();
            (d1.dot(d2) > 0.0 && d1.cross(d2).abs() <= 1e-9 && off <= JOIN_TOLERANCE)
                .then_some(Seg::Line { a: p0, b: p2 })
        }
        (
            Seg::Arc {
                c: c1,
                r: r1,
                start,
                sweep: w1,
            },
            Seg::Arc {
                c: c2,
                r: r2,
                sweep: w2,
                ..
            },
        ) => {
            let same = c1.distance(c2) <= JOIN_TOLERANCE
                && (r1 - r2).abs() <= JOIN_TOLERANCE
                && w1.signum() == w2.signum()
                && (w1 + w2).abs() <= TAU + 1e-12;
            same.then_some(Seg::Arc {
                c: c1,
                r: r1,
                start,
                sweep: w1 + w2,
            })
        }
        _ => None,
    }
}

/// The loop started at the point nearest `p` (the segment there is split).
pub fn start_nearest(l: &[Seg], p: Point2) -> Loop {
    let mut best: Option<(f64, usize, f64)> = None;
    for (i, s) in l.iter().enumerate() {
        let t = s.closest_param(p);
        let d = s.point_at(t).distance(p);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, i, t));
        }
    }
    let Some((_, i, t)) = best else {
        return l.to_vec();
    };
    let Some(seg) = l.get(i) else {
        return l.to_vec();
    };
    let mut out = Vec::with_capacity(l.len() + 1);
    let head_len = seg.length() * t;
    let tail_len = seg.length() * (1.0 - t);
    if tail_len > JOIN_TOLERANCE {
        out.push(seg.sub(t, 1.0));
    }
    out.extend(l.iter().skip(i + 1).copied());
    out.extend(l.iter().take(i).copied());
    if head_len > JOIN_TOLERANCE {
        out.push(seg.sub(0.0, t));
    }
    if out.is_empty() { l.to_vec() } else { out }
}

/// The nearest point of the loop to `p` and its distance.
pub fn nearest_point(l: &[Seg], p: Point2) -> Option<(Point2, f64)> {
    l.iter()
        .map(|s| {
            let q = s.point_at(s.closest_param(p));
            (q, q.distance(p))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    #[test]
    fn a_counter_clockwise_unit_circle_has_area_pi_and_winds_once_round_its_centre() {
        let c = Seg::Arc {
            c: p(2.0, 3.0),
            r: 1.0,
            start: 0.3,
            sweep: TAU,
        };
        assert!((signed_area(&[c]) - PI).abs() < 1e-12);
        assert_eq!(winding(&[c], p(2.0, 3.0)), 1);
        assert_eq!(winding(&[c], p(2.5, 3.2)), 1);
        assert_eq!(winding(&[c], p(4.0, 3.0)), 0);
        assert_eq!(winding(&[c.reversed()], p(2.0, 3.0)), -1);
    }

    #[test]
    fn a_line_crosses_a_circle_where_pythagoras_says() {
        // The x axis meets the circle of radius 5 about (0, 3) at x = ±4 (3-4-5 triangle).
        let l = Seg::line(p(-10.0, 0.0), p(10.0, 0.0));
        let c = Seg::Arc {
            c: p(0.0, 3.0),
            r: 5.0,
            start: 0.0,
            sweep: TAU,
        };
        let mut xs: Vec<f64> = intersect(&l, &c)
            .into_iter()
            .map(|(t, _)| l.point_at(t).x)
            .collect();
        xs.sort_by(f64::total_cmp);
        assert_eq!(xs.len(), 2);
        assert!((xs[0] + 4.0).abs() < 1e-9 && (xs[1] - 4.0).abs() < 1e-9);
    }

    #[test]
    fn an_arc_with_a_hole_in_its_sweep_is_not_hit_there() {
        // Upper half circle only: the line y = -3 meets the full circle but not this arc.
        let half = Seg::Arc {
            c: p(0.0, 0.0),
            r: 5.0,
            start: 0.0,
            sweep: PI,
        };
        let l = Seg::line(p(-10.0, -3.0), p(10.0, -3.0));
        assert!(intersect(&l, &half).is_empty());
        let m = Seg::line(p(-10.0, 3.0), p(10.0, 3.0));
        assert_eq!(intersect(&m, &half).len(), 2);
    }

    #[test]
    fn starting_a_square_at_a_point_splits_the_side_there() {
        let sq = vec![
            Seg::line(p(0.0, 0.0), p(10.0, 0.0)),
            Seg::line(p(10.0, 0.0), p(10.0, 10.0)),
            Seg::line(p(10.0, 10.0), p(0.0, 10.0)),
            Seg::line(p(0.0, 10.0), p(0.0, 0.0)),
        ];
        let r = start_nearest(&sq, p(4.0, -1.0));
        assert_eq!(r.len(), 5);
        assert!(r[0].start().distance(p(4.0, 0.0)) < 1e-12);
        assert!(is_closed(&r));
        assert!((loop_length(&r) - 40.0).abs() < 1e-12);
        assert_eq!(simplify(&r).len(), 4);
    }
}

//! Object snaps and direction locks for drawing: where a click near existing geometry
//! should land, and which constraint keeps it there.
//!
//! [`Sketch::snap`] looks at the geometry around a cursor position (sketch coordinates,
//! mm) and returns the best [`Snap`]: an existing point (an endpoint or a centre), the
//! sketch origin, the midpoint of a line, a quadrant of a circle or arc, the crossing of
//! two curves, the foot of the perpendicular or a tangent point from the previous point,
//! or the nearest point on a curve. Points beat midpoints, crossings and quadrants, which
//! beat perpendicular and tangent points, which beat a plain on-curve snap; within a rank
//! the nearest wins. [`Sketch::snap_constraints`] turns a snap into the constraints that
//! keep the new point there (coincident, fix at the origin, midpoint, point on curve,
//! perpendicular, tangent).
//!
//! [`lock_straight`] and [`lock_angle`] are the Shift locks: the end of a segment held
//! horizontal or vertical (whichever is nearer), or on a multiple of an angle step.
//!
//! All of it is closed-form plane geometry (projection onto a line, the right angle at a
//! tangent point, the crossings in `intersect.rs`), so it is exact up to rounding.

use crate::constraint::Constraint;
use crate::entity::{Curve, CurveId, PointId};
use crate::intersect::{self, Shape};
use crate::sketch::Sketch;
use chamfera_geom::{LINEAR_TOLERANCE, Point2, Vec2};
use std::collections::BTreeSet;
use std::f64::consts::FRAC_PI_2;

/// At most this many curves near the cursor are intersected pairwise.
pub const MAX_SNAP_CURVES: usize = 32;

/// What a snap lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapKind {
    /// An existing point that is not a centre: a line or arc end, or a lone point.
    Endpoint,
    /// The centre point of a circle or arc.
    Centre,
    /// The sketch origin (0, 0), where no point is yet.
    Origin,
    /// The middle of a line.
    Midpoint,
    /// Where two curves cross or touch.
    Intersection,
    /// The point of a circle or arc at 0, 90, 180 or 270 degrees from its centre.
    Quadrant,
    /// The foot of the perpendicular from the previous point onto a line.
    Perpendicular,
    /// Where a line from the previous point touches a circle or arc.
    Tangent,
    /// The nearest point on a curve.
    OnCurve,
}

impl SnapKind {
    /// The word shown beside the snap marker.
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "Endpoint",
            SnapKind::Centre => "Centre",
            SnapKind::Origin => "Origin",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::Intersection => "Intersection",
            SnapKind::Quadrant => "Quadrant",
            SnapKind::Perpendicular => "Perpendicular",
            SnapKind::Tangent => "Tangent",
            SnapKind::OnCurve => "On curve",
        }
    }

    /// Lower ranks win over higher ones, whatever the distance.
    fn rank(self) -> u8 {
        match self {
            SnapKind::Endpoint | SnapKind::Centre | SnapKind::Origin => 0,
            SnapKind::Midpoint | SnapKind::Intersection | SnapKind::Quadrant => 1,
            SnapKind::Perpendicular | SnapKind::Tangent => 2,
            SnapKind::OnCurve => 3,
        }
    }
}

/// A place a click snaps to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub kind: SnapKind,
    /// Where the click lands (mm).
    pub at: Point2,
    /// The existing point, for [`SnapKind::Endpoint`] and [`SnapKind::Centre`].
    pub point: Option<PointId>,
    /// The curve snapped to (for the curve kinds).
    pub curve: Option<CurveId>,
    /// The second curve of an [`SnapKind::Intersection`].
    pub other: Option<CurveId>,
}

/// Where to look for a snap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapQuery {
    /// The cursor on the sketch plane (mm).
    pub cursor: Point2,
    /// How far a point-like snap (points, origin, midpoints, crossings, quadrants,
    /// perpendicular and tangent points) may be from the cursor (mm).
    pub radius: f64,
    /// How far the cursor may be from a curve for an on-curve snap (mm).
    pub curve_radius: f64,
    /// The previous point of the segment being drawn, for perpendicular and tangent snaps.
    pub from: Option<Point2>,
}

/// The middle of the segment from `a` to `b`.
pub fn midpoint(a: Point2, b: Point2) -> Point2 {
    a.lerp(b, 0.5)
}

/// The foot of the perpendicular from `p` onto the infinite line through `a` and `b`, with
/// its parameter along the line (0 at `a`, 1 at `b`). `None` when `a` and `b` coincide.
pub fn perpendicular_foot(p: Point2, a: Point2, b: Point2) -> Option<(Point2, f64)> {
    let d = b - a;
    let len2 = d.dot(d);
    if !(len2.is_finite() && len2 > LINEAR_TOLERANCE * LINEAR_TOLERANCE) {
        return None;
    }
    let t = (p - a).dot(d) / len2;
    let foot = a.lerp(b, t);
    foot.x.is_finite().then_some((foot, t))
}

/// The points where lines from `p` touch the circle of centre `c` and radius `r`: none when
/// `p` is inside or on the circle, else two.
///
/// At a tangent point `t` the radius is at right angles to the line, so the triangle
/// `c, t, p` has its right angle at `t` and the angle at `c` is `acos(r / |p - c|)`.
pub fn tangent_points(p: Point2, c: Point2, r: f64) -> Vec<Point2> {
    let v = p - c;
    let d = v.length();
    if !(d.is_finite() && r.is_finite() && r > 0.0) || d <= r + LINEAR_TOLERANCE {
        return Vec::new();
    }
    let base = v.angle();
    let alpha = (r / d).acos();
    [base + alpha, base - alpha]
        .into_iter()
        .map(|a| c + Vec2::new(a.cos(), a.sin()) * r)
        .collect()
}

/// The points of a circle at 0, 90, 180 and 270 degrees from its centre.
pub fn quadrant_points(c: Point2, r: f64) -> [Point2; 4] {
    [
        Point2::new(c.x + r, c.y),
        Point2::new(c.x, c.y + r),
        Point2::new(c.x - r, c.y),
        Point2::new(c.x, c.y - r),
    ]
}

/// Shift lock: `to` moved so the segment from `from` is exactly horizontal or vertical,
/// whichever is nearer (horizontal on a tie).
pub fn lock_straight(from: Point2, to: Point2) -> Point2 {
    let d = to - from;
    if d.x.abs() >= d.y.abs() {
        Point2::new(to.x, from.y)
    } else {
        Point2::new(from.x, to.y)
    }
}

/// Angle lock: `to` moved onto the nearest direction from `from` that is a whole multiple
/// of `step` radians from +X, at the length of `to - from` projected onto it. Multiples of
/// 90 degrees come out exactly horizontal or vertical. `None` for a step that is not a
/// positive finite angle or when `to` is on `from`.
pub fn lock_angle(from: Point2, to: Point2, step: f64) -> Option<Point2> {
    if !(step.is_finite() && step > 1e-6) {
        return None;
    }
    let d = to - from;
    if d.length() <= LINEAR_TOLERANCE || !d.is_finite() {
        return None;
    }
    let angle = (d.angle() / step).round() * step;
    let dir = exact_direction(angle);
    let len = d.dot(dir).max(0.0);
    Some(from + dir * len)
}

/// The unit vector at `angle`, exact along the axes.
pub fn exact_direction(angle: f64) -> Vec2 {
    let quarter = angle / FRAC_PI_2;
    let k = quarter.round();
    if (quarter - k).abs() < 1e-9 {
        match (k as i64).rem_euclid(4) {
            0 => Vec2::X,
            1 => Vec2::Y,
            2 => Vec2::new(-1.0, 0.0),
            _ => Vec2::new(0.0, -1.0),
        }
    } else {
        Vec2::new(angle.cos(), angle.sin())
    }
}

/// The point of `shape` nearest `p`: the clamped foot on a segment, the radial point on a
/// circle, and on an arc the radial point inside its sweep or else the nearer end.
fn nearest_on(shape: &Shape, p: Point2) -> Option<Point2> {
    match *shape {
        Shape::Segment { a, b } => {
            let (_, t) = perpendicular_foot(p, a, b)?;
            Some(a.lerp(b, t.clamp(0.0, 1.0)))
        }
        Shape::Circle { c, r } => {
            let u = (p - c).normalized()?;
            Some(c + u * r)
        }
        Shape::Arc { sweep, .. } => {
            let t = shape.param(p);
            if shape.on(t) {
                Some(shape.point_at(t))
            } else {
                let s = shape.point_at(0.0);
                let e = shape.point_at(sweep);
                Some(if s.distance(p) <= e.distance(p) { s } else { e })
            }
        }
    }
}

impl Sketch {
    /// The best snap for `q`, or `None` when nothing is near enough (or the query is not
    /// finite).
    pub fn snap(&self, q: &SnapQuery) -> Option<Snap> {
        self.snap_candidates(q).into_iter().min_by(|a, b| {
            let da = a.at.distance(q.cursor);
            let db = b.at.distance(q.cursor);
            a.kind.rank().cmp(&b.kind.rank()).then(da.total_cmp(&db))
        })
    }

    /// Every snap within reach of `q`, in no particular order.
    pub fn snap_candidates(&self, q: &SnapQuery) -> Vec<Snap> {
        let cursor = q.cursor;
        let finite = |v: f64| v.is_finite() && v >= 0.0;
        if !(cursor.is_valid() && finite(q.radius) && finite(q.curve_radius)) {
            return Vec::new();
        }
        let from = q.from.filter(|p| p.is_valid());
        let mut out = Vec::new();
        let near = |p: Point2| p.distance(cursor) <= q.radius;
        let mut add = |kind, at: Point2, point, curve, other| {
            if at.is_valid() {
                out.push(Snap {
                    kind,
                    at,
                    point,
                    curve,
                    other,
                });
            }
        };

        let centres: BTreeSet<PointId> = self
            .curves()
            .filter_map(|(_, c)| match c {
                Curve::Circle { centre, .. }
                | Curve::Arc { centre, .. }
                | Curve::Ellipse { centre, .. }
                | Curve::EllipticalArc { centre, .. } => Some(*centre),
                Curve::Line { .. } => None,
            })
            .collect();
        let mut origin_taken = false;
        for (id, p) in self.points() {
            origin_taken |= p.coincident(Point2::ORIGIN);
            if near(p) {
                let kind = if centres.contains(&id) {
                    SnapKind::Centre
                } else {
                    SnapKind::Endpoint
                };
                add(kind, p, Some(id), None, None);
            }
        }
        if !origin_taken && near(Point2::ORIGIN) {
            add(SnapKind::Origin, Point2::ORIGIN, None, None, None);
        }

        // Curves near the cursor, for the crossing snaps.
        let mut close: Vec<(CurveId, Shape)> = Vec::new();
        for (id, curve) in self.curves() {
            let Some(shape) = self.shape_of(curve) else {
                continue;
            };
            let Some(nearest) = nearest_on(&shape, cursor) else {
                continue;
            };
            let gap = nearest.distance(cursor);
            if gap <= q.curve_radius {
                add(SnapKind::OnCurve, nearest, None, Some(id), None);
            }
            if gap <= q.radius && close.len() < MAX_SNAP_CURVES {
                close.push((id, shape));
            }
            match shape {
                Shape::Segment { a, b } => {
                    let m = midpoint(a, b);
                    if near(m) {
                        add(SnapKind::Midpoint, m, None, Some(id), None);
                    }
                    if let Some(f) = from
                        && let Some((foot, t)) = perpendicular_foot(f, a, b)
                        && shape.on(t)
                        && !foot.coincident(f)
                        && near(foot)
                    {
                        add(SnapKind::Perpendicular, foot, None, Some(id), None);
                    }
                }
                Shape::Circle { c, r } | Shape::Arc { c, r, .. } => {
                    for p in quadrant_points(c, r) {
                        if near(p) && shape.on(shape.param(p)) {
                            add(SnapKind::Quadrant, p, None, Some(id), None);
                        }
                    }
                    if let Some(f) = from {
                        for p in tangent_points(f, c, r) {
                            if near(p) && shape.on(shape.param(p)) {
                                add(SnapKind::Tangent, p, None, Some(id), None);
                            }
                        }
                    }
                }
            }
        }
        for (i, (a, sa)) in close.iter().enumerate() {
            for (b, sb) in close.iter().skip(i + 1) {
                for p in intersect::intersections(sa, sb) {
                    if near(p) {
                        add(SnapKind::Intersection, p, None, Some(*a), Some(*b));
                    }
                }
            }
        }
        out
    }

    /// The constraints that keep `point` where `snap` put it. `line` is the new line that
    /// ends at `point`, if any: perpendicular and tangent snaps constrain it too. Unknown
    /// ids give no constraint.
    pub fn snap_constraints(
        &self,
        snap: &Snap,
        point: PointId,
        line: Option<CurveId>,
    ) -> Vec<Constraint> {
        let on = |curve: Option<CurveId>| -> Option<Constraint> {
            let curve = curve?;
            match self.curve(curve)? {
                Curve::Line { .. } => Some(Constraint::PointOnLine { point, line: curve }),
                Curve::Circle { .. } | Curve::Arc { .. } => {
                    Some(Constraint::PointOnCircle { point, curve })
                }
                // No point-on-ellipse constraint yet: the snap only places the point.
                Curve::Ellipse { .. } | Curve::EllipticalArc { .. } => None,
            }
        };
        let line = line.filter(|l| self.curve(*l).is_some_and(Curve::is_line));
        let mut out = Vec::new();
        match snap.kind {
            SnapKind::Endpoint | SnapKind::Centre => {
                if let Some(p) = snap.point
                    && p != point
                    && self.point(p).is_some()
                {
                    out.push(Constraint::Coincident { a: p, b: point });
                }
            }
            SnapKind::Origin => out.push(Constraint::Fix {
                point,
                at: Point2::ORIGIN,
            }),
            SnapKind::Midpoint => {
                if let Some(c) = snap.curve
                    && self.curve(c).is_some_and(Curve::is_line)
                {
                    out.push(Constraint::Midpoint { point, line: c });
                }
            }
            SnapKind::OnCurve | SnapKind::Quadrant => out.extend(on(snap.curve)),
            SnapKind::Intersection => {
                out.extend(on(snap.curve));
                out.extend(on(snap.other));
            }
            SnapKind::Perpendicular => {
                out.extend(on(snap.curve));
                if let (Some(l), Some(c)) = (line, snap.curve)
                    && l != c
                    && self.curve(c).is_some_and(Curve::is_line)
                {
                    out.push(Constraint::Perpendicular { a: l, b: c });
                }
            }
            SnapKind::Tangent => {
                out.extend(on(snap.curve));
                if let (Some(l), Some(c)) = (line, snap.curve)
                    && self.curve(c).is_some_and(Curve::is_circular)
                {
                    out.push(Constraint::Tangent { a: l, b: c });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn close(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-12
    }

    fn query(x: f64, y: f64) -> SnapQuery {
        SnapQuery {
            cursor: p(x, y),
            radius: 1.0,
            curve_radius: 0.8,
            from: None,
        }
    }

    /// A sketch with one line from `a` to `b`.
    fn with_line(a: Point2, b: Point2) -> (Sketch, CurveId) {
        let mut s = Sketch::new();
        let pa = s.add_point(a).unwrap();
        let pb = s.add_point(b).unwrap();
        let l = s.add_line(pa, pb).unwrap();
        (s, l)
    }

    #[test]
    fn the_midpoint_of_0_0_to_10_4_is_5_2() {
        assert!(close(midpoint(p(0.0, 0.0), p(10.0, 4.0)), p(5.0, 2.0)));
        let (s, l) = with_line(p(0.0, 0.0), p(10.0, 4.0));
        let snap = s.snap(&query(5.3, 2.4)).unwrap();
        assert_eq!(snap.kind, SnapKind::Midpoint);
        assert_eq!(snap.curve, Some(l));
        assert!(close(snap.at, p(5.0, 2.0)));
    }

    #[test]
    fn a_perpendicular_foot_matches_the_hand_projection() {
        // From (2, 6) onto the line through (0, 0) and (10, 4): t = (2*10 + 6*4) / 116
        // = 44 / 116 = 11 / 29, so the foot is (110 / 29, 44 / 29).
        let (foot, t) = perpendicular_foot(p(2.0, 6.0), p(0.0, 0.0), p(10.0, 4.0)).unwrap();
        assert!((t - 11.0 / 29.0).abs() < 1e-15);
        assert!(close(foot, p(110.0 / 29.0, 44.0 / 29.0)));
        // The segment from the point to the foot is at right angles to the line.
        assert!((foot - p(2.0, 6.0)).dot(Vec2::new(10.0, 4.0)).abs() < 1e-12);
        assert!(perpendicular_foot(p(1.0, 1.0), p(3.0, 3.0), p(3.0, 3.0)).is_none());
        // As a snap from the previous point (2, 6), near the foot (3.793, 1.517).
        let (s, l) = with_line(p(0.0, 0.0), p(10.0, 4.0));
        let q = SnapQuery {
            from: Some(p(2.0, 6.0)),
            ..query(3.9, 1.7)
        };
        let snap = s.snap(&q).unwrap();
        assert_eq!(snap.kind, SnapKind::Perpendicular);
        assert!(close(snap.at, p(110.0 / 29.0, 44.0 / 29.0)));
        let new_end = PointId(99);
        let cs = s.snap_constraints(&snap, new_end, Some(CurveId(98)));
        // The new line is unknown to the sketch, so only the point-on is made.
        assert_eq!(
            cs,
            vec![Constraint::PointOnLine {
                point: new_end,
                line: l
            }]
        );
    }

    #[test]
    fn tangent_points_from_10_0_to_a_circle_of_radius_5_at_the_origin() {
        // cos(angle at the centre) = r / d = 5 / 10, so 60 degrees: (2.5, +-5 sin 60).
        let t = tangent_points(p(10.0, 0.0), p(0.0, 0.0), 5.0);
        let h = 5.0 * (PI / 3.0).sin();
        assert_eq!(t.len(), 2);
        assert!(close(t[0], p(2.5, h)) && close(t[1], p(2.5, -h)), "{t:?}");
        // The radius and the line from (10, 0) meet at right angles there.
        for q in &t {
            assert!((*q - p(0.0, 0.0)).dot(*q - p(10.0, 0.0)).abs() < 1e-12);
        }
        assert!(tangent_points(p(1.0, 0.0), p(0.0, 0.0), 5.0).is_empty());
        assert!(tangent_points(p(5.0, 0.0), p(0.0, 0.0), 5.0).is_empty());
    }

    #[test]
    fn endpoints_beat_midpoints_and_curves_and_the_origin_is_a_snap() {
        let (s, _) = with_line(p(10.0, 0.0), p(20.0, 0.0));
        let snap = s.snap(&query(19.6, 0.3)).unwrap();
        assert_eq!(snap.kind, SnapKind::Endpoint);
        assert!(close(snap.at, p(20.0, 0.0)));
        assert!(snap.point.is_some());
        // Away from the ends and the middle: on the curve, at the foot (13, 0).
        let snap = s.snap(&query(13.0, 0.5)).unwrap();
        assert_eq!(snap.kind, SnapKind::OnCurve);
        assert!(close(snap.at, p(13.0, 0.0)));
        let snap = s.snap(&query(0.4, -0.3)).unwrap();
        assert_eq!(snap.kind, SnapKind::Origin);
        assert!(close(snap.at, Point2::ORIGIN));
        assert_eq!(
            s.snap_constraints(&snap, PointId(1), None),
            vec![Constraint::Fix {
                point: PointId(1),
                at: Point2::ORIGIN
            }]
        );
        // Nothing near: no snap.
        assert!(s.snap(&query(15.0, 5.0)).is_none());
    }

    #[test]
    fn circles_give_their_centre_and_quadrants() {
        let mut s = Sketch::new();
        let c = s.add_point(p(30.0, 10.0)).unwrap();
        let circle = s.add_circle(c, 4.0).unwrap();
        let snap = s.snap(&query(30.2, 9.9)).unwrap();
        assert_eq!((snap.kind, snap.point), (SnapKind::Centre, Some(c)));
        // The top quadrant is (30, 14).
        let snap = s.snap(&query(30.3, 14.2)).unwrap();
        assert_eq!(snap.kind, SnapKind::Quadrant);
        assert!(close(snap.at, p(30.0, 14.0)));
        assert_eq!(
            s.snap_constraints(&snap, PointId(9), None),
            vec![Constraint::PointOnCircle {
                point: PointId(9),
                curve: circle
            }]
        );
    }

    #[test]
    fn two_crossing_lines_snap_to_their_intersection() {
        // y = x and y = 8 - x / 2 cross where x = 8 - x / 2: (16/3, 16/3). Their
        // midpoints (5, 5) and (6, 5) are near too, but further from the cursor.
        let mut s = Sketch::new();
        let ids: Vec<PointId> = [p(0.0, 0.0), p(10.0, 10.0), p(0.0, 8.0), p(12.0, 2.0)]
            .into_iter()
            .map(|q| s.add_point(q).unwrap())
            .collect();
        let a = s.add_line(ids[0], ids[1]).unwrap();
        let b = s.add_line(ids[2], ids[3]).unwrap();
        let snap = s.snap(&query(5.6, 5.5)).unwrap();
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert!(close(snap.at, p(16.0 / 3.0, 16.0 / 3.0)), "{snap:?}");
        let cs = s.snap_constraints(&snap, PointId(50), None);
        assert_eq!(cs.len(), 2);
        assert!(cs.contains(&Constraint::PointOnLine {
            point: PointId(50),
            line: a
        }));
        assert!(cs.contains(&Constraint::PointOnLine {
            point: PointId(50),
            line: b
        }));
    }

    #[test]
    fn tangent_snap_from_the_previous_point_and_its_constraints() {
        let mut s = Sketch::new();
        let c = s.add_point(p(0.0, 0.0)).unwrap();
        let circle = s.add_circle(c, 5.0).unwrap();
        let start = s.add_point(p(10.0, 0.0)).unwrap();
        let end = s.add_point(p(2.5, 4.0)).unwrap();
        let line = s.add_line(start, end).unwrap();
        let h = 5.0 * (PI / 3.0).sin();
        let q = SnapQuery {
            from: Some(p(10.0, 0.0)),
            ..query(2.7, h + 0.4)
        };
        let snaps = s.snap_candidates(&q);
        let tangent = snaps
            .iter()
            .find(|sn| sn.kind == SnapKind::Tangent)
            .copied()
            .unwrap();
        assert!(close(tangent.at, p(2.5, h)));
        assert_eq!(
            s.snap_constraints(&tangent, end, Some(line)),
            vec![
                Constraint::PointOnCircle {
                    point: end,
                    curve: circle
                },
                Constraint::Tangent { a: line, b: circle },
            ]
        );
    }

    #[test]
    fn arcs_only_snap_inside_their_sweep() {
        // A quarter arc from (5, 0) to (0, 5) round the origin: its quadrants are its ends
        // only, so (-5, 0) is not a snap.
        let mut s = Sketch::new();
        let c = s.add_point(p(0.0, 0.0)).unwrap();
        let a = s.add_point(p(5.0, 0.0)).unwrap();
        let b = s.add_point(p(0.0, 5.0)).unwrap();
        s.add_arc(c, a, b).unwrap();
        assert!(s.snap(&query(-5.2, 0.1)).is_none());
        // Its point at 45 degrees is on it.
        let r = 5.0 * std::f64::consts::FRAC_1_SQRT_2;
        let snap = s.snap(&query(r + 0.3, r + 0.3)).unwrap();
        assert_eq!(snap.kind, SnapKind::OnCurve);
        assert!(close(snap.at, p(r, r)));
    }

    #[test]
    fn shift_locks_make_exact_horizontal_vertical_and_stepped_directions() {
        let from = p(3.0, 4.0);
        assert_eq!(lock_straight(from, p(20.0, 6.0)), p(20.0, 4.0));
        assert_eq!(lock_straight(from, p(5.0, -9.0)), p(3.0, -9.0));
        // 15 degree steps: (3,4) + (10, 2.5) is at 14.04 degrees, so 15 degrees, at the
        // projected length 10 cos 15 + 2.5 sin 15.
        let step = 15f64.to_radians();
        let got = lock_angle(from, p(13.0, 6.5), step).unwrap();
        let a = step;
        let len = 10.0 * a.cos() + 2.5 * a.sin();
        assert!(
            close(got, p(3.0 + len * a.cos(), 4.0 + len * a.sin())),
            "{got:?}"
        );
        // Near 180 degrees the result is exactly level.
        let got = lock_angle(from, p(-7.0, 4.3), step).unwrap();
        assert_eq!(got.y, 4.0);
        assert!(lock_angle(from, from, step).is_none());
        assert!(lock_angle(from, p(1.0, 1.0), f64::NAN).is_none());
    }

    #[test]
    fn hostile_queries_give_no_snap() {
        let (s, _) = with_line(p(0.0, 0.0), p(10.0, 4.0));
        for q in [
            query(f64::NAN, 0.0),
            query(f64::INFINITY, 1.0),
            SnapQuery {
                radius: f64::NAN,
                ..query(5.0, 2.0)
            },
            SnapQuery {
                radius: -1.0,
                ..query(5.0, 2.0)
            },
        ] {
            assert!(s.snap(&q).is_none());
        }
        let q = SnapQuery {
            from: Some(p(f64::NAN, 1.0)),
            ..query(5.0, 2.0)
        };
        assert_eq!(s.snap(&q).map(|sn| sn.kind), Some(SnapKind::Midpoint));
    }
}

//! Tracking while drawing: guides that pull the cursor onto a line through existing
//! geometry when no object snap is near.
//!
//! [`Sketch::track`] finds, for a cursor (sketch mm):
//! - alignment guides: the vertical and horizontal lines through every sketch point (and
//!   the origin), when the cursor is within the radius of one;
//! - direction guides from the previous point of a line: at right angles to a line that
//!   ends there, along the tangent of an arc that ends there (both kept by a constraint,
//!   see [`Sketch::track_constraints`]), and polar guides at whole multiples of an angle
//!   step.
//!
//! A direction guide crossed by an alignment guide near the cursor lands on the crossing;
//! two alignment guides land on their corner. Everything is the projection of the cursor
//! onto a line, or the crossing of two lines, so it is exact up to rounding.

use crate::constraint::Constraint;
use crate::entity::{Curve, CurveId, PointId};
use crate::sketch::Sketch;
use crate::snap::exact_direction;
use chamfera_geom::{LINEAR_TOLERANCE, Point2, Vec2};

/// At most this many points are looked at for alignment guides.
pub const MAX_TRACK_POINTS: usize = 4096;

/// What a guide follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuideKind {
    /// The vertical line through a point: the cursor shares its x.
    AlignVertical,
    /// The horizontal line through a point: the cursor shares its y.
    AlignHorizontal,
    /// At right angles to the line that ends at the previous point.
    Perpendicular,
    /// Along the tangent of the arc that ends at the previous point.
    Tangent,
    /// A whole multiple of the polar angle step from the previous point.
    Polar,
}

impl GuideKind {
    /// The word shown beside the guide.
    pub fn label(self) -> &'static str {
        match self {
            GuideKind::AlignVertical | GuideKind::AlignHorizontal => "Aligned",
            GuideKind::Perpendicular => "Perpendicular",
            GuideKind::Tangent => "Tangent",
            GuideKind::Polar => "Polar",
        }
    }

    /// Direction guides start at the previous point; alignment guides at another point.
    pub fn is_direction(self) -> bool {
        matches!(
            self,
            GuideKind::Perpendicular | GuideKind::Tangent | GuideKind::Polar
        )
    }
}

/// One guide line: from `origin` (the point it goes through) along `dir` (a unit vector).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub kind: GuideKind,
    pub origin: Point2,
    pub dir: Vec2,
    /// The curve a perpendicular or tangent guide comes from.
    pub curve: Option<CurveId>,
}

/// Where tracking puts the cursor, and the one or two guides that put it there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub at: Point2,
    pub guides: [Option<Guide>; 2],
}

impl Track {
    /// The direction guide, if the track has one.
    pub fn direction(&self) -> Option<Guide> {
        self.guides
            .iter()
            .flatten()
            .copied()
            .find(|g| g.kind.is_direction())
    }
}

/// Where to look for guides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackQuery {
    /// The cursor on the sketch plane (mm).
    pub cursor: Point2,
    /// How far the cursor may be from a guide (mm).
    pub radius: f64,
    /// The previous point of the line being drawn.
    pub from: Option<Point2>,
    /// The existing sketch point at `from`, for perpendicular and tangent guides.
    pub from_point: Option<PointId>,
    /// Alignment guides through the sketch's points.
    pub align: bool,
    /// Perpendicular and tangent guides from the curves ending at `from_point`.
    pub infer: bool,
    /// The polar angle step (radians), or `None` for no polar guides.
    pub polar_step: Option<f64>,
}

/// The cursor projected onto the guide through `origin` along the unit `dir`, if it is
/// ahead of `origin` and within `radius` of the guide: `(foot, distance)`.
fn onto_ray(cursor: Point2, origin: Point2, dir: Vec2, radius: f64) -> Option<(Point2, f64)> {
    let d = cursor - origin;
    let t = d.dot(dir);
    if !(t.is_finite() && t > LINEAR_TOLERANCE) {
        return None;
    }
    let foot = origin + dir * t;
    let gap = foot.distance(cursor);
    (gap <= radius).then_some((foot, gap))
}

/// Where the guide through `o` along `u` crosses the guide through `p` along `v`.
pub fn guide_crossing(o: Point2, u: Vec2, p: Point2, v: Vec2) -> Option<Point2> {
    let den = u.cross(v);
    if !(den.is_finite() && den.abs() > 1e-9) {
        return None;
    }
    let t = (p - o).cross(v) / den;
    let x = o + u * t;
    x.is_valid().then_some(x)
}

impl Sketch {
    /// The direction guides from `q.from`, each with its distance from the cursor.
    fn direction_guides(&self, q: &TrackQuery, from: Point2) -> Vec<(Guide, Point2, f64)> {
        let mut out = Vec::new();
        let mut push = |kind, dir: Vec2, curve| {
            if let Some((at, gap)) = onto_ray(q.cursor, from, dir, q.radius) {
                out.push((
                    Guide {
                        kind,
                        origin: from,
                        dir,
                        curve,
                    },
                    at,
                    gap,
                ));
            }
        };
        if q.infer
            && let Some(start) = q.from_point
        {
            for (id, curve) in self.curves() {
                match *curve {
                    Curve::Line {
                        start: a, end: b, ..
                    } if a == start || b == start => {
                        let other = if a == start { b } else { a };
                        let Some(u) = self.point(other).and_then(|o| (o - from).normalized())
                        else {
                            continue;
                        };
                        let n = u.perp();
                        push(GuideKind::Perpendicular, n, Some(id));
                        push(GuideKind::Perpendicular, n * -1.0, Some(id));
                    }
                    Curve::Arc {
                        centre,
                        start: a,
                        end: b,
                        ..
                    } if a == start || b == start => {
                        // The tangent at an arc's end is at right angles to its radius.
                        let Some(r) = self.point(centre).and_then(|c| (from - c).normalized())
                        else {
                            continue;
                        };
                        let t = r.perp();
                        push(GuideKind::Tangent, t, Some(id));
                        push(GuideKind::Tangent, t * -1.0, Some(id));
                    }
                    _ => {}
                }
            }
        }
        if let Some(step) = q.polar_step.filter(|s| s.is_finite() && *s > 1e-6)
            && let Some(d) = (q.cursor - from).normalized()
        {
            let angle = (d.angle() / step).round() * step;
            push(GuideKind::Polar, exact_direction(angle), None);
        }
        out
    }

    /// The best vertical and horizontal alignment guides near the cursor.
    fn alignment_guides(&self, q: &TrackQuery) -> [Option<(Guide, f64)>; 2] {
        let mut best: [Option<(Guide, f64)>; 2] = [None, None];
        let mut consider = |p: Point2| {
            let candidates = [
                (GuideKind::AlignVertical, (q.cursor.x - p.x).abs(), Vec2::Y),
                (
                    GuideKind::AlignHorizontal,
                    (q.cursor.y - p.y).abs(),
                    Vec2::X,
                ),
            ];
            for (slot, (kind, gap, dir)) in best.iter_mut().zip(candidates) {
                // A guide needs some length to be seen: not through the cursor's own spot.
                if gap <= q.radius
                    && p.distance(q.cursor) > q.radius
                    && slot.is_none_or(|(_, g)| gap < g)
                {
                    *slot = Some((
                        Guide {
                            kind,
                            origin: p,
                            dir,
                            curve: None,
                        },
                        gap,
                    ));
                }
            }
        };
        consider(Point2::ORIGIN);
        for (_, p) in self.points().take(MAX_TRACK_POINTS) {
            consider(p);
        }
        best
    }

    /// The tracked cursor for `q`, or `None` when no guide is near (or the query is not
    /// finite).
    pub fn track(&self, q: &TrackQuery) -> Option<Track> {
        if !(q.cursor.is_valid() && q.radius.is_finite() && q.radius >= 0.0) {
            return None;
        }
        let from = q.from.filter(|p| p.is_valid());
        // Perpendicular and tangent guides beat polar ones; then the nearer wins.
        let direction = from.and_then(|f| {
            self.direction_guides(q, f).into_iter().min_by(|a, b| {
                let rank = |g: &Guide| u8::from(g.kind == GuideKind::Polar);
                rank(&a.0).cmp(&rank(&b.0)).then(a.2.total_cmp(&b.2))
            })
        });
        let [vertical, horizontal] = if q.align {
            self.alignment_guides(q)
        } else {
            [None, None]
        };
        if let Some((ray, at, _)) = direction {
            // An alignment guide crossing the direction guide near the cursor.
            let crossing = [vertical, horizontal]
                .into_iter()
                .flatten()
                .filter_map(|(g, _)| {
                    let x = guide_crossing(ray.origin, ray.dir, g.origin, g.dir)?;
                    let ahead = (x - ray.origin).dot(ray.dir) > LINEAR_TOLERANCE;
                    (ahead && x.distance(q.cursor) <= q.radius).then_some((g, x))
                })
                .min_by(|a, b| a.1.distance(q.cursor).total_cmp(&b.1.distance(q.cursor)));
            return Some(match crossing {
                Some((g, x)) => Track {
                    at: x,
                    guides: [Some(ray), Some(g)],
                },
                None => Track {
                    at,
                    guides: [Some(ray), None],
                },
            });
        }
        match (vertical, horizontal) {
            (Some((v, _)), Some((h, _))) => Some(Track {
                at: Point2::new(v.origin.x, h.origin.y),
                guides: [Some(v), Some(h)],
            }),
            (Some((v, _)), None) => Some(Track {
                at: Point2::new(v.origin.x, q.cursor.y),
                guides: [Some(v), None],
            }),
            (None, Some((h, _))) => Some(Track {
                at: Point2::new(q.cursor.x, h.origin.y),
                guides: [Some(h), None],
            }),
            (None, None) => None,
        }
    }

    /// The constraints that keep a new `line` along the direction guide of `track`: at
    /// right angles to the line it came from, or tangent to the arc. Alignment and polar
    /// guides only place the point. Unknown ids give no constraint.
    pub fn track_constraints(&self, track: &Track, line: CurveId) -> Vec<Constraint> {
        if !self.curve(line).is_some_and(Curve::is_line) {
            return Vec::new();
        }
        let Some(g) = track.direction() else {
            return Vec::new();
        };
        let Some(c) = g.curve.filter(|c| *c != line) else {
            return Vec::new();
        };
        match (g.kind, self.curve(c)) {
            (GuideKind::Perpendicular, Some(Curve::Line { .. })) => {
                vec![Constraint::Perpendicular { a: line, b: c }]
            }
            (GuideKind::Tangent, Some(Curve::Arc { .. })) => {
                vec![Constraint::Tangent { a: line, b: c }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn close(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-9
    }

    fn query(x: f64, y: f64) -> TrackQuery {
        TrackQuery {
            cursor: p(x, y),
            radius: 1.0,
            from: None,
            from_point: None,
            align: true,
            infer: true,
            polar_step: None,
        }
    }

    #[test]
    fn the_cursor_lines_up_with_a_point_and_with_two_points_at_their_corner() {
        let mut s = Sketch::new();
        s.add_point(p(20.0, 30.0)).unwrap();
        s.add_point(p(50.0, 10.0)).unwrap();
        // x within 0.6 of 20: onto the vertical through (20, 30), keeping y = 5.
        let t = s.track(&query(20.6, 5.0)).unwrap();
        assert_eq!(t.guides[0].map(|g| g.kind), Some(GuideKind::AlignVertical));
        assert!(close(t.at, p(20.0, 5.0)));
        // x near 20 and y near 10: the corner (20, 10) of the two guides.
        let t = s.track(&query(19.5, 10.7)).unwrap();
        assert!(close(t.at, p(20.0, 10.0)), "{t:?}");
        assert!(t.guides.iter().all(Option::is_some));
        // Lined up with nothing (the origin's lines are x = 0 and y = 0): no track.
        assert!(s.track(&query(35.0, 20.0)).is_none());
        // Alignment off: no track either.
        let off = TrackQuery {
            align: false,
            ..query(20.6, 5.0)
        };
        assert!(s.track(&off).is_none());
    }

    #[test]
    fn a_line_from_the_end_of_a_line_tracks_at_right_angles_and_is_constrained() {
        // The line (0, 0)-(10, 0) ends at (10, 0); a new line from there towards
        // (10.6, 7) is pulled onto x = 10, the perpendicular.
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(10.0, 0.0)).unwrap();
        let old = s.add_line(a, b).unwrap();
        let q = TrackQuery {
            from: Some(p(10.0, 0.0)),
            from_point: Some(b),
            align: false,
            ..query(10.6, 7.0)
        };
        let t = s.track(&q).unwrap();
        let g = t.direction().unwrap();
        assert_eq!((g.kind, g.curve), (GuideKind::Perpendicular, Some(old)));
        assert!(close(t.at, p(10.0, 7.0)));
        let e = s.add_point(t.at).unwrap();
        let new = s.add_line(b, e).unwrap();
        assert_eq!(
            s.track_constraints(&t, new),
            vec![Constraint::Perpendicular { a: new, b: old }]
        );
        // Without inference the same cursor tracks nothing.
        let off = TrackQuery { infer: false, ..q };
        assert!(s.track(&off).is_none());
    }

    #[test]
    fn a_slanted_line_gives_the_perpendicular_by_hand() {
        // The line (0, 0)-(3, 4) has direction (0.6, 0.8); at its end (3, 4) the
        // perpendicular is (-0.8, 0.6). A cursor 5 along it is (-1, 7); nudged by
        // 0.3 along the line to (-0.82, 7.24), it comes back to (-1, 7).
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(3.0, 4.0)).unwrap();
        s.add_line(a, b).unwrap();
        let q = TrackQuery {
            from: Some(p(3.0, 4.0)),
            from_point: Some(b),
            align: false,
            ..query(-0.82, 7.24)
        };
        let t = s.track(&q).unwrap();
        assert!(close(t.at, p(-1.0, 7.0)), "{t:?}");
    }

    #[test]
    fn a_line_from_the_end_of_an_arc_tracks_its_tangent() {
        // A quarter arc round the origin from (5, 0) to (0, 5): at (0, 5) the radius is
        // vertical, so the tangent is horizontal. From (0, 5) towards (-8, 5.5): (-8, 5).
        let mut s = Sketch::new();
        let c = s.add_point(p(0.0, 0.0)).unwrap();
        let a = s.add_point(p(5.0, 0.0)).unwrap();
        let b = s.add_point(p(0.0, 5.0)).unwrap();
        let arc = s.add_arc(c, a, b).unwrap();
        let q = TrackQuery {
            from: Some(p(0.0, 5.0)),
            from_point: Some(b),
            align: false,
            ..query(-8.0, 5.5)
        };
        let t = s.track(&q).unwrap();
        assert_eq!(t.direction().map(|g| g.kind), Some(GuideKind::Tangent));
        assert!(close(t.at, p(-8.0, 5.0)));
        let e = s.add_point(t.at).unwrap();
        let new = s.add_line(b, e).unwrap();
        assert_eq!(
            s.track_constraints(&t, new),
            vec![Constraint::Tangent { a: new, b: arc }]
        );
    }

    #[test]
    fn polar_guides_hold_whole_steps_and_meet_alignment_guides() {
        // From the origin with 15 degree steps, a cursor at 44 degrees, 10 out, is pulled
        // onto 45 degrees at its projected length 10 cos 1 degree.
        let s = Sketch::new();
        let a = 44f64.to_radians();
        let cursor = p(10.0 * a.cos(), 10.0 * a.sin());
        let q = TrackQuery {
            from: Some(Point2::ORIGIN),
            polar_step: Some(15f64.to_radians()),
            align: false,
            ..query(cursor.x, cursor.y)
        };
        let t = s.track(&q).unwrap();
        let len = 10.0 * 1f64.to_radians().cos();
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(t.direction().map(|g| g.kind), Some(GuideKind::Polar));
        assert!(close(t.at, p(len * h, len * h)), "{t:?}");
        // With a point at (7, 0), the vertical x = 7 crosses the 45 degree guide at (7, 7).
        let mut s = Sketch::new();
        s.add_point(p(7.0, 0.0)).unwrap();
        let q = TrackQuery {
            align: true,
            ..TrackQuery {
                cursor: p(7.3, 6.8),
                ..q
            }
        };
        let t = s.track(&q).unwrap();
        assert!(close(t.at, p(7.0, 7.0)), "{t:?}");
        assert_eq!(t.guides[1].map(|g| g.kind), Some(GuideKind::AlignVertical));
    }

    #[test]
    fn guides_cross_where_the_hand_solution_says() {
        // y = x and x = 4 cross at (4, 4); parallel guides do not cross.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let x = guide_crossing(Point2::ORIGIN, Vec2::new(h, h), p(4.0, -2.0), Vec2::Y).unwrap();
        assert!(close(x, p(4.0, 4.0)));
        assert!(guide_crossing(Point2::ORIGIN, Vec2::X, p(0.0, 1.0), Vec2::X).is_none());
    }

    #[test]
    fn hostile_queries_track_nothing() {
        let mut s = Sketch::new();
        s.add_point(p(1.0, 1.0)).unwrap();
        for q in [
            query(f64::NAN, 1.0),
            TrackQuery {
                radius: f64::INFINITY,
                ..query(1.2, 5.0)
            },
            TrackQuery {
                radius: -1.0,
                ..query(1.2, 5.0)
            },
        ] {
            assert!(s.track(&q).is_none());
        }
        let q = TrackQuery {
            from: Some(p(f64::NAN, 0.0)),
            polar_step: Some(f64::NAN),
            from_point: Some(PointId(999)),
            ..query(1.2, 5.0)
        };
        // Only the alignment with (1, 1) is left.
        let t = s.track(&q).unwrap();
        assert!(close(t.at, p(1.0, 5.0)));
    }
}

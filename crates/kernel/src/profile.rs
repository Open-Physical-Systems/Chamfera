//! Planar profiles (rung K4): the regions that [`extrude`](crate::extrude) and
//! [`revolve`](crate::revolve) sweep into solids.
//!
//! A [`Profile`] is a closed outer loop and any number of hole loops, each a chain of
//! [`ProfileEdge`]s (line segments and circular arcs, a full circle being one arc of a
//! whole turn) in the coordinates of a frame's plane. It maps directly from a sketch
//! region: a sketch line is a [`ProfileEdge::Line`] (start and end swapped when the loop
//! runs along it backwards), a sketch arc from `start` to `end` counter-clockwise about
//! `centre` is a [`ProfileEdge::Arc`] with `start_angle` the angle of `start` and a
//! positive `sweep` (negated, starting at the end, when the loop runs backwards), and a
//! sketch circle is [`ProfileEdge::circle`].
//!
//! [`Profile::check`] validates a profile: every edge valid and longer than
//! [`LINEAR_TOLERANCE`]; each loop closed (each edge ends where the next starts, within
//! [`PROFILE_JOIN_TOLERANCE`]) and simple (no two edges meet except adjacent edges at
//! their shared vertex, decided by the K1 intersections); each loop enclosing area; every
//! hole strictly inside the outer loop and outside the other holes, touching neither. The
//! orientation is fixed automatically: the outer loop is made counter-clockwise and holes
//! clockwise, by reversing a loop (its edge order and each edge) when needed. Every
//! checked edge remembers its index in the input loop, so names derived from it
//! (`side 3`) follow the input whatever the orientation.
//!
//! A loop may also hold B-spline edges ([`ProfileEdge::Spline`], rung K10), each run over
//! its whole domain. Their area terms are integrated numerically, and the
//! self-intersection checks meet them as fine chords (see `spline_chords`).

use crate::check;
use crate::topology::MAX_PROFILE_VERTICES;
use crate::{
    Arc2, Circle2, Intersections, KernelError, KernelResult, Segment2, arc_arc, circle_arc,
    circle_circle, segment_arc, segment_circle, segment_segment,
};
use chamfera_geom::{ANGULAR_TOLERANCE, BSplineCurve2, LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// How far apart (mm) one edge's end and the next edge's start may be. Half the linear
/// tolerance, so that a vertex placed at an edge's start is within the full tolerance of
/// the previous edge's end even after the small rounding a sweep adds.
pub const PROFILE_JOIN_TOLERANCE: f64 = LINEAR_TOLERANCE * 0.5;

/// One edge of a profile loop, in the plane coordinates of the sweep's frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileEdge {
    /// The segment from `a` to `b`.
    Line { a: Point2, b: Point2 },
    /// The arc of the circle about `centre` from angle `start_angle` through `sweep`
    /// radians (positive counter-clockwise, negative clockwise, `0 < |sweep| <= 2π`). An
    /// arc with `|sweep| = 2π` is a full circle and must be alone in its loop.
    Arc {
        centre: Point2,
        radius: f64,
        start_angle: f64,
        sweep: f64,
    },
    /// A B-spline (rung K10) run over its whole domain, from its start to its end; a
    /// closed spline must be given as two or more edges (e.g. split at its middle).
    Spline(BSplineCurve2),
}

impl ProfileEdge {
    /// A line from `a` to `b`.
    pub fn line(a: Point2, b: Point2) -> ProfileEdge {
        ProfileEdge::Line { a, b }
    }
    /// A full counter-clockwise circle starting at angle 0 (at `centre + (radius, 0)`).
    pub fn circle(centre: Point2, radius: f64) -> ProfileEdge {
        ProfileEdge::Arc {
            centre,
            radius,
            start_angle: 0.0,
            sweep: TAU,
        }
    }
    /// The counter-clockwise arc from `start` to `end` about `centre` (as a sketch stores
    /// it); `end` must be as far from `centre` as `start`, within [`LINEAR_TOLERANCE`].
    pub fn arc_from_points(
        centre: Point2,
        start: Point2,
        end: Point2,
    ) -> KernelResult<ProfileEdge> {
        let a = Arc2::from_center_start_end(centre, start, end)?;
        Ok(ProfileEdge::Arc {
            centre,
            radius: a.radius(),
            start_angle: a.start_angle(),
            sweep: a.sweep(),
        })
    }

    pub fn start(&self) -> Point2 {
        match *self {
            ProfileEdge::Line { a, .. } => a,
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                ..
            } => at_angle(centre, radius, start_angle),
            ProfileEdge::Spline(ref c) => c.point_at(c.domain().0),
        }
    }
    pub fn end(&self) -> Point2 {
        match *self {
            ProfileEdge::Line { b, .. } => b,
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => at_angle(centre, radius, start_angle + sweep),
            ProfileEdge::Spline(ref c) => c.point_at(c.domain().1),
        }
    }

    /// The same edge run the other way.
    pub fn reversed(&self) -> ProfileEdge {
        match *self {
            ProfileEdge::Line { a, b } => ProfileEdge::Line { a: b, b: a },
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => ProfileEdge::Arc {
                centre,
                radius,
                start_angle: start_angle + sweep,
                sweep: -sweep,
            },
            ProfileEdge::Spline(ref c) => ProfileEdge::Spline(c.reversed()),
        }
    }

    /// The edge mirrored in the plane's x axis (`y -> -y`).
    pub(crate) fn mirrored(&self) -> ProfileEdge {
        let m = |p: Point2| Point2::new(p.x, -p.y);
        match *self {
            ProfileEdge::Line { a, b } => ProfileEdge::Line { a: m(a), b: m(b) },
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => ProfileEdge::Arc {
                centre: m(centre),
                radius,
                start_angle: -start_angle,
                sweep: -sweep,
            },
            // A spline that cannot be mirrored (never for a valid one) keeps its shape;
            // the profile check then refuses the result as not closed.
            ProfileEdge::Spline(ref c) => ProfileEdge::Spline(c.mirrored_y().unwrap_or(c.clone())),
        }
    }

    /// True for an arc of a whole turn.
    pub fn is_full_circle(&self) -> bool {
        matches!(*self, ProfileEdge::Arc { sweep, .. } if sweep.abs() >= TAU - ANGULAR_TOLERANCE)
    }

    /// The point a fraction `f` of the way along the edge.
    pub(crate) fn point_at_fraction(&self, f: f64) -> Point2 {
        match *self {
            ProfileEdge::Line { a, b } => a.lerp(b, f),
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => at_angle(centre, radius, start_angle + sweep * f),
            ProfileEdge::Spline(ref c) => {
                let (lo, hi) = c.domain();
                c.point_at(lo + (hi - lo) * f)
            }
        }
    }

    /// The unit tangent in the edge's direction a fraction `f` of the way along it.
    pub(crate) fn tangent_at_fraction(&self, f: f64) -> Vec2 {
        match *self {
            ProfileEdge::Line { a, b } => (b - a).normalized().unwrap_or(Vec2::X),
            ProfileEdge::Arc {
                start_angle, sweep, ..
            } => {
                let (s, c) = (start_angle + sweep * f).sin_cos();
                Vec2::new(-s, c) * sweep.signum()
            }
            ProfileEdge::Spline(ref c) => {
                let (lo, hi) = c.domain();
                c.d1(lo + (hi - lo) * f).normalized().unwrap_or(Vec2::X)
            }
        }
    }

    /// `½ ∫ (x dy - y dx)` along the edge: summed around a loop, its signed area.
    ///
    /// Line: `(a × b) / 2`. Arc `x = cx + r cos t, y = cy + r sin t`, `t` from `a0` to
    /// `a1 = a0 + sweep`: `x dy - y dx = (r² + r cx cos t + r cy sin t) dt`, which
    /// integrates to `r² sweep + r cx (sin a1 - sin a0) - r cy (cos a1 - cos a0)`.
    pub(crate) fn area_term(&self) -> f64 {
        match *self {
            ProfileEdge::Line { a, b } => 0.5 * (a.x * b.y - b.x * a.y),
            ProfileEdge::Arc {
                centre,
                radius: r,
                start_angle: a0,
                sweep,
            } => {
                let a1 = a0 + sweep;
                0.5 * (r * r * sweep + r * centre.x * (a1.sin() - a0.sin())
                    - r * centre.y * (a1.cos() - a0.cos()))
            }
            // Adaptive Gauss-Legendre quadrature per knot span; a non-finite result
            // (no convergence) makes the profile check refuse the loop.
            ProfileEdge::Spline(ref c) => {
                let (lo, hi) = c.domain();
                c.area_term(lo, hi).unwrap_or(f64::NAN)
            }
        }
    }

    /// `(min, max)` of `n · (p - o)` over the edge's points `p`.
    pub(crate) fn extent_along(&self, o: Point2, n: Vec2) -> (f64, f64) {
        let (s, e) = ((self.start() - o).dot(n), (self.end() - o).dot(n));
        let (mut lo, mut hi) = (s.min(e), s.max(e));
        if let ProfileEdge::Spline(ref c) = *self {
            // The control points bound the curve (convex hull property): an outer bound.
            for p in c.control_points() {
                let d = (p - o).dot(n);
                lo = lo.min(d);
                hi = hi.max(d);
            }
        }
        if let ProfileEdge::Arc {
            centre,
            radius,
            start_angle,
            sweep,
        } = *self
        {
            // The circle's extremes along n are at the angle of ±n; include those the arc
            // passes through.
            let c = (centre - o).dot(n);
            let phi = n.angle();
            for (theta, value) in [(phi, c + radius), (phi + PI, c - radius)] {
                if angle_within(theta, start_angle, sweep) {
                    lo = lo.min(value);
                    hi = hi.max(value);
                }
            }
        }
        (lo, hi)
    }

    /// The K1 curve for the intersection checks.
    fn planar(&self) -> KernelResult<PlanarCurve> {
        Ok(match *self {
            ProfileEdge::Spline(ref c) => PlanarCurve::Polyline(spline_chords(c)?),
            ProfileEdge::Line { a, b } => PlanarCurve::Segment(Segment2::new(a, b)?),
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                if self.is_full_circle() {
                    PlanarCurve::Circle(Circle2::new(centre, radius)?)
                } else {
                    PlanarCurve::Arc(Arc2::new(centre, radius, start_angle, sweep)?)
                }
            }
        })
    }

    /// A box around the edge (the whole circle's box for an arc).
    fn bounds(&self) -> (Point2, Point2) {
        match *self {
            ProfileEdge::Line { a, b } => (
                Point2::new(a.x.min(b.x), a.y.min(b.y)),
                Point2::new(a.x.max(b.x), a.y.max(b.y)),
            ),
            ProfileEdge::Arc { centre, radius, .. } => (
                Point2::new(centre.x - radius, centre.y - radius),
                Point2::new(centre.x + radius, centre.y + radius),
            ),
            ProfileEdge::Spline(ref c) => c.hull_box(),
        }
    }

    /// Checks the edge's numbers: finite, within range, longer than the tolerance.
    fn check(&self, name: &str) -> KernelResult<ProfileEdge> {
        match *self {
            ProfileEdge::Line { a, b } => {
                check::point2(name, a)?;
                check::point2(name, b)?;
                if a.distance(b) <= LINEAR_TOLERANCE {
                    return Err(KernelError::InvalidInput(format!(
                        "{name} must be longer than {LINEAR_TOLERANCE} mm"
                    )));
                }
                Ok(self.clone())
            }
            ProfileEdge::Spline(ref c) => {
                let (lo, hi) = c.hull_box();
                if !(lo.is_valid() && hi.is_valid()) {
                    return Err(KernelError::InvalidInput(format!(
                        "{name} must lie within the model's working range"
                    )));
                }
                if self.start().distance(self.end()) <= LINEAR_TOLERANCE {
                    return Err(KernelError::InvalidInput(format!(
                        "{name} is a closed spline; give it as two or more edges"
                    )));
                }
                let length = c.length()?;
                if length <= LINEAR_TOLERANCE {
                    return Err(KernelError::InvalidInput(format!(
                        "{name} must be longer than {LINEAR_TOLERANCE} mm"
                    )));
                }
                Ok(self.clone())
            }
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                let arc = Arc2::new(centre, radius, start_angle, sweep).map_err(|e| {
                    KernelError::InvalidInput(format!("{name} is not a valid arc: {e}"))
                })?;
                if !(arc.start_point().is_valid() && arc.end_point().is_valid())
                    || !Point2::new(centre.x + radius, centre.y + radius).is_valid()
                    || !Point2::new(centre.x - radius, centre.y - radius).is_valid()
                {
                    return Err(KernelError::InvalidInput(format!(
                        "{name} must lie within the model's working range"
                    )));
                }
                // A sweep within the angular tolerance of a turn is a full circle.
                let sweep = if arc.sweep().abs() >= TAU - ANGULAR_TOLERANCE {
                    TAU * arc.sweep().signum()
                } else {
                    arc.sweep()
                };
                Ok(ProfileEdge::Arc {
                    centre,
                    radius,
                    start_angle: arc.start_angle(),
                    sweep,
                })
            }
        }
    }
}

fn at_angle(centre: Point2, radius: f64, theta: f64) -> Point2 {
    let (s, c) = theta.sin_cos();
    centre + Vec2::new(c, s) * radius
}

/// Whether the angle `theta` lies on the arc from `start` through `sweep`.
fn angle_within(theta: f64, start: f64, sweep: f64) -> bool {
    let (lo, w) = if sweep >= 0.0 {
        (start, sweep)
    } else {
        (start + sweep, -sweep)
    };
    check::angle_from(theta, lo) <= lo + w
}

enum PlanarCurve {
    Segment(Segment2),
    Arc(Arc2),
    Circle(Circle2),
    /// A spline's chords (see [`spline_chords`]).
    Polyline(Vec<Segment2>),
}

/// Chords per knot span of a spline in the profile's self-intersection checks.
const SPLINE_CHORDS_PER_SPAN: usize = 64;
/// Chords per full turn of an arc or circle checked against a spline.
const ARC_CHORDS_PER_TURN: f64 = 256.0;

/// A spline as chords for the intersection checks: [`SPLINE_CHORDS_PER_SPAN`] per knot
/// span. Two loop edges that cross are found unless they cross within the chords' sagitta
/// of each other; this is the one place the checks are not exact (K10 has no spline
/// intersection yet).
fn spline_chords(c: &BSplineCurve2) -> KernelResult<Vec<Segment2>> {
    chords(&c.polyline(SPLINE_CHORDS_PER_SPAN))
}

fn chords(points: &[Point2]) -> KernelResult<Vec<Segment2>> {
    points
        .windows(2)
        .filter(|w| w[0].distance(w[1]) > LINEAR_TOLERANCE)
        .map(|w| Segment2::new(w[0], w[1]))
        .collect()
}

/// A line, arc or circle as chords, to meet a spline's chords.
fn as_chords(c: &PlanarCurve) -> Vec<Segment2> {
    match c {
        PlanarCurve::Segment(s) => vec![*s],
        PlanarCurve::Polyline(p) => p.clone(),
        PlanarCurve::Arc(a) => arc_chords(a.center(), a.radius(), a.start_angle(), a.sweep()),
        PlanarCurve::Circle(c) => arc_chords(c.center(), c.radius(), 0.0, TAU),
    }
}

fn arc_chords(centre: Point2, radius: f64, start: f64, sweep: f64) -> Vec<Segment2> {
    let n = ((sweep.abs() / TAU) * ARC_CHORDS_PER_TURN).ceil().max(2.0) as usize;
    let points: Vec<Point2> = (0..=n)
        .map(|i| at_angle(centre, radius, start + sweep * i as f64 / n as f64))
        .collect();
    chords(&points).unwrap_or_default()
}

/// Chords per block when pruning chord pairs by boxes.
const CHORD_BLOCK: usize = 16;

/// Consecutive runs of [`CHORD_BLOCK`] chords with their boxes.
fn chord_blocks(c: &[Segment2]) -> Vec<(usize, (Point2, Point2))> {
    c.chunks(CHORD_BLOCK)
        .enumerate()
        .map(|(k, run)| {
            let mut lo = Point2::new(f64::INFINITY, f64::INFINITY);
            let mut hi = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
            for s in run {
                for p in [s.start(), s.end()] {
                    lo = Point2::new(lo.x.min(p.x), lo.y.min(p.y));
                    hi = Point2::new(hi.x.max(p.x), hi.y.max(p.y));
                }
            }
            (k * CHORD_BLOCK, (lo, hi))
        })
        .collect()
}

/// Where two chord lists meet (points and overlaps), for the spline cases; pairs of
/// blocks whose boxes are apart are skipped.
fn chord_intersections(a: &[Segment2], b: &[Segment2]) -> Intersections {
    let mut out = Intersections::default();
    let (ba, bb) = (chord_blocks(a), chord_blocks(b));
    for (ia, boxa) in &ba {
        for (ib, boxb) in &bb {
            if boxes_apart(*boxa, *boxb) {
                continue;
            }
            for x in a.iter().skip(*ia).take(CHORD_BLOCK) {
                for y in b.iter().skip(*ib).take(CHORD_BLOCK) {
                    let hits = segment_segment(x, y);
                    out.points.extend(hits.points);
                    out.overlaps.extend(hits.overlaps);
                }
            }
        }
    }
    out
}

/// True when two chords of one spline that are not neighbours meet: the spline crosses
/// or touches itself.
fn chords_cross_themselves(c: &[Segment2]) -> bool {
    let blocks = chord_blocks(c);
    for (ka, (ia, boxa)) in blocks.iter().enumerate() {
        for (ib, boxb) in blocks.iter().skip(ka) {
            if boxes_apart(*boxa, *boxb) {
                continue;
            }
            for (i, x) in c.iter().enumerate().skip(*ia).take(CHORD_BLOCK) {
                for (j, y) in c.iter().enumerate().skip(*ib).take(CHORD_BLOCK) {
                    if j <= i + 1 {
                        continue;
                    }
                    if !segment_segment(x, y).is_empty() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn intersections(a: &PlanarCurve, b: &PlanarCurve) -> Intersections {
    use PlanarCurve::{Arc, Circle, Polyline, Segment};
    match (a, b) {
        (Polyline(_), _) | (_, Polyline(_)) => chord_intersections(&as_chords(a), &as_chords(b)),
        (Segment(x), Segment(y)) => segment_segment(x, y),
        (Segment(x), Arc(y)) => segment_arc(x, y),
        (Segment(x), Circle(y)) => segment_circle(x, y),
        (Arc(x), Segment(y)) => segment_arc(y, x).swapped(),
        (Arc(x), Arc(y)) => arc_arc(x, y),
        (Arc(x), Circle(y)) => circle_arc(y, x).swapped(),
        (Circle(x), Segment(y)) => segment_circle(y, x).swapped(),
        (Circle(x), Arc(y)) => circle_arc(x, y),
        (Circle(x), Circle(y)) => circle_circle(x, y),
    }
}

/// A closed region in a frame's plane: an outer loop and holes. See the module
/// documentation for the rules [`Profile::check`] enforces.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    pub outer: Vec<ProfileEdge>,
    pub holes: Vec<Vec<ProfileEdge>>,
}

/// One edge of a checked loop.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedEdge {
    /// The geometry, running in the checked loop's direction, starting exactly at this
    /// edge's vertex (lines also end exactly at the next vertex).
    pub edge: ProfileEdge,
    /// The index of the input edge this came from.
    pub source: usize,
    /// The input index of the vertex at this edge's start (vertex `i` of an input loop is
    /// the start of input edge `i`).
    pub vertex_source: usize,
}

/// A checked loop: counter-clockwise for the outer loop, clockwise for holes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedLoop {
    pub edges: Vec<CheckedEdge>,
    /// `None` for the outer loop, `Some(j)` for input hole `j`.
    pub hole: Option<usize>,
}

impl CheckedLoop {
    /// The vertex at the start of checked edge `k`.
    pub fn vertex(&self, k: usize) -> Point2 {
        self.edges
            .get(k % self.edges.len().max(1))
            .map_or(Point2::ORIGIN, |e| e.edge.start())
    }
    pub fn len(&self) -> usize {
        self.edges.len()
    }
}

/// A profile that passed [`Profile::check`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedProfile {
    pub outer: CheckedLoop,
    pub holes: Vec<CheckedLoop>,
    /// The region's area (outer minus holes), mm².
    pub area: f64,
}

impl CheckedProfile {
    /// The outer loop followed by the holes.
    pub fn loops(&self) -> impl Iterator<Item = &CheckedLoop> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }
}

impl Profile {
    pub fn new(outer: Vec<ProfileEdge>, holes: Vec<Vec<ProfileEdge>>) -> Profile {
        Profile { outer, holes }
    }

    /// The closed polygon through `points` (each joined to the next, the last to the
    /// first), without holes.
    pub fn polygon(points: &[Point2]) -> Profile {
        Profile {
            outer: polygon_edges(points),
            holes: Vec::new(),
        }
    }

    /// The disc of `radius` about `centre`.
    pub fn circle(centre: Point2, radius: f64) -> Profile {
        Profile {
            outer: vec![ProfileEdge::circle(centre, radius)],
            holes: Vec::new(),
        }
    }

    /// The same profile with a hole loop added.
    pub fn with_hole(mut self, hole: Vec<ProfileEdge>) -> Profile {
        self.holes.push(hole);
        self
    }

    /// The region's area (outer loop minus holes), after checking the profile.
    pub fn area(&self) -> KernelResult<f64> {
        Ok(self.check()?.area)
    }

    /// The profile mirrored in the plane's x axis.
    pub(crate) fn mirrored(&self) -> Profile {
        Profile {
            outer: self.outer.iter().map(ProfileEdge::mirrored).collect(),
            holes: self
                .holes
                .iter()
                .map(|h| h.iter().map(ProfileEdge::mirrored).collect())
                .collect(),
        }
    }

    /// Validates the profile and fixes its orientation (see the module documentation).
    pub(crate) fn check(&self) -> KernelResult<CheckedProfile> {
        let total = self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>();
        if total > MAX_PROFILE_VERTICES || self.holes.len() > MAX_PROFILE_VERTICES {
            return Err(KernelError::InvalidInput(format!(
                "a profile may have at most {MAX_PROFILE_VERTICES} edges in all"
            )));
        }
        let (outer, outer_area) = check_loop(&self.outer, None)?;
        let mut holes = Vec::with_capacity(self.holes.len());
        let mut area = outer_area;
        for (j, h) in self.holes.iter().enumerate() {
            let (l, a) = check_loop(h, Some(j))?;
            area -= a;
            holes.push(l);
        }
        check_holes(&outer, &holes)?;
        if area <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
            return Err(KernelError::InvalidInput(
                "the profile must enclose an area".into(),
            ));
        }
        Ok(CheckedProfile { outer, holes, area })
    }
}

/// The closed chain of lines through `points`.
pub fn polygon_edges(points: &[Point2]) -> Vec<ProfileEdge> {
    let n = points.len();
    (0..n)
        .map(|i| ProfileEdge::Line {
            a: points[i],
            b: points[(i + 1) % n],
        })
        .collect()
}

fn loop_name(hole: Option<usize>) -> String {
    match hole {
        None => "the outer loop".into(),
        Some(j) => format!("hole {j}"),
    }
}

/// Checks one loop and returns it oriented (outer counter-clockwise, holes clockwise)
/// with its enclosed area (positive).
fn check_loop(edges: &[ProfileEdge], hole: Option<usize>) -> KernelResult<(CheckedLoop, f64)> {
    let name = loop_name(hole);
    let n = edges.len();
    if n == 0 {
        return Err(KernelError::InvalidInput(format!("{name} has no edges")));
    }
    let mut checked = Vec::with_capacity(n);
    for (i, e) in edges.iter().enumerate() {
        checked.push(e.check(&format!("edge {i} of {name}"))?);
    }
    if n == 1 && !checked[0].is_full_circle() {
        return Err(KernelError::InvalidInput(format!(
            "{name} has one edge, which must then be a full circle"
        )));
    }
    if n > 1 && checked.iter().any(ProfileEdge::is_full_circle) {
        return Err(KernelError::InvalidInput(format!(
            "a full circle must be alone in its loop ({name})"
        )));
    }
    for i in 0..n {
        let (e, next) = (&checked[i], &checked[(i + 1) % n]);
        if e.end().distance(next.start()) > PROFILE_JOIN_TOLERANCE {
            return Err(KernelError::InvalidInput(format!(
                "{name} is not closed: edge {i} must end where edge {} starts (within {PROFILE_JOIN_TOLERANCE} mm)",
                (i + 1) % n
            )));
        }
    }
    // Lines start and end exactly at the vertices (each the start of its edge).
    for i in 0..n {
        let next_start = checked[(i + 1) % n].start();
        if let ProfileEdge::Line { b, .. } = &mut checked[i] {
            *b = next_start;
        }
    }
    check_simple(&checked, &name)?;
    let signed: f64 = checked.iter().map(ProfileEdge::area_term).sum();
    if !signed.is_finite() || signed.abs() <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
        return Err(KernelError::InvalidInput(format!(
            "{name} must enclose an area"
        )));
    }
    let want_ccw = hole.is_none();
    let mut out: Vec<CheckedEdge> = checked
        .iter()
        .enumerate()
        .map(|(i, e)| CheckedEdge {
            edge: e.clone(),
            source: i,
            vertex_source: i,
        })
        .collect();
    if (signed > 0.0) != want_ccw {
        // Reversed: checked edge k is input edge n-1-k run backwards; it starts where
        // input edge n-1-k ends, at input vertex (n - k) % n.
        out = (0..n)
            .map(|k| {
                let src = n - 1 - k;
                CheckedEdge {
                    edge: checked[src].reversed(),
                    source: src,
                    vertex_source: (src + 1) % n,
                }
            })
            .collect();
        // Re-snap lines to the new vertices (each the start of its edge).
        for k in 0..n {
            let next_start = out[(k + 1) % n].edge.start();
            if let ProfileEdge::Line { b, .. } = &mut out[k].edge {
                *b = next_start;
            }
        }
        for k in 0..n {
            let prev_end = out[(k + n - 1) % n].edge.end();
            if let ProfileEdge::Line { a, .. } = &mut out[k].edge {
                *a = prev_end;
            }
        }
    }
    Ok((CheckedLoop { edges: out, hole }, signed.abs()))
}

fn boxes_apart(a: (Point2, Point2), b: (Point2, Point2)) -> bool {
    let t = LINEAR_TOLERANCE;
    a.1.x + t < b.0.x || b.1.x + t < a.0.x || a.1.y + t < b.0.y || b.1.y + t < a.0.y
}

/// No two edges of a loop meet, except adjacent edges at their shared vertex.
fn check_simple(edges: &[ProfileEdge], name: &str) -> KernelResult<()> {
    let n = edges.len();
    let curves = edges
        .iter()
        .map(ProfileEdge::planar)
        .collect::<KernelResult<Vec<_>>>()?;
    let boxes: Vec<_> = edges.iter().map(ProfileEdge::bounds).collect();
    for (i, c) in curves.iter().enumerate() {
        if let PlanarCurve::Polyline(chords) = c
            && chords_cross_themselves(chords)
        {
            return Err(KernelError::InvalidInput(format!(
                "{name} must not cross or touch itself (spline edge {i} crosses itself)"
            )));
        }
    }
    for i in 0..n {
        for j in i + 1..n {
            if boxes_apart(boxes[i], boxes[j]) {
                continue;
            }
            let hits = intersections(&curves[i], &curves[j]);
            // Shared vertices: j follows i, or i follows j (through the loop's end).
            let mut shared = Vec::new();
            if j == i + 1 {
                shared.push(edges[j].start());
            }
            if i == 0 && j == n - 1 {
                shared.push(edges[i].start());
            }
            let bad = !hits.overlaps.is_empty()
                || hits.points.iter().any(|p| {
                    !shared
                        .iter()
                        .any(|s| s.distance(p.point) <= LINEAR_TOLERANCE)
                });
            if bad {
                return Err(KernelError::InvalidInput(format!(
                    "{name} must not cross or touch itself (edges {i} and {j} meet)"
                )));
            }
        }
    }
    Ok(())
}

/// Crossings of the ray from `p` toward +x with a loop, half-open in y (`p` is not on
/// the loop). Arcs are split into pieces monotone in y; a piece that ends at a vertex
/// uses that vertex's own coordinates, so neighbouring edges agree exactly there.
fn ray_crossings(l: &CheckedLoop, p: Point2) -> usize {
    let crosses =
        |a: Point2, b: Point2, x_at: &dyn Fn() -> f64| (a.y > p.y) != (b.y > p.y) && p.x < x_at();
    let mut count = 0;
    let n = l.len();
    for (k, e) in l.edges.iter().enumerate() {
        let (start, end) = (l.vertex(k), l.vertex((k + 1) % n));
        match e.edge {
            ProfileEdge::Spline(ref c) => count += c.ray_crossings(start, end, p),
            ProfileEdge::Line { .. } => {
                let x = || start.x + (p.y - start.y) * (end.x - start.x) / (end.y - start.y);
                count += usize::from(crosses(start, end, &x));
            }
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                let dy = p.y - centre.y;
                let half = (radius * radius - dy * dy).max(0.0).sqrt();
                if e.edge.is_full_circle() {
                    // Two crossings where the line meets the circle, none when it misses
                    // or touches it.
                    if dy.abs() < radius {
                        count +=
                            usize::from(p.x < centre.x - half) + usize::from(p.x < centre.x + half);
                    }
                    continue;
                }
                let a1 = start_angle + sweep;
                let (lo, hi) = (start_angle.min(a1), start_angle.max(a1));
                let mut angles = vec![start_angle];
                let mut cuts = Vec::new();
                let mut m = ((lo - FRAC_PI_2) / PI).floor() + 1.0;
                while FRAC_PI_2 + m * PI < hi && cuts.len() < 8 {
                    cuts.push(FRAC_PI_2 + m * PI);
                    m += 1.0;
                }
                if sweep < 0.0 {
                    cuts.reverse();
                }
                angles.extend(cuts);
                angles.push(a1);
                let last = angles.len() - 2;
                for (i, w) in angles.windows(2).enumerate() {
                    let pa = if i == 0 {
                        start
                    } else {
                        at_angle(centre, radius, w[0])
                    };
                    let pb = if i == last {
                        end
                    } else {
                        at_angle(centre, radius, w[1])
                    };
                    let side = (0.5 * (w[0] + w[1])).cos().signum();
                    count += usize::from(crosses(pa, pb, &|| centre.x + side * half));
                }
            }
        }
    }
    count
}

/// Holes lie strictly inside the outer loop, outside each other, and no two loops touch.
fn check_holes(outer: &CheckedLoop, holes: &[CheckedLoop]) -> KernelResult<()> {
    let mut loops: Vec<&CheckedLoop> = vec![outer];
    loops.extend(holes.iter());
    let curves = loops
        .iter()
        .map(|l| {
            l.edges
                .iter()
                .map(|e| Ok((e.edge.planar()?, e.edge.bounds())))
                .collect::<KernelResult<Vec<_>>>()
        })
        .collect::<KernelResult<Vec<_>>>()?;
    for (i, a) in curves.iter().enumerate() {
        for b in curves.iter().skip(i + 1) {
            for (ca, ba) in a {
                for (cb, bb) in b {
                    if !boxes_apart(*ba, *bb) && !intersections(ca, cb).is_empty() {
                        return Err(KernelError::InvalidInput(
                            "holes must not touch the outline or each other".into(),
                        ));
                    }
                }
            }
        }
    }
    for (i, h) in holes.iter().enumerate() {
        let p = h.vertex(0);
        if ray_crossings(outer, p).is_multiple_of(2) {
            return Err(KernelError::InvalidInput(format!(
                "hole {} must lie inside the outline",
                h.hole.unwrap_or(i)
            )));
        }
        for (j, other) in holes.iter().enumerate() {
            if i != j && ray_crossings(other, p) % 2 == 1 {
                return Err(KernelError::InvalidInput(format!(
                    "hole {} must not lie inside hole {}",
                    h.hole.unwrap_or(i),
                    other.hole.unwrap_or(j)
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, w: f64, h: f64) -> Vec<ProfileEdge> {
        polygon_edges(&[
            Point2::new(x0, y0),
            Point2::new(x0 + w, y0),
            Point2::new(x0 + w, y0 + h),
            Point2::new(x0, y0 + h),
        ])
    }

    #[test]
    fn areas_of_polygons_circles_and_slots() {
        // 50 x 30 rectangle with an r = 5 hole: 1500 - 25π.
        let p = Profile::new(rect(0.0, 0.0, 50.0, 30.0), vec![])
            .with_hole(vec![ProfileEdge::circle(Point2::new(25.0, 15.0), 5.0)]);
        let a = p.area().unwrap();
        assert!((a - (1500.0 - 25.0 * PI)).abs() < 1e-9);
        // A slot: a 20 x 10 rectangle with half discs of r = 5 on its short sides:
        // 200 + 25π.
        let slot = Profile::new(
            vec![
                ProfileEdge::line(Point2::new(0.0, -5.0), Point2::new(20.0, -5.0)),
                ProfileEdge::Arc {
                    centre: Point2::new(20.0, 0.0),
                    radius: 5.0,
                    start_angle: -FRAC_PI_2,
                    sweep: PI,
                },
                ProfileEdge::line(Point2::new(20.0, 5.0), Point2::new(0.0, 5.0)),
                ProfileEdge::Arc {
                    centre: Point2::new(0.0, 0.0),
                    radius: 5.0,
                    start_angle: FRAC_PI_2,
                    sweep: PI,
                },
            ],
            vec![],
        );
        assert!((slot.area().unwrap() - (200.0 + 25.0 * PI)).abs() < 1e-9);
    }

    #[test]
    fn orientation_is_fixed_and_sources_are_kept() {
        // A clockwise square becomes counter-clockwise; checked edge 0 is input edge 3
        // reversed, starting at input vertex 0.
        let cw = polygon_edges(&[
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 1.0),
            Point2::new(1.0, 1.0),
            Point2::new(1.0, 0.0),
        ]);
        let c = Profile::new(cw, vec![]).check().unwrap();
        assert!((c.area - 1.0).abs() < 1e-15);
        let signed: f64 = c.outer.edges.iter().map(|e| e.edge.area_term()).sum();
        assert!(signed > 0.0);
        assert_eq!(c.outer.edges[0].source, 3);
        assert_eq!(c.outer.edges[0].vertex_source, 0);
        assert_eq!(c.outer.vertex(0), Point2::new(0.0, 0.0));
        // A counter-clockwise circle hole is turned clockwise.
        let p = Profile::new(rect(-5.0, -5.0, 10.0, 10.0), vec![])
            .with_hole(vec![ProfileEdge::circle(Point2::ORIGIN, 1.0)]);
        let c = p.check().unwrap();
        let ProfileEdge::Arc { sweep, .. } = c.holes[0].edges[0].edge else {
            panic!()
        };
        assert!(sweep < 0.0);
    }

    #[test]
    fn bad_profiles_are_refused() {
        let ok = rect(0.0, 0.0, 10.0, 10.0);
        assert!(Profile::new(ok.clone(), vec![]).check().is_ok());
        // Empty, open, too short, NaN, bow-tie.
        assert!(Profile::default().check().is_err());
        let mut open = ok.clone();
        open.pop();
        assert!(Profile::new(open, vec![]).check().is_err());
        let mut gap = ok.clone();
        gap[1] = ProfileEdge::line(Point2::new(10.0, 1e-6), Point2::new(10.0, 10.0));
        assert!(Profile::new(gap, vec![]).check().is_err());
        let nan = vec![ProfileEdge::circle(Point2::new(f64::NAN, 0.0), 1.0)];
        assert!(Profile::new(nan, vec![]).check().is_err());
        let bowtie = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 1.0),
            Point2::new(1.0, 0.0),
            Point2::new(0.0, 1.0),
        ]);
        assert!(bowtie.check().is_err());
        // A spike folding back on itself, and a flat triangle.
        let spike = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 1.0),
        ]);
        assert!(spike.check().is_err());
        let flat = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(2.0, 0.0),
        ]);
        assert!(flat.check().is_err());
        // A full circle with other edges; a lone non-circle; a zero sweep.
        let mut mixed = ok.clone();
        mixed.push(ProfileEdge::circle(Point2::new(5.0, 5.0), 1.0));
        assert!(Profile::new(mixed, vec![]).check().is_err());
        let lone = vec![ProfileEdge::Arc {
            centre: Point2::ORIGIN,
            radius: 1.0,
            start_angle: 0.0,
            sweep: PI,
        }];
        assert!(Profile::new(lone, vec![]).check().is_err());
        let zero = vec![ProfileEdge::Arc {
            centre: Point2::ORIGIN,
            radius: 1.0,
            start_angle: 0.0,
            sweep: 0.0,
        }];
        assert!(Profile::new(zero, vec![]).check().is_err());
        // An arc crossing a line of the same loop: the left side of a 4 x 1 rectangle
        // replaced by the clockwise arc from (0, 1) to (0, 0) about (1.5, 0.5), radius
        // √2.5, which crosses the bottom side at (3, 0) ((x - 1.5)² + 0.25 = 2.5).
        let a0 = 0.5f64.atan2(-1.5);
        let crossing = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0)),
            ProfileEdge::line(Point2::new(4.0, 0.0), Point2::new(4.0, 1.0)),
            ProfileEdge::line(Point2::new(4.0, 1.0), Point2::new(0.0, 1.0)),
            ProfileEdge::Arc {
                centre: Point2::new(1.5, 0.5),
                radius: 2.5f64.sqrt(),
                start_angle: a0,
                sweep: -2.0 * a0,
            },
        ];
        assert!(Profile::new(crossing, vec![]).check().is_err());
        let big = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0)),
            ProfileEdge::line(Point2::new(4.0, 0.0), Point2::new(4.0, 1.0)),
            ProfileEdge::line(Point2::new(4.0, 1.0), Point2::new(0.0, 1.0)),
            ProfileEdge::Arc {
                centre: Point2::new(0.0, 0.5),
                radius: 0.5,
                start_angle: FRAC_PI_2,
                sweep: -PI,
            },
        ];
        // Clockwise from (0, 1) to (0, 0) about (0, 0.5) bulges to +x inside the
        // rectangle without touching it: valid but the area shrinks by π/8.
        let a = Profile::new(big, vec![]).area().unwrap();
        assert!((a - (4.0 - PI / 8.0)).abs() < 1e-12);
        // Holes outside, overlapping, touching, nested.
        let outside = Profile::new(ok.clone(), vec![rect(20.0, 0.0, 1.0, 1.0)]);
        assert!(outside.check().is_err());
        let touching = Profile::new(ok.clone(), vec![rect(0.0, 2.0, 1.0, 1.0)]);
        assert!(touching.check().is_err());
        let overlapping = Profile::new(
            ok.clone(),
            vec![rect(2.0, 2.0, 3.0, 3.0), rect(4.0, 4.0, 3.0, 3.0)],
        );
        assert!(overlapping.check().is_err());
        let nested = Profile::new(
            ok.clone(),
            vec![rect(2.0, 2.0, 6.0, 6.0), rect(4.0, 4.0, 1.0, 1.0)],
        );
        assert!(nested.check().is_err());
        let tangent_circle = Profile::new(
            ok,
            vec![vec![ProfileEdge::circle(Point2::new(5.0, 1.0), 1.0)]],
        );
        assert!(tangent_circle.check().is_err());
    }
}

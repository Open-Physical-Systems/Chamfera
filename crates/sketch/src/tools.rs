//! Sketch tools: trim, extend, split, fillet, chamfer, offset, mirror, rectangular and
//! circular patterns, and three drawing helpers (centre rectangle, three-point arc,
//! tangent arc).
//!
//! Each tool is a [`ToolOp`]. [`Sketch::apply_tool`] applies it, checks the sketch and
//! solves; if anything fails the sketch is left exactly as it was. [`Sketch::preview_tool`]
//! applies it to a copy without solving, for a front end's preview.
//!
//! # Cutting curves
//! Curves are cut where they meet other non-construction curves (`intersect.rs`). A cut
//! that lands on an endpoint of the other curve reuses that point, so the pieces are joined
//! to it; otherwise a new point is made there and kept on the other curve (point on line or
//! point on circle). A cut curve keeps its id on its first kept piece, with the constraints
//! on it; constraints that measure its extent (length, midpoint, equal length) are dropped
//! because the piece is shorter. The other pieces of a line stay on the first piece's line
//! (point on line); pieces of an arc or circle share its centre, and a piece cut off from
//! the others is kept on its circle (point on circle). Points left on no curve are removed,
//! with their constraints.
//!
//! # Corners
//! A fillet replaces the corner of two lines with a tangent arc of the given radius (two
//! tangent constraints and a radius dimension); a chamfer with a line at the given
//! distances (two distance dimensions). The corner point itself stays, kept on both lines,
//! so constraints and dimensions on the sharp corner keep working.
//!
//! # Offset
//! A chain of lines and arcs (open or closed) is offset exactly: lines move along their
//! normal, arcs keep their centre and change radius. Where neighbours meet at an angle the
//! offset pieces are extended or cut back to meet (sharp corners), or, with
//! [`OffsetCorners::Round`], the outside corners get an arc around the original corner.
//! The copies are tied: offset lines are parallel to their originals at a distance
//! dimension, offset arcs share the original centre.

use crate::constraint::Constraint;
use crate::entity::{ConstraintId, Curve, CurveId, PointId};
use crate::intersect::{self, Shape, wrap};
use crate::sketch::{Sketch, SketchError};
use crate::solver::SolveError;
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Most curves one split or offset looks at (each pair of curves is intersected).
pub const MAX_TOOL_CURVES: usize = 2000;
/// Most copies one pattern makes.
pub const MAX_PATTERN_COPIES: usize = 1000;
/// Tangent directions closer than this (radians, as the sine of the angle) are treated as a
/// smooth join by the offset.
const SMOOTH_JOIN: f64 = 1e-9;

/// Why a tool did nothing. The sketch is unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ToolError {
    #[error(transparent)]
    Sketch(#[from] SketchError),
    #[error(transparent)]
    Solve(#[from] SolveError),
    #[error("{0}")]
    Refused(String),
}

fn refuse<T>(msg: impl Into<String>) -> Result<T, ToolError> {
    Err(ToolError::Refused(msg.into()))
}

/// What a tool changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolReport {
    pub created_points: Vec<PointId>,
    pub removed_points: Vec<PointId>,
    pub created_curves: Vec<CurveId>,
    /// Curves that kept their id but changed (a trimmed line, a filleted side).
    pub changed_curves: Vec<CurveId>,
    pub removed_curves: Vec<CurveId>,
    pub created_constraints: Vec<ConstraintId>,
    pub removed_constraints: Vec<ConstraintId>,
}

/// How an offset treats the outside of a corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OffsetCorners {
    /// Neighbouring offset pieces are extended until they meet.
    #[default]
    Sharp,
    /// An arc around the original corner joins them.
    Round,
}

/// One use of a sketch tool.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOp {
    /// Removes the piece of `curve` nearest `at` between its neighbouring cuts (or ends).
    /// A curve that nothing cuts is removed whole.
    Trim { curve: CurveId, at: Point2 },
    /// Lengthens the end of a line or arc nearest `at` to the first curve it would meet.
    Extend { curve: CurveId, at: Point2 },
    /// Splits `curve` and the curves it crosses where they cross; with `None`, every pair
    /// of non-construction curves.
    Split { curve: Option<CurveId> },
    /// Rounds the corner where two lines meet at `corner`.
    Fillet { corner: PointId, radius: f64 },
    /// Cuts the corner where two lines meet at `corner`, `distance` along the first line
    /// and `distance2` (default the same) along the second.
    Chamfer {
        corner: PointId,
        distance: f64,
        distance2: Option<f64>,
    },
    /// Offsets a chain of curves. One line or arc stands for the whole chain it is part
    /// of. `distance` is positive outward for a closed chain and to the left of an open
    /// one; with `side`, the offset goes toward that point and only the size of `distance`
    /// counts. `dimension` adds the distance dimensions that tie the copy.
    Offset {
        curves: Vec<CurveId>,
        distance: f64,
        side: Option<Point2>,
        corners: OffsetCorners,
        dimension: bool,
    },
    /// Mirrors curves and points in the line `axis`, tied by symmetric constraints.
    Mirror {
        curves: Vec<CurveId>,
        points: Vec<PointId>,
        axis: CurveId,
    },
    /// Copies curves and points on a grid, `columns` along X spaced `dx` and `rows` along
    /// Y spaced `dy` (the original is the first copy).
    RectangularPattern {
        curves: Vec<CurveId>,
        points: Vec<PointId>,
        columns: u32,
        rows: u32,
        dx: f64,
        dy: f64,
    },
    /// Copies curves and points around `centre`, `count` instances in all (the original
    /// is the first) spread over `angle` radians (a full turn spaces them evenly).
    CircularPattern {
        curves: Vec<CurveId>,
        points: Vec<PointId>,
        centre: Point2,
        count: u32,
        angle: f64,
    },
    /// A rectangle by its centre and one corner, with construction diagonals.
    CentreRectangle { centre: Point2, corner: Point2 },
    /// An arc from `start` through `through` to `end`.
    ThreePointArc {
        start: Point2,
        through: Point2,
        end: Point2,
    },
    /// An arc that leaves the free end `from` of a line or arc tangentially and ends at
    /// `to`.
    TangentArc { from: PointId, to: Point2 },
}

/// A curve that can cut others: its id, geometry and ends.
type Cutter = (CurveId, Shape, Option<(PointId, PointId)>);

/// A place where a curve is cut, from the curve's point of view.
#[derive(Debug, Clone)]
struct Cut {
    /// Parameter along the cut curve.
    t: f64,
    at: Point2,
    /// An endpoint of a cutting curve that is already there.
    point: Option<PointId>,
    cutters: Vec<CurveId>,
}

/// Which piece trim removes and where it is cut.
struct TrimPlan {
    shape: Shape,
    lower: Option<Cut>,
    upper: Option<Cut>,
    whole: bool,
}

/// An extension: which end goes where.
struct ExtendPlan {
    end: PointId,
    at_start: bool,
    to: Point2,
    target: CurveId,
    existing: Option<PointId>,
}

/// One line or arc of a chain, in the chain's direction.
#[derive(Debug, Clone, Copy)]
enum Elem {
    Line {
        a: Point2,
        b: Point2,
    },
    /// From angle `t0` through the signed sweep `sw` (positive counter-clockwise).
    Arc {
        c: Point2,
        r: f64,
        t0: f64,
        sw: f64,
    },
}

impl Elem {
    fn start(&self) -> Point2 {
        match *self {
            Elem::Line { a, .. } => a,
            Elem::Arc { c, r, t0, .. } => c + Vec2::new(t0.cos(), t0.sin()) * r,
        }
    }

    fn end(&self) -> Point2 {
        match *self {
            Elem::Line { b, .. } => b,
            Elem::Arc { c, r, t0, sw } => c + Vec2::new((t0 + sw).cos(), (t0 + sw).sin()) * r,
        }
    }

    fn tangent_at_angle(t: f64, sw: f64) -> Vec2 {
        Vec2::new(-t.sin(), t.cos()) * sw.signum()
    }

    fn tangent_start(&self) -> Vec2 {
        match *self {
            Elem::Line { a, b } => (b - a).normalized().unwrap_or(Vec2::X),
            Elem::Arc { t0, sw, .. } => Elem::tangent_at_angle(t0, sw),
        }
    }

    fn tangent_end(&self) -> Vec2 {
        match *self {
            Elem::Line { a, b } => (b - a).normalized().unwrap_or(Vec2::X),
            Elem::Arc { t0, sw, .. } => Elem::tangent_at_angle(t0 + sw, sw),
        }
    }

    /// Offset `d` to the left of the direction of travel.
    fn offset(&self, d: f64) -> Option<Elem> {
        match *self {
            Elem::Line { a, b } => {
                let n = (b - a).normalized()?.perp();
                Some(Elem::Line {
                    a: a + n * d,
                    b: b + n * d,
                })
            }
            Elem::Arc { c, r, t0, sw } => {
                // Left of a counter-clockwise arc is toward its centre.
                let r2 = r - d * sw.signum();
                (r2 > LINEAR_TOLERANCE && r2 <= MAX_COORDINATE).then_some(Elem::Arc {
                    c,
                    r: r2,
                    t0,
                    sw,
                })
            }
        }
    }

    /// The carrier (an infinite line or a full circle) as a shape.
    fn carrier(&self) -> Shape {
        match *self {
            Elem::Line { a, b } => Shape::Segment { a, b },
            Elem::Arc { c, r, .. } => Shape::Circle { c, r },
        }
    }

    /// +1 when `p` is to the left of the element, -1 to the right.
    fn side_of(&self, p: Point2) -> f64 {
        match *self {
            Elem::Line { a, b } => {
                if (b - a).cross(p - a) >= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            Elem::Arc { c, r, sw, .. } => {
                let inside = p.distance(c) < r;
                if inside == (sw > 0.0) { 1.0 } else { -1.0 }
            }
        }
    }

    fn distance_to(&self, p: Point2) -> f64 {
        match *self {
            Elem::Line { a, b } => {
                let s = Shape::Segment { a, b };
                s.point_at(s.param(p).clamp(0.0, 1.0)).distance(p)
            }
            Elem::Arc { c, r, t0, sw } => {
                let (a0, sweep) = if sw > 0.0 { (t0, sw) } else { (t0 + sw, -sw) };
                let s = Shape::Arc { c, r, a0, sweep };
                let t = s.param(p);
                if t <= sweep {
                    (p.distance(c) - r).abs()
                } else {
                    p.distance(self.start()).min(p.distance(self.end()))
                }
            }
        }
    }
}

/// A chain element with the ids it came from.
#[derive(Debug, Clone)]
struct ChainElem {
    curve: CurveId,
    elem: Elem,
    centre: Option<PointId>,
    /// The point at the end of this element in the chain's direction.
    end_point: PointId,
}

struct Chain {
    elems: Vec<ChainElem>,
    closed: bool,
}

fn check_length(what: &str, v: f64) -> Result<(), ToolError> {
    if v.is_finite() && v > LINEAR_TOLERANCE && v <= MAX_COORDINATE {
        Ok(())
    } else {
        refuse(format!(
            "{what} must be a finite length above 0 and at most 1e6 mm, got {v}"
        ))
    }
}

fn check_point(what: &str, p: Point2) -> Result<(), ToolError> {
    if p.is_valid() {
        Ok(())
    } else {
        refuse(format!("{what} is outside the working range"))
    }
}

/// The endpoints of a line or arc (a circle has none).
fn ends(curve: &Curve) -> Option<(PointId, PointId)> {
    match curve {
        Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => Some((*start, *end)),
        Curve::Circle { .. } => None,
    }
}

/// A curve with one end replaced.
fn with_end(curve: &Curve, at_start: bool, p: PointId) -> Curve {
    let mut c = curve.clone();
    match &mut c {
        Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => {
            if at_start {
                *start = p;
            } else {
                *end = p;
            }
        }
        Curve::Circle { .. } => {}
    }
    c
}

/// What changed between two versions of a sketch.
fn report(before: &Sketch, after: &Sketch) -> ToolReport {
    ToolReport {
        created_points: after
            .points()
            .filter(|(id, _)| before.point(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
        removed_points: before
            .points()
            .filter(|(id, _)| after.point(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
        created_curves: after
            .curves()
            .filter(|(id, _)| before.curve(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
        changed_curves: after
            .curves()
            .filter(|(id, c)| before.curve(*id).is_some_and(|b| b != *c))
            .map(|(id, _)| id)
            .collect(),
        removed_curves: before
            .curves()
            .filter(|(id, _)| after.curve(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
        created_constraints: after
            .constraints()
            .filter(|(id, _)| before.constraint(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
        removed_constraints: before
            .constraints()
            .filter(|(id, _)| after.constraint(*id).is_none())
            .map(|(id, _)| id)
            .collect(),
    }
}

/// A rigid motion of the plane used by patterns: a translation or a rotation.
#[derive(Debug, Clone, Copy)]
enum Motion {
    Translate(Vec2),
    Rotate { centre: Point2, angle: f64 },
}

impl Motion {
    fn apply(&self, p: Point2) -> Point2 {
        match *self {
            Motion::Translate(v) => p + v,
            Motion::Rotate { centre, angle } => {
                let (s, c) = angle.sin_cos();
                let d = p - centre;
                centre + Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
            }
        }
    }

    /// How horizontal and vertical constraints carry over: kept, swapped, or dropped.
    fn axis_turns(&self) -> Option<bool> {
        match *self {
            Motion::Translate(_) => Some(false),
            Motion::Rotate { angle, .. } => {
                let quarter = angle / FRAC_PI_2;
                let k = quarter.round();
                if (quarter - k).abs() > 1e-12 {
                    None
                } else {
                    Some((k as i64).rem_euclid(2) == 1)
                }
            }
        }
    }
}

impl Sketch {
    /// Applies a tool and solves. On failure the sketch is unchanged.
    pub fn apply_tool(&mut self, op: &ToolOp) -> Result<ToolReport, ToolError> {
        let backup = self.clone();
        let result = self.apply_unsolved(op).and_then(|()| {
            self.validate()?;
            self.solve()?;
            Ok(())
        });
        match result {
            Ok(()) => Ok(report(&backup, self)),
            Err(e) => {
                *self = backup;
                Err(e)
            }
        }
    }

    /// The sketch as it would be after the tool, without solving, and what changed; `None`
    /// when the tool does not apply. For previews.
    pub fn preview_tool(&self, op: &ToolOp) -> Option<(Sketch, ToolReport)> {
        let mut s = self.clone();
        s.apply_unsolved(op).ok()?;
        s.validate().ok()?;
        let r = report(self, &s);
        Some((s, r))
    }

    /// The piece trim would remove at `at`, as a polyline, for a preview.
    pub fn trim_preview(&self, curve: CurveId, at: Point2) -> Option<Vec<Point2>> {
        let plan = self.trim_plan(curve, at).ok()?;
        let s = plan.shape;
        let end = match s {
            Shape::Segment { .. } => 1.0,
            Shape::Arc { sweep, .. } => sweep,
            Shape::Circle { .. } => TAU,
        };
        if plan.whole {
            return Some(s.polyline(0.0, end));
        }
        let t0 = plan.lower.as_ref().map_or(0.0, |c| c.t);
        let mut t1 = plan.upper.as_ref().map_or(end, |c| c.t);
        if t1 <= t0 {
            t1 += TAU;
        }
        Some(s.polyline(t0, t1))
    }

    /// Where extend would take the end of `curve` nearest `at`: (from, to), for a preview.
    pub fn extend_preview(&self, curve: CurveId, at: Point2) -> Option<Vec<Point2>> {
        let plan = self.extend_plan(curve, at).ok()?;
        let c = self.curve(curve)?;
        match c {
            Curve::Line { .. } => Some(vec![self.point(plan.end)?, plan.to]),
            Curve::Arc { centre, .. } => {
                let centre = self.point(*centre)?;
                let from = self.point(plan.end)?;
                let r = centre.distance(from);
                let circle = Shape::Circle { c: centre, r };
                let t0 = circle.param(from);
                let mut t1 = circle.param(plan.to);
                if plan.at_start {
                    if t1 > t0 {
                        t1 -= TAU;
                    }
                } else if t1 < t0 {
                    t1 += TAU;
                }
                Some(circle.polyline(t0, t1))
            }
            Curve::Circle { .. } => None,
        }
    }

    fn apply_unsolved(&mut self, op: &ToolOp) -> Result<(), ToolError> {
        match op {
            ToolOp::Trim { curve, at } => self.trim(*curve, *at),
            ToolOp::Extend { curve, at } => self.extend(*curve, *at),
            ToolOp::Split { curve } => self.split(*curve),
            ToolOp::Fillet { corner, radius } => self.fillet(*corner, *radius),
            ToolOp::Chamfer {
                corner,
                distance,
                distance2,
            } => self.chamfer(*corner, *distance, distance2.unwrap_or(*distance)),
            ToolOp::Offset {
                curves,
                distance,
                side,
                corners,
                dimension,
            } => self.offset(curves, *distance, *side, *corners, *dimension),
            ToolOp::Mirror {
                curves,
                points,
                axis,
            } => self.mirror(curves, points, *axis),
            ToolOp::RectangularPattern {
                curves,
                points,
                columns,
                rows,
                dx,
                dy,
            } => self.rectangular_pattern(curves, points, *columns, *rows, *dx, *dy),
            ToolOp::CircularPattern {
                curves,
                points,
                centre,
                count,
                angle,
            } => self.circular_pattern(curves, points, *centre, *count, *angle),
            ToolOp::CentreRectangle { centre, corner } => self.centre_rectangle(*centre, *corner),
            ToolOp::ThreePointArc {
                start,
                through,
                end,
            } => self.three_point_arc(*start, *through, *end),
            ToolOp::TangentArc { from, to } => self.tangent_arc(*from, *to),
        }
    }

    // ----- shared helpers -----

    fn pos(&self, id: PointId) -> Result<Point2, ToolError> {
        self.point(id)
            .ok_or_else(|| SketchError::UnknownPoint(id).into())
    }

    fn curve_of(&self, id: CurveId) -> Result<Curve, ToolError> {
        self.curve(id)
            .cloned()
            .ok_or_else(|| SketchError::UnknownCurve(id).into())
    }

    /// The geometry of a curve.
    fn shape_of(&self, curve: &Curve) -> Option<Shape> {
        match curve {
            Curve::Line { start, end, .. } => Some(Shape::Segment {
                a: self.point(*start)?,
                b: self.point(*end)?,
            }),
            Curve::Circle { centre, radius, .. } => Some(Shape::Circle {
                c: self.point(*centre)?,
                r: *radius,
            }),
            Curve::Arc {
                centre, start, end, ..
            } => Shape::arc(self.point(*centre)?, self.point(*start)?, self.point(*end)?),
        }
    }

    fn shape(&self, id: CurveId) -> Result<Shape, ToolError> {
        let c = self.curve_of(id)?;
        match self.shape_of(&c) {
            Some(s) => Ok(s),
            None => refuse(format!("{id} has no length")),
        }
    }

    /// The other non-construction curves, with their geometry.
    fn cutters(&self, id: CurveId) -> Vec<Cutter> {
        self.curves()
            .filter(|(k, c)| *k != id && !c.is_construction())
            .filter_map(|(k, c)| Some((k, self.shape_of(c)?, ends(c))))
            .collect()
    }

    /// The constraint that keeps `point` on `curve`.
    fn on_curve(&self, point: PointId, curve: CurveId) -> Option<Constraint> {
        Some(if self.curve(curve)?.is_line() {
            Constraint::PointOnLine { point, line: curve }
        } else {
            Constraint::PointOnCircle { point, curve }
        })
    }

    /// Drops the constraints that measure a line's extent, after a tool changed it.
    fn drop_extent(&mut self, line: CurveId) {
        if !self.curve(line).is_some_and(Curve::is_line) {
            return;
        }
        self.retain_constraints(|c| match c {
            Constraint::Length { line: l, .. } | Constraint::Midpoint { line: l, .. } => *l != line,
            Constraint::Equal { a, b } => *a != line && *b != line,
            _ => true,
        });
    }

    /// Removes those of `points` that no curve uses any more.
    fn remove_orphans(&mut self, points: &[PointId]) -> Result<(), ToolError> {
        for p in points {
            if self.point(*p).is_some() && !self.curves().any(|(_, c)| c.points().contains(p)) {
                self.remove_point(*p)?;
            }
        }
        Ok(())
    }

    /// Where other curves cut `id`, inside it, in parameter order.
    fn cuts(&self, id: CurveId, shape: &Shape) -> Vec<Cut> {
        let mut cuts: Vec<Cut> = Vec::new();
        for (k, other, other_ends) in self.cutters(id) {
            for x in intersect::intersections(shape, &other) {
                let t = shape.param(x);
                if !shape.interior(t) {
                    continue;
                }
                let existing = other_ends.and_then(|(a, b)| {
                    [a, b]
                        .into_iter()
                        .find(|e| self.point(*e).is_some_and(|q| q.coincident(x)))
                });
                if let Some(c) = cuts.iter_mut().find(|c| c.at.coincident(x)) {
                    if !c.cutters.contains(&k) {
                        c.cutters.push(k);
                    }
                    if c.point.is_none() {
                        c.point = existing;
                    }
                } else {
                    cuts.push(Cut {
                        t,
                        at: x,
                        point: existing,
                        cutters: vec![k],
                    });
                }
            }
        }
        cuts.sort_by(|a, b| a.t.total_cmp(&b.t));
        cuts
    }

    /// The point at a cut: the cutting curve's endpoint, or a new point kept on the
    /// (first) cutting curve.
    fn cut_point(&mut self, cut: &Cut) -> Result<PointId, ToolError> {
        if let Some(p) = cut.point
            && self.point(p).is_some()
        {
            return Ok(p);
        }
        let p = self.add_point(cut.at)?;
        if let Some(k) = cut.cutters.first()
            && let Some(c) = self.on_curve(p, *k)
        {
            self.add_constraint(c)?;
        }
        Ok(p)
    }

    /// Rebuilds a line, arc or circle as the pieces between consecutive points of `chain`
    /// (cyclic for a circle), keeping those `keep` says. The first kept piece keeps the id.
    fn rebuild(&mut self, id: CurveId, chain: &[PointId], keep: &[bool]) -> Result<(), ToolError> {
        let curve = self.curve_of(id)?;
        let construction = curve.is_construction();
        let (cyclic, centre) = match &curve {
            Curve::Line { .. } => (false, None),
            Curve::Arc { centre, .. } => (false, Some(*centre)),
            Curve::Circle { centre, .. } => (true, Some(*centre)),
        };
        let n = if cyclic {
            chain.len()
        } else {
            chain.len().saturating_sub(1)
        };
        if n == 0 || keep.len() != n || (cyclic && n < 2) {
            return refuse("the curve cannot be cut there");
        }
        let piece = |i: usize| -> Option<(PointId, PointId)> {
            Some((*chain.get(i)?, *chain.get((i + 1) % chain.len())?))
        };
        let make = |s: PointId, e: PointId| match centre {
            None => Curve::Line {
                start: s,
                end: e,
                construction,
            },
            Some(c) => Curve::Arc {
                centre: c,
                start: s,
                end: e,
                construction,
            },
        };
        let kept = |i: usize| keep.get(i).copied().unwrap_or(false);
        let Some(primary) = (0..n).find(|i| kept(*i)) else {
            self.remove_curve(id)?;
            return Ok(());
        };
        let Some((ps, pe)) = piece(primary) else {
            return refuse("the curve cannot be cut there");
        };
        self.replace_curve(id, make(ps, pe))?;
        for i in (0..n).filter(|i| *i != primary && kept(*i)) {
            if let Some((s, e)) = piece(i) {
                self.add_curve(make(s, e))?;
            }
        }
        if centre.is_none() {
            self.drop_extent(id);
            // Every point of the other pieces stays on the first piece's line.
            let mut tie = BTreeSet::new();
            for i in (0..n).filter(|i| *i != primary && kept(*i)) {
                if let Some((s, e)) = piece(i) {
                    tie.insert(s);
                    tie.insert(e);
                }
            }
            tie.remove(&ps);
            tie.remove(&pe);
            for p in tie {
                self.add_constraint(Constraint::PointOnLine { point: p, line: id })?;
            }
        } else {
            // Pieces joined to the first one share its radius through their common points;
            // each run of pieces cut off from it keeps one point on its circle.
            let joined = |i: usize| -> bool {
                if cyclic {
                    let forward = (0..n)
                        .map(|k| (primary + k) % n)
                        .take_while(|j| kept(*j))
                        .any(|j| j == i);
                    let backward = (0..n)
                        .map(|k| (primary + n - k) % n)
                        .take_while(|j| kept(*j))
                        .any(|j| j == i);
                    forward || backward
                } else {
                    let (lo, hi) = (primary.min(i), primary.max(i));
                    (lo..=hi).all(kept)
                }
            };
            for i in (0..n).filter(|i| *i != primary && kept(*i) && !joined(*i)) {
                let previous_kept = if cyclic {
                    kept((i + n - 1) % n)
                } else {
                    i > 0 && kept(i - 1)
                };
                if !previous_kept && let Some((s, _)) = piece(i) {
                    self.add_constraint(Constraint::PointOnCircle {
                        point: s,
                        curve: id,
                    })?;
                }
            }
        }
        Ok(())
    }

    // ----- trim -----

    fn trim_plan(&self, id: CurveId, at: Point2) -> Result<TrimPlan, ToolError> {
        check_point("the trim point", at)?;
        let shape = self.shape(id)?;
        let cuts = self.cuts(id, &shape);
        let t = shape.param(at);
        let tc = match shape {
            Shape::Segment { .. } => t.clamp(0.0, 1.0),
            Shape::Arc { sweep, .. } => {
                if t <= sweep {
                    t
                } else if t - sweep < TAU - t {
                    sweep
                } else {
                    0.0
                }
            }
            Shape::Circle { .. } => t,
        };
        let below = cuts.iter().rev().find(|c| c.t <= tc).cloned();
        let above = cuts.iter().find(|c| c.t > tc).cloned();
        if let Shape::Circle { .. } = shape {
            if cuts.len() < 2 {
                return Ok(TrimPlan {
                    shape,
                    lower: None,
                    upper: None,
                    whole: true,
                });
            }
            let lower = below.or_else(|| cuts.last().cloned());
            let upper = above.or_else(|| cuts.first().cloned());
            return Ok(TrimPlan {
                shape,
                lower,
                upper,
                whole: false,
            });
        }
        let whole = below.is_none() && above.is_none();
        Ok(TrimPlan {
            shape,
            lower: below,
            upper: above,
            whole,
        })
    }

    fn trim(&mut self, id: CurveId, at: Point2) -> Result<(), ToolError> {
        let plan = self.trim_plan(id, at)?;
        let curve = self.curve_of(id)?;
        if plan.whole {
            self.remove_curve(id)?;
            return self.remove_orphans(&curve.points());
        }
        match curve {
            Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => {
                let mut chain = vec![start];
                if let Some(c) = &plan.lower {
                    chain.push(self.cut_point(c)?);
                }
                let removed = chain.len() - 1;
                if let Some(c) = &plan.upper {
                    chain.push(self.cut_point(c)?);
                }
                chain.push(end);
                let keep: Vec<bool> = (0..chain.len() - 1).map(|i| i != removed).collect();
                self.rebuild(id, &chain, &keep)?;
                self.remove_orphans(&[start, end])
            }
            Curve::Circle { .. } => {
                let (Some(lo), Some(hi)) = (&plan.lower, &plan.upper) else {
                    return refuse("the circle is not cut in two places");
                };
                let lo = self.cut_point(lo)?;
                let hi = self.cut_point(hi)?;
                if lo == hi {
                    return refuse("the circle is not cut in two places");
                }
                // Piece 0 runs from the cut below the click to the cut above it.
                self.rebuild(id, &[lo, hi], &[false, true])
            }
        }
    }

    // ----- split -----

    fn split(&mut self, only: Option<CurveId>) -> Result<(), ToolError> {
        let ids: Vec<CurveId> = match only {
            Some(k) => {
                let c = self.curve_of(k)?;
                if c.is_construction() {
                    return refuse("construction geometry is not split");
                }
                vec![k]
            }
            None => self
                .curves()
                .filter(|(_, c)| !c.is_construction())
                .map(|(k, _)| k)
                .collect(),
        };
        let all: Vec<Cutter> = self
            .curves()
            .filter(|(_, c)| !c.is_construction())
            .filter_map(|(k, c)| Some((k, self.shape_of(c)?, ends(c))))
            .collect();
        if all.len() > MAX_TOOL_CURVES {
            return refuse(format!(
                "a split looks at most {MAX_TOOL_CURVES} curves at once"
            ));
        }
        // Crossings as shared nodes: position and an existing endpoint to reuse.
        let mut nodes: Vec<(Point2, Option<PointId>)> = Vec::new();
        let mut cuts: BTreeMap<CurveId, Vec<(f64, usize)>> = BTreeMap::new();
        let find_end = |s: &Sketch, e: Option<(PointId, PointId)>, x: Point2| {
            e.and_then(|(a, b)| {
                [a, b]
                    .into_iter()
                    .find(|p| s.point(*p).is_some_and(|q| q.coincident(x)))
            })
        };
        for (i, (ki, si, ei)) in all.iter().enumerate() {
            for (kj, sj, ej) in all.iter().skip(i + 1) {
                if !(ids.contains(ki) || ids.contains(kj)) {
                    continue;
                }
                for x in intersect::intersections(si, sj) {
                    let (ti, tj) = (si.param(x), sj.param(x));
                    let (in_i, in_j) = (si.interior(ti), sj.interior(tj));
                    if !in_i && !in_j {
                        continue;
                    }
                    let existing = find_end(self, *ei, x).or_else(|| find_end(self, *ej, x));
                    let node = match nodes.iter().position(|(p, _)| p.coincident(x)) {
                        Some(n) => {
                            if let Some(slot) = nodes.get_mut(n)
                                && slot.1.is_none()
                            {
                                slot.1 = existing;
                            }
                            n
                        }
                        None => {
                            nodes.push((x, existing));
                            nodes.len() - 1
                        }
                    };
                    if in_i {
                        cuts.entry(*ki).or_default().push((ti, node));
                    }
                    if in_j {
                        cuts.entry(*kj).or_default().push((tj, node));
                    }
                }
            }
        }
        let mut node_points: Vec<Option<PointId>> = nodes.iter().map(|(_, e)| *e).collect();
        for (curve, mut list) in cuts {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            list.dedup_by_key(|(_, n)| *n);
            let c = self.curve_of(curve)?;
            if matches!(c, Curve::Circle { .. }) && list.len() < 2 {
                continue;
            }
            let mut inner = Vec::with_capacity(list.len());
            for (_, n) in &list {
                let p = match node_points.get(*n).copied().flatten() {
                    Some(p) => p,
                    None => {
                        let at = nodes.get(*n).map(|(p, _)| *p).unwrap_or_default();
                        let p = self.add_point(at)?;
                        if let Some(slot) = node_points.get_mut(*n) {
                            *slot = Some(p);
                        }
                        p
                    }
                };
                if !inner.contains(&p) {
                    inner.push(p);
                }
            }
            let chain: Vec<PointId> = match c {
                Curve::Line { start, end, .. } | Curve::Arc { start, end, .. } => {
                    let mut v = vec![start];
                    v.extend(inner.iter().filter(|p| **p != start && **p != end));
                    v.push(end);
                    v
                }
                Curve::Circle { .. } => inner,
            };
            let pieces = match c {
                Curve::Circle { .. } => chain.len(),
                _ => chain.len().saturating_sub(1),
            };
            if pieces < 2 {
                continue;
            }
            self.rebuild(curve, &chain, &vec![true; pieces])?;
        }
        Ok(())
    }

    // ----- extend -----

    fn extend_plan(&self, id: CurveId, at: Point2) -> Result<ExtendPlan, ToolError> {
        check_point("the extend point", at)?;
        let curve = self.curve_of(id)?;
        let Some((start, end)) = ends(&curve) else {
            return refuse("a circle has no end to extend");
        };
        let (s, e) = (self.pos(start)?, self.pos(end)?);
        let at_start = at.distance(s) < at.distance(e);
        let shape = self.shape(id)?;
        let mut best: Option<(f64, Point2, CurveId, Option<PointId>)> = None;
        for (k, other, other_ends) in self.cutters(id) {
            for x in intersect::carrier_intersections(&shape, &other) {
                if !other.on(other.param(x)) {
                    continue;
                }
                let how_far = match shape {
                    Shape::Segment { a, b } => {
                        let (from, back) = if at_start { (a, b) } else { (b, a) };
                        let Some(u) = (from - back).normalized() else {
                            continue;
                        };
                        let d = (x - from).dot(u);
                        (d > LINEAR_TOLERANCE).then_some(d)
                    }
                    Shape::Arc { c, r, a0, sweep } => {
                        let th = (x - c).angle();
                        let d = if at_start {
                            wrap(a0 - th)
                        } else {
                            wrap(th - (a0 + sweep))
                        };
                        let tol = LINEAR_TOLERANCE / r;
                        (d > tol && d < TAU - sweep - tol).then_some(d)
                    }
                    Shape::Circle { .. } => None,
                };
                let Some(d) = how_far else { continue };
                if best.as_ref().is_none_or(|b| d < b.0) {
                    let existing = other_ends.and_then(|(p, q)| {
                        [p, q]
                            .into_iter()
                            .find(|e| self.point(*e).is_some_and(|v| v.coincident(x)))
                    });
                    best = Some((d, x, k, existing));
                }
            }
        }
        let Some((_, to, target, existing)) = best else {
            return refuse("nothing lies ahead of that end to extend to");
        };
        let moving = if at_start { start } else { end };
        let shared = self
            .curves()
            .any(|(k, c)| k != id && c.points().contains(&moving))
            || self.constraints().any(|(_, c)| {
                matches!(c, Constraint::Coincident { a, b } if *a == moving || *b == moving)
            });
        if shared {
            return refuse("that end is joined to other geometry; extend a free end");
        }
        Ok(ExtendPlan {
            end: moving,
            at_start,
            to,
            target,
            existing,
        })
    }

    fn extend(&mut self, id: CurveId, at: Point2) -> Result<(), ToolError> {
        let plan = self.extend_plan(id, at)?;
        let curve = self.curve_of(id)?;
        let p = match plan.existing {
            Some(p) => p,
            None => {
                let p = self.add_point(plan.to)?;
                if let Some(c) = self.on_curve(p, plan.target) {
                    self.add_constraint(c)?;
                }
                p
            }
        };
        self.replace_curve(id, with_end(&curve, plan.at_start, p))?;
        self.drop_extent(id);
        self.remove_orphans(&[plan.end])
    }

    // ----- fillet and chamfer -----

    /// The two lines that meet at `corner` (directly or through coincident points): each
    /// line, its end at the corner, its far end and whether the corner end is its start.
    fn corner_lines(
        &self,
        corner: PointId,
    ) -> Result<[(CurveId, PointId, PointId, bool); 2], ToolError> {
        self.pos(corner)?;
        let mut group = BTreeSet::from([corner]);
        for (_, c) in self.constraints() {
            if let Constraint::Coincident { a, b } = c {
                if *a == corner {
                    group.insert(*b);
                } else if *b == corner {
                    group.insert(*a);
                }
            }
        }
        let mut lines = Vec::new();
        // Construction lines at the corner (a diagonal) stay on the kept corner point.
        for (id, c) in self.curves().filter(|(_, c)| !c.is_construction()) {
            match c {
                Curve::Line { start, end, .. } => {
                    if group.contains(start) {
                        lines.push((id, *start, *end, true));
                    } else if group.contains(end) {
                        lines.push((id, *end, *start, false));
                    }
                }
                Curve::Arc { start, end, .. } => {
                    if group.contains(start) || group.contains(end) {
                        return refuse("fillets and chamfers join two lines; an arc ends here");
                    }
                }
                Curve::Circle { .. } => {}
            }
        }
        match lines.as_slice() {
            [a, b] => Ok([*a, *b]),
            _ => refuse(format!(
                "a corner where exactly two lines meet is needed; {} lines end here",
                lines.len()
            )),
        }
    }

    /// The corner, the unit directions from it along each line and the lines' lengths.
    fn corner_frame(
        &self,
        lines: &[(CurveId, PointId, PointId, bool); 2],
    ) -> Result<(Point2, [Vec2; 2], [f64; 2]), ToolError> {
        let [(_, v1, f1, _), (_, v2, f2, _)] = *lines;
        let (v1p, v2p) = (self.pos(v1)?, self.pos(v2)?);
        let d1 = self.pos(f1)? - v1p;
        let d2 = self.pos(f2)? - v2p;
        let (Some(u1), Some(u2)) = (d1.normalized(), d2.normalized()) else {
            return refuse("a line at the corner has no length");
        };
        if u1.cross(u2).abs() < 1e-9 {
            return refuse("the lines are in line with each other: there is no corner");
        }
        Ok((v1p.lerp(v2p, 0.5), [u1, u2], [d1.length(), d2.length()]))
    }

    /// Replaces the corner end of each line with a new point; keeps the corner on both.
    fn cut_corner(
        &mut self,
        lines: &[(CurveId, PointId, PointId, bool); 2],
        at: [Point2; 2],
    ) -> Result<[PointId; 2], ToolError> {
        let mut new = [PointId(0); 2];
        for (k, (line, v, far, v_is_start)) in lines.iter().enumerate() {
            let p = self.add_point(at[k])?;
            let curve = self.curve_of(*line)?;
            self.replace_curve(*line, with_end(&curve, *v_is_start, p))?;
            // The line's length dimension still measures to the sharp corner, which stays:
            // it becomes a distance from the far end to the corner, with the same id (so an
            // expression driving it keeps working).
            let (line, v, far) = (*line, *v, *far);
            self.rewrite_constraints(|c| match c {
                Constraint::Length { line: l, value } if *l == line => Some(Constraint::Distance {
                    a: far,
                    b: v,
                    value: *value,
                }),
                _ => None,
            });
            self.drop_extent(line);
            let line = &line;
            let v = &v;
            self.add_constraint(Constraint::PointOnLine {
                point: *v,
                line: *line,
            })?;
            new[k] = p;
        }
        Ok(new)
    }

    fn fillet(&mut self, corner: PointId, radius: f64) -> Result<(), ToolError> {
        check_length("the radius", radius)?;
        let lines = self.corner_lines(corner)?;
        let (v, [u1, u2], [l1, l2]) = self.corner_frame(&lines)?;
        // The corner's half angle θ: the tangent points are r / tan θ from the corner and
        // the centre r / sin θ along the bisector.
        let half = 0.5 * u1.dot(u2).clamp(-1.0, 1.0).acos();
        let along = radius / half.tan();
        if along >= l1.min(l2) - LINEAR_TOLERANCE {
            return refuse(format!(
                "a radius of {radius} mm needs {along:.3} mm of each line; the lines are shorter"
            ));
        }
        let Some(bisector) = (u1 + u2).normalized() else {
            return refuse("the lines are in line with each other: there is no corner");
        };
        let centre_at = v + bisector * (radius / half.sin());
        let t1 = v + u1 * along;
        let t2 = v + u2 * along;
        let [p1, p2] = self.cut_corner(&lines, [t1, t2])?;
        let centre = self.add_point(centre_at)?;
        let ccw = (t1 - centre_at).cross(t2 - centre_at) > 0.0;
        let arc = if ccw {
            self.add_arc(centre, p1, p2)?
        } else {
            self.add_arc(centre, p2, p1)?
        };
        for (line, ..) in lines {
            self.add_constraint(Constraint::Tangent { a: line, b: arc })?;
        }
        self.add_constraint(Constraint::Radius {
            curve: arc,
            value: radius,
        })?;
        Ok(())
    }

    fn chamfer(&mut self, corner: PointId, d1: f64, d2: f64) -> Result<(), ToolError> {
        check_length("the chamfer distance", d1)?;
        check_length("the chamfer distance", d2)?;
        let lines = self.corner_lines(corner)?;
        let (v, [u1, u2], [l1, l2]) = self.corner_frame(&lines)?;
        if d1 >= l1 - LINEAR_TOLERANCE || d2 >= l2 - LINEAR_TOLERANCE {
            return refuse("the chamfer is longer than a line at the corner");
        }
        let [p1, p2] = self.cut_corner(&lines, [v + u1 * d1, v + u2 * d2])?;
        self.add_line(p1, p2)?;
        let [(_, v1, ..), (_, v2, ..)] = lines;
        self.add_constraint(Constraint::Distance {
            a: v1,
            b: p1,
            value: d1,
        })?;
        self.add_constraint(Constraint::Distance {
            a: v2,
            b: p2,
            value: d2,
        })?;
        Ok(())
    }

    // ----- offset -----

    /// Non-construction lines and arcs with an end at `p`.
    fn ends_at(&self, p: Point2) -> Vec<CurveId> {
        self.curves()
            .filter(|(_, c)| !c.is_construction())
            .filter(|(_, c)| {
                ends(c).is_some_and(|(s, e)| {
                    [s, e]
                        .into_iter()
                        .any(|q| self.point(q).is_some_and(|q| q.coincident(p)))
                })
            })
            .map(|(k, _)| k)
            .collect()
    }

    /// The chain `seed` is part of: neighbours are followed through points where exactly
    /// two curves end.
    fn chain_from(&self, seed: CurveId) -> Result<Vec<CurveId>, ToolError> {
        let curve = self.curve_of(seed)?;
        let Some((s, e)) = ends(&curve) else {
            return Ok(vec![seed]);
        };
        let mut found = vec![seed];
        for start in [s, e] {
            let mut current = seed;
            let mut at = start;
            for _ in 0..MAX_TOOL_CURVES {
                let p = self.pos(at)?;
                let here = self.ends_at(p);
                if here.len() != 2 {
                    break;
                }
                let Some(next) = here.into_iter().find(|k| *k != current) else {
                    break;
                };
                if found.contains(&next) {
                    break;
                }
                found.push(next);
                let Some((ns, ne)) = self.curve(next).and_then(ends) else {
                    break;
                };
                at = if self.pos(ns)?.coincident(p) { ne } else { ns };
                current = next;
            }
        }
        Ok(found)
    }

    /// Orders lines and arcs into one chain.
    fn chain(&self, ids: &[CurveId]) -> Result<Chain, ToolError> {
        let mut list: Vec<CurveId> = Vec::new();
        for id in ids {
            if !list.contains(id) {
                list.push(*id);
            }
        }
        if list.len() > MAX_TOOL_CURVES {
            return refuse(format!("an offset takes at most {MAX_TOOL_CURVES} curves"));
        }
        // Each curve as (id, start vertex, end vertex), vertices merged by position.
        let mut vertices: Vec<Point2> = Vec::new();
        let mut vertex = |p: Point2| match vertices.iter().position(|v| v.coincident(p)) {
            Some(i) => i,
            None => {
                vertices.push(p);
                vertices.len() - 1
            }
        };
        let mut items = Vec::new();
        for id in &list {
            let c = self.curve_of(*id)?;
            let Some((s, e)) = ends(&c) else {
                return refuse("a circle is offset on its own");
            };
            items.push((*id, vertex(self.pos(s)?), vertex(self.pos(e)?)));
        }
        let mut degree: BTreeMap<usize, usize> = BTreeMap::new();
        for (_, a, b) in &items {
            *degree.entry(*a).or_default() += 1;
            *degree.entry(*b).or_default() += 1;
        }
        if degree.values().any(|d| *d > 2) {
            return refuse("the curves branch: offset one chain at a time");
        }
        let open_ends: Vec<usize> = degree
            .iter()
            .filter(|(_, d)| **d == 1)
            .map(|(v, _)| *v)
            .collect();
        let closed = open_ends.is_empty();
        if !(closed || open_ends.len() == 2) {
            return refuse("the curves do not form one chain");
        }
        if closed && items.len() < 2 {
            return refuse("the curve closes on itself");
        }
        let mut current = match (open_ends.first(), items.first()) {
            (Some(v), _) => *v,
            (None, Some((_, a, _))) => *a,
            (None, None) => return refuse("nothing to offset"),
        };
        let mut used = vec![false; items.len()];
        let mut elems = Vec::new();
        for _ in 0..items.len() {
            let Some(k) = (0..items.len()).find(|k| {
                !used[*k]
                    && items
                        .get(*k)
                        .is_some_and(|(_, a, b)| *a == current || *b == current)
            }) else {
                return refuse("the curves do not form one chain");
            };
            used[k] = true;
            let Some(&(id, a, b)) = items.get(k) else {
                return refuse("the curves do not form one chain");
            };
            let reversed = a != current;
            current = if reversed { a } else { b };
            let c = self.curve_of(id)?;
            let elem = match &c {
                Curve::Line { start, end, .. } => {
                    let (p, q) = (self.pos(*start)?, self.pos(*end)?);
                    if reversed {
                        Elem::Line { a: q, b: p }
                    } else {
                        Elem::Line { a: p, b: q }
                    }
                }
                Curve::Arc { .. } => {
                    let Some(Shape::Arc { c, r, a0, sweep }) = self.shape_of(&c) else {
                        return refuse(format!("{id} has no radius"));
                    };
                    if reversed {
                        Elem::Arc {
                            c,
                            r,
                            t0: a0 + sweep,
                            sw: -sweep,
                        }
                    } else {
                        Elem::Arc {
                            c,
                            r,
                            t0: a0,
                            sw: sweep,
                        }
                    }
                }
                Curve::Circle { .. } => return refuse("a circle is offset on its own"),
            };
            let Some((s, e)) = ends(&c) else {
                return refuse("a circle is offset on its own");
            };
            let centre = match &c {
                Curve::Arc { centre, .. } => Some(*centre),
                _ => None,
            };
            elems.push(ChainElem {
                curve: id,
                elem,
                centre,
                end_point: if reversed { s } else { e },
            });
        }
        Ok(Chain { elems, closed })
    }

    fn offset(
        &mut self,
        curves: &[CurveId],
        distance: f64,
        side: Option<Point2>,
        corners: OffsetCorners,
        dimension: bool,
    ) -> Result<(), ToolError> {
        check_length("the offset distance", distance.abs())?;
        if let Some(p) = side {
            check_point("the side point", p)?;
        }
        let ids = match curves {
            [] => return refuse("choose the curves to offset"),
            [one] => self.chain_from(*one)?,
            many => many.to_vec(),
        };
        // A circle: a concentric circle.
        if let [one] = ids.as_slice()
            && let Curve::Circle { centre, radius, .. } = self.curve_of(*one)?
        {
            let c = self.pos(centre)?;
            let outward = match side {
                Some(p) => p.distance(c) > radius,
                None => distance > 0.0,
            };
            let r = if outward {
                radius + distance.abs()
            } else {
                radius - distance.abs()
            };
            check_length("the offset circle's radius", r)?;
            let new = self.add_circle(centre, r)?;
            if dimension {
                self.add_constraint(Constraint::Radius {
                    curve: new,
                    value: r,
                })?;
            }
            return Ok(());
        }
        let chain = self.chain(&ids)?;
        let elems: Vec<Elem> = chain.elems.iter().map(|e| e.elem).collect();
        let n = elems.len();
        // The signed distance to the left of the chain's direction.
        let d = match side {
            Some(p) => {
                let nearest = elems
                    .iter()
                    .min_by(|a, b| a.distance_to(p).total_cmp(&b.distance_to(p)));
                distance.abs() * nearest.map_or(1.0, |e| e.side_of(p))
            }
            None if chain.closed => {
                // Outward is to the right of a counter-clockwise chain.
                let area: f64 = elems
                    .iter()
                    .map(|e| {
                        let (a, b) = (e.start(), e.end());
                        let chord = 0.5 * (a.x * b.y - b.x * a.y);
                        match *e {
                            Elem::Line { .. } => chord,
                            Elem::Arc { r, sw, .. } => chord + 0.5 * r * r * (sw - sw.sin()),
                        }
                    })
                    .sum();
                if area > 0.0 { -distance } else { distance }
            }
            None => distance,
        };
        let mut offs = Vec::with_capacity(n);
        for e in &elems {
            match e.offset(d) {
                Some(o) => offs.push(o),
                None => return refuse("the offset is larger than an arc's radius"),
            }
        }
        let mut starts: Vec<Point2> = offs.iter().map(Elem::start).collect();
        let mut finishes: Vec<Point2> = offs.iter().map(Elem::end).collect();
        let joins = if chain.closed { n } else { n - 1 };
        let mut round: Vec<usize> = Vec::new();
        #[allow(clippy::needless_range_loop)] // k indexes four lists in step
        for k in 0..joins {
            let j = (k + 1) % n;
            let (Some(ek), Some(ej), Some(ok), Some(oj)) =
                (elems.get(k), elems.get(j), offs.get(k), offs.get(j))
            else {
                continue;
            };
            let (tin, tout) = (ek.tangent_end(), ej.tangent_start());
            let turn = tin.cross(tout);
            if turn.abs() <= SMOOTH_JOIN && tin.dot(tout) > 0.0 {
                let m = finishes[k].lerp(starts[j], 0.5);
                finishes[k] = m;
                starts[j] = m;
                continue;
            }
            let gap = turn * d < 0.0;
            if gap && corners == OffsetCorners::Round {
                round.push(k);
                continue;
            }
            let v = ek.end();
            let best = intersect::carrier_intersections(&ok.carrier(), &oj.carrier())
                .into_iter()
                .min_by(|a, b| a.distance(v).total_cmp(&b.distance(v)));
            match best {
                Some(x) => {
                    finishes[k] = x;
                    starts[j] = x;
                }
                None if gap => round.push(k),
                None => return refuse("the offset is too large for this shape"),
            }
        }
        // Each piece must still run the way its original does.
        for (i, (e, o)) in elems.iter().zip(&offs).enumerate() {
            let (s, f) = (starts[i], finishes[i]);
            let ok = match (*e, *o) {
                (Elem::Line { a, b }, _) => {
                    (f - s).dot(b - a) > 0.0 && s.distance(f) > LINEAR_TOLERANCE
                }
                (Elem::Arc { sw, .. }, Elem::Arc { c, r, .. }) => {
                    let (ts, tf) = ((s - c).angle(), (f - c).angle());
                    let sweep = if sw > 0.0 {
                        wrap(tf - ts)
                    } else {
                        wrap(ts - tf)
                    };
                    sweep > LINEAR_TOLERANCE / r && (sweep - sw.abs()).abs() < PI
                }
                _ => false,
            };
            if !ok {
                return refuse("the offset is too large for this shape");
            }
        }
        // Points: joins are shared unless a round corner sits between.
        let direct = |k: usize| !round.contains(&k);
        let mut sid: Vec<PointId> = Vec::with_capacity(n);
        let mut fid: Vec<PointId> = Vec::with_capacity(n);
        for i in 0..n {
            let s = match fid.last() {
                Some(prev) if direct(i - 1) => *prev,
                _ => self.add_point(starts[i])?,
            };
            let f = match sid.first() {
                Some(first) if chain.closed && i == n - 1 && direct(n - 1) => *first,
                _ => self.add_point(finishes[i])?,
            };
            sid.push(s);
            fid.push(f);
        }
        // Curves.
        let mut new_ids = Vec::with_capacity(n);
        for (i, ce) in chain.elems.iter().enumerate() {
            let id = match (ce.elem, ce.centre) {
                (Elem::Line { .. }, _) => self.add_line(sid[i], fid[i])?,
                (Elem::Arc { sw, .. }, Some(centre)) => {
                    if sw > 0.0 {
                        self.add_arc(centre, sid[i], fid[i])?
                    } else {
                        self.add_arc(centre, fid[i], sid[i])?
                    }
                }
                (Elem::Arc { .. }, None) => return refuse("an arc has no centre"),
            };
            new_ids.push(id);
        }
        let is_line = |i: usize| matches!(elems.get(i), Some(Elem::Line { .. }));
        let smooth = |k: usize| {
            let j = (k + 1) % n;
            match (elems.get(k), elems.get(j)) {
                (Some(a), Some(b)) => {
                    let (tin, tout) = (a.tangent_end(), b.tangent_start());
                    tin.cross(tout).abs() <= SMOOTH_JOIN && tin.dot(tout) > 0.0
                }
                _ => false,
            }
        };
        // Ties: lines parallel at a distance, arcs concentric and tangent to a line.
        for (i, ce) in chain.elems.iter().enumerate() {
            let new = new_ids[i];
            match ce.elem {
                Elem::Line { .. } => {
                    self.add_constraint(Constraint::Parallel {
                        a: ce.curve,
                        b: new,
                    })?;
                    if dimension {
                        self.add_constraint(Constraint::PointLineDistance {
                            point: sid[i],
                            line: ce.curve,
                            value: d.abs(),
                        })?;
                    }
                }
                Elem::Arc { .. } => {
                    let prev = (i > 0 || chain.closed).then(|| (i + n - 1) % n);
                    let next = (i + 1 < n || chain.closed).then(|| (i + 1) % n);
                    // Tangent to each line it joins smoothly; those fix its radius.
                    let lines: Vec<usize> = [
                        prev.filter(|p| is_line(*p) && smooth(*p) && direct(*p)),
                        next.filter(|q| is_line(*q) && smooth(i) && direct(i)),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    for l in &lines {
                        self.add_constraint(Constraint::Tangent {
                            a: new_ids[*l],
                            b: new,
                        })?;
                    }
                    if lines.is_empty()
                        && dimension
                        && let Elem::Arc { r, .. } = offs[i]
                    {
                        self.add_constraint(Constraint::Radius {
                            curve: new,
                            value: r,
                        })?;
                    }
                }
            }
        }
        // Round corners: an arc around the original corner.
        for k in round {
            let j = (k + 1) % n;
            let Some(ce) = chain.elems.get(k) else {
                continue;
            };
            let v = ce.end_point;
            let vp = self.pos(v)?;
            let ccw = (finishes[k] - vp).cross(starts[j] - vp) > 0.0;
            let arc = if ccw {
                self.add_arc(v, fid[k], sid[j])?
            } else {
                self.add_arc(v, sid[j], fid[k])?
            };
            let lines: Vec<usize> = [k, j].into_iter().filter(|e| is_line(*e)).collect();
            for l in &lines {
                self.add_constraint(Constraint::Tangent {
                    a: new_ids[*l],
                    b: arc,
                })?;
            }
            if lines.is_empty() {
                self.add_constraint(Constraint::Radius {
                    curve: arc,
                    value: d.abs(),
                })?;
            }
        }
        Ok(())
    }

    // ----- mirror and patterns -----

    /// The curves and the points to copy: the curves' points and the extra points.
    fn copy_set(
        &self,
        curves: &[CurveId],
        points: &[PointId],
        skip: Option<CurveId>,
    ) -> Result<(Vec<CurveId>, Vec<PointId>), ToolError> {
        let mut cs: Vec<CurveId> = Vec::new();
        for c in curves {
            self.curve_of(*c)?;
            if Some(*c) != skip && !cs.contains(c) {
                cs.push(*c);
            }
        }
        let mut ps: BTreeSet<PointId> = BTreeSet::new();
        for c in &cs {
            if let Some(curve) = self.curve(*c) {
                ps.extend(curve.points());
            }
        }
        for p in points {
            self.pos(*p)?;
            ps.insert(*p);
        }
        if cs.is_empty() && ps.is_empty() {
            return refuse("choose the geometry to copy");
        }
        Ok((cs, ps.into_iter().collect()))
    }

    /// Adds the copied curves, with `map` from old to new point ids.
    fn copy_curves(
        &mut self,
        curves: &[CurveId],
        map: &BTreeMap<PointId, PointId>,
        mirrored: bool,
    ) -> Result<BTreeMap<CurveId, CurveId>, ToolError> {
        let mut out = BTreeMap::new();
        let m = |p: &PointId| map.get(p).copied().unwrap_or(*p);
        for c in curves {
            let curve = self.curve_of(*c)?;
            let copy = match curve {
                Curve::Line {
                    start,
                    end,
                    construction,
                } => Curve::Line {
                    start: m(&start),
                    end: m(&end),
                    construction,
                },
                Curve::Circle {
                    centre,
                    radius,
                    construction,
                } => Curve::Circle {
                    centre: m(&centre),
                    radius,
                    construction,
                },
                // A reflection turns counter-clockwise into clockwise: swap the ends.
                Curve::Arc {
                    centre,
                    start,
                    end,
                    construction,
                } => Curve::Arc {
                    centre: m(&centre),
                    start: if mirrored { m(&end) } else { m(&start) },
                    end: if mirrored { m(&start) } else { m(&end) },
                    construction,
                },
            };
            if copy != curve {
                out.insert(*c, self.add_curve(copy)?);
            }
        }
        Ok(out)
    }

    fn mirror(
        &mut self,
        curves: &[CurveId],
        points: &[PointId],
        axis: CurveId,
    ) -> Result<(), ToolError> {
        let Curve::Line { start, end, .. } = self.curve_of(axis)? else {
            return refuse("mirror in a line");
        };
        let (a, b) = (self.pos(start)?, self.pos(end)?);
        let Some(u) = (b - a).normalized() else {
            return refuse("the mirror line has no length");
        };
        let (cs, ps) = self.copy_set(curves, points, Some(axis))?;
        let mut map = BTreeMap::new();
        for p in &ps {
            let q = self.pos(*p)?;
            let foot = a + u * (q - a).dot(u);
            if foot.coincident(q) {
                // On the mirror line: shared by both halves, and kept on the line.
                map.insert(*p, *p);
                let already = *p == start
                    || *p == end
                    || self.constraints().any(|(_, c)| {
                        matches!(c, Constraint::PointOnLine { point, line } if point == p && *line == axis)
                    });
                if !already {
                    self.add_constraint(Constraint::PointOnLine {
                        point: *p,
                        line: axis,
                    })?;
                }
                continue;
            }
            let image = foot + (foot - q);
            check_point("a mirrored point", image)?;
            let new = self.add_point(image)?;
            self.add_constraint(Constraint::Symmetric {
                a: *p,
                b: new,
                line: axis,
            })?;
            map.insert(*p, new);
        }
        let copies = self.copy_curves(&cs, &map, true)?;
        for (old, new) in copies {
            if let Some(Curve::Circle { .. }) = self.curve(old) {
                self.add_constraint(Constraint::Equal { a: old, b: new })?;
            }
        }
        Ok(())
    }

    /// One pattern copy: points moved by `motion`, curves copied, and the constraints among
    /// the copied entities copied too, so the copy keeps the original's shape.
    fn pattern_copy(
        &mut self,
        cs: &[CurveId],
        ps: &[PointId],
        motion: Motion,
    ) -> Result<(), ToolError> {
        let mut map = BTreeMap::new();
        for p in ps {
            let q = self.pos(*p)?;
            let image = motion.apply(q);
            if image.coincident(q) {
                map.insert(*p, *p);
                continue;
            }
            check_point("a pattern copy", image)?;
            map.insert(*p, self.add_point(image)?);
        }
        let curve_map = self.copy_curves(cs, &map, false)?;
        let point_set: BTreeSet<PointId> = ps.iter().copied().collect();
        let turns = motion.axis_turns();
        let originals: Vec<Constraint> = self
            .constraints()
            .map(|(_, c)| c.clone())
            .filter(|c| {
                c.point_refs().iter().all(|p| point_set.contains(p))
                    && c.curve_refs().iter().all(|k| cs.contains(k))
            })
            .collect();
        let mp = |p: PointId| map.get(&p).copied().unwrap_or(p);
        let mc = |k: CurveId| curve_map.get(&k).copied().unwrap_or(k);
        for c in originals {
            let copy = match c {
                Constraint::Horizontal { line } | Constraint::Vertical { line } => {
                    let horizontal = matches!(c, Constraint::Horizontal { .. });
                    match turns {
                        None => continue,
                        Some(swap) => {
                            if horizontal != swap {
                                Constraint::Horizontal { line: mc(line) }
                            } else {
                                Constraint::Vertical { line: mc(line) }
                            }
                        }
                    }
                }
                Constraint::Fix { point, at } => Constraint::Fix {
                    point: mp(point),
                    at: motion.apply(at),
                },
                Constraint::Coincident { a, b } => Constraint::Coincident { a: mp(a), b: mp(b) },
                Constraint::PointOnLine { point, line } => Constraint::PointOnLine {
                    point: mp(point),
                    line: mc(line),
                },
                Constraint::PointOnCircle { point, curve } => Constraint::PointOnCircle {
                    point: mp(point),
                    curve: mc(curve),
                },
                Constraint::Parallel { a, b } => Constraint::Parallel { a: mc(a), b: mc(b) },
                Constraint::Perpendicular { a, b } => {
                    Constraint::Perpendicular { a: mc(a), b: mc(b) }
                }
                Constraint::Tangent { a, b } => Constraint::Tangent { a: mc(a), b: mc(b) },
                Constraint::Equal { a, b } => Constraint::Equal { a: mc(a), b: mc(b) },
                Constraint::Midpoint { point, line } => Constraint::Midpoint {
                    point: mp(point),
                    line: mc(line),
                },
                Constraint::Symmetric { a, b, line } => Constraint::Symmetric {
                    a: mp(a),
                    b: mp(b),
                    line: mc(line),
                },
                Constraint::Distance { a, b, value } => Constraint::Distance {
                    a: mp(a),
                    b: mp(b),
                    value,
                },
                Constraint::PointLineDistance { point, line, value } => {
                    Constraint::PointLineDistance {
                        point: mp(point),
                        line: mc(line),
                        value,
                    }
                }
                Constraint::Length { line, value } => Constraint::Length {
                    line: mc(line),
                    value,
                },
                Constraint::Radius { curve, value } => Constraint::Radius {
                    curve: mc(curve),
                    value,
                },
                Constraint::Diameter { curve, value } => Constraint::Diameter {
                    curve: mc(curve),
                    value,
                },
                Constraint::Angle { a, b, value } => Constraint::Angle {
                    a: mc(a),
                    b: mc(b),
                    value,
                },
            };
            if copy != c {
                self.add_constraint(copy)?;
            }
        }
        Ok(())
    }

    fn rectangular_pattern(
        &mut self,
        curves: &[CurveId],
        points: &[PointId],
        columns: u32,
        rows: u32,
        dx: f64,
        dy: f64,
    ) -> Result<(), ToolError> {
        if columns == 0 || rows == 0 {
            return refuse("a pattern needs at least one column and one row");
        }
        let copies = u64::from(columns) * u64::from(rows) - 1;
        if copies == 0 {
            return refuse("a pattern of one makes no copies");
        }
        if copies > MAX_PATTERN_COPIES as u64 {
            return refuse(format!(
                "a pattern makes at most {MAX_PATTERN_COPIES} copies"
            ));
        }
        for (what, v, n) in [("dx", dx, columns), ("dy", dy, rows)] {
            if !v.is_finite() || v.abs() > MAX_COORDINATE {
                return refuse(format!("`{what}` must be a finite length"));
            }
            if n > 1 && v.abs() <= LINEAR_TOLERANCE {
                return refuse(format!("`{what}` must not be 0 with more than one copy"));
            }
        }
        let (cs, ps) = self.copy_set(curves, points, None)?;
        for j in 0..rows {
            for i in 0..columns {
                if i == 0 && j == 0 {
                    continue;
                }
                let v = Vec2::new(dx * f64::from(i), dy * f64::from(j));
                self.pattern_copy(&cs, &ps, Motion::Translate(v))?;
            }
        }
        Ok(())
    }

    fn circular_pattern(
        &mut self,
        curves: &[CurveId],
        points: &[PointId],
        centre: Point2,
        count: u32,
        angle: f64,
    ) -> Result<(), ToolError> {
        check_point("the pattern centre", centre)?;
        if count < 2 || count as usize > MAX_PATTERN_COPIES + 1 {
            return refuse(format!(
                "a circular pattern has 2 to {} instances",
                MAX_PATTERN_COPIES + 1
            ));
        }
        if !(angle.is_finite() && angle.abs() > 1e-9 && angle.abs() <= TAU + 1e-9) {
            return refuse("the pattern angle must be above 0 and at most a full turn");
        }
        let full = (angle.abs() - TAU).abs() <= 1e-9;
        let step = if full {
            angle.signum() * TAU / f64::from(count)
        } else {
            angle / f64::from(count - 1)
        };
        let (cs, ps) = self.copy_set(curves, points, None)?;
        for k in 1..count {
            self.pattern_copy(
                &cs,
                &ps,
                Motion::Rotate {
                    centre,
                    angle: step * f64::from(k),
                },
            )?;
        }
        Ok(())
    }

    // ----- drawing helpers -----

    fn centre_rectangle(&mut self, centre: Point2, corner: Point2) -> Result<(), ToolError> {
        check_point("the centre", centre)?;
        check_point("the corner", corner)?;
        let h = corner - centre;
        if h.x.abs() <= LINEAR_TOLERANCE || h.y.abs() <= LINEAR_TOLERANCE {
            return refuse("a rectangle needs a width and a height");
        }
        let a = centre - h;
        let b = corner;
        check_point("the opposite corner", a)?;
        let corners = [a, Point2::new(b.x, a.y), b, Point2::new(a.x, b.y)];
        let mut pts = Vec::with_capacity(4);
        for c in corners {
            pts.push(self.add_point(c)?);
        }
        for k in 0..4 {
            let line = self.add_line(pts[k], pts[(k + 1) % 4])?;
            self.add_constraint(if k % 2 == 0 {
                Constraint::Horizontal { line }
            } else {
                Constraint::Vertical { line }
            })?;
        }
        let mid = self.add_point(centre)?;
        let d1 = self.add_line(pts[0], pts[2])?;
        let d2 = self.add_line(pts[1], pts[3])?;
        self.set_construction(d1, true)?;
        self.set_construction(d2, true)?;
        // One midpoint is enough: a rectangle's diagonals bisect each other.
        self.add_constraint(Constraint::Midpoint {
            point: mid,
            line: d1,
        })?;
        Ok(())
    }

    fn three_point_arc(
        &mut self,
        start: Point2,
        through: Point2,
        end: Point2,
    ) -> Result<(), ToolError> {
        for p in [start, through, end] {
            check_point("an arc point", p)?;
        }
        // The circumcentre, worked out relative to the start for precision.
        let (m, e) = (through - start, end - start);
        let den = 2.0 * m.cross(e);
        if den.abs() <= LINEAR_TOLERANCE * e.length().max(m.length()) {
            return refuse("the three points are in a line");
        }
        let (mm, ee) = (m.dot(m), e.dot(e));
        let c = start + Vec2::new(e.y * mm - m.y * ee, m.x * ee - e.x * mm) * (1.0 / den);
        check_point("the arc centre", c)?;
        let centre = self.add_point(c)?;
        let s = self.add_point(start)?;
        let f = self.add_point(end)?;
        // Counter-clockwise from start to end when the through point turns left.
        if den > 0.0 {
            self.add_arc(centre, s, f)?;
        } else {
            self.add_arc(centre, f, s)?;
        }
        Ok(())
    }

    fn tangent_arc(&mut self, from: PointId, to: Point2) -> Result<(), ToolError> {
        check_point("the arc end", to)?;
        let p = self.pos(from)?;
        let users: Vec<(CurveId, Curve)> = self
            .curves()
            .filter(|(_, c)| ends(c).is_some_and(|(s, e)| s == from || e == from))
            .map(|(k, c)| (k, c.clone()))
            .collect();
        let [(curve, c)] = users.as_slice() else {
            return refuse("a tangent arc starts at the free end of one line or arc");
        };
        // The direction of travel leaving the curve at `from`.
        let t = match c {
            Curve::Line { start, end, .. } => {
                let (a, b) = (self.pos(*start)?, self.pos(*end)?);
                if *end == from { b - a } else { a - b }
            }
            Curve::Arc { centre, end, .. } => {
                let r = p - self.pos(*centre)?;
                if *end == from { r.perp() } else { -r.perp() }
            }
            Curve::Circle { .. } => return refuse("a circle has no end"),
        };
        let Some(t) = t.normalized() else {
            return refuse("the curve has no direction at that end");
        };
        let n = t.perp();
        let w = to - p;
        let den = 2.0 * n.dot(w);
        if den.abs() <= LINEAR_TOLERANCE {
            return refuse("the end is straight ahead: draw a line");
        }
        let rho = w.dot(w) / den;
        let centre_at = p + n * rho;
        check_point("the arc centre", centre_at)?;
        let centre = self.add_point(centre_at)?;
        let q = self.add_point(to)?;
        let arc = if rho > 0.0 {
            self.add_arc(centre, from, q)?
        } else {
            self.add_arc(centre, q, from)?
        };
        self.add_constraint(Constraint::Tangent { a: *curve, b: arc })?;
        Ok(())
    }
}

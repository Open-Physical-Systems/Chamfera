//! The sketch: storage, editing, validation and serialisation.
//!
//! Every edit is checked before it is applied, so a [`Sketch`] is always valid: ids refer to
//! entities of the right kind, numbers are finite and inside the working range, counts are
//! under the limits below. Deserialising re-checks all of this, because a design file is
//! hostile input.

use crate::constraint::Constraint;
use crate::entity::{ConstraintId, Curve, CurveId, PointId};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE, Point2, is_valid_coordinate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// Most points a sketch may hold.
pub const MAX_POINTS: usize = 10_000;
/// Most curves (lines, circles, arcs, ellipses) a sketch may hold.
pub const MAX_CURVES: usize = 10_000;
/// Most constraints and dimensions a sketch may hold.
pub const MAX_CONSTRAINTS: usize = 20_000;

/// Why an edit or a loaded sketch was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchError {
    #[error("{0} does not exist")]
    UnknownPoint(PointId),
    #[error("{0} does not exist")]
    UnknownCurve(CurveId),
    #[error("{0} does not exist")]
    UnknownConstraint(ConstraintId),
    #[error(
        "`{what}` must be a finite number of magnitude at most {MAX_COORDINATE} mm, got {value}"
    )]
    BadCoordinate { what: &'static str, value: f64 },
    #[error("`{what}` must be {allowed}, got {value}")]
    BadValue {
        what: &'static str,
        allowed: &'static str,
        value: f64,
    },
    #[error("{curve} is a {found}, but a {expected} is needed here")]
    WrongCurveKind {
        curve: CurveId,
        found: &'static str,
        expected: &'static str,
    },
    #[error("{what} must refer to two different entities")]
    SameEntityTwice { what: &'static str },
    #[error("a sketch may hold at most {limit} {what}")]
    TooMany { what: &'static str, limit: usize },
    #[error("id {0} is used twice")]
    DuplicateId(u32),
    #[error("the sketch has used up its ids")]
    IdsExhausted,
    #[error("`next_id` must be larger than every id in the sketch")]
    BadNextId,
    #[error("{0} is not a dimension and has no value")]
    NotADimension(ConstraintId),
    #[error("{0}")]
    BadSpline(String),
    #[error("{0} is a driven dimension: it shows a size and cannot set one")]
    Driven(ConstraintId),
}

/// A 2D sketch: points, curves and the constraints between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SketchData", into = "SketchData")]
pub struct Sketch {
    points: BTreeMap<PointId, Point2>,
    curves: BTreeMap<CurveId, Curve>,
    constraints: BTreeMap<ConstraintId, Constraint>,
    /// Dimensions that only show a size (driven, or reference, dimensions): they take no
    /// part in the solve, and their value follows the geometry after every solve.
    driven: BTreeSet<ConstraintId>,
    next_id: u32,
}

impl Default for Sketch {
    fn default() -> Self {
        Self::new()
    }
}

fn check_coordinate(what: &'static str, value: f64) -> Result<(), SketchError> {
    if is_valid_coordinate(value) {
        Ok(())
    } else {
        Err(SketchError::BadCoordinate { what, value })
    }
}

fn check_point2(what: &'static str, p: Point2) -> Result<(), SketchError> {
    check_coordinate(what, p.x)?;
    check_coordinate(what, p.y)
}

fn check_radius(what: &'static str, value: f64) -> Result<(), SketchError> {
    if value.is_finite() && value > LINEAR_TOLERANCE && value <= MAX_COORDINATE {
        Ok(())
    } else {
        Err(SketchError::BadValue {
            what,
            allowed: "a finite length above the linear tolerance and at most 1e6 mm",
            value,
        })
    }
}

/// Checks a dimension value for the given constraint kind.
fn check_dimension_value(c: &Constraint, value: f64) -> Result<(), SketchError> {
    match c {
        // A line's length must be positive: a zero-length line collapses to a point. A
        // distance of 0 is fine (the points meet, or the point lies on the line).
        Constraint::Length { .. } => {
            if value.is_finite() && value > LINEAR_TOLERANCE && value <= MAX_COORDINATE {
                Ok(())
            } else {
                Err(SketchError::BadValue {
                    what: "value",
                    allowed: "a finite length above 0 and at most 1e6 mm",
                    value,
                })
            }
        }
        Constraint::Distance { .. }
        | Constraint::HorizontalDistance { .. }
        | Constraint::VerticalDistance { .. }
        | Constraint::PointLineDistance { .. } => {
            if value.is_finite() && (0.0..=MAX_COORDINATE).contains(&value) {
                Ok(())
            } else {
                Err(SketchError::BadValue {
                    what: "value",
                    allowed: "a finite length from 0 to 1e6 mm",
                    value,
                })
            }
        }
        Constraint::Radius { .. } | Constraint::Diameter { .. } => check_radius("value", value),
        Constraint::ArcLength { .. } => {
            if value.is_finite() && value > LINEAR_TOLERANCE && value <= TAU * MAX_COORDINATE {
                Ok(())
            } else {
                Err(SketchError::BadValue {
                    what: "value",
                    allowed: "a finite length above 0 and at most 2π × 1e6 mm",
                    value,
                })
            }
        }
        Constraint::Angle { .. } => {
            if value.is_finite() && value.abs() <= TAU {
                Ok(())
            } else {
                Err(SketchError::BadValue {
                    what: "value",
                    allowed: "a finite angle from -2π to 2π radians",
                    value,
                })
            }
        }
        _ => Ok(()),
    }
}

impl Sketch {
    /// An empty sketch.
    pub fn new() -> Self {
        Self {
            points: BTreeMap::new(),
            curves: BTreeMap::new(),
            constraints: BTreeMap::new(),
            driven: BTreeSet::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> Result<u32, SketchError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(SketchError::IdsExhausted)?;
        Ok(id)
    }

    // ----- queries -----

    /// The position of a point.
    pub fn point(&self, id: PointId) -> Option<Point2> {
        self.points.get(&id).copied()
    }

    pub fn curve(&self, id: CurveId) -> Option<&Curve> {
        self.curves.get(&id)
    }

    pub fn constraint(&self, id: ConstraintId) -> Option<&Constraint> {
        self.constraints.get(&id)
    }

    /// All points, in id order.
    pub fn points(&self) -> impl Iterator<Item = (PointId, Point2)> + '_ {
        self.points.iter().map(|(k, v)| (*k, *v))
    }

    /// All curves, in id order.
    pub fn curves(&self) -> impl Iterator<Item = (CurveId, &Curve)> + '_ {
        self.curves.iter().map(|(k, v)| (*k, v))
    }

    /// All constraints, in id order.
    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintId, &Constraint)> + '_ {
        self.constraints.iter().map(|(k, v)| (*k, v))
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.curves.is_empty() && self.constraints.is_empty()
    }

    /// The radius of a circle or arc (an arc's radius is measured to its start point).
    pub fn radius(&self, id: CurveId) -> Option<f64> {
        match self.curves.get(&id)? {
            Curve::Circle { radius, .. } => Some(*radius),
            Curve::Arc { centre, start, .. } => {
                Some(self.point(*start)?.distance(self.point(*centre)?))
            }
            Curve::Line { .. }
            | Curve::Ellipse { .. }
            | Curve::EllipticalArc { .. }
            | Curve::Spline { .. } => None,
        }
    }

    // ----- editing -----

    /// Adds a free point.
    pub fn add_point(&mut self, at: Point2) -> Result<PointId, SketchError> {
        check_point2("point", at)?;
        if self.points.len() >= MAX_POINTS {
            return Err(SketchError::TooMany {
                what: "points",
                limit: MAX_POINTS,
            });
        }
        let id = PointId(self.alloc_id()?);
        self.points.insert(id, at);
        Ok(id)
    }

    fn insert_curve(&mut self, curve: Curve) -> Result<CurveId, SketchError> {
        self.check_curve(&curve)?;
        if self.curves.len() >= MAX_CURVES {
            return Err(SketchError::TooMany {
                what: "curves",
                limit: MAX_CURVES,
            });
        }
        let id = CurveId(self.alloc_id()?);
        self.curves.insert(id, curve);
        Ok(id)
    }

    /// Adds a line between two existing points.
    pub fn add_line(&mut self, start: PointId, end: PointId) -> Result<CurveId, SketchError> {
        self.insert_curve(Curve::Line {
            start,
            end,
            construction: false,
        })
    }

    /// Adds a circle around an existing centre point.
    pub fn add_circle(&mut self, centre: PointId, radius: f64) -> Result<CurveId, SketchError> {
        self.insert_curve(Curve::Circle {
            centre,
            radius,
            construction: false,
        })
    }

    /// Adds an arc swept counter-clockwise from `start` to `end` around `centre`. The end
    /// need not be at the start's radius yet; the next solve puts it there.
    pub fn add_arc(
        &mut self,
        centre: PointId,
        start: PointId,
        end: PointId,
    ) -> Result<CurveId, SketchError> {
        self.insert_curve(Curve::Arc {
            centre,
            start,
            end,
            construction: false,
        })
    }

    /// Adds an ellipse about `centre` with semi-axes to `major` and `minor`. The minor point
    /// need not be square to the major axis yet; the next solve puts it there.
    pub fn add_ellipse(
        &mut self,
        centre: PointId,
        major: PointId,
        minor: PointId,
    ) -> Result<CurveId, SketchError> {
        self.insert_curve(Curve::Ellipse {
            centre,
            major,
            minor,
            construction: false,
        })
    }

    /// Adds an arc of an ellipse, counter-clockwise from `start` to `end`. The ends need
    /// not be on the ellipse yet; the next solve puts them there.
    pub fn add_elliptical_arc(
        &mut self,
        centre: PointId,
        major: PointId,
        minor: PointId,
        start: PointId,
        end: PointId,
    ) -> Result<CurveId, SketchError> {
        self.insert_curve(Curve::EllipticalArc {
            centre,
            major,
            minor,
            start,
            end,
            construction: false,
        })
    }

    /// Marks a curve as construction geometry (or back).
    pub fn set_construction(&mut self, id: CurveId, construction: bool) -> Result<(), SketchError> {
        let curve = self
            .curves
            .get_mut(&id)
            .ok_or(SketchError::UnknownCurve(id))?;
        curve.set_construction(construction);
        Ok(())
    }

    /// Moves a point without solving (for example while building a sketch). Refused if it
    /// would put an arc's start on its centre, or give an ellipse a semi-axis of no length.
    pub fn move_point(&mut self, id: PointId, at: Point2) -> Result<(), SketchError> {
        check_point2("point", at)?;
        let old = self.point(id).ok_or(SketchError::UnknownPoint(id))?;
        self.points.insert(id, at);
        let arcs_ok = self
            .curves
            .values()
            .filter(|c| {
                (matches!(c, Curve::Arc { .. }) || c.is_elliptical()) && c.points().contains(&id)
            })
            .try_for_each(|c| self.check_curve(c));
        if let Err(e) = arcs_ok {
            self.points.insert(id, old);
            return Err(e);
        }
        Ok(())
    }

    /// Adds a constraint without solving. Use [`Sketch::try_add_constraint`] to add and solve
    /// in one step, keeping the sketch unchanged if the solve fails.
    pub fn add_constraint(&mut self, c: Constraint) -> Result<ConstraintId, SketchError> {
        self.check_constraint(&c)?;
        if self.constraints.len() >= MAX_CONSTRAINTS {
            return Err(SketchError::TooMany {
                what: "constraints",
                limit: MAX_CONSTRAINTS,
            });
        }
        let id = ConstraintId(self.alloc_id()?);
        self.constraints.insert(id, c);
        Ok(id)
    }

    /// Fixes a point where it is now.
    pub fn fix_point(&mut self, point: PointId) -> Result<ConstraintId, SketchError> {
        let at = self.point(point).ok_or(SketchError::UnknownPoint(point))?;
        self.add_constraint(Constraint::Fix { point, at })
    }

    /// Removes a constraint and returns it.
    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        let c = self
            .constraints
            .remove(&id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        self.driven.remove(&id);
        Ok(c)
    }

    /// True for a driven (reference) dimension: one that shows a size without setting it.
    pub fn is_driven(&self, id: ConstraintId) -> bool {
        self.driven.contains(&id)
    }

    /// Makes a dimension driven (it shows the size the other constraints give and takes no
    /// part in the solve) or driving again. A driven dimension's value is measured at once.
    pub fn set_driven(&mut self, id: ConstraintId, driven: bool) -> Result<(), SketchError> {
        let c = self
            .constraints
            .get(&id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        if !c.is_dimension() {
            return Err(SketchError::NotADimension(id));
        }
        if driven {
            self.driven.insert(id);
            self.refresh_driven();
        } else {
            self.driven.remove(&id);
        }
        Ok(())
    }

    /// The size a dimension measures on the geometry as it is now (mm or radians), or
    /// `None` for a constraint without a value or with missing geometry. An angle is the
    /// signed counter-clockwise angle from line `a` to line `b`, in (-π, π].
    pub fn measure(&self, c: &Constraint) -> Option<f64> {
        let line = |id: CurveId| match self.curve(id)? {
            Curve::Line { start, end, .. } => Some((self.point(*start)?, self.point(*end)?)),
            _ => None,
        };
        Some(match c {
            Constraint::Distance { a, b, .. } => self.point(*a)?.distance(self.point(*b)?),
            Constraint::HorizontalDistance { a, b, .. } => {
                (self.point(*b)?.x - self.point(*a)?.x).abs()
            }
            Constraint::VerticalDistance { a, b, .. } => {
                (self.point(*b)?.y - self.point(*a)?.y).abs()
            }
            Constraint::PointLineDistance { point, line: l, .. } => {
                let (a, b) = line(*l)?;
                let d = (b - a).normalized()?;
                d.cross(self.point(*point)? - a).abs()
            }
            Constraint::Length { line: l, .. } => {
                let (a, b) = line(*l)?;
                a.distance(b)
            }
            Constraint::Radius { curve, .. } => self.radius(*curve)?,
            Constraint::Diameter { curve, .. } => 2.0 * self.radius(*curve)?,
            Constraint::ArcLength { curve, .. } => match self.curve(*curve)? {
                Curve::Arc {
                    centre, start, end, ..
                } => {
                    let (c, s, e) = (self.point(*centre)?, self.point(*start)?, self.point(*end)?);
                    let (_, sweep) = crate::intersect::arc_angles(c, s, e)?;
                    c.distance(s) * sweep
                }
                _ => return None,
            },
            Constraint::Angle { a, b, .. } => {
                let (a0, a1) = line(*a)?;
                let (b0, b1) = line(*b)?;
                let (u, v) = (a1 - a0, b1 - b0);
                u.cross(v).atan2(u.dot(v))
            }
            _ => return None,
        })
    }

    /// Sets every driven dimension to the size it measures now. A size that is no valid
    /// value for its kind (a line of no length) keeps the last one.
    pub(crate) fn refresh_driven(&mut self) {
        let ids: Vec<ConstraintId> = self.driven.iter().copied().collect();
        for id in ids {
            let Some(c) = self.constraints.get(&id) else {
                continue;
            };
            let Some(v) = self.measure(c) else { continue };
            if check_dimension_value(c, v).is_ok()
                && let Some(c) = self.constraints.get_mut(&id)
            {
                c.set_value_unchecked(v);
            }
        }
    }

    /// Removes a curve and every constraint that refers to it. Its points stay.
    pub fn remove_curve(&mut self, id: CurveId) -> Result<Curve, SketchError> {
        let curve = self
            .curves
            .remove(&id)
            .ok_or(SketchError::UnknownCurve(id))?;
        self.constraints
            .retain(|_, c| !c.curve_refs().contains(&id));
        self.forget_driven();
        Ok(curve)
    }

    /// Removes a point, every curve that uses it and every constraint that refers to either.
    pub fn remove_point(&mut self, id: PointId) -> Result<(), SketchError> {
        if self.points.remove(&id).is_none() {
            return Err(SketchError::UnknownPoint(id));
        }
        let dead: BTreeSet<CurveId> = self
            .curves
            .iter()
            .filter(|(_, c)| c.points().contains(&id))
            .map(|(k, _)| *k)
            .collect();
        self.curves.retain(|k, _| !dead.contains(k));
        self.constraints.retain(|_, c| {
            !c.point_refs().contains(&id) && !c.curve_refs().iter().any(|k| dead.contains(k))
        });
        self.forget_driven();
        Ok(())
    }

    /// Drops driven marks of constraints that are gone.
    fn forget_driven(&mut self) {
        let constraints = &self.constraints;
        self.driven.retain(|k| constraints.contains_key(k));
    }

    /// Changes a dimension's value (mm or radians) without solving. Use
    /// [`Sketch::try_set_dimension`] to change and solve, keeping the old value on failure.
    pub fn set_dimension(&mut self, id: ConstraintId, value: f64) -> Result<(), SketchError> {
        let c = self
            .constraints
            .get_mut(&id)
            .ok_or(SketchError::UnknownConstraint(id))?;
        if !c.is_dimension() {
            return Err(SketchError::NotADimension(id));
        }
        if self.driven.contains(&id) {
            return Err(SketchError::Driven(id));
        }
        check_dimension_value(c, value)?;
        c.set_value_unchecked(value);
        Ok(())
    }

    // ----- crate-internal editing for the sketch tools -----

    /// Adds a curve of any kind (checked like the public adders).
    pub(crate) fn add_curve(&mut self, curve: Curve) -> Result<CurveId, SketchError> {
        self.insert_curve(curve)
    }

    /// Replaces a curve's definition, keeping its id and the constraints on it. The tools
    /// that call this drop the constraints that no longer make sense.
    pub(crate) fn replace_curve(&mut self, id: CurveId, curve: Curve) -> Result<(), SketchError> {
        if !self.curves.contains_key(&id) {
            return Err(SketchError::UnknownCurve(id));
        }
        self.check_curve(&curve)?;
        self.curves.insert(id, curve);
        Ok(())
    }

    /// Rewrites constraints in place, keeping their ids: `f` returns the new form, or
    /// `None` to leave one as it is. The tools validate the sketch afterwards.
    pub(crate) fn rewrite_constraints(
        &mut self,
        mut f: impl FnMut(&Constraint) -> Option<Constraint>,
    ) {
        for c in self.constraints.values_mut() {
            if let Some(new) = f(c) {
                *c = new;
            }
        }
    }

    /// Keeps only the constraints `keep` says yes to.
    pub(crate) fn retain_constraints(&mut self, mut keep: impl FnMut(&Constraint) -> bool) {
        self.constraints.retain(|_, c| keep(c));
        self.forget_driven();
    }

    // ----- crate-internal access for the solver -----

    pub(crate) fn set_point_unchecked(&mut self, id: PointId, at: Point2) {
        if let Some(p) = self.points.get_mut(&id) {
            *p = at;
        }
    }

    pub(crate) fn set_radius_unchecked(&mut self, id: CurveId, value: f64) {
        if let Some(Curve::Circle { radius, .. }) = self.curves.get_mut(&id) {
            *radius = value;
        }
    }

    // ----- validation -----

    fn check_point_id(&self, id: PointId) -> Result<(), SketchError> {
        if self.points.contains_key(&id) {
            Ok(())
        } else {
            Err(SketchError::UnknownPoint(id))
        }
    }

    fn curve_kind(&self, id: CurveId) -> Result<&Curve, SketchError> {
        self.curves.get(&id).ok_or(SketchError::UnknownCurve(id))
    }

    fn check_line(&self, id: CurveId) -> Result<(), SketchError> {
        let c = self.curve_kind(id)?;
        if c.is_line() {
            Ok(())
        } else {
            Err(SketchError::WrongCurveKind {
                curve: id,
                found: c.kind_name(),
                expected: "line",
            })
        }
    }

    fn check_circular(&self, id: CurveId) -> Result<(), SketchError> {
        let c = self.curve_kind(id)?;
        if c.is_circular() {
            Ok(())
        } else {
            Err(SketchError::WrongCurveKind {
                curve: id,
                found: c.kind_name(),
                expected: "circle or arc",
            })
        }
    }

    fn check_curve(&self, curve: &Curve) -> Result<(), SketchError> {
        for p in curve.points() {
            self.check_point_id(p)?;
        }
        match curve {
            Curve::Line { start, end, .. } => {
                if start == end {
                    return Err(SketchError::SameEntityTwice { what: "a line" });
                }
            }
            Curve::Circle { radius, .. } => check_radius("radius", *radius)?,
            Curve::Arc {
                centre, start, end, ..
            } => {
                if centre == start || centre == end || start == end {
                    return Err(SketchError::SameEntityTwice {
                        what: "an arc's centre, start and end",
                    });
                }
                let r = match (self.point(*centre), self.point(*start)) {
                    (Some(c), Some(s)) => c.distance(s),
                    _ => 0.0,
                };
                check_radius("arc radius", r)?;
            }
            Curve::Spline { points, closed, .. } => {
                crate::spline::check_points(points, *closed)?;
            }
            Curve::Ellipse {
                centre,
                major,
                minor,
                ..
            }
            | Curve::EllipticalArc {
                centre,
                major,
                minor,
                ..
            } => {
                let ids = curve.points();
                let distinct = ids
                    .iter()
                    .enumerate()
                    .all(|(i, p)| !ids.iter().skip(i + 1).any(|q| q == p));
                if !distinct {
                    return Err(SketchError::SameEntityTwice {
                        what: "an ellipse's points",
                    });
                }
                let axis = |p: &PointId| match (self.point(*centre), self.point(*p)) {
                    (Some(c), Some(q)) => c.distance(q),
                    _ => 0.0,
                };
                check_radius("ellipse major semi-axis", axis(major))?;
                check_radius("ellipse minor semi-axis", axis(minor))?;
            }
        }
        Ok(())
    }

    fn check_constraint(&self, c: &Constraint) -> Result<(), SketchError> {
        for p in c.point_refs() {
            self.check_point_id(p)?;
        }
        for k in c.curve_refs() {
            self.curve_kind(k)?;
        }
        match c {
            Constraint::Coincident { a, b }
            | Constraint::HorizontalPoints { a, b }
            | Constraint::VerticalPoints { a, b }
            | Constraint::HorizontalDistance { a, b, .. }
            | Constraint::VerticalDistance { a, b, .. }
            | Constraint::Distance { a, b, .. } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice {
                        what: c.kind_name(),
                    });
                }
            }
            Constraint::Symmetric { a, b, line } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice {
                        what: c.kind_name(),
                    });
                }
                self.check_line(*line)?;
            }
            Constraint::PointOnLine { line, .. }
            | Constraint::Horizontal { line }
            | Constraint::Vertical { line }
            | Constraint::Midpoint { line, .. }
            | Constraint::PointLineDistance { line, .. }
            | Constraint::Length { line, .. } => self.check_line(*line)?,
            Constraint::PointOnCircle { curve, .. }
            | Constraint::Radius { curve, .. }
            | Constraint::Diameter { curve, .. } => self.check_circular(*curve)?,
            Constraint::Parallel { a, b }
            | Constraint::Perpendicular { a, b }
            | Constraint::Collinear { a, b }
            | Constraint::Angle { a, b, .. } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice {
                        what: c.kind_name(),
                    });
                }
                self.check_line(*a)?;
                self.check_line(*b)?;
            }
            Constraint::Tangent { a, b } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice { what: "tangent" });
                }
                if self.curve_kind(*a)?.is_line() {
                    self.check_circular(*b)?;
                } else if self.curve_kind(*b)?.is_line() {
                    self.check_circular(*a)?;
                }
            }
            Constraint::Equal { a, b } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice { what: "equal" });
                }
                if self.curve_kind(*a)?.is_line() {
                    self.check_line(*b)?;
                } else {
                    self.check_circular(*b)?;
                }
            }
            Constraint::Concentric { a, b } => {
                if a == b {
                    return Err(SketchError::SameEntityTwice { what: "concentric" });
                }
                self.check_circular(*a)?;
                self.check_circular(*b)?;
            }
            Constraint::ArcLength { curve, .. } => {
                let k = self.curve_kind(*curve)?;
                if !matches!(k, Curve::Arc { .. }) {
                    return Err(SketchError::WrongCurveKind {
                        curve: *curve,
                        found: k.kind_name(),
                        expected: "arc",
                    });
                }
            }
            Constraint::Fix { at, .. } => check_point2("at", *at)?,
        }
        if let Some(v) = c.value() {
            check_dimension_value(c, v)?;
        }
        Ok(())
    }

    /// Checks every invariant. Edits keep these; loading a file re-checks them.
    pub fn validate(&self) -> Result<(), SketchError> {
        if self.points.len() > MAX_POINTS {
            return Err(SketchError::TooMany {
                what: "points",
                limit: MAX_POINTS,
            });
        }
        if self.curves.len() > MAX_CURVES {
            return Err(SketchError::TooMany {
                what: "curves",
                limit: MAX_CURVES,
            });
        }
        if self.constraints.len() > MAX_CONSTRAINTS {
            return Err(SketchError::TooMany {
                what: "constraints",
                limit: MAX_CONSTRAINTS,
            });
        }
        let mut seen = BTreeSet::new();
        let ids = self
            .points
            .keys()
            .map(|k| k.0)
            .chain(self.curves.keys().map(|k| k.0))
            .chain(self.constraints.keys().map(|k| k.0));
        for id in ids {
            if !seen.insert(id) {
                return Err(SketchError::DuplicateId(id));
            }
            if id >= self.next_id {
                return Err(SketchError::BadNextId);
            }
        }
        for p in self.points.values() {
            check_point2("point", *p)?;
        }
        for c in self.curves.values() {
            self.check_curve(c)?;
        }
        for c in self.constraints.values() {
            self.check_constraint(c)?;
        }
        for id in &self.driven {
            if !self
                .constraints
                .get(id)
                .is_some_and(Constraint::is_dimension)
            {
                return Err(SketchError::NotADimension(*id));
            }
        }
        Ok(())
    }
}

/// The serialised form: plain lists, so the file reads well and loading can check every
/// record before trusting it.
#[derive(Serialize, Deserialize)]
struct SketchData {
    next_id: u32,
    #[serde(default)]
    points: Vec<PointRecord>,
    #[serde(default)]
    curves: Vec<CurveRecord>,
    #[serde(default)]
    constraints: Vec<ConstraintRecord>,
}

#[derive(Serialize, Deserialize)]
struct PointRecord {
    id: PointId,
    at: Point2,
}

#[derive(Serialize, Deserialize)]
struct CurveRecord {
    id: CurveId,
    curve: Curve,
}

#[derive(Serialize, Deserialize)]
struct ConstraintRecord {
    id: ConstraintId,
    constraint: Constraint,
    /// A driven (reference) dimension; absent means driving.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    driven: bool,
}

impl TryFrom<SketchData> for Sketch {
    type Error = SketchError;

    fn try_from(data: SketchData) -> Result<Self, SketchError> {
        if data.points.len() > MAX_POINTS {
            return Err(SketchError::TooMany {
                what: "points",
                limit: MAX_POINTS,
            });
        }
        if data.curves.len() > MAX_CURVES {
            return Err(SketchError::TooMany {
                what: "curves",
                limit: MAX_CURVES,
            });
        }
        if data.constraints.len() > MAX_CONSTRAINTS {
            return Err(SketchError::TooMany {
                what: "constraints",
                limit: MAX_CONSTRAINTS,
            });
        }
        let mut sketch = Sketch {
            points: BTreeMap::new(),
            curves: BTreeMap::new(),
            constraints: BTreeMap::new(),
            driven: BTreeSet::new(),
            next_id: data.next_id,
        };
        for r in data.points {
            if sketch.points.insert(r.id, r.at).is_some() {
                return Err(SketchError::DuplicateId(r.id.0));
            }
        }
        for r in data.curves {
            if sketch.curves.insert(r.id, r.curve).is_some() {
                return Err(SketchError::DuplicateId(r.id.0));
            }
        }
        for r in data.constraints {
            if sketch.constraints.insert(r.id, r.constraint).is_some() {
                return Err(SketchError::DuplicateId(r.id.0));
            }
            if r.driven {
                sketch.driven.insert(r.id);
            }
        }
        sketch.validate()?;
        Ok(sketch)
    }
}

impl From<Sketch> for SketchData {
    fn from(s: Sketch) -> Self {
        SketchData {
            next_id: s.next_id,
            points: s
                .points
                .into_iter()
                .map(|(id, at)| PointRecord { id, at })
                .collect(),
            curves: s
                .curves
                .into_iter()
                .map(|(id, curve)| CurveRecord { id, curve })
                .collect(),
            constraints: s
                .constraints
                .into_iter()
                .map(|(id, constraint)| ConstraintRecord {
                    id,
                    constraint,
                    driven: s.driven.contains(&id),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    #[test]
    fn edits_reject_bad_numbers_and_unknown_ids() {
        let mut s = Sketch::new();
        assert!(s.add_point(p(f64::NAN, 0.0)).is_err());
        assert!(s.add_point(p(0.0, f64::INFINITY)).is_err());
        assert!(s.add_point(p(2.0 * MAX_COORDINATE, 0.0)).is_err());
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        assert_eq!(
            s.add_line(a, PointId(999)),
            Err(SketchError::UnknownPoint(PointId(999)))
        );
        assert!(s.add_line(a, a).is_err());
        assert!(s.add_circle(a, -1.0).is_err());
        assert!(s.add_circle(a, 0.0).is_err());
        assert!(s.add_circle(a, f64::NAN).is_err());
        let b = s.add_point(p(1.0, 0.0)).unwrap();
        let c = s.add_point(p(0.0, 1.0)).unwrap();
        // Arc centre equal to start: no radius.
        let same = s.add_point(p(0.0, 0.0)).unwrap();
        assert!(s.add_arc(a, same, b).is_err());
        assert!(s.add_arc(a, b, b).is_err());
        let arc = s.add_arc(a, b, c).unwrap();
        // Moving the arc's start onto its centre is refused.
        assert!(s.move_point(b, p(0.0, 0.0)).is_err());
        assert_eq!(s.point(b), Some(p(1.0, 0.0)));
        // A line constraint on an arc is the wrong kind.
        assert!(matches!(
            s.add_constraint(Constraint::Horizontal { line: arc }),
            Err(SketchError::WrongCurveKind { .. })
        ));
        assert!(
            s.add_constraint(Constraint::Coincident { a, b: a })
                .is_err()
        );
        assert!(
            s.add_constraint(Constraint::Radius {
                curve: arc,
                value: f64::NAN
            })
            .is_err()
        );
        assert!(
            s.add_constraint(Constraint::Angle {
                a: CurveId(77),
                b: CurveId(78),
                value: 1.0
            })
            .is_err()
        );
        // Nothing invalid was added.
        assert_eq!(s.constraints().count(), 0);
        s.validate().unwrap();
    }

    #[test]
    fn removing_a_point_removes_what_depends_on_it() {
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(10.0, 0.0)).unwrap();
        let c = s.add_point(p(10.0, 10.0)).unwrap();
        let l1 = s.add_line(a, b).unwrap();
        let l2 = s.add_line(b, c).unwrap();
        s.add_constraint(Constraint::Horizontal { line: l1 })
            .unwrap();
        s.add_constraint(Constraint::Perpendicular { a: l1, b: l2 })
            .unwrap();
        s.fix_point(c).unwrap();
        s.remove_point(a).unwrap();
        assert!(s.curve(l1).is_none());
        assert!(s.curve(l2).is_some());
        // Only the fix on c survives.
        assert_eq!(s.constraints().count(), 1);
        s.validate().unwrap();
        assert!(s.remove_point(a).is_err());
    }

    #[test]
    fn dimensions_change_only_to_allowed_values() {
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(10.0, 0.0)).unwrap();
        let l = s.add_line(a, b).unwrap();
        let len = s
            .add_constraint(Constraint::Length {
                line: l,
                value: 10.0,
            })
            .unwrap();
        let h = s
            .add_constraint(Constraint::Horizontal { line: l })
            .unwrap();
        s.set_dimension(len, 25.0).unwrap();
        assert_eq!(s.constraint(len).and_then(Constraint::value), Some(25.0));
        assert!(s.set_dimension(len, -1.0).is_err());
        assert!(s.set_dimension(len, f64::NAN).is_err());
        assert_eq!(s.set_dimension(h, 1.0), Err(SketchError::NotADimension(h)));
        assert_eq!(s.constraint(len).and_then(Constraint::value), Some(25.0));
    }

    #[test]
    fn a_sketch_round_trips_through_json() {
        let mut s = Sketch::new();
        let a = s.add_point(p(0.0, 0.0)).unwrap();
        let b = s.add_point(p(10.0, 0.5)).unwrap();
        let l = s.add_line(a, b).unwrap();
        let c = s.add_circle(b, 4.0).unwrap();
        s.set_construction(c, true).unwrap();
        s.add_constraint(Constraint::Tangent { a: l, b: c })
            .unwrap();
        s.fix_point(a).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: Sketch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn loading_rejects_dangling_ids_bad_numbers_and_duplicates() {
        let dangling = r#"{"next_id":10,"points":[{"id":1,"at":{"x":0,"y":0}}],
            "curves":[{"id":2,"curve":{"kind":"line","start":1,"end":5}}]}"#;
        assert!(serde_json::from_str::<Sketch>(dangling).is_err());
        let huge = r#"{"next_id":10,"points":[{"id":1,"at":{"x":1e300,"y":0}}]}"#;
        assert!(serde_json::from_str::<Sketch>(huge).is_err());
        let nan = r#"{"next_id":10,"points":[{"id":1,"at":{"x":null,"y":0}}]}"#;
        assert!(serde_json::from_str::<Sketch>(nan).is_err());
        let dup = r#"{"next_id":10,"points":[{"id":1,"at":{"x":0,"y":0}},
            {"id":1,"at":{"x":1,"y":0}}]}"#;
        assert!(serde_json::from_str::<Sketch>(dup).is_err());
        let cross_kind_dup = r#"{"next_id":10,"points":[{"id":1,"at":{"x":0,"y":0}},
            {"id":2,"at":{"x":1,"y":0}}],
            "curves":[{"id":1,"curve":{"kind":"line","start":1,"end":2}}]}"#;
        assert!(serde_json::from_str::<Sketch>(cross_kind_dup).is_err());
        let stale_next = r#"{"next_id":1,"points":[{"id":1,"at":{"x":0,"y":0}}]}"#;
        assert!(serde_json::from_str::<Sketch>(stale_next).is_err());
        let wrong_kind = r#"{"next_id":10,"points":[{"id":1,"at":{"x":0,"y":0}},
            {"id":2,"at":{"x":1,"y":0}}],
            "curves":[{"id":3,"curve":{"kind":"line","start":1,"end":2}}],
            "constraints":[{"id":4,"constraint":{"kind":"radius","curve":3,"value":2}}]}"#;
        assert!(serde_json::from_str::<Sketch>(wrong_kind).is_err());
        let empty = r#"{"next_id":1}"#;
        assert!(serde_json::from_str::<Sketch>(empty).unwrap().is_empty());
    }
}

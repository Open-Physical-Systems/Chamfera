//! Where to show each geometric constraint as a small marker, and which constraints stand
//! in the way of a new one.
//!
//! **Markers.** Every constraint that is not a dimension (dimensions have their own
//! labels) gets one marker per place it is about: a point constraint at the point, a line
//! constraint at the middle of the line, a constraint between two curves at each curve,
//! concentric at the shared centre, alignment of two points halfway between them.
//!
//! **Conflicts.** When a new constraint cannot be added (the solve reports conflicting
//! constraints or no solution), [`Sketch::conflicts_with`] names the existing constraints
//! whose removal lets it in: for each candidate, the sketch without it is solved with the
//! new constraints, and the candidate counts when that solve succeeds. The candidates are
//! the constraints that share a point or curve with the new ones or with geometry they
//! constrain, up to [`MAX_CONFLICT_CANDIDATES`], so the answer is an exact list of the
//! single removals that work.

use crate::constraint::Constraint;
use crate::entity::{ConstraintId, Curve, CurveId, PointId};
use crate::sketch::Sketch;
use crate::solver::SolveError;
use chamfera_geom::{LINEAR_TOLERANCE, Point2, Vec2};
use std::collections::BTreeSet;
use std::f64::consts::FRAC_PI_4;

/// Most constraints tried one by one when a new constraint conflicts.
pub const MAX_CONFLICT_CANDIDATES: usize = 200;

/// One marker of a constraint: which constraint, its kind (the serialised `kind`), and the
/// sketch point it is shown at.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintMarker {
    pub constraint: ConstraintId,
    pub kind: &'static str,
    pub at: Point2,
}

impl Sketch {
    /// A point to show a marker of a curve at: the middle of a line, the point 45° round a
    /// circle or ellipse, the middle of an arc or spline.
    pub fn curve_marker_point(&self, id: CurveId) -> Option<Point2> {
        let c = self.curve(id)?;
        match c {
            Curve::Line { start, end, .. } => {
                Some(self.point(*start)?.lerp(self.point(*end)?, 0.5))
            }
            Curve::Circle { centre, radius, .. } => {
                let k = self.point(*centre)?;
                Some(k + Vec2::new(FRAC_PI_4.cos(), FRAC_PI_4.sin()) * *radius)
            }
            Curve::Arc {
                centre, start, end, ..
            } => {
                let (k, s, e) = (self.point(*centre)?, self.point(*start)?, self.point(*end)?);
                let (a0, sweep) = crate::intersect::arc_angles(k, s, e)?;
                let mid = a0 + 0.5 * sweep;
                Some(k + Vec2::new(mid.cos(), mid.sin()) * k.distance(s))
            }
            Curve::Ellipse { .. } | Curve::EllipticalArc { .. } => {
                let e = self.ellipse_arc_of(c)?;
                let t = if e.is_full() {
                    FRAC_PI_4
                } else {
                    e.t0 + 0.5 * e.sweep
                };
                Some(e.point_at(t))
            }
            Curve::Spline { .. } => {
                let s = self.spline_of(c)?;
                let (lo, hi) = s.domain();
                Some(s.point_at(0.5 * (lo + hi)))
            }
        }
    }

    /// The markers of every geometric constraint (not dimensions), in constraint order.
    /// A constraint whose geometry is gone has none.
    pub fn constraint_markers(&self) -> Vec<ConstraintMarker> {
        let mut out = Vec::new();
        for (id, c) in self.constraints() {
            if c.is_dimension() {
                continue;
            }
            let kind = c.kind_name();
            let mut push = |at: Option<Point2>| {
                if let Some(at) = at {
                    out.push(ConstraintMarker {
                        constraint: id,
                        kind,
                        at,
                    });
                }
            };
            match c {
                Constraint::Coincident { a, .. } => push(self.point(*a)),
                Constraint::PointOnLine { point, .. }
                | Constraint::PointOnCircle { point, .. }
                | Constraint::Fix { point, .. }
                | Constraint::Midpoint { point, .. } => push(self.point(*point)),
                Constraint::Horizontal { line } | Constraint::Vertical { line } => {
                    push(self.curve_marker_point(*line));
                }
                Constraint::Parallel { a, b }
                | Constraint::Perpendicular { a, b }
                | Constraint::Tangent { a, b }
                | Constraint::Equal { a, b }
                | Constraint::Collinear { a, b } => {
                    push(self.curve_marker_point(*a));
                    push(self.curve_marker_point(*b));
                }
                Constraint::Concentric { a, .. } => push(self.centre_of(*a)),
                Constraint::Symmetric { a, b, .. } => {
                    push(self.point(*a));
                    push(self.point(*b));
                }
                Constraint::HorizontalPoints { a, b } | Constraint::VerticalPoints { a, b } => {
                    if let (Some(p), Some(q)) = (self.point(*a), self.point(*b)) {
                        push(Some(p.lerp(q, 0.5)));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The centre of a circle, arc, ellipse or elliptical arc.
    fn centre_of(&self, id: CurveId) -> Option<Point2> {
        match self.curve(id)? {
            Curve::Circle { centre, .. }
            | Curve::Arc { centre, .. }
            | Curve::Ellipse { centre, .. }
            | Curve::EllipticalArc { centre, .. } => self.point(*centre),
            _ => None,
        }
    }

    /// The points a constraint holds: its own points and those of its curves.
    fn held_points(&self, c: &Constraint) -> BTreeSet<PointId> {
        let mut set: BTreeSet<PointId> = c.point_refs().into_iter().collect();
        for curve in c.curve_refs() {
            if let Some(k) = self.curve(curve) {
                set.extend(k.points());
            }
        }
        set
    }

    /// Adds every constraint of `new` in order and solves after each; true when all of
    /// them fit without a line or circle shrinking to nothing (angle constraints can
    /// always be met by a line of no length, which is no answer).
    fn accepts(&self, new: &[Constraint]) -> Result<bool, SolveError> {
        let mut trial = self.clone();
        for c in new {
            match trial.try_add_constraint(c.clone()) {
                Ok(_) => {}
                Err(SolveError::Conflicting { .. } | SolveError::NoSolution { .. }) => {
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(!self.curves().any(|(id, _)| {
            let size = |s: &Sketch| s.curve_size(id).unwrap_or(0.0);
            size(self) > LINEAR_TOLERANCE && size(&trial) <= LINEAR_TOLERANCE
        }))
    }

    /// A line's length or a circle's or arc's radius.
    fn curve_size(&self, id: CurveId) -> Option<f64> {
        match self.curve(id)? {
            Curve::Line { start, end, .. } => Some(self.point(*start)?.distance(self.point(*end)?)),
            Curve::Circle { radius, .. } => Some(*radius),
            Curve::Arc { centre, start, .. } => {
                Some(self.point(*centre)?.distance(self.point(*start)?))
            }
            _ => None,
        }
    }

    /// The existing constraints whose removal (one at a time) lets every constraint of
    /// `new` be added; empty when `new` fits as it is. A driven dimension constrains
    /// nothing, so it is never one of them. Errors other than a conflict (an invalid
    /// constraint, a group too large to solve) are returned.
    pub fn conflicts_with(&self, new: &[Constraint]) -> Result<Vec<ConstraintId>, SolveError> {
        if self.accepts(new)? {
            return Ok(Vec::new());
        }
        // Candidates: constraints that hold a point the new ones hold, widened once through
        // the points those candidates hold (a chain of constraints is often what blocks).
        let mut reach: BTreeSet<PointId> = new.iter().flat_map(|c| self.held_points(c)).collect();
        for _ in 0..2 {
            let more: Vec<PointId> = self
                .constraints()
                .filter(|(_, c)| self.held_points(c).iter().any(|p| reach.contains(p)))
                .flat_map(|(_, c)| self.held_points(c))
                .collect();
            reach.extend(more);
        }
        let candidates: Vec<ConstraintId> = self
            .constraints()
            .filter(|(id, _)| !self.is_driven(*id))
            .filter(|(_, c)| {
                let held = self.held_points(c);
                held.is_empty() || held.iter().any(|p| reach.contains(p))
            })
            .map(|(id, _)| id)
            .take(MAX_CONFLICT_CANDIDATES)
            .collect();
        let mut out = Vec::new();
        for id in candidates {
            let mut without = self.clone();
            if without.remove_constraint(id).is_err() {
                continue;
            }
            if without.accepts(new)? {
                out.push(id);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(s: &mut Sketch, x: f64, y: f64) -> PointId {
        s.add_point(Point2::new(x, y)).unwrap()
    }

    #[test]
    fn markers_sit_on_the_geometry_they_constrain() {
        let mut s = Sketch::new();
        let a = pt(&mut s, 0.0, 0.0);
        let b = pt(&mut s, 40.0, 0.0);
        let c = pt(&mut s, 40.0, 30.0);
        let l1 = s.add_line(a, b).unwrap();
        let l2 = s.add_line(b, c).unwrap();
        let h = s
            .add_constraint(Constraint::Horizontal { line: l1 })
            .unwrap();
        let p = s
            .add_constraint(Constraint::Perpendicular { a: l1, b: l2 })
            .unwrap();
        let f = s.fix_point(a).unwrap();
        s.add_constraint(Constraint::Length {
            line: l1,
            value: 40.0,
        })
        .unwrap();
        let m = s.constraint_markers();
        // Horizontal at the middle of l1 (20, 0); perpendicular at both middles (20, 0)
        // and (40, 15); fix at (0, 0); the length dimension has none.
        assert_eq!(m.len(), 4, "{m:?}");
        let at = |id: ConstraintId| -> Vec<Point2> {
            m.iter()
                .filter(|k| k.constraint == id)
                .map(|k| k.at)
                .collect()
        };
        assert_eq!(at(h), vec![Point2::new(20.0, 0.0)]);
        assert_eq!(at(p), vec![Point2::new(20.0, 0.0), Point2::new(40.0, 15.0)]);
        assert_eq!(at(f), vec![Point2::new(0.0, 0.0)]);
        assert!(m.iter().any(|k| k.kind == "perpendicular"));
    }

    #[test]
    fn a_conflict_names_the_constraints_whose_removal_lets_it_in() {
        // p fixed at (0, 0), q level with it and 10 away, at (10, 0). Fixing q at (6, 8)
        // (also 10 from p) conflicts. Without "level" q can sit there; without p's fix p
        // can move to (-4, 8) or (16, 8), level with q and 10 away; without the distance
        // q still cannot be level with p at y = 8.
        let mut s = Sketch::new();
        let p = pt(&mut s, 0.0, 0.0);
        let q = pt(&mut s, 10.0, 0.0);
        let fix_p = s.fix_point(p).unwrap();
        let level = s
            .add_constraint(Constraint::HorizontalPoints { a: p, b: q })
            .unwrap();
        s.add_constraint(Constraint::Distance {
            a: p,
            b: q,
            value: 10.0,
        })
        .unwrap();
        s.solve().unwrap();
        let fix_q = [Constraint::Fix {
            point: q,
            at: Point2::new(6.0, 8.0),
        }];
        assert_eq!(s.conflicts_with(&fix_q).unwrap(), vec![fix_p, level]);
        // A constraint that fits has no conflicts.
        let mut free = Sketch::new();
        let p = pt(&mut free, 0.0, 0.0);
        let q = pt(&mut free, 10.0, 5.0);
        let line = free.add_line(p, q).unwrap();
        assert!(
            free.conflicts_with(&[Constraint::Vertical { line }])
                .unwrap()
                .is_empty()
        );
    }
}

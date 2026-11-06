//! Geometric constraints and dimensions.
//!
//! A constraint is a relation between entities; the solver turns each one into one or more
//! residual equations (see `equations.rs`). Dimensions are constraints that carry a value:
//! millimetres for distances, lengths, radii and diameters, radians for angles. The value is a
//! plain field so a later milestone can drive it from a parameter: change it with
//! [`crate::Sketch::set_dimension`] and solve again.
//!
//! Where a constraint has more than one solution branch (which side of a line a point keeps
//! its distance on, whether two circles touch outside or inside, which side of a line a
//! tangent circle lies on), the solver picks the branch closest to the current geometry
//! each time it solves, so a sketch never flips to the mirror solution.

use crate::entity::{CurveId, PointId};
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// One constraint or dimension. Curve kinds are checked when the constraint is added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    /// Two points are at the same place.
    Coincident { a: PointId, b: PointId },
    /// A point lies on the (infinite) line through `line`.
    PointOnLine { point: PointId, line: CurveId },
    /// A point lies on a circle, or on the full circle that carries an arc.
    PointOnCircle { point: PointId, curve: CurveId },
    /// A line is parallel to the X axis.
    Horizontal { line: CurveId },
    /// A line is parallel to the Y axis.
    Vertical { line: CurveId },
    /// Two lines are parallel (same or opposite direction).
    Parallel { a: CurveId, b: CurveId },
    /// Two lines are at right angles.
    Perpendicular { a: CurveId, b: CurveId },
    /// A line and a circle or arc, or two circles or arcs, touch tangentially. Tangency to an
    /// arc is to its full circle; the touching point need not be an arc endpoint.
    Tangent { a: CurveId, b: CurveId },
    /// Two lines have the same length, or two circles or arcs have the same radius.
    Equal { a: CurveId, b: CurveId },
    /// A point stays at `at`.
    Fix { point: PointId, at: Point2 },
    /// A point is the midpoint of a line.
    Midpoint { point: PointId, line: CurveId },
    /// Two points are mirror images in a line.
    Symmetric {
        a: PointId,
        b: PointId,
        line: CurveId,
    },
    /// Dimension: the distance between two points (mm).
    Distance { a: PointId, b: PointId, value: f64 },
    /// Dimension: the distance from a point to the (infinite) line through `line` (mm).
    PointLineDistance {
        point: PointId,
        line: CurveId,
        value: f64,
    },
    /// Dimension: the length of a line (mm).
    Length { line: CurveId, value: f64 },
    /// Dimension: the radius of a circle or arc (mm).
    Radius { curve: CurveId, value: f64 },
    /// Dimension: the diameter of a circle or arc (mm).
    Diameter { curve: CurveId, value: f64 },
    /// Dimension: the signed counter-clockwise angle from the direction of line `a`
    /// (start to end) to the direction of line `b` (radians).
    Angle { a: CurveId, b: CurveId, value: f64 },
}

impl Constraint {
    /// The dimension value (mm or radians), or `None` for a purely geometric constraint.
    pub fn value(&self) -> Option<f64> {
        match self {
            Constraint::Distance { value, .. }
            | Constraint::PointLineDistance { value, .. }
            | Constraint::Length { value, .. }
            | Constraint::Radius { value, .. }
            | Constraint::Diameter { value, .. }
            | Constraint::Angle { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// True when the constraint carries a value.
    pub fn is_dimension(&self) -> bool {
        self.value().is_some()
    }

    /// Replaces the value; returns false for a constraint without one. Range checks are
    /// done by [`crate::Sketch::set_dimension`].
    pub(crate) fn set_value_unchecked(&mut self, new: f64) -> bool {
        match self {
            Constraint::Distance { value, .. }
            | Constraint::PointLineDistance { value, .. }
            | Constraint::Length { value, .. }
            | Constraint::Radius { value, .. }
            | Constraint::Diameter { value, .. }
            | Constraint::Angle { value, .. } => {
                *value = new;
                true
            }
            _ => false,
        }
    }

    /// The points this constraint refers to directly.
    pub fn point_refs(&self) -> Vec<PointId> {
        match self {
            Constraint::Coincident { a, b }
            | Constraint::Distance { a, b, .. }
            | Constraint::Symmetric { a, b, .. } => vec![*a, *b],
            Constraint::PointOnLine { point, .. }
            | Constraint::PointOnCircle { point, .. }
            | Constraint::Fix { point, .. }
            | Constraint::Midpoint { point, .. }
            | Constraint::PointLineDistance { point, .. } => vec![*point],
            _ => Vec::new(),
        }
    }

    /// The curves this constraint refers to.
    pub fn curve_refs(&self) -> Vec<CurveId> {
        match self {
            Constraint::PointOnLine { line, .. }
            | Constraint::Horizontal { line }
            | Constraint::Vertical { line }
            | Constraint::Midpoint { line, .. }
            | Constraint::Symmetric { line, .. }
            | Constraint::PointLineDistance { line, .. }
            | Constraint::Length { line, .. } => vec![*line],
            Constraint::PointOnCircle { curve, .. }
            | Constraint::Radius { curve, .. }
            | Constraint::Diameter { curve, .. } => vec![*curve],
            Constraint::Parallel { a, b }
            | Constraint::Perpendicular { a, b }
            | Constraint::Tangent { a, b }
            | Constraint::Equal { a, b }
            | Constraint::Angle { a, b, .. } => vec![*a, *b],
            Constraint::Coincident { .. }
            | Constraint::Fix { .. }
            | Constraint::Distance { .. } => Vec::new(),
        }
    }

    /// A short name for messages, matching the serialised `kind`.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Constraint::Coincident { .. } => "coincident",
            Constraint::PointOnLine { .. } => "point_on_line",
            Constraint::PointOnCircle { .. } => "point_on_circle",
            Constraint::Horizontal { .. } => "horizontal",
            Constraint::Vertical { .. } => "vertical",
            Constraint::Parallel { .. } => "parallel",
            Constraint::Perpendicular { .. } => "perpendicular",
            Constraint::Tangent { .. } => "tangent",
            Constraint::Equal { .. } => "equal",
            Constraint::Fix { .. } => "fix",
            Constraint::Midpoint { .. } => "midpoint",
            Constraint::Symmetric { .. } => "symmetric",
            Constraint::Distance { .. } => "distance",
            Constraint::PointLineDistance { .. } => "point_line_distance",
            Constraint::Length { .. } => "length",
            Constraint::Radius { .. } => "radius",
            Constraint::Diameter { .. } => "diameter",
            Constraint::Angle { .. } => "angle",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dimensions_have_values() {
        let mut length = Constraint::Length {
            line: CurveId(1),
            value: 50.0,
        };
        assert_eq!(length.value(), Some(50.0));
        assert!(length.set_value_unchecked(80.0));
        assert_eq!(length.value(), Some(80.0));
        let mut horizontal = Constraint::Horizontal { line: CurveId(1) };
        assert!(!horizontal.is_dimension());
        assert!(!horizontal.set_value_unchecked(1.0));
    }

    #[test]
    fn constraints_serialise_with_a_kind_tag() {
        let c = Constraint::Distance {
            a: PointId(1),
            b: PointId(2),
            value: 3.5,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"kind":"distance","a":1,"b":2,"value":3.5}"#);
        let back: Constraint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert_eq!(c.kind_name(), "distance");
    }
}

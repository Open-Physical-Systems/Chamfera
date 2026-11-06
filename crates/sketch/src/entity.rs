//! Sketch entities: points, lines, circles and arcs, addressed by stable ids.
//!
//! Points own coordinates; curves refer to points by id, so moving a point moves every
//! curve that uses it. A circle owns its radius. An arc is a centre, a start and an end
//! point, swept counter-clockwise from start to end; its radius is the distance from the
//! centre to the start, and the solver keeps the end at the same distance (one implicit
//! equation per arc, which is why a free arc has 5 degrees of freedom, not 6).

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($name:ident, $label:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($label, " {}"), self.0)
            }
        }
    };
}

id_type!(
    PointId,
    "point",
    "A point of a sketch. Ids are never reused within a sketch."
);
id_type!(
    CurveId,
    "curve",
    "A line, circle or arc of a sketch. Ids are never reused within a sketch."
);
id_type!(
    ConstraintId,
    "constraint",
    "A constraint or dimension of a sketch. Ids are never reused within a sketch."
);

/// A curve of the sketch. Curves refer to points by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Curve {
    /// A straight segment from `start` to `end`.
    Line {
        start: PointId,
        end: PointId,
        #[serde(default)]
        construction: bool,
    },
    /// A full circle; the radius (mm) is one of the solver's unknowns.
    Circle {
        centre: PointId,
        radius: f64,
        #[serde(default)]
        construction: bool,
    },
    /// An arc swept counter-clockwise from `start` to `end` around `centre`.
    Arc {
        centre: PointId,
        start: PointId,
        end: PointId,
        #[serde(default)]
        construction: bool,
    },
}

impl Curve {
    /// Construction geometry helps constrain a sketch but never bounds a profile.
    pub fn is_construction(&self) -> bool {
        match self {
            Curve::Line { construction, .. }
            | Curve::Circle { construction, .. }
            | Curve::Arc { construction, .. } => *construction,
        }
    }

    pub(crate) fn set_construction(&mut self, value: bool) {
        match self {
            Curve::Line { construction, .. }
            | Curve::Circle { construction, .. }
            | Curve::Arc { construction, .. } => *construction = value,
        }
    }

    /// The points this curve refers to.
    pub fn points(&self) -> Vec<PointId> {
        match self {
            Curve::Line { start, end, .. } => vec![*start, *end],
            Curve::Circle { centre, .. } => vec![*centre],
            Curve::Arc {
                centre, start, end, ..
            } => vec![*centre, *start, *end],
        }
    }

    pub fn is_line(&self) -> bool {
        matches!(self, Curve::Line { .. })
    }

    /// True for circles and arcs: curves with a centre and a radius.
    pub fn is_circular(&self) -> bool {
        matches!(self, Curve::Circle { .. } | Curve::Arc { .. })
    }

    /// A short name for messages: "line", "circle" or "arc".
    pub fn kind_name(&self) -> &'static str {
        match self {
            Curve::Line { .. } => "line",
            Curve::Circle { .. } => "circle",
            Curve::Arc { .. } => "arc",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_print_with_their_kind() {
        assert_eq!(PointId(3).to_string(), "point 3");
        assert_eq!(CurveId(4).to_string(), "curve 4");
        assert_eq!(ConstraintId(5).to_string(), "constraint 5");
    }

    #[test]
    fn curves_list_their_points() {
        let arc = Curve::Arc {
            centre: PointId(1),
            start: PointId(2),
            end: PointId(3),
            construction: false,
        };
        assert_eq!(arc.points(), vec![PointId(1), PointId(2), PointId(3)]);
        assert!(arc.is_circular() && !arc.is_line());
        let mut line = Curve::Line {
            start: PointId(1),
            end: PointId(2),
            construction: false,
        };
        line.set_construction(true);
        assert!(line.is_construction());
    }
}

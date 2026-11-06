//! Layer 1: 2D sketches and Chamfera's own geometric constraint solver.
//!
//! # The model
//! A [`Sketch`] holds points, curves and constraints, each with a stable id
//! ([`PointId`], [`CurveId`], [`ConstraintId`]) that is never reused.
//! - **Points** own their coordinates (mm).
//! - **Curves** refer to points: a line has a start and an end; a circle a centre and its
//!   own radius; an arc a centre, a start and an end, swept counter-clockwise from start to
//!   end. Any curve can be construction geometry, which constrains but never bounds a
//!   profile.
//! - **Constraints** ([`Constraint`]) relate entities: coincident, point on line, point on
//!   circle or arc, horizontal, vertical, parallel, perpendicular, tangent, equal, fix,
//!   midpoint, symmetric; and **dimensions**, which carry a plain `f64` value (mm or
//!   radians) that a later milestone drives from parameters: distance, point-line distance,
//!   length, radius, diameter and angle.
//!
//! Every edit is validated, and deserialising re-validates everything (dangling ids,
//! non-finite or out-of-range numbers, counts), so a `Sketch` is always well formed.
//!
//! - **Splines** (`spline.rs`) are smooth curves through (fit) or guided by (control) a
//!   list of ordinary points, so constraints on the points shape them.
//!
//! # Solving
//! The unknowns are point coordinates and circle radii; each constraint is one or more
//! residual equations with an analytic Jacobian (`equations.rs`). [`Sketch::solve`] runs
//! Levenberg-Marquardt on each independent group of unknowns from the current geometry, so
//! it finds the nearest solution and the sketch does not jump or flip. A failed solve
//! leaves the sketch exactly as it was and says why ([`SolveError`]); conflicting
//! constraints are named. [`Sketch::drag`] moves a point toward a target while keeping the
//! constraints. [`Sketch::analyze`] (also part of every [`SolveReport`]) reports the
//! remaining degrees of freedom, under-, fully or over-constrained status, redundant
//! constraints and which points and curves are fully determined. See `solver.rs` for the
//! method and its tolerances.
//!
//! # Profiles
//! [`Sketch::profiles`] finds the closed regions bounded by non-construction curves: outer
//! loops counter-clockwise, holes clockwise, with exact signed areas. Curves are joined only
//! at shared (or coincident) endpoints; crossings and T-junctions in the middle of a curve
//! are not split by profile detection itself (`profile.rs`); the split tool below cuts
//! crossing curves into pieces that share points, after which their regions are found.
//!
//! # Tools
//! [`Sketch::apply_tool`] runs a [`ToolOp`]: trim, extend, split at crossings, fillet,
//! chamfer, offset, mirror, rectangular and circular patterns, centre rectangle,
//! three-point and tangent arcs (`tools.rs`, with planar intersections in
//! `intersect.rs`). A failed tool changes nothing.
//!
//! Clean room: the solver and profile code are written from textbooks and papers
//! (Madsen, Nielsen and Tingleff on Levenberg-Marquardt; Golub and Van Loan on QR and
//! Cholesky; Tarjan on bridges). Read `skills/chamfera-kernel/SKILL.md` before changing
//! them.

mod constraint;
mod entity;
mod equations;
mod intersect;
mod linalg;
mod profile;
mod sketch;
mod solver;
mod spline;
#[cfg(test)]
mod tests;
mod tools;
#[cfg(test)]
mod tools_tests;

pub use chamfera_geom as geom;
pub use constraint::Constraint;
pub use entity::{ConstraintId, Curve, CurveId, PointId};
pub use profile::{LoopSegment, ProfileLoop, Region};
pub use sketch::{MAX_CONSTRAINTS, MAX_CURVES, MAX_POINTS, Sketch, SketchError};
pub use solver::{
    ConstraintStatus, DETERMINED_TOLERANCE, DofReport, DragReport, MAX_DRAG_STEPS,
    MAX_GROUP_EQUATIONS, MAX_GROUP_UNKNOWNS, MAX_ITERATIONS, SOLVE_TARGET, SolveError, SolveReport,
};
pub use spline::{
    MAX_SPLINE_POINTS, SPLINE_SAMPLES_PER_SPAN, SplineForm, spline_geometry, spline_polyline,
};
pub use tools::{
    MAX_PATTERN_COPIES, MAX_TOOL_CURVES, OffsetCorners, ToolError, ToolOp, ToolReport,
};

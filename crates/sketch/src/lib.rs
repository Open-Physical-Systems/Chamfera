//! Layer 1: 2D sketches and Chamfera's own geometric constraint solver.
//!
//! # The model
//! A [`Sketch`] holds points, curves and constraints, each with a stable id
//! ([`PointId`], [`CurveId`], [`ConstraintId`]) that is never reused.
//! - **Points** own their coordinates (mm).
//! - **Curves** refer to points: a line has a start and an end; a circle a centre and its
//!   own radius; an arc a centre, a start and an end, swept counter-clockwise from start to
//!   end; an ellipse a centre and the ends of its two semi-axes; an elliptical arc those and
//!   a start and an end. Any curve can be construction geometry, which constrains but never bounds a
//!   profile.
//! - **Constraints** ([`Constraint`]) relate entities: coincident, point on line, point on
//!   circle or arc, horizontal, vertical, parallel, perpendicular, tangent, equal, fix,
//!   midpoint, symmetric, concentric, collinear, horizontal and vertical between two points;
//!   and **dimensions**, which carry a plain `f64` value (mm or
//!   radians) that a later milestone drives from parameters: distance, point-line distance,
//!   horizontal and vertical distance, length, radius, diameter, arc length and angle. A
//!   dimension can be driven (a reference dimension): it takes no part in the solve and
//!   shows the size the geometry has ([`Sketch::set_driven`]).
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
//! loops counter-clockwise, holes clockwise, with exact signed areas. Lines, arcs and
//! circles are cut where they cross, meet in a T-junction or overlap, so overlapping shapes
//! give their regions without splitting first (`profile.rs`); the split tool below makes
//! those pieces real curves.
//!
//! # Tools
//! [`Sketch::apply_tool`] runs a [`ToolOp`]: trim, extend, split at crossings, fillet,
//! chamfer, offset, mirror, rectangular and circular patterns, centre rectangle,
//! three-point and tangent arcs (`tools.rs`, with planar intersections in
//! `intersect.rs`), the shapes polygon, slot, ellipse and elliptical arc
//! (`shapes.rs`), the three-point rectangle, circles through two or three points or
//! tangent to curves, and moving, turning, scaling or copying geometry (`placed.rs`). A failed tool changes nothing. Curves fixed in place
//! ([`Sketch::add_fixed_curve`]) carry model geometry projected into a sketch.
//!
//! # Snaps
//! [`Sketch::snap`] finds where a drawing click near existing geometry should land
//! (endpoints, centres, the origin, midpoints, crossings, quadrants, perpendicular and
//! tangent points, on-curve) and [`Sketch::snap_constraints`] the constraints that keep
//! it there (`snap.rs`); [`Sketch::track`] the alignment, perpendicular, tangent and
//! polar guides when no snap is near (`track.rs`).
//!
//! Clean room: the solver and profile code are written from textbooks and papers
//! (Madsen, Nielsen and Tingleff on Levenberg-Marquardt; Golub and Van Loan on QR and
//! Cholesky; Tarjan on bridges). Read `skills/chamfera-kernel/SKILL.md` before changing
//! them.

mod constraint;
mod ellipse;
mod entity;
mod equations;
mod intersect;
mod linalg;
mod placed;
mod profile;
mod shapes;
mod sketch;
mod snap;
mod solver;
mod spline;
#[cfg(test)]
mod tests;
mod tools;
#[cfg(test)]
mod tools_tests;
mod track;

pub use chamfera_geom as geom;
pub use constraint::Constraint;
pub use ellipse::EllipseArc;
pub use entity::{ConstraintId, Curve, CurveId, PointId};
pub use placed::SketchTransform;
pub use profile::{LoopSegment, MAX_SPLIT_CURVES, ProfileLoop, Region};
pub use shapes::{FixedCurve, MAX_POLYGON_SIDES, MIN_POLYGON_SIDES};
pub use sketch::{MAX_CONSTRAINTS, MAX_CURVES, MAX_POINTS, Sketch, SketchError};
pub use snap::{
    MAX_SNAP_CURVES, Snap, SnapKind, SnapKinds, SnapQuery, exact_direction, lock_angle,
    lock_straight, midpoint, perpendicular_foot, quadrant_points, tangent_points,
};
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
pub use track::{Guide, GuideKind, MAX_TRACK_POINTS, Track, TrackQuery, guide_crossing};

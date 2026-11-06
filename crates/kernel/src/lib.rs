//! Layer 1: Chamfera's own boundary-representation (B-rep) kernel.
//!
//! Written from scratch, clean-room: from textbooks, papers and public specifications only
//! (see `skills/chamfera-kernel/SKILL.md` and `docs/kernel-ladder.md`). No code from
//! OpenCASCADE or any other GPL/LGPL kernel is read, copied or linked.
//!
//! Rung K1 is here: curves in space ([`Line3`], [`Segment3`], [`Circle3`], [`Arc3`],
//! [`Ellipse3`], [`EllipticalArc3`]), planar curves in a frame's plane ([`Line2`],
//! [`Segment2`], [`Circle2`], [`Arc2`]) and their pairwise intersections. Frames,
//! transforms and robust predicates (K0) live in `chamfera-geom`.
//!
//! Rung K2 is here: boundary-representation topology ([`Body`] with shells, faces,
//! loops, half-edges, edges and vertices), the Euler operators that build it, validity
//! checks ([`Body::validate_topology`], [`Body::validate`]), builders for a cuboid, a
//! prism with holes and a cylinder's topology, and exact area and volume of planar
//! solids.

mod check;
mod curve3;
mod intersect;
mod planar;
mod topology;

pub use curve3::{
    Arc3, Circle3, ClosestPoint, ELLIPSE_LENGTH_RELATIVE_TOLERANCE, Ellipse3, EllipticalArc3,
    Line3, Segment3,
};
pub use intersect::{
    Contact, IntersectionPoint, Intersections, Overlap, arc_arc, circle_arc, circle_circle,
    line_arc, line_circle, line_line, line_segment, segment_arc, segment_circle, segment_segment,
};
pub use planar::{Arc2, Circle2, Line2, Segment2};
pub use topology::{
    Body, Curve, CurveId, EdgeCurve, EdgeId, FaceId, HalfEdgeId, LoopId, MAX_NAME_LENGTH,
    MAX_PROFILE_VERTICES, MAX_TOPOLOGY_ENTITIES, Mvfs, PointId, ShellId, ShellReport, Surface,
    SurfaceId, TopologyError, TopologyReport, TopologySummary, VertexId,
};

use thiserror::Error;

/// Why a kernel operation could not produce a valid solid. A failing operation never
/// returns a half-built body.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KernelError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error("the result would not be a valid solid: {0}")]
    InvalidResult(String),
    #[error("a numeric method did not converge: {0}")]
    NoConvergence(String),
}

pub type KernelResult<T> = Result<T, KernelError>;

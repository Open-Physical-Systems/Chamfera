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
//! prism with holes and a cylinder, and exact area and volume.
//!
//! Rung K3 is here: analytic surfaces ([`Surface`]: plane, [`Cylinder`], [`Cone`],
//! [`Sphere`], [`Torus`]) with evaluation, normals, projection and closest points; faces on
//! them are validated, classified ([`Body::classify_point_on_face`]) and measured.
//!
//! Rung K4 is here: a planar [`Profile`] of lines and arcs with holes is swept into a valid
//! solid by [`extrude`], [`extrude_symmetric`] and [`revolve`], every face, edge and vertex
//! named from the profile element and role that made it.
//!
//! Rung K6 is here: surface-surface intersection ([`intersect_surfaces`], exact lines,
//! circles and ellipses or an explicit [`SurfaceIntersection`]), curve-surface
//! intersection ([`intersect_curve_surface`]) and point-solid classification
//! ([`classify_point`]), the geometry booleans (K7) need.
//!
//! Rung K7 is here: booleans on solids ([`union`], [`subtract`], [`intersect`]) by face
//! intersection, splitting, classification and sewing, with coplanar and touching cases,
//! multi-lump results and result faces named after the operand faces they came from.
//!
//! Rung K11 (first part) is here: edge blends, constant-radius [`fillet`] and
//! equal-distance [`chamfer`] on straight edges between planes (cylinders, sphere-octant
//! corners) and on plane/cylinder rims (tori, cones).
//!
//! Rung K12 (first part) is here: [`shell`] hollows a solid of planes, cylinders and
//! cones into walls of a thickness, with chosen faces open (an inward offset subtracted).

mod blend;
mod boolean;
mod check;
mod curve3;
mod intersect;
mod planar;
mod profile;
mod shell;
mod ssi;
mod surface;
mod sweep;
mod tessellate;
mod topology;

pub use blend::{MAX_BLEND_EDGES, chamfer, fillet};
pub use boolean::{
    BooleanOp, MAX_BOOLEAN_ENTITIES, MAX_BOOLEAN_FACES, boolean, intersect, subtract, union,
};
pub use curve3::{
    Arc3, Circle3, ClosestPoint, ELLIPSE_LENGTH_RELATIVE_TOLERANCE, Ellipse3, EllipticalArc3,
    Line3, Segment3,
};
pub use intersect::{
    Contact, IntersectionPoint, Intersections, Overlap, arc_arc, circle_arc, circle_circle,
    line_arc, line_circle, line_line, line_segment, segment_arc, segment_circle, segment_segment,
};
pub use planar::{Arc2, Circle2, Line2, Segment2};
pub use profile::{PROFILE_JOIN_TOLERANCE, Profile, ProfileEdge, polygon_edges};
pub use shell::shell;
pub use ssi::{
    CurveSurfaceIntersection, CurveSurfacePoint, IntersectionCurve, ON_SURFACE_SAMPLES,
    PointInSolid, RAY_DIRECTIONS, SurfaceIntersection, UnsupportedIntersection, classify_point,
    intersect_curve_surface, intersect_curve_surface_on, intersect_surfaces,
};
pub use surface::{Cone, Cylinder, Sphere, Surface, SurfaceParameters, SurfacePoint, Torus};
pub use sweep::{extrude, extrude_symmetric, revolve};
pub use tessellate::{BodyMesh, TessOptions, tessellate};
pub use topology::{
    Body, Curve, CurveId, EdgeCurve, EdgeId, FaceId, HalfEdgeId, LoopId, MAX_NAME_LENGTH,
    MAX_PROFILE_VERTICES, MAX_TOPOLOGY_ENTITIES, Mvfs, PointId, PointOnFace,
    SURFACE_QUADRATURE_TOLERANCE, ShellId, ShellReport, SurfaceId, TopologyError, TopologyReport,
    TopologySummary, VertexId,
};
pub(crate) use topology::{ListEdge, ListFace, ListVertex};

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
    #[error("the result is empty: the operation leaves no material")]
    EmptyResult,
}

pub type KernelResult<T> = Result<T, KernelError>;

//! Boundary-representation topology (rung K2): bodies, shells, faces, loops, half-edges,
//! edges and vertices, built and changed only by Euler operators.
//!
//! # Data structure
//! The half-edge structure of Mantyla, *An Introduction to Solid Modeling* (1988),
//! chapter 10, stored in index arenas with typed ids (no reference-counted graphs):
//!
//! - A [`Body`] owns every entity and the geometry they refer to, and holds one or more
//!   [`ShellId`]s. Each shell is a closed, connected, oriented surface: either the outer
//!   boundary of one lump (it encloses positive volume) or a void inside a lump (its faces
//!   point into the empty region, so it encloses negative volume). A body of several lumps
//!   (a disjoint union, K7) has one outer shell per lump; lumps that touch share no
//!   topology.
//! - A shell lists its faces. A face has one outer loop and any number of inner loops
//!   (rings), a surface and an orientation flag.
//! - A loop is a cycle of half-edges (`next`/`prev`). A half-edge is the use of an edge by
//!   a loop (a coedge); it starts at its `origin` vertex and ends at the origin of its
//!   `next`. An edge has exactly two half-edges, its *mates*, which run in opposite
//!   directions. The only half-edge without an edge is the single half-edge of a
//!   one-vertex loop (made by [`Body::mvfs`]).
//! - A vertex knows one half-edge starting at it and its point.
//!
//! # Surface bodies (open shells)
//! A body may also be a *surface body*: one or more open shells, sheets of faces with
//! boundary edges that have a face on one side only. The half-edge structure keeps every
//! edge two-sided by closing each hole of an open shell with a *boundary face*: a face
//! flagged as an opening, with no surface, that is not part of the model (the "boundary
//! half-edges" of Botsch et al., *Polygon Mesh Processing*, 2010, section 2.5, grouped
//! into loops of a face as in Mantyla's lamina). Euler operators, links and
//! Euler-Poincare work unchanged on the closed structure. The public queries hide the
//! boundary faces: [`Body::faces`], [`Body::shell_faces`] and [`Body::summary`] list the
//! real faces only, [`Body::boundary_edges`] the edges on the rim, and [`Body::is_open`]
//! tells a surface body from a solid. [`Body::validate`] refuses open shells;
//! [`Body::validate_as`] with [`Validation::Surface`] accepts them.
//!
//! Orientation: seen from outside the solid (against the outward normal), a face's outer
//! loop runs counter-clockwise and its inner loops clockwise, so the face is always on the
//! left of its half-edges.
//!
//! Geometry is referred to by id: a vertex to a point, an edge to a curve with a parameter
//! range ([`EdgeCurve`]; the edge's first half-edge runs along the curve), a face to a
//! [`Surface`]. Euler operators change topology only; geometry is attached with
//! [`Body::set_edge_curve`], [`Body::set_edge_line`] and [`Body::set_face_surface`].
//!
//! # Invariant
//! Every `Body` value is structurally sound: all links point at live entities and agree
//! with each other, loops are closed cycles, mates run in opposite directions, and the
//! Euler-Poincare formula holds ([`Body::validate_topology`]). Euler operators check their
//! preconditions before changing anything, so a failing operator leaves the body
//! unchanged; deserialization re-checks everything. [`Body::validate`] adds the geometric
//! checks a finished solid must pass.

mod assemble;
mod build;
mod euler;
mod geometry;
mod import;
mod measure;
mod open;
mod spline;
mod transform;
mod uv;
mod validate;

#[cfg(test)]
mod tests;

pub use crate::surface::Surface;
pub(crate) use assemble::{ListEdge, ListFace, ListVertex};
pub use build::MAX_PROFILE_VERTICES;
pub use euler::Mvfs;
pub use geometry::Curve;
pub use measure::SURFACE_QUADRATURE_TOLERANCE;
pub(crate) use measure::open_volume;
pub(crate) use open::FaceList;
pub use uv::PointOnFace;
pub(crate) use uv::{EdgeImage, LoopImage};
pub use validate::{ShellReport, TopologyError, TopologyReport, Validation};

use crate::{KernelError, KernelResult, Segment3};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Most entries one arena of a body may hold (hostile-input limit, also checked on load).
pub const MAX_TOPOLOGY_ENTITIES: usize = 10_000_000;
/// Longest name a face, edge or vertex may carry, in bytes.
pub const MAX_NAME_LENGTH: usize = 256;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            /// The arena index.
            pub fn index(self) -> usize {
                self.0 as usize
            }
            fn from_index(i: usize) -> Self {
                // Arenas never grow past MAX_TOPOLOGY_ENTITIES, far below u32::MAX.
                $name(u32::try_from(i).unwrap_or(u32::MAX))
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

id_type!(
    /// A shell of a [`Body`].
    ShellId
);
id_type!(
    /// A face of a [`Body`].
    FaceId
);
id_type!(
    /// A loop (outer boundary or ring) of a face.
    LoopId
);
id_type!(
    /// A half-edge (coedge): the oriented use of an edge by a loop.
    HalfEdgeId
);
id_type!(
    /// An edge of a [`Body`].
    EdgeId
);
id_type!(
    /// A vertex of a [`Body`].
    VertexId
);
id_type!(
    /// A point in a body's geometry.
    PointId
);
id_type!(
    /// A curve in a body's geometry.
    CurveId
);
id_type!(
    /// A surface in a body's geometry.
    SurfaceId
);

/// Slots of one entity kind; removed entities leave `None` so ids stay stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct Arena<T> {
    items: Vec<Option<T>>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena { items: Vec::new() }
    }
}

impl<T> Arena<T> {
    fn get(&self, i: usize) -> Option<&T> {
        self.items.get(i).and_then(Option::as_ref)
    }
    fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        self.items.get_mut(i).and_then(Option::as_mut)
    }
    /// Callers check room first ([`Body::ensure_room`]).
    fn push(&mut self, t: T) -> usize {
        self.items.push(Some(t));
        self.items.len() - 1
    }
    fn remove(&mut self, i: usize) {
        if let Some(slot) = self.items.get_mut(i) {
            *slot = None;
        }
    }
    fn slots(&self) -> usize {
        self.items.len()
    }
    fn live(&self) -> impl Iterator<Item = (usize, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)))
    }
    fn count(&self) -> usize {
        self.items.iter().filter(|t| t.is_some()).count()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Shell {
    faces: Vec<FaceId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Face {
    shell: ShellId,
    outer: LoopId,
    inner: Vec<LoopId>,
    surface: Option<SurfaceId>,
    /// True when the face's outward normal is the surface normal.
    same_sense: bool,
    name: Option<String>,
    /// True for a boundary face: an opening of an open shell, not part of the model (see
    /// the module documentation). It has no surface.
    #[serde(default, skip_serializing_if = "is_false")]
    boundary: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Loop {
    face: FaceId,
    first: HalfEdgeId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HalfEdge {
    #[serde(rename = "loop")]
    lp: LoopId,
    /// `None` only for the lone half-edge of a one-vertex loop.
    edge: Option<EdgeId>,
    origin: VertexId,
    next: HalfEdgeId,
    prev: HalfEdgeId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    /// `halves[0]` runs along the edge's curve (from `c(t0)` to `c(t1)`), `halves[1]`
    /// against it.
    halves: [HalfEdgeId; 2],
    curve: Option<EdgeCurve>,
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Vertex {
    /// A half-edge starting at this vertex.
    half_edge: HalfEdgeId,
    point: PointId,
    name: Option<String>,
}

/// The geometry of an edge: a curve and the parameter range the edge covers. The edge's
/// first half-edge runs from `curve(t0)` to `curve(t1)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeCurve {
    pub curve: CurveId,
    pub t0: f64,
    pub t1: f64,
}

/// Entity counts of a body (no tessellation involved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TopologySummary {
    pub shells: usize,
    pub faces: usize,
    /// All loops, outer and inner.
    pub loops: usize,
    /// Inner loops only (`loops - faces`).
    pub rings: usize,
    pub edges: usize,
    pub half_edges: usize,
    pub vertices: usize,
}

/// A boundary-representation body: shells of faces bounded by loops of half-edges, with
/// the geometry they lie on. See the module documentation for the structure and the
/// invariant every value keeps.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(try_from = "BodyData")]
pub struct Body {
    shells: Arena<Shell>,
    faces: Arena<Face>,
    loops: Arena<Loop>,
    half_edges: Arena<HalfEdge>,
    edges: Arena<Edge>,
    vertices: Arena<Vertex>,
    points: Arena<Point3>,
    curves: Arena<Curve>,
    surfaces: Arena<Surface>,
}

/// The serialized form of a [`Body`], checked by [`Body::validate_loaded`] on load.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyData {
    shells: Arena<Shell>,
    faces: Arena<Face>,
    loops: Arena<Loop>,
    half_edges: Arena<HalfEdge>,
    edges: Arena<Edge>,
    vertices: Arena<Vertex>,
    points: Arena<Point3>,
    curves: Arena<Curve>,
    surfaces: Arena<Surface>,
}

impl TryFrom<BodyData> for Body {
    type Error = TopologyError;
    fn try_from(d: BodyData) -> Result<Body, TopologyError> {
        let body = Body {
            shells: d.shells,
            faces: d.faces,
            loops: d.loops,
            half_edges: d.half_edges,
            edges: d.edges,
            vertices: d.vertices,
            points: d.points,
            curves: d.curves,
            surfaces: d.surfaces,
        };
        body.validate_loaded()?;
        Ok(body)
    }
}

fn missing(kind: &str, id: impl std::fmt::Display) -> KernelError {
    KernelError::InvalidInput(format!("{kind} {id} does not exist in this body"))
}

fn check_name(name: &str) -> KernelResult<()> {
    if name.len() > MAX_NAME_LENGTH {
        return Err(KernelError::InvalidInput(format!(
            "a name may be at most {MAX_NAME_LENGTH} bytes long"
        )));
    }
    Ok(())
}

// Internal accessors: `Result` for ids that may come from outside.
impl Body {
    fn shell(&self, id: ShellId) -> KernelResult<&Shell> {
        self.shells
            .get(id.index())
            .ok_or_else(|| missing("shell", id))
    }
    fn face(&self, id: FaceId) -> KernelResult<&Face> {
        self.faces
            .get(id.index())
            .ok_or_else(|| missing("face", id))
    }
    fn lp(&self, id: LoopId) -> KernelResult<&Loop> {
        self.loops
            .get(id.index())
            .ok_or_else(|| missing("loop", id))
    }
    fn he(&self, id: HalfEdgeId) -> KernelResult<&HalfEdge> {
        self.half_edges
            .get(id.index())
            .ok_or_else(|| missing("half-edge", id))
    }
    fn edge(&self, id: EdgeId) -> KernelResult<&Edge> {
        self.edges
            .get(id.index())
            .ok_or_else(|| missing("edge", id))
    }
    fn vertex(&self, id: VertexId) -> KernelResult<&Vertex> {
        self.vertices
            .get(id.index())
            .ok_or_else(|| missing("vertex", id))
    }

    /// Fails unless every arena has room for `n` more entries, so an operator can check
    /// once and then add entities without failing half-way.
    fn ensure_room(&self, n: usize) -> KernelResult<()> {
        let most = [
            self.shells.slots(),
            self.faces.slots(),
            self.loops.slots(),
            self.half_edges.slots(),
            self.edges.slots(),
            self.vertices.slots(),
            self.points.slots(),
            self.curves.slots(),
            self.surfaces.slots(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if most + n > MAX_TOPOLOGY_ENTITIES {
            return Err(KernelError::InvalidInput(format!(
                "a body may hold at most {MAX_TOPOLOGY_ENTITIES} entities of each kind"
            )));
        }
        Ok(())
    }

    /// The half-edges of a loop in order from its first one, at most one per half-edge
    /// of the body (so a corrupt cycle cannot loop forever).
    fn walk(&self, first: HalfEdgeId) -> KernelResult<Vec<HalfEdgeId>> {
        let mut out = Vec::new();
        let mut h = first;
        for _ in 0..=self.half_edges.slots() {
            out.push(h);
            h = self.he(h)?.next;
            if h == first {
                return Ok(out);
            }
        }
        Err(KernelError::InvalidResult(format!(
            "the loop through {first} does not close"
        )))
    }

    /// The other half-edge of `h`'s edge.
    fn mate_of(&self, h: HalfEdgeId) -> KernelResult<Option<HalfEdgeId>> {
        let Some(e) = self.he(h)?.edge else {
            return Ok(None);
        };
        let [a, b] = self.edge(e)?.halves;
        Ok(Some(if a == h { b } else { a }))
    }
}

// Public queries.
impl Body {
    /// An empty body; [`Body::mvfs`] starts a shell in it.
    pub fn new() -> Body {
        Body::default()
    }

    /// Entity counts. Boundary faces (the openings of a surface body) and their loops are
    /// not counted, so for an open shell `V - E + F` is the Euler characteristic of the
    /// surface itself, `2 - 2g - b` with `b` holes.
    pub fn summary(&self) -> TopologySummary {
        let faces = self.faces.live().filter(|(_, f)| !f.boundary).count();
        let loops = self
            .faces
            .live()
            .filter(|(_, f)| !f.boundary)
            .map(|(_, f)| 1 + f.inner.len())
            .sum();
        TopologySummary {
            shells: self.shells.count(),
            faces,
            loops,
            rings: loops.saturating_sub(faces),
            edges: self.edges.count(),
            half_edges: self.half_edges.count(),
            vertices: self.vertices.count(),
        }
    }

    /// About how many bytes the body holds: its slots, names, face rings and spline
    /// geometry. Caches use it to stay within their memory limit; it is an estimate, not
    /// an exact allocation count.
    pub fn approx_bytes(&self) -> usize {
        fn slots<T>(a: &Arena<T>) -> usize {
            a.items
                .len()
                .saturating_mul(std::mem::size_of::<Option<T>>())
        }
        fn name(n: &Option<String>) -> usize {
            n.as_ref().map_or(0, String::len)
        }
        // A spline holds its knots, weights and control points (up to three coordinates
        // each), eight bytes per number.
        fn spline(knots: usize, weights: usize) -> usize {
            knots
                .saturating_add(weights.saturating_mul(4))
                .saturating_mul(8)
        }
        let mut heap = 0usize;
        for (_, s) in self.shells.live() {
            heap = heap.saturating_add(s.faces.len().saturating_mul(4));
        }
        for (_, f) in self.faces.live() {
            heap = heap
                .saturating_add(f.inner.len().saturating_mul(4))
                .saturating_add(name(&f.name));
        }
        for (_, e) in self.edges.live() {
            heap = heap.saturating_add(name(&e.name));
        }
        for (_, v) in self.vertices.live() {
            heap = heap.saturating_add(name(&v.name));
        }
        for (_, c) in self.curves.live() {
            if let Curve::BSpline(b) = c {
                heap = heap.saturating_add(spline(b.knots().len(), b.weights().len()));
            }
        }
        for (_, s) in self.surfaces.live() {
            if let Surface::Extrusion(x) = s {
                let b = x.curve();
                heap = heap.saturating_add(spline(b.knots().len(), b.weights().len()));
            }
        }
        [
            slots(&self.shells),
            slots(&self.faces),
            slots(&self.loops),
            slots(&self.half_edges),
            slots(&self.edges),
            slots(&self.vertices),
            slots(&self.points),
            slots(&self.curves),
            slots(&self.surfaces),
            std::mem::size_of::<Body>(),
            heap,
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }

    pub fn shells(&self) -> Vec<ShellId> {
        self.shells
            .live()
            .map(|(i, _)| ShellId::from_index(i))
            .collect()
    }
    /// Every face of the body, in id order (not the boundary faces of an open shell).
    pub fn faces(&self) -> Vec<FaceId> {
        self.faces
            .live()
            .filter(|(_, f)| !f.boundary)
            .map(|(i, _)| FaceId::from_index(i))
            .collect()
    }
    /// The boundary faces: the openings of open shells (none for a solid).
    pub fn boundary_faces(&self) -> Vec<FaceId> {
        self.faces
            .live()
            .filter(|(_, f)| f.boundary)
            .map(|(i, _)| FaceId::from_index(i))
            .collect()
    }
    /// True for a surface body: some shell is open (has boundary edges).
    pub fn is_open(&self) -> bool {
        self.faces.live().any(|(_, f)| f.boundary)
    }
    /// True when `face` is a boundary face (an opening, not part of the model).
    pub fn face_is_boundary(&self, face: FaceId) -> KernelResult<bool> {
        Ok(self.face(face)?.boundary)
    }
    /// True when `e` lies on the rim of an open shell: one of its sides is an opening.
    pub fn edge_is_boundary(&self, e: EdgeId) -> KernelResult<bool> {
        let [a, b] = self.edge_faces(e)?;
        Ok(self.face(a)?.boundary || self.face(b)?.boundary)
    }
    /// The edges on the rims of open shells, in id order.
    pub fn boundary_edges(&self) -> Vec<EdgeId> {
        self.edges()
            .into_iter()
            .filter(|e| self.edge_is_boundary(*e).unwrap_or(false))
            .collect()
    }
    /// The face on the model side of a boundary edge (`None` for an edge that is not on a
    /// rim) and the half-edge it uses.
    pub fn boundary_edge_face(&self, e: EdgeId) -> KernelResult<Option<(FaceId, HalfEdgeId)>> {
        let halves = self.edge(e)?.halves;
        let faces = self.edge_faces(e)?;
        let open = [self.face(faces[0])?.boundary, self.face(faces[1])?.boundary];
        Ok(match open {
            [false, true] => Some((faces[0], halves[0])),
            [true, false] => Some((faces[1], halves[1])),
            _ => None,
        })
    }
    pub fn edges(&self) -> Vec<EdgeId> {
        self.edges
            .live()
            .map(|(i, _)| EdgeId::from_index(i))
            .collect()
    }
    pub fn vertices(&self) -> Vec<VertexId> {
        self.vertices
            .live()
            .map(|(i, _)| VertexId::from_index(i))
            .collect()
    }

    /// The faces of a shell (not its boundary faces).
    pub fn shell_faces(&self, shell: ShellId) -> KernelResult<Vec<FaceId>> {
        let mut out = Vec::new();
        for f in &self.shell(shell)?.faces {
            if !self.face(*f)?.boundary {
                out.push(*f);
            }
        }
        Ok(out)
    }
    /// True when a shell is open (it has boundary faces).
    pub fn shell_is_open(&self, shell: ShellId) -> KernelResult<bool> {
        for f in &self.shell(shell)?.faces {
            if self.face(*f)?.boundary {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn face_shell(&self, face: FaceId) -> KernelResult<ShellId> {
        Ok(self.face(face)?.shell)
    }
    pub fn face_outer_loop(&self, face: FaceId) -> KernelResult<LoopId> {
        Ok(self.face(face)?.outer)
    }
    pub fn face_inner_loops(&self, face: FaceId) -> KernelResult<Vec<LoopId>> {
        Ok(self.face(face)?.inner.clone())
    }
    /// The outer loop followed by the inner loops.
    pub fn face_loops(&self, face: FaceId) -> KernelResult<Vec<LoopId>> {
        let f = self.face(face)?;
        let mut out = vec![f.outer];
        out.extend(f.inner.iter().copied());
        Ok(out)
    }
    /// The edges bounding a face, each once, in loop order (outer loop first).
    pub fn face_edges(&self, face: FaceId) -> KernelResult<Vec<EdgeId>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for l in self.face_loops(face)? {
            for h in self.loop_half_edges(l)? {
                if let Some(e) = self.he(h)?.edge
                    && seen.insert(e)
                {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }
    /// The surface of a face and whether the face's outward normal agrees with it.
    pub fn face_surface(&self, face: FaceId) -> KernelResult<Option<(Surface, bool)>> {
        let f = self.face(face)?;
        let Some(s) = f.surface else {
            return Ok(None);
        };
        let surface = self
            .surfaces
            .get(s.index())
            .ok_or_else(|| missing("surface", s))?;
        Ok(Some((surface.clone(), f.same_sense)))
    }
    pub fn face_name(&self, face: FaceId) -> KernelResult<Option<&str>> {
        Ok(self.face(face)?.name.as_deref())
    }

    pub fn loop_face(&self, lp: LoopId) -> KernelResult<FaceId> {
        Ok(self.lp(lp)?.face)
    }
    /// The half-edges of a loop in order.
    pub fn loop_half_edges(&self, lp: LoopId) -> KernelResult<Vec<HalfEdgeId>> {
        self.walk(self.lp(lp)?.first)
    }
    /// The vertices of a loop in order (the origin of each half-edge).
    pub fn loop_vertices(&self, lp: LoopId) -> KernelResult<Vec<VertexId>> {
        self.loop_half_edges(lp)?
            .into_iter()
            .map(|h| Ok(self.he(h)?.origin))
            .collect()
    }
    /// The first half-edge of `lp` (in loop order) that starts at `v`, if any.
    pub fn loop_half_edge_from(&self, lp: LoopId, v: VertexId) -> KernelResult<Option<HalfEdgeId>> {
        for h in self.loop_half_edges(lp)? {
            if self.he(h)?.origin == v {
                return Ok(Some(h));
            }
        }
        Ok(None)
    }

    pub fn half_edge_loop(&self, h: HalfEdgeId) -> KernelResult<LoopId> {
        Ok(self.he(h)?.lp)
    }
    /// `None` for the lone half-edge of a one-vertex loop.
    pub fn half_edge_edge(&self, h: HalfEdgeId) -> KernelResult<Option<EdgeId>> {
        Ok(self.he(h)?.edge)
    }
    pub fn half_edge_origin(&self, h: HalfEdgeId) -> KernelResult<VertexId> {
        Ok(self.he(h)?.origin)
    }
    /// The vertex a half-edge ends at (the origin of the next one).
    pub fn half_edge_end(&self, h: HalfEdgeId) -> KernelResult<VertexId> {
        Ok(self.he(self.he(h)?.next)?.origin)
    }
    pub fn half_edge_next(&self, h: HalfEdgeId) -> KernelResult<HalfEdgeId> {
        Ok(self.he(h)?.next)
    }
    pub fn half_edge_prev(&self, h: HalfEdgeId) -> KernelResult<HalfEdgeId> {
        Ok(self.he(h)?.prev)
    }
    /// The other half-edge of the same edge (`None` for a one-vertex loop's half-edge).
    pub fn half_edge_mate(&self, h: HalfEdgeId) -> KernelResult<Option<HalfEdgeId>> {
        self.mate_of(h)
    }

    /// The two half-edges of an edge; the first runs along the edge's curve.
    pub fn edge_half_edges(&self, e: EdgeId) -> KernelResult<[HalfEdgeId; 2]> {
        Ok(self.edge(e)?.halves)
    }
    /// Start and end vertex of an edge (in the direction of its first half-edge).
    pub fn edge_vertices(&self, e: EdgeId) -> KernelResult<[VertexId; 2]> {
        let [a, b] = self.edge(e)?.halves;
        Ok([self.he(a)?.origin, self.he(b)?.origin])
    }
    /// The faces on either side of an edge: the face of its first half-edge, then of the
    /// second. They are the same face for an edge used twice by one face (a seam, or a
    /// bridge during construction).
    pub fn edge_faces(&self, e: EdgeId) -> KernelResult<[FaceId; 2]> {
        let [a, b] = self.edge(e)?.halves;
        Ok([self.lp(self.he(a)?.lp)?.face, self.lp(self.he(b)?.lp)?.face])
    }
    /// The edge's curve and range, if geometry has been attached.
    pub fn edge_curve(&self, e: EdgeId) -> KernelResult<Option<(Curve, f64, f64)>> {
        let Some(ec) = self.edge(e)?.curve else {
            return Ok(None);
        };
        let c = self
            .curves
            .get(ec.curve.index())
            .ok_or_else(|| missing("curve", ec.curve))?;
        Ok(Some((c.clone(), ec.t0, ec.t1)))
    }
    pub fn edge_name(&self, e: EdgeId) -> KernelResult<Option<&str>> {
        Ok(self.edge(e)?.name.as_deref())
    }

    pub fn vertex_point(&self, v: VertexId) -> KernelResult<Point3> {
        let p = self.vertex(v)?.point;
        self.points
            .get(p.index())
            .copied()
            .ok_or_else(|| missing("point", p))
    }
    pub fn vertex_name(&self, v: VertexId) -> KernelResult<Option<&str>> {
        Ok(self.vertex(v)?.name.as_deref())
    }
    /// The half-edges leaving a vertex in rotation order (`next(mate(h))` steps from one
    /// to the next, i.e. clockwise seen from outside). A vertex alone in a one-vertex loop
    /// has just that loop's half-edge.
    pub fn vertex_fan(&self, v: VertexId) -> KernelResult<Vec<HalfEdgeId>> {
        let first = self.vertex(v)?.half_edge;
        let mut out = Vec::new();
        let mut h = first;
        for _ in 0..=self.half_edges.slots() {
            out.push(h);
            let Some(m) = self.mate_of(h)? else {
                return Ok(out);
            };
            h = self.he(m)?.next;
            if h == first {
                return Ok(out);
            }
        }
        Err(KernelError::InvalidResult(format!(
            "the half-edges around {v} do not close"
        )))
    }
    /// The edges at a vertex in rotation order.
    pub fn vertex_edges(&self, v: VertexId) -> KernelResult<Vec<EdgeId>> {
        let mut out = Vec::new();
        for h in self.vertex_fan(v)? {
            if let Some(e) = self.he(h)?.edge
                && !out.contains(&e)
            {
                out.push(e);
            }
        }
        Ok(out)
    }
    /// The faces around a vertex in rotation order (each once).
    pub fn vertex_faces(&self, v: VertexId) -> KernelResult<Vec<FaceId>> {
        let mut out = Vec::new();
        for h in self.vertex_fan(v)? {
            let f = self.lp(self.he(h)?.lp)?.face;
            if !out.contains(&f) {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// The box around every vertex and edge (exact for the K1 curves); `None` for an
    /// empty body or one whose geometry is missing or invalid.
    pub fn bounding_box(&self) -> Option<Aabb> {
        let mut out: Option<Aabb> = None;
        let mut add = |b: Aabb| {
            out = Some(match out {
                Some(o) => o.union(b),
                None => b,
            });
        };
        for (i, _) in self.vertices.live() {
            let p = self.vertex_point(VertexId::from_index(i)).ok()?;
            add(Aabb { min: p, max: p });
        }
        for (i, _) in self.edges.live() {
            if let Some((c, t0, t1)) = self.edge_curve(EdgeId::from_index(i)).ok()? {
                add(c.bounding_box_on(t0, t1).ok()?);
            }
        }
        out
    }
}

// Attaching geometry and names.
impl Body {
    /// Puts `edge` on `curve` over `[t0, t1]`, its first half-edge running from
    /// `curve(t0)` to `curve(t1)`. The curve's end points must be the edge's vertex points
    /// within [`LINEAR_TOLERANCE`].
    pub fn set_edge_curve(
        &mut self,
        edge: EdgeId,
        curve: Curve,
        t0: f64,
        t1: f64,
    ) -> KernelResult<()> {
        curve.check_range(t0, t1)?;
        let [v0, v1] = self.edge_vertices(edge)?;
        let (p0, p1) = (self.vertex_point(v0)?, self.vertex_point(v1)?);
        if !(curve.point_at(t0).coincident(p0) && curve.point_at(t1).coincident(p1)) {
            return Err(KernelError::InvalidInput(format!(
                "the curve's end points must be the points of {edge}'s vertices within {LINEAR_TOLERANCE} mm"
            )));
        }
        self.ensure_room(1)?;
        let id = CurveId::from_index(self.curves.push(curve));
        if let Some(e) = self.edges.get_mut(edge.index()) {
            e.curve = Some(EdgeCurve { curve: id, t0, t1 });
        }
        Ok(())
    }

    /// Puts `edge` on the straight segment between its vertices.
    pub fn set_edge_line(&mut self, edge: EdgeId) -> KernelResult<()> {
        let [v0, v1] = self.edge_vertices(edge)?;
        let seg = Segment3::new(self.vertex_point(v0)?, self.vertex_point(v1)?)?;
        self.set_edge_curve(edge, Curve::Segment(seg), 0.0, 1.0)
    }

    /// Puts `face` on `surface`; `same_sense` says whether the face's outward normal is
    /// the surface normal (true) or its opposite. [`Body::validate`] checks that the
    /// face's loops agree with this orientation.
    pub fn set_face_surface(
        &mut self,
        face: FaceId,
        surface: Surface,
        same_sense: bool,
    ) -> KernelResult<()> {
        self.face(face)?;
        self.ensure_room(1)?;
        let id = SurfaceId::from_index(self.surfaces.push(surface));
        if let Some(f) = self.faces.get_mut(face.index()) {
            f.surface = Some(id);
            f.same_sense = same_sense;
        }
        Ok(())
    }

    /// Puts a face on the plane of its outer loop, oriented by the loop: the plane
    /// normal is the loop's vector area direction (Newell's method, exact for the K1
    /// curves), its origin the loop's first vertex.
    pub fn set_face_plane_from_loop(&mut self, face: FaceId) -> KernelResult<()> {
        let outer = self.face(face)?.outer;
        let first = self.loop_vertices(outer)?;
        let origin = match first.first() {
            Some(v) => self.vertex_point(*v)?,
            None => return Err(missing("loop vertex of", face)),
        };
        let area = self.loop_vector_area(outer)?;
        // Scale first: a tiny but valid face has a vector area below the length
        // tolerance that `normalized` would refuse.
        let largest = area.x.abs().max(area.y.abs()).max(area.z.abs());
        let direction = if largest > 0.0 && largest.is_finite() {
            area * (1.0 / largest)
        } else {
            area
        };
        let frame = Frame::from_normal(origin, direction).ok_or_else(|| {
            KernelError::InvalidInput(format!(
                "the outer loop of {face} encloses no area, so it has no plane"
            ))
        })?;
        self.set_face_surface(face, Surface::Plane(frame), true)
    }

    pub fn set_face_name(&mut self, face: FaceId, name: &str) -> KernelResult<()> {
        check_name(name)?;
        let f = self
            .faces
            .get_mut(face.index())
            .ok_or_else(|| missing("face", face))?;
        f.name = Some(name.to_owned());
        Ok(())
    }
    pub fn set_edge_name(&mut self, edge: EdgeId, name: &str) -> KernelResult<()> {
        check_name(name)?;
        let e = self
            .edges
            .get_mut(edge.index())
            .ok_or_else(|| missing("edge", edge))?;
        e.name = Some(name.to_owned());
        Ok(())
    }
    pub fn set_vertex_name(&mut self, vertex: VertexId, name: &str) -> KernelResult<()> {
        check_name(name)?;
        let v = self
            .vertices
            .get_mut(vertex.index())
            .ok_or_else(|| missing("vertex", vertex))?;
        v.name = Some(name.to_owned());
        Ok(())
    }

    /// Half the sum of the area moments of a loop's edges: the loop's vector area
    /// (normal times enclosed area for a planar loop, counter-clockwise about the normal).
    /// Needs every edge of the loop to have a curve.
    pub fn loop_vector_area(&self, lp: LoopId) -> KernelResult<Vec3> {
        let hs = self.loop_half_edges(lp)?;
        let reference = match hs.first() {
            Some(h) => self.vertex_point(self.he(*h)?.origin)?,
            None => return Ok(Vec3::ZERO),
        };
        let mut sum = Vec3::ZERO;
        for h in hs {
            let Some(e) = self.he(h)?.edge else {
                continue;
            };
            let (c, t0, t1) = self
                .edge_curve(e)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve yet")))?;
            let m = c.area_moment(t0, t1, reference);
            sum = if self.edge(e)?.halves[0] == h {
                sum + m
            } else {
                sum - m
            };
        }
        Ok(sum * 0.5)
    }
}

//! The construction behind [`super::chamfer`] and [`super::fillet`].
//!
//! 1. Every selected edge is analysed ([`EdgeBlend`]): its two faces, their outward
//!    normals, the directions into each face, convexity and the setback.
//! 2. Every end vertex of a blended edge is analysed ([`VertexBlend`]) and each *corner*
//!    of a face there (identified by the half-edge leaving the vertex in that face) gets
//!    its new point(s): the meeting point of the two boundary lines of the face, each
//!    moved inward by its setback when its edge is blended; a face with no blended edge
//!    at the vertex (an end face) gets two points joined by the blend's end curve.
//! 3. The new boundary is written as a face list: the old faces with new loops, one
//!    blend face per edge and a corner face per vertex where three blended edges meet,
//!    then linked by `Body::from_face_list` (the K7 assembler) and checked by
//!    [`Body::validate`].
//!
//! New vertices and edges are identified by what made them ([`VKey`], [`EKey`]), so the
//! faces that share one find the same entity.

use super::rim::{AXIS_PARALLEL_TOLERANCE, RimBlend};
use super::{Kind, bounded, ruled};
use crate::topology::{ListEdge, ListFace, ListVertex};
use crate::{
    Body, Circle3, Curve, EdgeId, Ellipse3, FaceId, HalfEdgeId, KernelError, KernelResult,
    Segment3, Sphere, Surface, VertexId,
};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::TAU;

/// Points of a face's sampled boundary per conic edge, for the self-crossing check.
const CONIC_SAMPLES: usize = 64;
/// Most boundary segments one face may have for the quadratic self-crossing check.
const MAX_CHECK_SEGMENTS: usize = 20_000;

pub(super) fn unsupported(msg: impl Into<String>) -> KernelError {
    KernelError::Unsupported(msg.into())
}
pub(super) fn too_large(kind: Kind, msg: impl std::fmt::Display) -> KernelError {
    KernelError::InvalidInput(format!(
        "the {} is too large: {msg}; use a smaller size",
        kind.word()
    ))
}
pub(super) fn internal(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

/// A vertex of the result, by what made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum VKey {
    /// An old vertex no blend touches.
    Orig(VertexId),
    /// The new end, near vertex `v`, of an unblended edge that a blend cuts short.
    OnEdge(EdgeId, VertexId),
    /// The new corner, near vertex `v`, of face `f` where both its edges at `v` are
    /// blended (or a rim's plane face).
    Corner(FaceId, VertexId),
}

/// An edge of the result, by what made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EKey {
    /// An old unblended edge (possibly cut short).
    Orig(EdgeId),
    /// The contact edge of blended edge `e` on face `f`.
    Contact(EdgeId, FaceId),
    /// An end edge of a blend, between two new vertices (ordered).
    Join(VKey, VKey),
}

/// The new point(s) of one face corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Corner {
    One(VKey),
    /// An end face: arriving at the first point, leaving from the second, the blend's end
    /// curve between them.
    Cap(VKey, VKey),
}

/// A planar face: a point on it and its outward unit normal.
#[derive(Debug, Clone, Copy)]
pub(super) struct FacePlane {
    pub origin: Point3,
    pub normal: Vec3,
}

/// A straight edge between two faces ruled along it (planes, cylinders along it,
/// extruded splines along it; see `ruled.rs`).
#[derive(Debug, Clone, Copy)]
pub(super) struct LineBlend {
    /// `faces[0]` holds the edge's first half-edge (running from `ends[0]` to `ends[1]`).
    pub faces: [FaceId; 2],
    pub ends: [VertexId; 2],
    pub points: [Point3; 2],
    /// Unit direction from `ends[0]` to `ends[1]`.
    pub dir: Vec3,
    /// Outward unit normals of the faces at the edge.
    pub normal: [Vec3; 2],
    /// From the edge to each face's contact line at each end: `contact[end][face]`.
    pub contact: [[Vec3; 2]; 2],
    /// From the edge to the ball centre at each end (fillets).
    pub spine: [Vec3; 2],
    /// The fillet radius at each end (different for a variable-radius fillet).
    pub radius: [f64; 2],
    pub convex: bool,
    /// Both faces planar.
    pub planar: bool,
}

impl LineBlend {
    /// True for a variable-radius fillet.
    fn variable(&self) -> bool {
        (self.radius[0] - self.radius[1]).abs() > LINEAR_TOLERANCE
    }
    /// 0 at the edge's first vertex, 1 at its second.
    fn end_index(&self, v: VertexId) -> usize {
        usize::from(v != self.ends[0])
    }
    /// The index of face `f` in `faces`.
    fn face_index(&self, f: FaceId) -> Option<usize> {
        self.faces.iter().position(|g| *g == f)
    }
    /// The contact point on face `faces[i]` at end `end`.
    fn contact_point(&self, end: usize, i: usize) -> Point3 {
        self.points[end] + self.contact[end][i]
    }
    /// The ball centre at end `end`.
    fn ball(&self, end: usize) -> Point3 {
        self.points[end] + self.spine[end]
    }
    /// The unit direction of the contact line on face `faces[i]`, from end 0 to end 1.
    fn contact_dir(&self, i: usize) -> KernelResult<Vec3> {
        unit(
            self.contact_point(1, i) - self.contact_point(0, i),
            "a contact line",
        )
    }
}

/// One blended edge.
#[derive(Debug, Clone, Copy)]
pub(super) enum EdgeBlend {
    Line(LineBlend),
    Rim(RimBlend),
}

impl EdgeBlend {
    fn faces(&self) -> [FaceId; 2] {
        match self {
            EdgeBlend::Line(l) => l.faces,
            EdgeBlend::Rim(r) => r.faces,
        }
    }
    fn ends(&self) -> [VertexId; 2] {
        match self {
            EdgeBlend::Line(l) => l.ends,
            EdgeBlend::Rim(r) => r.ends,
        }
    }
    fn convex(&self) -> bool {
        match self {
            EdgeBlend::Line(l) => l.convex,
            EdgeBlend::Rim(r) => r.convex,
        }
    }
    /// The offset from the edge to the contact line in face `f` (a constant-size blend
    /// between planes).
    fn shift_in(&self, f: FaceId) -> Option<Vec3> {
        match self {
            EdgeBlend::Line(l) => l.face_index(f).map(|i| l.contact[0][i]),
            EdgeBlend::Rim(_) => None,
        }
    }
    /// The contact point on face `f` at end vertex `v` (at point `vp`).
    fn contact_at(&self, f: FaceId, v: VertexId, vp: Point3) -> KernelResult<Point3> {
        match self {
            EdgeBlend::Line(l) => {
                let i = l
                    .face_index(f)
                    .ok_or_else(|| internal("a blend is not on its face"))?;
                Ok(l.contact_point(l.end_index(v), i))
            }
            EdgeBlend::Rim(r) => r.contact_point(f, vp),
        }
    }
    /// The ball centre at end vertex `v` (at point `vp`).
    fn ball_at(&self, v: VertexId, vp: Point3) -> KernelResult<Point3> {
        match self {
            EdgeBlend::Line(l) => Ok(l.ball(l.end_index(v))),
            EdgeBlend::Rim(r) => r.ball_at(vp),
        }
    }
    /// A plain blend between planes: constant size, both faces planar.
    fn planar(&self) -> bool {
        matches!(self, EdgeBlend::Line(l) if l.planar && !l.variable())
    }
}

/// A vertex at the end of at least one blended edge.
#[derive(Debug, Clone)]
struct VertexBlend {
    /// The blended edges at the vertex.
    blended: Vec<EdgeId>,
    /// The faces around the vertex.
    faces: Vec<FaceId>,
    /// The ball touching all three faces (fillets where two or three edges meet).
    center: Option<Point3>,
    /// Where two blends continue each other tangentially: the ball centre there and the
    /// edges' common tangent, the normal of the plane of their common section.
    tangent: Option<(Point3, Vec3)>,
}

/// The blend being built.
pub(super) struct Blend<'a> {
    body: &'a Body,
    kind: Kind,
    size: f64,
    /// The radii at the first and last vertex of the one edge of a variable-radius fillet.
    variable: Option<[f64; 2]>,
    edges: BTreeMap<EdgeId, EdgeBlend>,
    planes: HashMap<FaceId, FacePlane>,
    vertices: BTreeMap<VertexId, VertexBlend>,
    points: BTreeMap<VKey, Point3>,
    /// The new point(s) of the corner of each face at a modified vertex, by the
    /// half-edge leaving the vertex in that face.
    corners: HashMap<HalfEdgeId, Corner>,
}

fn unit(v: Vec3, what: &str) -> KernelResult<Vec3> {
    v.normalized()
        .ok_or_else(|| internal(format!("{what} has no direction")))
}

/// The outward unit normal and a point of a planar face, if it is planar.
pub(super) fn face_plane(body: &Body, f: FaceId) -> KernelResult<Option<FacePlane>> {
    Ok(match body.face_surface(f)? {
        Some((Surface::Plane(frame), same)) => Some(FacePlane {
            origin: frame.origin(),
            normal: if same { frame.z() } else { -frame.z() },
        }),
        _ => None,
    })
}

/// A readable name for an edge: its own, or the faces it separates.
pub(super) fn edge_label(body: &Body, e: EdgeId) -> KernelResult<String> {
    if let Some(n) = body.edge_name(e)? {
        return Ok(n.to_owned());
    }
    let [a, b] = body.edge_faces(e)?;
    Ok(format!(
        "edge between {} and {}",
        face_label(body, a)?,
        face_label(body, b)?
    ))
}
pub(super) fn face_label(body: &Body, f: FaceId) -> KernelResult<String> {
    Ok(body
        .face_name(f)?
        .map_or_else(|| f.to_string(), str::to_owned))
}
pub(super) fn vertex_label(body: &Body, v: VertexId) -> KernelResult<String> {
    Ok(body
        .vertex_name(v)?
        .map_or_else(|| v.to_string(), str::to_owned))
}

/// Whether an edge's curve is a straight line or segment.
fn is_straight(body: &Body, e: EdgeId) -> KernelResult<bool> {
    Ok(matches!(
        body.edge_curve(e)?,
        Some((Curve::Line(_) | Curve::Segment(_), _, _))
    ))
}

/// The meeting point of the lines `p0 + a d0` and `p1 + b d1`, both in the plane with
/// unit normal `n`; `None` when they are parallel.
fn meet_in_plane(p0: Point3, d0: Vec3, p1: Point3, d1: Vec3, n: Vec3) -> Option<Point3> {
    let den = d0.cross(d1).dot(n);
    if den.abs() <= ANGULAR_TOLERANCE * d0.length() * d1.length() || !den.is_finite() {
        return None;
    }
    let a = (p1 - p0).cross(d1).dot(n) / den;
    let p = p0 + d0 * a;
    p.is_valid().then_some(p)
}

/// The point where three planes `n_i · x = c_i` meet (Cramer's rule), or `None` when
/// they do not meet in one point.
fn three_planes(n: [Vec3; 3], c: [f64; 3]) -> Option<Point3> {
    let det = n[0].dot(n[1].cross(n[2]));
    if det.abs() <= ANGULAR_TOLERANCE || !det.is_finite() {
        return None;
    }
    let v =
        (n[1].cross(n[2]) * c[0] + n[2].cross(n[0]) * c[1] + n[0].cross(n[1]) * c[2]) * (1.0 / det);
    let p = Point3::new(v.x, v.y, v.z);
    p.is_valid().then_some(p)
}

/// A conic in a frame: `o + rx cos t x + ry sin t y`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Conic {
    pub frame: Frame,
    pub rx: f64,
    pub ry: f64,
}

impl Conic {
    fn param(&self, p: Point3) -> f64 {
        let l = self.frame.to_local(p);
        (l.y / self.ry).atan2(l.x / self.rx)
    }
    fn curve(&self) -> KernelResult<Curve> {
        if (self.rx - self.ry).abs() <= LINEAR_TOLERANCE * 1e-3 {
            Ok(Curve::Circle(Circle3::new(self.frame, self.rx)?))
        } else {
            Ok(Curve::Ellipse(Ellipse3::new(self.frame, self.rx, self.ry)?))
        }
    }
    /// The arc of the conic between `x` and `y` (both on it) whose middle is nearer
    /// `near`: the curve, its range and whether it runs from `x` to `y`.
    pub(super) fn arc_between(
        &self,
        x: Point3,
        y: Point3,
        near: Point3,
    ) -> KernelResult<(Curve, f64, f64, bool)> {
        let curve = self.curve()?;
        let (tx, ty) = (self.param(x), self.param(y));
        let forward_end = |a: f64, b: f64| {
            let mut b = b;
            while b <= a {
                b += TAU;
            }
            while b - a > TAU {
                b -= TAU;
            }
            b
        };
        let (a1, b1) = (tx, forward_end(tx, ty));
        let (a2, b2) = (ty, forward_end(ty, tx));
        let m1 = curve.point_at(0.5 * (a1 + b1)).distance(near);
        let m2 = curve.point_at(0.5 * (a2 + b2)).distance(near);
        let (t0, t1, from_x) = if m1 <= m2 {
            (a1, b1, true)
        } else {
            (a2, b2, false)
        };
        let (start, end) = if from_x { (x, y) } else { (y, x) };
        if !(curve.point_at(t0).coincident(start) && curve.point_at(t1).coincident(end)) {
            return Err(internal("a blend's end curve misses its end points"));
        }
        Ok((curve, t0, t1, from_x))
    }
}

impl<'a> Blend<'a> {
    pub(super) fn new(
        body: &'a Body,
        selected: &BTreeSet<EdgeId>,
        size: f64,
        kind: Kind,
        variable: Option<[f64; 2]>,
    ) -> KernelResult<Blend<'a>> {
        let mut b = Blend {
            body,
            kind,
            size,
            variable,
            edges: BTreeMap::new(),
            planes: HashMap::new(),
            vertices: BTreeMap::new(),
            points: BTreeMap::new(),
            corners: HashMap::new(),
        };
        for e in selected {
            let blend = b.analyse_edge(*e)?;
            b.edges.insert(*e, blend);
        }
        let ends: BTreeSet<VertexId> = b.edges.values().flat_map(|e| e.ends()).collect();
        for v in ends {
            b.analyse_vertex(v)?;
        }
        Ok(b)
    }

    fn plane(&mut self, f: FaceId) -> KernelResult<Option<FacePlane>> {
        if let Some(p) = self.planes.get(&f) {
            return Ok(Some(*p));
        }
        let p = face_plane(self.body, f)?;
        if let Some(p) = p {
            self.planes.insert(f, p);
        }
        Ok(p)
    }

    fn analyse_edge(&mut self, e: EdgeId) -> KernelResult<EdgeBlend> {
        let body = self.body;
        let label = edge_label(body, e)?;
        let [fa, fb] = body.edge_faces(e)?;
        if fa == fb {
            return Err(unsupported(format!(
                "{label} has the same face on both sides (a seam); seams cannot be blended"
            )));
        }
        let (pa, pb) = (self.plane(fa)?, self.plane(fb)?);
        let Some((curve, _, _)) = body.edge_curve(e)? else {
            return Err(internal(format!("{label} has no curve")));
        };
        let surface_word = |f: FaceId| -> KernelResult<String> {
            Ok(match body.face_surface(f)? {
                Some((Surface::Plane(_), _)) => "a plane",
                Some((Surface::Cylinder(_), _)) => "a cylinder",
                Some((Surface::Cone(_), _)) => "a cone",
                Some((Surface::Sphere(_), _)) => "a sphere",
                Some((Surface::Torus(_), _)) => "a torus",
                Some((Surface::Extrusion(_), _)) => "an extruded spline",
                None => "no surface",
            }
            .to_owned())
        };
        match curve {
            Curve::Line(_) | Curve::Segment(_) => {}
            Curve::Circle(_) | Curve::Arc(_) if pa.is_some() != pb.is_some() => {
                if self.variable.is_some() {
                    return Err(unsupported(format!(
                        "{label}: variable-radius fillets are supported on straight edges between planes only"
                    )));
                }
                return Ok(EdgeBlend::Rim(RimBlend::analyse(
                    body, e, self.kind, self.size,
                )?));
            }
            Curve::Ellipse(_) | Curve::EllipticalArc(_) | Curve::BSpline(_)
                if pa.is_some() || pb.is_some() =>
            {
                return Err(unsupported(format!(
                    "{label} is a curve where a plane meets {} at an angle: the rolling-ball surface along it is neither a cylinder nor a torus but a free-form surface, which needs NURBS blend surfaces the kernel does not have yet",
                    surface_word(if pa.is_some() { fb } else { fa })?
                )));
            }
            _ => {
                return Err(unsupported(format!(
                    "{label} lies between {} and {}: the rolling-ball blend of two curved faces meeting along a curve has no exact surface and needs NURBS blend surfaces the kernel does not have yet (where two equal pipes meet, the faces also touch at two points, where any blend would pinch to nothing)",
                    surface_word(fa)?,
                    surface_word(fb)?
                )));
            }
        }
        let ends = body.edge_vertices(e)?;
        let points = [body.vertex_point(ends[0])?, body.vertex_point(ends[1])?];
        let dir = unit(points[1] - points[0], "a straight edge")?;
        let radii = self.variable.unwrap_or([self.size; 2]);
        let first = ruled::analyse(body, &label, [fa, fb], points[0], dir, self.kind, radii[0])?;
        let r = if (radii[0] - radii[1]).abs() > LINEAR_TOLERANCE {
            if !first.planar {
                return Err(unsupported(format!(
                    "{label}: variable-radius fillets are supported between two planar faces only"
                )));
            }
            ruled::analyse(body, &label, [fa, fb], points[1], dir, self.kind, radii[1])?
        } else {
            first
        };
        Ok(EdgeBlend::Line(LineBlend {
            faces: [fa, fb],
            ends,
            points,
            dir,
            normal: first.normal,
            contact: [first.contact, r.contact],
            spine: [first.spine, r.spine],
            radius: radii,
            convex: first.convex,
            planar: first.planar,
        }))
    }

    fn insert_point(&mut self, key: VKey, p: Point3, v: VertexId) -> KernelResult<()> {
        if let Some(q) = self.points.get(&key) {
            if !q.coincident(p) {
                return Err(unsupported(format!(
                    "the blends at {} need different setbacks on one edge (faces meeting at unequal angles); such corners are not supported yet",
                    vertex_label(self.body, v)?
                )));
            }
            return Ok(());
        }
        self.points.insert(key, p);
        Ok(())
    }

    fn analyse_vertex(&mut self, v: VertexId) -> KernelResult<()> {
        let body = self.body;
        let label = vertex_label(body, v)?;
        let fan = body.vertex_fan(v)?;
        let edges = body.vertex_edges(v)?;
        let faces = body.vertex_faces(v)?;
        let blended: Vec<EdgeId> = edges
            .iter()
            .copied()
            .filter(|e| self.edges.contains_key(e))
            .collect();
        if let Some(rim) = blended.iter().find_map(|e| match self.edges.get(e) {
            Some(EdgeBlend::Rim(r)) if r.closed() => Some(*r),
            _ => None,
        }) {
            let corners = rim.corners(body, &fan)?;
            for (h, corner, key, p) in corners {
                self.insert_point(key, p, v)?;
                self.corners.insert(h, corner);
            }
            self.vertices.insert(
                v,
                VertexBlend {
                    blended,
                    faces,
                    center: None,
                    tangent: None,
                },
            );
            return Ok(());
        }
        if fan.len() != 3 || edges.len() != 3 || faces.len() != 3 {
            return Err(unsupported(format!(
                "{label} has {} edges; blends end only at vertices with three edges and three faces so far",
                edges.len()
            )));
        }
        let convexity: BTreeSet<bool> = blended
            .iter()
            .filter_map(|e| self.edges.get(e).map(EdgeBlend::convex))
            .collect();
        if blended.len() >= 2 && convexity.len() > 1 {
            return Err(unsupported(format!(
                "blended edges at {label} are convex and concave; such mixed corners are not supported yet"
            )));
        }
        match blended.as_slice() {
            [e] => return self.analyse_end(v, *e, &fan, blended.clone(), faces),
            [a, b] => {
                if let Some(tangent) = self.tangent_join(v, *a, *b)? {
                    return self.analyse_tangent_join(v, [*a, *b], tangent, &fan, faces);
                }
            }
            _ => {}
        }
        let mut planes = Vec::with_capacity(3);
        for f in &faces {
            let Some(p) = self.plane(*f)? else {
                return Err(unsupported(format!(
                    "blends meet at {label}, where {} is not planar; corners of several blends are supported between planes, or where blends continue each other tangentially",
                    face_label(body, *f)?
                )));
            };
            planes.push(p);
        }
        for e in &edges {
            if !is_straight(body, *e)? {
                return Err(unsupported(format!(
                    "blends meet at {label}, where {} is curved; corners of several blends are supported between straight edges, or where blends continue each other tangentially",
                    edge_label(body, *e)?
                )));
            }
        }
        if blended
            .iter()
            .any(|e| !self.edges.get(e).is_some_and(EdgeBlend::planar))
        {
            return Err(unsupported(format!(
                "blends meet at {label} where one of them is not a constant blend between planes"
            )));
        }
        let vp = body.vertex_point(v)?;
        // Every corner of a face at v: the meeting point of its two edge lines, each
        // moved by its setback when blended.
        for h in &fan {
            let f = body.loop_face(body.half_edge_loop(*h)?)?;
            let plane = self
                .plane(f)?
                .ok_or_else(|| internal("a face lost its plane"))?;
            let h_in = body.half_edge_prev(*h)?;
            let (Some(e_in), Some(e_out)) = (body.half_edge_edge(h_in)?, body.half_edge_edge(*h)?)
            else {
                return Err(internal(format!("a loop at {label} has no edges")));
            };
            let d_in = body.vertex_point(body.half_edge_origin(h_in)?)? - vp;
            let d_out = body.vertex_point(body.half_edge_end(*h)?)? - vp;
            let s_in = self.edges.get(&e_in).and_then(|b| b.shift_in(f));
            let s_out = self.edges.get(&e_out).and_then(|b| b.shift_in(f));
            let meet = |a: Vec3, b: Vec3| {
                meet_in_plane(vp + a, d_in, vp + b, d_out, plane.normal)
                    .ok_or_else(|| unsupported(format!("two edges at {label} are collinear")))
            };
            let corner = match (s_in, s_out) {
                (Some(a), Some(b)) => {
                    let key = VKey::Corner(f, v);
                    self.insert_point(key, meet(a, b)?, v)?;
                    Corner::One(key)
                }
                (Some(a), None) => {
                    let key = VKey::OnEdge(e_out, v);
                    self.insert_point(key, meet(a, Vec3::ZERO)?, v)?;
                    Corner::One(key)
                }
                (None, Some(b)) => {
                    let key = VKey::OnEdge(e_in, v);
                    self.insert_point(key, meet(Vec3::ZERO, b)?, v)?;
                    Corner::One(key)
                }
                (None, None) => Corner::Cap(VKey::OnEdge(e_in, v), VKey::OnEdge(e_out, v)),
            };
            self.corners.insert(*h, corner);
        }
        let mut center = None;
        if self.kind == Kind::Fillet && blended.len() >= 2 {
            // The ball touching all three planes: n_F · x = n_F · o_F - side r.
            let side = if convexity.contains(&true) { 1.0 } else { -1.0 };
            let n = [planes[0].normal, planes[1].normal, planes[2].normal];
            let c = [0, 1, 2].map(|i| {
                planes[i].normal.dot(planes[i].origin - Point3::ORIGIN) - side * self.size
            });
            let p = three_planes(n, c)
                .ok_or_else(|| unsupported(format!("the faces at {label} do not form a corner")))?;
            if blended.len() == 3 {
                // Each face corner must be the ball's touching point on that face.
                for (f, pl) in faces.iter().zip(&planes) {
                    let foot = p + pl.normal * (side * self.size);
                    let at = self.points.get(&VKey::Corner(*f, v)).copied();
                    if !at.is_some_and(|q| q.distance(foot) <= 1e3 * LINEAR_TOLERANCE) {
                        return Err(internal(format!(
                            "the corner ball at {label} does not touch its faces at their corners"
                        )));
                    }
                }
            }
            center = Some(p);
        }
        self.vertices.insert(
            v,
            VertexBlend {
                blended,
                faces,
                center,
                tangent: None,
            },
        );
        Ok(())
    }

    /// One blended edge `e` ends at `v`; the third face there (the end face) must be a
    /// plane. Each face of `e` gets the point where its contact curve meets the end face's
    /// plane (on the unblended edge between them); the end face gets the blend's end curve.
    fn analyse_end(
        &mut self,
        v: VertexId,
        e: EdgeId,
        fan: &[HalfEdgeId],
        blended: Vec<EdgeId>,
        faces: Vec<FaceId>,
    ) -> KernelResult<()> {
        let body = self.body;
        let label = vertex_label(body, v)?;
        let blend = *self
            .edges
            .get(&e)
            .ok_or_else(|| internal("a blended edge vanished"))?;
        let own = blend.faces();
        let Some(&end) = faces.iter().find(|f| !own.contains(f)) else {
            return Err(internal("a blend end has no end face"));
        };
        let Some(plane) = self.plane(end)? else {
            return Err(unsupported(format!(
                "the blend of {} ends at {label} on {}, which is not planar; a blend can end only on a planar face, or continue tangentially into another blended edge",
                edge_label(body, e)?,
                face_label(body, end)?
            )));
        };
        let vp = body.vertex_point(v)?;
        if let EdgeBlend::Rim(r) = &blend {
            // An arc's blend ends in its meridian section, so the end face must be the
            // plane through the axis and the vertex.
            let off = (r.center() - plane.origin).dot(plane.normal).abs();
            if r.axis().dot(plane.normal).abs() > AXIS_PARALLEL_TOLERANCE || off > LINEAR_TOLERANCE
            {
                return Err(unsupported(format!(
                    "the blend of the arc {} ends at {label} on {}, which is not a plane through the arc's axis; it can end only there, or continue tangentially into another blended edge",
                    edge_label(body, e)?,
                    face_label(body, end)?
                )));
            }
        }
        for h in fan {
            let f = body.loop_face(body.half_edge_loop(*h)?)?;
            let h_in = body.half_edge_prev(*h)?;
            let (Some(e_in), Some(e_out)) = (body.half_edge_edge(h_in)?, body.half_edge_edge(*h)?)
            else {
                return Err(internal(format!("a loop at {label} has no edges")));
            };
            if f == end {
                self.corners.insert(
                    *h,
                    Corner::Cap(VKey::OnEdge(e_in, v), VKey::OnEdge(e_out, v)),
                );
                continue;
            }
            let other = if e_in == e { e_out } else { e_in };
            let p = match &blend {
                EdgeBlend::Rim(r) => r.contact_point(f, vp)?,
                EdgeBlend::Line(l) => {
                    let i = l
                        .face_index(f)
                        .ok_or_else(|| internal("a blend is not on its face"))?;
                    let q = l.contact_point(l.end_index(v), i);
                    let d = l.contact_dir(i)?;
                    let cos = d.dot(plane.normal);
                    if cos.abs() <= ANGULAR_TOLERANCE.sqrt() {
                        return Err(unsupported(format!(
                            "the end face {} of {} is parallel to it",
                            face_label(body, end)?,
                            edge_label(body, e)?
                        )));
                    }
                    q + d * ((plane.origin - q).dot(plane.normal) / cos)
                }
            };
            let key = VKey::OnEdge(other, v);
            self.insert_point(key, p, v)?;
            self.corners.insert(*h, Corner::One(key));
        }
        self.vertices.insert(
            v,
            VertexBlend {
                blended,
                faces,
                center: None,
                tangent: None,
            },
        );
        Ok(())
    }

    /// The common unit tangent at `v` when blended edges `a` and `b` continue each other
    /// there: they share a face, their tangents at `v` are parallel and their other two
    /// faces are tangent to each other at `v` (a G1 join of a profile, such as a slot's
    /// line meeting its end arc).
    fn tangent_join(&self, v: VertexId, a: EdgeId, b: EdgeId) -> KernelResult<Option<Vec3>> {
        let body = self.body;
        let (Some(ba), Some(bb)) = (self.edges.get(&a), self.edges.get(&b)) else {
            return Ok(None);
        };
        let tangent = |e: EdgeId| -> KernelResult<Option<Vec3>> {
            let [v0, _] = body.edge_vertices(e)?;
            let Some((c, t0, t1)) = body.edge_curve(e)? else {
                return Ok(None);
            };
            Ok(c.d1(if v0 == v { t0 } else { t1 }).normalized())
        };
        let (Some(ta), Some(tb)) = (tangent(a)?, tangent(b)?) else {
            return Ok(None);
        };
        if ta.cross(tb).length() > AXIS_PARALLEL_TOLERANCE {
            return Ok(None);
        }
        let (fa, fb) = (ba.faces(), bb.faces());
        let Some(&shared) = fa.iter().find(|f| fb.contains(f)) else {
            return Ok(None);
        };
        let (Some(&oa), Some(&ob)) = (
            fa.iter().find(|f| **f != shared),
            fb.iter().find(|f| **f != shared),
        ) else {
            return Ok(None);
        };
        let vp = body.vertex_point(v)?;
        let normal = |f: FaceId| -> KernelResult<Option<Vec3>> {
            let Some((s, same)) = body.face_surface(f)? else {
                return Ok(None);
            };
            let q = s.parameters(vp);
            let n = s.normal_at(q.u, q.v);
            Ok(Some(if same { n } else { -n }))
        };
        let (Some(na), Some(nb)) = (normal(oa)?, normal(ob)?) else {
            return Ok(None);
        };
        if na.cross(nb).length() > AXIS_PARALLEL_TOLERANCE || na.dot(nb) <= 0.0 {
            return Ok(None);
        }
        Ok(Some(ta))
    }

    /// Two blends continuing each other tangentially at `v`: both have the same section
    /// there (the same ball and the same contact points), which becomes their common end
    /// curve. The shared face's corner is the common contact point on it; the unblended
    /// edge between the other two faces is cut short at their common contact point.
    fn analyse_tangent_join(
        &mut self,
        v: VertexId,
        pair: [EdgeId; 2],
        tangent: Vec3,
        fan: &[HalfEdgeId],
        faces: Vec<FaceId>,
    ) -> KernelResult<()> {
        let body = self.body;
        let label = vertex_label(body, v)?;
        let vp = body.vertex_point(v)?;
        let mut balls = Vec::with_capacity(2);
        for e in pair {
            let b = self
                .edges
                .get(&e)
                .ok_or_else(|| internal("a blended edge vanished"))?;
            balls.push(b.ball_at(v, vp)?);
        }
        if self.kind == Kind::Fillet
            && let [p, q] = balls[..]
            && !p.coincident(q)
        {
            return Err(unsupported(format!(
                "the fillets meeting tangentially at {label} have different sections there (faces at different angles); this is not supported yet"
            )));
        }
        for h in fan {
            let f = body.loop_face(body.half_edge_loop(*h)?)?;
            let h_in = body.half_edge_prev(*h)?;
            let (Some(e_in), Some(e_out)) = (body.half_edge_edge(h_in)?, body.half_edge_edge(*h)?)
            else {
                return Err(internal(format!("a loop at {label} has no edges")));
            };
            let in_blended = self.edges.contains_key(&e_in);
            let out_blended = self.edges.contains_key(&e_out);
            let (key, by) = match (in_blended, out_blended) {
                (true, true) => (VKey::Corner(f, v), e_in),
                (true, false) => (VKey::OnEdge(e_out, v), e_in),
                (false, true) => (VKey::OnEdge(e_in, v), e_out),
                (false, false) => {
                    return Err(internal(format!(
                        "a face at the tangent join {label} has no blended edge"
                    )));
                }
            };
            let b = self
                .edges
                .get(&by)
                .ok_or_else(|| internal("a blended edge vanished"))?;
            let p = b.contact_at(f, v, vp)?;
            self.insert_point(key, p, v)?;
            if in_blended && out_blended {
                let other = self
                    .edges
                    .get(&e_out)
                    .ok_or_else(|| internal("a blended edge vanished"))?;
                self.insert_point(key, other.contact_at(f, v, vp)?, v)?;
            }
            self.corners.insert(*h, Corner::One(key));
        }
        let center = balls.first().copied().unwrap_or(vp);
        self.vertices.insert(
            v,
            VertexBlend {
                blended: pair.to_vec(),
                faces,
                center: None,
                tangent: Some((center, tangent)),
            },
        );
        Ok(())
    }

    fn point(&self, k: VKey) -> KernelResult<Point3> {
        self.points
            .get(&k)
            .copied()
            .ok_or_else(|| unsupported("a blend corner could not be resolved"))
    }

    /// The single new point of face `f`'s corner at `v` on a blended edge.
    fn corner_key(&self, e: EdgeId, f: FaceId, v: VertexId) -> KernelResult<VKey> {
        let body = self.body;
        let [h0, h1] = body.edge_half_edges(e)?;
        // The half-edge of `f` leaving `v` along or into `e`: the one of `e` in `f`
        // when it starts at `v`, else the next one after it.
        for h in [h0, h1] {
            if body.loop_face(body.half_edge_loop(h)?)? != f {
                continue;
            }
            let leaving = if body.half_edge_origin(h)? == v {
                h
            } else {
                body.half_edge_next(h)?
            };
            return match self.corners.get(&leaving) {
                Some(Corner::One(k)) => Ok(*k),
                _ => Err(internal("a blended edge's corner has two points")),
            };
        }
        Err(internal("a blended edge is not in its face"))
    }

    /// Builds the result.
    pub(super) fn run(self) -> KernelResult<Body> {
        let mut spec = Spec::default();
        let body = self.body;
        let word = self.kind.word();

        // Vertices: old ones keep their names; new ones are named from what made them.
        for v in body.vertices() {
            if !self.vertices.contains_key(&v) {
                spec.vertex(
                    VKey::Orig(v),
                    body.vertex_point(v)?,
                    body.vertex_name(v)?.map(str::to_owned),
                );
            }
        }
        for (k, p) in &self.points {
            let name = match k {
                VKey::Orig(_) => None,
                VKey::OnEdge(e, v) => Some(format!(
                    "{word} of {} at {}",
                    edge_label(body, *e)?,
                    vertex_label(body, *v)?
                )),
                VKey::Corner(f, v) => Some(format!(
                    "{word} of {} at {}",
                    face_label(body, *f)?,
                    vertex_label(body, *v)?
                )),
            };
            spec.vertex(*k, *p, name.map(bounded));
        }

        // Old unblended edges, cut short where a blend reaches them.
        for e in body.edges() {
            if self.edges.contains_key(&e) {
                continue;
            }
            let [v0, v1] = body.edge_vertices(e)?;
            let key_at = |v: VertexId| {
                if self.vertices.contains_key(&v) {
                    VKey::OnEdge(e, v)
                } else {
                    VKey::Orig(v)
                }
            };
            let (k0, k1) = (key_at(v0), key_at(v1));
            let name = body.edge_name(e)?.map(str::to_owned);
            let (curve, t0, t1) = body
                .edge_curve(e)?
                .ok_or_else(|| internal(format!("{e} has no curve")))?;
            if k0 == VKey::Orig(v0) && k1 == VKey::Orig(v1) {
                spec.edge(EKey::Orig(e), k0, k1, curve, t0, t1, name)?;
                continue;
            }
            let (p0, p1) = (self.point_or(&spec, k0)?, self.point_or(&spec, k1)?);
            let consumed = || {
                Ok::<_, KernelError>(too_large(
                    self.kind,
                    format!("it would consume {}", edge_label(body, e)?),
                ))
            };
            if matches!(curve, Curve::Line(_) | Curve::Segment(_)) {
                let old = body.vertex_point(v1)? - body.vertex_point(v0)?;
                let new = p1 - p0;
                if new.dot(old) <= LINEAR_TOLERANCE * old.length()
                    || new.length() <= LINEAR_TOLERANCE
                {
                    return Err(consumed()?);
                }
                let seg = Segment3::new(p0, p1)?;
                spec.edge(EKey::Orig(e), k0, k1, Curve::Segment(seg), 0.0, 1.0, name)?;
                continue;
            }
            // A curved edge keeps its curve over a shorter range.
            if v0 == v1 {
                return Err(unsupported(format!(
                    "a blend would cut the closed edge {}",
                    edge_label(body, e)?
                )));
            }
            let s0 = if k0 == VKey::Orig(v0) {
                t0
            } else {
                param_on(&curve, t0, t1, p0)?
            };
            let s1 = if k1 == VKey::Orig(v1) {
                t1
            } else {
                param_on(&curve, t0, t1, p1)?
            };
            if s0.partial_cmp(&s1) != Some(std::cmp::Ordering::Less) || p0.coincident(p1) {
                return Err(consumed()?);
            }
            spec.edge(EKey::Orig(e), k0, k1, curve, s0, s1, name)?;
        }

        // Contact edges and end edges of each blend.
        for (e, blend) in &self.edges {
            let label = edge_label(body, *e)?;
            let faces = blend.faces();
            let [v0, v1] = blend.ends();
            for f in faces {
                let (k0, k1) = (self.corner_key(*e, f, v0)?, self.corner_key(*e, f, v1)?);
                let name = Some(bounded(format!(
                    "{word} of {label} on {}",
                    face_label(body, f)?
                )));
                let (curve, t0, t1) = match blend {
                    EdgeBlend::Line(l) => {
                        let (p0, p1) = (self.point(k0)?, self.point(k1)?);
                        let d = p1 - p0;
                        if d.dot(l.dir) <= LINEAR_TOLERANCE || d.length() <= LINEAR_TOLERANCE {
                            return Err(too_large(
                                self.kind,
                                format!("it would consume {}", face_label(body, f)?),
                            ));
                        }
                        (Curve::Segment(Segment3::new(p0, p1)?), 0.0, 1.0)
                    }
                    EdgeBlend::Rim(r) => r.contact_curve(f, self.point(k0)?, self.point(k1)?)?,
                };
                spec.edge(EKey::Contact(*e, f), k0, k1, curve, t0, t1, name)?;
            }
            for v in [v0, v1] {
                let x = self.corner_key(*e, faces[0], v)?;
                let y = self.corner_key(*e, faces[1], v)?;
                let key = join(x, y);
                if spec.edges.contains_key(&key) {
                    continue;
                }
                let (px, py) = (self.point(x)?, self.point(y)?);
                let (curve, t0, t1, from_x) = self.end_curve(*e, blend, v, px, py)?;
                let (from, to) = if from_x { (x, y) } else { (y, x) };
                let name = match blend {
                    EdgeBlend::Rim(r) if r.closed() => format!("{word} of {label} seam"),
                    _ => {
                        format!("{word} of {label} at {}", vertex_label(body, v)?)
                    }
                };
                spec.edge(key, from, to, curve, t0, t1, Some(bounded(name)))?;
            }
        }

        // The old faces with their new loops.
        let mut modified = Vec::new();
        for f in body.faces() {
            let (surface, same_sense) = body
                .face_surface(f)?
                .ok_or_else(|| internal(format!("{f} has no surface")))?;
            let mut loops = Vec::new();
            let mut touched = false;
            for lp in body.face_loops(f)? {
                let mut out = Vec::new();
                for h in body.loop_half_edges(lp)? {
                    let e = body
                        .half_edge_edge(h)?
                        .ok_or_else(|| internal("a loop of a solid has no edge"))?;
                    let along = body.edge_half_edges(e)?[0] == h;
                    if self.edges.contains_key(&e) {
                        touched = true;
                        out.push(spec.coedge_dir(EKey::Contact(e, f), along)?);
                    } else {
                        out.push(spec.coedge_dir(EKey::Orig(e), along)?);
                    }
                    let next = body.half_edge_next(h)?;
                    if let Some(c) = self.corners.get(&next) {
                        touched = true;
                        if let Corner::Cap(a, b) = c {
                            out.push(spec.coedge_from(join(*a, *b), *a)?);
                        }
                    }
                }
                loops.push(out);
            }
            if touched {
                modified.push(spec.faces.len());
            }
            spec.faces.push(SpecFace {
                surface,
                same_sense,
                name: body.face_name(f)?.map(str::to_owned),
                loops,
            });
        }

        // One blend face per edge.
        for (e, blend) in &self.edges {
            let [fa, fb] = blend.faces();
            let [v0, v1] = blend.ends();
            let (a0, a1) = (self.corner_key(*e, fa, v0)?, self.corner_key(*e, fa, v1)?);
            let (b0, b1) = (self.corner_key(*e, fb, v0)?, self.corner_key(*e, fb, v1)?);
            let lp = vec![
                spec.coedge_dir(EKey::Contact(*e, fa), false)?,
                spec.coedge_from(join(a0, b0), a0)?,
                spec.coedge_dir(EKey::Contact(*e, fb), true)?,
                spec.coedge_from(join(b1, a1), b1)?,
            ];
            let (surface, same_sense) = match blend {
                EdgeBlend::Line(l) => match self.kind {
                    Kind::Chamfer => (plane_of(&spec, &lp)?, true),
                    Kind::Fillet => (fillet_surface(l)?, l.convex),
                },
                EdgeBlend::Rim(r) => r.surface()?,
            };
            if self.kind == Kind::Chamfer
                && let EdgeBlend::Line(l) = blend
                && let Surface::Plane(fr) = surface
                && fr.z().dot(l.normal[0] + l.normal[1]) <= 0.0
            {
                return Err(internal("a chamfer face points inward"));
            }
            spec.faces.push(SpecFace {
                surface,
                same_sense,
                name: Some(bounded(format!("{word} of {}", edge_label(body, *e)?))),
                loops: vec![lp],
            });
        }

        // A corner face where three blended edges meet.
        for (v, vb) in &self.vertices {
            if vb.blended.len() < 3 {
                continue;
            }
            let mut pieces = Vec::new();
            for e in &vb.blended {
                let blend = self
                    .edges
                    .get(e)
                    .ok_or_else(|| internal("a blended edge vanished"))?;
                let [fa, fb] = blend.faces();
                let [v0, _] = blend.ends();
                let (x, y) = (self.corner_key(*e, fa, *v)?, self.corner_key(*e, fb, *v)?);
                // The blend face runs x -> y at its start vertex and y -> x at its end; the
                // corner face uses the edge the other way.
                pieces.push(if *v == v0 { (y, x) } else { (x, y) });
            }
            let mut lp = Vec::new();
            let Some(&(first, _)) = pieces.first() else {
                continue;
            };
            let mut at = first;
            for _ in 0..pieces.len() {
                let Some(&(from, to)) = pieces.iter().find(|(from, _)| *from == at) else {
                    return Err(internal("a corner face does not close"));
                };
                lp.push(spec.coedge_from(join(from, to), from)?);
                at = to;
            }
            if at != first {
                return Err(internal("a corner face does not close"));
            }
            let (surface, same_sense) = match self.kind {
                Kind::Chamfer => (plane_of(&spec, &lp)?, true),
                Kind::Fillet => {
                    let p = vb
                        .center
                        .ok_or_else(|| internal("a fillet corner has no ball"))?;
                    // The chart a revolve gives an octant: the pole at the first face's
                    // corner, so the two arcs through it are meridians, and the seam
                    // (u = 0) opposite the other two corners.
                    let mut dirs = Vec::with_capacity(3);
                    for f in &vb.faces {
                        dirs.push(unit(self.point(VKey::Corner(*f, *v))? - p, "a corner")?);
                    }
                    let [pole, a, b] = dirs[..] else {
                        return Err(internal("a fillet corner needs three faces"));
                    };
                    let frame = Frame::from_normal_and_x(p, pole, -(a + b))
                        .ok_or_else(|| internal("a corner frame is degenerate"))?;
                    let convex = vb
                        .blended
                        .first()
                        .and_then(|e| self.edges.get(e))
                        .is_some_and(EdgeBlend::convex);
                    (Surface::Sphere(Sphere::new(frame, self.size)?), convex)
                }
            };
            spec.faces.push(SpecFace {
                surface,
                same_sense,
                name: Some(bounded(format!(
                    "{word} corner at {}",
                    vertex_label(body, *v)?
                ))),
                loops: vec![lp],
            });
        }

        let out = spec.build()?;
        out.validate()?;
        let faces = out.faces();
        for i in modified {
            if let Some(f) = faces.get(i) {
                check_face_simple(&out, *f, self.kind)?;
            }
        }
        Ok(out)
    }

    fn point_or(&self, spec: &Spec, k: VKey) -> KernelResult<Point3> {
        match spec.vertices.get(&k) {
            Some(i) => spec
                .list
                .get(*i)
                .map(|v| v.point)
                .ok_or_else(|| internal("a vertex vanished")),
            None => self.point(k),
        }
    }

    /// The end curve of `e`'s blend at `v`, between its corner points `x` (on the
    /// edge's first face) and `y`.
    fn end_curve(
        &self,
        e: EdgeId,
        blend: &EdgeBlend,
        v: VertexId,
        x: Point3,
        y: Point3,
    ) -> KernelResult<(Curve, f64, f64, bool)> {
        let body = self.body;
        let vp = body.vertex_point(v)?;
        let line = match blend {
            // A closed rim's seam, or an arc's meridian section at its end (on a plane
            // through its axis, or where it continues tangentially).
            EdgeBlend::Rim(r) => return r.section_curve(vp, x, y),
            EdgeBlend::Line(l) => l,
        };
        if self.kind == Kind::Chamfer {
            return Ok((Curve::Segment(Segment3::new(x, y)?), 0.0, 1.0, true));
        }
        let vb = self
            .vertices
            .get(&v)
            .ok_or_else(|| internal("a blend end lost its vertex"))?;
        if let Some((p, t)) = vb.tangent {
            // The common section of two blends continuing each other: the circle of the
            // ball there, in the plane normal to the edges' common tangent.
            let r = line.radius[line.end_index(v)];
            let frame = Frame::from_normal_and_x(p, t, x - p)
                .ok_or_else(|| internal("a tangent join's section is degenerate"))?;
            return Conic {
                frame,
                rx: r,
                ry: r,
            }
            .arc_between(x, y, vp);
        }
        let r = self.size;
        let c0 = line.ball(0);
        match vb.blended.len() {
            1 => {
                // The end face: the one at v that the blended edge does not bound.
                let Some(end) = vb.faces.iter().find(|f| !line.faces.contains(f)) else {
                    return Err(internal("a blend end has no end face"));
                };
                let plane = self
                    .planes
                    .get(end)
                    .ok_or_else(|| internal("an end face lost its plane"))?;
                if line.variable() {
                    return cone_section(line, plane, x, y, vp);
                }
                let n = plane.normal;
                let cos = line.dir.dot(n);
                if cos.abs() <= ANGULAR_TOLERANCE.sqrt() {
                    return Err(unsupported(format!(
                        "the end face of {} is parallel to it",
                        edge_label(body, e)?
                    )));
                }
                let lambda = (plane.origin - c0).dot(n) / cos;
                let center = c0 + line.dir * lambda;
                section(center, n, line.dir, r)?.arc_between(x, y, vp)
            }
            2 => {
                let p = vb
                    .center
                    .ok_or_else(|| internal("a fillet corner has no ball"))?;
                let Some(other) = vb.blended.iter().find(|o| **o != e) else {
                    return Err(internal("a two-blend corner lost its other edge"));
                };
                let Some(EdgeBlend::Line(o)) = self.edges.get(other) else {
                    return Err(unsupported("a fillet meets a rim fillet"));
                };
                // Equal-radius cylinders whose axes meet at p meet in the two planes
                // through p with normals t_e ± t_o (|(x-p)·t_e| = |(x-p)·t_o|).
                let mut best: Option<(f64, Vec3)> = None;
                for n in [line.dir - o.dir, line.dir + o.dir] {
                    let Some(n) = n.normalized() else { continue };
                    let off = ((x - p).dot(n)).abs().max(((y - p).dot(n)).abs());
                    if best.is_none_or(|(b, _)| off < b) {
                        best = Some((off, n));
                    }
                }
                let Some((off, n)) = best else {
                    return Err(internal("two fillets at a corner are parallel"));
                };
                if off > 1e3 * LINEAR_TOLERANCE {
                    return Err(unsupported(format!(
                        "the fillets at {} do not meet in a plane",
                        vertex_label(body, v)?
                    )));
                }
                let arc = section(p, n, line.dir, r)?.arc_between(x, y, vp)?;
                // The arc must lie on the other cylinder too.
                let mid = arc.0.point_at(0.5 * (arc.1 + arc.2));
                let w = mid - p;
                let dist = (w - o.dir * w.dot(o.dir)).length();
                if (dist - r).abs() > 1e3 * LINEAR_TOLERANCE {
                    return Err(internal(
                        "two fillets at a corner do not meet along their section",
                    ));
                }
                Ok(arc)
            }
            _ => {
                let p = vb
                    .center
                    .ok_or_else(|| internal("a fillet corner has no ball"))?;
                let n = (x - p).cross(y - p);
                let frame = Frame::from_normal_and_x(p, n, x - p)
                    .ok_or_else(|| internal("a corner arc is degenerate"))?;
                Conic {
                    frame,
                    rx: r,
                    ry: r,
                }
                .arc_between(x, y, vp)
            }
        }
    }
}

/// The section of the cylinder of radius `r` about the axis through `center` along
/// `dir` by the plane through `center` with unit normal `n`: an ellipse with semi-axis
/// `r` along `dir × n` and `r / |dir · n|` across it, in the plane.
fn section(center: Point3, n: Vec3, dir: Vec3, r: f64) -> KernelResult<Conic> {
    let cos = dir.dot(n).abs();
    if cos >= 1.0 - ANGULAR_TOLERANCE {
        let frame = Frame::from_normal(center, n)
            .ok_or_else(|| internal("a fillet section is degenerate"))?;
        return Ok(Conic {
            frame,
            rx: r,
            ry: r,
        });
    }
    let a = unit(dir.cross(n), "a fillet section's minor axis")?;
    let major = n.cross(a);
    let frame = Frame::from_normal_and_x(center, n, major)
        .ok_or_else(|| internal("a fillet section is degenerate"))?;
    Ok(Conic {
        frame,
        rx: r / cos,
        ry: r,
    })
}

/// The fillet surface of a straight edge: the cylinder of radius `r` about the spine, or
/// for a radius changing linearly from `r0` to `r1`, the envelope of the balls whose
/// centres run along the straight spine from `c0` to `c1`: the cone about that spine with
/// `sin α = (r1 - r0) / |c1 - c0|`, at distance `r0 / cos α` from the axis level with `c0`
/// (so that the ball about `c0` touches it). The cone touches each face along the
/// contact line (both are tangent to every ball), whose foot points run from
/// `r0 / tan(β/2)` to `r1 / tan(β/2)` and meet at the apex on the edge's line.
fn fillet_surface(l: &LineBlend) -> KernelResult<Surface> {
    let c0 = l.ball(0);
    let toward_edge = unit(l.points[0] - c0, "a fillet's spine offset")?;
    if !l.variable() {
        let frame = Frame::from_normal_and_x(c0, l.dir, -toward_edge)
            .ok_or_else(|| internal("a fillet's frame is degenerate"))?;
        return Surface::cylinder(frame, l.radius[0]);
    }
    let axis = l.ball(1) - c0;
    let len = axis.length();
    let sin = (l.radius[1] - l.radius[0]) / len;
    if !(sin.is_finite() && sin.abs() < 1.0 - ANGULAR_TOLERANCE.sqrt()) {
        return Err(unsupported(
            "the fillet's radius changes faster than the edge is long",
        ));
    }
    let alpha = sin.asin();
    let frame = Frame::from_normal_and_x(c0, unit(axis, "a fillet's spine")?, -toward_edge)
        .ok_or_else(|| internal("a fillet's frame is degenerate"))?;
    Surface::cone(frame, l.radius[0] / alpha.cos(), alpha)
}

/// The end curve of a variable-radius fillet on its end face: the arc between `x` and
/// `y` of the cone's section by the face's plane (an ellipse, K6).
fn cone_section(
    line: &LineBlend,
    plane: &FacePlane,
    x: Point3,
    y: Point3,
    near: Point3,
) -> KernelResult<(Curve, f64, f64, bool)> {
    let cone = fillet_surface(line)?;
    let frame = Frame::from_normal(plane.origin, plane.normal)
        .ok_or_else(|| internal("an end face has no plane"))?;
    let hit = crate::intersect_surfaces(&Surface::Plane(frame), &cone)?;
    for c in hit.curves() {
        let conic = match &c.curve {
            Curve::Ellipse(e) => Conic {
                frame: *e.frame(),
                rx: e.rx(),
                ry: e.ry(),
            },
            Curve::Circle(c) => Conic {
                frame: *c.frame(),
                rx: c.radius(),
                ry: c.radius(),
            },
            _ => continue,
        };
        if let Ok(arc) = conic.arc_between(x, y, near) {
            return Ok(arc);
        }
    }
    Err(unsupported(
        "the variable-radius fillet does not end in an ellipse on its end face",
    ))
}

/// Radians a point's angle may fall before an edge's start and still count as on it.
const PARAM_SLACK: f64 = 1e-9;

/// The parameter in `[t0, t1]` of the point `p` of an edge's curve.
fn param_on(c: &Curve, t0: f64, t1: f64, p: Point3) -> KernelResult<f64> {
    let angle = |frame: &Frame, rx: f64, ry: f64| {
        let l = frame.to_local(p);
        let a = (l.y / ry).atan2(l.x / rx);
        crate::check::angle_from(a, t0 - PARAM_SLACK).max(t0)
    };
    let t = match c {
        Curve::Line(l) => (p - l.origin()).dot(l.direction()),
        Curve::Segment(s) => {
            let d = s.end() - s.start();
            (p - s.start()).dot(d) / d.dot(d)
        }
        Curve::Circle(k) => angle(k.frame(), k.radius(), k.radius()),
        Curve::Arc(a) => angle(a.circle().frame(), a.circle().radius(), a.circle().radius()),
        Curve::Ellipse(e) => angle(e.frame(), e.rx(), e.ry()),
        Curve::EllipticalArc(a) => angle(a.ellipse().frame(), a.ellipse().rx(), a.ellipse().ry()),
        Curve::BSpline(b) => b.closest_point(p).u,
    };
    let t = t.clamp(t0, t1);
    if !c.point_at(t).coincident(p) {
        return Err(internal("a blend's new vertex is off the edge it cuts"));
    }
    Ok(t)
}

fn join(a: VKey, b: VKey) -> EKey {
    if a <= b {
        EKey::Join(a, b)
    } else {
        EKey::Join(b, a)
    }
}

/// The plane of a planar loop of segments, oriented by the loop (Newell's method).
fn plane_of(spec: &Spec, lp: &[SpecCoedge]) -> KernelResult<Surface> {
    let mut pts = Vec::with_capacity(lp.len());
    for c in lp {
        let e = spec
            .list_edges
            .get(c.edge)
            .ok_or_else(|| internal("a chamfer face lost an edge"))?;
        let v = if c.forward { e.from } else { e.to };
        let p = spec
            .list
            .get(v)
            .ok_or_else(|| internal("a chamfer face lost a vertex"))?;
        pts.push(p.point);
    }
    let Some(&origin) = pts.first() else {
        return Err(internal("a chamfer face has no vertices"));
    };
    let mut n = Vec3::ZERO;
    for (i, p) in pts.iter().enumerate() {
        let q = pts[(i + 1) % pts.len()];
        n = n + (*p - origin).cross(q - origin);
    }
    let frame =
        Frame::from_normal(origin, n).ok_or_else(|| internal("a chamfer face has no area"))?;
    Ok(Surface::Plane(frame))
}

/// A vertex of the face list being written.
struct SpecVertex {
    point: Point3,
    name: Option<String>,
}

/// An edge of the face list, from vertex `from` (at `curve(t0)`) to `to`.
struct SpecEdge {
    from: usize,
    to: usize,
    curve: Curve,
    t0: f64,
    t1: f64,
    name: Option<String>,
}

/// One use of an edge by a loop, along it or against it.
#[derive(Debug, Clone, Copy)]
struct SpecCoedge {
    edge: usize,
    forward: bool,
}

/// A face of the face list; `loops[0]` is the outer loop.
struct SpecFace {
    surface: Surface,
    same_sense: bool,
    name: Option<String>,
    loops: Vec<Vec<SpecCoedge>>,
}

/// The face list being written, with lookups by key.
#[derive(Default)]
struct Spec {
    vertices: HashMap<VKey, usize>,
    list: Vec<SpecVertex>,
    edges: BTreeMap<EKey, usize>,
    list_edges: Vec<SpecEdge>,
    faces: Vec<SpecFace>,
}

impl Spec {
    fn vertex(&mut self, k: VKey, point: Point3, name: Option<String>) {
        if self.vertices.contains_key(&k) {
            return;
        }
        self.vertices.insert(k, self.list.len());
        self.list.push(SpecVertex { point, name });
    }
    fn vid(&self, k: VKey) -> KernelResult<usize> {
        self.vertices
            .get(&k)
            .copied()
            .ok_or_else(|| internal("a blend refers to a vertex it did not make"))
    }
    #[allow(clippy::too_many_arguments)]
    fn edge(
        &mut self,
        key: EKey,
        from: VKey,
        to: VKey,
        curve: Curve,
        t0: f64,
        t1: f64,
        name: Option<String>,
    ) -> KernelResult<()> {
        let (from, to) = (self.vid(from)?, self.vid(to)?);
        self.edges.insert(key, self.list_edges.len());
        self.list_edges.push(SpecEdge {
            from,
            to,
            curve,
            t0,
            t1,
            name,
        });
        Ok(())
    }
    fn eid(&self, key: EKey) -> KernelResult<usize> {
        self.edges
            .get(&key)
            .copied()
            .ok_or_else(|| internal("a blend refers to an edge it did not make"))
    }
    fn coedge_dir(&self, key: EKey, forward: bool) -> KernelResult<SpecCoedge> {
        Ok(SpecCoedge {
            edge: self.eid(key)?,
            forward,
        })
    }
    /// The use of edge `key` leaving vertex `from`.
    fn coedge_from(&self, key: EKey, from: VKey) -> KernelResult<SpecCoedge> {
        let edge = self.eid(key)?;
        let e = self
            .list_edges
            .get(edge)
            .ok_or_else(|| internal("an edge vanished"))?;
        let v = self.vid(from)?;
        if e.from == v {
            Ok(SpecCoedge {
                edge,
                forward: true,
            })
        } else if e.to == v {
            Ok(SpecCoedge {
                edge,
                forward: false,
            })
        } else {
            Err(internal("a blend edge does not start where its loop is"))
        }
    }
    /// Links the face list into a body (shells are its connected components).
    fn build(self) -> KernelResult<Body> {
        let vertices: Vec<ListVertex> = self
            .list
            .into_iter()
            .map(|v| ListVertex {
                point: v.point,
                name: v.name,
            })
            .collect();
        let edges: Vec<ListEdge> = self
            .list_edges
            .into_iter()
            .map(|e| ListEdge {
                vertices: [e.from, e.to],
                curve: e.curve,
                t0: e.t0,
                t1: e.t1,
                name: e.name,
            })
            .collect();
        let faces: Vec<ListFace> = self
            .faces
            .into_iter()
            .map(|f| ListFace {
                surface: f.surface,
                same_sense: f.same_sense,
                name: f.name,
                loops: f
                    .loops
                    .into_iter()
                    .map(|l| l.into_iter().map(|c| (c.edge, c.forward)).collect())
                    .collect(),
            })
            .collect();
        Body::from_face_list(&vertices, &edges, &faces)
    }
}

/// A boundary segment of a face: loop index, position in the loop, end points.
type Piece = (usize, usize, (f64, f64), (f64, f64));

/// Refuses a planar face whose boundary crosses itself (a blend reaching across it):
/// every pair of non-adjacent boundary segments (conics sampled) must not cross.
fn check_face_simple(body: &Body, f: FaceId, kind: Kind) -> KernelResult<()> {
    let Some((Surface::Plane(frame), _)) = body.face_surface(f)? else {
        return Ok(());
    };
    let mut loops: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut total = 0usize;
    for lp in body.face_loops(f)? {
        let mut pts = Vec::new();
        for h in body.loop_half_edges(lp)? {
            let Some(e) = body.half_edge_edge(h)? else {
                continue;
            };
            let Some((c, t0, t1)) = body.edge_curve(e)? else {
                continue;
            };
            let along = body.edge_half_edges(e)?[0] == h;
            let n = match c {
                Curve::Line(_) | Curve::Segment(_) => 1,
                _ => CONIC_SAMPLES,
            };
            for k in 0..n {
                let s = k as f64 / n as f64;
                let t = if along {
                    t0 + (t1 - t0) * s
                } else {
                    t1 - (t1 - t0) * s
                };
                let p = frame.to_plane(c.point_at(t));
                pts.push((p.x, p.y));
            }
        }
        total += pts.len();
        if total > MAX_CHECK_SEGMENTS {
            return Err(unsupported(format!(
                "{} has too many edges to check after blending",
                face_label(body, f)?
            )));
        }
        loops.push(pts);
    }
    let cross = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let segs: Vec<Piece> = loops
        .iter()
        .enumerate()
        .flat_map(|(li, l)| {
            let n = l.len();
            (0..n).map(move |i| (li, i, l[i], l[(i + 1) % n]))
        })
        .collect();
    for (i, &(la, ia, a0, a1)) in segs.iter().enumerate() {
        for &(lb, ib, b0, b1) in segs.iter().skip(i + 1) {
            if la == lb {
                let n = loops.get(la).map_or(0, Vec::len);
                if ib == ia + 1 || (ia == 0 && ib + 1 == n) {
                    continue;
                }
            }
            let scale = [a0, a1, b0, b1]
                .iter()
                .map(|p| p.0.abs().max(p.1.abs()))
                .fold(1.0, f64::max);
            let eps = 1e-12 * scale * scale;
            let (d1, d2) = (cross(a0, a1, b0), cross(a0, a1, b1));
            let (d3, d4) = (cross(b0, b1, a0), cross(b0, b1, a1));
            if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
                && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
            {
                return Err(too_large(
                    kind,
                    format!("{} would cross itself", face_label(body, f)?),
                ));
            }
        }
    }
    Ok(())
}

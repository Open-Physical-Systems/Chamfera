//! Validity checks for bodies.
//!
//! [`Body::validate_topology`] checks the combinatorial structure; every `Body` value
//! passes it (Euler operators keep it, loading re-checks it). [`Body::validate`] adds
//! the geometric checks a finished solid must pass. Neither panics on any input.
//!
//! The Euler-Poincare form used, per shell (Mantyla 1988, section 9.2):
//!
//! `V - E + F - (L - F) = 2 (1 - G)`
//!
//! with V, E, F, L the shell's vertices, edges, faces and loops (so `L - F` counts the
//! inner loops, "rings"), and G its genus (number of through holes). Summed over a body's
//! S shells this is `V - E + F - (L - F) = 2 (S - G)`. The check requires the left side
//! to be even and at most 2 per shell, and reports `G = (2 - (V - E + 2F - L)) / 2`.
//! For a structure whose loops are closed cycles with mates in opposite directions this
//! holds automatically (it is necessary, not sufficient, for a valid solid), so it mainly
//! catches counts that disagree with the links, e.g. a face that is not connected to the
//! rest of its shell.

use super::{
    Body, EdgeId, FaceId, HalfEdgeId, LoopId, MAX_NAME_LENGTH, MAX_TOPOLOGY_ENTITIES, ShellId,
    Surface, TopologySummary, VertexId,
};
use chamfera_geom::LINEAR_TOLERANCE;
use std::collections::HashSet;
use thiserror::Error;

/// What is wrong with a body. Every message names the entity.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TopologyError {
    #[error("the body holds more than {MAX_TOPOLOGY_ENTITIES} entities of one kind")]
    TooLarge,
    #[error("{from} refers to {to}, which does not exist")]
    Dangling { from: String, to: String },
    #[error("{0} is listed more than once or by the wrong owner")]
    BadOwnership(String),
    #[error("{0} belongs to no owner (shell, face or loop)")]
    Orphan(String),
    #[error("{0} has no faces")]
    EmptyShell(ShellId),
    #[error("the half-edges of {0} do not form one closed cycle")]
    OpenLoop(LoopId),
    #[error("the next and prev links of {0} disagree")]
    BadLinks(HalfEdgeId),
    #[error("{0} has no edge but is not alone in its loop")]
    StrayLoneHalfEdge(HalfEdgeId),
    #[error("{0} does not have two distinct half-edges that both refer to it")]
    MissingTwin(EdgeId),
    #[error(
        "the half-edges of {0} do not run head to tail in opposite directions (each must end where its mate starts)"
    )]
    TwinsNotOpposite(EdgeId),
    #[error("the half-edges around {0} do not form a single fan (non-manifold vertex)")]
    VertexFan(VertexId),
    #[error("{0} is used by more than one shell")]
    SharedBetweenShells(String),
    #[error("the faces of {0} are not connected")]
    ShellNotConnected(ShellId),
    #[error(
        "Euler-Poincare fails for {shell}: V - E + F - (L - F) = {chi}, which must be even and at most 2"
    )]
    EulerPoincare { shell: ShellId, chi: i64 },
    #[error("a name of {0} is longer than {MAX_NAME_LENGTH} bytes")]
    NameTooLong(String),
    #[error("{0} has no finite point inside the model's working range")]
    InvalidPoint(VertexId),
    #[error("{0} has no curve")]
    MissingCurve(EdgeId),
    #[error("{0} has an invalid curve range: {1}")]
    BadCurveRange(EdgeId, String),
    #[error("the curve of {0} does not start and end at its vertices within {LINEAR_TOLERANCE} mm")]
    VertexOffCurve(EdgeId),
    #[error("{0} joins two different vertices at the same point")]
    DegenerateEdge(EdgeId),
    #[error("{0} has no surface")]
    MissingSurface(FaceId),
    #[error("a vertex or edge of {0} is farther than {LINEAR_TOLERANCE} mm from its surface")]
    OffSurface(FaceId),
    #[error("{0} is used twice by one loop of a planar face (a dangling wire edge)")]
    WireEdge(EdgeId),
    #[error("{0} has a loop without edges")]
    VertexLoop(FaceId),
    #[error(
        "the loops of {0} disagree with its orientation (outer loop counter-clockwise, inner loops clockwise about the outward normal)"
    )]
    WrongOrientation(FaceId),
    #[error("{0} encloses no area")]
    DegenerateFace(FaceId),
    #[error(
        "{0} encloses no volume (its faces do not close, or they point inward with no lump around it)"
    )]
    NotOutward(ShellId),
    #[error("{0} is a void (its faces point inward) that does not lie inside a lump of the body")]
    VoidOutside(ShellId),
    #[error(
        "{0} is open (it has boundary edges): this is a surface body, not a closed solid; stitch or patch it closed, or thicken it"
    )]
    OpenShell(ShellId),
    #[error("{0} has an opening on both sides (an edge of no face)")]
    BareEdge(EdgeId),
    #[error("{0} is only openings: a shell needs at least one face")]
    OnlyOpenings(ShellId),
    #[error("{0} is an opening of an open shell but carries a surface or rings")]
    BadOpening(FaceId),
}

/// What [`Body::validate_as`] checks the body as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Validation {
    /// A closed solid: every shell closed, enclosing volume (as [`Body::validate`]).
    #[default]
    Solid,
    /// A surface body: shells may be open (boundary edges with one face); every face is
    /// checked as for a solid, but no shell needs to enclose volume.
    Surface,
}

impl From<TopologyError> for crate::KernelError {
    fn from(e: TopologyError) -> Self {
        crate::KernelError::InvalidResult(e.to_string())
    }
}

/// Counts and genus of one shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellReport {
    pub shell: ShellId,
    pub vertices: usize,
    pub edges: usize,
    pub faces: usize,
    pub loops: usize,
    /// Through holes: `(2 - (V - E + F - (L - F))) / 2`.
    pub genus: usize,
}

/// The result of a successful validity check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyReport {
    pub summary: TopologySummary,
    pub shells: Vec<ShellReport>,
    /// Total genus over all shells.
    pub genus: usize,
}

impl TopologyReport {
    /// `V - E + F - (L - F)` of the whole body, which equals `2 (S - G)`.
    pub fn euler_characteristic(&self) -> i64 {
        let s = &self.summary;
        let i = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        i(s.vertices) - i(s.edges) + 2 * i(s.faces) - i(s.loops)
    }
}

fn dangling(from: impl std::fmt::Display, to: impl std::fmt::Display) -> TopologyError {
    TopologyError::Dangling {
        from: from.to_string(),
        to: to.to_string(),
    }
}

impl Body {
    /// Checks the combinatorial structure: every link points at a live entity and agrees
    /// with its back link; every loop is a closed cycle of half-edges chaining head to
    /// tail; every edge has exactly two half-edges that run in opposite directions (so
    /// adjacent faces are consistently oriented); every vertex has one fan of half-edges
    /// (manifold); every shell is connected and satisfies Euler-Poincare (module docs).
    pub fn validate_topology(&self) -> Result<TopologyReport, TopologyError> {
        let slots = [
            self.shells.slots(),
            self.faces.slots(),
            self.loops.slots(),
            self.half_edges.slots(),
            self.edges.slots(),
            self.vertices.slots(),
            self.points.slots(),
            self.curves.slots(),
            self.surfaces.slots(),
        ];
        if slots.iter().any(|n| *n > MAX_TOPOLOGY_ENTITIES) {
            return Err(TopologyError::TooLarge);
        }

        // Shells own faces.
        let mut face_seen = vec![false; self.faces.slots()];
        for (si, shell) in self.shells.live() {
            let s = ShellId::from_index(si);
            if shell.faces.is_empty() {
                return Err(TopologyError::EmptyShell(s));
            }
            for f in &shell.faces {
                let face = self.faces.get(f.index()).ok_or_else(|| dangling(s, f))?;
                if face.shell != s || face_seen.get(f.index()).copied().unwrap_or(true) {
                    return Err(TopologyError::BadOwnership(f.to_string()));
                }
                face_seen[f.index()] = true;
            }
        }
        for (fi, face) in self.faces.live() {
            let f = FaceId::from_index(fi);
            if !face_seen.get(fi).copied().unwrap_or(false) {
                return Err(TopologyError::Orphan(f.to_string()));
            }
            if let Some(s) = face.surface
                && self.surfaces.get(s.index()).is_none()
            {
                return Err(dangling(f, s));
            }
            if face
                .name
                .as_ref()
                .is_some_and(|n| n.len() > MAX_NAME_LENGTH)
            {
                return Err(TopologyError::NameTooLong(f.to_string()));
            }
            if face.boundary && (face.surface.is_some() || !face.inner.is_empty()) {
                return Err(TopologyError::BadOpening(f));
            }
        }
        for (si, shell) in self.shells.live() {
            let all_open = shell
                .faces
                .iter()
                .all(|f| self.faces.get(f.index()).is_some_and(|face| face.boundary));
            if all_open {
                return Err(TopologyError::OnlyOpenings(ShellId::from_index(si)));
            }
        }

        // Faces own loops.
        let mut loop_seen = vec![false; self.loops.slots()];
        for (fi, face) in self.faces.live() {
            let f = FaceId::from_index(fi);
            for l in std::iter::once(&face.outer).chain(face.inner.iter()) {
                let lp = self.loops.get(l.index()).ok_or_else(|| dangling(f, l))?;
                if lp.face != f || loop_seen.get(l.index()).copied().unwrap_or(true) {
                    return Err(TopologyError::BadOwnership(l.to_string()));
                }
                loop_seen[l.index()] = true;
            }
        }
        if let Some((li, _)) = self
            .loops
            .live()
            .find(|(i, _)| !loop_seen.get(*i).copied().unwrap_or(false))
        {
            return Err(TopologyError::Orphan(LoopId::from_index(li).to_string()));
        }

        // Loops own half-edges: closed cycles, consistent links.
        let mut he_seen = vec![false; self.half_edges.slots()];
        for (li, lp) in self.loops.live() {
            let l = LoopId::from_index(li);
            let mut h = lp.first;
            let mut len = 0usize;
            loop {
                let he = self
                    .half_edges
                    .get(h.index())
                    .ok_or_else(|| dangling(l, h))?;
                if he.lp != l || he_seen.get(h.index()).copied().unwrap_or(true) {
                    return Err(TopologyError::OpenLoop(l));
                }
                he_seen[h.index()] = true;
                len += 1;
                let next = self
                    .half_edges
                    .get(he.next.index())
                    .ok_or_else(|| dangling(h, he.next))?;
                if next.prev != h {
                    return Err(TopologyError::BadLinks(h));
                }
                if self.vertices.get(he.origin.index()).is_none() {
                    return Err(dangling(h, he.origin));
                }
                h = he.next;
                if h == lp.first {
                    break;
                }
            }
            if len > 1 {
                for h in self
                    .walk(lp.first)
                    .map_err(|_| TopologyError::OpenLoop(l))?
                {
                    if self
                        .half_edges
                        .get(h.index())
                        .is_some_and(|he| he.edge.is_none())
                    {
                        return Err(TopologyError::StrayLoneHalfEdge(h));
                    }
                }
            }
        }
        if let Some((hi, _)) = self
            .half_edges
            .live()
            .find(|(i, _)| !he_seen.get(*i).copied().unwrap_or(false))
        {
            return Err(TopologyError::Orphan(
                HalfEdgeId::from_index(hi).to_string(),
            ));
        }

        // Edges and their two half-edges.
        for (hi, he) in self.half_edges.live() {
            let h = HalfEdgeId::from_index(hi);
            if let Some(e) = he.edge {
                let edge = self.edges.get(e.index()).ok_or_else(|| dangling(h, e))?;
                if !edge.halves.contains(&h) {
                    return Err(TopologyError::MissingTwin(e));
                }
            }
        }
        for (ei, edge) in self.edges.live() {
            let e = EdgeId::from_index(ei);
            let [a, b] = edge.halves;
            let (Some(ha), Some(hb)) = (
                self.half_edges.get(a.index()),
                self.half_edges.get(b.index()),
            ) else {
                return Err(TopologyError::MissingTwin(e));
            };
            if a == b || ha.edge != Some(e) || hb.edge != Some(e) {
                return Err(TopologyError::MissingTwin(e));
            }
            let origin = |h: HalfEdgeId| self.half_edges.get(h.index()).map(|x| x.origin);
            // Head to tail: each half-edge ends (where its next starts) at its mate's origin.
            if origin(ha.next) != Some(hb.origin) || origin(hb.next) != Some(ha.origin) {
                return Err(TopologyError::TwinsNotOpposite(e));
            }
            if let Some(c) = edge.curve
                && self.curves.get(c.curve.index()).is_none()
            {
                return Err(dangling(e, c.curve));
            }
            let opening = |h: &super::HalfEdge| {
                self.loops
                    .get(h.lp.index())
                    .and_then(|l| self.faces.get(l.face.index()))
                    .is_some_and(|f| f.boundary)
            };
            if opening(ha) && opening(hb) {
                return Err(TopologyError::BareEdge(e));
            }
            if edge
                .name
                .as_ref()
                .is_some_and(|n| n.len() > MAX_NAME_LENGTH)
            {
                return Err(TopologyError::NameTooLong(e.to_string()));
            }
        }

        // Vertices: one fan each, covering every half-edge that starts there.
        let mut out_degree = vec![0usize; self.vertices.slots()];
        for (_, he) in self.half_edges.live() {
            if let Some(d) = out_degree.get_mut(he.origin.index()) {
                *d += 1;
            }
        }
        for (vi, vx) in self.vertices.live() {
            let v = VertexId::from_index(vi);
            let he = self
                .half_edges
                .get(vx.half_edge.index())
                .ok_or_else(|| dangling(v, vx.half_edge))?;
            if he.origin != v {
                return Err(TopologyError::VertexFan(v));
            }
            if self.points.get(vx.point.index()).is_none() {
                return Err(dangling(v, vx.point));
            }
            if vx.name.as_ref().is_some_and(|n| n.len() > MAX_NAME_LENGTH) {
                return Err(TopologyError::NameTooLong(v.to_string()));
            }
            let fan = self
                .vertex_fan(v)
                .map_err(|_| TopologyError::VertexFan(v))?;
            let distinct: HashSet<HalfEdgeId> = fan.iter().copied().collect();
            let all_from_v = fan.iter().all(|h| {
                self.half_edges
                    .get(h.index())
                    .is_some_and(|x| x.origin == v)
            });
            if !all_from_v
                || distinct.len() != fan.len()
                || out_degree.get(vi).copied() != Some(fan.len())
            {
                return Err(TopologyError::VertexFan(v));
            }
        }

        // Shells: membership, connectivity, Euler-Poincare.
        let face_of = |h: HalfEdgeId| -> Option<FaceId> {
            let he = self.half_edges.get(h.index())?;
            Some(self.loops.get(he.lp.index())?.face)
        };
        let shell_of = |f: FaceId| self.faces.get(f.index()).map(|x| x.shell);
        for (ei, edge) in self.edges.live() {
            let [a, b] = edge.halves;
            if face_of(a).and_then(shell_of) != face_of(b).and_then(shell_of) {
                return Err(TopologyError::SharedBetweenShells(
                    EdgeId::from_index(ei).to_string(),
                ));
            }
        }
        let mut shells = Vec::new();
        for (si, shell) in self.shells.live() {
            let s = ShellId::from_index(si);
            let mut verts = HashSet::new();
            let mut edges = HashSet::new();
            let mut loops = 0usize;
            for f in &shell.faces {
                for l in self.face_loops(*f).map_err(|_| dangling(s, f))? {
                    loops += 1;
                    for h in self
                        .loop_half_edges(l)
                        .map_err(|_| TopologyError::OpenLoop(l))?
                    {
                        let Some(he) = self.half_edges.get(h.index()) else {
                            return Err(dangling(l, h));
                        };
                        verts.insert(he.origin);
                        if let Some(e) = he.edge {
                            edges.insert(e);
                        }
                    }
                }
            }
            for v in &verts {
                let fan = self
                    .vertex_fan(*v)
                    .map_err(|_| TopologyError::VertexFan(*v))?;
                if fan
                    .iter()
                    .any(|h| face_of(*h).and_then(shell_of) != Some(s))
                {
                    return Err(TopologyError::SharedBetweenShells(v.to_string()));
                }
            }
            self.check_connected(s, &shell.faces)?;
            let n = |x: usize| i64::try_from(x).unwrap_or(i64::MAX / 4);
            let chi = n(verts.len()) - n(edges.len()) + 2 * n(shell.faces.len()) - n(loops);
            if chi > 2 || chi.rem_euclid(2) != 0 {
                return Err(TopologyError::EulerPoincare { shell: s, chi });
            }
            shells.push(ShellReport {
                shell: s,
                vertices: verts.len(),
                edges: edges.len(),
                faces: shell.faces.len(),
                loops,
                genus: usize::try_from((2 - chi) / 2).unwrap_or(0),
            });
        }
        let genus = shells.iter().map(|s| s.genus).sum();
        Ok(TopologyReport {
            summary: self.summary(),
            shells,
            genus,
        })
    }

    /// Faces of a shell reach each other across edges.
    fn check_connected(&self, s: ShellId, faces: &[FaceId]) -> Result<(), TopologyError> {
        let Some(start) = faces.first() else {
            return Err(TopologyError::EmptyShell(s));
        };
        let seen: HashSet<FaceId> = self.faces_reachable(*start).into_iter().collect();
        if seen.len() == faces.len() && faces.iter().all(|f| seen.contains(f)) {
            Ok(())
        } else {
            Err(TopologyError::ShellNotConnected(s))
        }
    }

    /// What loading a body checks: the topology, plus that every point is valid and every
    /// edge range fits its curve (curves and frames are re-validated as they are read).
    pub(super) fn validate_loaded(&self) -> Result<TopologyReport, TopologyError> {
        let report = self.validate_topology()?;
        for (vi, _) in self.vertices.live() {
            let v = VertexId::from_index(vi);
            if !self.vertex_point(v).is_ok_and(|p| p.is_valid()) {
                return Err(TopologyError::InvalidPoint(v));
            }
        }
        for (ei, _) in self.edges.live() {
            let e = EdgeId::from_index(ei);
            if let Ok(Some((c, t0, t1))) = self.edge_curve(e) {
                c.check_range(t0, t1)
                    .map_err(|err| TopologyError::BadCurveRange(e, err.to_string()))?;
            }
        }
        Ok(report)
    }

    /// Checks that the body is a valid solid: [`Body::validate_topology`], then geometry.
    /// Every vertex has a valid point; every edge has a curve whose ends are its
    /// vertices' points within [`LINEAR_TOLERANCE`]; every face has a surface and no loop
    /// that is a bare vertex; on a planar face every vertex and edge lies in the plane,
    /// no edge is used twice by one loop, and the outer loop runs counter-clockwise and
    /// inner loops clockwise about the outward normal; on a curved face every vertex and
    /// edge lies on the surface, and each loop whose `(u, v)` image closes runs
    /// counter-clockwise (outer) or clockwise (inner) in `(u, v)` when the outward normal is
    /// `S_u × S_v`, the other way when it is the opposite (loops around a periodic
    /// direction without a seam are not checked for orientation); every shell encloses
    /// non-zero volume, positive for a lump and negative for a void inside a lump (which also
    /// catches a face whose orientation flag is wrong; see `check_shell_volumes`).
    pub fn validate(&self) -> Result<TopologyReport, TopologyError> {
        self.validate_as(Validation::Solid)
    }

    /// [`Body::validate`] for a solid; for a surface body ([`Validation::Surface`]) the
    /// same checks except that shells may be open and need not enclose volume (the
    /// orientation of neighbouring faces is still consistent: every edge's two uses run in
    /// opposite directions, an opening's included). [`Validation::Solid`] fails with
    /// [`TopologyError::OpenShell`] on an open shell.
    pub fn validate_as(&self, mode: Validation) -> Result<TopologyReport, TopologyError> {
        let report = self.validate_loaded()?;
        if mode == Validation::Solid {
            for (si, _) in self.shells.live() {
                let s = ShellId::from_index(si);
                if self.shell_is_open(s).unwrap_or(true) {
                    return Err(TopologyError::OpenShell(s));
                }
            }
        }
        for (ei, edge) in self.edges.live() {
            let e = EdgeId::from_index(ei);
            let Some((c, t0, t1)) = self.edge_curve(e).ok().flatten() else {
                return Err(TopologyError::MissingCurve(e));
            };
            let [v0, v1] = self
                .edge_vertices(e)
                .map_err(|_| TopologyError::MissingTwin(e))?;
            let (Ok(p0), Ok(p1)) = (self.vertex_point(v0), self.vertex_point(v1)) else {
                return Err(TopologyError::InvalidPoint(v0));
            };
            if !(c.point_at(t0).coincident(p0) && c.point_at(t1).coincident(p1)) {
                return Err(TopologyError::VertexOffCurve(e));
            }
            if v0 != v1 && p0.coincident(p1) {
                return Err(TopologyError::DegenerateEdge(e));
            }
            let [a, b] = edge.halves;
            let same_loop = self.half_edges.get(a.index()).map(|h| h.lp)
                == self.half_edges.get(b.index()).map(|h| h.lp);
            if same_loop
                && let Ok([f, _]) = self.edge_faces(e)
                && matches!(self.face_surface(f), Ok(Some((Surface::Plane(_), _))))
            {
                return Err(TopologyError::WireEdge(e));
            }
        }
        for f in self.faces() {
            let Some((surface, same_sense)) = self.face_surface(f).ok().flatten() else {
                return Err(TopologyError::MissingSurface(f));
            };
            match surface {
                Surface::Plane(_) => self.check_planar_face(f, &surface, same_sense)?,
                _ => self.check_curved_face(f, &surface, same_sense)?,
            }
        }
        if mode == Validation::Solid {
            self.check_shell_volumes()?;
        }
        Ok(report)
    }

    /// Every shell encloses a non-zero volume. A shell enclosing positive volume is the
    /// outer boundary of a lump; one enclosing negative volume (its faces point into the
    /// region it bounds) is a void, which booleans make (K7), and must lie inside a lump:
    /// its bounding box inside the bounding box of a positive shell, and the body's total
    /// volume positive.
    fn check_shell_volumes(&self) -> Result<(), TopologyError> {
        let mut lumps = Vec::new();
        let mut voids = Vec::new();
        for (si, _) in self.shells.live() {
            let s = ShellId::from_index(si);
            let v = self
                .shell_volume(s)
                .map_err(|_| TopologyError::NotOutward(s))?;
            let b = self.shell_box(s).ok_or(TopologyError::NotOutward(s))?;
            let tiny = LINEAR_TOLERANCE * LINEAR_TOLERANCE * LINEAR_TOLERANCE;
            if !v.is_finite() || v.abs() <= tiny {
                return Err(TopologyError::NotOutward(s));
            }
            if v > 0.0 {
                lumps.push((v, b));
            } else {
                voids.push((s, v, b));
            }
        }
        let total: f64 =
            lumps.iter().map(|l| l.0).sum::<f64>() + voids.iter().map(|v| v.1).sum::<f64>();
        for (s, _, b) in &voids {
            let inside = lumps.iter().any(|(_, l)| {
                l.min.x - LINEAR_TOLERANCE <= b.min.x
                    && l.min.y - LINEAR_TOLERANCE <= b.min.y
                    && l.min.z - LINEAR_TOLERANCE <= b.min.z
                    && b.max.x <= l.max.x + LINEAR_TOLERANCE
                    && b.max.y <= l.max.y + LINEAR_TOLERANCE
                    && b.max.z <= l.max.z + LINEAR_TOLERANCE
            });
            if !inside || total <= 0.0 {
                return Err(TopologyError::VoidOutside(*s));
            }
        }
        Ok(())
    }

    /// The box around the edges of a shell's faces.
    fn shell_box(&self, s: ShellId) -> Option<chamfera_geom::Aabb> {
        let mut out: Option<chamfera_geom::Aabb> = None;
        for f in self.shell_faces(s).ok()? {
            for e in self.face_edges(f).ok()? {
                let (c, t0, t1) = self.edge_curve(e).ok()??;
                let b = c.bounding_box_on(t0, t1).ok()?;
                out = Some(match out {
                    Some(o) => o.union(b),
                    None => b,
                });
            }
        }
        out
    }

    fn check_planar_face(
        &self,
        f: FaceId,
        surface: &Surface,
        same_sense: bool,
    ) -> Result<(), TopologyError> {
        let normal = surface
            .plane_normal()
            .ok_or(TopologyError::MissingSurface(f))?;
        let normal = if same_sense { normal } else { -normal };
        let loops = self.face_loops(f).map_err(|_| dangling(f, "its loops"))?;
        for (k, l) in loops.iter().enumerate() {
            let hs = self
                .loop_half_edges(*l)
                .map_err(|_| TopologyError::OpenLoop(*l))?;
            for h in &hs {
                let Some(he) = self.half_edges.get(h.index()) else {
                    return Err(dangling(l, h));
                };
                let Some(e) = he.edge else {
                    return Err(TopologyError::VertexLoop(f));
                };
                let Some((c, t0, t1)) = self.edge_curve(e).ok().flatten() else {
                    return Err(TopologyError::MissingCurve(e));
                };
                // Five samples catch a curve that leaves the plane; the K1 curves are
                // planar, so a curve in the plane at five points lies in it.
                for i in 0..=4 {
                    let t = t0 + (t1 - t0) * f64::from(i) / 4.0;
                    let off = surface
                        .plane_distance(c.point_at(t))
                        .unwrap_or(f64::INFINITY);
                    if off.is_nan() || off.abs() > LINEAR_TOLERANCE {
                        return Err(TopologyError::OffSurface(f));
                    }
                }
            }
            let area = self
                .loop_vector_area(*l)
                .map_err(|_| TopologyError::OffSurface(f))?
                .dot(normal);
            let tiny = LINEAR_TOLERANCE * LINEAR_TOLERANCE;
            if !area.is_finite() || area.abs() <= tiny {
                return Err(TopologyError::DegenerateFace(f));
            }
            let outer = k == 0;
            if outer != (area > 0.0) {
                return Err(TopologyError::WrongOrientation(f));
            }
        }
        Ok(())
    }

    fn check_curved_face(
        &self,
        f: FaceId,
        surface: &Surface,
        same_sense: bool,
    ) -> Result<(), TopologyError> {
        let orient = if same_sense { 1.0 } else { -1.0 };
        let loops = self.face_loops(f).map_err(|_| dangling(f, "its loops"))?;
        for (k, l) in loops.iter().enumerate() {
            let hs = self
                .loop_half_edges(*l)
                .map_err(|_| TopologyError::OpenLoop(*l))?;
            for h in &hs {
                let Some(he) = self.half_edges.get(h.index()) else {
                    return Err(dangling(l, h));
                };
                let Some(e) = he.edge else {
                    return Err(TopologyError::VertexLoop(f));
                };
                let Some((c, t0, t1)) = self.edge_curve(e).ok().flatten() else {
                    return Err(TopologyError::MissingCurve(e));
                };
                // Nine samples catch a curve that leaves the surface.
                for i in 0..=8 {
                    let t = t0 + (t1 - t0) * f64::from(i) / 8.0;
                    let off = surface.distance(c.point_at(t));
                    if off.is_nan() || off > LINEAR_TOLERANCE {
                        return Err(TopologyError::OffSurface(f));
                    }
                }
            }
            let image = self
                .loop_image(*l, surface, orient)
                .map_err(|_| TopologyError::OffSurface(f))?;
            if !image.closed {
                continue;
            }
            let area = image.signed_area() * orient;
            if !area.is_finite() || area.abs() <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
                return Err(TopologyError::DegenerateFace(f));
            }
            if (k == 0) != (area > 0.0) {
                return Err(TopologyError::WrongOrientation(f));
            }
        }
        Ok(())
    }
}

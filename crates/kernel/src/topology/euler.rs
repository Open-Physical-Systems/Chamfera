//! Euler operators: the only way topology is built and changed.
//!
//! After Mantyla, *An Introduction to Solid Modeling* (1988), chapter 9 (the operators
//! and their effect on the Euler-Poincare formula) and chapter 11 (their implementation on
//! the half-edge structure), and Stroud, *Boundary Representation Modelling Techniques*
//! (2006), chapter 3. The implementation here is our own.
//!
//! Each operator keeps `V - E + F - (L - F) = 2(S - G)` (V vertices, E edges, F faces,
//! L loops, S shells, G genus) and every structural invariant of [`Body`]. Each first
//! checks its preconditions and returns an error without changing anything if they fail.
//!
//! | Operator | Change (V, E, F, L, S, G) | Inverse |
//! |---|---|---|
//! | [`Body::mvfs`] make vertex, face, shell | (+1, 0, +1, +1, +1, 0) | [`Body::kvfs`] |
//! | [`Body::mev`] make edge and vertex | (+1, +1, 0, 0, 0, 0) | [`Body::kev`] |
//! | [`Body::mef`] make edge and face | (0, +1, +1, +1, 0, 0) | [`Body::kef`] |
//! | [`Body::kemr`] kill edge, make ring | (0, -1, 0, +1, 0, 0) | [`Body::mekr`] |
//! | [`Body::kfmrh`] kill face, make ring and hole | (0, 0, -1, 0, 0, +1) or (0, 0, -1, 0, -1, 0) | [`Body::mfkrh`] (the reverse change) |
//!
//! Loops are rebuilt from explicit half-edge sequences: for a loop `... p → h → ...`,
//! `walk(h)` lists it starting at `h`, and `link` writes a sequence back as a cycle.

use super::{
    Body, Edge, EdgeId, Face, FaceId, HalfEdge, HalfEdgeId, Loop, LoopId, PointId, Shell, ShellId,
    Vertex, VertexId,
};
use crate::{KernelError, KernelResult};
use chamfera_geom::{MAX_COORDINATE, Point3};

/// What [`Body::mvfs`] made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mvfs {
    pub shell: ShellId,
    pub face: FaceId,
    pub vertex: VertexId,
    /// The lone half-edge of the face's one-vertex loop.
    pub half_edge: HalfEdgeId,
}

fn check_point(p: Point3) -> KernelResult<Point3> {
    if p.is_valid() {
        Ok(p)
    } else {
        Err(KernelError::InvalidInput(format!(
            "a vertex point must have finite coordinates within ±{MAX_COORDINATE} mm"
        )))
    }
}

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

// Low-level helpers. They are called only after an operator has checked its
// preconditions and room, so every id they touch is live.
impl Body {
    fn new_point(&mut self, p: Point3) -> PointId {
        PointId::from_index(self.points.push(p))
    }

    fn new_vertex(&mut self, p: Point3) -> VertexId {
        let point = self.new_point(p);
        // The half-edge is set by the caller once it exists.
        VertexId::from_index(self.vertices.push(Vertex {
            half_edge: HalfEdgeId(u32::MAX),
            point,
            name: None,
        }))
    }

    fn new_half_edge(&mut self, lp: LoopId, edge: Option<EdgeId>, origin: VertexId) -> HalfEdgeId {
        let id = HalfEdgeId::from_index(self.half_edges.slots());
        self.half_edges.push(HalfEdge {
            lp,
            edge,
            origin,
            next: id,
            prev: id,
        });
        id
    }

    /// A new edge with fresh half-edges `a` (from `from`, in `la`) and `b` (from `to`,
    /// in `lb`), not yet linked into their loops.
    fn new_edge(
        &mut self,
        from: VertexId,
        la: LoopId,
        to: VertexId,
        lb: LoopId,
    ) -> (EdgeId, HalfEdgeId, HalfEdgeId) {
        let e = EdgeId::from_index(self.edges.slots());
        let a = self.new_half_edge(la, Some(e), from);
        let b = self.new_half_edge(lb, Some(e), to);
        self.edges.push(Edge {
            halves: [a, b],
            curve: None,
            name: None,
        });
        (e, a, b)
    }

    fn new_loop(&mut self, face: FaceId) -> LoopId {
        LoopId::from_index(self.loops.push(Loop {
            face,
            first: HalfEdgeId(u32::MAX),
        }))
    }

    /// Writes `seq` (non-empty) into `lp` as a cycle.
    fn link(&mut self, lp: LoopId, seq: &[HalfEdgeId]) {
        let n = seq.len();
        for (i, h) in seq.iter().enumerate() {
            let next = seq[(i + 1) % n];
            let prev = seq[(i + n - 1) % n];
            if let Some(he) = self.half_edges.get_mut(h.index()) {
                he.next = next;
                he.prev = prev;
                he.lp = lp;
            }
        }
        if let (Some(l), Some(first)) = (self.loops.get_mut(lp.index()), seq.first()) {
            l.first = *first;
        }
    }

    /// Writes `seq` into `lp`, or makes `lp` a one-vertex loop at `v` if `seq` is empty.
    /// Returns the lone half-edge when one was made.
    fn link_or_lone(&mut self, lp: LoopId, seq: &[HalfEdgeId], v: VertexId) -> Option<HalfEdgeId> {
        if seq.is_empty() {
            let h = self.new_half_edge(lp, None, v);
            self.link(lp, &[h]);
            self.set_vertex_half_edge(v, h);
            Some(h)
        } else {
            self.link(lp, seq);
            None
        }
    }

    fn set_vertex_half_edge(&mut self, v: VertexId, h: HalfEdgeId) {
        if let Some(vx) = self.vertices.get_mut(v.index()) {
            vx.half_edge = h;
        }
    }

    /// Points `v` at a half-edge of `candidates` starting at it, if its own half-edge is
    /// one of `dead`.
    fn repoint_vertex(&mut self, v: VertexId, dead: &[HalfEdgeId], candidates: &[HalfEdgeId]) {
        let current = match self.vertices.get(v.index()) {
            Some(vx) => vx.half_edge,
            None => return,
        };
        if !dead.contains(&current) {
            return;
        }
        let found = candidates.iter().copied().find(|h| {
            self.half_edges
                .get(h.index())
                .is_some_and(|he| he.origin == v)
        });
        if let Some(h) = found {
            self.set_vertex_half_edge(v, h);
        }
    }

    /// The loop through `h`, starting at `h`, without the lone half-edge of a one-vertex
    /// loop (so a one-vertex loop gives an empty list).
    fn walk_edges(&self, h: HalfEdgeId) -> KernelResult<Vec<HalfEdgeId>> {
        if self.he(h)?.edge.is_none() {
            return Ok(Vec::new());
        }
        self.walk(h)
    }
}

impl Body {
    /// **MVFS**, make vertex, face, shell: starts a new shell holding one face whose only
    /// loop is the single vertex at `point` (a "skeletal" solid). Inverse: [`Body::kvfs`].
    pub fn mvfs(&mut self, point: Point3) -> KernelResult<Mvfs> {
        let point = check_point(point)?;
        self.ensure_room(1)?;
        let shell = ShellId::from_index(self.shells.slots());
        let face = FaceId::from_index(self.faces.slots());
        let lp = self.new_loop(face);
        let vertex = self.new_vertex(point);
        let half_edge = self.new_half_edge(lp, None, vertex);
        self.link(lp, &[half_edge]);
        self.set_vertex_half_edge(vertex, half_edge);
        self.faces.push(Face {
            shell,
            outer: lp,
            inner: Vec::new(),
            surface: None,
            same_sense: true,
            name: None,
        });
        self.shells.push(Shell { faces: vec![face] });
        Ok(Mvfs {
            shell,
            face,
            vertex,
            half_edge,
        })
    }

    /// **KVFS**, kill vertex, face, shell: removes a skeletal shell (one face, one
    /// one-vertex loop). Inverse of [`Body::mvfs`].
    pub fn kvfs(&mut self, shell: ShellId) -> KernelResult<()> {
        let s = self.shell(shell)?;
        let [face] = s.faces.as_slice() else {
            return Err(invalid(format!(
                "KVFS needs {shell} to have exactly one face"
            )));
        };
        let face = *face;
        let f = self.face(face)?;
        if !f.inner.is_empty() {
            return Err(invalid(format!("KVFS needs {face} to have no inner loops")));
        }
        let lp = f.outer;
        let h = self.lp(lp)?.first;
        let he = self.he(h)?;
        if he.edge.is_some() || he.next != h {
            return Err(invalid(format!(
                "KVFS needs {face} to be bounded by a single vertex"
            )));
        }
        let v = he.origin;
        self.half_edges.remove(h.index());
        self.vertices.remove(v.index());
        self.loops.remove(lp.index());
        self.faces.remove(face.index());
        self.shells.remove(shell.index());
        Ok(())
    }

    /// **MEV**, make edge and vertex: adds a new vertex at `point` and an edge to it from
    /// the origin `v` of half-edge `at`, as a strut in `at`'s loop, inserted in the corner
    /// just before `at`. The loop `... p → at ...` becomes `... p → a → b → at ...`, with
    /// `a` running from `v` to the new vertex and `b` back. On a one-vertex loop, the loop
    /// becomes `a → b`. Returns the edge (first half-edge `a`) and the new vertex.
    /// Inverse: [`Body::kev`].
    pub fn mev(&mut self, at: HalfEdgeId, point: Point3) -> KernelResult<(EdgeId, VertexId)> {
        let point = check_point(point)?;
        let he = self.he(at)?;
        let (lp, v, lone) = (he.lp, he.origin, he.edge.is_none());
        let rest = self.walk_edges(at)?;
        self.ensure_room(2)?;
        let nv = self.new_vertex(point);
        let (e, a, b) = self.new_edge(v, lp, nv, lp);
        let mut seq = vec![a, b];
        seq.extend(rest);
        self.link(lp, &seq);
        self.set_vertex_half_edge(nv, b);
        if lone {
            self.half_edges.remove(at.index());
            self.set_vertex_half_edge(v, a);
        }
        Ok((e, nv))
    }

    /// **KEV**, kill edge and vertex: removes a strut edge together with its dangling end
    /// `vertex`, which must have no other edge. Inverse of [`Body::mev`]; a loop left with
    /// no edges becomes a one-vertex loop.
    pub fn kev(&mut self, edge: EdgeId, vertex: VertexId) -> KernelResult<()> {
        let [h0, h1] = self.edge(edge)?.halves;
        let (o0, o1) = (self.he(h0)?.origin, self.he(h1)?.origin);
        // `b` leaves the dying vertex, `a` arrives at it.
        let (a, b) = if o1 == vertex && o0 != vertex {
            (h0, h1)
        } else if o0 == vertex && o1 != vertex {
            (h1, h0)
        } else {
            return Err(invalid(format!(
                "KEV needs {vertex} to be one end of {edge}, which must join two different vertices"
            )));
        };
        if self.he(a)?.next != b {
            return Err(invalid(format!(
                "KEV needs {vertex} to have no edge other than {edge}"
            )));
        }
        let lp = self.he(a)?.lp;
        let v = self.he(a)?.origin;
        let rest: Vec<HalfEdgeId> = self
            .walk(b)?
            .into_iter()
            .filter(|h| *h != a && *h != b)
            .collect();
        self.ensure_room(1)?;
        self.half_edges.remove(a.index());
        self.half_edges.remove(b.index());
        self.edges.remove(edge.index());
        self.vertices.remove(vertex.index());
        if self.link_or_lone(lp, &rest, v).is_none() {
            self.repoint_vertex(v, &[a], &rest);
        }
        Ok(())
    }

    /// **MEF**, make edge and face: joins the origins `v1` of `h1` and `v2` of `h2`, two
    /// half-edges of the same loop, with a new edge that splits the loop in two and makes
    /// a new face. With the loop `h1 ... p2 → h2 ... p1 →`, the old loop keeps
    /// `a → h2 ... p1` (`a` from `v1` to `v2`) and the new face's loop is
    /// `b → h1 ... p2` (`b` from `v2` to `v1`). When `h1 == h2` the new edge is closed (a
    /// loop from `v1` to itself, e.g. a circle): the old loop gets `a` before `h1` and the
    /// new face is bounded by `b` alone. The new face joins the same shell, has no
    /// surface yet, and inner loops stay with the old face. Returns the edge and the new
    /// face. Inverse: [`Body::kef`].
    pub fn mef(&mut self, h1: HalfEdgeId, h2: HalfEdgeId) -> KernelResult<(EdgeId, FaceId)> {
        let he1 = self.he(h1)?;
        let he2 = self.he(h2)?;
        if he1.lp != he2.lp {
            return Err(invalid(format!(
                "MEF needs {h1} and {h2} to be in the same loop"
            )));
        }
        let (lp, v1, v2, lone) = (he1.lp, he1.origin, he2.origin, he1.edge.is_none());
        let face = self.lp(lp)?.face;
        let shell = self.face(face)?.shell;
        self.shell(shell)?;
        let cycle = self.walk_edges(h1)?;
        let (x, y) = if h1 == h2 {
            (Vec::new(), cycle)
        } else {
            let k = cycle
                .iter()
                .position(|h| *h == h2)
                .ok_or_else(|| invalid(format!("MEF: {h2} is not in the loop of {h1}")))?;
            let (x, y) = cycle.split_at(k);
            (x.to_vec(), y.to_vec())
        };
        self.ensure_room(2)?;
        let new_face = FaceId::from_index(self.faces.slots());
        let new_lp = self.new_loop(new_face);
        let (e, a, b) = self.new_edge(v1, lp, v2, new_lp);
        let mut keep = vec![a];
        keep.extend(y);
        let mut split = vec![b];
        split.extend(x);
        self.link(lp, &keep);
        self.link(new_lp, &split);
        if lone {
            self.half_edges.remove(h1.index());
            self.set_vertex_half_edge(v1, a);
        }
        self.faces.push(Face {
            shell,
            outer: new_lp,
            inner: Vec::new(),
            surface: None,
            same_sense: true,
            name: None,
        });
        if let Some(s) = self.shells.get_mut(shell.index()) {
            s.faces.push(new_face);
        }
        Ok((e, new_face))
    }

    /// **KEF**, kill edge and face: removes an edge between two different faces and
    /// merges the face of its second half-edge into the face of its first (the face
    /// [`Body::mef`] made dies, so `kef` undoes `mef`). The two loops are joined: the old
    /// loop without `a` followed by the other loop without `b`. Other loops of the dying
    /// face become inner loops of the surviving one. Inverse of [`Body::mef`].
    pub fn kef(&mut self, edge: EdgeId) -> KernelResult<()> {
        let [a, b] = self.edge(edge)?.halves;
        let (la, lb) = (self.he(a)?.lp, self.he(b)?.lp);
        let (fa, fb) = (self.lp(la)?.face, self.lp(lb)?.face);
        if fa == fb {
            return Err(invalid(format!(
                "KEF needs the two sides of {edge} to be different faces; use KEMR for an edge inside one face"
            )));
        }
        let (v1, v2) = (self.he(a)?.origin, self.he(b)?.origin);
        let dying = self.face(fb)?.clone();
        let shell = dying.shell;
        self.shell(shell)?;
        let mut merged: Vec<HalfEdgeId> = self.walk(a)?.into_iter().skip(1).collect();
        merged.extend(self.walk(b)?.into_iter().skip(1));
        self.ensure_room(1)?;
        let moved: Vec<LoopId> = std::iter::once(dying.outer)
            .chain(dying.inner.iter().copied())
            .filter(|l| *l != lb)
            .collect();
        for l in &moved {
            if let Some(lr) = self.loops.get_mut(l.index()) {
                lr.face = fa;
            }
        }
        if let Some(f) = self.faces.get_mut(fa.index()) {
            f.inner.extend(moved);
        }
        if let Some(s) = self.shells.get_mut(shell.index()) {
            s.faces.retain(|f| *f != fb);
        }
        self.faces.remove(fb.index());
        self.loops.remove(lb.index());
        self.half_edges.remove(a.index());
        self.half_edges.remove(b.index());
        self.edges.remove(edge.index());
        if self.link_or_lone(la, &merged, v1).is_none() {
            self.repoint_vertex(v1, &[a, b], &merged);
            self.repoint_vertex(v2, &[a, b], &merged);
        }
        Ok(())
    }

    /// **KEMR**, kill edge, make ring: removes an edge whose two half-edges are in the
    /// same loop (a bridge between an outer boundary and a hole), splitting the loop in
    /// two. With the loop `h → X → mate(h) → Y`, the loop keeps `Y` and `X` becomes a new
    /// inner loop (ring) of the same face; an empty part becomes a one-vertex loop.
    /// Returns the new ring. Inverse: [`Body::mekr`].
    pub fn kemr(&mut self, h: HalfEdgeId) -> KernelResult<LoopId> {
        let he = self.he(h)?;
        let (lp, v1) = (he.lp, he.origin);
        let edge = he
            .edge
            .ok_or_else(|| invalid(format!("KEMR needs {h} to belong to an edge")))?;
        let b = self
            .mate_of(h)?
            .ok_or_else(|| invalid(format!("KEMR needs {h} to have a mate")))?;
        let hb = self.he(b)?;
        if hb.lp != lp {
            return Err(invalid(format!(
                "KEMR needs both sides of {edge} in the same loop; use KEF between two faces"
            )));
        }
        let v2 = hb.origin;
        if v1 == v2 {
            return Err(invalid(format!(
                "KEMR needs {edge} to join two different vertices"
            )));
        }
        let face = self.lp(lp)?.face;
        self.face(face)?;
        let cycle = self.walk(h)?;
        let k = cycle
            .iter()
            .position(|x| *x == b)
            .ok_or_else(|| invalid(format!("KEMR: the mate of {h} is not in its loop")))?;
        let x: Vec<HalfEdgeId> = cycle.get(1..k).unwrap_or_default().to_vec();
        let y: Vec<HalfEdgeId> = cycle.get(k + 1..).unwrap_or_default().to_vec();
        self.ensure_room(3)?;
        let ring = self.new_loop(face);
        if let Some(f) = self.faces.get_mut(face.index()) {
            f.inner.push(ring);
        }
        self.half_edges.remove(h.index());
        self.half_edges.remove(b.index());
        self.edges.remove(edge.index());
        if self.link_or_lone(lp, &y, v1).is_none() {
            self.repoint_vertex(v1, &[h, b], &y);
        }
        if self.link_or_lone(ring, &x, v2).is_none() {
            self.repoint_vertex(v2, &[h, b], &x);
        }
        Ok(ring)
    }

    /// **MEKR**, make edge, kill ring: joins two different loops of one face with a new
    /// edge from the origin `v1` of `h1` to the origin `v2` of `h2`. The result is one
    /// loop `a → (loop of h2 from h2) → b → (loop of h1 from h1)`, kept in `h1`'s loop;
    /// `h2`'s loop dies (if it was the outer loop, the joined loop becomes the outer one).
    /// Returns the edge. Inverse of [`Body::kemr`].
    pub fn mekr(&mut self, h1: HalfEdgeId, h2: HalfEdgeId) -> KernelResult<EdgeId> {
        let (he1, he2) = (self.he(h1)?, self.he(h2)?);
        let (l1, l2, v1, v2) = (he1.lp, he2.lp, he1.origin, he2.origin);
        let (lone1, lone2) = (he1.edge.is_none(), he2.edge.is_none());
        if l1 == l2 {
            return Err(invalid(format!(
                "MEKR needs {h1} and {h2} in different loops; use MEF within one loop"
            )));
        }
        let face = self.lp(l1)?.face;
        if self.lp(l2)?.face != face {
            return Err(invalid(format!(
                "MEKR needs {h1} and {h2} in loops of the same face"
            )));
        }
        let f = self.face(face)?;
        let l2_outer = f.outer == l2;
        let s1 = self.walk_edges(h1)?;
        let s2 = self.walk_edges(h2)?;
        self.ensure_room(2)?;
        let (e, a, b) = self.new_edge(v1, l1, v2, l1);
        let mut seq = vec![a];
        seq.extend(s2);
        seq.push(b);
        seq.extend(s1);
        self.link(l1, &seq);
        if let Some(f) = self.faces.get_mut(face.index()) {
            if l2_outer {
                f.outer = l1;
                f.inner.retain(|l| *l != l1);
            } else {
                f.inner.retain(|l| *l != l2);
            }
        }
        self.loops.remove(l2.index());
        if lone1 {
            self.half_edges.remove(h1.index());
            self.set_vertex_half_edge(v1, a);
        }
        if lone2 {
            self.half_edges.remove(h2.index());
            self.set_vertex_half_edge(v2, b);
        }
        Ok(e)
    }

    /// **KFMRH**, kill face, make ring and hole: face `kill`, which must have a single
    /// loop, dies and its loop becomes an inner loop of face `keep`. In one shell this
    /// makes a through hole (genus + 1); if the faces are in different shells, the shells
    /// merge into `keep`'s (S - 1). Inverse: [`Body::mfkrh`].
    pub fn kfmrh(&mut self, keep: FaceId, kill: FaceId) -> KernelResult<()> {
        if keep == kill {
            return Err(invalid("KFMRH needs two different faces"));
        }
        let fk = self.face(keep)?;
        let fd = self.face(kill)?;
        if !fd.inner.is_empty() {
            return Err(invalid(format!("KFMRH needs {kill} to have a single loop")));
        }
        let (sk, sd, ring) = (fk.shell, fd.shell, fd.outer);
        self.shell(sk)?;
        let moved = self.shell(sd)?.faces.clone();
        if let Some(l) = self.loops.get_mut(ring.index()) {
            l.face = keep;
        }
        if let Some(f) = self.faces.get_mut(keep.index()) {
            f.inner.push(ring);
        }
        self.faces.remove(kill.index());
        if sk == sd {
            if let Some(s) = self.shells.get_mut(sd.index()) {
                s.faces.retain(|f| *f != kill);
            }
        } else {
            let moved: Vec<FaceId> = moved.into_iter().filter(|f| *f != kill).collect();
            for f in &moved {
                if let Some(face) = self.faces.get_mut(f.index()) {
                    face.shell = sk;
                }
            }
            if let Some(s) = self.shells.get_mut(sk.index()) {
                s.faces.extend(moved);
            }
            self.shells.remove(sd.index());
        }
        Ok(())
    }

    /// **MFKRH**, make face, kill ring and hole: inner loop `ring` of a face becomes the
    /// outer loop of a new face, with no surface yet. If the new face is still joined to
    /// the old one through edges, this closes a through hole (genus - 1); otherwise the
    /// part holding the new face becomes a new shell (S + 1). Returns the new face.
    /// Inverse of [`Body::kfmrh`].
    pub fn mfkrh(&mut self, ring: LoopId) -> KernelResult<FaceId> {
        let face = self.lp(ring)?.face;
        let f = self.face(face)?;
        if !f.inner.contains(&ring) {
            return Err(invalid(format!(
                "MFKRH needs {ring} to be an inner loop of its face"
            )));
        }
        let shell = f.shell;
        self.shell(shell)?;
        self.ensure_room(1)?;
        let new_face = FaceId::from_index(self.faces.push(Face {
            shell,
            outer: ring,
            inner: Vec::new(),
            surface: None,
            same_sense: true,
            name: None,
        }));
        if let Some(l) = self.loops.get_mut(ring.index()) {
            l.face = new_face;
        }
        if let Some(f) = self.faces.get_mut(face.index()) {
            f.inner.retain(|l| *l != ring);
        }
        if let Some(s) = self.shells.get_mut(shell.index()) {
            s.faces.push(new_face);
        }
        // If no edge path joins the new face to the old one any more, the shell has come
        // apart: the new face's part becomes a shell of its own.
        let part = self.faces_reachable(new_face);
        if !part.contains(&face) {
            let new_shell = ShellId::from_index(self.shells.slots());
            for f in &part {
                if let Some(x) = self.faces.get_mut(f.index()) {
                    x.shell = new_shell;
                }
            }
            if let Some(s) = self.shells.get_mut(shell.index()) {
                s.faces.retain(|f| !part.contains(f));
            }
            self.shells.push(Shell {
                faces: part.into_iter().collect(),
            });
        }
        Ok(new_face)
    }

    /// The faces reachable from `start` across edges (breadth-first), in visiting order.
    pub(super) fn faces_reachable(&self, start: FaceId) -> Vec<FaceId> {
        let mut seen = std::collections::HashSet::from([start]);
        let mut order = vec![start];
        let mut i = 0;
        while let Some(f) = order.get(i).copied() {
            i += 1;
            for e in self.face_edges(f).unwrap_or_default() {
                for g in self.edge_faces(e).unwrap_or([f, f]) {
                    if seen.insert(g) {
                        order.push(g);
                    }
                }
            }
        }
        order
    }
}

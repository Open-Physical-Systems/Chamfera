//! Gluing swept pieces end to end: a body as a face list, and two face lists joined
//! across a pair of matching cap faces.
//!
//! Piece `k + 1` starts exactly where piece `k` ends: its start cap is piece `k`'s end
//! cap, the same profile placed by the same frame, facing the other way. Joining them
//! is the inverse of Mantyla's "glue" of two faces (*An Introduction to Solid Modeling*,
//! 1988, section 9.6): both caps are removed and their boundaries identified, vertex
//! with vertex and edge with edge, so the side faces of both pieces meet along the
//! shared edges. No surface intersection is needed, and the result is assembled with
//! [`Body::from_face_list`] and checked by the caller with [`Body::validate`].

use crate::{Body, Curve, KernelError, KernelResult, ListEdge, ListFace, ListVertex};
use chamfera_geom::LINEAR_TOLERANCE;
use std::collections::HashMap;

fn internal(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

/// A body as a face list.
#[derive(Debug, Clone, Default)]
pub(crate) struct List {
    pub vertices: Vec<ListVertex>,
    pub edges: Vec<ListEdge>,
    pub faces: Vec<ListFace>,
}

impl List {
    /// The face list of `body` (every edge and face must carry geometry).
    pub fn of(body: &Body) -> KernelResult<List> {
        let mut out = List::default();
        let mut vmap = HashMap::new();
        for v in body.vertices() {
            vmap.insert(v, out.vertices.len());
            out.vertices.push(ListVertex {
                point: body.vertex_point(v)?,
                name: body.vertex_name(v)?.map(str::to_owned),
            });
        }
        let mut emap = HashMap::new();
        for e in body.edges() {
            let [a, b] = body.edge_vertices(e)?;
            let (curve, t0, t1) = body
                .edge_curve(e)?
                .ok_or_else(|| internal("a swept edge has no curve"))?;
            let (Some(&a), Some(&b)) = (vmap.get(&a), vmap.get(&b)) else {
                return Err(internal("an edge ends at a vertex the body does not list"));
            };
            emap.insert(e, out.edges.len());
            out.edges.push(ListEdge {
                vertices: [a, b],
                curve,
                t0,
                t1,
                name: body.edge_name(e)?.map(str::to_owned),
            });
        }
        for f in body.faces() {
            let (surface, same_sense) = body
                .face_surface(f)?
                .ok_or_else(|| internal("a swept face has no surface"))?;
            let mut loops = Vec::new();
            for lp in body.face_loops(f)? {
                let mut uses = Vec::new();
                for h in body.loop_half_edges(lp)? {
                    let e = body
                        .half_edge_edge(h)?
                        .ok_or_else(|| internal("a loop of a swept face has no edge"))?;
                    let forward = body.edge_half_edges(e)?[0] == h;
                    let i = *emap
                        .get(&e)
                        .ok_or_else(|| internal("a loop uses an edge the body does not list"))?;
                    uses.push((i, forward));
                }
                loops.push(uses);
            }
            out.faces.push(ListFace {
                surface,
                same_sense,
                name: body.face_name(f)?.map(str::to_owned),
                loops,
            });
        }
        Ok(out)
    }

    /// The index of the face named `name`.
    pub fn face_named(&self, name: &str) -> KernelResult<usize> {
        self.faces
            .iter()
            .position(|f| f.name.as_deref() == Some(name))
            .ok_or_else(|| internal(format!("a swept piece has no face {name:?}")))
    }

    pub fn into_body(self) -> KernelResult<Body> {
        Body::from_face_list(&self.vertices, &self.edges, &self.faces)
    }

    /// The edges used by face `f`.
    fn face_edges(&self, f: usize) -> Vec<usize> {
        self.faces
            .get(f)
            .map(|face| face.loops.iter().flatten().map(|u| u.0).collect())
            .unwrap_or_default()
    }

    /// Joins `next` onto `self` across `self`'s face `end` and `next`'s face `start`,
    /// which must have the same boundary (within [`LINEAR_TOLERANCE`]). Both faces are
    /// removed; `self`'s vertices and edges are kept for the shared boundary.
    pub fn glue(mut self, end: usize, next: List, start: usize) -> KernelResult<List> {
        let ours = self.face_edges(end);
        let theirs = next.face_edges(start);
        if ours.len() != theirs.len() {
            return Err(internal("two swept pieces do not meet along the same loop"));
        }
        // Vertices of the shared boundary, by position.
        let mut vmap: HashMap<usize, usize> = HashMap::new();
        for &te in &theirs {
            let Some(edge) = next.edges.get(te) else {
                return Err(internal("a cap edge is missing"));
            };
            for tv in edge.vertices {
                if vmap.contains_key(&tv) {
                    continue;
                }
                let p = next
                    .vertices
                    .get(tv)
                    .ok_or_else(|| internal("a cap vertex is missing"))?
                    .point;
                let found = ours
                    .iter()
                    .filter_map(|oe| self.edges.get(*oe))
                    .flat_map(|e| e.vertices)
                    .find(|ov| {
                        self.vertices
                            .get(*ov)
                            .is_some_and(|v| v.point.distance(p) <= LINEAR_TOLERANCE)
                    })
                    .ok_or_else(|| internal("two swept pieces do not meet at a vertex"))?;
                vmap.insert(tv, found);
            }
        }
        // Edges of the shared boundary: same end vertices and the same midpoint.
        let mut emap: HashMap<usize, (usize, bool)> = HashMap::new();
        for &te in &theirs {
            let Some(t) = next.edges.get(te) else {
                return Err(internal("a cap edge is missing"));
            };
            let [ta, tb] = [
                vmap.get(&t.vertices[0]).copied(),
                vmap.get(&t.vertices[1]).copied(),
            ];
            let (tm, td) = midpoint(t);
            let mut hit = None;
            for &oe in &ours {
                let Some(o) = self.edges.get(oe) else { continue };
                let (om, od) = midpoint(o);
                if om.distance(tm) > LINEAR_TOLERANCE {
                    continue;
                }
                let same = [Some(o.vertices[0]), Some(o.vertices[1])] == [ta, tb];
                let swapped = [Some(o.vertices[1]), Some(o.vertices[0])] == [ta, tb];
                let flip = if o.vertices[0] == o.vertices[1] {
                    od.dot(td) < 0.0
                } else if same {
                    false
                } else if swapped {
                    true
                } else {
                    continue;
                };
                hit = Some((oe, flip));
                break;
            }
            let hit = hit.ok_or_else(|| internal("two swept pieces do not meet along an edge"))?;
            emap.insert(te, hit);
        }
        // Everything else of `next` is appended.
        for (i, v) in next.vertices.iter().enumerate() {
            if !vmap.contains_key(&i) {
                vmap.insert(i, self.vertices.len());
                self.vertices.push(v.clone());
            }
        }
        for (i, e) in next.edges.iter().enumerate() {
            if emap.contains_key(&i) {
                continue;
            }
            let [a, b] = e.vertices;
            let (Some(&a), Some(&b)) = (vmap.get(&a), vmap.get(&b)) else {
                return Err(internal("a swept edge lost a vertex"));
            };
            emap.insert(i, (self.edges.len(), false));
            self.edges.push(ListEdge {
                vertices: [a, b],
                ..e.clone()
            });
        }
        let mut faces: Vec<ListFace> = self
            .faces
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i != end)
            .map(|(_, f)| f)
            .collect();
        for (i, f) in next.faces.into_iter().enumerate() {
            if i == start {
                continue;
            }
            let mut loops = Vec::with_capacity(f.loops.len());
            for lp in &f.loops {
                let mut uses = Vec::with_capacity(lp.len());
                for &(e, fwd) in lp {
                    let &(ne, flip) = emap
                        .get(&e)
                        .ok_or_else(|| internal("a swept face uses an unknown edge"))?;
                    uses.push((ne, fwd ^ flip));
                }
                loops.push(uses);
            }
            faces.push(ListFace { loops, ..f });
        }
        self.faces = faces;
        Ok(self)
    }
}

/// An edge's midpoint and its direction of travel there.
fn midpoint(e: &ListEdge) -> (chamfera_geom::Point3, chamfera_geom::Vec3) {
    let tm = 0.5 * (e.t0 + e.t1);
    let sign = if e.t1 >= e.t0 { 1.0 } else { -1.0 };
    let c: &Curve = &e.curve;
    (c.point_at(tm), c.d1(tm) * sign)
}

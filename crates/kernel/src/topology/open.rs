//! Surface bodies from face lists: a body's faces as a face list, and a body with some of
//! its faces removed (so it becomes an open shell). See the module documentation of
//! `topology` for how openings are represented.

use super::{Body, FaceId, ListEdge, ListFace, ListVertex, Validation};
use crate::{KernelError, KernelResult};
use std::collections::{HashMap, HashSet};

/// A face list (as `Body::from_face_list` takes) of some faces of a body.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FaceList {
    pub vertices: Vec<ListVertex>,
    pub edges: Vec<ListEdge>,
    pub faces: Vec<ListFace>,
}

impl FaceList {
    /// Assembles the list as a body: a solid when every edge has two uses (checked with
    /// [`Body::validate`]), else a surface body (checked with [`Validation::Surface`]).
    pub fn assemble(&self) -> KernelResult<Body> {
        let body = Body::from_face_list_open(&self.vertices, &self.edges, &self.faces)?;
        let mode = if body.is_open() {
            Validation::Surface
        } else {
            Validation::Solid
        };
        body.validate_as(mode)?;
        Ok(body)
    }

    /// Reverses every face: loops run the other way and the orientation flags flip.
    pub fn reverse(&mut self) {
        for f in &mut self.faces {
            f.same_sense = !f.same_sense;
            for l in &mut f.loops {
                l.reverse();
                for u in l.iter_mut() {
                    u.1 = !u.1;
                }
            }
        }
    }
}

impl Body {
    /// The faces of this body for which `keep` is true, as a face list: their edges and
    /// vertices (only those the kept faces use), with geometry and names. Fails for a
    /// face with no surface or a loop that is a lone vertex.
    pub(crate) fn face_list(&self, keep: impl Fn(FaceId) -> bool) -> KernelResult<FaceList> {
        let mut out = FaceList::default();
        let mut vmap = HashMap::new();
        let mut emap = HashMap::new();
        for f in self.faces() {
            if !keep(f) {
                continue;
            }
            let (surface, same_sense) = self
                .face_surface(f)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{f} has no surface")))?;
            let mut loops = Vec::new();
            for l in self.face_loops(f)? {
                let mut lp = Vec::new();
                for h in self.loop_half_edges(l)? {
                    let e = self.half_edge_edge(h)?.ok_or_else(|| {
                        KernelError::Unsupported(format!(
                            "{l} is a lone vertex, not a loop of edges"
                        ))
                    })?;
                    let index = match emap.get(&e) {
                        Some(i) => *i,
                        None => {
                            let (curve, t0, t1) = self.edge_curve(e)?.ok_or_else(|| {
                                KernelError::InvalidInput(format!("{e} has no curve"))
                            })?;
                            let mut vv = [0usize; 2];
                            for (k, v) in self.edge_vertices(e)?.into_iter().enumerate() {
                                vv[k] = match vmap.get(&v) {
                                    Some(i) => *i,
                                    None => {
                                        out.vertices.push(ListVertex {
                                            point: self.vertex_point(v)?,
                                            name: self.vertex_name(v)?.map(str::to_owned),
                                        });
                                        vmap.insert(v, out.vertices.len() - 1);
                                        out.vertices.len() - 1
                                    }
                                };
                            }
                            out.edges.push(ListEdge {
                                vertices: vv,
                                curve,
                                t0,
                                t1,
                                name: self.edge_name(e)?.map(str::to_owned),
                            });
                            emap.insert(e, out.edges.len() - 1);
                            out.edges.len() - 1
                        }
                    };
                    lp.push((index, self.edge_half_edges(e)?[0] == h));
                }
                loops.push(lp);
            }
            out.faces.push(ListFace {
                surface,
                same_sense,
                name: self.face_name(f)?.map(str::to_owned),
                loops,
            });
        }
        Ok(out)
    }

    /// This body with `remove` taken out, leaving openings where they were: a surface
    /// body (or the same solid when nothing is removed). Faces, edges and vertices keep
    /// their names; edges and vertices no remaining face uses go. Fails when nothing would
    /// be left, or when the remaining faces touch only at a point (a rim passing a vertex
    /// twice).
    pub fn without_faces(&self, remove: &[FaceId]) -> KernelResult<Body> {
        let gone: HashSet<FaceId> = remove.iter().copied().collect();
        for f in &gone {
            self.face(*f)?;
        }
        let list = self.face_list(|f| !gone.contains(&f))?;
        if list.faces.is_empty() {
            return Err(KernelError::EmptyResult);
        }
        list.assemble()
    }
}

//! The shared vertices and edges of a boolean: every point and edge piece both operands
//! and their intersection curves produce, each geometric point or piece once.
//!
//! Vertices are merged within [`LINEAR_TOLERANCE`] (a spatial hash finds the candidates).
//! Two edge pieces are the same edge when they join the same two vertices and each one's
//! midpoint and quarter points lie on the other: an operand's edge, the intersection curve
//! that runs along it and the other operand's edge on the same line all become one edge,
//! which is what lets the pieces sew together.

use super::MAX_BOOLEAN_ENTITIES;
use super::span::{Span, internal};
use crate::{KernelError, KernelResult};
use chamfera_geom::{LINEAR_TOLERANCE, Point3};
use std::collections::HashMap;

/// Size of a spatial-hash cell (mm): far larger than the tolerance, so a point's matches
/// are in its own cell or a neighbour.
const CELL: f64 = 1e-3;

/// An edge of the registry, oriented from `v[0]` to `v[1]` along `span`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EdgeRec {
    pub span: Span,
    pub v: [usize; 2],
    pub name: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct Registry {
    pub points: Vec<Point3>,
    pub vertex_names: Vec<Option<String>>,
    grid: HashMap<(i64, i64, i64), Vec<usize>>,
    pub edges: Vec<EdgeRec>,
    by_pair: HashMap<(usize, usize), Vec<usize>>,
}

fn cell(p: Point3) -> (i64, i64, i64) {
    (
        (p.x / CELL).floor() as i64,
        (p.y / CELL).floor() as i64,
        (p.z / CELL).floor() as i64,
    )
}

fn too_many() -> KernelError {
    KernelError::InvalidInput(format!(
        "the boolean would need more than {MAX_BOOLEAN_ENTITIES} vertices or edges"
    ))
}

impl Registry {
    /// The vertex at `p` (within the tolerance), if there is one.
    pub fn find_vertex(&self, p: Point3) -> Option<usize> {
        let (x, y, z) = cell(p);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = self.grid.get(&(x + dx, y + dy, z + dz)) {
                        for &i in ids {
                            if let Some(q) = self.points.get(i) {
                                let d = q.distance(p);
                                if d <= LINEAR_TOLERANCE && best.is_none_or(|b| d < b.0) {
                                    best = Some((d, i));
                                }
                            }
                        }
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The vertex at `p`, added if new.
    pub fn vertex(&mut self, p: Point3, name: Option<&str>) -> KernelResult<usize> {
        if !p.is_valid() {
            return Err(internal("an intersection point left the working range"));
        }
        if let Some(i) = self.find_vertex(p) {
            if self.vertex_names.get(i).is_some_and(Option::is_none)
                && let Some(n) = name
                && let Some(slot) = self.vertex_names.get_mut(i)
            {
                *slot = Some(n.to_owned());
            }
            return Ok(i);
        }
        if self.points.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(too_many());
        }
        let i = self.points.len();
        self.points.push(p);
        self.vertex_names.push(name.map(str::to_owned));
        self.grid.entry(cell(p)).or_default().push(i);
        Ok(i)
    }

    pub fn point(&self, v: usize) -> KernelResult<Point3> {
        self.points
            .get(v)
            .copied()
            .ok_or_else(|| internal(format!("vertex {v} is missing")))
    }

    pub fn edge(&self, e: usize) -> KernelResult<&EdgeRec> {
        self.edges
            .get(e)
            .ok_or_else(|| internal(format!("edge {e} is missing")))
    }

    /// The edge from `v0` to `v1` along `span`, added if new. Returns its id and whether
    /// `span` runs along it (true) or against it.
    pub fn edge_piece(
        &mut self,
        span: Span,
        v0: usize,
        v1: usize,
        name: Option<&str>,
    ) -> KernelResult<(usize, bool)> {
        let key = (v0.min(v1), v0.max(v1));
        if let Some(ids) = self.by_pair.get(&key) {
            for &e in ids {
                let Some(rec) = self.edges.get(e) else {
                    continue;
                };
                if same_piece(&rec.span, &span)? {
                    let forward = if v0 != v1 {
                        rec.v[0] == v0
                    } else {
                        // A closed piece: compare directions at a common point.
                        let p = span.at(0.25);
                        let t = rec.span.param_of(p)?.unwrap_or(rec.span.t0);
                        let d_new = span.carrier.d1(span.t0 + 0.25 * (span.t1 - span.t0));
                        d_new.dot(rec.span.carrier.d1(t)) > 0.0
                    };
                    if rec.name.is_none()
                        && let Some(n) = name
                        && let Some(r) = self.edges.get_mut(e)
                    {
                        r.name = Some(n.to_owned());
                    }
                    return Ok((e, forward));
                }
            }
        }
        if self.edges.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(too_many());
        }
        let e = self.edges.len();
        self.edges.push(EdgeRec {
            span,
            v: [v0, v1],
            name: name.map(str::to_owned),
        });
        self.by_pair.entry(key).or_default().push(e);
        Ok((e, true))
    }

    /// A copy of edge `e` as a new edge (for splitting a non-manifold edge).
    pub fn duplicate_edge(&mut self, e: usize) -> KernelResult<usize> {
        let rec = self.edge(e)?.clone();
        if self.edges.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(too_many());
        }
        self.edges.push(rec);
        Ok(self.edges.len() - 1)
    }

    /// A copy of vertex `v` (for splitting a non-manifold vertex).
    pub fn duplicate_vertex(&mut self, v: usize) -> KernelResult<usize> {
        let p = self.point(v)?;
        let name = self.vertex_names.get(v).cloned().flatten();
        if self.points.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(too_many());
        }
        self.points.push(p);
        self.vertex_names.push(name);
        Ok(self.points.len() - 1)
    }

    /// A new edge not merged with any other (joined edges).
    pub fn new_edge(&mut self, rec: EdgeRec) -> KernelResult<usize> {
        if self.edges.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(too_many());
        }
        let key = (rec.v[0].min(rec.v[1]), rec.v[0].max(rec.v[1]));
        self.edges.push(rec);
        let e = self.edges.len() - 1;
        self.by_pair.entry(key).or_default().push(e);
        Ok(e)
    }
}

/// Two spans with the same end vertices are the same piece when sample points of each lie
/// on the other.
fn same_piece(a: &Span, b: &Span) -> KernelResult<bool> {
    for s in [0.25, 0.5, 0.75] {
        if a.distance(b.at(s))? > LINEAR_TOLERANCE || b.distance(a.at(s))? > LINEAR_TOLERANCE {
            return Ok(false);
        }
    }
    Ok(true)
}

//! Assembling a body from a face list (for booleans, K7).
//!
//! A boolean ends with a set of faces, each bounded by loops of oriented edge uses, that
//! together close one or more shells. [`Body::from_face_list`] turns such a list into the
//! half-edge structure in one step: one half-edge per edge use, linked in loop order, mated
//! with the opposite use of the same edge. It is the structure the Euler operators would
//! build face by face (Mantyla, *An Introduction to Solid Modeling*, section 11.5 builds
//! bodies from face lists the same way); building it directly avoids choosing an operator
//! sequence for an arbitrary shell. Nothing is trusted: every precondition is checked,
//! shells are the connected components of faces across edges, and the result goes through
//! [`Body::validate_topology`] before it is returned. The caller adds [`Body::validate`].

use super::{
    Body, CurveId, Edge, EdgeCurve, EdgeId, Face, FaceId, HalfEdge, HalfEdgeId, Loop, LoopId,
    MAX_TOPOLOGY_ENTITIES, PointId, Shell, ShellId, SurfaceId, Vertex, VertexId, check_name,
};
use crate::{Curve, KernelError, KernelResult, Surface};
use chamfera_geom::Point3;

/// A vertex of a face list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListVertex {
    pub point: Point3,
    pub name: Option<String>,
}

/// An edge of a face list: from `vertices[0]` to `vertices[1]` along `curve` over
/// `[t0, t1]`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListEdge {
    pub vertices: [usize; 2],
    pub curve: Curve,
    pub t0: f64,
    pub t1: f64,
    pub name: Option<String>,
}

/// A face of a face list: its surface, orientation flag and name, and its loops (outer
/// first) as `(edge index, runs along the edge)` uses in loop order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListFace {
    pub surface: Surface,
    pub same_sense: bool,
    pub name: Option<String>,
    pub loops: Vec<Vec<(usize, bool)>>,
}

fn bad(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

/// Union-find root with path halving.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    for _ in 0..parent.len() {
        let p = parent[i];
        if p == i {
            return i;
        }
        let gp = parent[p];
        parent[i] = gp;
        i = gp;
    }
    i
}

impl Body {
    /// The body bounded by `faces`. Every edge must be used exactly twice, once in each
    /// direction; consecutive uses in a loop must chain head to tail; every vertex must be
    /// used. Shells are the connected components of faces across edges. Fails with
    /// [`KernelError::InvalidResult`] (and builds nothing) when the list is inconsistent.
    pub(crate) fn from_face_list(
        vertices: &[ListVertex],
        edges: &[ListEdge],
        faces: &[ListFace],
    ) -> KernelResult<Body> {
        Body::assemble(vertices, edges, faces, false)
    }

    /// [`Body::from_face_list`] for a surface body: an edge may also be used once, by one
    /// face. Each missing use becomes a half-edge of a boundary face (an opening, see the
    /// module documentation of `topology`): the missing uses chain head to tail into the
    /// loops of the openings, one opening per loop. Where two missing uses start at one
    /// vertex (sheets touching at a point) the list is refused. A list where every edge is
    /// used twice gives the same closed body as [`Body::from_face_list`].
    pub(crate) fn from_face_list_open(
        vertices: &[ListVertex],
        edges: &[ListEdge],
        faces: &[ListFace],
    ) -> KernelResult<Body> {
        Body::assemble(vertices, edges, faces, true)
    }

    fn assemble(
        vertices: &[ListVertex],
        edges: &[ListEdge],
        faces: &[ListFace],
        open: bool,
    ) -> KernelResult<Body> {
        let loops_total: usize = faces.iter().map(|f| f.loops.len()).sum();
        let uses_total: usize = faces.iter().flat_map(|f| &f.loops).map(Vec::len).sum();
        if [
            vertices.len(),
            edges.len(),
            faces.len(),
            loops_total,
            uses_total,
        ]
        .iter()
        .any(|n| *n > MAX_TOPOLOGY_ENTITIES)
        {
            return Err(KernelError::InvalidInput(format!(
                "a body may hold at most {MAX_TOPOLOGY_ENTITIES} entities of each kind"
            )));
        }
        if faces.is_empty() {
            return Err(bad("a body needs at least one face"));
        }
        // Each edge: the half-edge running along it and the one against it.
        let mut halves: Vec<[Option<usize>; 2]> = vec![[None, None]; edges.len()];
        let mut edge_face: Vec<Option<usize>> = vec![None; edges.len()];
        let mut parent: Vec<usize> = (0..faces.len()).collect();
        let mut h = 0usize;
        for (fi, f) in faces.iter().enumerate() {
            if let Some(n) = &f.name {
                check_name(n)?;
            }
            for lp in &f.loops {
                if lp.is_empty() {
                    return Err(bad(format!("face {fi} of the list has an empty loop")));
                }
                for (k, &(e, fwd)) in lp.iter().enumerate() {
                    let edge = edges.get(e).ok_or_else(|| {
                        bad(format!("face {fi} uses edge {e}, which is not listed"))
                    })?;
                    let slot = &mut halves[e][usize::from(!fwd)];
                    if slot.is_some() {
                        return Err(bad(format!(
                            "edge {e} is used more than once in the same direction"
                        )));
                    }
                    *slot = Some(h);
                    h += 1;
                    match edge_face[e] {
                        None => edge_face[e] = Some(fi),
                        Some(other) => {
                            let (a, b) = (root(&mut parent, other), root(&mut parent, fi));
                            parent[a] = b;
                        }
                    }
                    // Head to tail with the next use.
                    let end = edge.vertices[usize::from(fwd)];
                    let (ne, nf) = lp[(k + 1) % lp.len()];
                    let next = edges.get(ne).ok_or_else(|| {
                        bad(format!("face {fi} uses edge {ne}, which is not listed"))
                    })?;
                    let start = next.vertices[usize::from(!nf)];
                    if end != start {
                        return Err(bad(format!(
                            "a loop of face {fi} does not chain head to tail at edge {e}"
                        )));
                    }
                }
            }
        }
        // Openings: each missing use, chained head to tail into loops.
        let mut openings: Vec<Vec<(usize, bool)>> = Vec::new();
        if open {
            let mut missing: Vec<(usize, bool)> = Vec::new();
            let mut starting: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();
            for (e, edge) in edges.iter().enumerate() {
                if halves[e].iter().all(Option::is_none) {
                    return Err(bad(format!("edge {e} is used by no face")));
                }
                for (slot, fwd) in [(0usize, true), (1, false)] {
                    if halves[e][slot].is_none() {
                        let origin = edge.vertices[usize::from(!fwd)];
                        if starting.insert(origin, missing.len()).is_some() {
                            return Err(bad(format!(
                                "the surface's rim passes vertex {origin} more than once (sheets touching at a point)"
                            )));
                        }
                        missing.push((e, fwd));
                    }
                }
            }
            let mut done = vec![false; missing.len()];
            for start in 0..missing.len() {
                if done[start] {
                    continue;
                }
                let mut cycle = Vec::new();
                let mut k = start;
                for _ in 0..=missing.len() {
                    if done[k] {
                        break;
                    }
                    done[k] = true;
                    let (e, fwd) = missing[k];
                    cycle.push((e, fwd));
                    let end = edges[e].vertices[usize::from(fwd)];
                    k = *starting.get(&end).ok_or_else(|| {
                        bad(format!("the rim does not close at vertex {end}"))
                    })?;
                }
                if k != start {
                    return Err(bad("the rim of the surface does not close into loops"));
                }
                // Number the opening's half-edges after every face's.
                for &(e, fwd) in &cycle {
                    halves[e][usize::from(!fwd)] = Some(h);
                    h += 1;
                }
                // The opening joins the shell of the face across its first edge.
                parent.push(parent.len());
                if let Some(f) = cycle.first().and_then(|(e, _)| edge_face[*e]) {
                    let (a, b) = (root(&mut parent, f), parent.len() - 1);
                    parent[b] = a;
                }
                openings.push(cycle);
            }
        }
        for (e, edge) in edges.iter().enumerate() {
            if halves[e].iter().any(Option::is_none) {
                return Err(bad(format!(
                    "edge {e} is not used once in each direction, so the faces do not close"
                )));
            }
            if let Some(n) = &edge.name {
                check_name(n)?;
            }
            if edge.vertices.iter().any(|v| *v >= vertices.len()) {
                return Err(bad(format!(
                    "edge {e} refers to a vertex that is not listed"
                )));
            }
        }

        let mut body = Body::new();
        // Shells: one per connected component, in order of their first face.
        let all_faces = faces.len() + openings.len();
        let mut shell_of_root: Vec<Option<usize>> = vec![None; all_faces];
        let mut face_shell = Vec::with_capacity(all_faces);
        for fi in 0..all_faces {
            let r = root(&mut parent, fi);
            let s = match shell_of_root[r] {
                Some(s) => s,
                None => {
                    let s = body.shells.push(Shell { faces: Vec::new() });
                    shell_of_root[r] = Some(s);
                    s
                }
            };
            face_shell.push(s);
        }
        let mut vertex_half: Vec<Option<usize>> = vec![None; vertices.len()];
        for (vi, v) in vertices.iter().enumerate() {
            if let Some(n) = &v.name {
                check_name(n)?;
            }
            let p = body.points.push(v.point);
            body.vertices.push(Vertex {
                half_edge: HalfEdgeId::from_index(0),
                point: PointId::from_index(p),
                name: v.name.clone(),
            });
            if p != vi {
                return Err(bad("vertex numbering went out of step"));
            }
        }
        for (e, edge) in edges.iter().enumerate() {
            let c = body.curves.push(edge.curve.clone());
            let [Some(a), Some(b)] = halves[e] else {
                return Err(bad(format!("edge {e} lost a half-edge")));
            };
            body.edges.push(Edge {
                halves: [HalfEdgeId::from_index(a), HalfEdgeId::from_index(b)],
                curve: Some(EdgeCurve {
                    curve: CurveId::from_index(c),
                    t0: edge.t0,
                    t1: edge.t1,
                }),
                name: edge.name.clone(),
            });
        }
        let mut lp_index = 0usize;
        let mut h = 0usize;
        for (fi, f) in faces.iter().enumerate() {
            let s = body.surfaces.push(f.surface.clone());
            let first_loop = lp_index;
            for lp in &f.loops {
                let first = h;
                let n = lp.len();
                for (k, &(e, fwd)) in lp.iter().enumerate() {
                    let edge = &edges[e];
                    let origin = edge.vertices[usize::from(!fwd)];
                    vertex_half[origin].get_or_insert(h);
                    let next = first + (k + 1) % n;
                    let prev = first + (k + n - 1) % n;
                    body.half_edges.push(HalfEdge {
                        lp: LoopId::from_index(lp_index),
                        edge: Some(EdgeId::from_index(e)),
                        origin: VertexId::from_index(origin),
                        next: HalfEdgeId::from_index(next),
                        prev: HalfEdgeId::from_index(prev),
                    });
                    h += 1;
                }
                body.loops.push(Loop {
                    face: FaceId::from_index(fi),
                    first: HalfEdgeId::from_index(first),
                });
                lp_index += 1;
            }
            let shell = face_shell[fi];
            body.faces.push(Face {
                shell: ShellId::from_index(shell),
                outer: LoopId::from_index(first_loop),
                inner: (first_loop + 1..lp_index).map(LoopId::from_index).collect(),
                surface: Some(SurfaceId::from_index(s)),
                same_sense: f.same_sense,
                name: f.name.clone(),
                boundary: false,
            });
            if let Some(sh) = body.shells.get_mut(shell) {
                sh.faces.push(FaceId::from_index(fi));
            }
        }
        for (j, cycle) in openings.iter().enumerate() {
            let fi = faces.len() + j;
            let first = h;
            let n = cycle.len();
            for (k, &(e, fwd)) in cycle.iter().enumerate() {
                let origin = edges[e].vertices[usize::from(!fwd)];
                vertex_half[origin].get_or_insert(h);
                body.half_edges.push(HalfEdge {
                    lp: LoopId::from_index(lp_index),
                    edge: Some(EdgeId::from_index(e)),
                    origin: VertexId::from_index(origin),
                    next: HalfEdgeId::from_index(first + (k + 1) % n),
                    prev: HalfEdgeId::from_index(first + (k + n - 1) % n),
                });
                h += 1;
            }
            body.loops.push(Loop {
                face: FaceId::from_index(fi),
                first: HalfEdgeId::from_index(first),
            });
            let shell = face_shell[fi];
            body.faces.push(Face {
                shell: ShellId::from_index(shell),
                outer: LoopId::from_index(lp_index),
                inner: Vec::new(),
                surface: None,
                same_sense: true,
                name: None,
                boundary: true,
            });
            lp_index += 1;
            if let Some(sh) = body.shells.get_mut(shell) {
                sh.faces.push(FaceId::from_index(fi));
            }
        }
        for (vi, half) in vertex_half.iter().enumerate() {
            let Some(half) = half else {
                return Err(bad(format!("vertex {vi} of the list is used by no edge")));
            };
            if let Some(v) = body.vertices.get_mut(vi) {
                v.half_edge = HalfEdgeId::from_index(*half);
            }
        }
        body.validate_topology()?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Segment3, TopologyError};
    use chamfera_geom::{Frame, Vec3};

    /// The unit cube as a face list: 8 corners, 12 segments, 6 squares.
    fn cube() -> (Vec<ListVertex>, Vec<ListEdge>, Vec<ListFace>) {
        let p =
            |i: usize| Point3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64);
        let vertices: Vec<ListVertex> = (0..8)
            .map(|i| ListVertex {
                point: p(i),
                name: None,
            })
            .collect();
        let pairs = [
            (0, 1),
            (1, 3),
            (3, 2),
            (2, 0),
            (4, 5),
            (5, 7),
            (7, 6),
            (6, 4),
            (0, 4),
            (1, 5),
            (3, 7),
            (2, 6),
        ];
        let edges: Vec<ListEdge> = pairs
            .iter()
            .map(|&(a, b)| ListEdge {
                vertices: [a, b],
                curve: Curve::Segment(Segment3::new(p(a), p(b)).unwrap()),
                t0: 0.0,
                t1: 1.0,
                name: None,
            })
            .collect();
        let plane = |o: Point3, n: Vec3| Surface::Plane(Frame::from_normal(o, n).unwrap());
        let o = Point3::ORIGIN;
        let one = Point3::new(1.0, 1.0, 1.0);
        let faces = vec![
            // Bottom (normal -z): 0 -> 2 -> 3 -> 1.
            (
                plane(o, -Vec3::Z),
                vec![(3, false), (2, false), (1, false), (0, false)],
            ),
            // Top (+z): 4 -> 5 -> 7 -> 6.
            (
                plane(one, Vec3::Z),
                vec![(4, true), (5, true), (6, true), (7, true)],
            ),
            // y = 0 (-y): 0 -> 1 -> 5 -> 4.
            (
                plane(o, -Vec3::Y),
                vec![(0, true), (9, true), (4, false), (8, false)],
            ),
            // x = 1 (+x): 1 -> 3 -> 7 -> 5.
            (
                plane(one, Vec3::X),
                vec![(1, true), (10, true), (5, false), (9, false)],
            ),
            // y = 1 (+y): 3 -> 2 -> 6 -> 7.
            (
                plane(one, Vec3::Y),
                vec![(2, true), (11, true), (6, false), (10, false)],
            ),
            // x = 0 (-x): 2 -> 0 -> 4 -> 6.
            (
                plane(o, -Vec3::X),
                vec![(3, true), (8, true), (7, false), (11, false)],
            ),
        ]
        .into_iter()
        .map(|(surface, lp)| ListFace {
            surface,
            same_sense: true,
            name: None,
            loops: vec![lp],
        })
        .collect();
        (vertices, edges, faces)
    }

    #[test]
    fn a_cube_face_list_assembles_into_a_valid_unit_cube() {
        let (v, e, f) = cube();
        let b = Body::from_face_list(&v, &e, &f).unwrap();
        let r = b.validate().unwrap();
        // V - E + F = 8 - 12 + 6 = 2: one shell, genus 0.
        assert_eq!(
            (r.summary.vertices, r.summary.edges, r.summary.faces),
            (8, 12, 6)
        );
        assert_eq!((r.summary.shells, r.genus), (1, 0));
        assert!((b.volume().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_open_or_inconsistent_face_list_is_refused() {
        let (v, e, mut f) = cube();
        f.pop();
        assert!(matches!(
            Body::from_face_list(&v, &e, &f),
            Err(KernelError::InvalidResult(_))
        ));
        let (v, e, mut f) = cube();
        f[0].loops[0].swap(0, 1);
        assert!(Body::from_face_list(&v, &e, &f).is_err());
        let (v, e, mut f) = cube();
        f[0].loops[0][0].0 = 99;
        assert!(Body::from_face_list(&v, &e, &f).is_err());
    }

    #[test]
    fn a_cube_with_inward_faces_is_a_void_and_needs_a_lump_around_it() {
        // Reversed loops and flags: the same cube with its faces pointing in.
        let (v, e, f) = cube();
        let inward: Vec<ListFace> = f
            .iter()
            .map(|face| ListFace {
                surface: face.surface.clone(),
                same_sense: false,
                name: None,
                loops: vec![face.loops[0].iter().rev().map(|&(e, d)| (e, !d)).collect()],
            })
            .collect();
        let b = Body::from_face_list(&v, &e, &inward).unwrap();
        assert!(matches!(b.validate(), Err(TopologyError::VoidOutside(_))));
    }
}

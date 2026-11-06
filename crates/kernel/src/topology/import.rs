//! Building a one-shell body from a boundary description, for importers (STEP).
//!
//! An exchange file describes a solid as faces bounded by loops of oriented edge uses,
//! not as a sequence of Euler operators. [`Body::from_boundary`] links that description
//! into the half-edge structure directly and then checks the result with
//! [`Body::validate`], so a body is returned only when it is a valid solid.
//!
//! # The description
//! - `points`: one per vertex. The vertex of point `i` is `VertexId` index `i`.
//! - `edges`: `(from, to, curve, t0, t1)`: the edge runs from vertex `from` to vertex `to`
//!   along `curve` over `[t0, t1]` (`curve(t0)` at `from`, `curve(t1)` at `to`). Edge `i`
//!   is `EdgeId` index `i`.
//! - `faces`: `(surface, same_sense, loops)`. Each loop is a cycle of edge uses
//!   `(edge, forward)`, `forward` when the use runs from the edge's `from` to its `to`.
//!   Loops run with the face on their left seen against the face's outward normal (the
//!   kernel's and Part 42's convention). Face `i` is `FaceId` index `i`; loops are
//!   numbered in input order over all faces (`LoopId` index), which lets a caller map an
//!   error back to its own entities.
//!
//! The loops of a face may come in any order: the outer loop is the first one that runs
//! counter-clockwise about the outward normal (planar faces: its vector area; curved
//! faces: its closed `(u, v)` image). When no loop can be told to be outer (a band
//! around a cylinder, whose loops do not close in `(u, v)`), the first loop is.
//!
//! Every edge must be used exactly twice, once in each direction: the description is a
//! closed, consistently oriented shell.

use super::{
    Arena, Body, Edge, EdgeCurve, EdgeId, Face, FaceId, HalfEdge, HalfEdgeId, Loop, LoopId,
    MAX_TOPOLOGY_ENTITIES, Shell, ShellId, Surface, SurfaceId, TopologyError, Vertex, VertexId,
};
use super::{Curve, CurveId, PointId};
use chamfera_geom::{LINEAR_TOLERANCE, Point3};

/// An edge of a boundary description: from vertex, to vertex, curve, `t0`, `t1`
/// (private aliases, so the public signature is plain tuples).
type InEdge = (usize, usize, Curve, f64, f64);
/// A face of a boundary description: surface, `same_sense`, loops of `(edge, forward)`.
type InFace = (Surface, bool, Vec<Vec<(usize, bool)>>);

fn dangling(from: impl std::fmt::Display, to: impl std::fmt::Display) -> TopologyError {
    TopologyError::Dangling {
        from: from.to_string(),
        to: to.to_string(),
    }
}

impl Body {
    /// Builds a one-shell solid from a boundary description (see the module
    /// documentation of `topology::import` for its form and numbering) and validates it
    /// with [`Body::validate`]. The error names the first entity that is wrong, by the
    /// ids the description implies.
    pub fn from_boundary(
        points: &[Point3],
        edges: &[InEdge],
        faces: &[InFace],
    ) -> Result<Body, TopologyError> {
        let body = link(points, edges, faces)?;
        body.validate()?;
        Ok(body)
    }
}

/// The half-edge structure of the description, before geometric validation.
fn link(points: &[Point3], edges: &[InEdge], faces: &[InFace]) -> Result<Body, TopologyError> {
    let uses: usize = faces
        .iter()
        .flat_map(|f| f.2.iter())
        .map(Vec::len)
        .fold(0usize, usize::saturating_add);
    let loops: usize = faces
        .iter()
        .map(|f| f.2.len())
        .fold(0, usize::saturating_add);
    if [points.len(), edges.len(), faces.len(), uses, loops]
        .iter()
        .any(|n| *n > MAX_TOPOLOGY_ENTITIES)
    {
        return Err(TopologyError::TooLarge);
    }
    let shell = ShellId::from_index(0);
    if faces.is_empty() {
        return Err(TopologyError::EmptyShell(shell));
    }
    let mut body = Body::new();
    for p in points {
        body.points.push(*p);
    }
    for (i, (from, to, curve, t0, t1)) in edges.iter().enumerate() {
        let e = EdgeId::from_index(i);
        for v in [*from, *to] {
            if v >= points.len() {
                return Err(dangling(e, VertexId::from_index(v)));
            }
        }
        curve
            .check_range(*t0, *t1)
            .map_err(|err| TopologyError::BadCurveRange(e, err.to_string()))?;
        body.curves.push(curve.clone());
    }
    // Per edge: its forward and backward use.
    let mut halves: Vec<[Option<HalfEdgeId>; 2]> = vec![[None, None]; edges.len()];
    // Per vertex: a half-edge starting there.
    let mut starts: Vec<Option<HalfEdgeId>> = vec![None; points.len()];
    let mut shell_faces = Vec::with_capacity(faces.len());
    for (fi, (surface, same_sense, face_loops)) in faces.iter().enumerate() {
        let f = FaceId::from_index(fi);
        let s = SurfaceId::from_index(body.surfaces.push(surface.clone()));
        let mut loop_ids = Vec::with_capacity(face_loops.len());
        for uses in face_loops {
            let l = LoopId::from_index(body.loops.slots());
            if uses.is_empty() {
                return Err(TopologyError::VertexLoop(f));
            }
            let first = body.half_edges.slots();
            let n = uses.len();
            for (k, &(ei, forward)) in uses.iter().enumerate() {
                let h = HalfEdgeId::from_index(first + k);
                let (from, to, ..) = edges
                    .get(ei)
                    .ok_or_else(|| dangling(l, EdgeId::from_index(ei)))?;
                let e = EdgeId::from_index(ei);
                let (origin, end) = if forward { (*from, *to) } else { (*to, *from) };
                // The next use must start where this one ends.
                let (ni, nf) = uses[(k + 1) % n];
                let (nfrom, nto, ..) = edges
                    .get(ni)
                    .ok_or_else(|| dangling(l, EdgeId::from_index(ni)))?;
                if end != if nf { *nfrom } else { *nto } {
                    return Err(TopologyError::OpenLoop(l));
                }
                let slot = &mut halves[ei][usize::from(!forward)];
                if slot.is_some() {
                    return Err(TopologyError::TwinsNotOpposite(e));
                }
                *slot = Some(h);
                if let Some(start) = starts.get_mut(origin)
                    && start.is_none()
                {
                    *start = Some(h);
                }
                body.half_edges.push(HalfEdge {
                    lp: l,
                    edge: Some(e),
                    origin: VertexId::from_index(origin),
                    next: HalfEdgeId::from_index(first + (k + 1) % n),
                    prev: HalfEdgeId::from_index(first + (k + n - 1) % n),
                });
            }
            body.loops.push(Loop {
                face: f,
                first: HalfEdgeId::from_index(first),
            });
            loop_ids.push(l);
        }
        let Some((&outer, inner)) = loop_ids.split_first() else {
            return Err(TopologyError::VertexLoop(f));
        };
        body.faces.push(Face {
            shell,
            outer,
            inner: inner.to_vec(),
            surface: Some(s),
            same_sense: *same_sense,
            name: None,
        });
        shell_faces.push(f);
    }
    body.shells.push(Shell { faces: shell_faces });
    for (i, (h, (_, _, _, t0, t1))) in halves.iter().zip(edges).enumerate() {
        let e = EdgeId::from_index(i);
        let [Some(a), Some(b)] = *h else {
            return Err(TopologyError::MissingTwin(e));
        };
        let (t0, t1) = (*t0, *t1);
        body.edges.push(Edge {
            halves: [a, b],
            curve: Some(EdgeCurve {
                curve: CurveId::from_index(i),
                t0,
                t1,
            }),
            name: None,
        });
    }
    // Vertices keep their point's index; a point no edge uses leaves an empty slot.
    let mut vertices = Arena::default();
    for (i, start) in starts.iter().enumerate() {
        match start {
            Some(h) => {
                vertices.push(Vertex {
                    half_edge: *h,
                    point: PointId::from_index(i),
                    name: None,
                });
            }
            None => {
                vertices.items.push(None);
            }
        }
    }
    body.vertices = vertices;
    body.validate_topology()?;
    body.choose_outer_loops();
    Ok(body)
}

impl Body {
    /// Makes the first counter-clockwise loop of each face (see the module
    /// documentation) its outer loop, keeping the others in order as rings.
    fn choose_outer_loops(&mut self) {
        for fi in 0..self.faces.slots() {
            let f = FaceId::from_index(fi);
            let Ok(loops) = self.face_loops(f) else {
                continue;
            };
            if loops.len() < 2 {
                continue;
            }
            let Ok(Some((surface, same_sense))) = self.face_surface(f) else {
                continue;
            };
            let orient = if same_sense { 1.0 } else { -1.0 };
            let runs_ccw = |l: LoopId| -> bool {
                match surface {
                    Surface::Plane(frame) => self
                        .loop_vector_area(l)
                        .is_ok_and(|a| a.dot(frame.z()) * orient > LINEAR_TOLERANCE.powi(2)),
                    _ => self
                        .loop_image(l, &surface, orient)
                        .is_ok_and(|img| img.closed && img.signed_area() * orient > 0.0),
                }
            };
            let Some(outer) = loops.iter().copied().find(|l| runs_ccw(*l)) else {
                continue;
            };
            let inner: Vec<LoopId> = loops.iter().copied().filter(|l| *l != outer).collect();
            if let Some(face) = self.faces.get_mut(fi) {
                face.outer = outer;
                face.inner = inner;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Circle3, Segment3};
    use chamfera_geom::{Frame, Vec3};
    use std::f64::consts::TAU;

    fn seg(points: &[Point3], a: usize, b: usize) -> InEdge {
        (
            a,
            b,
            Curve::Segment(Segment3::new(points[a], points[b]).unwrap()),
            0.0,
            1.0,
        )
    }

    /// The unit cube: vertices `i = x + 2y + 4z`, the 12 edges along the axes, and each
    /// face's loop counter-clockwise about its outward normal (written out by hand).
    fn cube() -> (Vec<Point3>, Vec<InEdge>, Vec<InFace>) {
        let points: Vec<Point3> = (0..8)
            .map(|i| Point3::new(f64::from(i & 1), f64::from((i >> 1) & 1), f64::from(i >> 2)))
            .collect();
        let pairs = [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7), // along x: edges 0-3
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7), // along y: edges 4-7
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7), // along z: edges 8-11
        ];
        let edges: Vec<InEdge> = pairs.iter().map(|&(a, b)| seg(&points, a, b)).collect();
        let plane = |o: Point3, n: Vec3| Surface::Plane(Frame::from_normal(o, n).unwrap());
        let o = Point3::ORIGIN;
        let one = Point3::new(1.0, 1.0, 1.0);
        let faces = vec![
            // bottom z=0, normal -z: 0 -> 2 -> 3 -> 1
            (
                plane(o, -Vec3::Z),
                true,
                vec![vec![(4, true), (1, true), (5, false), (0, false)]],
            ),
            // top z=1, normal +z: 4 -> 5 -> 7 -> 6
            (
                plane(one, Vec3::Z),
                true,
                vec![vec![(2, true), (7, true), (3, false), (6, false)]],
            ),
            // front y=0, normal -y: 0 -> 1 -> 5 -> 4
            (
                plane(o, -Vec3::Y),
                true,
                vec![vec![(0, true), (9, true), (2, false), (8, false)]],
            ),
            // back y=1, normal +y: 2 -> 6 -> 7 -> 3
            (
                plane(one, Vec3::Y),
                true,
                vec![vec![(10, true), (3, true), (11, false), (1, false)]],
            ),
            // left x=0, normal -x: 0 -> 4 -> 6 -> 2
            (
                plane(o, -Vec3::X),
                true,
                vec![vec![(8, true), (6, true), (10, false), (4, false)]],
            ),
            // right x=1, normal +x: 1 -> 3 -> 7 -> 5
            (
                plane(one, Vec3::X),
                true,
                vec![vec![(5, true), (11, true), (7, false), (9, false)]],
            ),
        ];
        (points, edges, faces)
    }

    #[test]
    fn a_cube_from_its_boundary_is_a_valid_unit_solid() {
        let (p, e, f) = cube();
        let body = Body::from_boundary(&p, &e, &f).unwrap();
        let s = body.summary();
        assert_eq!((s.vertices, s.edges, s.faces, s.loops), (8, 12, 6, 6));
        assert!((body.volume().unwrap() - 1.0).abs() < 1e-12);
        assert!((body.area().unwrap() - 6.0).abs() < 1e-12);
        // Ids follow the description.
        assert_eq!(body.vertex_point(VertexId::from_index(7)).unwrap(), p[7]);
        assert_eq!(
            body.edge_vertices(EdgeId::from_index(11)).unwrap(),
            [VertexId::from_index(3), VertexId::from_index(7)]
        );
    }

    #[test]
    fn broken_descriptions_name_the_entity() {
        let (p, e, f) = cube();
        // Edge 0 used twice forward: face 0 runs it backward in the cube; flip that use.
        let mut g = f.clone();
        g[0].2[0][3].1 = true;
        assert!(matches!(
            Body::from_boundary(&p, &e, &g),
            Err(TopologyError::OpenLoop(_))
        ));
        // A missing face leaves its edges used once.
        let h: Vec<InFace> = f[..5].to_vec();
        assert!(matches!(
            Body::from_boundary(&p, &e, &h),
            Err(TopologyError::MissingTwin(_))
        ));
        // A face turned inside out: its loop reversed, against its neighbours.
        let mut k = f.clone();
        k[1].2[0] = k[1].2[0].iter().rev().map(|&(e, fw)| (e, !fw)).collect();
        assert!(Body::from_boundary(&p, &e, &k).is_err());
        // An edge index past the end.
        let mut d = f.clone();
        d[2].2[0][0].0 = 99;
        assert!(matches!(
            Body::from_boundary(&p, &e, &d),
            Err(TopologyError::Dangling { .. })
        ));
        // Empty input.
        assert!(Body::from_boundary(&[], &[], &[]).is_err());
        let mut v = f;
        v[3].2.push(Vec::new());
        assert!(matches!(
            Body::from_boundary(&p, &e, &v),
            Err(TopologyError::VertexLoop(f)) if f.index() == 3
        ));
    }

    #[test]
    fn the_outer_loop_is_found_when_it_comes_second() {
        // A 4 x 4 x 1 block with a round 1 mm hole through it: the top face lists the
        // hole's loop first. Expected volume 16 - π, by hand.
        let mut points = Vec::new();
        for z in [0.0, 1.0] {
            for (x, y) in [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)] {
                points.push(Point3::new(x, y, z));
            }
        }
        // Hole seam points at (3, 2, z).
        points.push(Point3::new(3.0, 2.0, 0.0)); // 8
        points.push(Point3::new(3.0, 2.0, 1.0)); // 9
        let mut edges: Vec<InEdge> = Vec::new();
        for i in 0..4 {
            edges.push(seg(&points, i, (i + 1) % 4)); // bottom 0-3
        }
        for i in 0..4 {
            edges.push(seg(&points, 4 + i, 4 + (i + 1) % 4)); // top 4-7
        }
        for i in 0..4 {
            edges.push(seg(&points, i, 4 + i)); // vertical 8-11
        }
        let circle = |z: f64| {
            let c = Circle3::new(
                Frame::XY.with_origin(Point3::new(2.0, 2.0, z)).unwrap(),
                1.0,
            );
            Curve::Circle(c.unwrap())
        };
        edges.push((8, 8, circle(0.0), 0.0, TAU)); // 12
        edges.push((9, 9, circle(1.0), 0.0, TAU)); // 13
        edges.push(seg(&points, 8, 9)); // 14, the hole's seam
        let plane = |z: f64, n: Vec3| {
            Surface::Plane(Frame::from_normal(Point3::new(0.0, 0.0, z), n).unwrap())
        };
        let side = |a: usize, n: Vec3| Surface::Plane(Frame::from_normal(points[a], n).unwrap());
        let hole = Surface::cylinder(
            Frame::XY.with_origin(Point3::new(2.0, 2.0, 0.0)).unwrap(),
            1.0,
        )
        .unwrap();
        let faces: Vec<InFace> = vec![
            // bottom, normal -z: outer 0 -> 3 -> 2 -> 1, hole counter-clockwise about +z.
            (
                plane(0.0, -Vec3::Z),
                true,
                vec![
                    vec![(3, false), (2, false), (1, false), (0, false)],
                    vec![(12, true)],
                ],
            ),
            // top, normal +z: hole loop first (clockwise about +z), then the outer loop.
            (
                plane(1.0, Vec3::Z),
                true,
                vec![
                    vec![(13, false)],
                    vec![(4, true), (5, true), (6, true), (7, true)],
                ],
            ),
            (
                side(0, -Vec3::Y),
                true,
                vec![vec![(0, true), (9, true), (4, false), (8, false)]],
            ),
            (
                side(1, Vec3::X),
                true,
                vec![vec![(1, true), (10, true), (5, false), (9, false)]],
            ),
            (
                side(2, Vec3::Y),
                true,
                vec![vec![(2, true), (11, true), (6, false), (10, false)]],
            ),
            (
                side(3, -Vec3::X),
                true,
                vec![vec![(3, true), (8, true), (7, false), (11, false)]],
            ),
            // The hole's wall faces the axis: outward normal is -(S_u x S_v).
            (
                hole,
                false,
                vec![vec![(12, false), (14, true), (13, true), (14, false)]],
            ),
        ];
        let body = Body::from_boundary(&points, &edges, &faces).unwrap();
        let top = FaceId::from_index(1);
        assert_eq!(body.face_outer_loop(top).unwrap(), LoopId::from_index(3));
        let v = body.volume().unwrap();
        assert!((v - (16.0 - std::f64::consts::PI)).abs() < 1e-9, "{v}");
    }
}

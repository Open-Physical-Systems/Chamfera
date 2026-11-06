//! An editable copy of a body's boundary: faces with their surfaces and loops, edges with
//! their old curves, vertices with their old points. A face edit changes surfaces (offset,
//! move) or the topology (delete), then [`Draft::build`] works the geometry out again from
//! the surfaces alone and assembles a validated body.
//!
//! # Rebuilding the geometry
//! Every vertex lies on the surfaces of the faces around it, so a vertex whose faces
//! changed is moved by Gauss-Newton on their implicit equations `f_i(x) = 0` (one per
//! surface, signed distances along the normal), from its old point, with the least-norm
//! step where the faces fix fewer than three directions (a seam, a closed circle): the
//! method of the shell's offset body (`shell/offset.rs`, after Forsyth 1995), here with
//! spheres and tori too. The implicit equations, with `ρ` the distance from the axis and
//! `z` the height in the surface's frame:
//!
//! | Surface | `f(x)` | `∇f` |
//! |---|---|---|
//! | plane | `z` | frame z |
//! | cylinder, radius `r` | `ρ - r` | `e(u)` |
//! | cone, radius `r`, half-angle `α` | `(ρ - r) cos α - z sin α` | `cos α e(u) - sin α z` |
//! | sphere, radius `r` | `‖x - o‖ - r` | `(x - o) / ‖x - o‖` |
//! | torus, `R`, `a` | `‖(ρ - R, z)‖ - a` | `((ρ - R) e(u) + z ẑ) / ‖(ρ - R, z)‖` |
//!
//! Each is the signed distance along the surface normal of `surface.rs` (do Carmo,
//! *Differential Geometry of Curves and Surfaces*, section 2-3, for the meridians).
//!
//! An edge whose vertices or faces changed gets a new curve on both new surfaces: first
//! the straight segment or the circle about the old circle's axis through its new end
//! points (what an offset or a move of a plane gives), else the K6 intersection curve of
//! its two surfaces ([`intersect_surfaces`]: lines, circles, ellipses) through both ends,
//! run the way the old edge ran. Every candidate is checked against both surfaces along
//! its length; an edge with no exact curve is refused.

use crate::shell::offset::solve3;
use crate::topology::{Body, Curve, ListEdge, ListFace, ListVertex};
use crate::{Circle3, KernelError, KernelResult, Segment3, Surface, intersect_surfaces};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point3, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

/// Most Gauss-Newton steps for one vertex (the equations are linear for planes and
/// nearly so for the rest).
const MAX_VERTEX_STEPS: usize = 48;
/// Damping added to the normal equations so the least-norm step is taken where the faces
/// fix fewer than three directions; far below the entries (unit gradients give order 1).
const DAMPING: f64 = 1e-12;
/// Points along each new edge checked against both of its surfaces.
const EDGE_SAMPLES: usize = 8;

#[derive(Debug, Clone)]
pub(super) struct DraftVertex {
    /// The old point, the start of the vertex's solve.
    pub point: Point3,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct DraftEdge {
    pub vertices: [usize; 2],
    pub curve: Curve,
    pub t0: f64,
    pub t1: f64,
    pub name: Option<String>,
    /// The edge must get a new curve even if its faces and vertices look unchanged.
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub(super) struct DraftFace {
    pub surface: Surface,
    pub same_sense: bool,
    pub name: Option<String>,
    /// Loops, outer first, as `(edge index, runs along the edge)` uses.
    pub loops: Vec<Vec<(usize, bool)>>,
    /// The surface changed: its vertices and edges are worked out again.
    pub changed: bool,
}

/// A body's boundary being edited. Removed edges and faces are `None`; merged vertices
/// share a root in `parent`.
#[derive(Debug, Clone)]
pub(super) struct Draft {
    pub vertices: Vec<DraftVertex>,
    pub parent: Vec<usize>,
    pub edges: Vec<Option<DraftEdge>>,
    pub faces: Vec<Option<DraftFace>>,
    /// Vertices that must be solved again (merged ones).
    pub dirty_vertices: BTreeSet<usize>,
}

impl Draft {
    /// The editable copy of `body`, with the body's face, edge and vertex order.
    pub fn from_body(body: &Body) -> KernelResult<Draft> {
        let vertex_ids = body.vertices();
        let vertex_index: BTreeMap<_, _> = vertex_ids
            .iter()
            .enumerate()
            .map(|(i, v)| (*v, i))
            .collect();
        let mut vertices = Vec::with_capacity(vertex_ids.len());
        for v in &vertex_ids {
            vertices.push(DraftVertex {
                point: body.vertex_point(*v)?,
                name: body.vertex_name(*v)?.map(str::to_owned),
            });
        }
        let edge_ids = body.edges();
        let edge_index: BTreeMap<_, _> =
            edge_ids.iter().enumerate().map(|(i, e)| (*e, i)).collect();
        let mut edges = Vec::with_capacity(edge_ids.len());
        for e in &edge_ids {
            let [a, b] = body.edge_vertices(*e)?;
            let (curve, t0, t1) = body
                .edge_curve(*e)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve")))?;
            let index = |v| {
                vertex_index
                    .get(&v)
                    .copied()
                    .ok_or_else(|| KernelError::InvalidInput(format!("{v} is not listed")))
            };
            edges.push(Some(DraftEdge {
                vertices: [index(a)?, index(b)?],
                curve,
                t0,
                t1,
                name: body.edge_name(*e)?.map(str::to_owned),
                dirty: false,
            }));
        }
        let mut faces = Vec::new();
        for f in body.faces() {
            let (surface, same_sense) = body
                .face_surface(f)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{f} has no surface")))?;
            let mut loops = Vec::new();
            for lp in body.face_loops(f)? {
                let mut uses = Vec::new();
                for h in body.loop_half_edges(lp)? {
                    let e = body.half_edge_edge(h)?.ok_or_else(|| {
                        KernelError::Unsupported(
                            "editing faces of a body with a one-vertex loop".into(),
                        )
                    })?;
                    let i = *edge_index
                        .get(&e)
                        .ok_or_else(|| KernelError::InvalidInput(format!("{e} is not listed")))?;
                    uses.push((i, body.edge_half_edges(e)?[0] == h));
                }
                loops.push(uses);
            }
            faces.push(Some(DraftFace {
                surface,
                same_sense,
                name: body.face_name(f)?.map(str::to_owned),
                loops,
                changed: false,
            }));
        }
        let n = vertices.len();
        Ok(Draft {
            vertices,
            parent: (0..n).collect(),
            edges,
            faces,
            dirty_vertices: BTreeSet::new(),
        })
    }

    /// The root of a vertex's merge set.
    pub fn root(&self, mut v: usize) -> usize {
        for _ in 0..self.parent.len() {
            match self.parent.get(v) {
                Some(&p) if p != v => v = p,
                _ => break,
            }
        }
        v
    }

    /// Merges two vertices into one, to be solved again.
    pub fn merge(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb
            && let Some(p) = self.parent.get_mut(rb)
        {
            *p = ra;
        }
        self.dirty_vertices.insert(ra);
    }

    /// The faces using each live edge, one entry per use.
    pub fn edge_uses(&self) -> Vec<Vec<usize>> {
        let mut uses = vec![Vec::new(); self.edges.len()];
        for (fi, f) in self.faces.iter().enumerate() {
            let Some(f) = f else { continue };
            for lp in &f.loops {
                for &(e, _) in lp {
                    if let Some(u) = uses.get_mut(e) {
                        u.push(fi);
                    }
                }
            }
        }
        uses
    }

    /// The faces around face `f` across each of its edges: `(edge, other face)` per use.
    pub fn neighbours(&self, f: usize) -> Vec<(usize, usize)> {
        let uses = self.edge_uses();
        let Some(Some(face)) = self.faces.get(f) else {
            return Vec::new();
        };
        face.loops
            .iter()
            .flatten()
            .map(|&(e, _)| {
                let other = uses
                    .get(e)
                    .and_then(|u| u.iter().copied().find(|g| *g != f))
                    .unwrap_or(f);
                (e, other)
            })
            .collect()
    }

    /// The validated body this draft describes, every changed vertex and edge worked out
    /// again from the surfaces.
    pub fn build(&self) -> KernelResult<Body> {
        let uses = self.edge_uses();
        // The faces around each vertex root, and whether any of them changed.
        let mut around: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (ei, e) in self.edges.iter().enumerate() {
            let Some(e) = e else { continue };
            let faces = uses.get(ei).cloned().unwrap_or_default();
            if faces.len() != 2 {
                return Err(KernelError::InvalidResult(format!(
                    "edge {ei} is used by {} faces after the edit, not 2",
                    faces.len()
                )));
            }
            for v in e.vertices {
                around.entry(self.root(v)).or_default().extend(&faces);
            }
        }
        let face = |i: usize| -> KernelResult<&DraftFace> {
            self.faces
                .get(i)
                .and_then(Option::as_ref)
                .ok_or_else(|| KernelError::InvalidResult(format!("face {i} is gone")))
        };
        let mut points: BTreeMap<usize, Point3> = BTreeMap::new();
        let mut moved: BTreeSet<usize> = BTreeSet::new();
        for (&v, faces) in &around {
            let dirty = self.dirty_vertices.contains(&v)
                || faces.iter().any(|f| face(*f).is_ok_and(|f| f.changed));
            let guess = self.guess(v);
            let name = self
                .vertices
                .get(v)
                .and_then(|x| x.name.clone())
                .unwrap_or_else(|| format!("vertex {v}"));
            let p = if dirty {
                moved.insert(v);
                let surfaces = faces
                    .iter()
                    .map(|f| face(*f).map(|f| f.surface))
                    .collect::<KernelResult<Vec<_>>>()?;
                solve_vertex(guess, &surfaces, &name)?
            } else {
                guess
            };
            points.insert(v, p);
        }
        // Compact: live vertex roots, live edges, live faces.
        let vertex_index: BTreeMap<usize, usize> =
            points.keys().enumerate().map(|(i, v)| (*v, i)).collect();
        let list_vertices: Vec<ListVertex> = points
            .iter()
            .map(|(v, p)| ListVertex {
                point: *p,
                name: self.vertices.get(*v).and_then(|x| x.name.clone()),
            })
            .collect();
        let mut edge_index: BTreeMap<usize, usize> = BTreeMap::new();
        let mut list_edges = Vec::new();
        for (ei, e) in self.edges.iter().enumerate() {
            let Some(e) = e else { continue };
            let [a, b] = [self.root(e.vertices[0]), self.root(e.vertices[1])];
            let (Some(&pa), Some(&pb)) = (points.get(&a), points.get(&b)) else {
                return Err(KernelError::InvalidResult(format!(
                    "edge {ei} lost a vertex"
                )));
            };
            let faces = uses.get(ei).cloned().unwrap_or_default();
            let surfaces = faces
                .iter()
                .map(|f| face(*f).map(|f| (f.surface, f.changed)))
                .collect::<KernelResult<Vec<_>>>()?;
            let dirty =
                e.dirty || moved.contains(&a) || moved.contains(&b) || surfaces.iter().any(|s| s.1);
            let name = e.name.clone().unwrap_or_else(|| format!("edge {ei}"));
            let (curve, t0, t1) = if dirty {
                let s: Vec<Surface> = surfaces.iter().map(|s| s.0).collect();
                new_edge_curve(e, a == b, [pa, pb], &s, &name)?
            } else {
                (e.curve, e.t0, e.t1)
            };
            let (Some(&ia), Some(&ib)) = (vertex_index.get(&a), vertex_index.get(&b)) else {
                return Err(KernelError::InvalidResult(format!(
                    "edge {ei} lost a vertex"
                )));
            };
            edge_index.insert(ei, list_edges.len());
            list_edges.push(ListEdge {
                vertices: [ia, ib],
                curve,
                t0,
                t1,
                name: e.name.clone(),
            });
        }
        let mut list_faces = Vec::new();
        for f in self.faces.iter().flatten() {
            let loops = f
                .loops
                .iter()
                .map(|lp| {
                    lp.iter()
                        .map(|(e, fwd)| {
                            edge_index.get(e).map(|i| (*i, *fwd)).ok_or_else(|| {
                                KernelError::InvalidResult(format!(
                                    "a loop uses a removed edge {e}"
                                ))
                            })
                        })
                        .collect::<KernelResult<Vec<_>>>()
                })
                .collect::<KernelResult<Vec<_>>>()?;
            list_faces.push(ListFace {
                surface: f.surface,
                same_sense: f.same_sense,
                name: f.name.clone(),
                loops,
            });
        }
        let out = Body::from_face_list(&list_vertices, &list_edges, &list_faces)?;
        out.validate()
            .map_err(|e| KernelError::InvalidResult(format!("the edited faces: {e}")))?;
        Ok(out)
    }

    /// The start of a vertex root's solve: the mean of its merged vertices' old points.
    fn guess(&self, root: usize) -> Point3 {
        let mut sum = Vec3::new(0.0, 0.0, 0.0);
        let mut n = 0.0;
        let mut first = None;
        for (i, v) in self.vertices.iter().enumerate() {
            if self.root(i) == root {
                let p = v.point;
                first.get_or_insert(p);
                sum = sum + Vec3::new(p.x, p.y, p.z);
                n += 1.0;
            }
        }
        match first {
            Some(_) if n > 0.0 => Point3::new(sum.x / n, sum.y / n, sum.z / n),
            Some(p) => p,
            None => Point3::new(0.0, 0.0, 0.0),
        }
    }
}

/// The implicit equation of `s` at `x`: the signed distance along the surface normal and
/// its gradient (see the module documentation).
pub(super) fn implicit(s: &Surface, x: Point3) -> KernelResult<(f64, Vec3)> {
    let f = s.frame();
    let l = f.to_local(x);
    let rho = l.x.hypot(l.y);
    let radial = || {
        if rho <= LINEAR_TOLERANCE {
            return Err(KernelError::Unsupported(
                "editing a face with a vertex on its surface's axis (a pole or an apex)".into(),
            ));
        }
        Ok(f.vector_to_world(Vec3::new(l.x / rho, l.y / rho, 0.0)))
    };
    match s {
        Surface::Plane(_) => Ok((l.z, f.z())),
        Surface::Cylinder(c) => Ok((rho - c.radius(), radial()?)),
        Surface::Cone(c) => {
            let (sn, cs) = c.half_angle().sin_cos();
            Ok((
                (rho - c.radius()) * cs - l.z * sn,
                radial()? * cs - f.z() * sn,
            ))
        }
        Surface::Sphere(sp) => {
            let v = x - f.origin();
            let d = v.length();
            if d <= LINEAR_TOLERANCE {
                return Err(KernelError::Unsupported(
                    "editing a face with a vertex at a sphere's centre".into(),
                ));
            }
            Ok((d - sp.radius(), v * (1.0 / d)))
        }
        Surface::Torus(t) => {
            if t.major_radius() <= LINEAR_TOLERANCE {
                return Err(KernelError::Unsupported(
                    "editing faces on a lemon-shaped torus".into(),
                ));
            }
            let e = radial()?;
            let (dr, dz) = (rho - t.major_radius(), l.z);
            let q = dr.hypot(dz);
            if q <= LINEAR_TOLERANCE {
                return Err(KernelError::Unsupported(
                    "editing a face with a vertex on a torus's centre circle".into(),
                ));
            }
            Ok((q - t.minor_radius(), (e * dr + f.z() * dz) * (1.0 / q)))
        }
    }
}

/// The point on every surface of `surfaces` nearest (least-norm) to `start`.
fn solve_vertex(start: Point3, surfaces: &[Surface], name: &str) -> KernelResult<Point3> {
    let mut x = start;
    for _ in 0..MAX_VERTEX_STEPS {
        let mut m = [[0.0f64; 3]; 3];
        let mut rhs = [0.0f64; 3];
        let mut worst = 0.0f64;
        for s in surfaces {
            let (d, g) = implicit(s, x)?;
            worst = worst.max(d.abs());
            let g = [g.x, g.y, g.z];
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += g[i] * g[j];
                }
                rhs[i] -= g[i] * d;
            }
        }
        if worst <= LINEAR_TOLERANCE * 1e-3 {
            return Ok(x);
        }
        for (i, row) in m.iter_mut().enumerate() {
            row[i] += DAMPING;
        }
        let step = solve3(m, rhs).ok_or_else(|| {
            KernelError::NoConvergence(format!("moving the vertex {name} onto its faces"))
        })?;
        x = x + Vec3::new(step[0], step[1], step[2]);
        if !x.is_valid() {
            return Err(KernelError::InvalidInput(format!(
                "the vertex {name} would leave the working range"
            )));
        }
    }
    let worst = surfaces
        .iter()
        .map(|s| implicit(s, x).map(|(d, _)| d.abs()))
        .collect::<KernelResult<Vec<_>>>()?
        .into_iter()
        .fold(0.0, f64::max);
    if worst <= LINEAR_TOLERANCE * 0.1 {
        return Ok(x);
    }
    Err(KernelError::Unsupported(format!(
        "the faces around the vertex {name} no longer meet in one point"
    )))
}

/// True when the curve over `[t0, t1]` lies on every surface of `surfaces`.
fn on_surfaces(c: &Curve, t0: f64, t1: f64, surfaces: &[Surface]) -> bool {
    (0..=EDGE_SAMPLES).all(|i| {
        let t = t0 + (t1 - t0) * (i as f64 / EDGE_SAMPLES as f64);
        let p = c.point_at(t);
        surfaces.iter().all(|s| s.distance(p) <= LINEAR_TOLERANCE)
    })
}

/// `a` moved by whole turns into `(-π, π]`.
fn wrap_pi(a: f64) -> f64 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

/// The new curve of edge `e` from `ends[0]` to `ends[1]` on `surfaces` (its two faces').
fn new_edge_curve(
    e: &DraftEdge,
    closed: bool,
    ends: [Point3; 2],
    surfaces: &[Surface],
    name: &str,
) -> KernelResult<(Curve, f64, f64)> {
    let old_dir = e.curve.d1(e.t0);
    if !closed && ends[0].coincident(ends[1]) {
        return Err(KernelError::InvalidInput(format!(
            "the edge {name} would shrink to a point"
        )));
    }
    if let Some(c) = straight_or_coaxial(e, closed, ends)
        && on_surfaces(&c.0, c.1, c.2, surfaces)
    {
        return Ok(c);
    }
    if let [a, b] = surfaces
        && let Some(c) = from_intersection(a, b, closed, ends, old_dir)?
        && on_surfaces(&c.0, c.1, c.2, surfaces)
    {
        return Ok(c);
    }
    Err(KernelError::Unsupported(format!(
        "the edge {name} would need a curve other than a line, circle or ellipse, or its faces no longer meet"
    )))
}

/// The segment between the new ends, or the circle about the old circle's axis through
/// them, run the way the old edge ran.
fn straight_or_coaxial(
    e: &DraftEdge,
    closed: bool,
    ends: [Point3; 2],
) -> Option<(Curve, f64, f64)> {
    let old = [e.curve.point_at(e.t0), e.curve.point_at(e.t1)];
    match e.curve {
        Curve::Line(_) | Curve::Segment(_) => {
            if closed || (ends[1] - ends[0]).dot(old[1] - old[0]) <= 0.0 {
                return None;
            }
            let s = Segment3::new(ends[0], ends[1]).ok()?;
            Some((Curve::Segment(s), 0.0, 1.0))
        }
        Curve::Circle(_) | Curve::Arc(_) => {
            let circle = match e.curve {
                Curve::Circle(c) => c,
                Curve::Arc(a) => *a.circle(),
                _ => return None,
            };
            let frame = circle.frame();
            let z = frame.z();
            let height = |p: Point3| (p - frame.origin()).dot(z);
            let h = height(ends[0]);
            let centre = frame.origin() + z * h;
            let f = frame.with_origin(centre)?;
            let radial = |p: Point3| {
                let v = p - centre;
                (v - z * v.dot(z)).length()
            };
            let r = radial(ends[0]);
            if r <= LINEAR_TOLERANCE
                || (height(ends[1]) - h).abs() > LINEAR_TOLERANCE
                || (radial(ends[1]) - r).abs() > LINEAR_TOLERANCE
            {
                return None;
            }
            let angle = |p: Point3| {
                let v = p - centre;
                v.dot(f.y()).atan2(v.dot(f.x()))
            };
            let n0 = e.t0 + wrap_pi(angle(ends[0]) - e.t0);
            let n1 = if closed {
                n0 + (e.t1 - e.t0)
            } else {
                e.t1 + wrap_pi(angle(ends[1]) - e.t1)
            };
            if n1 - n0 <= ANGULAR_TOLERANCE.max(LINEAR_TOLERANCE / r) {
                return None;
            }
            let c = Circle3::new(f, r).ok()?;
            Some((Curve::Circle(c), n0, n1))
        }
        Curve::Ellipse(_) | Curve::EllipticalArc(_) => None,
    }
}

/// The K6 intersection curve of `a` and `b` through both ends, run along `old_dir` at the
/// start; `None` when no curve of the intersection passes through both.
fn from_intersection(
    a: &Surface,
    b: &Surface,
    closed: bool,
    ends: [Point3; 2],
    old_dir: Vec3,
) -> KernelResult<Option<(Curve, f64, f64)>> {
    let x = intersect_surfaces(a, b)?;
    for ic in x.curves() {
        let found = match ic.curve {
            Curve::Line(l) => {
                let on = |p: Point3| {
                    l.closest_point(p)
                        .is_ok_and(|c| c.distance <= LINEAR_TOLERANCE)
                };
                if closed || !on(ends[0]) || !on(ends[1]) || (ends[1] - ends[0]).dot(old_dir) <= 0.0
                {
                    None
                } else {
                    Segment3::new(ends[0], ends[1])
                        .ok()
                        .map(|s| (Curve::Segment(s), 0.0, 1.0))
                }
            }
            Curve::Circle(c) => {
                let c = if c.d1(param_on(&Curve::Circle(c), ends[0])).dot(old_dir) < 0.0 {
                    c.reversed()
                } else {
                    c
                };
                periodic_range(Curve::Circle(c), closed, ends)
            }
            Curve::Ellipse(el) => {
                let el = if el.d1(param_on(&Curve::Ellipse(el), ends[0])).dot(old_dir) < 0.0 {
                    el.reversed()
                } else {
                    el
                };
                periodic_range(Curve::Ellipse(el), closed, ends)
            }
            _ => None,
        };
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

/// The parameter of the point of a circle or ellipse nearest to `p`.
fn param_on(c: &Curve, p: Point3) -> f64 {
    match c {
        Curve::Circle(c) => c.closest_point(p).map(|q| q.t).unwrap_or(0.0),
        Curve::Ellipse(e) => e.closest_point(p).map(|q| q.t).unwrap_or(0.0),
        _ => 0.0,
    }
}

/// The range from `ends[0]` forward to `ends[1]` on a closed curve (a full turn when the
/// edge is closed), when both ends lie on it.
fn periodic_range(c: Curve, closed: bool, ends: [Point3; 2]) -> Option<(Curve, f64, f64)> {
    let t0 = param_on(&c, ends[0]);
    let t1 = param_on(&c, ends[1]);
    if !(c.point_at(t0).coincident(ends[0]) && c.point_at(t1).coincident(ends[1])) {
        return None;
    }
    let t1 = if closed {
        t0 + TAU
    } else {
        t0 + (t1 - t0).rem_euclid(TAU)
    };
    (t1 - t0 > ANGULAR_TOLERANCE).then_some((c, t0, t1))
}

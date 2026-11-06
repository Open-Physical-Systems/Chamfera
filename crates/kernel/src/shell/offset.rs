//! The offset body of a shell: the solid with every closed face moved toward the inside
//! (or outside) by the thickness, same topology, new geometry.
//!
//! Signed distances along a surface's normal (positive on the side the normal points to),
//! with `ρ` the distance from the axis and `z` the height in the surface's frame:
//!
//! | Surface | signed distance `d(x)` | its gradient | offset by `δ` |
//! |---|---|---|---|
//! | plane | `(x - o) · z` | `z` | origin moved by `δ z` |
//! | cylinder, radius `r` | `ρ - r` | `e(u)` | radius `r + δ` |
//! | cone, radius `r`, half-angle `α` | `(ρ - r) cos α - z sin α` | `cos α e(u) - sin α z` | radius `r + δ / cos α` at `z = 0` |
//!
//! The cone row: the generator through `(r, 0)` has direction `(sin α, cos α)` in the
//! `(ρ, z)` half-plane and normal `(cos α, -sin α)` (the surface normal of `surface.rs`),
//! so the distance is the dot product of `(ρ - r, z)` with that normal, and the offset
//! generator `d = δ` crosses `z = 0` at `ρ = r + δ / cos α`.

use super::bounded;
use crate::curve3::{Arc3, Circle3, Segment3};
use crate::topology::{Body, Curve, EdgeId, FaceId, ListEdge, ListFace, ListVertex, VertexId};
use crate::{KernelError, KernelResult, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point3, Vec3};
use std::collections::{BTreeSet, HashMap};
use std::f64::consts::{PI, TAU};

/// Most Gauss-Newton steps for one vertex; the equations are linear for planes and
/// nearly so for the rest, so a handful suffice.
const MAX_VERTEX_STEPS: usize = 32;
/// Damping added to the normal equations so a vertex whose faces fix fewer than three
/// directions (a seam, a tangent face) takes the least-norm step. Far below any entry of
/// the matrix (unit normals give entries of order 1).
const DAMPING: f64 = 1e-12;
/// Points along each new edge checked against both offset surfaces.
const EDGE_SAMPLES: usize = 8;

/// One face's offset: the surface it moves to and the signed distance `δ` along the
/// original surface's normal.
struct Moved {
    original: Surface,
    delta: f64,
}

/// The signed distance of `x` from `s` along its normal and the unit gradient there.
fn signed_distance(s: &Surface, x: Point3) -> KernelResult<(f64, Vec3)> {
    let f = s.frame();
    let l = f.to_local(x);
    let rho = l.x.hypot(l.y);
    let radial = || {
        if rho <= LINEAR_TOLERANCE {
            return Err(KernelError::Unsupported(
                "shelling a face with a vertex on its surface's axis".into(),
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
        Surface::Sphere(_) | Surface::Torus(_) | Surface::Extrusion(_) => {
            Err(unsupported_surface())
        }
    }
}

fn unsupported_surface() -> KernelError {
    KernelError::Unsupported(
        "shelling faces on spheres, tori or extruded splines: planes, cylinders and cones only for now".into(),
    )
}

fn vanishes(what: &str) -> KernelError {
    KernelError::InvalidInput(format!(
        "the wall thickness is too large for this shape: {what}"
    ))
}

/// `s` offset by `delta` along its normal.
fn offset_surface(s: &Surface, delta: f64, name: &str) -> KernelResult<Surface> {
    match s {
        Surface::Plane(f) => f
            .with_origin(f.origin() + f.z() * delta)
            .map(Surface::Plane)
            .ok_or_else(|| vanishes(&format!("the offset of {name} leaves the working range"))),
        Surface::Cylinder(c) => {
            let r = c.radius() + delta;
            if r <= LINEAR_TOLERANCE {
                return Err(vanishes(&format!("the cylinder {name} would vanish")));
            }
            Surface::cylinder(*c.frame(), r)
        }
        Surface::Cone(c) => {
            let a = c.half_angle();
            let r = c.radius() + delta / a.cos();
            if r > LINEAR_TOLERANCE {
                return Surface::cone(*c.frame(), r, a);
            }
            // The offset generator crosses the frame's plane at a negative radius: put the
            // frame where the offset nappe has radius `target` instead (`ρ = r + z tan α`).
            let target = 1.0 + r.abs();
            let z0 = (target - r) / a.tan();
            let f = c.frame();
            let frame = f.with_origin(f.origin() + f.z() * z0).ok_or_else(|| {
                vanishes(&format!("the offset of {name} leaves the working range"))
            })?;
            Surface::cone(frame, target, a)
        }
        Surface::Sphere(_) | Surface::Torus(_) | Surface::Extrusion(_) => {
            Err(unsupported_surface())
        }
    }
}

/// Solves a 3x3 symmetric positive definite system by Cholesky factorisation.
fn solve3(m: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let mut l = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..=i {
            let sum = m[i][j] - (0..j).map(|k| l[i][k] * l[j][k]).sum::<f64>();
            if i == j {
                if sum.is_nan() || sum <= 0.0 {
                    return None;
                }
                l[i][i] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    let mut y = [0.0f64; 3];
    for i in 0..3 {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[i][k] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    let mut x = [0.0f64; 3];
    for i in (0..3).rev() {
        let mut sum = y[i];
        for k in i + 1..3 {
            sum -= l[k][i] * x[k];
        }
        x[i] = sum / l[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// The point on every offset surface of `faces` nearest (in the least-norm sense) to `p`.
fn move_vertex(p: Point3, faces: &[&Moved], name: &str) -> KernelResult<Point3> {
    let mut x = p;
    for _ in 0..MAX_VERTEX_STEPS {
        let mut m = [[0.0f64; 3]; 3];
        let mut rhs = [0.0f64; 3];
        let mut worst = 0.0f64;
        for f in faces {
            let (d, g) = signed_distance(&f.original, x)?;
            let r = d - f.delta;
            worst = worst.max(r.abs());
            let g = [g.x, g.y, g.z];
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += g[i] * g[j];
                }
                rhs[i] -= g[i] * r;
            }
        }
        if worst <= LINEAR_TOLERANCE * 1e-3 {
            return Ok(x);
        }
        for (i, row) in m.iter_mut().enumerate() {
            row[i] += DAMPING;
        }
        let step = solve3(m, rhs).ok_or_else(|| {
            KernelError::NoConvergence(format!("moving the vertex {name} onto its offset faces"))
        })?;
        x = x + Vec3::new(step[0], step[1], step[2]);
        if !x.is_valid() {
            return Err(vanishes(&format!(
                "the vertex {name} would leave the working range"
            )));
        }
    }
    let worst = faces
        .iter()
        .map(|f| signed_distance(&f.original, x).map(|(d, _)| (d - f.delta).abs()))
        .collect::<KernelResult<Vec<_>>>()?
        .into_iter()
        .fold(0.0, f64::max);
    if worst <= LINEAR_TOLERANCE * 0.1 {
        return Ok(x);
    }
    Err(KernelError::Unsupported(format!(
        "the offset faces at the vertex {name} do not meet in one point"
    )))
}

/// `a` moved by whole turns into `(-π, π]`.
fn wrap_pi(a: f64) -> f64 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

/// The new curve of an edge with old curve `c` over `[t0, t1]` from `old` to `new` end
/// points.
fn move_edge(
    c: &Curve,
    t0: f64,
    t1: f64,
    old: [Point3; 2],
    new: [Point3; 2],
    name: &str,
) -> KernelResult<(Curve, f64, f64)> {
    match c {
        Curve::Line(_) | Curve::Segment(_) => {
            let seg = Segment3::new(new[0], new[1])
                .map_err(|_| vanishes(&format!("the edge {name} would vanish")))?;
            if (new[1] - new[0]).dot(old[1] - old[0]) <= 0.0 || seg.length() <= LINEAR_TOLERANCE {
                return Err(vanishes(&format!("the edge {name} would turn over")));
            }
            Ok((Curve::Segment(seg), 0.0, 1.0))
        }
        Curve::Circle(_) | Curve::Arc(_) => {
            let circle = match c {
                Curve::Circle(c) => *c,
                Curve::Arc(a) => *a.circle(),
                _ => return Err(KernelError::Unsupported("edge curve".into())),
            };
            let frame = circle.frame();
            let z = frame.z();
            let height = |p: Point3| (p - frame.origin()).dot(z);
            let h = height(new[0]);
            let centre = frame.origin() + z * h;
            let f = frame
                .with_origin(centre)
                .ok_or_else(|| vanishes(&format!("the edge {name} leaves the working range")))?;
            let radial = |p: Point3| {
                let v = p - centre;
                (v - z * v.dot(z)).length()
            };
            let r = radial(new[0]);
            if r <= LINEAR_TOLERANCE {
                return Err(vanishes(&format!(
                    "the circle {name} would shrink to a point"
                )));
            }
            if (height(new[1]) - h).abs() > LINEAR_TOLERANCE
                || (radial(new[1]) - r).abs() > LINEAR_TOLERANCE
            {
                return Err(KernelError::Unsupported(format!(
                    "the offset of the circular edge {name} is not a circle about the same axis"
                )));
            }
            let angle = |p: Point3| {
                let v = p - centre;
                v.dot(f.y()).atan2(v.dot(f.x()))
            };
            let n0 = t0 + wrap_pi(angle(new[0]) - t0);
            let n1 = t1 + wrap_pi(angle(new[1]) - t1);
            // A closed circle (one vertex) keeps its full turn.
            let n1 = if old[0].coincident(old[1]) && (t1 - t0) > PI {
                n0 + (t1 - t0)
            } else {
                n1
            };
            if n1 - n0 <= ANGULAR_TOLERANCE.max(LINEAR_TOLERANCE / r) {
                return Err(vanishes(&format!(
                    "the arc {name} would vanish or turn over"
                )));
            }
            let curve = match c {
                Curve::Circle(_) => Curve::Circle(Circle3::new(f, r)?),
                _ => Curve::Arc(Arc3::new(f, r, n0, n1 - n0)?),
            };
            Ok((curve, n0, n1))
        }
        Curve::Ellipse(_) | Curve::EllipticalArc(_) => Err(KernelError::Unsupported(format!(
            "shelling a body with an elliptical edge ({name})"
        ))),
        Curve::BSpline(_) => Err(KernelError::Unsupported(format!(
            "shelling a body with a spline edge ({name})"
        ))),
    }
}

/// How the shell names an entity it made from one called `name`.
fn shell_name(name: &str) -> String {
    bounded(format!("shell of {name}"))
}

/// The body with every face not in `open` moved by `thickness` against its outward
/// normal (inward for a positive thickness), every vertex and edge following. The result
/// is validated; a face or edge that would vanish or turn over is an error.
pub(super) fn offset_body(
    body: &Body,
    open: &BTreeSet<FaceId>,
    thickness: f64,
) -> KernelResult<Body> {
    let faces = body.faces();
    let mut moved: HashMap<FaceId, Moved> = HashMap::new();
    let mut list_faces = Vec::with_capacity(faces.len());
    let edges = body.edges();
    let edge_index: HashMap<EdgeId, usize> =
        edges.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    for (k, &f) in faces.iter().enumerate() {
        let (surface, same_sense) = body
            .face_surface(f)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{f} has no surface")))?;
        let name = body
            .face_name(f)?
            .map_or_else(|| format!("face {k}"), str::to_owned);
        // Against the outward normal, which is the surface normal when `same_sense`.
        let delta = if open.contains(&f) {
            0.0
        } else if same_sense {
            -thickness
        } else {
            thickness
        };
        let new_surface = offset_surface(&surface, delta, &name)?;
        let mut loops = Vec::new();
        for lp in body.face_loops(f)? {
            let mut uses = Vec::new();
            for h in body.loop_half_edges(lp)? {
                let e = body.half_edge_edge(h)?.ok_or_else(|| {
                    KernelError::Unsupported("shelling a face with a one-vertex loop".into())
                })?;
                let i = *edge_index
                    .get(&e)
                    .ok_or_else(|| KernelError::InvalidInput(format!("{e} is not listed")))?;
                uses.push((i, body.edge_half_edges(e)?[0] == h));
            }
            loops.push(uses);
        }
        list_faces.push(ListFace {
            surface: new_surface,
            same_sense,
            name: Some(shell_name(&name)),
            loops,
        });
        moved.insert(
            f,
            Moved {
                original: surface,
                delta,
            },
        );
    }

    let vertices = body.vertices();
    let mut vertex_index: HashMap<VertexId, usize> = HashMap::new();
    let mut list_vertices = Vec::with_capacity(vertices.len());
    let mut new_points: HashMap<VertexId, Point3> = HashMap::new();
    for (k, &v) in vertices.iter().enumerate() {
        let name = body
            .vertex_name(v)?
            .map_or_else(|| format!("vertex {k}"), str::to_owned);
        let around = body
            .vertex_faces(v)?
            .into_iter()
            .map(|f| {
                moved
                    .get(&f)
                    .ok_or_else(|| KernelError::InvalidInput(format!("{f} is not listed")))
            })
            .collect::<KernelResult<Vec<_>>>()?;
        let p = move_vertex(body.vertex_point(v)?, &around, &name)?;
        new_points.insert(v, p);
        vertex_index.insert(v, k);
        list_vertices.push(ListVertex {
            point: p,
            name: body.vertex_name(v)?.map(shell_name),
        });
    }

    let mut list_edges = Vec::with_capacity(edges.len());
    for (k, &e) in edges.iter().enumerate() {
        let name = body
            .edge_name(e)?
            .map_or_else(|| format!("edge {k}"), str::to_owned);
        let [v0, v1] = body.edge_vertices(e)?;
        let (c, t0, t1) = body
            .edge_curve(e)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve")))?;
        let point = |v: VertexId| {
            new_points
                .get(&v)
                .copied()
                .ok_or_else(|| KernelError::InvalidInput(format!("{v} is not listed")))
        };
        let old = [body.vertex_point(v0)?, body.vertex_point(v1)?];
        let new = [point(v0)?, point(v1)?];
        let (curve, n0, n1) = move_edge(&c, t0, t1, old, new, &name)?;
        // The new curve must lie on both offset faces along its whole length.
        for f in body.edge_faces(e)? {
            let m = moved
                .get(&f)
                .ok_or_else(|| KernelError::InvalidInput(format!("{f} is not listed")))?;
            for i in 0..=EDGE_SAMPLES {
                let t = n0 + (n1 - n0) * (i as f64 / EDGE_SAMPLES as f64);
                let (d, _) = signed_distance(&m.original, curve.point_at(t))?;
                if (d - m.delta).abs() > LINEAR_TOLERANCE {
                    return Err(KernelError::Unsupported(format!(
                        "the offset of the edge {name} is not a segment or a circle about the same axis"
                    )));
                }
            }
        }
        let index = |v: VertexId| {
            vertex_index
                .get(&v)
                .copied()
                .ok_or_else(|| KernelError::InvalidInput(format!("{v} is not listed")))
        };
        list_edges.push(ListEdge {
            vertices: [index(v0)?, index(v1)?],
            curve,
            t0: n0,
            t1: n1,
            name: body.edge_name(e)?.map(shell_name),
        });
    }

    let out = Body::from_face_list(&list_vertices, &list_edges, &list_faces)
        .map_err(|e| vanishes(&format!("the offset faces do not close: {e}")))?;
    out.validate()
        .map_err(|e| vanishes(&format!("the offset faces turn inside out: {e}")))?;
    Ok(out)
}

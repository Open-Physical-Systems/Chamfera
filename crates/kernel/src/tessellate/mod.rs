//! Tessellation (rung K5): a valid [`Body`] to a watertight triangle mesh within a chord
//! tolerance, for display and STL export. Meshes never decide anything about the model.
//!
//! # Method
//! 1. **Edges first.** Every edge curve is discretised once ([`edges`]: lines keep their
//!    two end points; circles, arcs and ellipses are subdivided so the chord error is at
//!    most `chord_tolerance` and the tangent turns at most `angle_tolerance` per
//!    segment, e.g. a circle of radius `r` needs `ceil(θ / (2 acos(1 - tol / r)))`
//!    segments over the angle `θ`). Each body vertex becomes one mesh vertex and each
//!    interior edge point one more, so both faces of an edge use the *same* mesh
//!    vertices along it: the mesh has no cracks by construction.
//! 2. **Faces in (u, v).** A face's loops are walked half-edge by half-edge, taking each
//!    edge's points forwards or backwards, mapped into the face surface's parameter
//!    domain ([`domain::SurfaceDomain`]) and oriented so the outer loop runs
//!    counter-clockwise there. The polygon with holes is triangulated by hole bridging
//!    and ear clipping with exact predicates ([`triangulate`]), then improved by
//!    Delaunay flips that never touch the boundary ([`delaunay`]). Curved faces are
//!    refined inside until the triangles are within tolerance of the surface; planes
//!    need none.
//! 3. **Orientation.** Triangles wind counter-clockwise about the face's outward normal
//!    (the surface normal, reversed when the face's `same_sense` is false), so the whole
//!    mesh is consistently oriented and its signed volume is the body's volume.
//!
//! Limits: the options are checked, the triangle budget `max_triangles` is checked
//! before anything large is allocated (per edge, then per face), and every loop is
//! bounded. Nothing panics on any body.
//!
//! # What K3 plugs in for curved surfaces
//! Planes are tessellated through the same (u, v) path that curved faces will use. To
//! tessellate a new [`Surface`] variant (cylinder, cone, sphere, torus):
//! 1. implement [`domain::SurfaceDomain`] for it in `domain.rs`: `surface_point(u, v)`,
//!    its inverse `surface_uv(point)`, `surface_normal(u, v)`, and `u_period` /
//!    `v_period` for closed directions (e.g. `2πr` for `u = r · angle` on a cylinder).
//!    Keep (u, v) roughly isometric and `∂S/∂u × ∂S/∂v` along the surface normal; leave
//!    `is_flat` false so faces are refined;
//! 2. add its arm to [`domain::domain_for`] (the catch-all arm refuses it as
//!    [`KernelError::Unsupported`] until then).
//!
//! That is all: loops crossing a seam edge are unwrapped by the periods
//! ([`domain::unwrap_loop`]), and the refinement inserts interior points until each
//! triangle's centroid and interior edge midpoints are within `chord_tolerance` of the
//! surface and its normal within `angle_tolerance`. A face whose loop winds around a
//! periodic surface without a seam edge (e.g. a cylinder band bounded by two circles
//! only) is refused; the K2 builders always give a lateral face a seam. Faces on surfaces
//! with poles (sphere, cone apex) need the pole to be a vertex of the face's loops, or
//! a parameterisation that avoids it.

mod delaunay;
mod domain;
mod edges;
mod triangulate;

#[cfg(test)]
mod tests;

use crate::{Body, EdgeId, FaceId, KernelError, KernelResult, Surface, VertexId};
use chamfera_geom::{
    ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, MAX_TRIANGLES, PREDICATE_MIN_MAGNITUDE,
    Point2, Point3, TriMesh,
};
use std::collections::HashMap;
use std::f64::consts::PI;

/// How finely to tessellate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TessOptions {
    /// Largest distance between a curve or surface and its chords or triangles (mm).
    /// At least [`LINEAR_TOLERANCE`].
    pub chord_tolerance: f64,
    /// Largest turn of a curve's tangent per segment, and largest angle between a
    /// triangle and its surface's normal (radians), in `(ANGULAR_TOLERANCE, π]`.
    pub angle_tolerance: f64,
    /// Most triangles the whole mesh may have, at most [`MAX_TRIANGLES`]. A body that
    /// would need more is refused rather than truncated.
    pub max_triangles: usize,
}

impl Default for TessOptions {
    /// 0.01 mm chord error, 15° per segment, five million triangles.
    fn default() -> Self {
        TessOptions {
            chord_tolerance: 0.01,
            angle_tolerance: PI / 12.0,
            max_triangles: 5_000_000,
        }
    }
}

impl TessOptions {
    fn check(&self) -> KernelResult<()> {
        let c = self.chord_tolerance;
        if !(c.is_finite() && (LINEAR_TOLERANCE..=MAX_COORDINATE).contains(&c)) {
            return Err(KernelError::InvalidInput(format!(
                "`chord_tolerance` must be a finite number between {LINEAR_TOLERANCE} and {MAX_COORDINATE} mm"
            )));
        }
        let a = self.angle_tolerance;
        if !(a.is_finite() && a > ANGULAR_TOLERANCE && a <= PI) {
            return Err(KernelError::InvalidInput(format!(
                "`angle_tolerance` must be a finite angle above {ANGULAR_TOLERANCE} and at most π radians"
            )));
        }
        if !(1..=MAX_TRIANGLES).contains(&self.max_triangles) {
            return Err(KernelError::InvalidInput(format!(
                "`max_triangles` must be between 1 and {MAX_TRIANGLES}"
            )));
        }
        Ok(())
    }
}

/// A tessellated body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BodyMesh {
    /// The triangles, counter-clockwise seen from outside; closed and consistently
    /// oriented (every mesh edge is used by exactly two triangles, in opposite
    /// directions).
    pub mesh: TriMesh,
    /// The face each triangle belongs to (for picking), one per triangle.
    pub face_of_triangle: Vec<FaceId>,
    /// Each edge's polyline along its curve direction (from its first vertex), the same
    /// points the triangles use: the true edges to draw, not mesh feature edges.
    pub edge_polylines: Vec<(EdgeId, Vec<Point3>)>,
}

/// Tessellates every face of `body`. Fails, without a partial mesh, when the options are
/// out of range, the body lacks geometry (an edge without a curve, a face without a
/// surface: [`KernelError::Unsupported`] for faces whose surface cannot be tessellated
/// yet), or the mesh would exceed `max_triangles`.
pub fn tessellate(body: &Body, options: &TessOptions) -> KernelResult<BodyMesh> {
    options.check()?;
    body.validate_topology()?;
    let mut out = BodyMesh::default();
    let mut vertex_index: HashMap<VertexId, u32> = HashMap::new();
    for v in body.vertices() {
        let i = index(out.mesh.positions.len())?;
        out.mesh.positions.push(body.vertex_point(v)?);
        vertex_index.insert(v, i);
    }

    // Edges: count first, so a hostile tolerance fails before allocating.
    let mut plans = Vec::new();
    let mut total = 0usize;
    for e in body.edges() {
        let (curve, t0, t1) = body.edge_curve(e)?.ok_or_else(|| {
            KernelError::InvalidInput(format!("{e} has no curve, so it cannot be tessellated"))
        })?;
        let n = edges::segment_count(&curve, t0, t1, options)?;
        total = total.saturating_add(n);
        // Every segment is a side of at least one triangle on each of its two faces,
        // and a triangle has three sides: at least 2n/3 triangles.
        if total.saturating_mul(2) / 3 > options.max_triangles {
            return Err(over_budget(options));
        }
        plans.push((e, curve, t0, t1, n));
    }
    let mut edge_points: HashMap<EdgeId, Vec<u32>> = HashMap::new();
    for (e, curve, t0, t1, n) in plans {
        let [v0, v1] = body.edge_vertices(e)?;
        let points = edges::discretise(
            &curve,
            t0,
            t1,
            n,
            body.vertex_point(v0)?,
            body.vertex_point(v1)?,
        );
        let mut ids = Vec::with_capacity(points.len());
        let last = points.len().saturating_sub(1);
        for (k, p) in points.iter().enumerate() {
            let id = match k {
                0 => vertex_id(&vertex_index, v0)?,
                k if k == last => vertex_id(&vertex_index, v1)?,
                _ => {
                    let i = index(out.mesh.positions.len())?;
                    out.mesh.positions.push(*p);
                    i
                }
            };
            ids.push(id);
        }
        edge_points.insert(e, ids);
        out.edge_polylines.push((e, points));
    }

    for f in body.faces() {
        let left = options.max_triangles - out.mesh.triangles.len();
        let tris = tessellate_face(
            body,
            f,
            &edge_points,
            &mut out.mesh.positions,
            options,
            left,
        )?;
        out.face_of_triangle
            .extend(std::iter::repeat_n(f, tris.len()));
        out.mesh.triangles.extend(tris);
    }
    out.mesh.validate().map_err(|e| {
        KernelError::InvalidResult(format!("the tessellation is not a usable mesh: {e}"))
    })?;
    Ok(out)
}

fn over_budget(options: &TessOptions) -> KernelError {
    KernelError::InvalidInput(format!(
        "the mesh would need more than `max_triangles` = {} triangles; raise `chord_tolerance` or `angle_tolerance`, or allow more triangles",
        options.max_triangles
    ))
}

fn index(i: usize) -> KernelResult<u32> {
    u32::try_from(i).map_err(|_| KernelError::InvalidInput("the mesh has too many vertices".into()))
}

fn vertex_id(map: &HashMap<VertexId, u32>, v: VertexId) -> KernelResult<u32> {
    map.get(&v)
        .copied()
        .ok_or_else(|| KernelError::InvalidResult(format!("{v} is not in the mesh")))
}

/// Coordinates the exact predicates cannot take (non-zero below 2^-200, from rounding
/// in the projection) become zero; they are far below any length the model resolves.
fn snap(v: f64) -> f64 {
    if v.abs() < PREDICATE_MIN_MAGNITUDE {
        0.0
    } else {
        v
    }
}

/// Triangles of one face, as mesh vertex indices; points added inside the face are
/// appended to `positions`. `budget` is how many triangles the face may still have.
fn tessellate_face(
    body: &Body,
    face: FaceId,
    edge_points: &HashMap<EdgeId, Vec<u32>>,
    positions: &mut Vec<Point3>,
    options: &TessOptions,
    budget: usize,
) -> KernelResult<Vec<[u32; 3]>> {
    let (surface, same_sense): (Surface, bool) = body.face_surface(face)?.ok_or_else(|| {
        KernelError::Unsupported(format!(
            "{face} has no surface yet (curved surfaces arrive with K3), so it cannot be tessellated"
        ))
    })?;
    let domain = domain::domain_for(&surface)?;

    // Each loop as a ring of local points (one per use of a mesh vertex, so a seam used
    // twice gets two (u, v) positions).
    let mut global: Vec<u32> = Vec::new();
    let mut uv: Vec<Point2> = Vec::new();
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for lp in body.face_loops(face)? {
        let mut ring_global = Vec::new();
        for h in body.loop_half_edges(lp)? {
            let e = body.half_edge_edge(h)?.ok_or_else(|| {
                KernelError::InvalidInput(format!("{face} has a loop of a single vertex"))
            })?;
            let ids = edge_points
                .get(&e)
                .ok_or_else(|| KernelError::InvalidResult(format!("{e} was not discretised")))?;
            let forward = body.edge_half_edges(e)?[0] == h;
            let n = ids.len().saturating_sub(1);
            if forward {
                ring_global.extend_from_slice(ids.get(..n).unwrap_or(&[]));
            } else {
                ring_global.extend(ids.get(1..).unwrap_or(&[]).iter().rev());
            }
        }
        let mut ring_uv: Vec<Point2> = ring_global
            .iter()
            .map(|&g| {
                let p = positions.get(g as usize).copied().unwrap_or(Point3::ORIGIN);
                let q = domain.surface_uv(p);
                Point2::new(snap(q.x), snap(q.y))
            })
            .collect();
        domain::unwrap_loop(&mut ring_uv, domain.u_period(), domain.v_period())?;
        if !same_sense {
            // The outward normal is against the surface's: run the loops the other way
            // in (u, v) and turn the triangles back at the end.
            ring_global.reverse();
            ring_uv.reverse();
        }
        let start = uv.len();
        let ring: Vec<u32> = (start..start + ring_uv.len())
            .map(index)
            .collect::<KernelResult<_>>()?;
        global.extend(ring_global);
        uv.extend(ring_uv);
        rings.push(ring);
    }
    let Some((outer, holes)) = rings.split_first() else {
        return Ok(Vec::new());
    };
    if triangulate::twice_signed_area(&uv, outer) <= 0.0
        || holes
            .iter()
            .any(|h| triangulate::twice_signed_area(&uv, h) >= 0.0)
    {
        return Err(KernelError::InvalidInput(format!(
            "the loops of {face} do not run counter-clockwise (outer) and clockwise (holes) about its outward normal"
        )));
    }
    let expected = uv.len() + 2 * holes.len();
    if expected.saturating_sub(2) > budget {
        return Err(over_budget(options));
    }

    let tri = triangulate::triangulate(&uv, outer, holes)?;
    let xyz: Vec<Point3> = global
        .iter()
        .map(|&g| positions.get(g as usize).copied().unwrap_or(Point3::ORIGIN))
        .collect();
    let boundary_points = uv.len();
    let mut mesh = delaunay::Mesh2::new(uv, xyz, tri.triangles);
    mesh.make_delaunay();
    if !domain.is_flat() {
        mesh.refine(
            domain.as_ref(),
            options.chord_tolerance,
            options.angle_tolerance,
            budget,
        )?;
    }
    // Points the refinement added become new mesh vertices.
    for p in mesh.xyz.iter().skip(boundary_points) {
        global.push(index(positions.len())?);
        positions.push(*p);
    }
    let map = |l: u32| -> KernelResult<u32> {
        global
            .get(l as usize)
            .copied()
            .ok_or_else(|| KernelError::InvalidResult("a triangle lost its vertex".into()))
    };
    mesh.triangles
        .iter()
        .map(|&[a, b, c]| {
            let (a, b, c) = (map(a)?, map(b)?, map(c)?);
            Ok(if same_sense { [a, b, c] } else { [a, c, b] })
        })
        .collect()
}

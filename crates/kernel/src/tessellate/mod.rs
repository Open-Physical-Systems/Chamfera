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
//!    refined inside until the triangles are within tolerance of the surface, then
//!    improved by smoothing and flips judged in space; planes need none.
//! 3. **Orientation.** Triangles wind counter-clockwise about the face's outward normal
//!    (the surface normal, reversed when the face's `same_sense` is false), so the whole
//!    mesh is consistently oriented and its signed volume is the body's volume.
//!
//! Limits: the options are checked, the triangle budget `max_triangles` is checked
//! before anything large is allocated (per edge, then per face), and every loop is
//! bounded. Nothing panics on any body.
//!
//! # Curved faces
//! Every [`Surface`] has a [`domain::SurfaceDomain`] (`domain.rs`): the plane its frame's
//! coordinates; the cylinder, cone, sphere and torus their kernel parameters scaled to
//! lengths, `(s_u θ, s_v w)`, with `u` periodic (and `v` on a torus).
//!
//! - **Seams.** A seam edge is used twice by its face; its points get two (u, v)
//!   positions a period apart, because the loop is unwrapped point by point to the
//!   representative nearest the previous one. A face whose loop winds around a
//!   periodic surface without a seam edge (a cylinder band bounded by two circles only)
//!   is refused; the K2 and K4 builders always give a lateral face a seam.
//! - **Poles and apexes.** A loop vertex on the surface's axis (a sphere's pole, a cone's
//!   apex, a spindle torus's tip, where `θ` is undefined) is replaced in (u, v) by a run
//!   of copies along its singular line, from the u the loop arrives with to the u it
//!   leaves with: toward -u at a pole above the face, +u below it, by up to a full
//!   period (see `loop_uv`). The copies are one mesh vertex, so the triangles fanning
//!   into them share the pole bit for bit; the triangles between two copies are segments
//!   in space and are dropped, which leaves the fan closed. A pole that is not a vertex
//!   of the face's loops cannot be tessellated by this chart (sweeps never make one).
//! - **Refinement** inserts side midpoints or centroids until every triangle's centroid
//!   and interior side midpoints are within `chord_tolerance` of the surface, the
//!   normal turns by at most twice `angle_tolerance` along each interior side, and no
//!   interior side spans more than a quarter turn (near a pole the other tests allow
//!   wider triangles, which would fold over); smoothing and flips in space then improve
//!   the triangles' fit without breaking those tests ([`delaunay`]).

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
use domain::SurfaceDomain;
use std::collections::HashMap;
use std::f64::consts::PI;

/// How finely to tessellate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TessOptions {
    /// Largest distance between a curve or surface and its chords or triangles (mm).
    /// At least [`LINEAR_TOLERANCE`].
    pub chord_tolerance: f64,
    /// Largest turn of a curve's tangent per segment, and about the largest angle
    /// between a triangle and its surface's normal (radians; the surface normal turns
    /// by at most twice this along a triangle's side), in `(ANGULAR_TOLERANCE, π]`.
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
/// surface: [`KernelError::Unsupported`]), or the mesh would exceed `max_triangles`.
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
            "{face} has no surface, so it cannot be tessellated"
        ))
    })?;
    // Each loop as the ring of mesh vertices along its half-edges.
    let mut loops: Vec<Vec<u32>> = Vec::new();
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
        if !same_sense {
            // The outward normal is against the surface's: run the loops the other way
            // in (u, v) and turn the triangles back at the end.
            ring_global.reverse();
        }
        loops.push(ring_global);
    }
    let at = |g: u32| positions.get(g as usize).copied().unwrap_or(Point3::ORIGIN);
    let boundary: Vec<Point3> = loops.iter().flatten().map(|&g| at(g)).collect();
    let domain = domain::domain_for(&surface, &boundary)?;

    // Each loop as a ring of local points in (u, v): one per use of a mesh vertex, so a
    // seam used twice gets two positions and a pole a run of them.
    let mut global: Vec<u32> = Vec::new();
    let mut uv: Vec<Point2> = Vec::new();
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for ring_global in loops {
        let (ring_global, ring_uv) = loop_uv(
            domain.as_ref(),
            positions,
            &ring_global,
            options.angle_tolerance,
            budget,
        )
        .map_err(|e| match e {
            KernelError::InvalidInput(m) => KernelError::InvalidInput(format!("{face}: {m}")),
            other => other,
        })?;
        if ring_uv.len() > budget.saturating_add(2) {
            return Err(over_budget(options));
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
    let mut out = Vec::with_capacity(mesh.triangles.len());
    for &[a, b, c] in &mesh.triangles {
        let (a, b, c) = (map(a)?, map(b)?, map(c)?);
        // A triangle on two copies of a pole is a segment in space; its two sides there
        // run both ways between the same vertices, so leaving it out keeps the mesh
        // closed (each of those sides is matched by a neighbour's).
        if a == b || b == c || c == a {
            continue;
        }
        out.push(if same_sense { [a, b, c] } else { [a, c, b] });
    }
    Ok(out)
}

/// A loop's ring of mesh vertices (already in the order that puts the face on its left
/// in (u, v)) as local points: the mesh vertex of each and its (u, v), unwrapped across
/// seams, with every singular point (pole, apex) replaced by a run of copies along its
/// singular line.
///
/// At a singular point u is undefined; the loop arrives along one meridian and leaves
/// along another, and in (u, v) it must walk along the singular line between them. With
/// the face on the left, a pole above the face (the loop arrives from smaller v) is
/// walked toward -u and one below it toward +u (as the top and bottom of a
/// counter-clockwise rectangle are), by more than zero and at most one period: the
/// full period when the loop arrives and leaves along the same seam (a whole sphere or
/// cone), less at the corner of a partial revolve. The run has a copy at least every
/// half [`SurfaceDomain::max_span`], so the triangles fanning into the pole stay narrow;
/// triangles between two copies are segments in space and are dropped afterwards.
fn loop_uv(
    domain: &dyn SurfaceDomain,
    positions: &[Point3],
    ring: &[u32],
    angle_tolerance: f64,
    max_points: usize,
) -> KernelResult<(Vec<u32>, Vec<Point2>)> {
    let pts: Vec<(u32, Point2, bool)> = ring
        .iter()
        .map(|&g| {
            let p = positions.get(g as usize).copied().unwrap_or(Point3::ORIGIN);
            let q = domain.surface_uv(p);
            (g, Point2::new(snap(q.x), snap(q.y)), domain.is_singular(p))
        })
        .collect();
    let n = pts.len();
    // Start at a regular point, so every singular run has regular points on both sides.
    let Some(start) = pts.iter().position(|q| !q.2) else {
        return Err(KernelError::InvalidInput(
            "a face loop lies entirely on its surface's axis".into(),
        ));
    };
    let at = |k: usize| pts.get((start + k) % n).copied();
    let (pu, pv) = (domain.u_period(), domain.v_period());
    // A copy at least every angle tolerance around the axis (and half the fold span).
    let spacing = match pu {
        Some(p) => (p * angle_tolerance / std::f64::consts::TAU).min(0.5 * domain.max_span().0),
        None => f64::INFINITY,
    };
    let mut out_g: Vec<u32> = Vec::with_capacity(n);
    let mut out_uv: Vec<Point2> = Vec::with_capacity(n);
    let mut last_regular_v = 0.0;
    for k in 0..n {
        let Some((g, q, singular)) = at(k) else {
            continue;
        };
        let Some(last) = out_uv.last().copied() else {
            out_g.push(g);
            out_uv.push(q);
            last_regular_v = q.y;
            continue;
        };
        let v = domain::nearest(last.y, q.y, pv);
        if !singular {
            out_g.push(g);
            out_uv.push(Point2::new(domain::nearest(last.x, q.x, pu), v));
            last_regular_v = v;
            continue;
        }
        let next = (1..n).filter_map(|j| at(k + j)).find(|q| !q.2);
        let next_is_singular = at(k + 1).is_some_and(|q| q.2);
        let dir = if v > last_regular_v {
            -1.0
        } else if v < last_regular_v {
            1.0
        } else {
            0.0
        };
        let (Some(p), Some((_, next_q, _)), false, true) = (pu, next, next_is_singular, dir != 0.0)
        else {
            // Within a run of singular points (only on faces near the tolerance), or with
            // nothing to walk: keep u.
            out_g.push(g);
            out_uv.push(Point2::new(last.x, v));
            continue;
        };
        let step = domain::directed_step(next_q.x - last.x, dir, p, p * 1e-12);
        let copies = (step.abs() / spacing).ceil();
        if !(copies.is_finite() && copies <= max_points as f64) {
            return Err(KernelError::InvalidInput(
                "a pole would need more points than `max_triangles` at this angle tolerance".into(),
            ));
        }
        // `copies` is a whole number no larger than `max_points`, so it fits.
        let m = (copies as usize).max(1);
        for i in 0..=m {
            out_g.push(g);
            out_uv.push(Point2::new(last.x + step * (i as f64 / m as f64), v));
        }
    }
    domain::check_closed(&out_uv, pu, pv)?;
    Ok((out_g, out_uv))
}

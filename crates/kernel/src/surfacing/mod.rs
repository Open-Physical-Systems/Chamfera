//! Surface bodies (open shells): swept surfaces without caps, planar patches, stitching
//! surfaces together, and trimming them.
//!
//! # Swept surfaces
//! [`surface_extrude`], [`surface_revolve`] and [`surface_sweep`] reuse the solid sweeps
//! (K4 and the path sweep) and then take faces out with [`Body::without_faces`], which
//! leaves openings where they were (see the `topology` module documentation):
//! - a closed profile keeps its swept sides and loses its caps (`top` and `bottom`,
//!   `start cap` and `end cap`, `start` and `end`);
//! - an open chain of profile edges is first closed into a simple region (by the chord
//!   between its ends, or by two lines through a point beside it when the chord is
//!   degenerate, crosses the chain or puts the region across a revolve's axis); the
//!   region is swept, and the faces swept by the closing lines and the caps are taken out.
//!
//! Every face keeps the name the solid sweep gives it (`side i` is swept by chain edge
//! `i`), so references to the surface's faces and edges follow it like a solid's.
//!
//! # Patches
//! [`patch_profile`] fills a closed sketch profile with a planar face (normal along the
//! sketch normal); [`patch_edges`] fills a closed planar loop of a body's edges. On the
//! rim of a surface body the patch runs each edge against the surface's own use of it, so
//! patch and surface have consistent orientations and stitch together.
//!
//! # Stitching and trimming
//! [`stitch`] (`stitch.rs`) joins surface bodies along edges they share; [`trim_by_plane`]
//! and [`trim_by_solid`] cut a surface with the boolean machinery (K7): the surface's
//! faces are split along their intersection with the tool, classified against it, and the
//! kept pieces sewn into a new open shell.

mod stitch;
#[cfg(test)]
mod tests;

pub use stitch::stitch;

use crate::topology::{FaceList, Validation};
use crate::{
    Body, Curve, EdgeId, KernelError, KernelResult, Line2, ListEdge, ListFace, ListVertex,
    PathSegment, Profile, ProfileEdge, Surface,
};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Vec2};
use std::collections::HashMap;

/// Most edges an open profile chain or a patch loop may have.
pub const MAX_SURFACE_EDGES: usize = 10_000;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// What a swept surface sweeps: the sides of a closed profile, or an open chain of
/// profile edges joined end to end.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceProfile {
    Closed(Profile),
    Open(Vec<ProfileEdge>),
}

/// `profile` extruded by `distance` as a surface: the swept sides only.
pub fn surface_extrude(
    frame: &Frame,
    profile: &SurfaceProfile,
    distance: f64,
) -> KernelResult<Body> {
    swept(profile, None, &["top", "bottom"], |p| {
        crate::extrude(frame, p, distance)
    })
}

/// `profile` revolved about `axis` by `angle` as a surface: the swept sides only.
pub fn surface_revolve(
    frame: &Frame,
    profile: &SurfaceProfile,
    axis: &Line2,
    angle: f64,
) -> KernelResult<Body> {
    swept(profile, Some(axis), &["start cap", "end cap"], |p| {
        crate::revolve(frame, p, axis, angle)
    })
}

/// `profile` swept along `path` as a surface: the swept sides only.
pub fn surface_sweep(
    frame: &Frame,
    profile: &SurfaceProfile,
    path: &[PathSegment],
) -> KernelResult<Body> {
    swept(profile, None, &["start", "end"], |p| {
        crate::sweep_path(frame, p, path)
    })
}

/// The index `i` of a face named `side i` or `segment k side i`.
fn side_index(name: &str) -> Option<usize> {
    let (head, i) = name.rsplit_once("side ")?;
    if !(head.is_empty() || head.starts_with("segment ")) {
        return None;
    }
    i.parse().ok()
}

/// Runs the solid sweep `op` on the profile (closed first when it is an open chain) and
/// takes out the caps and the faces swept by closing lines.
fn swept(
    profile: &SurfaceProfile,
    axis: Option<&Line2>,
    caps: &[&str],
    op: impl Fn(&Profile) -> KernelResult<Body>,
) -> KernelResult<Body> {
    let strip = |body: Body, chain: Option<usize>| -> KernelResult<Body> {
        let mut remove = Vec::new();
        for f in body.faces() {
            let name = body.face_name(f)?.unwrap_or("");
            let closing = chain.is_some_and(|n| side_index(name).is_some_and(|i| i >= n));
            if caps.contains(&name) || closing {
                remove.push(f);
            }
        }
        body.without_faces(&remove)
    };
    match profile {
        SurfaceProfile::Closed(p) => strip(op(p)?, None),
        SurfaceProfile::Open(chain) => {
            let chain = check_chain(chain)?;
            if chain_closed(&chain) {
                return strip(op(&Profile::new(chain, Vec::new()))?, None);
            }
            let mut first_err = None;
            for closing in closures(&chain, axis) {
                let mut edges = chain.clone();
                edges.extend(closing);
                match op(&Profile::new(edges, Vec::new())) {
                    Ok(body) => return strip(body, Some(chain.len())),
                    Err(e) => {
                        first_err.get_or_insert(e);
                    }
                }
            }
            Err(first_err.unwrap_or_else(|| invalid("the open curve could not be swept")))
        }
    }
}

fn check_chain(chain: &[ProfileEdge]) -> KernelResult<Vec<ProfileEdge>> {
    if chain.is_empty() {
        return Err(invalid("pick at least one curve to sweep"));
    }
    if chain.len() > MAX_SURFACE_EDGES {
        return Err(invalid(format!(
            "an open curve may have at most {MAX_SURFACE_EDGES} edges"
        )));
    }
    for w in chain.windows(2) {
        if w[0].end().distance(w[1].start()) > crate::PROFILE_JOIN_TOLERANCE {
            return Err(invalid(
                "the curves must join end to end, in order, into one chain",
            ));
        }
    }
    for e in chain {
        let ok = |p: Point2| p.is_valid() && p.x.abs().max(p.y.abs()) <= MAX_COORDINATE;
        if !(ok(e.start()) && ok(e.end())) {
            return Err(invalid("a curve lies outside the working range"));
        }
    }
    Ok(chain.to_vec())
}

fn chain_closed(chain: &[ProfileEdge]) -> bool {
    match (chain.first(), chain.last()) {
        (Some(a), Some(b)) => b.end().distance(a.start()) <= crate::PROFILE_JOIN_TOLERANCE,
        _ => false,
    }
}

/// Ways to close an open chain from its end back to its start, to try in order: the
/// chord, then two lines through a point beside the chord's middle (each side, near and
/// far), preferring the side away from a revolve's axis.
fn closures(chain: &[ProfileEdge], axis: Option<&Line2>) -> Vec<Vec<ProfileEdge>> {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return Vec::new();
    };
    let (a, b) = (first.start(), last.end());
    let mut out = Vec::new();
    if a.distance(b) > LINEAR_TOLERANCE {
        out.push(vec![ProfileEdge::line(b, a)]);
    }
    // The chain's size: the box around its edge ends.
    let mut lo = a;
    let mut hi = a;
    for e in chain {
        for p in [e.start(), e.end()] {
            lo = Point2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Point2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
    }
    let size = (hi - lo).length().max(a.distance(b)).max(1.0);
    let mid = Point2::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
    let Some(perp) = (b - a).normalized().map(|d| d.perp()) else {
        return out;
    };
    // Away from the axis first.
    let mut sides = [1.0, -1.0];
    if let Some(ax) = axis {
        let n = ax.direction().perp();
        let side = (mid - ax.origin()).dot(n);
        if perp.dot(n) * side < 0.0 {
            sides = [-1.0, 1.0];
        }
    }
    for scale in [1.0, 4.0] {
        for s in sides {
            let p = mid + perp * (s * scale * size);
            if p.x.abs().max(p.y.abs()) <= MAX_COORDINATE {
                out.push(vec![ProfileEdge::line(b, p), ProfileEdge::line(p, a)]);
            }
        }
    }
    out
}

/// A planar face filling `profile` (with its holes) in `frame`'s plane, its normal along
/// the frame's normal. The face is named `patch`, its edges `patch i` (`hole j patch i`).
pub fn patch_profile(frame: &Frame, profile: &Profile) -> KernelResult<Body> {
    // Extruded the other way, the extrusion's `bottom` lies on the sketch plane facing
    // along the frame's normal.
    let solid = crate::extrude(frame, profile, -1.0)?;
    let mut remove = Vec::new();
    let mut keep = None;
    for f in solid.faces() {
        if solid.face_name(f)? == Some("bottom") {
            keep = Some(f);
        } else {
            remove.push(f);
        }
    }
    if keep.is_none() {
        return Err(KernelError::InvalidResult(
            "the patch's face went missing".into(),
        ));
    }
    let mut body = solid.without_faces(&remove)?;
    for f in body.faces() {
        body.set_face_name(f, "patch")?;
    }
    for e in body.edges() {
        if let Some(n) = body.edge_name(e)?.map(str::to_owned) {
            body.set_edge_name(e, &n.replacen("bottom", "patch", 1))?;
        }
    }
    for v in body.vertices() {
        if let Some(n) = body.vertex_name(v)?.map(str::to_owned) {
            body.set_vertex_name(v, &n.replacen("bottom", "patch", 1))?;
        }
    }
    Ok(body)
}

/// A planar face filling the closed loop of `edges` of `body` (any order). On the rim of
/// a surface the patch runs each edge against the surface's use of it, so the two can be
/// stitched; otherwise the loop's first edge sets the direction. The edges must join into
/// one closed loop lying in a plane. The face is named `patch`; edges keep their names.
pub fn patch_edges(body: &Body, edges: &[EdgeId]) -> KernelResult<Body> {
    if edges.is_empty() {
        return Err(invalid("pick the edges of a closed loop to patch"));
    }
    if edges.len() > MAX_SURFACE_EDGES {
        return Err(invalid(format!(
            "a patch may have at most {MAX_SURFACE_EDGES} edges"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    // Per edge: its vertices and the direction the patch must use it in, if fixed.
    let mut info = Vec::new();
    for &e in edges {
        if !seen.insert(e) {
            return Err(invalid("an edge is picked twice"));
        }
        let vs = body.edge_vertices(e)?;
        let fixed = match body.boundary_edge_face(e)? {
            // The surface uses `h`; the patch runs against it.
            Some((_, h)) => Some(body.edge_half_edges(e)?[0] != h),
            None => None,
        };
        info.push((e, vs, fixed));
    }
    // Chain head to tail.
    let mut used = vec![false; info.len()];
    let (e0, vs0, fixed0) = info[0];
    let fwd0 = fixed0.unwrap_or(true);
    used[0] = true;
    let mut lp = vec![(e0, fwd0)];
    let start = if fwd0 { vs0[0] } else { vs0[1] };
    let mut at = if fwd0 { vs0[1] } else { vs0[0] };
    for _ in 1..info.len() {
        let mut found = None;
        for (k, (e, vs, fixed)) in info.iter().enumerate() {
            if used[k] {
                continue;
            }
            let options: &[bool] = match fixed {
                Some(true) => &[true],
                Some(false) => &[false],
                None => &[true, false],
            };
            for &fwd in options {
                let from = if fwd { vs[0] } else { vs[1] };
                if from == at {
                    found = Some((k, *e, fwd, if fwd { vs[1] } else { vs[0] }));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((k, e, fwd, end)) = found else {
            return Err(invalid(
                "the edges do not join into one closed loop (or their directions disagree with the surfaces they bound)",
            ));
        };
        used[k] = true;
        lp.push((e, fwd));
        at = end;
    }
    if at != start {
        return Err(invalid("the edges do not close into a loop"));
    }
    // A face list of the loop.
    let mut list = FaceList::default();
    let mut vmap = HashMap::new();
    let mut uses = Vec::new();
    let mut area = chamfera_geom::Vec3::ZERO;
    let reference = body.vertex_point(start)?;
    for &(e, fwd) in &lp {
        let (curve, t0, t1) = body
            .edge_curve(e)?
            .ok_or_else(|| invalid(format!("{e} has no curve")))?;
        let m = curve.area_moment(t0, t1, reference);
        area = if fwd { area + m } else { area - m };
        let mut vv = [0usize; 2];
        for (k, v) in body.edge_vertices(e)?.into_iter().enumerate() {
            vv[k] = match vmap.get(&v) {
                Some(i) => *i,
                None => {
                    list.vertices.push(ListVertex {
                        point: body.vertex_point(v)?,
                        name: body.vertex_name(v)?.map(str::to_owned),
                    });
                    vmap.insert(v, list.vertices.len() - 1);
                    list.vertices.len() - 1
                }
            };
        }
        list.edges.push(ListEdge {
            vertices: vv,
            curve,
            t0,
            t1,
            name: body.edge_name(e)?.map(str::to_owned),
        });
        uses.push((list.edges.len() - 1, fwd));
    }
    let largest = area.x.abs().max(area.y.abs()).max(area.z.abs());
    let frame = if largest > LINEAR_TOLERANCE * LINEAR_TOLERANCE && largest.is_finite() {
        Frame::from_normal(reference, area * (1.0 / largest))
    } else {
        None
    }
    .ok_or_else(|| invalid("the loop encloses no area, so it has no plane to patch"))?;
    let surface = Surface::Plane(frame);
    for edge in &list.edges {
        for i in 0..=8 {
            let t = edge.t0 + (edge.t1 - edge.t0) * f64::from(i) / 8.0;
            let off = surface
                .plane_distance(edge.curve.point_at(t))
                .unwrap_or(f64::INFINITY);
            if !(off.abs() <= LINEAR_TOLERANCE) {
                return Err(KernelError::Unsupported(
                    "the edges do not lie in one plane; only planar patches are supported yet"
                        .into(),
                ));
            }
        }
    }
    list.faces.push(ListFace {
        surface,
        same_sense: true,
        name: Some("patch".into()),
        loops: vec![uses],
    });
    list.assemble()
}

/// `surface` trimmed by the plane `plane`: the part on the side the plane's normal points
/// to is removed when `keep_below` is true, the other part otherwise. Fails with
/// [`KernelError::EmptyResult`] when nothing is left, and as unsupported where the surface
/// meets the plane in a curve K6 cannot represent.
pub fn trim_by_plane(surface: &Body, plane: &Frame, keep_below: bool) -> KernelResult<Body> {
    let bounds = surface
        .bounding_box()
        .ok_or_else(|| invalid("the surface has no geometry"))?;
    // A box beyond the surface on the removed side: a square about the surface's centre
    // projected on the plane, extruded off it.
    let size = (bounds.max - bounds.min).length() + 10.0;
    let centre = plane.to_plane(bounds.center());
    let square = Profile::polygon(&[
        centre + Vec2::new(-size, -size),
        centre + Vec2::new(size, -size),
        centre + Vec2::new(size, size),
        centre + Vec2::new(-size, size),
    ]);
    let distance = if keep_below { 2.0 * size } else { -2.0 * size };
    let tool = crate::extrude(plane, &square, distance)?;
    trim_by_solid(surface, &tool, false)
}

/// `surface` trimmed by the closed solid `tool`: the pieces inside the tool are kept when
/// `keep_inside`, else the pieces outside. Pieces lying on the tool's boundary go.
pub fn trim_by_solid(surface: &Body, tool: &Body, keep_inside: bool) -> KernelResult<Body> {
    surface
        .validate_as(Validation::Surface)
        .map_err(|e| invalid(format!("the surface is not valid: {e}")))?;
    crate::boolean::trim(surface, tool, keep_inside)
}

/// Whether two curve points coincide (used by stitching).
pub(crate) fn same_point_on(a: &Curve, ta: f64, b: &Curve, tb: f64) -> bool {
    a.point_at(ta).coincident(b.point_at(tb))
}

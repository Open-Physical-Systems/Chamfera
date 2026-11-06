//! Splitting by a plane, and thickening a face: the body tools of a modelling workflow.
//!
//! - [`split_body`] cuts a solid into the material on either side of a plane, by two
//!   booleans (K7) with a box standing on the plane and larger than the solid: the
//!   part in front of the plane is `body ∩ box`, the part behind it `body − box`. The
//!   box's face on the plane becomes the new face of both parts (named by the caller).
//! - [`split_faces`] imprints a plane on chosen flat faces: each face is cut along the
//!   line where the plane meets it, so the body gains faces and edges but keeps its
//!   shape and volume. It works on the boundary description of the solid (the form
//!   [`Body::from_boundary`] reads, Mantyla ch. 12 and ISO 10303-42): the face's edges
//!   are split where they cross the plane (exact K6 curve-surface intersection), the
//!   pieces of the line inside the face (decided by the K3 inside-face test at their
//!   middle) become new straight edges, each splitting a loop in two, and the body is
//!   linked again and validated.
//! - [`thicken`] gives a face a thickness: a flat face is extruded along its normal
//!   (K4) from its own loops, and a full cylindrical face between two circles becomes a
//!   ring, the exact difference of two coaxial cylinders, extruded as an annulus.
//!
//! Every failure is a [`KernelError`] and no body; the input is never changed.

use crate::{
    Body, Curve, FaceId, KernelError, KernelResult, PointOnFace, Profile, ProfileEdge, Segment3,
    Surface, intersect_curve_surface_on,
};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point3};
use std::collections::BTreeMap;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

fn unsupported(msg: impl Into<String>) -> KernelError {
    KernelError::Unsupported(msg.into())
}

/// The signed distance of `p` in front of the plane (along its normal).
fn side(plane: &Frame, p: Point3) -> f64 {
    (p - plane.origin()).dot(plane.z())
}

/// The two parts of `body` on either side of `plane`: `(behind, in front)`, in front
/// being along the plane's normal. The face each part gets on the plane is named
/// `face_name`. Refused when the plane does not cut the solid (one side would be empty).
pub fn split_body(body: &Body, plane: &Frame, face_name: &str) -> KernelResult<(Body, Body)> {
    let bounds = body
        .bounding_box()
        .ok_or_else(|| invalid("the solid is empty"))?;
    let centre = bounds.center();
    let diagonal = bounds.size().length();
    let offset = side(plane, centre);
    // Every point of the solid is within diagonal / 2 of the centre, so a square of half
    // size `diagonal` about the centre's foot on the plane, and a height past the far
    // side, contain all of it in front of the plane.
    let half = diagonal + 1.0;
    let height = diagonal + offset.abs() + 1.0;
    let foot = centre + plane.z() * (-offset);
    if !foot.is_valid() || half + height + foot.distance(Point3::ORIGIN) > MAX_COORDINATE {
        return Err(invalid("the split plane is too far from the solid"));
    }
    let frame = plane
        .with_origin(foot)
        .ok_or_else(|| invalid("the split plane leaves the working range"))?;
    let corners = [(-half, -half), (half, -half), (half, half), (-half, half)];
    let outer = (0..4)
        .map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            ProfileEdge::line(
                chamfera_geom::Point2::new(a.0, a.1),
                chamfera_geom::Point2::new(b.0, b.1),
            )
        })
        .collect();
    let mut tool = crate::extrude(&frame, &Profile::new(outer, Vec::new()), height)?;
    for f in tool.faces() {
        tool.set_face_name(f, face_name)?;
    }
    let misses = || invalid("the plane does not cut the solid: all of it lies on one side");
    let front = match crate::intersect(body, &tool) {
        Ok(b) => b,
        Err(KernelError::EmptyResult) => return Err(misses()),
        Err(e) => return Err(e),
    };
    let back = match crate::subtract(body, &tool) {
        Ok(b) => b,
        Err(KernelError::EmptyResult) => return Err(misses()),
        Err(e) => return Err(e),
    };
    Ok((back, front))
}

/// An edge of a boundary description: from, to, curve, t0, t1.
type DescEdge = (usize, usize, Curve, f64, f64);
/// A face of a boundary description: surface, same sense, loops of (edge, forward).
type DescFace = (Surface, bool, Vec<Vec<(usize, bool)>>);

/// A one-shell body as the description [`Body::from_boundary`] reads, with its names.
struct Description {
    points: Vec<Point3>,
    edges: Vec<DescEdge>,
    faces: Vec<DescFace>,
    face_names: Vec<Option<String>>,
    edge_names: Vec<Option<String>>,
    vertex_names: Vec<Option<String>>,
    /// The input face behind each description face.
    face_ids: Vec<FaceId>,
}

fn describe(body: &Body) -> KernelResult<Description> {
    if body.shells().len() != 1 {
        return Err(unsupported(
            "splitting faces works on a solid of one shell (no separate lumps or voids)",
        ));
    }
    let vertices = body.vertices();
    let vindex: BTreeMap<_, usize> = vertices.iter().enumerate().map(|(i, v)| (*v, i)).collect();
    let mut points = Vec::with_capacity(vertices.len());
    let mut vertex_names = Vec::with_capacity(vertices.len());
    for v in &vertices {
        points.push(body.vertex_point(*v)?);
        vertex_names.push(body.vertex_name(*v)?.map(str::to_owned));
    }
    let edge_ids = body.edges();
    let eindex: BTreeMap<_, usize> = edge_ids.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let mut edges = Vec::with_capacity(edge_ids.len());
    let mut edge_names = Vec::with_capacity(edge_ids.len());
    for e in &edge_ids {
        let [a, b] = body.edge_vertices(*e)?;
        let (curve, t0, t1) = body
            .edge_curve(*e)?
            .ok_or_else(|| invalid(format!("{e} has no curve")))?;
        let from = *vindex
            .get(&a)
            .ok_or_else(|| invalid("a vertex is missing"))?;
        let to = *vindex
            .get(&b)
            .ok_or_else(|| invalid("a vertex is missing"))?;
        edges.push((from, to, curve, t0, t1));
        edge_names.push(body.edge_name(*e)?.map(str::to_owned));
    }
    let face_ids = body.faces();
    let mut faces = Vec::with_capacity(face_ids.len());
    let mut face_names = Vec::with_capacity(face_ids.len());
    for f in &face_ids {
        let (surface, same) = body
            .face_surface(*f)?
            .ok_or_else(|| invalid(format!("{f} has no surface")))?;
        let mut loops = Vec::new();
        for lp in body.face_loops(*f)? {
            let mut uses = Vec::new();
            for h in body.loop_half_edges(lp)? {
                let Some(e) = body.half_edge_edge(h)? else {
                    return Err(unsupported("a face bounded by a lone vertex"));
                };
                let ei = *eindex
                    .get(&e)
                    .ok_or_else(|| invalid("an edge is missing"))?;
                let forward = body.edge_half_edges(e)?[0] == h;
                uses.push((ei, forward));
            }
            loops.push(uses);
        }
        faces.push((surface, same, loops));
        face_names.push(body.face_name(*f)?.map(str::to_owned));
    }
    Ok(Description {
        points,
        edges,
        faces,
        face_names,
        edge_names,
        vertex_names,
        face_ids,
    })
}

impl Description {
    /// The vertex a use of an edge starts at.
    fn start(&self, (e, forward): (usize, bool)) -> Option<usize> {
        let (a, b, ..) = self.edges.get(e)?;
        Some(if forward { *a } else { *b })
    }

    /// A vertex at `p`: an existing end of edge `e` when `p` is on it, else a new one.
    fn add_point(&mut self, p: Point3) -> usize {
        self.points.push(p);
        self.vertex_names.push(None);
        self.points.len() - 1
    }

    /// Splits edge `e` at the curve parameters `ts` (strictly inside its range), with a
    /// new vertex at each, and replaces every use of it by the pieces.
    fn split_edge(&mut self, e: usize, ts: &[f64]) -> KernelResult<()> {
        if ts.is_empty() {
            return Ok(());
        }
        let (from, to, curve, t0, t1) = self
            .edges
            .get(e)
            .cloned()
            .ok_or_else(|| invalid("an edge is missing"))?;
        let name = self.edge_names.get(e).cloned().flatten();
        let mut cuts: Vec<f64> = ts.to_vec();
        cuts.sort_by(f64::total_cmp);
        let mut pieces = Vec::with_capacity(cuts.len() + 1);
        let (mut at, mut at_t) = (from, t0);
        for t in cuts {
            let v = self.add_point(curve.point_at(t));
            pieces.push((at, v, at_t, t));
            at = v;
            at_t = t;
        }
        pieces.push((at, to, at_t, t1));
        let mut ids = Vec::with_capacity(pieces.len());
        for (k, (a, b, s0, s1)) in pieces.into_iter().enumerate() {
            curve.check_range(s0, s1)?;
            if k == 0 {
                if let Some(slot) = self.edges.get_mut(e) {
                    *slot = (a, b, curve.clone(), s0, s1);
                }
                ids.push(e);
            } else {
                self.edges.push((a, b, curve.clone(), s0, s1));
                self.edge_names.push(name.clone());
                ids.push(self.edges.len() - 1);
            }
        }
        for face in &mut self.faces {
            for lp in &mut face.2 {
                let mut next = Vec::with_capacity(lp.len() + ids.len());
                for &(x, forward) in lp.iter() {
                    if x != e {
                        next.push((x, forward));
                    } else if forward {
                        next.extend(ids.iter().map(|i| (*i, true)));
                    } else {
                        next.extend(ids.iter().rev().map(|i| (*i, false)));
                    }
                }
                *lp = next;
            }
        }
        Ok(())
    }
}

/// `body` with each of `faces` (flat) split along the line where `plane` meets it. The
/// new edges are named `edge_name`; the pieces behind the plane keep their face's name,
/// and the pieces in front are returned (with that name too, for the caller to rename).
/// Refused: curved faces, a plane parallel to a face or missing it, a plane running along
/// a face's edge, faces with holes cut more than once, and solids of several shells.
pub fn split_faces(
    body: &Body,
    faces: &[FaceId],
    plane: &Frame,
    edge_name: &str,
) -> KernelResult<(Body, Vec<FaceId>)> {
    if faces.is_empty() {
        return Err(invalid("give at least one face to split"));
    }
    body.validate()
        .map_err(|e| invalid(format!("the body is not a valid solid: {e}")))?;
    let mut d = describe(body)?;
    let cut_surface = Surface::Plane(*plane);
    let on_plane = |p: Point3| side(plane, p).abs() <= LINEAR_TOLERANCE;
    // Which description faces to split.
    let mut targets = Vec::new();
    for f in faces {
        let i = d
            .face_ids
            .iter()
            .position(|x| x == f)
            .ok_or_else(|| invalid(format!("{f} is not a face of the body")))?;
        if !targets.contains(&i) {
            targets.push(i);
        }
    }
    // Split every edge of the target faces where it crosses the plane, once per edge.
    let mut done = std::collections::BTreeSet::new();
    for &fi in &targets {
        let (surface, same, _) = d
            .faces
            .get(fi)
            .cloned()
            .ok_or_else(|| invalid("a face is missing"))?;
        let Surface::Plane(fp) = surface else {
            return Err(unsupported(
                "only flat faces can be split by a plane for now",
            ));
        };
        let normal = if same { fp.z() } else { -fp.z() };
        if normal.cross(plane.z()).length() <= ANGULAR_TOLERANCE {
            return Err(invalid(
                "the plane is parallel to the face, so it cannot split it",
            ));
        }
        let edges: Vec<usize> = d
            .faces
            .get(fi)
            .map(|f| f.2.iter().flatten().map(|u| u.0).collect())
            .unwrap_or_default();
        for e in edges {
            if !done.insert(e) {
                continue;
            }
            let Some((from, to, curve, t0, t1)) = d.edges.get(e).cloned() else {
                continue;
            };
            let hits = intersect_curve_surface_on(&curve, t0, t1, &cut_surface)?;
            if let crate::CurveSurfaceIntersection::OnSurface { .. } = hits {
                return Err(invalid(
                    "the plane runs along an edge of the face; there is nothing to split there",
                ));
            }
            let (pa, pb) = (
                d.points.get(from).copied().unwrap_or(Point3::ORIGIN),
                d.points.get(to).copied().unwrap_or(Point3::ORIGIN),
            );
            let mut ts: Vec<f64> = Vec::new();
            for h in hits.points() {
                if h.point.coincident(pa) || h.point.coincident(pb) {
                    continue;
                }
                if h.t <= t0 || h.t >= t1 || ts.iter().any(|t| (t - h.t).abs() <= f64::EPSILON) {
                    continue;
                }
                ts.push(h.t);
            }
            d.split_edge(e, &ts)?;
            // The new pieces need no second look.
            for k in d.edges.len().saturating_sub(ts.len())..d.edges.len() {
                done.insert(k);
            }
        }
    }
    // Trace the chords of each face and split its outer loop with them.
    let mut front = Vec::new();
    let mut new_faces: Vec<(usize, DescFace)> = Vec::new();
    for &fi in &targets {
        let (surface, same, loops) = d
            .faces
            .get(fi)
            .cloned()
            .ok_or_else(|| invalid("a face is missing"))?;
        let Surface::Plane(fp) = surface else {
            continue;
        };
        let normal = if same { fp.z() } else { -fp.z() };
        let dir = normal
            .cross(plane.z())
            .normalized()
            .ok_or_else(|| invalid("the plane is parallel to the face"))?;
        let face_id = *d
            .face_ids
            .get(fi)
            .ok_or_else(|| invalid("a face is missing"))?;
        // Vertices of the face on the plane, along the line, without repeats.
        let mut along: Vec<(f64, usize, usize)> = Vec::new(); // (position, vertex, loop)
        for (li, lp) in loops.iter().enumerate() {
            for u in lp {
                let v = d.start(*u).ok_or_else(|| invalid("an edge is missing"))?;
                let p = d.points.get(v).copied().unwrap_or(Point3::ORIGIN);
                if on_plane(p) && !along.iter().any(|x| x.1 == v) {
                    along.push(((p - plane.origin()).dot(dir), v, li));
                }
            }
        }
        along.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut chords = Vec::new();
        for w in along.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b.0 - a.0 <= LINEAR_TOLERANCE {
                continue;
            }
            let (pa, pb) = (
                d.points.get(a.1).copied().unwrap_or(Point3::ORIGIN),
                d.points.get(b.1).copied().unwrap_or(Point3::ORIGIN),
            );
            let mid = pa + (pb - pa) * 0.5;
            if body.classify_point_on_face(face_id, mid)? == PointOnFace::Inside {
                if a.2 != 0 || b.2 != 0 {
                    return Err(unsupported(
                        "the plane crosses a hole of the face; split faces with holes the plane misses",
                    ));
                }
                chords.push((a.1, b.1));
            }
        }
        if chords.is_empty() {
            return Err(invalid("the plane does not cross the face"));
        }
        if loops.len() > 1 && chords.len() > 1 {
            return Err(unsupported(
                "a face with holes can be split by one cut across it only",
            ));
        }
        let mut pieces: Vec<Vec<(usize, bool)>> = vec![
            loops
                .first()
                .cloned()
                .ok_or_else(|| invalid("a face has no loop"))?,
        ];
        for (a, b) in chords {
            let starts =
                |lp: &Vec<(usize, bool)>, v: usize| lp.iter().position(|u| d.start(*u) == Some(v));
            let Some(k) = pieces
                .iter()
                .position(|lp| starts(lp, a).is_some() && starts(lp, b).is_some())
            else {
                return Err(unsupported(
                    "the cut across the face does not close a piece of it",
                ));
            };
            let lp = pieces.remove(k);
            let ia = starts(&lp, a).unwrap_or(0);
            let ib = starts(&lp, b).unwrap_or(0);
            let n = lp.len();
            let walk = |from: usize, to: usize| -> Vec<(usize, bool)> {
                let mut out = Vec::new();
                let mut i = from;
                for _ in 0..n {
                    if i == to {
                        break;
                    }
                    if let Some(u) = lp.get(i) {
                        out.push(*u);
                    }
                    i = (i + 1) % n;
                }
                out
            };
            let (pa, pb) = (
                d.points.get(a).copied().unwrap_or(Point3::ORIGIN),
                d.points.get(b).copied().unwrap_or(Point3::ORIGIN),
            );
            d.edges
                .push((a, b, Curve::Segment(Segment3::new(pa, pb)?), 0.0, 1.0));
            d.edge_names.push(Some(edge_name.to_owned()));
            let c = d.edges.len() - 1;
            // a ... b, then back along the chord; b ... a, then along the chord.
            let mut one = walk(ia, ib);
            one.push((c, false));
            let mut two = walk(ib, ia);
            two.push((c, true));
            pieces.push(one);
            pieces.push(two);
        }
        // Each piece's side of the plane, from a vertex off it.
        let piece_side = |lp: &Vec<(usize, bool)>| -> f64 {
            lp.iter()
                .filter_map(|u| d.start(*u))
                .filter_map(|v| d.points.get(v).copied())
                .map(|p| side(plane, p))
                .find(|s| s.abs() > LINEAR_TOLERANCE)
                .unwrap_or(0.0)
        };
        let sides: Vec<f64> = pieces.iter().map(piece_side).collect();
        // Holes (one cut only) go with the piece on their side.
        let mut holes: Vec<Vec<Vec<(usize, bool)>>> = vec![Vec::new(); pieces.len()];
        for hole in loops.iter().skip(1) {
            let s = piece_side(hole);
            let k = sides
                .iter()
                .position(|x| (*x > 0.0) == (s > 0.0))
                .unwrap_or(0);
            if let Some(h) = holes.get_mut(k) {
                h.push(hole.clone());
            }
        }
        for (k, (outer, inner)) in pieces.into_iter().zip(holes).enumerate() {
            let mut lps = vec![outer];
            lps.extend(inner);
            let entry = (surface.clone(), same, lps);
            let is_front = sides.get(k).copied().unwrap_or(0.0) > 0.0;
            if k == 0 {
                if let Some(slot) = d.faces.get_mut(fi) {
                    *slot = entry;
                }
                if is_front {
                    front.push(fi);
                }
            } else {
                new_faces.push((fi, entry));
                if is_front {
                    front.push(d.faces.len() + new_faces.len() - 1);
                }
            }
        }
    }
    for (from, f) in new_faces {
        let name = d.face_names.get(from).cloned().flatten();
        d.faces.push(f);
        d.face_names.push(name);
    }
    let mut out = Body::from_boundary(&d.points, &d.edges, &d.faces)
        .map_err(|e| KernelError::InvalidResult(format!("splitting the face: {e}")))?;
    let (fs, es, vs) = (out.faces(), out.edges(), out.vertices());
    if fs.len() != d.faces.len() || es.len() != d.edges.len() || vs.len() != d.points.len() {
        return Err(KernelError::InvalidResult(
            "splitting the face lost an entity".into(),
        ));
    }
    for (f, n) in fs.iter().zip(&d.face_names) {
        if let Some(n) = n {
            out.set_face_name(*f, n)?;
        }
    }
    for (e, n) in es.iter().zip(&d.edge_names) {
        if let Some(n) = n {
            out.set_edge_name(*e, n)?;
        }
    }
    for (v, n) in vs.iter().zip(&d.vertex_names) {
        if let Some(n) = n {
            out.set_vertex_name(*v, n)?;
        }
    }
    let front_ids = front.iter().filter_map(|i| fs.get(*i).copied()).collect();
    Ok((out, front_ids))
}

/// The plane of a flat face with its normal pointing out of the solid.
fn outward_plane(body: &Body, face: FaceId) -> KernelResult<Option<Frame>> {
    Ok(match body.face_surface(face)? {
        Some((Surface::Plane(f), same)) => Some(if same { f } else { f.flipped() }),
        _ => None,
    })
}

/// One edge use as a profile edge in `frame`'s plane.
fn profile_edge(frame: &Frame, curve: &Curve, t0: f64, t1: f64) -> KernelResult<ProfileEdge> {
    let p = |q: Point3| frame.to_plane(q);
    match curve {
        Curve::Segment(_) | Curve::Line(_) => Ok(ProfileEdge::line(
            p(curve.point_at(t0)),
            p(curve.point_at(t1)),
        )),
        Curve::Circle(_) | Curve::Arc(_) => {
            let circle = match curve {
                Curve::Arc(a) => *a.circle(),
                Curve::Circle(c) => *c,
                _ => return Err(unsupported("an edge kind")),
            };
            let dot = circle.normal().dot(frame.z());
            if (dot.abs() - 1.0).abs() > ANGULAR_TOLERANCE {
                return Err(invalid("an arc of the face does not lie in its plane"));
            }
            let centre = p(circle.center());
            let start = p(curve.point_at(t0));
            Ok(ProfileEdge::Arc {
                centre,
                radius: circle.radius(),
                start_angle: (start.y - centre.y).atan2(start.x - centre.x),
                sweep: (t1 - t0) * dot.signum(),
            })
        }
        _ => Err(unsupported(
            "thickening a face bounded by ellipses or splines",
        )),
    }
}

/// A solid made by giving `face` of `body` a thickness: the material between the offsets
/// `lo` and `hi` (mm, `lo < hi`) of the face along its outward normal. So `(0, t)` grows
/// it outward by `t`, `(-t, 0)` inward, `(-t/2, t/2)` symmetrically. Flat faces (lines,
/// arcs and circles, with holes) are extruded; a full cylindrical face between two
/// circles about its axis becomes a ring. Other faces are refused.
pub fn thicken(body: &Body, face: FaceId, lo: f64, hi: f64) -> KernelResult<Body> {
    if !(lo.is_finite() && hi.is_finite()) || hi - lo <= LINEAR_TOLERANCE {
        return Err(invalid("the thickness must be a number above 0"));
    }
    if lo.abs().max(hi.abs()) > MAX_COORDINATE {
        return Err(invalid("the thickness leaves the working range"));
    }
    if let Some(frame) = outward_plane(body, face)? {
        let mut loops = Vec::new();
        for lp in body.face_loops(face)? {
            let mut edges = Vec::new();
            for h in body.loop_half_edges(lp)? {
                let Some(e) = body.half_edge_edge(h)? else {
                    return Err(unsupported("a face bounded by a lone vertex"));
                };
                let (curve, t0, t1) = body
                    .edge_curve(e)?
                    .ok_or_else(|| invalid("an edge has no curve"))?;
                let pe = profile_edge(&frame, &curve, t0, t1)?;
                let forward = body.edge_half_edges(e)?[0] == h;
                edges.push(if forward { pe } else { pe.reversed() });
            }
            loops.push(edges);
        }
        let mut loops = loops.into_iter();
        let outer = loops
            .next()
            .ok_or_else(|| invalid("the face has no loop"))?;
        let profile = Profile::new(outer, loops.collect());
        let start = frame
            .with_origin(frame.origin() + frame.z() * lo)
            .ok_or_else(|| invalid("the thickness leaves the working range"))?;
        return crate::extrude(&start, &profile, hi - lo);
    }
    let Some((Surface::Cylinder(c), same)) = body.face_surface(face)? else {
        return Err(unsupported(
            "thicken works on flat faces and on cylindrical faces for now",
        ));
    };
    let axis = c.frame().z();
    let origin = c.frame().origin();
    let edges = body.face_edges(face)?;
    let mut heights = Vec::new();
    for e in &edges {
        // The seam runs along the axis, with the face on both sides.
        if body.edge_faces(*e)? == [face, face] {
            continue;
        }
        let circle = match body.edge_curve(*e)? {
            Some((Curve::Circle(k), ..)) => k,
            Some((Curve::Arc(a), t0, t1))
                if t1 - t0 >= std::f64::consts::TAU - ANGULAR_TOLERANCE =>
            {
                *a.circle()
            }
            _ => {
                return Err(unsupported(
                    "thicken works on a full cylindrical face between two circles for now",
                ));
            }
        };
        let to_centre = circle.center() - origin;
        let v = to_centre.dot(axis);
        let off_axis = (to_centre - axis * v).length();
        if off_axis > LINEAR_TOLERANCE
            || (circle.normal().dot(axis).abs() - 1.0).abs() > ANGULAR_TOLERANCE
            || (circle.radius() - c.radius()).abs() > LINEAR_TOLERANCE
        {
            return Err(unsupported(
                "thicken works on a cylindrical face cut square at both ends for now",
            ));
        }
        heights.push(v);
    }
    let [v0, v1] = match heights.as_slice() {
        [a, b] => [a.min(*b), a.max(*b)],
        _ => {
            return Err(unsupported(
                "thicken works on a full cylindrical face between two circles for now",
            ));
        }
    };
    if v1 - v0 <= LINEAR_TOLERANCE {
        return Err(invalid("the cylindrical face has no height"));
    }
    // The face's outward normal points away from the axis on a boss, toward it in a hole.
    let sign = if same { 1.0 } else { -1.0 };
    let (ra, rb) = (c.radius() + sign * lo, c.radius() + sign * hi);
    let (inner, outer) = (ra.min(rb), ra.max(rb));
    if inner <= LINEAR_TOLERANCE {
        return Err(invalid(
            "the thickness reaches the cylinder's axis; make it thinner",
        ));
    }
    let base = c
        .frame()
        .with_origin(origin + axis * v0)
        .ok_or_else(|| invalid("the cylinder leaves the working range"))?;
    let centre = chamfera_geom::Point2::new(0.0, 0.0);
    let profile = Profile::new(
        vec![ProfileEdge::circle(centre, outer)],
        vec![vec![ProfileEdge::circle(centre, inner)]],
    );
    crate::extrude(&base, &profile, v1 - v0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Vec3;
    use std::f64::consts::PI;

    fn block() -> Body {
        // The 50 x 30 x 10 box from the origin.
        Body::cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0)).unwrap()
    }

    fn named(body: &Body, name: &str) -> FaceId {
        body.faces()
            .into_iter()
            .find(|f| body.face_name(*f).unwrap() == Some(name))
            .unwrap()
    }

    fn top(body: &Body) -> FaceId {
        // The face whose outward normal is +z at z = 10.
        body.faces()
            .into_iter()
            .find(|f| {
                outward_plane(body, *f).unwrap().is_some_and(|p| {
                    p.z().dot(Vec3::Z) > 0.99 && (p.origin().z - 10.0).abs() < 1e-9
                })
            })
            .unwrap()
    }

    fn at_x(x: f64) -> Frame {
        Frame::YZ.with_origin(Point3::new(x, 0.0, 0.0)).unwrap()
    }

    #[test]
    fn splitting_the_box_at_x_20_gives_6000_and_9000() {
        let b = block();
        let (back, front) = split_body(&b, &at_x(20.0), "split").unwrap();
        // 20 * 30 * 10 behind the plane, 30 * 30 * 10 in front.
        assert!((back.volume().unwrap() - 6000.0).abs() < 1e-6);
        assert!((front.volume().unwrap() - 9000.0).abs() < 1e-6);
        // The area grows by the two new 30 x 10 faces.
        let before = b.area().unwrap();
        let after = back.area().unwrap() + front.area().unwrap();
        assert!((after - before - 2.0 * 30.0 * 10.0).abs() < 1e-6);
        let a = back.face_area(named(&back, "split")).unwrap();
        assert!((a - 300.0).abs() < 1e-6);
        assert!((front.face_area(named(&front, "split")).unwrap() - 300.0).abs() < 1e-6);
    }

    #[test]
    fn a_plane_that_misses_or_only_touches_the_box_is_refused() {
        let b = block();
        assert!(matches!(
            split_body(&b, &at_x(80.0), "split"),
            Err(KernelError::InvalidInput(_))
        ));
        assert!(matches!(
            split_body(&b, &at_x(0.0), "split"),
            Err(KernelError::InvalidInput(_))
        ));
    }

    #[test]
    fn splitting_the_top_face_gives_seven_faces_and_the_same_volume() {
        let b = block();
        let t = top(&b);
        let (out, front) = split_faces(&b, &[t], &at_x(20.0), "split").unwrap();
        assert_eq!(out.faces().len(), 7);
        // V - E + F = 2: the box's 8, 12, 6 plus 2 vertices, 3 edges (two halves of
        // the cut edges and the chord) and 1 face.
        assert_eq!(out.vertices().len(), 10);
        assert_eq!(out.edges().len(), 15);
        out.validate().unwrap();
        assert!((out.volume().unwrap() - 15000.0).abs() < 1e-6);
        assert!((out.area().unwrap() - b.area().unwrap()).abs() < 1e-6);
        // The piece in front (x > 20) is 30 x 30.
        assert_eq!(front.len(), 1);
        assert!((out.face_area(front[0]).unwrap() - 900.0).abs() < 1e-6);
    }

    #[test]
    fn split_faces_refuses_parallel_missing_and_curved_cases() {
        let b = block();
        let t = top(&b);
        let flat = Frame::XY.with_origin(Point3::new(0.0, 0.0, 5.0)).unwrap();
        assert!(split_faces(&b, &[t], &flat, "s").is_err());
        assert!(split_faces(&b, &[t], &at_x(70.0), "s").is_err());
        assert!(split_faces(&b, &[], &at_x(20.0), "s").is_err());
        let c = crate::extrude(
            &Frame::XY,
            &Profile::new(
                vec![ProfileEdge::circle(
                    chamfera_geom::Point2::new(0.0, 0.0),
                    10.0,
                )],
                vec![],
            ),
            20.0,
        )
        .unwrap();
        let side = named(&c, "side 0");
        assert!(matches!(
            split_faces(&c, &[side], &at_x(0.0), "s"),
            Err(KernelError::Unsupported(_))
        ));
    }

    #[test]
    fn a_face_with_a_hole_is_split_across_beside_the_hole() {
        // A 40 x 20 plate 5 thick with a hole r = 3 at (30, 10), split at x = 10: the
        // hole goes with the front piece; volume 40*20*5 - 9π*5.
        let outer = polygon(&[(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)]);
        let hole = vec![ProfileEdge::circle(
            chamfera_geom::Point2::new(30.0, 10.0),
            3.0,
        )];
        let plate = crate::extrude(&Frame::XY, &Profile::new(outer, vec![hole]), 5.0).unwrap();
        let t = named(&plate, "top");
        let (out, front) = split_faces(&plate, &[t], &at_x(10.0), "s").unwrap();
        let v = 4000.0 - 9.0 * PI * 5.0;
        assert!((out.volume().unwrap() - v).abs() < 1e-6);
        // The front piece: 30 x 20 less the hole.
        assert!((out.face_area(front[0]).unwrap() - (600.0 - 9.0 * PI)).abs() < 1e-6);
    }

    fn polygon(pts: &[(f64, f64)]) -> Vec<ProfileEdge> {
        (0..pts.len())
            .map(|k| {
                let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                ProfileEdge::line(
                    chamfera_geom::Point2::new(a.0, a.1),
                    chamfera_geom::Point2::new(b.0, b.1),
                )
            })
            .collect()
    }

    #[test]
    fn thickening_the_top_face_by_3_gives_4500() {
        let b = block();
        let t = top(&b);
        // 50 * 30 * 3 above the top, from z = 10 to 13.
        let out = thicken(&b, t, 0.0, 3.0).unwrap();
        assert!((out.volume().unwrap() - 4500.0).abs() < 1e-6);
        let bb = out.bounding_box().unwrap();
        assert!((bb.min.z - 10.0).abs() < 1e-9 && (bb.max.z - 13.0).abs() < 1e-9);
        let sym = thicken(&b, t, -1.5, 1.5).unwrap();
        assert!((sym.volume().unwrap() - 4500.0).abs() < 1e-6);
    }

    #[test]
    fn thickening_a_cylinder_face_by_2_outward_gives_a_ring() {
        let c = crate::extrude(
            &Frame::XY,
            &Profile::new(
                vec![ProfileEdge::circle(
                    chamfera_geom::Point2::new(0.0, 0.0),
                    10.0,
                )],
                vec![],
            ),
            20.0,
        )
        .unwrap();
        let side = named(&c, "side 0");
        let ring = thicken(&c, side, 0.0, 2.0).unwrap();
        // π (12² − 10²) · 20.
        let v = PI * (144.0 - 100.0) * 20.0;
        assert!((ring.volume().unwrap() - v).abs() < 1e-6);
        // Inward by 2: π (10² − 8²) · 20; to the axis: refused.
        let inner = thicken(&c, side, -2.0, 0.0).unwrap();
        assert!((inner.volume().unwrap() - PI * 36.0 * 20.0).abs() < 1e-6);
        assert!(thicken(&c, side, -10.0, 0.0).is_err());
    }

    #[test]
    fn thicken_refuses_hostile_sizes_and_other_surfaces() {
        let b = block();
        let t = top(&b);
        assert!(thicken(&b, t, 0.0, 0.0).is_err());
        assert!(thicken(&b, t, 0.0, f64::NAN).is_err());
        assert!(thicken(&b, t, 0.0, 1e300).is_err());
        let s = crate::revolve(
            &Frame::XZ,
            &Profile::new(
                vec![
                    ProfileEdge::Arc {
                        centre: chamfera_geom::Point2::new(0.0, 0.0),
                        radius: 5.0,
                        start_angle: -PI / 2.0,
                        sweep: PI,
                    },
                    ProfileEdge::line(
                        chamfera_geom::Point2::new(0.0, 5.0),
                        chamfera_geom::Point2::new(0.0, -5.0),
                    ),
                ],
                vec![],
            ),
            &crate::Line2::new(
                chamfera_geom::Point2::new(0.0, 0.0),
                chamfera_geom::Vec2::new(0.0, 1.0),
            )
            .unwrap(),
            2.0 * PI,
        );
        if let Ok(sphere) = s {
            for f in sphere.faces() {
                if let Some((Surface::Sphere(_), _)) = sphere.face_surface(f).unwrap() {
                    assert!(matches!(
                        thicken(&sphere, f, 0.0, 1.0),
                        Err(KernelError::Unsupported(_))
                    ));
                }
            }
        }
    }
}

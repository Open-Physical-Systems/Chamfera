//! The second half of the boolean: classify each face piece against the other solid,
//! keep the pieces the operation needs, and sew them into closed shells.

use super::registry::{EdgeRec, Registry};
use super::span::{Carrier, Span, internal};
use super::split::{FacePiece, Loaded};
use super::trace::{Side, Use, interior_points, regions, start_vertex, trace};
use super::{BooleanOp, Operand as Which};
use crate::{
    Body, KernelError, KernelResult, ListEdge, ListFace, ListVertex, PointInSolid, PointOnFace,
    classify_point,
};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE};
use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

/// Length (mm) of the chord into each face that orders faces tangent along an edge: long
/// enough that a face bending with radius up to 1e3 mm leaves its tangent plane by
/// `c² / 2R` = 5e-12, far above rounding, short enough to stay near the edge.
const PAIR_CHORD: f64 = 1e-4;

/// Where a face piece lies relative to the other solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    In,
    Out,
    /// On the other solid's boundary, with the same outward normal.
    OnSame,
    /// On the other solid's boundary, with the opposite outward normal.
    OnOpposite,
}

/// Classifies `piece` (of operand `own`) against `other` by interior sample points: ray
/// casting decides inside and outside; a point on the other boundary is matched with a
/// coincident face there and the outward normals compared. A sample that cannot be
/// decided (it lands on an edge of the other solid, or every ray is degenerate) is
/// replaced by the next one.
pub(super) fn classify(
    piece: &FacePiece,
    own: &Loaded<'_>,
    other: &Loaded<'_>,
    partners: &[usize],
) -> KernelResult<Class> {
    let side = own.face(piece.face)?.side.clone();
    for p in interior_points(&piece.region, &side) {
        match classify_point(other.body, p) {
            Ok(PointInSolid::Inside) => return Ok(Class::In),
            Ok(PointInSolid::Outside) => return Ok(Class::Out),
            Ok(PointInSolid::OnBoundary) => {
                for &g in partners {
                    let gf = other.face(g)?;
                    if other.body.classify_point_on_face(gf.id, p)? != PointOnFace::Outside {
                        let same = side.normal(p).dot(gf.side.normal(p)) > 0.0;
                        return Ok(if same {
                            Class::OnSame
                        } else {
                            Class::OnOpposite
                        });
                    }
                }
            }
            Err(_) => {}
        }
    }
    Err(KernelError::NoConvergence(format!(
        "no interior point of a piece of face `{}` could be classified against the other solid",
        own.face(piece.face)?.name.as_deref().unwrap_or("(unnamed)")
    )))
}

/// Whether a piece of operand `which` with class `c` is kept, and reversed.
pub(super) fn select(op: BooleanOp, which: Which, c: Class) -> Option<bool> {
    use BooleanOp as O;
    use Class as C;
    match (op, which, c) {
        (O::Union, Which::A, C::Out | C::OnSame) | (O::Union, Which::B, C::Out) => Some(false),
        (O::Intersect, Which::A, C::In | C::OnSame) | (O::Intersect, Which::B, C::In) => {
            Some(false)
        }
        (O::Subtract, Which::A, C::Out | C::OnOpposite) => Some(false),
        (O::Subtract, Which::B, C::In) => Some(true),
        _ => None,
    }
}

/// A face of the result being sewn.
#[derive(Debug, Clone)]
pub(super) struct OutFace {
    pub side: Side,
    pub name: Option<String>,
    pub loops: Vec<Vec<Use>>,
    /// The operand face it comes from (for merging pieces of one curved face).
    pub origin: (Which, usize),
    /// Registry edges that were cuts of the origin face.
    pub cut_edges: HashSet<usize>,
}

impl OutFace {
    pub fn new(piece: FacePiece, side: Side, name: Option<String>, reverse: bool) -> OutFace {
        let mut loops = piece.region.loops;
        let mut side = side;
        if reverse {
            for l in &mut loops {
                l.reverse();
                for u in l.iter_mut() {
                    u.1 = !u.1;
                }
            }
            side.orient = -side.orient;
        }
        OutFace {
            side,
            name,
            loops,
            origin: (piece.which, piece.face),
            cut_edges: piece.cut_edges,
        }
    }
}

/// Position of a use: (face, loop, index).
type At = (usize, usize, usize);

fn uses_of(faces: &[OutFace]) -> HashMap<usize, Vec<(At, bool)>> {
    let mut m: HashMap<usize, Vec<(At, bool)>> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for (li, l) in f.loops.iter().enumerate() {
            for (k, u) in l.iter().enumerate() {
                m.entry(u.0).or_default().push(((fi, li, k), u.1));
            }
        }
    }
    m
}

fn not_closed(e: usize, n: usize) -> KernelError {
    KernelError::InvalidResult(format!(
        "the kept faces do not close: an edge (#{e}) is used {n} times, which needs two uses in opposite directions"
    ))
}

/// Gives every edge exactly two uses: an edge where more than two kept faces meet (two
/// solids touching along it) is split into one edge per pair of faces that bound the same
/// wedge of material, found by sorting the faces around the edge.
fn pair_edges(faces: &mut [OutFace], reg: &mut Registry, open: bool) -> KernelResult<()> {
    let mut m: Vec<(usize, Vec<(At, bool)>)> = uses_of(faces).into_iter().collect();
    m.sort_by_key(|x| x.0);
    for (e, us) in m {
        let n = us.len();
        if open && n == 1 {
            // A rim edge of a surface.
            continue;
        }
        if n == 2 {
            if us[0].1 == us[1].1 {
                return Err(not_closed(e, n));
            }
            continue;
        }
        if n % 2 == 1 || n == 0 {
            return Err(not_closed(e, n));
        }
        let span = reg.edge(e)?.span.clone();
        let tm = 0.5 * (span.t0 + span.t1);
        let mid = span.carrier.point(tm);
        let t = span
            .carrier
            .d1(tm)
            .normalized()
            .ok_or_else(|| internal("an edge has no direction"))?;
        let mut around = Vec::with_capacity(n);
        for (at, d) in &us {
            let f = faces
                .get(at.0)
                .ok_or_else(|| internal("a face went missing"))?;
            let normal = f.side.normal(mid);
            let along = if *d { t } else { -t };
            let w = normal.cross(along);
            // Faces tangent along the edge (a spline side touching a plane along a
            // ruling) leave with the same `w`; a short chord into each face's surface,
            // which bends away by its curvature, orders them.
            let chord = match w.normalized() {
                Some(wu) => {
                    let q = mid + wu * PAIR_CHORD;
                    let p = f.side.surface.closest_point(q).map_or(q, |c| c.point);
                    let c = p - mid;
                    c - t * c.dot(t)
                }
                None => w,
            };
            around.push((*at, *d, w, normal, chord));
        }
        let e1 = around
            .first()
            .and_then(|a| a.2.normalized())
            .ok_or_else(|| internal("a face meets an edge edge-on"))?;
        let e2 = t.cross(e1);
        around.sort_by(|a, b| {
            let ta = a.4.dot(e2).atan2(a.4.dot(e1));
            let tb = b.4.dot(e2).atan2(b.4.dot(e1));
            ta.total_cmp(&tb)
        });
        let mut pairs = Vec::new();
        let mut used = vec![false; n];
        for i in 0..n {
            let j = (i + 1) % n;
            if used[i] || used[j] {
                continue;
            }
            let (_, di, wi, ni, _) = around[i];
            let (_, dj, _, _, _) = around[j];
            if t.cross(wi).dot(-ni) > 0.0 && di != dj {
                used[i] = true;
                used[j] = true;
                pairs.push((around[i].0, around[j].0));
            }
        }
        if pairs.len() * 2 != n {
            return Err(KernelError::Unsupported(format!(
                "{n} faces of the result meet at one edge in a way that cannot be paired into manifold edges"
            )));
        }
        for (k, (a, b)) in pairs.into_iter().enumerate() {
            if k == 0 {
                continue;
            }
            let ne = reg.duplicate_edge(e)?;
            for at in [a, b] {
                if let Some(u) = faces
                    .get_mut(at.0)
                    .and_then(|f| f.loops.get_mut(at.1))
                    .and_then(|l| l.get_mut(at.2))
                {
                    u.0 = ne;
                }
            }
        }
    }
    Ok(())
}

/// Union-find root.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    for _ in 0..parent.len() {
        if parent[i] == i {
            break;
        }
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Whether two kept faces that share an edge are one face: planes that coincide with the
/// same outward normal, or pieces of one curved operand face.
fn mergeable(a: &OutFace, b: &OutFace) -> bool {
    use crate::Surface;
    match (&a.side.surface, &b.side.surface) {
        (Surface::Plane(fa), Surface::Plane(fb)) => {
            let na = fa.z() * a.side.orient;
            let nb = fb.z() * b.side.orient;
            na.cross(nb).length() <= ANGULAR_TOLERANCE
                && na.dot(nb) > 0.0
                && (fb.origin() - fa.origin()).dot(fa.z()).abs() <= LINEAR_TOLERANCE
        }
        (Surface::Plane(_), _) | (_, Surface::Plane(_)) => false,
        _ => a.origin == b.origin && a.side.orient == b.side.orient,
    }
}

/// Merges neighbouring kept faces that are one face (see [`mergeable`]): the edges
/// between them are removed and the rest traced again into loops.
fn merge_faces(faces: Vec<OutFace>, reg: &Registry) -> KernelResult<Vec<OutFace>> {
    let m = uses_of(&faces);
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut edges: Vec<_> = m.iter().collect();
    edges.sort_by_key(|x| *x.0);
    for (_, us) in &edges {
        if let [(a, _), (b, _)] = us.as_slice()
            && a.0 != b.0
            && let (Some(fa), Some(fb)) = (faces.get(a.0), faces.get(b.0))
            && mergeable(fa, fb)
        {
            let (ra, rb) = (root(&mut parent, a.0), root(&mut parent, b.0));
            if ra != rb {
                parent[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..faces.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut keys: Vec<usize> = groups.keys().copied().collect();
    keys.sort_unstable();
    let mut out = Vec::new();
    for k in keys {
        let members = groups.get(&k).cloned().unwrap_or_default();
        let Some(first) = members.first().and_then(|i| faces.get(*i)) else {
            continue;
        };
        if members.len() == 1 {
            out.push(first.clone());
            continue;
        }
        let planar = matches!(first.side.surface, crate::Surface::Plane(_));
        let all: Vec<Use> = members
            .iter()
            .filter_map(|i| faces.get(*i))
            .flat_map(|f| f.loops.iter().flatten().copied())
            .collect();
        let present: HashSet<Use> = all.iter().copied().collect();
        let keep: Vec<Use> = all
            .iter()
            .copied()
            .filter(|u| {
                let inner = present.contains(&(u.0, !u.1));
                !(inner && (planar || first.cut_edges.contains(&u.0)))
            })
            .collect();
        let cycles = trace(reg, &first.side, &keep, &HashSet::new())?;
        for region in regions(reg, &first.side, cycles)? {
            out.push(OutFace {
                side: first.side.clone(),
                name: first.name.clone(),
                loops: region.loops,
                origin: first.origin,
                cut_edges: first.cut_edges.clone(),
            });
        }
    }
    Ok(out)
}

/// Joins pairs of edges on one carrier that meet at a vertex no other edge reaches (left
/// over where a face was split and merged again).
fn join_edges(faces: &mut [OutFace], reg: &mut Registry) -> KernelResult<()> {
    let limit = reg.edges.len() + 1;
    for _ in 0..limit {
        let m = uses_of(faces);
        let mut ends: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut ids: Vec<usize> = m.keys().copied().collect();
        ids.sort_unstable();
        for e in &ids {
            let r = reg.edge(*e)?;
            ends.entry(r.v[0]).or_default().push(*e);
            ends.entry(r.v[1]).or_default().push(*e);
        }
        let mut vs: Vec<usize> = ends.keys().copied().collect();
        vs.sort_unstable();
        let mut done = false;
        for v in vs {
            let Some(es) = ends.get(&v) else { continue };
            let [e1, e2] = es.as_slice() else { continue };
            let (e1, e2) = (*e1, *e2);
            if e1 == e2 {
                continue;
            }
            if let Some(joined) = try_join(reg, &m, v, e1, e2)? {
                let ne = reg.new_edge(joined)?;
                rewrite_joined(faces, e1, e2, ne);
                done = true;
                break;
            }
        }
        if !done {
            return Ok(());
        }
    }
    Ok(())
}

/// The joined edge of `e1` and `e2` at `v`, if they lie on one carrier and bound the same
/// two faces.
fn try_join(
    reg: &Registry,
    m: &HashMap<usize, Vec<(At, bool)>>,
    v: usize,
    e1: usize,
    e2: usize,
) -> KernelResult<Option<EdgeRec>> {
    let (r1, r2) = (reg.edge(e1)?, reg.edge(e2)?);
    if r1.v[0] == r1.v[1] || r2.v[0] == r2.v[1] {
        return Ok(None);
    }
    let face_set = |e: usize| -> Vec<usize> {
        let mut f: Vec<usize> = m
            .get(&e)
            .map(|u| u.iter().map(|x| x.0.0).collect())
            .unwrap_or_default();
        f.sort_unstable();
        f
    };
    let fs = face_set(e1);
    if fs != face_set(e2) || fs.len() != 2 || fs[0] == fs[1] {
        return Ok(None);
    }
    if !r1.span.same_carrier(&r2.span)? {
        return Ok(None);
    }
    let a = if r1.v[0] == v { r1.v[1] } else { r1.v[0] };
    let c = if r2.v[0] == v { r2.v[1] } else { r2.v[0] };
    let s = r1.span.clone();
    let delta = r2.span.t1 - r2.span.t0;
    let (span, verts) = match &s.carrier {
        // Open carriers: the far end's parameter by projection (a spline's pieces may
        // come from different copies of one curve, so their parameters are not shared).
        Carrier::Line(_) | Carrier::Spline(_) => {
            let (tc, _) = s.carrier.project(reg.point(c)?)?;
            if r1.v[1] == v {
                (Span::new(s.carrier.clone(), s.t0, tc), [a, c])
            } else {
                (Span::new(s.carrier.clone(), tc, s.t1), [c, a])
            }
        }
        _ => {
            if r1.v[1] == v {
                (Span::new(s.carrier.clone(), s.t0, s.t1 + delta), [a, c])
            } else {
                (Span::new(s.carrier.clone(), s.t0 - delta, s.t1), [c, a])
            }
        }
    };
    if span.t1 <= span.t0
        || span.t1.is_nan()
        || (span.carrier.periodic() && span.t1 - span.t0 > TAU + 1e-9)
    {
        return Ok(None);
    }
    // The joined span must end at the far vertices.
    if !(span.start().coincident(reg.point(verts[0])?)
        && span.end().coincident(reg.point(verts[1])?))
    {
        return Ok(None);
    }
    Ok(Some(EdgeRec {
        span,
        v: verts,
        name: r1.name.clone().or_else(|| r2.name.clone()),
    }))
}

/// Replaces each consecutive pair of uses of `e1` and `e2` by one use of `ne` (which
/// runs the way `e1` does).
fn rewrite_joined(faces: &mut [OutFace], e1: usize, e2: usize, ne: usize) {
    for f in faces {
        for l in &mut f.loops {
            let n = l.len();
            let Some(k) = l.iter().position(|u| u.0 == e1) else {
                continue;
            };
            let d = l[k].1;
            let next = (k + 1) % n;
            let prev = (k + n - 1) % n;
            let other = if l.get(next).is_some_and(|u| u.0 == e2) {
                next
            } else if l.get(prev).is_some_and(|u| u.0 == e2) {
                prev
            } else {
                continue;
            };
            l[k] = (ne, d);
            if other != k {
                l.remove(other);
            }
        }
    }
}

/// Gives each fan of faces around a vertex its own vertex: solids that touch at a point
/// or along an edge share no vertex in the result.
fn split_fans(faces: &[OutFace], reg: &mut Registry) -> KernelResult<()> {
    // Outgoing uses per vertex, and the rotation h -> mate(prev(h)).
    let mut pos: HashMap<Use, At> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for (li, l) in f.loops.iter().enumerate() {
            for (k, u) in l.iter().enumerate() {
                pos.insert(*u, (fi, li, k));
            }
        }
    }
    let prev_of = |at: At| -> Option<Use> {
        let l = faces.get(at.0)?.loops.get(at.1)?;
        let n = l.len();
        l.get((at.2 + n - 1) % n).copied()
    };
    let mut by_vertex: HashMap<usize, Vec<Use>> = HashMap::new();
    let mut all: Vec<Use> = pos.keys().copied().collect();
    all.sort_unstable();
    for u in &all {
        by_vertex
            .entry(start_vertex(reg, *u)?)
            .or_default()
            .push(*u);
    }
    let mut vs: Vec<usize> = by_vertex.keys().copied().collect();
    vs.sort_unstable();
    for v in vs {
        let outs = by_vertex.get(&v).cloned().unwrap_or_default();
        let mut orbit: HashMap<Use, usize> = HashMap::new();
        let mut count = 0usize;
        for &h0 in &outs {
            if orbit.contains_key(&h0) {
                continue;
            }
            let mut h = h0;
            for _ in 0..=outs.len() {
                orbit.insert(h, count);
                let at = pos
                    .get(&h)
                    .copied()
                    .ok_or_else(|| internal("a use went missing"))?;
                let p = prev_of(at).ok_or_else(|| internal("a loop went missing"))?;
                let mate = (p.0, !p.1);
                if mate == h0 || orbit.contains_key(&mate) {
                    break;
                }
                h = mate;
            }
            count += 1;
        }
        if count <= 1 {
            continue;
        }
        let mut copies = vec![v];
        for _ in 1..count {
            copies.push(reg.duplicate_vertex(v)?);
        }
        for (u, k) in orbit {
            let w = copies[k];
            if let Some(r) = reg.edges.get_mut(u.0) {
                if u.1 {
                    r.v[0] = w;
                } else {
                    r.v[1] = w;
                }
            }
        }
    }
    Ok(())
}

/// Sews the kept faces into a body: pairs edges, merges faces that are one, joins split
/// edges, separates touching solids, and assembles and validates the result.
pub(super) fn sew(faces: Vec<OutFace>, reg: Registry) -> KernelResult<Body> {
    sew_with(faces, reg, false)
}

/// [`sew`] for a surface: edges used once stay as the rim of an open shell, and fans are
/// not separated (sheets touching at a point are refused when the body is assembled).
pub(super) fn sew_open(faces: Vec<OutFace>, reg: Registry) -> KernelResult<Body> {
    sew_with(faces, reg, true)
}

fn sew_with(faces: Vec<OutFace>, mut reg: Registry, open: bool) -> KernelResult<Body> {
    let mut faces = faces;
    pair_edges(&mut faces, &mut reg, open)?;
    let mut faces = merge_faces(faces, &reg)?;
    join_edges(&mut faces, &mut reg)?;
    if !open {
        split_fans(&faces, &mut reg)?;
    }
    // Compact the registry into a face list.
    let mut vmap: HashMap<usize, usize> = HashMap::new();
    let mut emap: HashMap<usize, usize> = HashMap::new();
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    let mut list = Vec::with_capacity(faces.len());
    for f in &faces {
        let mut loops = Vec::with_capacity(f.loops.len());
        for l in &f.loops {
            let mut lp = Vec::with_capacity(l.len());
            for &(e, d) in l {
                let id = match emap.get(&e) {
                    Some(id) => *id,
                    None => {
                        let r = reg.edge(e)?.clone();
                        let mut vv = [0usize; 2];
                        for (k, v) in r.v.iter().enumerate() {
                            vv[k] = match vmap.get(v) {
                                Some(i) => *i,
                                None => {
                                    vertices.push(ListVertex {
                                        point: reg.point(*v)?,
                                        name: reg.vertex_names.get(*v).cloned().flatten(),
                                    });
                                    vmap.insert(*v, vertices.len() - 1);
                                    vertices.len() - 1
                                }
                            };
                        }
                        let (curve, t0, t1) =
                            r.span.edge_curve(reg.point(r.v[0])?, reg.point(r.v[1])?)?;
                        edges.push(ListEdge {
                            vertices: vv,
                            curve,
                            t0,
                            t1,
                            name: r.name.clone(),
                        });
                        emap.insert(e, edges.len() - 1);
                        edges.len() - 1
                    }
                };
                lp.push((id, d));
            }
            loops.push(lp);
        }
        list.push(ListFace {
            surface: f.side.surface.clone(),
            same_sense: f.side.orient > 0.0,
            name: f.name.clone(),
            loops,
        });
    }
    if open {
        let body = Body::from_face_list_open(&vertices, &edges, &list)?;
        let mode = if body.is_open() {
            crate::topology::Validation::Surface
        } else {
            crate::topology::Validation::Solid
        };
        body.validate_as(mode).map_err(|e| {
            KernelError::InvalidResult(format!("the trimmed surface is not valid: {e}"))
        })?;
        return Ok(body);
    }
    let body = Body::from_face_list(&vertices, &edges, &list)?;
    body.validate().map_err(|e| {
        KernelError::InvalidResult(format!("the boolean's result is not a valid solid: {e}"))
    })?;
    Ok(body)
}

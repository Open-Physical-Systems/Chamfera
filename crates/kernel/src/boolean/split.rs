//! The first half of the boolean: load both operands into the shared registry, intersect
//! every pair of faces, trim the intersection curves to both faces, split edges at every
//! new vertex, and split each face into pieces along its cuts.

use super::registry::Registry;
use super::span::{Carrier, Span, crossings, internal};
use super::trace::{Region, Side, Use, regions, trace};
use super::{MAX_BOOLEAN_ENTITIES, Operand as Which};
use crate::{
    Body, FaceId, KernelError, KernelResult, PointOnFace, Surface, SurfaceIntersection,
    intersect_surfaces,
};
use chamfera_geom::{Aabb, LINEAR_TOLERANCE, Point3, Vec3};
use std::collections::{HashMap, HashSet};
use std::f64::consts::TAU;

/// A face of an operand.
#[derive(Debug, Clone)]
pub(super) struct InFace {
    pub id: FaceId,
    pub side: Side,
    pub name: Option<String>,
    pub bbox: Aabb,
    /// Loops as `(operand edge index, runs along the edge)`.
    pub loops: Vec<Vec<(usize, bool)>>,
}

/// An edge of an operand.
#[derive(Debug, Clone)]
pub(super) struct InEdge {
    pub span: Span,
    pub v: [usize; 2],
    /// The operand faces on either side (the same face for a seam).
    pub faces: [usize; 2],
    pub name: Option<String>,
    /// Its registry pieces in its own direction.
    pub pieces: Vec<Use>,
}

/// One operand of the boolean.
#[derive(Debug)]
pub(super) struct Loaded<'a> {
    pub which: Which,
    pub body: &'a Body,
    pub faces: Vec<InFace>,
    pub edges: Vec<InEdge>,
}

impl Loaded<'_> {
    pub fn face(&self, i: usize) -> KernelResult<&InFace> {
        self.faces
            .get(i)
            .ok_or_else(|| internal(format!("operand face {i} is missing")))
    }
    fn edge(&self, i: usize) -> KernelResult<&InEdge> {
        self.edges
            .get(i)
            .ok_or_else(|| internal(format!("operand edge {i} is missing")))
    }

    /// The surface on the other side of edge `e` from face `f` (`None` for a seam).
    fn neighbour(&self, f: usize, e: usize) -> KernelResult<Option<Surface>> {
        let ed = self.edge(e)?;
        let other = if ed.faces[0] == f {
            ed.faces[1]
        } else {
            ed.faces[0]
        };
        if other == f {
            return Ok(None);
        }
        Ok(Some(self.face(other)?.side.surface))
    }

    /// The distinct operand edges of face `f`.
    fn face_edges(&self, f: usize) -> KernelResult<Vec<usize>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for lp in &self.face(f)?.loops {
            for &(e, _) in lp {
                if seen.insert(e) {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    /// Where `p` lies on face `f`.
    fn on_face(&self, f: usize, p: Point3) -> KernelResult<PointOnFace> {
        self.body.classify_point_on_face(self.face(f)?.id, p)
    }
}

/// How far a face can bulge beyond the box of its edges: nothing for a plane (a planar
/// region lies in the convex hull of its boundary); on a cylinder or cone a face point
/// is at most `2 ρ_max` from a boundary point at the same height (heights of a face's
/// points are within its boundary's range); spheres and tori use their whole box.
fn face_box(body: &Body, f: FaceId, surface: &Surface) -> KernelResult<Aabb> {
    let mut b: Option<Aabb> = None;
    let mut rho_max: f64 = 0.0;
    for e in body.face_edges(f)? {
        let (c, t0, t1) = body
            .edge_curve(e)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve")))?;
        let eb = c.bounding_box_on(t0, t1)?;
        if surface.period_u().is_some() {
            for p in [eb.min, eb.max, c.point_at(t0), c.point_at(t1)] {
                let l = surface.frame().to_local(p);
                rho_max = rho_max.max(l.x.hypot(l.y));
            }
        }
        b = Some(b.map_or(eb, |o| o.union(eb)));
    }
    let mut b = b.ok_or_else(|| KernelError::InvalidInput(format!("{f} has no edges")))?;
    let grow = |b: Aabb, d: f64| Aabb {
        min: b.min + Vec3::new(-d, -d, -d),
        max: b.max + Vec3::new(d, d, d),
    };
    match surface {
        Surface::Plane(_) => {}
        Surface::Cylinder(c) => b = grow(b, 2.0 * c.radius()),
        Surface::Cone(_) => b = grow(b, 2.0 * rho_max),
        Surface::Sphere(s) => {
            let r = s.radius();
            let o = s.frame().origin();
            b = b.union(Aabb {
                min: o + Vec3::new(-r, -r, -r),
                max: o + Vec3::new(r, r, r),
            });
        }
        Surface::Torus(t) => {
            let r = t.major_radius().abs() + t.minor_radius();
            let o = t.frame().origin();
            b = b.union(Aabb {
                min: o + Vec3::new(-r, -r, -r),
                max: o + Vec3::new(r, r, r),
            });
        }
    }
    Ok(grow(b, LINEAR_TOLERANCE))
}

fn boxes_meet(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x
        && b.min.x <= a.max.x
        && a.min.y <= b.max.y
        && b.min.y <= a.max.y
        && a.min.z <= b.max.z
        && b.min.z <= a.max.z
}

/// Loads `body` and registers its vertices.
pub(super) fn load<'a>(
    body: &'a Body,
    which: Which,
    reg: &mut Registry,
) -> KernelResult<Loaded<'a>> {
    let mut vmap = HashMap::new();
    for v in body.vertices() {
        let id = reg.vertex(body.vertex_point(v)?, body.vertex_name(v)?)?;
        vmap.insert(v, id);
    }
    let face_ids = body.faces();
    let fmap: HashMap<FaceId, usize> = face_ids.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let edge_ids = body.edges();
    let emap: HashMap<_, usize> = edge_ids.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let mut edges = Vec::with_capacity(edge_ids.len());
    for e in &edge_ids {
        let (c, t0, t1) = body
            .edge_curve(*e)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve")))?;
        let [v0, v1] = body.edge_vertices(*e)?;
        let [f0, f1] = body.edge_faces(*e)?;
        let lookup = |m: &HashMap<_, usize>, k| {
            m.get(&k)
                .copied()
                .ok_or_else(|| internal("an operand refers to a missing entity"))
        };
        edges.push(InEdge {
            span: Span::from_curve(&c, t0, t1)?,
            v: [lookup(&vmap, v0)?, lookup(&vmap, v1)?],
            faces: [
                fmap.get(&f0)
                    .copied()
                    .ok_or_else(|| internal("missing face"))?,
                fmap.get(&f1)
                    .copied()
                    .ok_or_else(|| internal("missing face"))?,
            ],
            name: body.edge_name(*e)?.map(str::to_owned),
            pieces: Vec::new(),
        });
    }
    let mut faces = Vec::with_capacity(face_ids.len());
    for f in &face_ids {
        let (surface, same) = body
            .face_surface(*f)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{f} has no surface")))?;
        let mut loops = Vec::new();
        for l in body.face_loops(*f)? {
            let mut lp = Vec::new();
            for h in body.loop_half_edges(l)? {
                let e = body.half_edge_edge(h)?.ok_or_else(|| {
                    KernelError::Unsupported(format!("{l} is a lone vertex, not a loop of edges"))
                })?;
                let fwd = body.edge_half_edges(e)?[0] == h;
                lp.push((
                    emap.get(&e)
                        .copied()
                        .ok_or_else(|| internal("missing edge"))?,
                    fwd,
                ));
            }
            loops.push(lp);
        }
        faces.push(InFace {
            id: *f,
            side: Side {
                surface,
                orient: if same { 1.0 } else { -1.0 },
            },
            name: body.face_name(*f)?.map(str::to_owned),
            bbox: face_box(body, *f, &surface)?,
            loops,
        });
    }
    Ok(Loaded {
        which,
        body,
        faces,
        edges,
    })
}

/// A piece of an intersection curve (or of an imprinted edge) that lies on some faces.
#[derive(Debug, Clone)]
pub(super) struct Cut {
    pub span: Span,
    pub v: [usize; 2],
    /// `(operand, face index)` of the faces it splits.
    pub faces: Vec<(Which, usize)>,
    pub name: Option<String>,
    pub pieces: Vec<Use>,
}

/// Everything the face-pair intersections found.
#[derive(Debug, Default)]
pub(super) struct Cuts {
    pub cuts: Vec<Cut>,
    /// Pairs `(face of A, face of B)` on the same surface.
    pub coincident: Vec<(usize, usize)>,
}

fn label(a: Option<&str>, b: Option<&str>) -> Option<String> {
    let (a, b) = (a?, b?);
    let mut s = format!("{a} x {b}");
    if s.len() > crate::MAX_NAME_LENGTH {
        let mut cut = crate::MAX_NAME_LENGTH;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    Some(s)
}

/// Sorted, distinct (by point) parameters within `lo..hi`; on a whole closed carrier
/// (`cyclic`) moved into `[lo, lo + 2π)`.
fn sorted_params(c: &Carrier, lo: f64, hi: f64, cyclic: bool, mut ts: Vec<f64>) -> Vec<f64> {
    if c.periodic() {
        for t in &mut ts {
            *t = lo + (*t - lo).rem_euclid(TAU);
            // An arc's end found as the start of the next turn.
            if !cyclic && *t > hi && c.point(*t).coincident(c.point(lo)) {
                *t = lo;
            }
        }
    }
    ts.retain(|t| t.is_finite() && (cyclic || (*t >= lo && *t <= hi)));
    ts.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for t in ts {
        if out
            .last()
            .is_none_or(|l| !c.point(*l).coincident(c.point(t)))
        {
            out.push(t);
        }
    }
    if cyclic
        && out.len() > 1
        && let (Some(f), Some(l)) = (out.first(), out.last())
        && c.point(*f).coincident(c.point(*l))
    {
        out.pop();
    }
    out
}

/// The intervals between consecutive parameters (cyclic on a whole closed carrier, and
/// the whole turn when there are none).
fn intervals(lo: f64, cyclic: bool, ts: &[f64]) -> Vec<(f64, f64)> {
    if cyclic {
        if ts.is_empty() {
            return vec![(lo, lo + TAU)];
        }
        let n = ts.len();
        (0..n)
            .map(|i| {
                let a = ts[i];
                let b = if i + 1 < n { ts[i + 1] } else { ts[0] + TAU };
                (a, b)
            })
            .collect()
    } else {
        ts.windows(2).map(|w| (w[0], w[1])).collect()
    }
}

struct Pair<'a, 'b> {
    a: &'a Loaded<'b>,
    b: &'a Loaded<'b>,
}

impl Pair<'_, '_> {
    fn op(&self, w: Which) -> &Loaded<'_> {
        match w {
            Which::A => self.a,
            Which::B => self.b,
        }
    }
}

/// Parameters on `c` (within `lo..hi`) where it meets the boundary of face `f`.
fn boundary_params(
    op: &Loaded<'_>,
    f: usize,
    c: &Carrier,
    lo: f64,
    hi: f64,
) -> KernelResult<Vec<f64>> {
    let face = op.face(f)?;
    let mut ts = Vec::new();
    for e in op.face_edges(f)? {
        let nb = op.neighbour(f, e)?;
        ts.extend(crossings(
            c,
            lo,
            hi,
            &face.side.surface,
            &op.edge(e)?.span,
            nb.as_ref(),
        )?);
    }
    Ok(ts)
}

/// Adds the pieces of carrier `c` over `lo..hi` that lie on every face of `on` (inside or
/// on the boundary), split at `extra` parameters and the faces' boundaries, as cuts of
/// the faces in `splits`.
#[allow(clippy::too_many_arguments)]
fn add_cuts(
    pair: &Pair<'_, '_>,
    reg: &mut Registry,
    out: &mut Cuts,
    c: &Carrier,
    lo: f64,
    hi: f64,
    extra: &[f64],
    on: &[(Which, usize)],
    splits: &[(Which, usize)],
    name: Option<String>,
) -> KernelResult<()> {
    let mut ts: Vec<f64> = extra.to_vec();
    for &(w, f) in on {
        ts.extend(boundary_params(pair.op(w), f, c, lo, hi)?);
    }
    let cyclic = c.periodic() && hi - lo >= TAU - 1e-12;
    let ts = sorted_params(c, lo, hi, cyclic, ts);
    for (s0, s1) in intervals(lo, cyclic, &ts) {
        let span = Span::new(*c, s0, s1);
        let full = span.is_full_turn();
        if !full && span.start().coincident(span.end()) {
            continue;
        }
        let mid = span.mid();
        let mut keep = true;
        for &(w, f) in on {
            if pair.op(w).on_face(f, mid)? == PointOnFace::Outside {
                keep = false;
                break;
            }
        }
        if !keep {
            continue;
        }
        let v0 = reg.vertex(span.start(), None)?;
        let v1 = if full {
            v0
        } else {
            reg.vertex(span.end(), None)?
        };
        if out.cuts.len() >= MAX_BOOLEAN_ENTITIES {
            return Err(KernelError::InvalidInput(format!(
                "the boolean would need more than {MAX_BOOLEAN_ENTITIES} intersection pieces"
            )));
        }
        out.cuts.push(Cut {
            span,
            v: [v0, v1],
            faces: splits.to_vec(),
            name: name.clone(),
            pieces: Vec::new(),
        });
    }
    Ok(())
}

/// The edges of face `from` laid onto the coincident face `to`, as cuts of `to`.
fn imprint(
    pair: &Pair<'_, '_>,
    reg: &mut Registry,
    out: &mut Cuts,
    from: (Which, usize),
    to: (Which, usize),
) -> KernelResult<()> {
    let src = pair.op(from.0);
    for e in src.face_edges(from.1)? {
        let ed = src.edge(e)?;
        let s = ed.span;
        add_cuts(
            pair,
            reg,
            out,
            &s.carrier,
            s.t0,
            s.t1,
            &[s.t0, s.t1],
            &[to],
            &[to],
            ed.name.clone(),
        )?;
    }
    Ok(())
}

/// Intersects every pair of faces whose boxes meet.
pub(super) fn intersect_faces(
    a: &Loaded<'_>,
    b: &Loaded<'_>,
    reg: &mut Registry,
) -> KernelResult<Cuts> {
    let pair = Pair { a, b };
    let mut out = Cuts::default();
    for (ia, fa) in a.faces.iter().enumerate() {
        for (ib, fb) in b.faces.iter().enumerate() {
            if !boxes_meet(&fa.bbox, &fb.bbox) {
                continue;
            }
            let both = [(Which::A, ia), (Which::B, ib)];
            match intersect_surfaces(&fa.side.surface, &fb.side.surface)? {
                SurfaceIntersection::Empty => {}
                SurfaceIntersection::Coincident => {
                    out.coincident.push((ia, ib));
                    imprint(&pair, reg, &mut out, (Which::B, ib), (Which::A, ia))?;
                    imprint(&pair, reg, &mut out, (Which::A, ia), (Which::B, ib))?;
                }
                SurfaceIntersection::Unsupported(why) => {
                    return Err(KernelError::Unsupported(format!(
                        "the boolean needs the intersection of face `{}` of the first solid and face `{}` of the second, and {why}",
                        fa.name.as_deref().unwrap_or("(unnamed)"),
                        fb.name.as_deref().unwrap_or("(unnamed)"),
                    )));
                }
                other => {
                    for ic in other.curves() {
                        let (c, lo, hi) = Carrier::from_curve(&ic.curve, ic.range.0, ic.range.1)?;
                        add_cuts(
                            &pair,
                            reg,
                            &mut out,
                            &c,
                            lo,
                            hi,
                            &[],
                            &both,
                            &both,
                            label(fa.name.as_deref(), fb.name.as_deref()),
                        )?;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// `span` from `v0` to `v1` split at every registry vertex on its interior, as registry
/// edges in the span's direction.
fn split_span(
    reg: &mut Registry,
    span: &Span,
    v0: usize,
    v1: usize,
    name: Option<&str>,
) -> KernelResult<Vec<Use>> {
    let bb = span.bounding_box()?;
    let t = LINEAR_TOLERANCE;
    let mut at: Vec<(f64, usize)> = Vec::new();
    for (i, p) in reg.points.iter().enumerate() {
        if i == v0 || i == v1 {
            continue;
        }
        if p.x < bb.min.x - t
            || p.x > bb.max.x + t
            || p.y < bb.min.y - t
            || p.y > bb.max.y + t
            || p.z < bb.min.z - t
            || p.z > bb.max.z + t
        {
            continue;
        }
        if let Some(s) = span.param_of(*p)?
            && s > span.t0
            && s < span.t1
        {
            at.push((s, i));
        }
    }
    at.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut stops = vec![(span.t0, v0)];
    stops.extend(at);
    stops.push((span.t1, v1));
    let mut out = Vec::new();
    for w in stops.windows(2) {
        let ((s0, a), (s1, b)) = (w[0], w[1]);
        if a == b && !(span.is_full_turn() && stops.len() == 2) {
            continue;
        }
        let piece = Span::new(span.carrier, s0, s1);
        out.push(reg.edge_piece(piece, a, b, name)?);
    }
    Ok(out)
}

/// Splits every operand edge and every cut at the registry's vertices.
pub(super) fn split_edges(
    ops: [&mut Loaded<'_>; 2],
    cuts: &mut Cuts,
    reg: &mut Registry,
) -> KernelResult<()> {
    for op in ops {
        for e in &mut op.edges {
            e.pieces = split_span(reg, &e.span, e.v[0], e.v[1], e.name.as_deref())?;
        }
    }
    for c in &mut cuts.cuts {
        c.pieces = split_span(reg, &c.span, c.v[0], c.v[1], c.name.as_deref())?;
    }
    Ok(())
}

/// A piece of an operand face: the region and where it came from.
#[derive(Debug, Clone)]
pub(super) struct FacePiece {
    pub which: Which,
    pub face: usize,
    pub region: Region,
    /// Registry edges that are cuts of this face (not on its original boundary).
    pub cut_edges: HashSet<usize>,
}

/// Splits face `f` of `op` along its cuts.
pub(super) fn split_face(
    op: &Loaded<'_>,
    f: usize,
    cuts: &[&Cut],
    reg: &Registry,
) -> KernelResult<Vec<FacePiece>> {
    let face = op.face(f)?;
    let mut uses: Vec<Use> = Vec::new();
    let mut boundary = HashSet::new();
    for lp in &face.loops {
        for &(e, fwd) in lp {
            let pieces = &op.edge(e)?.pieces;
            if fwd {
                for &(id, d) in pieces {
                    uses.push((id, d));
                    boundary.insert(id);
                }
            } else {
                for &(id, d) in pieces.iter().rev() {
                    uses.push((id, !d));
                    boundary.insert(id);
                }
            }
        }
    }
    let mut removable = HashSet::new();
    for c in cuts {
        for &(id, _) in &c.pieces {
            if !boundary.contains(&id) && removable.insert(id) {
                uses.push((id, true));
                uses.push((id, false));
            }
        }
    }
    let cycles = trace(reg, &face.side, &uses, &removable)?;
    let regs = regions(reg, &face.side, cycles)?;
    Ok(regs
        .into_iter()
        .map(|region| FacePiece {
            which: op.which,
            face: f,
            region,
            cut_edges: removable.clone(),
        })
        .collect())
}

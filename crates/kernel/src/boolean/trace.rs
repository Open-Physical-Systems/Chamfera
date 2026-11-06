//! Splitting a face along its cut edges: tracing the regions of the face's edge graph,
//! sorting their loops into outer loops and holes, and finding interior sample points.
//!
//! # Tracing
//! A face's graph holds its boundary pieces, each usable only in the direction the face
//! uses it, and its cut pieces, usable both ways. With the face on the left of every
//! half-edge, the loop through a half-edge arriving at vertex `w` continues with the
//! half-edge leaving `w` that comes first clockwise after the arriving one's reverse,
//! seen against the outward normal (Mantyla, *An Introduction to Solid Modeling*,
//! section 15.5, for the same rule in the boolean's face splitting). Angles are taken in
//! the tangent plane at `w`; two half-edges leaving with the same tangent (curves that
//! touch) are ordered by a short chord along each, and turning back along the arriving
//! edge comes last (a seam turns back at a pole). Every half-edge is in exactly one loop.
//!
//! Before tracing, cut pieces with a free end (an intersection that ends inside the face,
//! e.g. where the other solid's edge only touches the face) are pruned; after tracing, a
//! cut piece both of whose directions ended up in one loop separates nothing and is
//! removed, and the face is traced again.
//!
//! # Outer loops and holes
//! A loop's signed area (exact on a plane, from its `(u, v)` image on a curved surface,
//! positive counter-clockwise about the outward normal) says whether it bounds a region
//! (positive) or is a hole in one (negative). A hole belongs to the smallest outer loop
//! whose image contains a point of the hole, among the loops that share no edge with it.
//!
//! # `(u, v)` images
//! As in `topology/uv.rs`: edges are sampled, projected onto the surface's parameters and
//! unwrapped across the periodic direction; at a singular point (a pole or apex) the image
//! steps along the singular line in the direction the face's orientation demands.

use super::registry::Registry;
use super::span::internal;
use crate::{KernelResult, Surface};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::collections::{HashMap, HashSet};
use std::f64::consts::{PI, TAU};

/// An oriented use of a registry edge: `(edge, runs along it)`.
pub(super) type Use = (usize, bool);

/// Two directions leaving a vertex closer than this (radians) are ordered by chords.
const TIE: f64 = 1e-9;
/// Fraction of an edge's parameter span used for the tie-breaking chord.
const CHORD: f64 = 1e-3;
/// Two image coordinates closer than this are the same (as in `topology/uv.rs`).
const IMAGE_TOLERANCE: f64 = 1e-9;

/// A face's surface and how its outward normal relates to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Side {
    pub surface: Surface,
    /// `+1` when the outward normal is the surface normal, `-1` otherwise.
    pub orient: f64,
}

impl Side {
    pub fn normal(&self, p: Point3) -> Vec3 {
        match self.surface {
            Surface::Plane(f) => f.z() * self.orient,
            s => {
                let q = s.parameters(p);
                s.normal_at(q.u, q.v) * self.orient
            }
        }
    }
}

pub(super) fn start_vertex(reg: &Registry, u: Use) -> KernelResult<usize> {
    let r = reg.edge(u.0)?;
    Ok(if u.1 { r.v[0] } else { r.v[1] })
}

pub(super) fn end_vertex(reg: &Registry, u: Use) -> KernelResult<usize> {
    let r = reg.edge(u.0)?;
    Ok(if u.1 { r.v[1] } else { r.v[0] })
}

/// The point at fraction `s` of a use, in its direction.
pub(super) fn use_point(reg: &Registry, u: Use, s: f64) -> KernelResult<Point3> {
    let r = reg.edge(u.0)?;
    Ok(r.span.at(if u.1 { s } else { 1.0 - s }))
}

fn start_tangent(reg: &Registry, u: Use) -> KernelResult<Vec3> {
    let s = reg.edge(u.0)?.span;
    Ok(if u.1 {
        s.carrier.d1(s.t0)
    } else {
        -s.carrier.d1(s.t1)
    })
}

fn end_tangent(reg: &Registry, u: Use) -> KernelResult<Vec3> {
    let s = reg.edge(u.0)?.span;
    Ok(if u.1 {
        s.carrier.d1(s.t1)
    } else {
        -s.carrier.d1(s.t0)
    })
}

/// Angles of a direction and a chord in the tangent frame at a vertex.
#[derive(Debug, Clone, Copy)]
struct Heading {
    tangent: f64,
    chord: f64,
}

fn angle(frame: &Frame, d: Vec3) -> f64 {
    d.dot(frame.y()).atan2(d.dot(frame.x()))
}

/// How far clockwise `c` comes after `twin`, as (by tangent, by chord), each in
/// `[0, 2π)`. A tangent within [`TIE`] of the twin's counts as just after it or just
/// before it (0 or 2π) as the chords say.
fn clockwise_after(twin: Heading, c: Heading) -> (f64, f64) {
    let near = |d: f64| !(TIE..=TAU - TIE).contains(&d);
    let dt = (twin.tangent - c.tangent).rem_euclid(TAU);
    let dc = (twin.chord - c.chord).rem_euclid(TAU);
    if near(dt) {
        if near(dc) {
            return (TAU, TAU);
        }
        return (if dc < PI { 0.0 } else { TAU }, dc);
    }
    (dt, dc)
}

/// `a` comes before `b`: by tangent, or by chord when the tangents tie.
fn before(a: (f64, f64), b: (f64, f64)) -> bool {
    if (a.0 - b.0).abs() <= TIE {
        a.1 < b.1
    } else {
        a.0 < b.0
    }
}

/// The loops of a face graph (see the module documentation). `uses` are the allowed
/// half-edges; edges in `removable` (the cut pieces) may be pruned.
pub(super) fn trace(
    reg: &Registry,
    side: &Side,
    uses: &[Use],
    removable: &HashSet<usize>,
) -> KernelResult<Vec<Vec<Use>>> {
    let mut set: Vec<Use> = Vec::new();
    let mut seen = HashSet::new();
    for u in uses {
        if seen.insert(*u) {
            set.push(*u);
        }
    }
    // Prune cut pieces with a free end.
    for _ in 0..=set.len() {
        let mut ends: HashMap<usize, usize> = HashMap::new();
        let mut edges = HashSet::new();
        for u in &set {
            if edges.insert(u.0) {
                let r = reg.edge(u.0)?;
                *ends.entry(r.v[0]).or_default() += 1;
                *ends.entry(r.v[1]).or_default() += 1;
            }
        }
        let mut dead = HashSet::new();
        for e in &edges {
            if removable.contains(e) {
                let r = reg.edge(*e)?;
                if r.v.iter().any(|v| ends.get(v).copied().unwrap_or(0) <= 1) {
                    dead.insert(*e);
                }
            }
        }
        if dead.is_empty() {
            break;
        }
        set.retain(|u| !dead.contains(&u.0));
    }
    for _ in 0..=set.len() {
        let cycles = trace_once(reg, side, &set)?;
        let mut bridge = None;
        'find: for c in &cycles {
            let edges: HashSet<Use> = c.iter().copied().collect();
            for u in c {
                if removable.contains(&u.0) && edges.contains(&(u.0, !u.1)) {
                    bridge = Some(u.0);
                    break 'find;
                }
            }
        }
        match bridge {
            Some(e) => set.retain(|u| u.0 != e),
            None => return Ok(cycles),
        }
    }
    Err(internal("tracing a split face did not settle"))
}

fn trace_once(reg: &Registry, side: &Side, set: &[Use]) -> KernelResult<Vec<Vec<Use>>> {
    // Tangent frames at the vertices.
    let mut frames: HashMap<usize, Frame> = HashMap::new();
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, u) in set.iter().enumerate() {
        let v = start_vertex(reg, *u)?;
        out.entry(v).or_default().push(i);
        for w in [v, end_vertex(reg, *u)?] {
            if let std::collections::hash_map::Entry::Vacant(slot) = frames.entry(w) {
                let p = reg.point(w)?;
                let f = Frame::from_normal(p, side.normal(p))
                    .ok_or_else(|| internal("a face has no normal at a vertex"))?;
                slot.insert(f);
            }
        }
    }
    let mut leaving = Vec::with_capacity(set.len());
    for u in set {
        let v = start_vertex(reg, *u)?;
        let f = frames
            .get(&v)
            .ok_or_else(|| internal("a vertex lost its frame"))?;
        let p = reg.point(v)?;
        leaving.push(Heading {
            tangent: angle(f, start_tangent(reg, *u)?),
            chord: angle(f, use_point(reg, *u, CHORD)? - p),
        });
    }
    let index: HashMap<Use, usize> = set.iter().enumerate().map(|(i, u)| (*u, i)).collect();
    let mut next = vec![usize::MAX; set.len()];
    for (i, u) in set.iter().enumerate() {
        let w = end_vertex(reg, *u)?;
        let f = frames
            .get(&w)
            .ok_or_else(|| internal("a vertex lost its frame"))?;
        let p = reg.point(w)?;
        let twin = Heading {
            tangent: angle(f, -end_tangent(reg, *u)?),
            chord: angle(f, use_point(reg, *u, 1.0 - CHORD)? - p),
        };
        let twin_id = index.get(&(u.0, !u.1)).copied();
        let mut best: Option<((f64, f64), usize)> = None;
        for &c in out.get(&w).map(Vec::as_slice).unwrap_or(&[]) {
            // Turning back along the same edge comes last (a seam's end at a pole).
            let d = if Some(c) == twin_id {
                (2.0 * TAU, 2.0 * TAU)
            } else {
                clockwise_after(twin, leaving[c])
            };
            if best.is_none_or(|b| before(d, b.0)) {
                best = Some((d, c));
            }
        }
        next[i] = best
            .map(|b| b.1)
            .ok_or_else(|| internal("a split face's edge graph has a dead end"))?;
    }
    let mut visited = vec![false; set.len()];
    let mut cycles = Vec::new();
    for start in 0..set.len() {
        if visited[start] {
            continue;
        }
        let mut cycle = Vec::new();
        let mut j = start;
        for _ in 0..=set.len() {
            if visited[j] {
                return Err(internal(
                    "the loops of a split face do not close consistently",
                ));
            }
            visited[j] = true;
            cycle.push(set[j]);
            j = next[j];
            if j == start {
                break;
            }
        }
        if j != start {
            return Err(internal("a loop of a split face does not close"));
        }
        cycles.push(cycle);
    }
    Ok(cycles)
}

/// A region of a face: an outer loop and its holes, with their images in one consistent
/// `(u, v)` frame.
#[derive(Debug, Clone)]
pub(super) struct Region {
    pub loops: Vec<Vec<Use>>,
    pub images: Vec<Vec<(f64, f64)>>,
}

/// The image of one loop: the samples of each use, and its signed area (positive
/// counter-clockwise about the outward normal).
struct Image {
    edges: Vec<Vec<(f64, f64)>>,
    area: f64,
}

impl Image {
    fn points(&self) -> Vec<(f64, f64)> {
        self.edges.iter().flatten().copied().collect()
    }
}

fn samples_for(reg: &Registry, u: Use) -> KernelResult<usize> {
    let s = reg.edge(u.0)?.span;
    if !s.carrier.periodic() {
        return Ok(1);
    }
    let n = ((s.t1 - s.t0).abs() / TAU * 128.0).ceil();
    Ok(if n.is_finite() && n > 8.0 {
        (n as usize).min(4096)
    } else {
        8
    })
}

fn shoelace(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    let mut sum = 0.0;
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        sum += a.0 * b.1 - b.0 * a.1;
    }
    0.5 * sum
}

fn nearest(x: f64, target: f64, p: f64) -> f64 {
    x + ((target - x) / p).round() * p
}

fn directed(step: f64, dir: f64, p: f64) -> f64 {
    if dir > 0.0 {
        let k = ((IMAGE_TOLERANCE - step) / p).floor() + 1.0;
        step + k * p
    } else {
        let k = ((-IMAGE_TOLERANCE - step) / p).ceil() - 1.0;
        step + k * p
    }
}

fn pole_direction(v_before: f64, v_s: f64, orient: f64) -> f64 {
    let side = v_before - v_s;
    if side.abs() <= IMAGE_TOLERANCE {
        0.0
    } else {
        side.signum() * orient
    }
}

fn image(reg: &Registry, side: &Side, cycle: &[Use]) -> KernelResult<Image> {
    if let Surface::Plane(frame) = side.surface {
        let mut edges = Vec::new();
        let mut moment = Vec3::ZERO;
        let r = match cycle.first() {
            Some(u) => use_point(reg, *u, 0.0)?,
            None => return Err(internal("an empty loop")),
        };
        for u in cycle {
            let n = samples_for(reg, *u)?;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let q = frame.to_plane(use_point(reg, *u, i as f64 / n as f64)?);
                pts.push((q.x, q.y));
            }
            edges.push(pts);
            let m = reg.edge(u.0)?.span.area_moment(r);
            moment = if u.1 { moment + m } else { moment - m };
        }
        let area = 0.5 * moment.dot(frame.z()) * side.orient;
        return Ok(Image { edges, area });
    }
    let surface = side.surface;
    let orient = side.orient;
    let pu = surface.period_u();
    let pv = surface.period_v();
    let mut edges: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut sing: Vec<(bool, bool)> = Vec::new();
    for u in cycle {
        let n = samples_for(reg, *u)?.max(8);
        let mut raw = Vec::with_capacity(n + 1);
        for i in 0..=n {
            raw.push(surface.parameters(use_point(reg, *u, i as f64 / n as f64)?));
        }
        if raw.iter().all(|q| q.singular) {
            return Err(internal("an edge lies on the axis of its face's surface"));
        }
        let regular: Vec<usize> = (0..raw.len()).filter(|i| !raw[*i].singular).collect();
        let mut uv: Vec<(f64, f64)> = raw.iter().map(|q| (q.u, q.v)).collect();
        for (i, q) in raw.iter().enumerate() {
            if q.singular
                && let Some(j) = regular.iter().min_by_key(|j| j.abs_diff(i))
            {
                uv[i].0 = raw[*j].u;
            }
        }
        for i in 1..uv.len() {
            let (a, b) = uv[i - 1];
            if let Some(p) = pu {
                uv[i].0 = nearest(uv[i].0, a, p);
            }
            if let Some(p) = pv {
                uv[i].1 = nearest(uv[i].1, b, p);
            }
        }
        let s_start = raw.first().is_some_and(|q| q.singular);
        let s_end = raw.last().is_some_and(|q| q.singular);
        if let (Some(prev), Some(&(ps_start, ps_end))) = (edges.last(), sing.last())
            && let (Some(&(lu, lv)), Some(&(fu, fv))) = (prev.last(), uv.first())
        {
            let _ = ps_start;
            let mut du = 0.0;
            let mut dv = 0.0;
            if let Some(p) = pv {
                dv = nearest(fv, lv, p) - fv;
            }
            if let Some(p) = pu {
                let dir = if ps_end && s_start {
                    let before = prev.len().checked_sub(2).and_then(|k| prev.get(k));
                    before.map_or(0.0, |b| pole_direction(b.1, lv, orient))
                } else {
                    0.0
                };
                du = if dir == 0.0 {
                    nearest(fu, lu, p) - fu
                } else {
                    lu + directed(fu - lu, dir, p) - fu
                };
            }
            for q in &mut uv {
                q.0 += du;
                q.1 += dv;
            }
        }
        edges.push(uv);
        sing.push((s_start, s_end));
    }
    let (Some(first), Some(last)) = (edges.first(), edges.last()) else {
        return Err(internal("an empty loop"));
    };
    let (Some(&(fu, fv)), Some(&(lu, lv))) = (first.first(), last.last()) else {
        return Err(internal("a loop without samples"));
    };
    let closed_v = pv.is_none() || (fv - lv).abs() <= IMAGE_TOLERANCE;
    let closed_u = match pu {
        None => true,
        Some(p) => {
            let first_sing = sing.first().is_some_and(|s| s.0);
            let last_sing = sing.last().is_some_and(|s| s.1);
            let du = fu - lu;
            let dir = if last_sing && first_sing {
                let before = last.len().checked_sub(2).and_then(|k| last.get(k));
                before.map_or(0.0, |b| pole_direction(b.1, lv, orient))
            } else {
                0.0
            };
            if dir == 0.0 {
                du.abs() <= IMAGE_TOLERANCE
            } else {
                // Close the image with the final step along the singular line.
                let step = directed(du, dir, p);
                if let Some(l) = edges.last_mut() {
                    l.push((lu + step, lv));
                }
                (step - du).abs() <= IMAGE_TOLERANCE
            }
        }
    };
    if !(closed_u && closed_v) {
        return Err(crate::KernelError::Unsupported(
            "a split face has a loop that goes around its surface without a seam".into(),
        ));
    }
    let pts: Vec<(f64, f64)> = edges.iter().flatten().copied().collect();
    let area = shoelace(&pts) * orient;
    Ok(Image { edges, area })
}

/// Even-odd crossings of the ray from `(u, v)` toward +u with the closed polygon `p`.
fn crossings(p: &[(f64, f64)], u: f64, v: f64) -> usize {
    let n = p.len();
    let mut c = 0;
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        if (a.1 > v) != (b.1 > v) {
            let x = a.0 + (v - a.1) * (b.0 - a.0) / (b.1 - a.1);
            if x > u {
                c += 1;
            }
        }
    }
    c
}

/// The regions bounded by the traced `cycles` of one face.
pub(super) fn regions(
    reg: &Registry,
    side: &Side,
    cycles: Vec<Vec<Use>>,
) -> KernelResult<Vec<Region>> {
    let mut images = Vec::with_capacity(cycles.len());
    for c in &cycles {
        images.push(image(reg, side, c)?);
    }
    let tiny = LINEAR_TOLERANCE * LINEAR_TOLERANCE;
    let mut outers: Vec<usize> = Vec::new();
    let mut holes: Vec<usize> = Vec::new();
    for (i, im) in images.iter().enumerate() {
        if !im.area.is_finite() || im.area.abs() <= tiny {
            return Err(internal("a split face has a piece without area"));
        }
        if im.area > 0.0 {
            outers.push(i);
        } else {
            holes.push(i);
        }
    }
    let period = side.surface.period_u();
    let mut owner: Vec<(usize, f64)> = Vec::new();
    for &h in &holes {
        let hole_edges: HashSet<usize> = cycles[h].iter().map(|u| u.0).collect();
        let Some(test) = images[h]
            .edges
            .first()
            .and_then(|e| e.get(e.len() / 2).copied())
        else {
            return Err(internal("a hole without samples"));
        };
        let mut best: Option<(f64, usize, f64)> = None;
        for &o in &outers {
            if cycles[o].iter().any(|u| hole_edges.contains(&u.0)) {
                continue;
            }
            let pts = images[o].points();
            let shifts: &[f64] = if period.is_some() {
                &[0.0, -1.0, 1.0, -2.0, 2.0]
            } else {
                &[0.0]
            };
            for k in shifts {
                let s = k * period.unwrap_or(0.0);
                if crossings(&pts, test.0 + s, test.1) % 2 == 1 {
                    let a = images[o].area;
                    if best.is_none_or(|b| a < b.0) {
                        best = Some((a, o, s));
                    }
                    break;
                }
            }
        }
        let (_, o, s) = best.ok_or_else(|| internal("a hole of a split face lies in no region"))?;
        owner.push((o, s));
    }
    let mut out = Vec::with_capacity(outers.len());
    for &o in &outers {
        let mut loops = vec![cycles[o].clone()];
        let mut ims = vec![images[o].points()];
        for (k, &h) in holes.iter().enumerate() {
            if let Some(&(oo, s)) = owner.get(k)
                && oo == o
            {
                loops.push(cycles[h].clone());
                ims.push(images[h].points().iter().map(|p| (p.0 + s, p.1)).collect());
            }
        }
        out.push(Region { loops, images: ims });
    }
    Ok(out)
}

/// Points inside a region, best first: midpoints of the widest spans cut from the region
/// by lines of constant `v` halfway between the images' sample heights.
pub(super) fn interior_points(region: &Region, side: &Side) -> Vec<Point3> {
    let mut vs: Vec<f64> = region.images.iter().flatten().map(|p| p.1).collect();
    vs.sort_by(f64::total_cmp);
    vs.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
    let step = (vs.len() / 96).max(1);
    let levels: Vec<f64> = vs.iter().step_by(step).copied().collect();
    let mut found: Vec<(f64, (f64, f64))> = Vec::new();
    for w in levels.windows(2) {
        let gap = w[1] - w[0];
        if gap <= 1e-9 {
            continue;
        }
        for f in [0.5, 0.382] {
            let v = w[0] + f * gap;
            let mut xs = Vec::new();
            for im in &region.images {
                let n = im.len();
                for i in 0..n {
                    let (a, b) = (im[i], im[(i + 1) % n]);
                    if (a.1 > v) != (b.1 > v) {
                        xs.push(a.0 + (v - a.1) * (b.0 - a.0) / (b.1 - a.1));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for [x0, x1] in xs.as_chunks::<2>().0 {
                let width = x1 - x0;
                for g in [0.5f64, 0.382, 0.618] {
                    let score = (g.min(1.0 - g) * width).min(f.min(1.0 - f) * gap);
                    found.push((score, (x0 + g * width, v)));
                }
            }
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found
        .into_iter()
        .take(12)
        .map(|(_, (u, v))| side.surface.point_at(u, v))
        .collect()
}

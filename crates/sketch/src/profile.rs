//! Profiles: the closed regions bounded by a sketch's non-construction curves.
//!
//! Lines and arcs form a planar graph whose vertices are their endpoints; endpoints are the
//! same vertex when they are the same point, are joined by a coincident constraint, or lie
//! within `LINEAR_TOLERANCE` of each other. Circles are closed loops on their own.
//! Elliptical arcs join like arcs; full ellipses, like circles, are loops on their own.
//!
//! 1. Bridges (edges whose removal disconnects the graph) cannot bound a region, so they
//!    are dropped; this also drops dangling segments.
//! 2. Around each vertex the outgoing edges are ordered by tangent angle (ties broken by
//!    curvature). Following each half-edge with the next edge clockwise from its twin
//!    traces every face of the graph with the face on the left: bounded faces come out
//!    counter-clockwise (positive area), the outside of each connected piece clockwise.
//! 3. Each clockwise outside boundary (and each circle, reversed) becomes a hole of the
//!    smallest bounded face of another piece that contains it.
//!
//! A [`Region`] is one bounded face: a counter-clockwise outer loop, clockwise holes, and
//! its area (outer area minus holes). A circle inside a rectangle gives two regions: the
//! rectangle with a circular hole, and the disc.
//!
//! **Limits.** Curves are not split here where they cross or where one ends in the middle
//! of another (a T-junction): a junction counts only where curves share an endpoint, and
//! crossing curves give undefined, though never panicking, results. The split tool
//! (`ToolOp::Split`, `sketch.split`) cuts such drawings into pieces first. Arcs are measured exactly; signed
//! areas use Green's theorem on each line, arc and elliptical arc.

use crate::constraint::Constraint;
use crate::ellipse::EllipseArc;
use crate::entity::Curve;
use crate::entity::{CurveId, PointId};
use crate::sketch::Sketch;
use crate::spline::{ray_crossings, spline_polyline};
use chamfera_geom::{BSplineCurve2, LINEAR_TOLERANCE, Point2, Vec2};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::Arc;

/// Faces with less area than this (mm²) are slivers between coincident curves, not regions.
const MIN_AREA: f64 = LINEAR_TOLERANCE;
/// Outgoing tangents closer than this (radians) are treated as the same direction and
/// ordered by curvature instead.
const TANGENT_BUCKET: f64 = 1e-9;

/// One curve of a loop, and whether the loop runs along it backwards (end to start; for a
/// circle or arc, clockwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopSegment {
    pub curve: CurveId,
    pub reversed: bool,
}

/// A closed chain of curves.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileLoop {
    pub segments: Vec<LoopSegment>,
    /// Positive for a counter-clockwise loop (mm²).
    pub signed_area: f64,
}

/// A closed region: an outer loop (counter-clockwise) with holes (clockwise).
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub outer: ProfileLoop,
    pub holes: Vec<ProfileLoop>,
    /// Outer area minus the holes' areas (mm²).
    pub area: f64,
}

/// A line or arc in its forward direction.
/// Where along a boundary its inside/outside probe is taken (see `point_at_fraction`).
const PROBE_FRACTION: f64 = 0.381_966;
/// A sweep this close to a full turn is a full circle (radians).
const ANGLE_EPS: f64 = 1e-12;

#[derive(Debug, Clone)]
enum Geo {
    Line {
        a: Point2,
        b: Point2,
    },
    Arc {
        c: Point2,
        r: f64,
        a0: f64,
        sweep: f64,
    },
    /// A spline from `a` (its first point) to `b` (its last; `a` again when closed).
    Spline {
        curve: Arc<BSplineCurve2>,
        a: Point2,
        b: Point2,
    },
    Ellipse(EllipseArc),
}

impl Geo {
    /// ∮ (x dy - y dx) / 2 along the curve, forwards or backwards.
    ///
    /// Line: `(a × b) / 2`. Arc `x = cx + r cos t, y = cy + r sin t`, t from a0 to a1:
    /// `x dy - y dx = (r² + r cx cos t + r cy sin t) dt`, which integrates to
    /// `r² Δ + r cx (sin a1 - sin a0) - r cy (cos a1 - cos a0)`.
    fn area(&self, reversed: bool) -> f64 {
        let fwd = match *self {
            Geo::Line { a, b } => 0.5 * (a.x * b.y - b.x * a.y),
            Geo::Arc { c, r, a0, sweep } => {
                let a1 = a0 + sweep;
                0.5 * (r * r * sweep + r * c.x * (a1.sin() - a0.sin())
                    - r * c.y * (a1.cos() - a0.cos()))
            }
            // Gauss-Legendre quadrature of the same integrand on each knot span.
            Geo::Spline { ref curve, .. } => {
                let (lo, hi) = curve.domain();
                curve.area_term(lo, hi).unwrap_or(0.0)
            }
            // p = c + a cos t u + b sin t v with u × v = 1: p × p' = ab
            // - a sin t (c × u) + b cos t (c × v), which integrates to
            // ab Δ + a (c × u)(cos t1 - cos t0) + b (c × v)(sin t1 - sin t0).
            Geo::Ellipse(e) => {
                let (t0, t1) = (e.t0, e.t0 + e.sweep);
                let c = e.centre - Point2::ORIGIN;
                let (cu, cv) = (c.cross(e.axis), c.cross(e.minor_axis()));
                0.5 * (e.a * e.b * e.sweep
                    + e.a * cu * (t1.cos() - t0.cos())
                    + e.b * cv * (t1.sin() - t0.sin()))
            }
        };
        if reversed { -fwd } else { fwd }
    }

    /// A point part-way along the curve (fraction `f` of its length, from its start).
    /// Used as the probe for point-in-region tests: a generic fraction keeps the probe
    /// off vertices and off other curves' turning points, where ray casting has ties.
    fn point_at_fraction(&self, f: f64) -> Point2 {
        match *self {
            Geo::Line { a, b } => a.lerp(b, f),
            Geo::Arc { c, r, a0, sweep } => {
                let t = a0 + sweep * f;
                c + Vec2::new(t.cos(), t.sin()) * r
            }
            Geo::Spline { ref curve, .. } => {
                let (lo, hi) = curve.domain();
                curve.point_at(lo + (hi - lo) * f)
            }
            Geo::Ellipse(e) => e.point_at(e.t0 + e.sweep * f),
        }
    }

    /// Direction angle and signed curvature leaving the start of the (possibly reversed)
    /// curve.
    fn leaving(&self, reversed: bool) -> (f64, f64) {
        match *self {
            Geo::Line { a, b } => {
                let d = if reversed { a - b } else { b - a };
                (d.angle(), 0.0)
            }
            Geo::Arc { r, a0, sweep, .. } => {
                if reversed {
                    let t = a0 + sweep;
                    (Vec2::new(t.sin(), -t.cos()).angle(), -1.0 / r)
                } else {
                    (Vec2::new(-a0.sin(), a0.cos()).angle(), 1.0 / r)
                }
            }
            Geo::Spline { ref curve, .. } => {
                // Signed curvature κ = (C' × C'') / |C'|³; running backwards flips both
                // the tangent and the sign of κ.
                let (lo, hi) = curve.domain();
                let u = if reversed { hi } else { lo };
                let d = curve.derivatives(u, 2);
                let (d1, d2) = (
                    d.get(1).copied().unwrap_or(Vec2::X),
                    d.get(2).copied().unwrap_or(Vec2::ZERO),
                );
                let speed = d1.length();
                let k = if speed > 0.0 {
                    d1.cross(d2) / (speed * speed * speed)
                } else {
                    0.0
                };
                if reversed {
                    ((-d1).angle(), -k)
                } else {
                    (d1.angle(), k)
                }
            }
            // Curvature of the ellipse at t: ab / |p'(t)|³.
            Geo::Ellipse(e) => {
                let t = if reversed { e.t0 + e.sweep } else { e.t0 };
                let d = e.tangent_at(t);
                let l = d.length().max(LINEAR_TOLERANCE);
                let k = e.a * e.b / (l * l * l);
                if reversed {
                    ((-d).angle(), -k)
                } else {
                    (d.angle(), k)
                }
            }
        }
    }

    /// How many times a ray from `p` toward +X crosses the curve (half-open in y, so a
    /// chain of curves is counted consistently).
    fn crossings(&self, p: Point2) -> usize {
        match *self {
            Geo::Line { a, b } => usize::from(line_crosses(a, b, p)),
            Geo::Arc { c, r, a0, sweep } => {
                // Split into pieces monotone in y, at t = π/2 + kπ.
                let end = a0 + sweep;
                let full = sweep >= TAU - ANGLE_EPS;
                // The start and end of a full circle are one point: give them the same y,
                // or sin(a0) and sin(a0 + 2π) differ in the last bits and a ray through
                // that point counts one crossing instead of two.
                let y_of = |t: f64| {
                    let t = if full && t >= end { a0 } else { t };
                    c.y + r * t.sin()
                };
                let mut t0 = a0;
                let mut count = 0;
                let mut guard = 0;
                while t0 < end && guard < 8 {
                    guard += 1;
                    let k = ((t0 - FRAC_PI_2) / PI).floor() + 1.0;
                    let t1 = (FRAC_PI_2 + k * PI).min(end);
                    let y0 = y_of(t0);
                    let y1 = y_of(t1);
                    if (y0 > p.y) != (y1 > p.y) {
                        let h = (r * r - (p.y - c.y) * (p.y - c.y)).max(0.0).sqrt();
                        let mid = 0.5 * (t0 + t1);
                        let x = if mid.cos() >= 0.0 { c.x + h } else { c.x - h };
                        if x > p.x {
                            count += 1;
                        }
                    }
                    t0 = t1;
                }
                count
            }
            Geo::Spline { ref curve, a, b } => ray_crossings(curve, a, b, p),
            Geo::Ellipse(e) => ellipse_crossings(&e, p),
        }
    }
}

/// Ray crossings for an ellipse arc, split like a circular arc into pieces monotone in y.
///
/// `y(t) = cy + a cos t uy + b sin t vy = cy + K cos(t - φ)` with
/// `K = |(a uy, b vy)|`, `φ = atan2(b vy, a uy)`, so y turns at `t = φ + kπ`. In a piece
/// where `sin(t - φ) ≥ 0` the crossing is at `t = φ + acos(h)`, else `φ - acos(h)`, with
/// `h = (p.y - cy) / K`.
fn ellipse_crossings(e: &EllipseArc, p: Point2) -> usize {
    let (ky, kx) = (e.b * e.minor_axis().y, e.a * e.axis.y);
    let big_k = ky.hypot(kx);
    if !(big_k.is_finite() && big_k > 0.0) {
        return 0;
    }
    let phi = ky.atan2(kx);
    let end = e.t0 + e.sweep;
    let full = e.is_full();
    let y_of = |t: f64| {
        let t = if full && t >= end { e.t0 } else { t };
        e.point_at(t).y
    };
    let mut t0 = e.t0;
    let mut count = 0;
    let mut guard = 0;
    while t0 < end && guard < 8 {
        guard += 1;
        let k = ((t0 - phi) / PI).floor() + 1.0;
        let t1 = (phi + k * PI).min(end);
        let (y0, y1) = (y_of(t0), y_of(t1));
        if (y0 > p.y) != (y1 > p.y) {
            let h = ((p.y - e.centre.y) / big_k).clamp(-1.0, 1.0);
            let mid = 0.5 * (t0 + t1);
            let t = if (mid - phi).sin() >= 0.0 {
                phi + h.acos()
            } else {
                phi - h.acos()
            };
            if e.point_at(t).x > p.x {
                count += 1;
            }
        }
        t0 = t1;
    }
    count
}

fn line_crosses(a: Point2, b: Point2, p: Point2) -> bool {
    if (a.y > p.y) == (b.y > p.y) {
        return false;
    }
    let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
    x > p.x
}

struct Edge {
    curve: CurveId,
    u: usize,
    v: usize,
    geo: Geo,
}

/// A loop under construction: segments as (geometry, reversed).
struct Traced {
    segments: Vec<(CurveId, Geo, bool)>,
    area: f64,
    piece: usize,
}

impl Traced {
    fn contains(&self, p: Point2) -> bool {
        let n: usize = self.segments.iter().map(|(_, g, _)| g.crossings(p)).sum();
        n % 2 == 1
    }

    fn into_loop(self) -> ProfileLoop {
        ProfileLoop {
            segments: self
                .segments
                .iter()
                .map(|(curve, _, reversed)| LoopSegment {
                    curve: *curve,
                    reversed: *reversed,
                })
                .collect(),
            signed_area: self.area,
        }
    }
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i) {
        if p == i {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = gp;
        }
        i = gp;
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb
        && let Some(slot) = parent.get_mut(ra.max(rb))
    {
        *slot = ra.min(rb);
    }
}

fn arc_geo(c: Point2, s: Point2, e: Point2) -> Option<Geo> {
    let r = c.distance(s);
    if !(r.is_finite() && r > LINEAR_TOLERANCE) {
        return None;
    }
    let a0 = (s - c).angle();
    let mut sweep = (e - c).angle() - a0;
    if sweep <= 0.0 {
        sweep += TAU;
    }
    Some(Geo::Arc { c, r, a0, sweep })
}

/// Indices of bridge edges (Tarjan's low-link, iterative so deep graphs cannot overflow the
/// stack).
fn bridges(vertex_count: usize, edges: &[Edge]) -> Vec<bool> {
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); vertex_count];
    for (k, e) in edges.iter().enumerate() {
        if let Some(a) = adj.get_mut(e.u) {
            a.push((e.v, k));
        }
        if let Some(a) = adj.get_mut(e.v) {
            a.push((e.u, k));
        }
    }
    let mut is_bridge = vec![false; edges.len()];
    let mut disc = vec![usize::MAX; vertex_count];
    let mut low = vec![0usize; vertex_count];
    let mut timer = 0usize;
    for s in 0..vertex_count {
        if disc.get(s) != Some(&usize::MAX) {
            continue;
        }
        disc[s] = timer;
        low[s] = timer;
        timer += 1;
        // (vertex, edge used to enter it, next adjacency position)
        let mut stack: Vec<(usize, Option<usize>, usize)> = vec![(s, None, 0)];
        while let Some(top) = stack.last_mut() {
            let (v, entered_by, pos) = *top;
            if let Some(&(w, e)) = adj.get(v).and_then(|a| a.get(pos)) {
                top.2 += 1;
                if Some(e) == entered_by {
                    continue;
                }
                if disc[w] == usize::MAX {
                    disc[w] = timer;
                    low[w] = timer;
                    timer += 1;
                    stack.push((w, Some(e), 0));
                } else {
                    low[v] = low[v].min(disc[w]);
                }
            } else {
                stack.pop();
                if let Some(&(p, _, _)) = stack.last() {
                    low[p] = low[p].min(low[v]);
                    if low[v] > disc[p]
                        && let Some(e) = entered_by
                        && let Some(slot) = is_bridge.get_mut(e)
                    {
                        *slot = true;
                    }
                }
            }
        }
    }
    is_bridge
}

impl Sketch {
    /// The closed regions bounded by the non-construction lines, arcs and circles, each with
    /// a counter-clockwise outer loop and clockwise holes. Dangling curves are ignored.
    pub fn profiles(&self) -> Vec<Region> {
        // Vertices: endpoints of non-construction lines and arcs, merged.
        let mut endpoints: Vec<PointId> = Vec::new();
        for (_, c) in self.curves() {
            match c {
                Curve::Line {
                    start,
                    end,
                    construction: false,
                }
                | Curve::Arc {
                    start,
                    end,
                    construction: false,
                    ..
                }
                | Curve::EllipticalArc {
                    start,
                    end,
                    construction: false,
                    ..
                } => endpoints.extend([*start, *end]),
                Curve::Spline {
                    points,
                    closed: false,
                    construction: false,
                    ..
                } => {
                    if let (Some(s), Some(e)) = (points.first(), points.last()) {
                        endpoints.extend([*s, *e]);
                    }
                }
                _ => {}
            }
        }
        endpoints.sort_unstable();
        endpoints.dedup();
        let index: BTreeMap<PointId, usize> =
            endpoints.iter().enumerate().map(|(i, p)| (*p, i)).collect();
        let mut parent: Vec<usize> = (0..endpoints.len()).collect();
        for (_, c) in self.constraints() {
            if let Constraint::Coincident { a, b } = c
                && let (Some(&i), Some(&j)) = (index.get(a), index.get(b))
            {
                union(&mut parent, i, j);
            }
        }
        let mut by_x: Vec<(usize, Point2)> = endpoints
            .iter()
            .enumerate()
            .filter_map(|(i, id)| self.point(*id).map(|p| (i, p)))
            .collect();
        by_x.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        for (k, (i, p)) in by_x.iter().enumerate() {
            for (j, q) in by_x.iter().skip(k + 1) {
                if q.x - p.x > LINEAR_TOLERANCE {
                    break;
                }
                if p.coincident(*q) {
                    union(&mut parent, *i, *j);
                }
            }
        }
        let vertex = |parent: &mut [usize], id: PointId| index.get(&id).map(|&i| find(parent, i));

        // Edges.
        let mut edges = Vec::new();
        let mut circles: Vec<(CurveId, Geo)> = Vec::new();
        for (id, c) in self.curves() {
            if c.is_construction() {
                continue;
            }
            match c {
                Curve::Line { start, end, .. } => {
                    let (Some(u), Some(v), Some(a), Some(b)) = (
                        vertex(&mut parent, *start),
                        vertex(&mut parent, *end),
                        self.point(*start),
                        self.point(*end),
                    ) else {
                        continue;
                    };
                    if u != v {
                        edges.push(Edge {
                            curve: id,
                            u,
                            v,
                            geo: Geo::Line { a, b },
                        });
                    }
                }
                Curve::Arc {
                    centre, start, end, ..
                } => {
                    let (Some(u), Some(v), Some(c), Some(s), Some(e)) = (
                        vertex(&mut parent, *start),
                        vertex(&mut parent, *end),
                        self.point(*centre),
                        self.point(*start),
                        self.point(*end),
                    ) else {
                        continue;
                    };
                    if u != v
                        && let Some(geo) = arc_geo(c, s, e)
                    {
                        edges.push(Edge {
                            curve: id,
                            u,
                            v,
                            geo,
                        });
                    }
                }
                Curve::EllipticalArc { start, end, .. } => {
                    let (Some(u), Some(v), Some(geo)) = (
                        vertex(&mut parent, *start),
                        vertex(&mut parent, *end),
                        self.ellipse_arc_of(c),
                    ) else {
                        continue;
                    };
                    if u != v {
                        edges.push(Edge {
                            curve: id,
                            u,
                            v,
                            geo: Geo::Ellipse(geo),
                        });
                    }
                }
                Curve::Ellipse { .. } => {
                    if let Some(geo) = self.ellipse_arc_of(c) {
                        circles.push((id, Geo::Ellipse(geo)));
                    }
                }
                Curve::Spline { points, closed, .. } => {
                    let Some(curve) = self.spline_of(c) else {
                        continue;
                    };
                    let (Some(first), Some(last)) = (points.first(), points.last()) else {
                        continue;
                    };
                    let (Some(a), Some(b)) = (self.point(*first), self.point(*last)) else {
                        continue;
                    };
                    let curve = Arc::new(curve);
                    if *closed {
                        // A closed spline is a loop of its own, like a circle; both its
                        // ends are the curve's start.
                        let s = curve.point_at(curve.domain().0);
                        circles.push((id, Geo::Spline { curve, a: s, b: s }));
                    } else if let (Some(u), Some(v)) =
                        (vertex(&mut parent, *first), vertex(&mut parent, *last))
                        && u != v
                    {
                        edges.push(Edge {
                            curve: id,
                            u,
                            v,
                            geo: Geo::Spline { curve, a, b },
                        });
                    }
                }
                Curve::Circle { centre, radius, .. } => {
                    if let Some(c) = self.point(*centre) {
                        circles.push((
                            id,
                            Geo::Arc {
                                c,
                                r: *radius,
                                a0: 0.0,
                                sweep: TAU,
                            },
                        ));
                    }
                }
            }
        }
        let vcount = endpoints.len();
        let is_bridge = bridges(vcount, &edges);
        let kept: Vec<&Edge> = edges
            .iter()
            .zip(&is_bridge)
            .filter(|(_, b)| !**b)
            .map(|(e, _)| e)
            .collect();

        // Pieces (connected components of what is left).
        let mut piece_parent: Vec<usize> = (0..vcount).collect();
        for e in &kept {
            union(&mut piece_parent, e.u, e.v);
        }

        // Half-edges 2k (u → v) and 2k + 1 (v → u), sorted around each vertex.
        let he_count = kept.len() * 2;
        let from = |h: usize| {
            kept.get(h / 2)
                .map(|e| if h.is_multiple_of(2) { e.u } else { e.v })
        };
        let mut outgoing: Vec<Vec<(i64, f64, usize)>> = vec![Vec::new(); vcount];
        for h in 0..he_count {
            let (Some(e), Some(u)) = (kept.get(h / 2), from(h)) else {
                continue;
            };
            let (angle, curvature) = e.geo.leaving(h % 2 == 1);
            let bucket = (angle / TANGENT_BUCKET).round() as i64;
            if let Some(list) = outgoing.get_mut(u) {
                list.push((bucket, curvature, h));
            }
        }
        let mut position = vec![0usize; he_count];
        for list in &mut outgoing {
            list.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            for (i, item) in list.iter().enumerate() {
                if let Some(slot) = position.get_mut(item.2) {
                    *slot = i;
                }
            }
        }
        // next(h): at h's head, the outgoing half-edge just clockwise of h's twin.
        let next = |h: usize| -> Option<usize> {
            let twin = h ^ 1;
            let head = from(twin)?;
            let list = outgoing.get(head)?;
            let i = *position.get(twin)?;
            let j = if i == 0 {
                list.len().checked_sub(1)?
            } else {
                i - 1
            };
            list.get(j).map(|t| t.2)
        };

        let mut faces: Vec<Traced> = Vec::new();
        let mut boundaries: Vec<Traced> = Vec::new();
        let mut visited = vec![false; he_count];
        for h0 in 0..he_count {
            if visited.get(h0) != Some(&false) {
                continue;
            }
            let mut segments = Vec::new();
            let mut h = h0;
            let mut closed = false;
            for _ in 0..=he_count {
                if let Some(v) = visited.get_mut(h) {
                    *v = true;
                }
                let Some(e) = kept.get(h / 2) else { break };
                segments.push((e.curve, e.geo.clone(), h % 2 == 1));
                match next(h) {
                    Some(n) if n == h0 => {
                        closed = true;
                        break;
                    }
                    Some(n) => h = n,
                    None => break,
                }
            }
            if !closed {
                continue;
            }
            let area: f64 = segments.iter().map(|(_, g, r)| g.area(*r)).sum();
            let piece = kept
                .get(h0 / 2)
                .map(|e| find(&mut piece_parent, e.u))
                .unwrap_or(0);
            let t = Traced {
                segments,
                area,
                piece,
            };
            if area > MIN_AREA {
                faces.push(t);
            } else if area < -MIN_AREA {
                boundaries.push(t);
            }
        }
        // Each circle is a piece of its own: a face and a boundary.
        for (k, (id, geo)) in circles.iter().enumerate() {
            let piece = vcount + k;
            let area = geo.area(false);
            if area <= MIN_AREA {
                continue;
            }
            faces.push(Traced {
                segments: vec![(*id, geo.clone(), false)],
                area,
                piece,
            });
            boundaries.push(Traced {
                segments: vec![(*id, geo.clone(), true)],
                area: -area,
                piece,
            });
        }

        // Holes go to the smallest face of another piece that contains them.
        let mut holes: Vec<Vec<Traced>> = (0..faces.len()).map(|_| Vec::new()).collect();
        for b in boundaries {
            let Some((_, geo, _)) = b.segments.first() else {
                continue;
            };
            let probe = geo.point_at_fraction(PROBE_FRACTION);
            let owner = faces
                .iter()
                .enumerate()
                .filter(|(_, f)| f.piece != b.piece && f.contains(probe))
                .min_by(|x, y| x.1.area.total_cmp(&y.1.area))
                .map(|(i, _)| i);
            if let Some(list) = owner.and_then(|i| holes.get_mut(i)) {
                list.push(b);
            }
        }
        faces
            .into_iter()
            .zip(holes)
            .map(|(f, hs)| {
                let area = f.area + hs.iter().map(|h| h.area).sum::<f64>();
                Region {
                    outer: f.into_loop(),
                    holes: hs.into_iter().map(Traced::into_loop).collect(),
                    area,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(s: &mut Sketch, x: f64, y: f64) -> PointId {
        s.add_point(Point2::new(x, y)).unwrap()
    }

    /// A closed polygon through shared points.
    fn polygon(s: &mut Sketch, corners: &[(f64, f64)]) -> Vec<CurveId> {
        let ids: Vec<PointId> = corners.iter().map(|(x, y)| pt(s, *x, *y)).collect();
        (0..ids.len())
            .map(|i| s.add_line(ids[i], ids[(i + 1) % ids.len()]).unwrap())
            .collect()
    }

    #[test]
    fn a_rectangle_is_one_counter_clockwise_region() {
        let mut s = Sketch::new();
        // Drawn clockwise on purpose: the loop still comes out counter-clockwise.
        polygon(
            &mut s,
            &[(0.0, 0.0), (0.0, 30.0), (50.0, 30.0), (50.0, 0.0)],
        );
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area - 1500.0).abs() < 1e-9);
        assert!(regions[0].outer.signed_area > 0.0);
        assert_eq!(regions[0].outer.segments.len(), 4);
        assert!(regions[0].holes.is_empty());
    }

    #[test]
    fn a_circle_in_a_rectangle_is_a_hole_and_a_disc() {
        let mut s = Sketch::new();
        polygon(
            &mut s,
            &[(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)],
        );
        let c = pt(&mut s, 25.0, 15.0);
        let circle = s.add_circle(c, 5.0).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 2);
        let ring = regions.iter().find(|r| !r.holes.is_empty()).unwrap();
        // 50 × 30 - π 5².
        assert!((ring.area - (1500.0 - 25.0 * PI)).abs() < 1e-9);
        assert_eq!(ring.holes.len(), 1);
        let hole = &ring.holes[0];
        assert!(
            (hole.signed_area + 25.0 * PI).abs() < 1e-9,
            "clockwise hole"
        );
        assert_eq!(
            hole.segments,
            vec![LoopSegment {
                curve: circle,
                reversed: true
            }]
        );
        let disc = regions.iter().find(|r| r.holes.is_empty()).unwrap();
        assert!((disc.area - 25.0 * PI).abs() < 1e-9);
    }

    #[test]
    fn construction_and_dangling_curves_bound_nothing() {
        let mut s = Sketch::new();
        let lines = polygon(
            &mut s,
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        );
        // A dangling tail from a corner and a free segment.
        let corner = match s.curve(lines[0]).unwrap() {
            Curve::Line { end, .. } => *end,
            _ => unreachable!(),
        };
        let tail = pt(&mut s, 20.0, 0.0);
        s.add_line(corner, tail).unwrap();
        let a = pt(&mut s, 30.0, 30.0);
        let b = pt(&mut s, 40.0, 30.0);
        s.add_line(a, b).unwrap();
        // A construction circle.
        let c = pt(&mut s, 5.0, 5.0);
        let cc = s.add_circle(c, 2.0).unwrap();
        s.set_construction(cc, true).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area - 100.0).abs() < 1e-9);
        assert!(regions[0].holes.is_empty());
        // Making one side construction opens the loop.
        s.set_construction(lines[2], true).unwrap();
        assert!(s.profiles().is_empty());
    }

    #[test]
    fn two_squares_sharing_an_edge_give_two_regions() {
        // A T-junction at shared points: (10,0) and (10,10) are shared by three lines.
        let mut s = Sketch::new();
        let p: Vec<PointId> = [
            (0.0, 0.0),
            (10.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]
        .iter()
        .map(|(x, y)| pt(&mut s, *x, *y))
        .collect();
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0), (1, 4)] {
            s.add_line(p[a], p[b]).unwrap();
        }
        let regions = s.profiles();
        assert_eq!(regions.len(), 2);
        for r in &regions {
            assert!((r.area - 100.0).abs() < 1e-9);
            assert_eq!(r.outer.segments.len(), 4);
        }
    }

    #[test]
    fn a_slot_of_lines_and_arcs_has_the_exact_area() {
        // Slot: centres (0,0) and (20,0), radius 5. Area = 20·10 + π·5².
        let mut s = Sketch::new();
        let c1 = pt(&mut s, 0.0, 0.0);
        let c2 = pt(&mut s, 20.0, 0.0);
        let a = pt(&mut s, 0.0, -5.0);
        let b = pt(&mut s, 20.0, -5.0);
        let c = pt(&mut s, 20.0, 5.0);
        let d = pt(&mut s, 0.0, 5.0);
        s.add_line(a, b).unwrap();
        s.add_arc(c2, b, c).unwrap(); // right end, counter-clockwise from bottom to top
        s.add_line(c, d).unwrap();
        s.add_arc(c1, d, a).unwrap(); // left end, counter-clockwise from top to bottom
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area - (200.0 + 25.0 * PI)).abs() < 1e-9);
    }

    #[test]
    fn a_lens_of_two_arcs_between_the_same_points() {
        // Two arcs of radius √2 centred at (0,0) and (2,0)... instead use the classic
        // half-discs: arcs centred at the origin from (1,0) to (-1,0) and back, closing a
        // unit circle out of two edges between the same two vertices: area π.
        let mut s = Sketch::new();
        let o = pt(&mut s, 0.0, 0.0);
        let r = pt(&mut s, 1.0, 0.0);
        let l = pt(&mut s, -1.0, 0.0);
        s.add_arc(o, r, l).unwrap();
        s.add_arc(o, l, r).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area - PI).abs() < 1e-12);
    }

    #[test]
    fn nested_holes_go_to_the_smallest_container() {
        let mut s = Sketch::new();
        polygon(
            &mut s,
            &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        );
        polygon(
            &mut s,
            &[(10.0, 10.0), (90.0, 10.0), (90.0, 90.0), (10.0, 90.0)],
        );
        let c = pt(&mut s, 50.0, 50.0);
        s.add_circle(c, 10.0).unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 3);
        let mut areas: Vec<f64> = regions.iter().map(|r| r.area).collect();
        areas.sort_by(f64::total_cmp);
        // Disc 100π; inner square minus disc 6400 - 100π; frame 10000 - 6400.
        assert!((areas[0] - 100.0 * PI).abs() < 1e-9);
        assert!((areas[1] - 3600.0).abs() < 1e-9);
        assert!((areas[2] - (6400.0 - 100.0 * PI)).abs() < 1e-9);
        for r in &regions {
            assert!(r.holes.len() <= 1);
            for h in &r.holes {
                assert!(h.signed_area < 0.0);
            }
        }
    }

    #[test]
    fn coincident_endpoints_close_a_loop() {
        let mut s = Sketch::new();
        let a = pt(&mut s, 0.0, 0.0);
        let b = pt(&mut s, 10.0, 0.0);
        let b2 = pt(&mut s, 10.0, 0.0);
        let c = pt(&mut s, 0.0, 10.0);
        let c2 = pt(&mut s, 0.5, 10.0); // not at c yet, but constrained coincident
        let a2 = pt(&mut s, 0.0, 0.0);
        s.add_line(a, b).unwrap();
        s.add_line(b2, c).unwrap();
        s.add_line(c2, a2).unwrap();
        s.add_constraint(Constraint::Coincident { a: c, b: c2 })
            .unwrap();
        let regions = s.profiles();
        assert_eq!(regions.len(), 1);
    }

    #[test]
    fn an_empty_sketch_has_no_profiles() {
        assert!(Sketch::new().profiles().is_empty());
    }
}

#[cfg(test)]
mod hole_tests {
    use super::*;

    fn plate(s: &mut Sketch, w: f64, h: f64) {
        let p: Vec<PointId> = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4]).unwrap();
        }
    }

    /// Two r = 8 holes in an 80 x 50 plate, centred at the same height: the plate is one
    /// region of 4000 - 2 * 64π with two holes, plus a disc of 64π for each circle. Circle
    /// centres at one height used to make the inside test count one crossing, not two.
    #[test]
    fn two_circles_level_with_each_other_are_both_holes_of_the_plate() {
        let mut s = Sketch::new();
        plate(&mut s, 80.0, 50.0);
        for cx in [20.0, 60.0] {
            let c = s.add_point(Point2::new(cx, 25.0)).unwrap();
            s.add_circle(c, 8.0).unwrap();
        }
        let disc = 64.0 * PI;
        let mut regions = s.profiles();
        regions.sort_by(|a, b| b.area.total_cmp(&a.area));
        assert_eq!(regions.len(), 3);
        assert!((regions[0].area - (4000.0 - 2.0 * disc)).abs() < 1e-9);
        assert_eq!(regions[0].holes.len(), 2);
        for r in &regions[1..] {
            assert!((r.area - disc).abs() < 1e-9);
            assert!(r.holes.is_empty());
        }
    }

    /// Five r = 5 holes in rows and columns (many level and aligned pairs).
    #[test]
    fn a_grid_of_holes_all_belong_to_the_plate() {
        let mut s = Sketch::new();
        plate(&mut s, 100.0, 60.0);
        for (cx, cy) in [
            (20.0, 20.0),
            (50.0, 20.0),
            (80.0, 20.0),
            (20.0, 45.0),
            (50.0, 45.0),
        ] {
            let c = s.add_point(Point2::new(cx, cy)).unwrap();
            s.add_circle(c, 5.0).unwrap();
        }
        let regions = s.profiles();
        assert_eq!(regions.len(), 6);
        let plate = regions
            .iter()
            .max_by(|a, b| a.area.total_cmp(&b.area))
            .unwrap();
        assert_eq!(plate.holes.len(), 5);
        assert!((plate.area - (6000.0 - 5.0 * 25.0 * PI)).abs() < 1e-9);
    }
}

/// Segments per full turn when a loop is turned into a polyline for containment tests.
const LOOP_SEGMENTS_PER_TURN: f64 = 128.0;

impl Sketch {
    /// A loop as a closed polyline (first point not repeated), arcs subdivided. Returns an
    /// empty list if a curve of the loop is gone.
    pub fn loop_polyline(&self, l: &ProfileLoop) -> Vec<Point2> {
        let mut out = Vec::new();
        for seg in &l.segments {
            let mut pts: Vec<Point2> = match self.curve(seg.curve) {
                Some(Curve::Line { start, end, .. }) => {
                    match (self.point(*start), self.point(*end)) {
                        (Some(a), Some(b)) => vec![a, b],
                        _ => return Vec::new(),
                    }
                }
                Some(Curve::Circle { centre, radius, .. }) => match self.point(*centre) {
                    Some(c) => {
                        let n = LOOP_SEGMENTS_PER_TURN as usize;
                        (0..=n)
                            .map(|k| {
                                let a = TAU * k as f64 / n as f64;
                                c + Vec2::new(a.cos(), a.sin()) * *radius
                            })
                            .collect()
                    }
                    None => return Vec::new(),
                },
                Some(Curve::Arc {
                    centre, start, end, ..
                }) => {
                    let (Some(c), Some(s), Some(e)) =
                        (self.point(*centre), self.point(*start), self.point(*end))
                    else {
                        return Vec::new();
                    };
                    let Some(Geo::Arc { c, r, a0, sweep }) = arc_geo(c, s, e) else {
                        return Vec::new();
                    };
                    let n = ((LOOP_SEGMENTS_PER_TURN * sweep / TAU).ceil() as usize).max(2);
                    (0..=n)
                        .map(|k| {
                            let a = a0 + sweep * k as f64 / n as f64;
                            c + Vec2::new(a.cos(), a.sin()) * r
                        })
                        .collect()
                }
                Some(c @ Curve::Spline { .. }) => match self.spline_of(c) {
                    Some(curve) => spline_polyline(&curve),
                    None => return Vec::new(),
                },
                Some(c @ (Curve::Ellipse { .. } | Curve::EllipticalArc { .. })) => {
                    match self.ellipse_arc_of(c) {
                        Some(e) => e.polyline(LOOP_SEGMENTS_PER_TURN as usize),
                        None => return Vec::new(),
                    }
                }
                None => return Vec::new(),
            };
            if seg.reversed {
                pts.reverse();
            }
            // Each segment's last point is the next one's first.
            pts.pop();
            out.extend(pts);
        }
        out
    }

    /// The region that contains `p` (inside its outer loop, outside its holes), the
    /// smallest if several do. This is how features refer to a profile across edits: by a
    /// point inside it, not by an index that changes when the sketch changes.
    pub fn region_at(&self, p: Point2) -> Option<Region> {
        self.profiles()
            .into_iter()
            .filter(|r| self.region_contains(r, p))
            .min_by(|a, b| a.area.total_cmp(&b.area))
    }

    /// True when `p` is inside the region's outer loop and outside all its holes.
    pub fn region_contains(&self, r: &Region, p: Point2) -> bool {
        polygon_contains(&self.loop_polyline(&r.outer), p)
            && !r
                .holes
                .iter()
                .any(|h| polygon_contains(&self.loop_polyline(h), p))
    }

    /// A point well inside the region (not on or near its boundary): the middle of the
    /// widest inside span along a horizontal line through the region. Used to remember
    /// which profile a feature uses.
    pub fn region_interior_point(&self, r: &Region) -> Option<Point2> {
        let loops: Vec<Vec<Point2>> = std::iter::once(&r.outer)
            .chain(r.holes.iter())
            .map(|l| self.loop_polyline(l))
            .collect();
        let outer = loops.first()?;
        let (lo, hi) = outer
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.y), hi.max(p.y))
            });
        let span = hi - lo;
        if !span.is_finite() || span <= LINEAR_TOLERANCE {
            return None;
        }
        let mut best: Option<(f64, Point2)> = None;
        // Lines at fractions that avoid vertices on round numbers.
        for k in 0..23 {
            let y = lo + (hi - lo) * ((k as f64 + 0.5) / 23.0 + 0.003);
            let mut xs: Vec<f64> = Vec::new();
            for poly in &loops {
                for i in 0..poly.len() {
                    let (Some(a), Some(b)) = (poly.get(i), poly.get((i + 1) % poly.len())) else {
                        continue;
                    };
                    if (a.y > y) != (b.y > y) {
                        xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                    }
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.chunks(2) {
                if let [x0, x1] = pair {
                    let w = x1 - x0;
                    let mid = Point2::new((x0 + x1) * 0.5, y);
                    if best.is_none_or(|(bw, _)| w > bw) && self.region_contains(r, mid) {
                        best = Some((w, mid));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }
}

/// Even-odd point-in-polygon test for a closed polyline.
fn polygon_contains(poly: &[Point2], p: Point2) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + 1) % n)) else {
            continue;
        };
        if line_crosses(*a, *b, p) {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod region_lookup_tests {
    use super::*;

    /// An 80 x 50 plate with a centred r = 10 hole: the plate's interior point is not in
    /// the hole (where a centroid would be), and looking it up finds the plate again; the
    /// hole's centre finds the disc.
    #[test]
    fn interior_points_find_their_region_again() {
        let mut s = Sketch::new();
        let p: Vec<PointId> = [(0.0, 0.0), (80.0, 0.0), (80.0, 50.0), (0.0, 50.0)]
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4]).unwrap();
        }
        let c = s.add_point(Point2::new(40.0, 25.0)).unwrap();
        s.add_circle(c, 10.0).unwrap();
        let plate = s
            .profiles()
            .into_iter()
            .max_by(|a, b| a.area.total_cmp(&b.area))
            .unwrap();
        let seed = s.region_interior_point(&plate).unwrap();
        assert!(
            seed.distance(Point2::new(40.0, 25.0)) > 10.0,
            "{seed:?} is in the hole"
        );
        let again = s.region_at(seed).unwrap();
        assert!((again.area - (4000.0 - 100.0 * PI)).abs() < 1e-9);
        let disc = s.region_at(Point2::new(40.0, 25.0)).unwrap();
        assert!((disc.area - 100.0 * PI).abs() < 1e-9);
        assert!(s.region_at(Point2::new(200.0, 0.0)).is_none());
        // The loop polyline of the plate has its 4 corners, the circle 128 points.
        assert_eq!(s.loop_polyline(&plate.outer).len(), 4);
        assert_eq!(s.loop_polyline(&disc.outer).len(), 128);
    }
}

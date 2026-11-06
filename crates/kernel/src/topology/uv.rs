//! Faces seen in their surface's parameter plane: loop images and the inside-face test.
//!
//! # Loop images
//! A loop of a face on a curved surface is mapped into the `(u, v)` plane by sampling
//! each edge along its half-edge and projecting the samples
//! ([`Surface::parameters`]). The image is unwrapped: across a periodic direction each
//! sample takes the representative nearest to the previous one, and each edge is shifted
//! by whole periods to continue where the previous edge ended. A seam edge (used twice by
//! one face) therefore appears twice, one period apart.
//!
//! At a singular point (a sphere's pole, a cone's apex, a spindle torus's tip) `u` is
//! undefined: the sample takes `u` from its neighbour on the same edge, and the image
//! joins the two edges with a straight step along the singular line `v = v_s`. The step's
//! direction follows from the face's orientation: with the outward normal along
//! `S_u × S_v` (`same_sense`), the face lies to the left of its loops in `(u, v)`, so a
//! step along a singular line above the face (the face approaches it from smaller `v`)
//! runs toward -u, one below it toward +u, by more than zero and at most one period.
//!
//! An image is *closed* when it ends where it starts. A loop that goes once around a
//! periodic direction without a seam does not close; the measures accept such loops in
//! `u` (see `measure.rs`), the inside test and the orientation check do not.
//!
//! The images of the edges K4 builds (lines along the axis or through it, circles about
//! the axis, profile curves in a plane through the axis) are straight lines in `(u, v)`,
//! so the sampled image is exact. A general edge's image is a polyline through its
//! samples ([`samples_for`]).
//!
//! # Inside-face test
//! [`Body::classify_point_on_face`]: a point farther than [`LINEAR_TOLERANCE`] from the
//! surface is outside; one within it of an edge or vertex is on the boundary; otherwise
//! its `(u, v)` (each periodic representative within the image's range) is tested by ray
//! crossing (even-odd over all loops, half-open in `v`) against the closed loop images.
//! On a plane the images of circles and arcs are exact circular arcs and splines are crossed
//! exactly ([`chamfera_geom::BSplineCurve2::ray_crossings`]); elsewhere they are
//! the sampled polylines, exact for straight images.

use super::{Body, Curve, EdgeId, FaceId, LoopId, Surface};
use crate::{KernelError, KernelResult};
use chamfera_geom::{LINEAR_TOLERANCE, Point3};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Two unwrapped parameters closer than this (radians, or mm for a cylinder's height) are
/// the same: images are built from exact projections, so only rounding separates them.
const IMAGE_TOLERANCE: f64 = 1e-9;
/// Samples per knot span of a spline edge's image.
const SPLINE_SAMPLES_PER_SPAN: usize = 32;

/// Where a point lies relative to a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointOnFace {
    Inside,
    /// Within [`LINEAR_TOLERANCE`] of an edge or vertex of the face.
    Boundary,
    /// Not on the face (including points off its surface).
    Outside,
}

/// One edge of a loop image.
#[derive(Debug, Clone)]
pub(crate) struct EdgeImage {
    pub edge: EdgeId,
    pub curve: Curve,
    pub t0: f64,
    pub t1: f64,
    /// The half-edge runs along the curve (from `t0` to `t1`).
    pub forward: bool,
    /// Unwrapped `(u, v)` of the samples, in half-edge order.
    pub uv: Vec<(f64, f64)>,
    pub singular_start: bool,
    pub singular_end: bool,
}

/// The image of one loop.
#[derive(Debug, Clone)]
pub(crate) struct LoopImage {
    pub edges: Vec<EdgeImage>,
    /// The image ends where it starts.
    pub closed: bool,
    /// The image goes around the periodic `v` direction (a torus loop without a seam).
    pub wraps_v: bool,
}

/// A step of a loop image along a singular line: from `u0` to `u1` at `v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PoleStep {
    pub v: f64,
    pub u0: f64,
    pub u1: f64,
}

impl LoopImage {
    /// Every sample point in order (joints appear twice; harmless for areas and ray
    /// crossings).
    pub fn points(&self) -> Vec<(f64, f64)> {
        self.edges
            .iter()
            .flat_map(|e| e.uv.iter().copied())
            .collect()
    }

    /// The steps along singular lines between consecutive edges (and from the last edge
    /// back to the first when the image is closed).
    pub fn pole_steps(&self) -> Vec<PoleStep> {
        let n = self.edges.len();
        let mut out = Vec::new();
        for i in 0..n {
            if i + 1 == n && !self.closed {
                break;
            }
            let (Some(a), Some(b)) = (self.edges.get(i), self.edges.get((i + 1) % n)) else {
                continue;
            };
            if a.singular_end
                && b.singular_start
                && let (Some(pa), Some(pb)) = (a.uv.last(), b.uv.first())
                && (pa.0 - pb.0).abs() > IMAGE_TOLERANCE
            {
                out.push(PoleStep {
                    v: pa.1,
                    u0: pa.0,
                    u1: pb.0,
                });
            }
        }
        out
    }

    /// Signed area of a closed image (shoelace), positive counter-clockwise.
    pub fn signed_area(&self) -> f64 {
        let p = self.points();
        let n = p.len();
        let mut sum = 0.0;
        for i in 0..n {
            let (a, b) = (p[i], p[(i + 1) % n]);
            sum += a.0 * b.1 - b.0 * a.1;
        }
        0.5 * sum
    }
}

/// How many intervals an edge is sampled with: enough that consecutive samples are less
/// than a sixteenth of a turn apart around any axis (so unwrapping is unambiguous), at
/// least 8.
pub(super) fn samples_for(curve: &Curve, t0: f64, t1: f64) -> usize {
    if let Curve::BSpline(c) = curve {
        // A spline's image bends within each knot span: a fixed number per span.
        let (lo, hi) = (t0.min(t1), t0.max(t1));
        let spans = c
            .breaks()
            .windows(2)
            .filter(|w| w[1] > lo && w[0] < hi)
            .count()
            .max(1);
        return (spans * SPLINE_SAMPLES_PER_SPAN).clamp(16, 2048);
    }
    let turns = match curve {
        Curve::Line(_) | Curve::Segment(_) => 0.0,
        _ => (t1 - t0).abs() / TAU,
    };
    let n = (turns * 64.0).ceil();
    if n.is_finite() && n > 8.0 {
        (n as usize).min(4096)
    } else {
        8
    }
}

/// The representative of `x + k p` nearest to `target`.
fn nearest(x: f64, target: f64, p: f64) -> f64 {
    x + ((target - x) / p).round() * p
}

/// `step + k p` for the integer `k` that puts it in `(0, p]` (`dir > 0`) or `[-p, 0)`
/// (`dir < 0`), treating values within [`IMAGE_TOLERANCE`] of 0 as 0.
fn directed(step: f64, dir: f64, p: f64) -> f64 {
    if dir > 0.0 {
        let k = ((IMAGE_TOLERANCE - step) / p).floor() + 1.0;
        step + k * p
    } else {
        let k = ((-IMAGE_TOLERANCE - step) / p).ceil() - 1.0;
        step + k * p
    }
}

/// The required pole step's direction: `+1` toward +u, `-1` toward -u, 0 if unknown.
/// `v_before` is a regular sample next to the singular line `v_s` on the face's side.
fn pole_direction(v_before: f64, v_s: f64, orient: f64) -> f64 {
    let side = v_before - v_s;
    if side.abs() <= IMAGE_TOLERANCE {
        0.0
    } else {
        side.signum() * orient
    }
}

impl Body {
    /// The image of loop `lp` on `surface`, for a face whose outward normal is
    /// `orient * (S_u × S_v)` (`orient` = ±1).
    pub(crate) fn loop_image(
        &self,
        lp: LoopId,
        surface: &Surface,
        orient: f64,
    ) -> KernelResult<LoopImage> {
        let pu = surface.period_u();
        let pv = surface.period_v();
        let mut edges: Vec<EdgeImage> = Vec::new();
        for h in self.loop_half_edges(lp)? {
            let Some(e) = self.half_edge_edge(h)? else {
                return Err(KernelError::Unsupported(format!(
                    "{lp} is a lone vertex; a loop image needs edges"
                )));
            };
            let (curve, t0, t1) = self
                .edge_curve(e)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve yet")))?;
            let forward = self.edge_half_edges(e)?[0] == h;
            let n = samples_for(&curve, t0, t1);
            let mut raw = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let s = i as f64 / n as f64;
                let t = if forward {
                    t0 + (t1 - t0) * s
                } else {
                    t1 - (t1 - t0) * s
                };
                let p = curve.point_at(t);
                if !p.is_valid() {
                    return Err(KernelError::InvalidInput(format!(
                        "{e} has points outside the working range"
                    )));
                }
                raw.push(surface.parameters(p));
            }
            if raw.iter().all(|q| q.singular) {
                return Err(KernelError::InvalidInput(format!(
                    "{e} lies on the axis of its face's surface"
                )));
            }
            // Singular samples take u from their nearest regular neighbour.
            let regular: Vec<usize> = (0..raw.len()).filter(|i| !raw[*i].singular).collect();
            let mut uv: Vec<(f64, f64)> = raw.iter().map(|q| (q.u, q.v)).collect();
            for (i, q) in raw.iter().enumerate() {
                if q.singular
                    && let Some(j) = regular.iter().min_by_key(|j| j.abs_diff(i))
                {
                    uv[i].0 = raw[*j].u;
                }
            }
            // Unwrap along the edge.
            for i in 1..uv.len() {
                let (pu_prev, pv_prev) = uv[i - 1];
                if let Some(p) = pu {
                    uv[i].0 = nearest(uv[i].0, pu_prev, p);
                }
                if let Some(p) = pv {
                    uv[i].1 = nearest(uv[i].1, pv_prev, p);
                }
            }
            let singular_start = raw.first().is_some_and(|q| q.singular);
            let singular_end = raw.last().is_some_and(|q| q.singular);
            // Continue where the previous edge ended.
            if let Some(prev) = edges.last()
                && let (Some(&(lu, lv)), Some(&(fu, fv))) = (prev.uv.last(), uv.first())
            {
                let mut du = 0.0;
                let mut dv = 0.0;
                if let Some(p) = pv {
                    dv = nearest(fv, lv, p) - fv;
                }
                if let Some(p) = pu {
                    let dir = if prev.singular_end && singular_start {
                        let before = prev.uv.len().checked_sub(2).and_then(|k| prev.uv.get(k));
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
            edges.push(EdgeImage {
                edge: e,
                curve,
                t0,
                t1,
                forward,
                uv,
                singular_start,
                singular_end,
            });
        }
        let (first, last) = match (edges.first(), edges.last()) {
            (Some(f), Some(l)) => (f, l),
            _ => {
                return Err(KernelError::InvalidInput(format!("{lp} has no edges")));
            }
        };
        let (Some(&(fu, fv)), Some(&(lu, lv))) = (first.uv.first(), last.uv.last()) else {
            return Err(KernelError::InvalidInput(format!("{lp} has no samples")));
        };
        let wraps_v = pv.is_some() && (fv - lv).abs() > IMAGE_TOLERANCE;
        let du = fu - lu;
        let closed_u = match pu {
            None => true,
            Some(p) => {
                let dir = if last.singular_end && first.singular_start {
                    let before = last.uv.len().checked_sub(2).and_then(|k| last.uv.get(k));
                    before.map_or(0.0, |b| pole_direction(b.1, lv, orient))
                } else {
                    0.0
                };
                if dir == 0.0 {
                    du.abs() <= IMAGE_TOLERANCE
                } else {
                    (directed(du, dir, p) - du).abs() <= IMAGE_TOLERANCE
                }
            }
        };
        Ok(LoopImage {
            edges,
            closed: closed_u && !wraps_v,
            wraps_v,
        })
    }

    /// The outward orientation of a face relative to its surface: `+1` when the outward
    /// normal is `S_u × S_v`, else `-1`.
    pub(crate) fn face_orientation(&self, face: FaceId) -> KernelResult<(Surface, f64)> {
        let (surface, same) = self
            .face_surface(face)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{face} has no surface yet")))?;
        Ok((surface, if same { 1.0 } else { -1.0 }))
    }

    /// Where `p` lies relative to `face`: inside, on its boundary, or outside (see the
    /// module documentation for the method). Needs a surface on the face and a curve on
    /// every edge; faces on curved surfaces whose loops do not close in `(u, v)` (no
    /// seam) and points at a singular point inside a face are not supported yet.
    pub fn classify_point_on_face(&self, face: FaceId, p: Point3) -> KernelResult<PointOnFace> {
        crate::check::point3("point", p)?;
        let (surface, orient) = self.face_orientation(face)?;
        if surface.distance(p) > LINEAR_TOLERANCE {
            return Ok(PointOnFace::Outside);
        }
        for e in self.face_edges(face)? {
            let (c, t0, t1) = self
                .edge_curve(e)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve yet")))?;
            if c.distance_on(t0, t1, p)? <= LINEAR_TOLERANCE {
                return Ok(PointOnFace::Boundary);
            }
        }
        for v in self.face_vertices(face)? {
            if self.vertex_point(v)?.coincident(p) {
                return Ok(PointOnFace::Boundary);
            }
        }
        let q = surface.parameters(p);
        let loops = self.face_loops(face)?;
        if let Surface::Plane(frame) = surface {
            let mut pieces = Vec::new();
            for l in loops {
                pieces.extend(self.planar_pieces(l, &frame)?);
            }
            let odd = pieces
                .iter()
                .map(|piece| piece.crossings(q.u, q.v))
                .sum::<usize>()
                % 2
                == 1;
            return Ok(if odd {
                PointOnFace::Inside
            } else {
                PointOnFace::Outside
            });
        }
        if q.singular {
            return Err(KernelError::Unsupported(
                "classifying a point on the axis of a curved face".into(),
            ));
        }
        let mut images = Vec::new();
        for l in loops {
            let img = self.loop_image(l, &surface, orient)?;
            if !img.closed {
                return Err(KernelError::Unsupported(format!(
                    "{l} goes around its surface without a seam; the inside test needs closed (u, v) loops"
                )));
            }
            images.push(img.points());
        }
        let bounds = |k: usize| {
            images
                .iter()
                .flatten()
                .map(|p| if k == 0 { p.0 } else { p.1 })
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                    (lo.min(x), hi.max(x))
                })
        };
        let candidates = |x: f64, period: Option<f64>, (lo, hi): (f64, f64)| -> Vec<f64> {
            match period {
                None => vec![x],
                Some(p) => {
                    let mut out = Vec::new();
                    let mut y = x + ((lo - x) / p).floor() * p;
                    while y <= hi + p && out.len() < 4 {
                        if y >= lo - IMAGE_TOLERANCE && y <= hi + IMAGE_TOLERANCE {
                            out.push(y);
                        }
                        y += p;
                    }
                    out
                }
            }
        };
        for u in candidates(q.u, surface.period_u(), bounds(0)) {
            for v in candidates(q.v, surface.period_v(), bounds(1)) {
                let crossings: usize = images.iter().map(|pts| polyline_crossings(pts, u, v)).sum();
                if crossings % 2 == 1 {
                    return Ok(PointOnFace::Inside);
                }
            }
        }
        Ok(PointOnFace::Outside)
    }

    /// The vertices of a face's loops.
    fn face_vertices(&self, face: FaceId) -> KernelResult<Vec<super::VertexId>> {
        let mut out = Vec::new();
        for l in self.face_loops(face)? {
            out.extend(self.loop_vertices(l)?);
        }
        Ok(out)
    }

    /// The exact images of a planar loop's edges in the plane's coordinates, as pieces
    /// monotone in `v`. Every piece that ends at a vertex uses that vertex's own `(u, v)`,
    /// so neighbouring pieces agree exactly there and the half-open crossing rule holds.
    fn planar_pieces(&self, lp: LoopId, frame: &chamfera_geom::Frame) -> KernelResult<Vec<Piece>> {
        let uv = |p: Point3| {
            let q = frame.to_plane(p);
            (q.x, q.y)
        };
        let mut out = Vec::new();
        for h in self.loop_half_edges(lp)? {
            let Some(e) = self.half_edge_edge(h)? else {
                continue;
            };
            let (c, t0, t1) = self
                .edge_curve(e)?
                .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve yet")))?;
            let forward = self.edge_half_edges(e)?[0] == h;
            let start = uv(self.vertex_point(self.half_edge_origin(h)?)?);
            let end = uv(self.vertex_point(self.half_edge_end(h)?)?);
            let (ta, tb) = if forward { (t0, t1) } else { (t1, t0) };
            let circle = match &c {
                Curve::Circle(k) => Some((*k.frame(), k.radius())),
                Curve::Arc(a) => Some((*a.circle().frame(), a.circle().radius())),
                _ => None,
            };
            if let Some((cf, r)) = circle {
                let centre = uv(cf.origin());
                let x = frame.vector_to_local(cf.x());
                let phase = x.y.atan2(x.x);
                let sense = if cf.z().dot(frame.z()) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                let (a0, a1) = (phase + sense * ta, phase + sense * tb);
                // Cut at the circle's top and bottom (angles π/2 + kπ), in travel order.
                let (lo, hi) = (a0.min(a1), a0.max(a1));
                let mut cuts = Vec::new();
                let mut k = ((lo - FRAC_PI_2) / PI).floor() + 1.0;
                while FRAC_PI_2 + k * PI < hi && cuts.len() < 8 {
                    cuts.push(FRAC_PI_2 + k * PI);
                    k += 1.0;
                }
                if a1 < a0 {
                    cuts.reverse();
                }
                let mut angles = vec![a0];
                angles.extend(cuts);
                angles.push(a1);
                let n = angles.len();
                for (i, w) in angles.windows(2).enumerate() {
                    let at = |a: f64| (centre.0 + r * a.cos(), centre.1 + r * a.sin());
                    let pa = if i == 0 { start } else { at(w[0]) };
                    let pb = if i + 2 == n { end } else { at(w[1]) };
                    out.push(Piece::Arc {
                        a: pa,
                        b: pb,
                        centre,
                        radius: r,
                        side: (0.5 * (w[0] + w[1])).cos().signum(),
                    });
                }
            } else if let Curve::BSpline(sc) = &c {
                // Exact: the spline in the plane's coordinates, its own ray crossings.
                let plane = sc.to_plane(frame)?;
                let (lo, hi) = (t0.min(t1), t0.max(t1));
                let (dlo, dhi) = plane.domain();
                let piece = if lo <= dlo && hi >= dhi {
                    plane
                } else {
                    plane.segment(lo.max(dlo), hi.min(dhi))?
                };
                let (a, b) = if forward { (start, end) } else { (end, start) };
                out.push(Piece::Spline {
                    curve: piece,
                    a: chamfera_geom::Point2::new(a.0, a.1),
                    b: chamfera_geom::Point2::new(b.0, b.1),
                });
            } else {
                let n = samples_for(&c, t0, t1);
                let mut pts = vec![start];
                if !matches!(c, Curve::Line(_) | Curve::Segment(_)) {
                    for i in 1..n {
                        let s = i as f64 / n as f64;
                        pts.push(uv(c.point_at(ta + (tb - ta) * s)));
                    }
                }
                pts.push(end);
                for w in pts.windows(2) {
                    out.push(Piece::Segment(w[0], w[1]));
                }
            }
        }
        Ok(out)
    }
}

/// A piece of a planar loop's image, monotone in `v`.
enum Piece {
    Segment((f64, f64), (f64, f64)),
    /// The arc of the circle about `centre` from `a` to `b` on the half where
    /// `sign(u - centre.u) = side`.
    Arc {
        a: (f64, f64),
        b: (f64, f64),
        centre: (f64, f64),
        radius: f64,
        side: f64,
    },
    /// A spline from `a` (its domain's start) to `b`, crossed exactly.
    Spline {
        curve: chamfera_geom::BSplineCurve2,
        a: chamfera_geom::Point2,
        b: chamfera_geom::Point2,
    },
}

impl Piece {
    /// Crossings (0 or 1) of the ray from `(u, v)` toward +u, half-open in `v`.
    fn crossings(&self, u: f64, v: f64) -> usize {
        if let Piece::Spline { curve, a, b } = self {
            return curve.ray_crossings(*a, *b, chamfera_geom::Point2::new(u, v));
        }
        let hit = match *self {
            Piece::Spline { .. } => false,
            Piece::Segment(a, b) => {
                (a.1 > v) != (b.1 > v) && u < a.0 + (v - a.1) * (b.0 - a.0) / (b.1 - a.1)
            }
            Piece::Arc {
                a,
                b,
                centre,
                radius,
                side,
            } => {
                (a.1 > v) != (b.1 > v) && {
                    let dy = v - centre.1;
                    u < centre.0 + side * (radius * radius - dy * dy).max(0.0).sqrt()
                }
            }
        };
        usize::from(hit)
    }
}

/// Crossings of the ray from `(u, v)` toward +u with the open polyline `p`.
fn open_polyline_crossings(p: &[(f64, f64)], u: f64, v: f64) -> usize {
    p.windows(2)
        .filter(|w| {
            let (a, b) = (w[0], w[1]);
            (a.1 > v) != (b.1 > v) && u < a.0 + (v - a.1) * (b.0 - a.0) / (b.1 - a.1)
        })
        .count()
}

/// Crossings with the closed polygon through `p`. Points within [`IMAGE_TOLERANCE`] of
/// the previous one (the two images of a joint, apart only by rounding) are dropped, so
/// the polygon passes through each joint once and the half-open rule holds there.
fn polyline_crossings(p: &[(f64, f64)], u: f64, v: f64) -> usize {
    let mut closed: Vec<(f64, f64)> = Vec::with_capacity(p.len() + 1);
    for q in p {
        let same = closed.last().is_some_and(|l| {
            (l.0 - q.0).abs() <= IMAGE_TOLERANCE && (l.1 - q.1).abs() <= IMAGE_TOLERANCE
        });
        if !same {
            closed.push(*q);
        }
    }
    if let Some(first) = closed.first().copied() {
        closed.push(first);
    }
    open_polyline_crossings(&closed, u, v)
}

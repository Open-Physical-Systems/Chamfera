//! Edge geometry for the boolean: a carrier curve (line, circle, ellipse) and a parameter
//! span on it, with projection, splitting and curve-curve crossings on a face.

use crate::{
    Circle3, Curve, CurveSurfaceIntersection, Ellipse3, KernelError, KernelResult, Line3, Segment3,
    Surface, intersect_curve_surface_on,
};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::TAU;

/// The full curve an edge piece lies on. Segments become lines (parameter = distance
/// from the line's origin) and arcs their circles, so pieces of one edge share a carrier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Carrier {
    Line(Line3),
    Circle(Circle3),
    Ellipse(Ellipse3),
}

impl Carrier {
    /// The carrier and parameter range of an edge curve over `[t0, t1]`.
    pub fn from_curve(c: &Curve, t0: f64, t1: f64) -> KernelResult<(Carrier, f64, f64)> {
        Ok(match c {
            Curve::Line(l) => (Carrier::Line(*l), t0, t1),
            Curve::Segment(s) => {
                let d = s.end() - s.start();
                let len = d.length();
                (Carrier::Line(Line3::new(s.start(), d)?), t0 * len, t1 * len)
            }
            Curve::Circle(k) => (Carrier::Circle(*k), t0, t1),
            Curve::Arc(a) => (Carrier::Circle(*a.circle()), t0, t1),
            Curve::Ellipse(e) => (Carrier::Ellipse(*e), t0, t1),
            Curve::EllipticalArc(a) => (Carrier::Ellipse(*a.ellipse()), t0, t1),
            Curve::BSpline(_) => {
                return Err(KernelError::Unsupported(
                    "booleans on solids with spline edges (K10 surfaces come later)".into(),
                ));
            }
        })
    }

    /// The carrier as a topology curve over its whole range.
    pub fn curve(&self) -> Curve {
        match self {
            Carrier::Line(l) => Curve::Line(*l),
            Carrier::Circle(c) => Curve::Circle(*c),
            Carrier::Ellipse(e) => Curve::Ellipse(*e),
        }
    }

    pub fn periodic(&self) -> bool {
        !matches!(self, Carrier::Line(_))
    }

    pub fn point(&self, t: f64) -> Point3 {
        match self {
            Carrier::Line(l) => l.point_at(t),
            Carrier::Circle(c) => c.point_at(t),
            Carrier::Ellipse(e) => e.point_at(t),
        }
    }

    pub fn d1(&self, t: f64) -> Vec3 {
        match self {
            Carrier::Line(l) => l.d1(t),
            Carrier::Circle(c) => c.d1(t),
            Carrier::Ellipse(e) => e.d1(t),
        }
    }

    /// The parameter of the carrier point nearest to `p` (in `[0, 2π)` on a closed
    /// carrier) and its distance from `p`.
    pub fn project(&self, p: Point3) -> KernelResult<(f64, f64)> {
        let c = match self {
            Carrier::Line(l) => l.closest_point(p)?,
            Carrier::Circle(c) => c.closest_point(p)?,
            Carrier::Ellipse(e) => e.closest_point(p)?,
        };
        Ok((c.t, c.distance))
    }

    /// The plane of a circle or ellipse.
    pub fn plane(&self) -> Option<Frame> {
        match self {
            Carrier::Line(_) => None,
            Carrier::Circle(c) => Some(*c.frame()),
            Carrier::Ellipse(e) => Some(*e.frame()),
        }
    }
}

/// A piece of a carrier: `t0 < t1`, at most one turn on a closed carrier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Span {
    pub carrier: Carrier,
    pub t0: f64,
    pub t1: f64,
}

impl Span {
    pub fn new(carrier: Carrier, t0: f64, t1: f64) -> Span {
        Span { carrier, t0, t1 }
    }

    pub fn from_curve(c: &Curve, t0: f64, t1: f64) -> KernelResult<Span> {
        let (carrier, t0, t1) = Carrier::from_curve(c, t0, t1)?;
        Ok(Span { carrier, t0, t1 })
    }

    pub fn start(&self) -> Point3 {
        self.carrier.point(self.t0)
    }
    pub fn end(&self) -> Point3 {
        self.carrier.point(self.t1)
    }
    /// The point at fraction `s` of the span.
    pub fn at(&self, s: f64) -> Point3 {
        self.carrier.point(self.t0 + (self.t1 - self.t0) * s)
    }
    pub fn mid(&self) -> Point3 {
        self.at(0.5)
    }

    /// Closed: a full turn of a circle or ellipse.
    pub fn is_full_turn(&self) -> bool {
        self.carrier.periodic() && self.t1 - self.t0 >= TAU - 1e-12
    }

    /// The parameter of `p` within the span, if `p` lies on it within
    /// [`LINEAR_TOLERANCE`]. Points at an end return that end's parameter exactly.
    pub fn param_of(&self, p: Point3) -> KernelResult<Option<f64>> {
        if p.coincident(self.start()) {
            return Ok(Some(self.t0));
        }
        if p.coincident(self.end()) {
            return Ok(Some(self.t1));
        }
        let (t, d) = self.carrier.project(p)?;
        if d > LINEAR_TOLERANCE {
            return Ok(None);
        }
        let t = if self.carrier.periodic() {
            self.t0 + (t - self.t0).rem_euclid(TAU)
        } else {
            t
        };
        Ok((t > self.t0 && t < self.t1).then_some(t))
    }

    /// The distance from `p` to the span.
    pub fn distance(&self, p: Point3) -> KernelResult<f64> {
        let ends = self.start().distance(p).min(self.end().distance(p));
        let (t, d) = self.carrier.project(p)?;
        let t = if self.carrier.periodic() {
            self.t0 + (t - self.t0).rem_euclid(TAU)
        } else {
            t
        };
        Ok(if t >= self.t0 && t <= self.t1 {
            d.min(ends)
        } else {
            ends
        })
    }

    pub fn bounding_box(&self) -> KernelResult<Aabb> {
        let b = self.curve_on().bounding_box_on(self.t0, self.t1)?;
        Ok(b)
    }

    /// The carrier as a curve the span's range is valid on.
    fn curve_on(&self) -> Curve {
        self.carrier.curve()
    }

    /// The edge curve for the topology: a segment between `p0` and `p1` (the vertex
    /// points) on a line, the circle or ellipse itself over the span otherwise.
    pub fn edge_curve(&self, p0: Point3, p1: Point3) -> KernelResult<(Curve, f64, f64)> {
        Ok(match self.carrier {
            Carrier::Line(_) => (Curve::Segment(Segment3::new(p0, p1)?), 0.0, 1.0),
            Carrier::Circle(c) => (Curve::Circle(c), self.t0, self.t1),
            Carrier::Ellipse(e) => (Curve::Ellipse(e), self.t0, self.t1),
        })
    }

    /// `∫ (c - r) × c' dt` over the span (see [`Curve::area_moment`]).
    pub fn area_moment(&self, r: Point3) -> Vec3 {
        self.carrier.curve().area_moment(self.t0, self.t1, r)
    }

    /// True when every one of a few points of `other` lies on this span's carrier.
    pub fn same_carrier(&self, other: &Span) -> KernelResult<bool> {
        for s in [0.0, 0.31, 0.5, 0.77, 1.0] {
            let (_, d) = self.carrier.project(other.at(s))?;
            if d > LINEAR_TOLERANCE {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Surfaces whose intersection with a curve `c` lying on `face_surface` gives the points
/// where `c` meets the edge `edge` of that face: on a planar face the plane through a
/// straight edge perpendicular to the face, or the cylinder over a circular edge (both
/// meet the face's plane exactly in the edge's carrier); then the surface of the face on
/// the edge's other side (`neighbour`); then the plane of a circular or elliptical edge,
/// or the plane through a straight edge and the face surface's axis. The first one the
/// curve does not lie in decides.
pub(super) fn edge_cutters(
    face_surface: &Surface,
    edge: &Span,
    neighbour: Option<&Surface>,
) -> Vec<Surface> {
    let mut out = Vec::new();
    let p0 = edge.start();
    if let Surface::Plane(f) = face_surface {
        match edge.carrier {
            Carrier::Line(l) => {
                if let Some(n) = f.z().cross(l.direction()).normalized()
                    && let Some(fr) = Frame::from_normal(p0, n)
                {
                    out.push(Surface::Plane(fr));
                }
            }
            Carrier::Circle(c) => {
                if let Ok(s) = Surface::cylinder(*c.frame(), c.radius()) {
                    out.push(s);
                }
            }
            Carrier::Ellipse(_) => {}
        }
    }
    if let Some(n) = neighbour {
        out.push(n.clone());
    }
    match edge.carrier {
        Carrier::Line(l) => {
            if !matches!(face_surface, Surface::Plane(_)) {
                let axis = face_surface.frame();
                let d = l.direction();
                let n = d
                    .cross(axis.z())
                    .normalized()
                    .or_else(|| d.cross(p0 - axis.origin()).normalized());
                if let Some(n) = n
                    && let Some(fr) = Frame::from_normal(p0, n)
                {
                    out.push(Surface::Plane(fr));
                }
            }
        }
        Carrier::Circle(_) | Carrier::Ellipse(_) => {
            if let Some(fr) = edge.carrier.plane() {
                out.push(Surface::Plane(fr));
            }
        }
    }
    out
}

/// The parameters (within `lo..hi`) where carrier `c`, lying on `face_surface`, meets the
/// edge `edge` of that face: crossings found through [`edge_cutters`], plus the edge's
/// end points that lie on `c`.
pub(super) fn crossings(
    c: &Carrier,
    lo: f64,
    hi: f64,
    face_surface: &Surface,
    edge: &Span,
    neighbour: Option<&Surface>,
) -> KernelResult<Vec<f64>> {
    let mut out = Vec::new();
    let curve = c.curve();
    for cutter in edge_cutters(face_surface, edge, neighbour) {
        match intersect_curve_surface_on(&curve, lo, hi, &cutter)? {
            CurveSurfaceIntersection::OnSurface { .. } => continue,
            CurveSurfaceIntersection::Points(pts) => {
                for p in pts {
                    if edge.distance(p.point)? <= LINEAR_TOLERANCE {
                        out.push(p.t);
                    }
                }
                break;
            }
        }
    }
    for p in [edge.start(), edge.end()] {
        if let Some(t) = param_in(c, lo, hi, p)? {
            out.push(t);
        }
    }
    Ok(out)
}

/// The parameter of `p` on carrier `c` within `lo..hi` (a closed carrier's parameter is
/// moved by whole turns into the range), if `p` lies on it.
pub(super) fn param_in(c: &Carrier, lo: f64, hi: f64, p: Point3) -> KernelResult<Option<f64>> {
    let (t, d) = c.project(p)?;
    if d > LINEAR_TOLERANCE {
        return Ok(None);
    }
    if c.periodic() {
        let t = lo + (t - lo).rem_euclid(TAU);
        if t <= hi {
            return Ok(Some(t));
        }
        // Just below `lo` (by rounding): the start.
        if c.point(lo).coincident(p) {
            return Ok(Some(lo));
        }
        return Ok(None);
    }
    Ok((t >= lo - LINEAR_TOLERANCE && t <= hi + LINEAR_TOLERANCE).then_some(t.clamp(lo, hi)))
}

pub(super) fn internal(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

//! Curve-surface intersection: lines, segments, circles, arcs, ellipses and elliptical
//! arcs against planes, cylinders, cones, spheres and tori.
//!
//! # Method
//! Every surface has an implicit equation `F = 0` in its frame's local coordinates
//! (Hoffmann, *Geometric and Solid Modeling*, chapter 7; Patrikalakis and Maekawa,
//! section 5.6 for curve-implicit surface intersection by substitution):
//!
//! | Surface | `F(x, y, z)` | degree |
//! |---|---|---|
//! | plane | `z` | 1 |
//! | cylinder | `x² + y² - r²` | 2 |
//! | sphere | `x² + y² + z² - r²` | 2 |
//! | cone | `cos² α (x² + y²) - (r cos α + z sin α)²` (both nappes) | 2 |
//! | torus | `(x² + y² + z² + R² - a²)² - 4 R² (x² + y²)` | 4 |
//!
//! A line or segment is `q0 + τ dq`, so `F` along it is a polynomial in `τ`. A circle or
//! ellipse `C + U cos t + V sin t` becomes rational in the half-angle tangent
//! `w = tan((t - φ) / 2)`: `cos(t - φ) = (1 - w²)/(1 + w²)`, `sin(t - φ) = 2w/(1 + w²)`, so
//! `(1 + w²)^deg F` is a polynomial of twice the degree. Two charts, `φ = 0` and `φ = π`,
//! each over `|t - φ| <= 0.6 π`, cover the whole conic with overlap and keep `w` bounded.
//!
//! Real roots come from derivative recursion and bisection (`poly.rs`); roots on the
//! cone's other nappe are dropped (they are not on the surface).
//!
//! # Tangency
//! A critical point of the polynomial (a root of its derivative) whose curve point lies
//! within [`LINEAR_TOLERANCE`] of the surface is a tangent contact. Roots next to it, with
//! no other critical point in between (the polynomial is monotone there, so the curve
//! stays that close), merge into it. So a line at distance `r ± LINEAR_TOLERANCE` from a
//! cylinder's axis touches it once, farther out it misses, farther in it crosses twice:
//! the same band the surface-surface cases use. Contacts from the two charts, and any two
//! contacts whose points coincide or whose stretch between them stays within the
//! tolerance of the surface while one of them is tangent, are merged.
//!
//! At a surface's singular points (a cone's apex, a spindle torus's tips) the implicit
//! gradient vanishes, so a curve through one is reported tangent there.
//!
//! A curve lying on the surface over its whole range (every one of
//! [`ON_SURFACE_SAMPLES`] samples within the tolerance) is
//! [`CurveSurfaceIntersection::OnSurface`]; a line along a cone's generator through the
//! apex lies on it over the half on the nappe.
//!
//! An infinite line is first clipped to the working box `|x|, |y|, |z| <= MAX_COORDINATE`.
//! Work is bounded: fixed degrees, two charts, fixed sample counts.

use super::poly::{Poly, real_roots};
use crate::{Contact, Curve, KernelError, KernelResult, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, Point3, Vec3};
use std::f64::consts::{PI, TAU};

/// Samples along a curve's range that must all lie on a surface for the curve to count
/// as lying on it: more than the 9 points that pin down the degree-8 conic-torus
/// polynomial.
pub const ON_SURFACE_SAMPLES: usize = 17;
/// Half-width of each conic chart, as a fraction of π (charts overlap by 0.2 π a side).
const CHART_HALF_WIDTH: f64 = 0.6;
/// Samples inside a stretch between two contacts when deciding whether they merge.
const MERGE_SAMPLES: usize = 5;

/// A point where a curve meets a surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSurfacePoint {
    /// The curve parameter (within the requested range).
    pub t: f64,
    pub point: Point3,
    /// [`Contact::Crossing`] when the curve passes through the surface,
    /// [`Contact::Tangent`] when it touches it (its tangent lies in the tangent plane).
    pub contact: Contact,
}

/// Everything a curve has in common with a surface.
#[derive(Debug, Clone, PartialEq)]
pub enum CurveSurfaceIntersection {
    /// Isolated points, ascending in `t` (possibly none).
    Points(Vec<CurveSurfacePoint>),
    /// The curve lies on the surface for `t0 <= t <= t1` (all of the requested range,
    /// except for a cone's generator through the apex, which lies on the nappe for half
    /// of its length).
    OnSurface { t0: f64, t1: f64 },
}

impl CurveSurfaceIntersection {
    /// The isolated points (none when the curve lies on the surface).
    pub fn points(&self) -> &[CurveSurfacePoint] {
        match self {
            CurveSurfaceIntersection::Points(p) => p,
            CurveSurfaceIntersection::OnSurface { .. } => &[],
        }
    }
}

/// Where `curve`, over its whole parameter range ([`Curve::range`]), meets `surface`.
pub fn intersect_curve_surface(
    curve: &Curve,
    surface: &Surface,
) -> KernelResult<CurveSurfaceIntersection> {
    let (t0, t1) = curve.range();
    intersect_curve_surface_on(curve, t0, t1, surface)
}

/// Where `curve`, over `t0 <= t <= t1`, meets `surface`. The range must be ordered and
/// finite, except that a line's range may be infinite; a circle's or ellipse's range spans
/// at most one turn.
pub fn intersect_curve_surface_on(
    curve: &Curve,
    t0: f64,
    t1: f64,
    surface: &Surface,
) -> KernelResult<CurveSurfaceIntersection> {
    if matches!(curve, Curve::BSpline(_)) || matches!(surface, Surface::Extrusion(_)) {
        return Err(KernelError::Unsupported(
            "intersections of spline curves and surfaces are not supported yet (K10)".into(),
        ));
    }
    let ordered = !t0.is_nan() && !t1.is_nan() && t0 <= t1;
    let finite = t0.is_finite() && t1.is_finite();
    let bad_range = |what: &str| {
        KernelError::InvalidInput(format!(
            "the curve range `t0..t1` must be {what}, with t0 <= t1"
        ))
    };
    match curve {
        Curve::Line(l) => {
            if !ordered || t0 == f64::INFINITY || t1 == f64::NEG_INFINITY {
                return Err(bad_range("ordered (infinite ends allowed for a line)"));
            }
            linear(curve, l.origin(), l.direction(), t0, t1, surface)
        }
        Curve::Segment(s) => {
            if !(ordered && finite) {
                return Err(bad_range("finite"));
            }
            linear(curve, s.start(), s.end() - s.start(), t0, t1, surface)
        }
        Curve::Circle(_) | Curve::Arc(_) | Curve::Ellipse(_) | Curve::EllipticalArc(_) => {
            if !(ordered && finite && t1 - t0 <= TAU + ANGULAR_TOLERANCE) {
                return Err(bad_range("finite and at most one turn"));
            }
            let (frame, rx, ry) = match curve {
                Curve::Circle(c) => (*c.frame(), c.radius(), c.radius()),
                Curve::Arc(a) => (
                    *a.circle().frame(),
                    a.circle().radius(),
                    a.circle().radius(),
                ),
                Curve::Ellipse(e) => (*e.frame(), e.rx(), e.ry()),
                Curve::EllipticalArc(a) => {
                    (*a.ellipse().frame(), a.ellipse().rx(), a.ellipse().ry())
                }
                _ => return Err(bad_range("valid")),
            };
            conic(
                curve,
                frame.origin(),
                frame.x() * rx,
                frame.y() * ry,
                t0,
                t1,
                rx.min(ry),
                surface,
            )
        }
        Curve::BSpline(_) => Err(KernelError::Unsupported(
            "intersections of spline curves are not supported yet (K10)".into(),
        )),
    }
}

/// `F` of `surface` on polynomial coordinates `x/d`, `y/d`, `z/d` (local), times `d^deg`.
fn implicit(surface: &Surface, x: &Poly, y: &Poly, z: &Poly, d: &Poly) -> Poly {
    let sq = |p: &Poly| p.mul(p);
    let rho2 = sq(x).add(&sq(y));
    match surface {
        // Refused before (`intersect_curve_surface_on`); no implicit form here.
        Surface::Plane(_) | Surface::Extrusion(_) => z.clone(),
        Surface::Cylinder(c) => rho2.sub(&sq(d).scale(c.radius() * c.radius())),
        Surface::Sphere(s) => rho2.add(&sq(z)).sub(&sq(d).scale(s.radius() * s.radius())),
        Surface::Cone(c) => {
            let (s, co) = c.half_angle().sin_cos();
            let lin = d.scale(c.radius() * co).add(&z.scale(s));
            rho2.scale(co * co).sub(&sq(&lin))
        }
        Surface::Torus(t) => {
            let (big, a) = (t.major_radius(), t.minor_radius());
            let q = rho2.add(&sq(z)).add(&sq(d).scale((big - a) * (big + a)));
            sq(&q).sub(&sq(d).mul(&rho2).scale(4.0 * big * big))
        }
    }
}

/// Polynomial coordinates (local to the surface frame) of `base + τ dir`.
fn local_line(surface: &Surface, base: Point3, dir: Vec3) -> [Poly; 3] {
    let f = surface.frame();
    let q = f.to_local(base);
    let v = f.vector_to_local(dir);
    [
        Poly(vec![q.x, v.x]),
        Poly(vec![q.y, v.y]),
        Poly(vec![q.z, v.z]),
    ]
}

/// The parameter interval of `origin + t dir` inside the working box, within `[t0, t1]`.
fn clip_to_box(origin: Point3, dir: Vec3, t0: f64, t1: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (t0, t1);
    for (o, d) in [(origin.x, dir.x), (origin.y, dir.y), (origin.z, dir.z)] {
        if d == 0.0 {
            if o.abs() > MAX_COORDINATE {
                return None;
            }
            continue;
        }
        let (a, b) = ((-MAX_COORDINATE - o) / d, (MAX_COORDINATE - o) / d);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
    }
    (lo.is_finite() && hi.is_finite() && lo <= hi).then_some((lo, hi))
}

fn on_surface_everywhere(
    surface: &Surface,
    t0: f64,
    t1: f64,
    point: impl Fn(f64) -> Point3,
) -> bool {
    (0..ON_SURFACE_SAMPLES).all(|k| {
        let t = t0 + (t1 - t0) * k as f64 / (ON_SURFACE_SAMPLES - 1) as f64;
        surface.distance(point(t)) <= LINEAR_TOLERANCE
    })
}

/// A line or segment `origin + t dir` (`dir` need not be a unit vector).
fn linear(
    curve: &Curve,
    origin: Point3,
    dir: Vec3,
    t0: f64,
    t1: f64,
    surface: &Surface,
) -> KernelResult<CurveSurfaceIntersection> {
    let Some((lo, hi)) = clip_to_box(origin, dir, t0, t1) else {
        return Ok(CurveSurfaceIntersection::Points(vec![]));
    };
    let speed = dir.length();
    let at = |t: f64| origin + dir * t;
    if let Surface::Cone(c) = surface
        && let Some(r) = generator_through_apex(c, origin, dir, lo, hi)
    {
        return Ok(r);
    }
    if on_surface_everywhere(surface, lo, hi, at) {
        return Ok(CurveSurfaceIntersection::OnSurface { t0: lo, t1: hi });
    }
    let eps = LINEAR_TOLERANCE / speed;
    // Re-centre at the point nearest the surface's origin for well-conditioned
    // coefficients: t = tc + τ.
    let tc = ((surface.frame().origin() - origin).dot(dir) / (speed * speed)).clamp(lo, hi);
    let base = at(tc);
    let [x, y, z] = local_line(surface, base, dir);
    let p = implicit(surface, &x, &y, &z, &Poly::constant(1.0));
    let contacts = chart_contacts(
        &p,
        lo - tc - eps,
        hi - tc + eps,
        |tau| at(tc + tau),
        surface,
    );
    let mut cands: Vec<CurveSurfacePoint> = contacts
        .into_iter()
        .map(|(tau, contact)| {
            let t = (tc + tau).clamp(lo, hi);
            CurveSurfacePoint {
                t,
                point: curve.point_at(t),
                contact,
            }
        })
        .collect();
    cands.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(CurveSurfaceIntersection::Points(merge(
        cands,
        surface,
        false,
        |t| curve.point_at(t),
    )))
}

/// A line along a generator of `cone` through its apex lies on the nappe for the half
/// beyond the apex. `None` when the line is not such a generator.
fn generator_through_apex(
    cone: &crate::Cone,
    origin: Point3,
    dir: Vec3,
    lo: f64,
    hi: f64,
) -> Option<CurveSurfaceIntersection> {
    let apex = cone.apex();
    let speed = dir.length();
    let unit = dir * (1.0 / speed);
    let w = if cone.half_angle() > 0.0 {
        cone.frame().z()
    } else {
        -cone.frame().z()
    };
    let t_apex = (apex - origin).dot(dir) / (speed * speed);
    let foot = origin + dir * t_apex;
    if foot.distance(apex) > LINEAR_TOLERANCE {
        return None;
    }
    let theta = unit.cross(w).length().atan2(unit.dot(w));
    let a = cone.half_angle().abs();
    let (from, to) = if (theta - a).abs() <= ANGULAR_TOLERANCE {
        (t_apex.max(lo), hi)
    } else if (theta - (PI - a)).abs() <= ANGULAR_TOLERANCE {
        (lo, t_apex.min(hi))
    } else {
        return None;
    };
    let eps = LINEAR_TOLERANCE / speed;
    Some(if to - from > eps {
        CurveSurfaceIntersection::OnSurface { t0: from, t1: to }
    } else if to - from >= -eps {
        let t = t_apex.clamp(lo, hi);
        CurveSurfaceIntersection::Points(vec![CurveSurfacePoint {
            t,
            point: origin + dir * t,
            contact: Contact::Tangent,
        }])
    } else {
        CurveSurfaceIntersection::Points(vec![])
    })
}

/// A circle or ellipse `centre + u cos t + v sin t` over `[t0, t1]`; `min_radius` is its
/// smaller semi-axis (its least speed).
#[allow(clippy::too_many_arguments)]
fn conic(
    curve: &Curve,
    centre: Point3,
    u: Vec3,
    v: Vec3,
    t0: f64,
    t1: f64,
    min_radius: f64,
    surface: &Surface,
) -> KernelResult<CurveSurfaceIntersection> {
    let at = |t: f64| {
        let (s, c) = t.sin_cos();
        centre + u * c + v * s
    };
    if on_surface_everywhere(surface, t0, t1, at) {
        return Ok(CurveSurfaceIntersection::OnSurface { t0, t1 });
    }
    let eps = LINEAR_TOLERANCE / min_radius;
    let f = surface.frame();
    let c0 = f.to_local(centre);
    let wmax = (CHART_HALF_WIDTH * PI * 0.5).tan();
    let mut cands = Vec::new();
    for phi in [0.0, PI] {
        let sign = if phi == 0.0 { 1.0 } else { -1.0 };
        let lu = f.vector_to_local(u * sign);
        let lv = f.vector_to_local(v * sign);
        // Numerators over (1 + w²): c0 (1 + w²) + lu (1 - w²) + lv 2w.
        let num = |c: f64, a: f64, b: f64| Poly(vec![c + a, 2.0 * b, c - a]);
        let x = num(c0.x, lu.x, lv.x);
        let y = num(c0.y, lu.y, lv.y);
        let z = num(c0.z, lu.z, lv.z);
        let p = implicit(surface, &x, &y, &z, &Poly(vec![1.0, 0.0, 1.0]));
        let param = |w: f64| phi + 2.0 * w.atan();
        for (w, contact) in chart_contacts(&p, -wmax, wmax, |w| at(param(w)), surface) {
            // The representative of t in [t0 - eps, t0 - eps + 2π).
            let base = t0 - eps;
            let t = base + (param(w) - base).rem_euclid(TAU);
            cands.push(CurveSurfacePoint {
                t,
                point: at(t),
                contact,
            });
        }
    }
    cands.sort_by(|a, b| a.t.total_cmp(&b.t));
    let merged = merge(cands, surface, true, at);
    let out = merged
        .into_iter()
        .filter(|c| c.t <= t1 + eps)
        .map(|c| {
            let t = c.t.clamp(t0, t1);
            CurveSurfacePoint {
                t,
                point: curve.point_at(t),
                contact: c.contact,
            }
        })
        .collect();
    Ok(CurveSurfaceIntersection::Points(out))
}

/// Roots and tangent critical points of `p` on `[lo, hi]` (see the module documentation).
fn chart_contacts(
    p: &Poly,
    lo: f64,
    hi: f64,
    point: impl Fn(f64) -> Point3,
    surface: &Surface,
) -> Vec<(f64, Contact)> {
    let near = |s: f64| surface.distance(point(s)) <= LINEAR_TOLERANCE;
    let roots = real_roots(p, lo, hi);
    let crits = real_roots(&p.derivative(), lo, hi);
    let tangents: Vec<f64> = crits.iter().copied().filter(|c| near(*c)).collect();
    let mut out = Vec::new();
    for r in roots {
        if !near(r) {
            continue;
        }
        let merged = tangents.iter().any(|c| {
            let (a, b) = (r.min(*c), r.max(*c));
            !crits.iter().any(|k| *k > a && *k < b)
        });
        if !merged {
            out.push((r, Contact::Crossing));
        }
    }
    out.extend(tangents.into_iter().map(|c| (c, Contact::Tangent)));
    out
}

/// Merge contacts (sorted by `t`) that are the same point, or that one tangent contact
/// covers (the curve stays within the tolerance between them). For a periodic curve the
/// last and first contacts are neighbours too.
fn merge(
    cands: Vec<CurveSurfacePoint>,
    surface: &Surface,
    periodic: bool,
    at: impl Fn(f64) -> Point3,
) -> Vec<CurveSurfacePoint> {
    let same = |a: &CurveSurfacePoint, b: &CurveSurfacePoint, tb: f64| {
        if a.point.coincident(b.point) {
            return true;
        }
        if a.contact == Contact::Crossing && b.contact == Contact::Crossing {
            return false;
        }
        (1..=MERGE_SAMPLES).all(|k| {
            let t = a.t + (tb - a.t) * k as f64 / (MERGE_SAMPLES + 1) as f64;
            surface.distance(at(t)) <= LINEAR_TOLERANCE
        })
    };
    let combine = |a: CurveSurfacePoint, b: CurveSurfacePoint| {
        if b.contact == Contact::Tangent && a.contact == Contact::Crossing {
            b
        } else {
            a
        }
    };
    let mut out: Vec<CurveSurfacePoint> = Vec::with_capacity(cands.len());
    for c in cands {
        match out.last_mut() {
            Some(last) if same(last, &c, c.t) => *last = combine(*last, c),
            _ => out.push(c),
        }
    }
    if periodic
        && out.len() >= 2
        && let (Some(first), Some(last)) = (out.first().copied(), out.last().copied())
        && same(&last, &first, first.t + TAU)
    {
        // Keep the first representative (it is in range when either is) and the
        // stronger contact.
        out.pop();
        if let Some(f) = out.first_mut()
            && last.contact == Contact::Tangent
        {
            f.contact = Contact::Tangent;
        }
    }
    out
}

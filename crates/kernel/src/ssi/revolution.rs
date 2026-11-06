//! Coaxial surfaces of revolution, and planes perpendicular to an axis, by meridians.
//!
//! A surface of revolution about an axis is swept by its meridian: its trace in a
//! half-plane through the axis, in coordinates `(ρ, h)` (distance from the axis, height
//! along it). Working in the whole plane through the axis with a signed `ρ` keeps every
//! meridian a line, ray or full circle, at the cost of also adding its mirror image
//! `ρ -> -ρ`:
//!
//! | Surface | Meridian primitives (signed plane) |
//! |---|---|
//! | plane perpendicular to the axis at height `h0` | the line `h = h0` |
//! | cylinder, radius `r` | the lines `ρ = ±r` |
//! | cone, apex height `h_a`, half-angle `a`, nappe toward `σ = ±1` | the rays from `(0, h_a)` along `(±sin a, σ cos a)` |
//! | sphere, centre height `h_c`, radius `r` | the circle about `(0, h_c)` of radius `r` |
//! | torus, centre height `h_c`, major `R`, minor `a` | the circles about `(±R, h_c)` of radius `a` |
//!
//! Two coaxial surfaces share exactly the circles swept by the common points of their
//! meridians with `ρ > 0`, and the axis points where the meridians meet on `ρ = 0`. Where
//! the two meridians are tangent the surfaces are tangent along the whole circle (their
//! normals lie in the meridian plane and are the meridians' normals). Meridians that
//! overlap along a stretch belong to the same surface (a line, ray or circle of one of
//! these families determines the surface), so the surfaces coincide.

use super::{SurfaceIntersection, beyond_range, circle_curve, half_chord, parallel};
use crate::{Contact, KernelResult, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, Point2, Point3, Vec2};

/// A meridian primitive in the signed meridian plane: `x` is `ρ`, `y` is `h`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Meridian {
    /// `p + t d` with `d` a unit vector, for all `t`, or `t >= 0` when `ray`.
    Line {
        p: Point2,
        d: Vec2,
        ray: bool,
    },
    Circle {
        c: Point2,
        r: f64,
    },
}

/// What two primitives have in common.
pub(crate) enum Common {
    Points(Vec<(Point2, Contact)>),
    /// They coincide along a stretch longer than the tolerance.
    Overlap,
}

/// The axis shared by two curved surfaces, as a frame whose z axis is the axis: two
/// parallel axes through each other's origin, a sphere whose centre lies on the other's
/// axis, or any two spheres (the line through their centres).
pub(crate) fn common_axis(a: &Surface, b: &Surface) -> Option<Frame> {
    let axis = |s: &Surface| match s {
        Surface::Cylinder(_) | Surface::Cone(_) | Surface::Torus(_) => Some(*s.frame()),
        _ => None,
    };
    let on_axis = |f: &Frame, s: &Surface| {
        let l = f.to_local(s.frame().origin());
        l.x.hypot(l.y) <= LINEAR_TOLERANCE
    };
    match (axis(a), axis(b)) {
        (Some(fa), Some(fb)) => (parallel(fa.z(), fb.z()) && on_axis(&fa, b)).then_some(fa),
        (Some(fa), None) => (matches!(b, Surface::Sphere(_)) && on_axis(&fa, b)).then_some(fa),
        (None, Some(fb)) => (matches!(a, Surface::Sphere(_)) && on_axis(&fb, a)).then_some(fb),
        (None, None) => {
            if !(matches!(a, Surface::Sphere(_)) && matches!(b, Surface::Sphere(_))) {
                return None;
            }
            let (oa, ob) = (a.frame().origin(), b.frame().origin());
            Frame::from_normal(oa, ob - oa).or(Some(*a.frame()))
        }
    }
}

/// The meridian primitives of a curved surface about `axis` (none for a plane).
pub(crate) fn meridians(axis: &Frame, s: &Surface) -> Vec<Meridian> {
    let height = |p: Point3| (p - axis.origin()).dot(axis.z());
    let hc = height(s.frame().origin());
    match s {
        Surface::Plane(_) => Vec::new(),
        Surface::Cylinder(c) => [-1.0, 1.0]
            .map(|k| Meridian::Line {
                p: Point2::new(k * c.radius(), hc),
                d: Vec2::Y,
                ray: false,
            })
            .to_vec(),
        Surface::Cone(c) => {
            let alpha = c.half_angle();
            let (sa, ca) = alpha.abs().sin_cos();
            let sigma = alpha.signum() * c.frame().z().dot(axis.z()).signum();
            let p = Point2::new(0.0, height(c.apex()));
            [-1.0, 1.0]
                .map(|k| Meridian::Line {
                    p,
                    d: Vec2::new(k * sa, sigma * ca),
                    ray: true,
                })
                .to_vec()
        }
        Surface::Sphere(sp) => vec![Meridian::Circle {
            c: Point2::new(0.0, hc),
            r: sp.radius(),
        }],
        Surface::Torus(t) => [-1.0, 1.0]
            .map(|k| Meridian::Circle {
                c: Point2::new(k * t.major_radius(), hc),
                r: t.minor_radius(),
            })
            .to_vec(),
    }
}

/// Two coaxial curved surfaces.
pub(crate) fn coaxial(axis: &Frame, a: &Surface, b: &Surface) -> KernelResult<SurfaceIntersection> {
    solve(axis, &meridians(axis, a), &meridians(axis, b))
}

/// A plane perpendicular to the axis of a curved surface.
pub(crate) fn plane_perpendicular(plane: &Frame, s: &Surface) -> KernelResult<SurfaceIntersection> {
    let axis = *s.frame();
    let h0 = (plane.origin() - axis.origin()).dot(axis.z());
    let line = Meridian::Line {
        p: Point2::new(0.0, h0),
        d: Vec2::X,
        ray: false,
    };
    solve(&axis, &[line], &meridians(&axis, s))
}

fn solve(axis: &Frame, ma: &[Meridian], mb: &[Meridian]) -> KernelResult<SurfaceIntersection> {
    let mut hits = Vec::new();
    for x in ma {
        for y in mb {
            match common(x, y) {
                Common::Overlap => return Ok(SurfaceIntersection::Coincident),
                Common::Points(p) => hits.extend(p),
            }
        }
    }
    // Keep ρ >= 0; points on the axis become isolated points, the others circles.
    let mut on_axis: Vec<f64> = Vec::new();
    let mut circles: Vec<(f64, f64, Contact)> = Vec::new();
    for (q, contact) in hits {
        if q.x < -LINEAR_TOLERANCE {
            continue;
        }
        if q.x <= LINEAR_TOLERANCE {
            if !on_axis.iter().any(|h| (h - q.y).abs() <= LINEAR_TOLERANCE) {
                on_axis.push(q.y);
            }
        } else if let Some(c) = circles.iter_mut().find(|(r, h, _)| {
            (r - q.x).abs() <= LINEAR_TOLERANCE && (h - q.y).abs() <= LINEAR_TOLERANCE
        }) {
            if contact == Contact::Tangent {
                c.2 = Contact::Tangent;
            }
        } else {
            circles.push((q.x, q.y, contact));
        }
    }
    on_axis.sort_by(f64::total_cmp);
    circles.sort_by(|p, q| p.1.total_cmp(&q.1).then(p.0.total_cmp(&q.0)));
    let (o, z) = (axis.origin(), axis.z());
    let mut curves = Vec::with_capacity(circles.len());
    for (rho, h, contact) in circles {
        curves.push(circle_curve(o + z * h, z, axis.x(), rho, contact)?);
    }
    let mut points = Vec::with_capacity(on_axis.len());
    for h in on_axis {
        let p = o + z * h;
        if !p.is_valid() {
            return Err(beyond_range("point"));
        }
        points.push(p);
    }
    Ok(SurfaceIntersection::from_parts(curves, points))
}

/// The common points of two meridian primitives.
pub(crate) fn common(x: &Meridian, y: &Meridian) -> Common {
    match (*x, *y) {
        (
            Meridian::Line { p, d, ray },
            Meridian::Line {
                p: q,
                d: e,
                ray: ray2,
            },
        ) => line_line(p, d, ray, q, e, ray2),
        (Meridian::Line { p, d, ray }, Meridian::Circle { c, r })
        | (Meridian::Circle { c, r }, Meridian::Line { p, d, ray }) => line_circle(p, d, ray, c, r),
        (Meridian::Circle { c, r }, Meridian::Circle { c: c2, r: r2 }) => {
            circle_circle(c, r, c2, r2)
        }
    }
}

fn ray_ok(ray: bool, t: f64) -> bool {
    !ray || t >= -LINEAR_TOLERANCE
}

/// Two lines (or rays). Parallel when `|d × e| <= ANGULAR_TOLERANCE`; then collinear when
/// within [`LINEAR_TOLERANCE`], and the overlap of their parameter ranges on the first
/// decides between a stretch, a single point (rays meeting end to end) and nothing.
fn line_line(p: Point2, d: Vec2, ray: bool, q: Point2, e: Vec2, ray2: bool) -> Common {
    let cr = d.cross(e);
    if cr.abs() <= ANGULAR_TOLERANCE {
        if (q - p).cross(d).abs() > LINEAR_TOLERANCE {
            return Common::Points(vec![]);
        }
        let s = (q - p).dot(d);
        let lo1 = if ray { 0.0 } else { f64::NEG_INFINITY };
        let (lo2, hi2) = match (ray2, d.dot(e) > 0.0) {
            (false, _) => (f64::NEG_INFINITY, f64::INFINITY),
            (true, true) => (s, f64::INFINITY),
            (true, false) => (f64::NEG_INFINITY, s),
        };
        let (lo, hi) = (lo1.max(lo2), hi2);
        if hi - lo > LINEAR_TOLERANCE {
            return Common::Overlap;
        }
        if hi - lo >= -LINEAR_TOLERANCE && lo.is_finite() {
            return Common::Points(vec![(p + d * lo, Contact::Crossing)]);
        }
        return Common::Points(vec![]);
    }
    let t1 = (q - p).cross(e) / cr;
    let t2 = (q - p).cross(d) / cr;
    if !(ray_ok(ray, t1) && ray_ok(ray2, t2)) {
        return Common::Points(vec![]);
    }
    let t1 = if ray { t1.max(0.0) } else { t1 };
    Common::Points(vec![(p + d * t1, Contact::Crossing)])
}

/// A line (or ray) and a circle: tangent when the line's distance from the centre is
/// within [`LINEAR_TOLERANCE`] of the radius.
fn line_circle(p: Point2, d: Vec2, ray: bool, c: Point2, r: f64) -> Common {
    let tf = (c - p).dot(d);
    let foot = p + d * tf;
    let delta = foot.distance(c);
    if delta > r + LINEAR_TOLERANCE {
        return Common::Points(vec![]);
    }
    if (delta - r).abs() <= LINEAR_TOLERANCE {
        return Common::Points(if ray_ok(ray, tf) {
            vec![(foot, Contact::Tangent)]
        } else {
            vec![]
        });
    }
    let h = half_chord(r, delta);
    Common::Points(
        [tf - h, tf + h]
            .into_iter()
            .filter(|t| ray_ok(ray, *t))
            .map(|t| (p + d * if ray { t.max(0.0) } else { t }, Contact::Crossing))
            .collect(),
    )
}

/// Two circles. With centre distance `D`: equal concentric circles overlap; `D` within
/// [`LINEAR_TOLERANCE`] of `r1 + r2` or `|r1 - r2|` touches; between them they cross at
/// `a = (D² + r1² - r2²) / 2D` along the centre line and `±sqrt(r1² - a²)` across it.
pub(crate) fn circle_circle(c1: Point2, r1: f64, c2: Point2, r2: f64) -> Common {
    // Measure from the smaller circle: `r1 - a` cancels less there (a sphere of radius
    // 1e6 through a unit sphere would otherwise lose six digits of the common circle).
    if r2 < r1 {
        return circle_circle(c2, r2, c1, r1);
    }
    let v = c2 - c1;
    let dist = v.length();
    if dist <= LINEAR_TOLERANCE {
        return if (r1 - r2).abs() <= LINEAR_TOLERANCE {
            Common::Overlap
        } else {
            Common::Points(vec![])
        };
    }
    let u = v * (1.0 / dist);
    let (outer, inner) = (r1 + r2, (r1 - r2).abs());
    if dist > outer + LINEAR_TOLERANCE || dist < inner - LINEAR_TOLERANCE {
        return Common::Points(vec![]);
    }
    if (dist - outer).abs() <= LINEAR_TOLERANCE {
        return Common::Points(vec![(c1 + u * r1, Contact::Tangent)]);
    }
    if (dist - inner).abs() <= LINEAR_TOLERANCE {
        let k = if r1 >= r2 { r1 } else { -r1 };
        return Common::Points(vec![(c1 + u * k, Contact::Tangent)]);
    }
    let a = (dist * dist + (r1 - r2) * (r1 + r2)) / (2.0 * dist);
    let h = half_chord(r1, a);
    let base = c1 + u * a;
    Common::Points(vec![
        (base + u.perp() * -h, Contact::Crossing),
        (base + u.perp() * h, Contact::Crossing),
    ])
}

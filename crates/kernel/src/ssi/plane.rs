//! A plane against every surface.
//!
//! Notation: the plane has unit normal `n` and origin `p0`; a curved surface has frame
//! origin `o` and axis `z`. "Parallel to the axis" means `|n · z| <= ANGULAR_TOLERANCE`,
//! "perpendicular to the axis" means `|n × z| <= ANGULAR_TOLERANCE`.

use super::{
    SurfaceIntersection, UnsupportedIntersection, beyond_range, circle_curve, ellipse_curve,
    half_chord, line_curve, parallel, perpendicular, revolution,
};
use crate::{Cone, Contact, Cylinder, KernelResult, Sphere, Surface, Torus};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, Point3};

/// Two planes: one line, or coincident, or parallel and apart.
///
/// With unit normals `n1`, `n2` and `u = n1 × n2`, the line runs along `u`. Taking
/// `p0 = o1` as the origin, the point `p = o1 + α n1 + β n2` lies on both planes when
/// `α + β c = 0` and `α c + β = δ`, with `c = n1 · n2` and `δ = n2 · (o2 - o1)`; so
/// `β = δ / (1 - c²)`, `α = -c β`. The line's origin is then moved to the point of the line
/// nearest the midpoint of the two plane origins, to keep it near both.
pub(crate) fn plane_plane(a: &Frame, b: &Frame) -> KernelResult<SurfaceIntersection> {
    let (n1, n2) = (a.z(), b.z());
    let delta = n2.dot(b.origin() - a.origin());
    if parallel(n1, n2) {
        return Ok(if delta.abs() <= LINEAR_TOLERANCE {
            SurfaceIntersection::Coincident
        } else {
            SurfaceIntersection::Empty
        });
    }
    let u = n1.cross(n2);
    let c = n1.dot(n2);
    let beta = delta / u.dot(u);
    let p = a.origin() + n1 * (-c * beta) + n2 * beta;
    let dir = u.normalized().ok_or_else(|| beyond_range("line"))?;
    let mid = a.origin() + (b.origin() - a.origin()) * 0.5;
    let p = p + dir * (mid - p).dot(dir);
    Ok(SurfaceIntersection::Curves {
        curves: vec![line_curve(p, dir, false, Contact::Crossing)?],
        points: vec![],
    })
}

pub(crate) fn plane_surface(plane: &Frame, other: &Surface) -> KernelResult<SurfaceIntersection> {
    match other {
        Surface::Plane(q) => plane_plane(plane, q),
        Surface::Cylinder(c) => plane_cylinder(plane, c),
        Surface::Sphere(s) => plane_sphere(plane, s),
        Surface::Cone(c) => plane_cone(plane, c),
        Surface::Torus(t) => plane_torus(plane, t),
        Surface::Extrusion(e) => super::extrusion::plane_extrusion(plane, e),
    }
}

/// A plane and a cylinder of radius `r`.
///
/// - Parallel to the axis: the axis is at signed distance `d = n · (o - p0)` from the
///   plane; its foot is `o - d n`. `|d| > r` misses, `|d| = r` touches along one line,
///   `|d| < r` cuts two lines `±sqrt(r² - d²)` from the foot across the plane.
/// - Perpendicular: the circle of radius `r` where the axis meets the plane.
/// - Oblique at angle `θ` between plane and axis (`sin θ = |n · z|`): an ellipse centred
///   where the axis meets the plane, semi-minor axis `r` along `m = z × n` (perpendicular
///   to the axis, so it stays at distance `r`) and semi-major axis `r / sin θ` along
///   `M = n × m`, whose component across the axis is `|n · z|` per unit length.
pub(crate) fn plane_cylinder(plane: &Frame, cyl: &Cylinder) -> KernelResult<SurfaceIntersection> {
    let n = plane.z();
    let f = cyl.frame();
    let (o, z, r) = (f.origin(), f.z(), cyl.radius());
    let c = n.dot(z);
    if perpendicular(n, z) {
        let d = n.dot(o - plane.origin());
        let foot = o + n * -d;
        // The axis direction projected into the plane, so the lines lie in it exactly.
        let dir = (z - n * c)
            .normalized()
            .ok_or_else(|| beyond_range("line"))?;
        let across = n.cross(dir);
        let ad = d.abs();
        if ad > r + LINEAR_TOLERANCE {
            return Ok(SurfaceIntersection::Empty);
        }
        if (ad - r).abs() <= LINEAR_TOLERANCE {
            return Ok(SurfaceIntersection::Tangent {
                curves: vec![line_curve(foot, dir, false, Contact::Tangent)?],
                points: vec![],
            });
        }
        let h = half_chord(r, ad);
        return Ok(SurfaceIntersection::Curves {
            curves: vec![
                line_curve(foot + across * -h, dir, false, Contact::Crossing)?,
                line_curve(foot + across * h, dir, false, Contact::Crossing)?,
            ],
            points: vec![],
        });
    }
    let centre = o + z * (n.dot(plane.origin() - o) / c);
    if parallel(n, z) {
        return Ok(SurfaceIntersection::Curves {
            curves: vec![circle_curve(centre, n, f.x(), r, Contact::Crossing)?],
            points: vec![],
        });
    }
    let m = z
        .cross(n)
        .normalized()
        .ok_or_else(|| beyond_range("ellipse"))?;
    let major = n.cross(m);
    Ok(SurfaceIntersection::Curves {
        curves: vec![ellipse_curve(centre, n, major, r / c.abs(), r)?],
        points: vec![],
    })
}

/// A plane and a sphere of radius `r`: the centre is at distance `|d|` from the plane;
/// `|d| > r` misses, `|d| = r` touches at the foot, `|d| < r` cuts the circle of radius
/// `sqrt(r² - d²)` about the foot.
pub(crate) fn plane_sphere(plane: &Frame, s: &Sphere) -> KernelResult<SurfaceIntersection> {
    let n = plane.z();
    let o = s.frame().origin();
    let r = s.radius();
    let d = n.dot(o - plane.origin());
    let foot = o + n * -d;
    let ad = d.abs();
    if ad > r + LINEAR_TOLERANCE {
        return Ok(SurfaceIntersection::Empty);
    }
    let rho = half_chord(r, ad);
    if (ad - r).abs() <= LINEAR_TOLERANCE || rho <= LINEAR_TOLERANCE {
        return Ok(SurfaceIntersection::Tangent {
            curves: vec![],
            points: vec![valid(foot, "point")?],
        });
    }
    Ok(SurfaceIntersection::Curves {
        curves: vec![circle_curve(foot, n, plane.x(), rho, Contact::Crossing)?],
        points: vec![],
    })
}

/// A plane and a cone (one nappe, apex `A`, axis direction into the nappe `w`, half-angle
/// `a = |α|`). Perpendicular planes go through the meridian method (circle, apex point or
/// nothing). Otherwise, with `n` turned so that `c = n · w >= 0`, `s = |n × w|`, the
/// plane `n · (p - A) = d` makes the angle `β = atan2(c, s)` with the axis, and:
///
/// - through the apex (`|d| <= LINEAR_TOLERANCE`): `β > a` meets only the apex; `β = a`
///   touches along one generator; `β < a` cuts two generators. A generator
///   `g = cos a w + sin a e` (with `e` a unit vector across the axis) lies in the plane when
///   `g · n = 0`; writing `e = cos φ p̂ + sin φ q̂` with `p̂ = (n - c w) / s` and
///   `q̂ = w × p̂` gives `cos φ = -c cos a / (s sin a)`.
/// - otherwise `β > a` is an ellipse, `β = a` a parabola and `β < a` a hyperbola. For the
///   ellipse, put `p - A = d n + x e1 + y e2` with `e1 = (w - c n) / s`, `e2 = n × e1`;
///   then `(p - A) · w = d c + x s` and the double cone `((p - A) · w)² = cos² a |p - A|²`
///   becomes `K x² - 2 d c s x + cos² a y² + d² (cos² a - c²) = 0` with
///   `K = c² - sin² a > 0`. Completing the square: centre `x0 = d c s / K`, semi-axes
///   `d cos a sin a / K` along `e1` and `d sin a / sqrt(K)` along `e2`. The ellipse lies
///   where `(p - A) · w` has the sign of `d c`, so on this nappe only when `d > 0`.
///   (Check: `c = 1` gives a circle of radius `d tan a`.)
pub(crate) fn plane_cone(plane: &Frame, cone: &Cone) -> KernelResult<SurfaceIntersection> {
    let f = cone.frame();
    if parallel(plane.z(), f.z()) {
        return revolution::plane_perpendicular(plane, &Surface::Cone(*cone));
    }
    let alpha = cone.half_angle();
    let a = alpha.abs();
    let (sa, ca) = a.sin_cos();
    let w = if alpha > 0.0 { f.z() } else { -f.z() };
    let apex = cone.apex();
    let mut n = plane.z();
    let mut d = n.dot(plane.origin() - apex);
    if n.dot(w) < 0.0 {
        n = -n;
        d = -d;
    }
    let c = n.dot(w);
    let s = n.cross(w).length();
    let delta = c.atan2(s) - a;
    if d.abs() <= LINEAR_TOLERANCE {
        let apex = valid(apex, "apex")?;
        if delta > ANGULAR_TOLERANCE {
            return Ok(SurfaceIntersection::Tangent {
                curves: vec![],
                points: vec![apex],
            });
        }
        let p_hat = (n - w * c) * (1.0 / s);
        let q_hat = w.cross(p_hat);
        if delta.abs() <= ANGULAR_TOLERANCE {
            let g = w * ca - p_hat * sa;
            return Ok(SurfaceIntersection::Tangent {
                curves: vec![line_curve(apex, g, true, Contact::Tangent)?],
                points: vec![],
            });
        }
        let cos_phi = (-(c * ca) / (s * sa)).clamp(-1.0, 1.0);
        let sin_phi = (1.0 - cos_phi * cos_phi).max(0.0).sqrt();
        let mut curves = Vec::new();
        for sign in [-1.0, 1.0] {
            let e = p_hat * cos_phi + q_hat * (sign * sin_phi);
            curves.push(line_curve(apex, w * ca + e * sa, true, Contact::Crossing)?);
        }
        return Ok(SurfaceIntersection::Curves {
            curves,
            points: vec![],
        });
    }
    if delta > ANGULAR_TOLERANCE {
        if d < 0.0 {
            return Ok(SurfaceIntersection::Empty);
        }
        let k = (c - sa) * (c + sa);
        let e1 = (w - n * c) * (1.0 / s);
        let big = d * ca * sa / k;
        let small = d * sa / k.sqrt();
        let centre = apex + n * d + e1 * (d * c * s / k);
        if big <= LINEAR_TOLERANCE {
            return Ok(SurfaceIntersection::Tangent {
                curves: vec![],
                points: vec![valid(centre, "point")?],
            });
        }
        if small <= LINEAR_TOLERANCE {
            return Err(crate::KernelError::Unsupported(format!(
                "an intersection ellipse narrower than {LINEAR_TOLERANCE} mm"
            )));
        }
        return Ok(SurfaceIntersection::Curves {
            curves: vec![ellipse_curve(centre, n, e1, big, small)?],
            points: vec![],
        });
    }
    if delta >= -ANGULAR_TOLERANCE {
        return Ok(if d < 0.0 {
            SurfaceIntersection::Empty
        } else {
            SurfaceIntersection::Unsupported(UnsupportedIntersection::ConeParabola)
        });
    }
    // A plane steeper than the generators cuts both nappes, one hyperbola branch each.
    Ok(SurfaceIntersection::Unsupported(
        UnsupportedIntersection::ConeHyperbola,
    ))
}

/// A plane and a torus (major `R`, minor `a`). Perpendicular planes go through the
/// meridian method. A plane through the axis meets the torus in its two meridian circles,
/// radius `a` about the points `±R` from the axis across the plane. A plane farther than
/// `|R| + a` from the centre misses the bounding sphere. Other planes are unsupported.
pub(crate) fn plane_torus(plane: &Frame, t: &Torus) -> KernelResult<SurfaceIntersection> {
    let f = t.frame();
    let n = plane.z();
    if parallel(n, f.z()) {
        return revolution::plane_perpendicular(plane, &Surface::Torus(*t));
    }
    let d = n.dot(f.origin() - plane.origin());
    if d.abs() > t.major_radius().abs() + t.minor_radius() + LINEAR_TOLERANCE {
        return Ok(SurfaceIntersection::Empty);
    }
    if perpendicular(n, f.z()) && d.abs() <= LINEAR_TOLERANCE {
        let o = f.origin() + n * -d;
        let axis = (f.z() - n * n.dot(f.z()))
            .normalized()
            .ok_or_else(|| beyond_range("circle"))?;
        let across = n.cross(axis);
        let big = t.major_radius();
        let mut curves = Vec::new();
        for sign in [-1.0, 1.0] {
            curves.push(circle_curve(
                o + across * (sign * big),
                n,
                across,
                t.minor_radius(),
                Contact::Crossing,
            )?);
        }
        return Ok(SurfaceIntersection::Curves {
            curves,
            points: vec![],
        });
    }
    Ok(SurfaceIntersection::Unsupported(
        UnsupportedIntersection::TorusSection,
    ))
}

fn valid(p: Point3, what: &str) -> KernelResult<Point3> {
    if p.is_valid() {
        Ok(p)
    } else {
        Err(beyond_range(what))
    }
}

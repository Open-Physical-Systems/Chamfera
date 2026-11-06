//! Surface-surface intersection (rung K6), curve-surface intersection and point-solid
//! classification: the geometric questions booleans (K7) ask.
//!
//! # Surface-surface: [`intersect_surfaces`]
//! Every case is analytic and returns exact K1 curves ([`Curve`]: lines, circles,
//! ellipses) or an explicit [`SurfaceIntersection`]:
//! - [`SurfaceIntersection::Empty`]: no common point;
//! - [`SurfaceIntersection::Coincident`]: the same surface (within tolerance);
//! - [`SurfaceIntersection::Curves`]: the surfaces cross along at least one curve. Each
//!   [`IntersectionCurve`] says whether the surfaces cross or touch along it
//!   ([`Contact`]); isolated common points (a cone's apex, a pole) come alongside;
//! - [`SurfaceIntersection::Tangent`]: the surfaces only touch, along curves or at points;
//! - [`SurfaceIntersection::Unsupported`]: the intersection is a curve K1 cannot
//!   represent exactly ([`UnsupportedIntersection`] names which); these wait for the
//!   NURBS curves of K10.
//!
//! | Pair | Exact results | Unsupported |
//! |---|---|---|
//! | plane / plane | line, coincident, parallel (empty) | |
//! | plane / cylinder | two lines, one tangent line, empty (plane parallel to the axis); circle (perpendicular); ellipse (oblique) | |
//! | plane / sphere | circle, tangent point, empty | |
//! | plane / cone | circle, ellipse, apex point, one tangent generator, two generators through the apex (rays), empty | parabola, hyperbola |
//! | plane / torus | up to four circles (perpendicular to the axis; two for a ring torus); two circles (plane through the axis); empty (beyond the bounding sphere) | other planes (spiric sections, Villarceau circles) |
//! | cylinder / cylinder | coaxial (coincident or empty); parallel axes (0 to 2 lines, tangent line); equal radii with intersecting axes (two ellipses in the planes bisecting the axes); empty (axes farther apart than the radii) | other skew or unequal cases (quartic curves) |
//! | two coaxial surfaces of revolution (cylinder, cone, sphere, torus; a sphere is coaxial with anything whose axis passes through its centre, and any two spheres are) | circles and points on the axis, crossing or tangent; coincident | |
//! | other curved pairs | empty when a bounding sphere proves it | the rest |
//!
//! Coaxial surfaces of revolution are intersected in a meridian half-plane: each surface
//! meets the half-plane through the axis in its meridian curves (lines for cylinders and
//! cones, circles for spheres and tori), every common point `(ρ, h)` of two meridians
//! sweeps a common circle of radius `ρ` at height `h`, and a common tangent there makes
//! the surfaces tangent along that circle (Hoffmann, *Geometric and Solid Modeling*,
//! chapter 7; Miller and Goldman, "Geometric algorithms for detecting and calculating all
//! conic sections in the intersection of any two natural quadric surfaces", 1995, which
//! treats the coaxial and plane cases geometrically in the same spirit).
//!
//! # Tolerance policy
//! One tolerance band per decision, from `chamfera_geom`: directions are parallel or
//! perpendicular when the sine of the angle between them is at most
//! [`ANGULAR_TOLERANCE`]; a distance decides touching when it is within
//! [`LINEAR_TOLERANCE`] of the critical value (a plane at distance `r ± LINEAR_TOLERANCE`
//! from a cylinder's axis is tangent; farther out it misses, farther in it cuts two
//! lines). Each decision is taken once, on one scalar, so the classification flips
//! consistently as the input moves. Results are built through the K1 constructors, which
//! validate them: a result that would leave the working range
//! ([`chamfera_geom::MAX_COORDINATE`]) or would not be finite is an error, never a NaN.
//! Work is bounded: every case is closed form or a fixed-degree polynomial.
//!
//! # Curve-surface: [`intersect_curve_surface`]
//! See `curve.rs`: lines, segments, circles, arcs, ellipses and elliptical arcs against
//! every surface, with parameters, points and tangency.
//!
//! # Point-solid: [`classify_point`]
//! See `classify.rs`: ray casting with the K3 inside-face test.

mod classify;
mod curve;
mod cylinders;
mod plane;
mod poly;
mod revolution;
#[cfg(test)]
mod tests;

pub use classify::{PointInSolid, RAY_DIRECTIONS, classify_point};
pub use curve::{
    CurveSurfaceIntersection, CurveSurfacePoint, ON_SURFACE_SAMPLES, intersect_curve_surface,
    intersect_curve_surface_on,
};

use crate::{Circle3, Contact, Curve, Ellipse3, KernelError, KernelResult, Line3, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::TAU;

/// A curve along which two surfaces meet.
#[derive(Debug, Clone, PartialEq)]
pub struct IntersectionCurve {
    pub curve: Curve,
    /// The parameter range of `curve` that lies on both surfaces: `(0, 2π)` for a circle
    /// or ellipse, `(-∞, ∞)` for a whole line, `(0, ∞)` for a ray from a cone's apex
    /// (the line's origin is the apex and its direction points into the nappe).
    pub range: (f64, f64),
    /// [`Contact::Crossing`] when the surfaces cross along the curve,
    /// [`Contact::Tangent`] when they touch along it (same tangent plane).
    pub contact: Contact,
}

/// A curve case K6 does not represent exactly yet. Each needs a curve type K1 lacks;
/// they wait for K10 (NURBS curves) unless a dedicated exact type is added first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedIntersection {
    /// A plane parallel to one generator of a cone, not through the apex: a parabola.
    ConeParabola,
    /// A plane steeper than the cone's generators, not through the apex: one branch of a
    /// hyperbola on the cone's nappe.
    ConeHyperbola,
    /// A plane meeting a torus neither perpendicular to its axis nor through it: spiric
    /// sections (quartics, including the Villarceau circle pairs of bitangent planes).
    TorusSection,
    /// Two cylinders with non-parallel axes that do not meet with equal radii: a quartic
    /// space curve.
    CylinderQuartic,
    /// Two curved surfaces that are not coaxial (other than the cylinder pairs above),
    /// and no bounding sphere proves them apart.
    NonCoaxial,
    /// A surface of linear extrusion of a B-spline (K10 surfaces have no intersections
    /// yet).
    Spline,
}

impl std::fmt::Display for UnsupportedIntersection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            UnsupportedIntersection::ConeParabola => {
                "a plane parallel to a cone's generator cuts a parabola"
            }
            UnsupportedIntersection::ConeHyperbola => {
                "a plane steeper than a cone's generators cuts a hyperbola"
            }
            UnsupportedIntersection::TorusSection => {
                "a plane neither perpendicular to a torus's axis nor through it cuts a quartic"
            }
            UnsupportedIntersection::CylinderQuartic => {
                "these two cylinders meet in a quartic space curve"
            }
            UnsupportedIntersection::NonCoaxial => {
                "these curved surfaces are not coaxial; their intersection needs NURBS curves (K10)"
            }
            UnsupportedIntersection::Spline => {
                "a spline surface's intersections are not supported yet (K10)"
            }
        })
    }
}

/// Everything two surfaces have in common.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceIntersection {
    /// No common point.
    Empty,
    /// The two surfaces are the same point set (within tolerance).
    Coincident,
    /// The surfaces cross along at least one curve. `curves` may also hold curves along
    /// which they touch (each says which); `points` are isolated common points (an apex
    /// or pole where the surfaces meet without a curve).
    Curves {
        curves: Vec<IntersectionCurve>,
        points: Vec<Point3>,
    },
    /// The surfaces only touch: every curve has [`Contact::Tangent`], and `points` are
    /// isolated touching points. At least one of the two lists is non-empty.
    Tangent {
        curves: Vec<IntersectionCurve>,
        points: Vec<Point3>,
    },
    /// The intersection is a curve K6 cannot represent exactly yet.
    Unsupported(UnsupportedIntersection),
}

impl SurfaceIntersection {
    /// The curves of a `Curves` or `Tangent` result (empty otherwise).
    pub fn curves(&self) -> &[IntersectionCurve] {
        match self {
            SurfaceIntersection::Curves { curves, .. }
            | SurfaceIntersection::Tangent { curves, .. } => curves,
            _ => &[],
        }
    }
    /// The isolated points of a `Curves` or `Tangent` result (empty otherwise).
    pub fn points(&self) -> &[Point3] {
        match self {
            SurfaceIntersection::Curves { points, .. }
            | SurfaceIntersection::Tangent { points, .. } => points,
            _ => &[],
        }
    }

    /// The result for these parts: `Curves` when a curve crosses, `Tangent` when there
    /// is anything else, `Empty` when there is nothing.
    pub(crate) fn from_parts(curves: Vec<IntersectionCurve>, points: Vec<Point3>) -> Self {
        if curves.iter().any(|c| c.contact == Contact::Crossing) {
            SurfaceIntersection::Curves { curves, points }
        } else if curves.is_empty() && points.is_empty() {
            SurfaceIntersection::Empty
        } else {
            SurfaceIntersection::Tangent { curves, points }
        }
    }
}

/// The intersection of two surfaces (see the module documentation for the cases). The
/// result does not depend on the order of `a` and `b` beyond the order of its curves.
/// Errors only when a result would leave the working range.
pub fn intersect_surfaces(a: &Surface, b: &Surface) -> KernelResult<SurfaceIntersection> {
    use Surface as S;
    match (a, b) {
        (S::Extrusion(_), _) | (_, S::Extrusion(_)) => Ok(SurfaceIntersection::Unsupported(
            UnsupportedIntersection::Spline,
        )),
        (S::Plane(p), S::Plane(q)) => plane::plane_plane(p, q),
        (S::Plane(p), other) | (other, S::Plane(p)) => plane::plane_surface(p, other),
        (S::Cylinder(c), S::Cylinder(d)) => cylinders::cylinder_cylinder(c, d),
        _ => {
            if let Some(axis) = revolution::common_axis(a, b) {
                return revolution::coaxial(&axis, a, b);
            }
            if bounding_spheres_apart(a, b) || bounding_spheres_apart(b, a) {
                return Ok(SurfaceIntersection::Empty);
            }
            Ok(SurfaceIntersection::Unsupported(
                UnsupportedIntersection::NonCoaxial,
            ))
        }
    }
}

/// True when `a` is bounded (a sphere or torus) and its bounding sphere stays more than
/// [`LINEAR_TOLERANCE`] away from `b`, which proves the two apart.
fn bounding_spheres_apart(a: &Surface, b: &Surface) -> bool {
    let (centre, radius) = match a {
        Surface::Sphere(s) => (s.frame().origin(), s.radius()),
        Surface::Torus(t) => (
            t.frame().origin(),
            t.major_radius().abs() + t.minor_radius(),
        ),
        _ => return false,
    };
    b.distance(centre) > radius + LINEAR_TOLERANCE
}

// ------------------------------------------------------------------ shared helpers

/// Two unit directions are parallel (or anti-parallel): `|a × b| <= ANGULAR_TOLERANCE`.
pub(crate) fn parallel(a: Vec3, b: Vec3) -> bool {
    a.cross(b).length() <= ANGULAR_TOLERANCE
}

/// Two unit directions are perpendicular: `|a · b| <= ANGULAR_TOLERANCE`.
pub(crate) fn perpendicular(a: Vec3, b: Vec3) -> bool {
    a.dot(b).abs() <= ANGULAR_TOLERANCE
}

pub(crate) fn beyond_range(what: &str) -> KernelError {
    KernelError::InvalidInput(format!(
        "the intersection {what} would lie beyond the working range of ±{} mm",
        chamfera_geom::MAX_COORDINATE
    ))
}

/// A frame at `origin` with z axis `normal` and x along `x_hint` (or a deterministic x
/// when the hint is parallel to the normal).
pub(crate) fn frame_at(origin: Point3, normal: Vec3, x_hint: Vec3) -> KernelResult<Frame> {
    Frame::from_normal_and_x(origin, normal, x_hint)
        .or_else(|| Frame::from_normal(origin, normal))
        .ok_or_else(|| beyond_range("curve's frame"))
}

pub(crate) fn circle_curve(
    centre: Point3,
    normal: Vec3,
    x_hint: Vec3,
    radius: f64,
    contact: Contact,
) -> KernelResult<IntersectionCurve> {
    let frame = frame_at(centre, normal, x_hint)?;
    let circle = Circle3::new(frame, radius).map_err(|_| beyond_range("circle"))?;
    Ok(IntersectionCurve {
        curve: Curve::Circle(circle),
        range: (0.0, TAU),
        contact,
    })
}

pub(crate) fn ellipse_curve(
    centre: Point3,
    normal: Vec3,
    major: Vec3,
    rx: f64,
    ry: f64,
) -> KernelResult<IntersectionCurve> {
    let frame = frame_at(centre, normal, major)?;
    let e = Ellipse3::new(frame, rx, ry).map_err(|_| beyond_range("ellipse"))?;
    Ok(IntersectionCurve {
        curve: Curve::Ellipse(e),
        range: (0.0, TAU),
        contact: Contact::Crossing,
    })
}

/// A whole line, or a ray `t >= 0` from `origin` when `ray`.
pub(crate) fn line_curve(
    origin: Point3,
    direction: Vec3,
    ray: bool,
    contact: Contact,
) -> KernelResult<IntersectionCurve> {
    let line = Line3::new(origin, direction).map_err(|_| beyond_range("line"))?;
    Ok(IntersectionCurve {
        curve: Curve::Line(line),
        range: (if ray { 0.0 } else { f64::NEG_INFINITY }, f64::INFINITY),
        contact,
    })
}

/// `sqrt(r² - d²)` computed as `sqrt((r - d)(r + d))` for accuracy, never NaN.
pub(crate) fn half_chord(r: f64, d: f64) -> f64 {
    ((r - d) * (r + d)).max(0.0).sqrt()
}

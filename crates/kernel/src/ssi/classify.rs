//! Point-solid classification by ray casting (Mantyla, *An Introduction to Solid
//! Modeling*, chapter 12; Requicha and Tilove's point membership classification).
//!
//! A point on a face (inside it or on its boundary, by the K3 inside-face test
//! [`Body::classify_point_on_face`]) is on the solid's boundary. Otherwise a ray from the
//! point is intersected with every face's surface ([`intersect_curve_surface_on`]) and
//! the hits inside their faces are counted: an odd count means inside.
//!
//! # Degenerate rays
//! A ray is degenerate when it lies on a face's surface, touches a surface tangentially
//! at a point of the face, or hits a face on its boundary (an edge or vertex, where the
//! crossing count would be ambiguous) or where the inside-face test cannot decide (on a
//! surface's axis). A degenerate ray is abandoned and the next of
//! [`RAY_DIRECTIONS`]'s fixed directions is cast: they are chosen far from the
//! coordinate axes, their diagonals and from each other, so a model built from usual
//! directions cannot make all of them degenerate. If every direction is degenerate the
//! classification fails with [`KernelError::NoConvergence`] instead of guessing.

use super::curve::{CurveSurfaceIntersection, intersect_curve_surface_on};
use crate::{Body, Contact, Curve, KernelError, KernelResult, Line3, PointOnFace};
use chamfera_geom::{LINEAR_TOLERANCE, Point3, Vec3};

/// Where a point lies relative to a solid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointInSolid {
    Inside,
    Outside,
    /// Within [`LINEAR_TOLERANCE`] of a face (including its edges and vertices).
    OnBoundary,
}

/// The ray directions tried in order (normalised before use).
pub const RAY_DIRECTIONS: [Vec3; 8] = [
    Vec3::new(0.4273, 0.5761, 0.6967),
    Vec3::new(-0.6512, 0.3374, 0.6797),
    Vec3::new(0.2795, -0.7953, 0.5378),
    Vec3::new(-0.3581, -0.4669, -0.8086),
    Vec3::new(0.8172, 0.1931, -0.5432),
    Vec3::new(-0.1137, 0.9305, -0.3484),
    Vec3::new(0.5862, -0.3147, -0.7466),
    Vec3::new(-0.9071, -0.2248, 0.3559),
];

/// Whether `p` is inside `body`, outside it or on its boundary. Needs a surface on every
/// face and a curve on every edge.
pub fn classify_point(body: &Body, p: Point3) -> KernelResult<PointInSolid> {
    crate::check::point3("point", p)?;
    let faces = body.faces();
    if faces.is_empty() {
        return Err(KernelError::InvalidInput(
            "a body to classify against needs at least one face".into(),
        ));
    }
    let mut surfaces = Vec::with_capacity(faces.len());
    for f in &faces {
        if body.classify_point_on_face(*f, p)? != PointOnFace::Outside {
            return Ok(PointInSolid::OnBoundary);
        }
        let (s, _) = body
            .face_surface(*f)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{f} has no surface yet")))?;
        surfaces.push(s);
    }
    'directions: for d in RAY_DIRECTIONS {
        let Some(dir) = d.normalized() else {
            continue;
        };
        let ray = Curve::Line(Line3::new(p, dir)?);
        let mut crossings = 0usize;
        for (f, s) in faces.iter().zip(&surfaces) {
            let hits = match intersect_curve_surface_on(&ray, 0.0, f64::INFINITY, s)? {
                CurveSurfaceIntersection::OnSurface { .. } => continue 'directions,
                CurveSurfaceIntersection::Points(hits) => hits,
            };
            for h in hits {
                // The start point is on no face (checked above).
                if h.t <= LINEAR_TOLERANCE {
                    continue;
                }
                match body.classify_point_on_face(*f, h.point) {
                    Ok(PointOnFace::Outside) => {}
                    Ok(PointOnFace::Inside) if h.contact == Contact::Crossing => crossings += 1,
                    _ => continue 'directions,
                }
            }
        }
        return Ok(if crossings % 2 == 1 {
            PointInSolid::Inside
        } else {
            PointInSolid::Outside
        });
    }
    Err(KernelError::NoConvergence(format!(
        "every one of the {} ray directions met a face degenerately",
        RAY_DIRECTIONS.len()
    )))
}

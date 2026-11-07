//! Direct face editing (rung K12, with the shell): [`offset_faces`] moves faces along
//! their normals, [`delete_faces`] removes faces and heals the gap, [`move_faces`]
//! translates or turns planar faces. The faces around an edited face follow: they are
//! extended or trimmed to the new surfaces, and the edges and vertices between them are
//! worked out again on exact geometry.
//!
//! # Method
//! A face edit keeps the body's surfaces as the description of its shape and rebuilds the
//! rest from them (Stroud, *Boundary Representation Modelling Techniques*, 2006, the
//! chapters on local operations and offsets; Forsyth, "Shelling and offsetting bodies",
//! 1995, for offsetting a face while its neighbours follow):
//!
//! 1. The body is copied into a draft of faces, loops, edges and vertices (`draft.rs`).
//! 2. The edit changes the draft: an offset replaces each chosen face's surface by its
//!    offset (a plane moves along its normal; a cylinder, sphere or torus tube changes
//!    radius by the distance; a cone keeps axis and half-angle while its radius changes by
//!    `d / cos α`, as in the shell), a move transforms a plane, a delete changes the
//!    topology (`delete.rs`).
//! 3. Every vertex whose faces changed is moved onto all of its faces' surfaces
//!    (Gauss-Newton from the old point), every edge whose vertices or faces changed gets
//!    the exact curve where its two faces now meet (a segment, a coaxial circle, or the
//!    K6 intersection: line, circle, ellipse), checked along its length.
//! 4. The body is assembled from the face list and validated ([`Body::validate`]).
//! 5. The result may not cross itself: with `V` the volume, the added material
//!    `new − old` and the removed `old − new` (K7) must satisfy
//!    `V(new) = V(old) + V(new − old) − V(old − new)`, which a body whose faces cross each
//!    other breaks. An offset outward must only add material and one inward only remove.
//!    Where K7 cannot intersect a face pair yet (a plane cutting a cone in a hyperbola),
//!    only the sign of the volume change is checked.
//!
//! The distance of an offset is along each face's outward normal: positive grows the
//! solid there (a top face rises, a boss gets fatter, a hole gets narrower), negative
//! shrinks it.
//!
//! # Refused
//! - Hostile distances and transforms, unknown faces, a body that is not valid:
//!   [`KernelError::InvalidInput`].
//! - A face or edge that would vanish or turn over, a face that would cross another
//!   (the volume check above), a vertex whose faces no longer meet in one point (a face
//!   moved off a tangent neighbour): [`KernelError::InvalidInput`] or
//!   [`KernelError::Unsupported`].
//! - New edges that are not lines, circles or ellipses (K6 [`UnsupportedIntersection`]
//!   cases such as a cone cut in a parabola), vertices on a surface's axis, lemon tori:
//!   [`KernelError::Unsupported`].
//! - Deleting faces other than fillets and chamfers with four edges, or groups that fill
//!   a hole, pocket or boss through inner loops of the faces around them:
//!   [`KernelError::Unsupported`].
//!
//! Nothing half-built is returned. Faces, edges and vertices keep their names, so
//! references to them survive the edit; deleted faces' names are gone.
//!
//! [`UnsupportedIntersection`]: crate::UnsupportedIntersection

mod delete;
mod draft;
#[cfg(test)]
mod tests;

use crate::boolean::{MAX_BOOLEAN_FACES, subtract};
use crate::topology::{Body, FaceId};
use crate::{KernelError, KernelResult, Surface};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE, Transform};
use draft::Draft;
use std::collections::{BTreeMap, BTreeSet};

/// Relative agreement of the volume identity in step 5. Volumes of curved faces come
/// from adaptive quadrature (relative), so the check is relative too; a face crossing
/// another changes the balance by far more.
const VOLUME_AGREEMENT: f64 = 1e-6;

/// What the result of an edit may do to the material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Adds,
    Removes,
    Either,
}

/// `body` with each face of `faces` moved by `distance` along its outward normal and its
/// neighbours following. See the module documentation for what is supported.
pub fn offset_faces(body: &Body, faces: &[FaceId], distance: f64) -> KernelResult<Body> {
    if !(distance.is_finite()
        && distance.abs() > LINEAR_TOLERANCE
        && distance.abs() <= MAX_COORDINATE)
    {
        return Err(KernelError::InvalidInput(format!(
            "the offset distance must be a finite length with {LINEAR_TOLERANCE} < |distance| <= {MAX_COORDINATE} mm (not 0)"
        )));
    }
    let (mut draft, chosen) = prepare(body, faces)?;
    for &i in &chosen {
        let Some(Some(f)) = draft.faces.get_mut(i) else {
            continue;
        };
        let delta = if f.same_sense { distance } else { -distance };
        let name = f.name.clone().unwrap_or_else(|| format!("face {i}"));
        f.surface = offset_surface(&f.surface, delta, &name)?;
        f.changed = true;
    }
    let out = draft.build().map_err(too_far)?;
    let change = if distance > 0.0 {
        Change::Adds
    } else {
        Change::Removes
    };
    check_no_crossing(body, &out, change)?;
    Ok(out)
}

/// `body` with the planar faces `faces` moved by the rigid transform `t` (a translation,
/// a turn, or both) and their neighbours following.
pub fn move_faces(body: &Body, faces: &[FaceId], t: &Transform) -> KernelResult<Body> {
    if !t.is_rigid() {
        return Err(KernelError::InvalidInput(
            "a face moves by a translation or a rotation only".into(),
        ));
    }
    let (mut draft, chosen) = prepare(body, faces)?;
    for &i in &chosen {
        let Some(Some(f)) = draft.faces.get_mut(i) else {
            continue;
        };
        let Surface::Plane(frame) = f.surface else {
            return Err(KernelError::Unsupported(
                "moving a curved face: only planar faces move for now".into(),
            ));
        };
        let moved = frame.transformed(t).ok_or_else(|| {
            KernelError::InvalidInput("the moved face leaves the working range".into())
        })?;
        f.surface = Surface::Plane(moved);
        f.changed = true;
    }
    let out = draft.build().map_err(too_far)?;
    check_no_crossing(body, &out, Change::Either)?;
    Ok(out)
}

/// `body` without the faces `faces`, the gap closed by extending the faces around it
/// (a fillet or chamfer face removed, a hole filled, a boss taken off).
pub fn delete_faces(body: &Body, faces: &[FaceId]) -> KernelResult<Body> {
    let (mut draft, chosen) = prepare(body, faces)?;
    if chosen.len() == draft.faces.iter().flatten().count() {
        return Err(KernelError::InvalidInput(
            "deleting every face leaves nothing".into(),
        ));
    }
    delete::delete_in_draft(&mut draft, &chosen)?;
    let out = draft.build()?;
    check_no_crossing(body, &out, Change::Either)?;
    Ok(out)
}

/// The draft of a valid body and the draft indices of `faces` (at least one, each once).
fn prepare(body: &Body, faces: &[FaceId]) -> KernelResult<(Draft, BTreeSet<usize>)> {
    let all = body.faces();
    if all.is_empty() {
        return Err(KernelError::InvalidInput("the body is empty".into()));
    }
    if all.len() > MAX_BOOLEAN_FACES {
        return Err(KernelError::InvalidInput(format!(
            "a body to edit may have at most {MAX_BOOLEAN_FACES} faces"
        )));
    }
    if faces.is_empty() {
        return Err(KernelError::InvalidInput("pick at least one face".into()));
    }
    if faces.len() > all.len() {
        return Err(KernelError::InvalidInput(
            "more faces picked than the body has".into(),
        ));
    }
    let index: BTreeMap<FaceId, usize> = all.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut chosen = BTreeSet::new();
    for f in faces {
        let i = index
            .get(f)
            .ok_or_else(|| KernelError::InvalidInput(format!("{f} is not a face of the body")))?;
        chosen.insert(*i);
    }
    body.validate().map_err(|e| {
        KernelError::InvalidInput(format!("the body to edit is not a valid solid: {e}"))
    })?;
    Ok((Draft::from_body(body)?, chosen))
}

/// `s` offset by `delta` along its surface normal.
fn offset_surface(s: &Surface, delta: f64, name: &str) -> KernelResult<Surface> {
    let vanish = || {
        KernelError::InvalidInput(format!(
            "the offset of {name} by {delta} mm would make it vanish"
        ))
    };
    match s {
        Surface::Plane(f) => f
            .with_origin(f.origin() + f.z() * delta)
            .map(Surface::Plane)
            .ok_or_else(|| {
                KernelError::InvalidInput(format!("the offset of {name} leaves the working range"))
            }),
        Surface::Cylinder(c) => {
            let r = c.radius() + delta;
            if r <= LINEAR_TOLERANCE {
                return Err(vanish());
            }
            Surface::cylinder(*c.frame(), r)
        }
        Surface::Cone(c) => {
            let a = c.half_angle();
            let r = c.radius() + delta / a.cos();
            if r > LINEAR_TOLERANCE {
                return Surface::cone(*c.frame(), r, a);
            }
            // The offset generator crosses the frame's plane at a negative radius: put the
            // frame where the offset nappe has radius `target` (`ρ = r + z tan α`), as the
            // shell does.
            let target = 1.0 + r.abs();
            let z0 = (target - r) / a.tan();
            let f = c.frame();
            let frame = f.with_origin(f.origin() + f.z() * z0).ok_or_else(vanish)?;
            Surface::cone(frame, target, a)
        }
        Surface::Sphere(sp) => {
            let r = sp.radius() + delta;
            if r <= LINEAR_TOLERANCE {
                return Err(vanish());
            }
            Surface::sphere(*sp.frame(), r)
        }
        Surface::Torus(t) => {
            let a = t.minor_radius() + delta;
            if a <= LINEAR_TOLERANCE || t.major_radius() <= LINEAR_TOLERANCE {
                return Err(vanish());
            }
            Surface::torus(*t.frame(), t.major_radius(), a)
        }
        Surface::Extrusion(_) => Err(KernelError::Unsupported(format!(
            "offsetting the spline face {name} (K10): planes, cylinders, cones, spheres and tori only"
        ))),
    }
}

/// An edge or face that no longer closes after a move means the move went too far.
fn too_far(e: KernelError) -> KernelError {
    match e {
        KernelError::InvalidResult(m) => KernelError::InvalidInput(format!(
            "the faces would cross or turn over at that distance: {m}"
        )),
        other => other,
    }
}

/// The volume of `a − b`, zero when nothing is left.
fn difference_volume(a: &Body, b: &Body) -> KernelResult<f64> {
    match subtract(a, b) {
        Ok(d) => d.volume(),
        Err(KernelError::EmptyResult) => Ok(0.0),
        Err(e) => Err(e),
    }
}

/// Refuses a result whose faces cross each other (step 5 of the method).
fn check_no_crossing(old: &Body, new: &Body, change: Change) -> KernelResult<()> {
    let crossing = || {
        KernelError::InvalidInput(
            "the edited faces would cross other faces of the body; use a smaller distance".into(),
        )
    };
    let v_old = old.volume()?;
    let v_new = new.volume()?;
    if v_new.is_nan() || v_new <= 0.0 {
        return Err(crossing());
    }
    let scale = v_old.max(v_new).max(1.0);
    let tiny = VOLUME_AGREEMENT * scale;
    let (added, removed) = match (difference_volume(new, old), difference_volume(old, new)) {
        (Ok(a), Ok(r)) => (a, r),
        // K7 cannot intersect some face pairs yet (a plane cutting a cone in a hyperbola):
        // fall back to the volume moving the expected way.
        (Err(KernelError::Unsupported(_)), _) | (_, Err(KernelError::Unsupported(_))) => {
            return match change {
                Change::Adds if v_new < v_old - tiny => Err(crossing()),
                Change::Removes if v_new > v_old + tiny => Err(crossing()),
                _ => Ok(()),
            };
        }
        (Err(e), _) | (_, Err(e)) => return Err(e),
    };
    if (v_new - (v_old + added - removed)).abs() > VOLUME_AGREEMENT * scale {
        return Err(crossing());
    }
    match change {
        Change::Adds if removed > tiny => Err(crossing()),
        Change::Removes if added > tiny => Err(crossing()),
        _ => Ok(()),
    }
}

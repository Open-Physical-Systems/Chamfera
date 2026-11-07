//! Shelling (rung K12, first part): [`shell`] hollows a solid into walls of a given
//! thickness, with chosen faces removed to open it.
//!
//! # Method
//! A shell is the solid minus its inward offset (Forsyth, "Shelling and offsetting
//! bodies", *Proceedings of the Third ACM Symposium on Solid Modeling and Applications*,
//! 1995; Stroud, *Boundary Representation Modelling Techniques*, 2006, chapter on offsets
//! and shelling). Here it is built in two steps, each on exact geometry:
//!
//! 1. **Offset body** (`offset.rs`). Every face of the solid is moved toward the inside by
//!    the thickness `t`, keeping the solid's topology: a plane moves along its normal, a
//!    cylinder, sphere or torus tube changes radius by `t`, and a cone keeps its axis and
//!    half-angle `α` while its radius changes by `t / cos α` (the surfaces of revolution
//!    offset into surfaces of revolution about the same axis: do Carmo, *Differential
//!    Geometry of Curves and Surfaces*, section 2-3, the offset of a meridian along its
//!    normal). An open face is not moved: the offset body is flush with the solid there.
//!    Each vertex moves to the point that lies on every offset surface of its faces
//!    (Gauss-Newton on the signed distances, from the old point, with the least-norm step
//!    where the faces fix fewer than three directions: a seam or a tangent face), each
//!    edge to the curve on its two offset faces through its moved vertices (a segment, or
//!    a circle about the same axis), checked against both surfaces along its length.
//! 2. **Subtract** (K7). The shell is the solid minus the offset body. Where a face is
//!    open, the two bodies share that face with the same normal, so the subtraction drops
//!    it and leaves the opening; with no open face the offset body is a void inside the
//!    solid (a closed hollow body, two shells).
//!
//! A negative thickness shells outward: the faces move outward, and the shell is the
//! offset body minus the solid.
//!
//! # Refused
//! - A thickness that is not a finite length with `LINEAR_TOLERANCE < |t| <=
//!   MAX_COORDINATE` (zero included), unknown open faces, every face open, a body that is
//!   not one valid lump without voids: [`KernelError::InvalidInput`] or
//!   [`KernelError::Unsupported`].
//! - Faces on spheres or tori, edges on ellipses, vertices on a surface's axis (poles and
//!   apexes), cones passing their apex and spindle or lemon tori: [`KernelError::Unsupported`]
//!   for now. Planes, cylinders and cones are the first supported surfaces (extruded and
//!   revolved parts, a boss on a plate, a block with filleted edges); spheres and tori are
//!   offset by the same rules but are refused until their edges are covered by tests.
//! - A thickness that makes an offset face or edge vanish or turn over (a cylinder of
//!   radius at most `t`, an edge whose ends cross), or makes the walls cross each other or
//!   the outside (found when the shell's volume is not the solid's minus the offset's):
//!   [`KernelError::InvalidInput`]. Nothing half-built is returned: the result passes
//!   [`Body::validate`].
//!
//! # Names
//! The solid's faces keep their names (`top` stays `top`); each wall face made by the
//! offset is named from the face it offsets, `shell of <face>` (an unnamed face is called
//! `face <n>`). Edges and vertices of the offset are `shell of <edge>` and
//! `shell of <vertex>` when the original has a name; the subtraction names the edges
//! where walls meet open faces `<face> x shell of <face>` (K7).

pub(crate) mod offset;
#[cfg(test)]
mod tests;

use crate::boolean::{MAX_BOOLEAN_FACES, subtract};
use crate::topology::{Body, FaceId, MAX_NAME_LENGTH};
use crate::{KernelError, KernelResult};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE};
use std::collections::BTreeSet;

/// Relative agreement required between the shell's volume and the solid's minus the
/// offset body's. A separate constant because volumes of curved faces come from adaptive
/// quadrature (`SURFACE_QUADRATURE_TOLERANCE`, relative), so the check is relative too;
/// a wall crossing another or the outside changes the volume by far more.
const VOLUME_AGREEMENT: f64 = 1e-6;

/// `body` hollowed to walls of `thickness`, with `open_faces` removed. A positive
/// thickness keeps the outside and moves the walls inward; a negative one keeps the
/// solid as the inside and grows the walls outward. See the module documentation for
/// what is supported, what is refused and how faces are named.
pub fn shell(body: &Body, open_faces: &[FaceId], thickness: f64) -> KernelResult<Body> {
    if !(thickness.is_finite()
        && thickness.abs() > LINEAR_TOLERANCE
        && thickness.abs() <= MAX_COORDINATE)
    {
        return Err(KernelError::InvalidInput(format!(
            "the shell thickness must be a finite length with {LINEAR_TOLERANCE} < |thickness| <= {MAX_COORDINATE} mm (not 0)"
        )));
    }
    let faces = body.faces();
    if faces.is_empty() {
        return Err(KernelError::InvalidInput(
            "the body to shell is empty".into(),
        ));
    }
    if faces.len() > MAX_BOOLEAN_FACES {
        return Err(KernelError::InvalidInput(format!(
            "a body to shell may have at most {MAX_BOOLEAN_FACES} faces"
        )));
    }
    if open_faces.len() > faces.len() {
        return Err(KernelError::InvalidInput(
            "more open faces than the body has faces".into(),
        ));
    }
    let open: BTreeSet<FaceId> = open_faces.iter().copied().collect();
    for f in &open {
        body.face_shell(*f)?;
    }
    if open.len() == faces.len() {
        return Err(KernelError::InvalidInput(
            "at least one face must stay closed: with every face open no wall is left".into(),
        ));
    }
    body.validate().map_err(|e| {
        KernelError::InvalidInput(format!("the body to shell is not a valid solid: {e}"))
    })?;
    if body.shells().len() != 1 {
        return Err(KernelError::Unsupported(
            "shelling a body of several lumps or with a void".into(),
        ));
    }
    let moved = offset::offset_body(body, &open, thickness)?;
    let (outside, inside) = if thickness > 0.0 {
        (body, &moved)
    } else {
        (&moved, body)
    };
    let v_out = outside.volume()?;
    let v_in = inside.volume()?;
    if !(v_in > 0.0 && v_in < v_out) {
        return Err(too_thick(thickness, "the offset leaves no room for a wall"));
    }
    let out = subtract(outside, inside).map_err(|e| match e {
        KernelError::EmptyResult => too_thick(thickness, "no wall is left"),
        other => other,
    })?;
    let want = v_out - v_in;
    let got = out.volume()?;
    if (got - want).abs() > VOLUME_AGREEMENT * v_out.max(1.0) {
        return Err(too_thick(
            thickness,
            "the offset walls would cross each other or the outside",
        ));
    }
    out.validate()
        .map_err(|e| KernelError::InvalidResult(format!("the shelled body: {e}")))?;
    Ok(out)
}

fn too_thick(thickness: f64, why: &str) -> KernelError {
    KernelError::InvalidInput(format!(
        "a wall thickness of {thickness} mm is too large for this shape: {why}"
    ))
}

/// `name` cut to at most [`MAX_NAME_LENGTH`] bytes on a character boundary.
fn bounded(name: String) -> String {
    if name.len() <= MAX_NAME_LENGTH {
        return name;
    }
    let mut end = MAX_NAME_LENGTH;
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name.get(..end).unwrap_or_default().to_owned()
}

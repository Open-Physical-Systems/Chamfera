//! Draft: planar faces tilted by an angle about a neutral plane, the faces around them
//! re-trimmed ([`draft_faces`]), and a body split into its lumps ([`lumps`]).
//!
//! # Method
//! A draft is a face move (the module documentation, steps 1 to 5) where each face turns
//! about its own axis. With `d` the pull direction (the neutral plane's unit normal) and
//! `m` a face's outward unit normal, the face's direction across the pull is
//! `p = (m - (m·d) d) / |m - (m·d) d|`, and the face already leans by `φ = asin(m·d)`
//! towards `d`. The drafted face has the normal `cos θ p + sin θ d`: it turns by `θ - φ`
//! about the axis `k = p × d` (for orthonormal `p` and `d`, `k × p = d`, so a positive turn
//! leans the normal towards the pull), through the line where the face's plane meets the
//! neutral plane, which therefore stays in place. A point `x = α m + β d` of that line
//! solves `m·x = m·o_f` and `d·x = d·o_n` (`o_f`, `o_n` points of the two planes):
//!
//! ```text
//! [ 1    m·d ] [α]   [m·o_f]
//! [ m·d  1   ] [β] = [d·o_n],   det = 1 - (m·d)² > 0 when the face is not parallel to the plane.
//! ```
//!
//! A positive `θ` makes the solid narrower along the pull: a wall `x = a` with outward
//! normal `+x`, pulled along `+z` from `z = 0`, becomes `x = a - z tan θ`. The faces around
//! the drafted ones keep their planes and are trimmed to the turned ones by the draft's
//! rebuild (Stroud 2006, local operations), and the result is checked not to cross itself.
//!
//! Only planar faces are drafted, and only where every face around them is planar: the
//! rebuild would have to find new plane-cylinder ellipses at seams it cannot place yet.

use super::draft::Draft;
use super::{Change, check_no_crossing, prepare, too_far};
use crate::topology::{Body, FaceId};
use crate::{KernelError, KernelResult, Surface};
use chamfera_geom::{ANGULAR_TOLERANCE, Frame, Point3, Transform};
use std::collections::BTreeSet;

/// The steepest draft: 80 degrees. Steeper faces meet their neighbours at grazing angles
/// where the trimmed edges would run far beyond the body.
pub const MAX_DRAFT_ANGLE: f64 = 80.0 * std::f64::consts::PI / 180.0;

/// Smallest `|m - (m·d) d|` of a face that can be drafted: below it the face is parallel
/// to the neutral plane (sin of the angle between them, 1e-6 rad).
const PARALLEL_TOLERANCE: f64 = 1e-6;

fn face_label(name: &Option<String>, i: usize) -> String {
    name.clone().unwrap_or_else(|| format!("face {i}"))
}

/// `body` with each planar face of `faces` tilted to `angle` radians from the pull
/// direction `neutral.z()`, turning about the line where it meets the neutral plane
/// (`neutral`'s XY plane); the faces around them are re-trimmed. A positive angle makes
/// the solid narrower along the pull, a negative one wider. See the module documentation.
pub fn draft_faces(
    body: &Body,
    faces: &[FaceId],
    neutral: &Frame,
    angle: f64,
) -> KernelResult<Body> {
    if !(angle.is_finite() && angle.abs() > ANGULAR_TOLERANCE && angle.abs() <= MAX_DRAFT_ANGLE) {
        return Err(KernelError::InvalidInput(format!(
            "the draft angle must be a finite angle with 0 < |angle| <= {MAX_DRAFT_ANGLE} radians (80 degrees)"
        )));
    }
    let (mut draft, chosen) = prepare(body, faces)?;
    let pull = neutral.z();
    let on_neutral = neutral.origin();
    let mut turns = Vec::with_capacity(chosen.len());
    for &i in &chosen {
        let Some(Some(f)) = draft.faces.get(i) else {
            continue;
        };
        let name = face_label(&f.name, i);
        let Surface::Plane(frame) = f.surface else {
            return Err(KernelError::Unsupported(format!(
                "drafting the curved face {name}: only planar faces take a draft for now"
            )));
        };
        check_planar_neighbours(&draft, i, &name)?;
        let m = if f.same_sense { frame.z() } else { -frame.z() };
        let s = m.dot(pull);
        let across = (m - pull * s)
            .normalized()
            .filter(|_| (m - pull * s).length() > PARALLEL_TOLERANCE);
        let Some(across) = across else {
            return Err(KernelError::InvalidInput(format!(
                "{name} is parallel to the neutral plane: only faces that run along the pull direction can be drafted"
            )));
        };
        let lean = s.clamp(-1.0, 1.0).asin();
        let turn = angle - lean;
        if turn.abs() <= ANGULAR_TOLERANCE {
            continue;
        }
        let axis = across.cross(pull);
        let det = 1.0 - s * s;
        let mf = m.dot(frame.origin() - Point3::ORIGIN);
        let dn = pull.dot(on_neutral - Point3::ORIGIN);
        let alpha = (mf - s * dn) / det;
        let beta = (dn - s * mf) / det;
        let pivot = Point3::ORIGIN + m * alpha + pull * beta;
        let t = Transform::rotation_about(pivot, axis, turn).ok_or_else(|| {
            KernelError::InvalidInput(format!("{name} cannot be turned about the neutral plane"))
        })?;
        let moved = frame.transformed(&t).ok_or_else(|| {
            KernelError::InvalidInput(format!("the drafted {name} leaves the working range"))
        })?;
        turns.push((i, moved));
    }
    if turns.is_empty() {
        return Err(KernelError::InvalidInput(
            "every face already has that draft".into(),
        ));
    }
    for (i, moved) in turns {
        if let Some(Some(f)) = draft.faces.get_mut(i) {
            f.surface = Surface::Plane(moved);
            f.changed = true;
        }
    }
    let out = draft.build().map_err(too_far)?;
    check_no_crossing(body, &out, Change::Either)?;
    Ok(out)
}

/// Refuses a face with a curved neighbour (see the module documentation).
fn check_planar_neighbours(draft: &Draft, i: usize, name: &str) -> KernelResult<()> {
    for (_, g) in draft.neighbours(i) {
        let Some(Some(other)) = draft.faces.get(g) else {
            continue;
        };
        if !matches!(other.surface, Surface::Plane(_)) {
            return Err(KernelError::Unsupported(format!(
                "{name} meets the curved face {}: a draft next to cylinders and other curved faces is not supported yet",
                face_label(&other.name, g)
            )));
        }
    }
    Ok(())
}

/// The lumps of `body` as separate bodies, in shell order, with their names. A body
/// with a void (an inner shell) is refused: a lump is built from one shell.
pub fn lumps(body: &Body) -> KernelResult<Vec<Body>> {
    let shells = body.shells();
    if shells.len() <= 1 {
        return Ok(vec![body.clone()]);
    }
    let all = body.faces();
    let mut out = Vec::with_capacity(shells.len());
    for s in &shells {
        let keep: BTreeSet<FaceId> = body.shell_faces(*s)?.into_iter().collect();
        let mut draft = Draft::from_body(body)?;
        for (i, f) in all.iter().enumerate() {
            if !keep.contains(f)
                && let Some(slot) = draft.faces.get_mut(i)
            {
                *slot = None;
            }
        }
        let uses = draft.edge_uses();
        for (e, slot) in draft.edges.iter_mut().enumerate() {
            if uses.get(e).is_none_or(Vec::is_empty) {
                *slot = None;
            }
        }
        let lump = draft.build()?;
        if lump.volume()? <= 0.0 {
            return Err(KernelError::Unsupported(
                "a body with a void cannot be split into lumps yet".into(),
            ));
        }
        out.push(lump);
    }
    Ok(out)
}

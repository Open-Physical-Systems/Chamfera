//! Booleans on solids (rung K7): [`union`], [`subtract`] and [`intersect`].
//!
//! # Pipeline
//! The classic boundary-evaluation scheme (Requicha and Voelcker, "Boolean operations in
//! solid modeling: boundary evaluation and merging algorithms", 1985; Mantyla, *An
//! Introduction to Solid Modeling*, chapter 15; Hoffmann, *Geometric and Solid Modeling*,
//! chapter 3), on exact geometry:
//!
//! 1. **Load.** Both operands (each must pass [`Body::validate`]) go into one registry of
//!    vertices and edges (`registry.rs`): points within [`LINEAR_TOLERANCE`] are one
//!    vertex, and edge pieces with the same end vertices and the same points are one
//!    edge, whichever operand or intersection made them.
//! 2. **Intersect faces.** Every pair of faces (one from each operand) whose boxes meet is
//!    intersected with K6 [`intersect_surfaces`]. Each intersection curve is split where it
//!    meets either face's boundary (K6 curve-surface intersection against a surface that
//!    meets the face exactly along the boundary edge, `span.rs`), and the intervals whose
//!    midpoint lies on both faces (the K3 inside-face test) become cuts. Coincident faces
//!    (the same plane or surface) exchange their edges instead: each face's edges, trimmed
//!    to the other face, are cuts of the other (an imprint).
//! 3. **Split.** Every operand edge and every cut is split at each vertex lying on it, so
//!    both faces at an edge see the same pieces. Each face's graph (its boundary pieces in
//!    the face's direction, its cut pieces both ways) is traced into loops, the loops
//!    sorted into outer loops and holes, and so into pieces (`trace.rs`).
//! 4. **Classify.** Each piece is classified against the other solid at an interior
//!    sample point (K6 [`classify_point`]): inside, outside, or on the boundary with the
//!    same or the opposite outward normal (a coincident face).
//! 5. **Select.** Union keeps A's pieces outside B or on it with the same normal and B's
//!    pieces outside A; intersect keeps the pieces inside the other solid and A's pieces on
//!    it with the same normal; subtract keeps A's pieces outside B or on it with the
//!    opposite normal, and B's pieces inside A, reversed. So a face shared with the same
//!    normal is kept once, and faces in contact with opposite normals (the internal faces
//!    of a union) vanish.
//! 6. **Sew** (`sew.rs`). Every edge must now have exactly two uses in opposite
//!    directions; where more faces meet (solids touching along an edge) they are paired
//!    by the material wedge they bound and the edge is split. Neighbouring pieces on one
//!    plane with the same normal, and pieces of one curved face, merge back into one face
//!    (their common edges removed, the rest traced again); edges on one carrier that meet
//!    at a vertex no other edge reaches are joined; each fan of faces around a vertex gets
//!    its own vertex. Shells are the connected components (`Body::from_face_list`), and
//!    the body must pass [`Body::validate`]; anything else is an error, never a body.
//!
//! # Coplanar and touching cases
//! - Face on face (a boss standing on a plate): the coincident faces imprint each other,
//!   the contact patch of both classifies as on-the-boundary with opposite normals and is
//!   dropped by a union, so the result is one shell with no internal face; the plate's top
//!   keeps a ring around the boss.
//! - A cut flush with a face (a pocket whose top is the plate's top, a through cut whose
//!   bottom is the plate's bottom): the shared patch has the same normal on both, so a
//!   subtraction drops it and leaves an opening.
//! - Coaxial cylinders: the same cylinder is coincident and imprints; different radii
//!   never meet; the caps are coincident planes.
//! - An edge or a vertex touching a face, or solids touching along an edge or at a
//!   point: the touching intersection is a dangling cut, which is pruned; the results
//!   share no topology there (a union gives one lump per solid, touching but separate).
//!
//! # Lumps and voids
//! A disjoint union has one shell per lump. A subtraction that hollows a solid (B strictly
//! inside A) gives a void: a second shell whose faces point into the hole (negative
//! volume). [`Body::validate`] accepts a void inside a lump.
//!
//! # Names (persistent naming, the start of K8)
//! Every face of the result keeps the name of the operand face it is a piece of, so a
//! face `top` of the plate is still `top` after a boss is joined onto it, and a reference
//! to `top` still resolves. The engine names operand faces `feature:role` (e.g. `A:top`,
//! `B:side 2`), so the result tells which feature each face came from; booleans nest
//! without renaming. When one operand face ends up as several result faces (a slot cuts a
//! top in two), they all carry its name; `chamfera_doc::naming` tells the pieces apart
//! (`A:top#1`, `A:top#2`) and resolves references to them. Faces merged from several
//! pieces take the name of the first piece, operand A before B. Edges keep the names of
//! the operand edges they are pieces of; an edge made by an intersection is named
//! `<face of A> x <face of B>` (an imprinted edge keeps its operand edge's name);
//! vertices keep operand vertex names.
//!
//! # What is refused
//! - Operands that are not valid solids ([`KernelError::InvalidInput`]).
//! - Face pairs whose surfaces meet in a curve K6 cannot represent yet (skew or unequal
//!   crossing cylinders, parabolic or hyperbolic cone sections, general torus sections,
//!   other non-coaxial curved pairs): [`KernelError::Unsupported`], naming both faces.
//! - Configurations whose kept faces cannot be paired into manifold edges or whose pieces
//!   do not trace consistently ([`KernelError::InvalidResult`]), and pieces that no
//!   interior point classifies ([`KernelError::NoConvergence`]).
//! - An empty result is [`KernelError::EmptyResult`].
//!
//! # Work and tolerance
//! Work is bounded by the input size: at most [`MAX_BOOLEAN_FACES`] faces per operand,
//! [`MAX_BOOLEAN_ENTITIES`] new vertices, edges and cuts, and every loop is bounded by the
//! number of entities. Every distance decision uses [`LINEAR_TOLERANCE`] and every angle
//! decision [`chamfera_geom::ANGULAR_TOLERANCE`]; topology is decided on exact surfaces
//! and curves, never on a mesh.

mod patch_box;
mod registry;
mod sew;
mod span;
mod split;
mod trace;

#[cfg(test)]
mod tests;

use crate::{Body, KernelError, KernelResult};
use registry::Registry;
use sew::{OutFace, classify, select};
use split::{intersect_faces, load, split_edges, split_face};

#[cfg(doc)]
use crate::{classify_point, intersect_surfaces};
#[cfg(doc)]
use chamfera_geom::LINEAR_TOLERANCE;

/// Most faces an operand of a boolean may have (hostile-input limit).
pub const MAX_BOOLEAN_FACES: usize = 20_000;
/// Most vertices, edges and intersection pieces one boolean may create.
pub const MAX_BOOLEAN_ENTITIES: usize = 1_000_000;

/// Which boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// Material in either solid.
    Union,
    /// Material in the first solid and not in the second.
    Subtract,
    /// Material in both solids.
    Intersect,
}

/// The two operands, `A` (first) and `B` (second).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Operand {
    A,
    B,
}

/// `a ∪ b`: everything in either solid (see the module documentation). A result of
/// separate lumps has one shell per lump.
pub fn union(a: &Body, b: &Body) -> KernelResult<Body> {
    boolean(a, b, BooleanOp::Union)
}

/// `a − b`: what of `a` lies outside `b`. Fails with [`KernelError::EmptyResult`] when
/// nothing is left.
pub fn subtract(a: &Body, b: &Body) -> KernelResult<Body> {
    boolean(a, b, BooleanOp::Subtract)
}

/// `a ∩ b`: what lies in both solids. Fails with [`KernelError::EmptyResult`] when they
/// share no volume.
pub fn intersect(a: &Body, b: &Body) -> KernelResult<Body> {
    boolean(a, b, BooleanOp::Intersect)
}

/// The boolean `op` of `a` and `b`: a new valid body, or an error and no body. Neither
/// input is changed.
pub fn boolean(a: &Body, b: &Body, op: BooleanOp) -> KernelResult<Body> {
    for (body, which) in [(a, "first"), (b, "second")] {
        body.validate().map_err(|e| {
            KernelError::InvalidInput(format!("the {which} solid is not a valid solid: {e}"))
        })?;
        if body.faces().is_empty() {
            return Err(KernelError::InvalidInput(format!(
                "the {which} solid is empty"
            )));
        }
        if body.faces().len() > MAX_BOOLEAN_FACES {
            return Err(KernelError::InvalidInput(format!(
                "the {which} solid has more than {MAX_BOOLEAN_FACES} faces"
            )));
        }
        if body.has_spline_geometry() {
            return Err(KernelError::Unsupported(format!(
                "the {which} solid has spline faces; booleans on them need NURBS surface intersection (K10)"
            )));
        }
    }
    let mut reg = Registry::default();
    let mut la = load(a, Operand::A, &mut reg)?;
    let mut lb = load(b, Operand::B, &mut reg)?;
    let mut cuts = intersect_faces(&la, &lb, &mut reg)?;
    split_edges([&mut la, &mut lb], &mut cuts, &mut reg)?;

    let mut partners_a: Vec<Vec<usize>> = vec![Vec::new(); la.faces.len()];
    let mut partners_b: Vec<Vec<usize>> = vec![Vec::new(); lb.faces.len()];
    for &(ia, ib) in &cuts.coincident {
        if let Some(p) = partners_a.get_mut(ia) {
            p.push(ib);
        }
        if let Some(p) = partners_b.get_mut(ib) {
            p.push(ia);
        }
    }
    let mut cuts_of: std::collections::HashMap<(Operand, usize), Vec<&split::Cut>> =
        std::collections::HashMap::new();
    for c in &cuts.cuts {
        for f in &c.faces {
            cuts_of.entry(*f).or_default().push(c);
        }
    }

    let mut kept = Vec::new();
    for (own, other, partners) in [(&la, &lb, &partners_a), (&lb, &la, &partners_b)] {
        for f in 0..own.faces.len() {
            let fc = cuts_of
                .get(&(own.which, f))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let none = Vec::new();
            let p = partners.get(f).unwrap_or(&none);
            let face = own.face(f)?;
            for piece in split_face(own, f, fc, &reg)? {
                let class = classify(&piece, own, other, p)?;
                if let Some(reverse) = select(op, own.which, class) {
                    kept.push(OutFace::new(
                        piece,
                        face.side.clone(),
                        face.name.clone(),
                        reverse,
                    ));
                }
            }
        }
    }
    if kept.is_empty() {
        return Err(KernelError::EmptyResult);
    }
    sew::sew(kept, reg)
}

/// The surface body `a` trimmed by the closed solid `tool`: `a`'s faces are split along
/// their intersection with the tool's (steps 1 to 3 of the pipeline), each piece is
/// classified against the tool (step 4), the pieces inside (`keep_inside`) or outside are
/// kept, and they are sewn into open shells (step 6 without the pairing of edges where
/// more faces meet). Pieces on the tool's boundary go.
pub(crate) fn trim(a: &Body, tool: &Body, keep_inside: bool) -> KernelResult<Body> {
    tool.validate().map_err(|e| {
        KernelError::InvalidInput(format!("the trimming tool is not a closed solid: {e}"))
    })?;
    for (body, which) in [(a, "surface"), (tool, "trimming tool")] {
        if body.faces().is_empty() {
            return Err(KernelError::InvalidInput(format!("the {which} is empty")));
        }
        if body.faces().len() > MAX_BOOLEAN_FACES {
            return Err(KernelError::InvalidInput(format!(
                "the {which} has more than {MAX_BOOLEAN_FACES} faces"
            )));
        }
        if body.has_spline_geometry() {
            return Err(KernelError::Unsupported(format!(
                "the {which} has spline faces; trimming them needs NURBS surface intersection (K10)"
            )));
        }
    }
    let mut reg = Registry::default();
    let mut la = load(a, Operand::A, &mut reg)?;
    let mut lb = load(tool, Operand::B, &mut reg)?;
    let mut cuts = intersect_faces(&la, &lb, &mut reg)?;
    split_edges([&mut la, &mut lb], &mut cuts, &mut reg)?;
    let mut cuts_of: std::collections::HashMap<usize, Vec<&split::Cut>> =
        std::collections::HashMap::new();
    for c in &cuts.cuts {
        for f in &c.faces {
            if f.0 == Operand::A {
                cuts_of.entry(f.1).or_default().push(c);
            }
        }
    }
    let mut kept = Vec::new();
    for f in 0..la.faces.len() {
        let fc = cuts_of.get(&f).map(Vec::as_slice).unwrap_or(&[]);
        let face = la.face(f)?;
        for piece in split_face(&la, f, fc, &reg)? {
            let class = classify(&piece, &la, &lb, &[])?;
            let keep = match class {
                sew::Class::In => keep_inside,
                sew::Class::Out => !keep_inside,
                sew::Class::OnSame | sew::Class::OnOpposite => false,
            };
            if keep {
                kept.push(OutFace::new(
                    piece,
                    face.side.clone(),
                    face.name.clone(),
                    false,
                ));
            }
        }
    }
    if kept.is_empty() {
        return Err(KernelError::EmptyResult);
    }
    sew::sew_open(kept, reg)
}

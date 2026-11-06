//! The engine side of persistent naming (rung K8): every consumer of a stored face, edge
//! or vertex reference finds it through [`resolve`], with the signature its owner keeps
//! in the design (`Document::references`). The rule itself lives in
//! [`chamfera_doc::naming`]; this module adds the bodies built so far and the commands'
//! side (recording a signature when a reference is made).

pub use chamfera_doc::naming::{Found, Kind, NameError, Signature, Want, resolve_one};
use chamfera_doc::{Document, FaceRef};
use chamfera_geom::Point3;
use chamfera_kernel::{Body, Entity};

/// The bodies of a rebuild's `(feature, name, body)` list.
pub(crate) fn bodies(made: &[(u32, String, Body)]) -> Vec<&Body> {
    made.iter().map(|m| &m.2).collect()
}

/// The full name of a face reference: `2:top`, or `2:top#2` for one piece.
pub fn face_ref_name(r: &FaceRef) -> String {
    crate::solids::face_key(r.feature, &r.face)
}

/// Resolves `reference`, held by timeline item `owner`, among `bodies` (see
/// [`chamfera_doc::naming::resolve`]). `Found::body` indexes `bodies`.
pub(crate) fn resolve(
    doc: &Document,
    owner: u32,
    bodies: &[&Body],
    kind: Kind,
    reference: &str,
    want: Want,
) -> Result<Vec<Found>, NameError> {
    chamfera_doc::naming::resolve(
        bodies,
        kind,
        reference,
        doc.signature(owner, reference),
        want,
    )
}

/// [`resolve`] for a face reference.
pub(crate) fn resolve_face(
    doc: &Document,
    owner: u32,
    bodies: &[&Body],
    r: &FaceRef,
    want: Want,
) -> Result<Vec<Found>, NameError> {
    resolve(doc, owner, bodies, Kind::Face, &face_ref_name(r), want)
}

/// The first face [`resolve_face`] finds, with its body.
pub(crate) fn one_face<'a>(
    doc: &Document,
    owner: u32,
    bodies: &[&'a Body],
    r: &FaceRef,
    want: Want,
) -> Result<(usize, &'a Body, chamfera_kernel::FaceId), NameError> {
    let found = resolve_face(doc, owner, bodies, r, want)?;
    let gone = || NameError::Gone {
        kind: "face",
        reference: face_ref_name(r),
    };
    let f = found.first().ok_or_else(gone)?;
    let body = bodies.get(f.body).ok_or_else(gone)?;
    Ok((f.body, body, f.face().ok_or_else(gone)?))
}

impl crate::Session {
    fn built_bodies(&self) -> Vec<&Body> {
        self.solids().iter().map(|s| &s.body).collect()
    }

    /// Resolves a reference among the solids built last, with the signature `owner`
    /// keeps for it (none when `owner` is `None`). `Found::body` indexes
    /// [`crate::Session::solids`].
    pub fn resolve_reference(
        &self,
        owner: Option<u32>,
        kind: Kind,
        reference: &str,
        want: Want,
    ) -> Result<Vec<Found>, NameError> {
        let bodies = self.built_bodies();
        let sig = owner.and_then(|o| self.doc.signature(o, reference));
        chamfera_doc::naming::resolve(&bodies, kind, reference, sig, want)
    }

    /// [`Self::resolve_reference`] with a point instead of a stored signature: the piece
    /// of a split face or edge that `point` lies on (or is clearly nearest).
    pub fn resolve_at_point(
        &self,
        kind: Kind,
        reference: &str,
        point: Option<Point3>,
        want: Want,
    ) -> Result<Vec<Found>, NameError> {
        let bodies = self.built_bodies();
        let sig = point.filter(|p| p.is_valid()).map(|p| Signature {
            point: Some([p.x, p.y, p.z]),
            ..Signature::default()
        });
        chamfera_doc::naming::resolve(&bodies, kind, reference, sig.as_ref(), want)
    }

    /// The unique name of an entity of solid `solid` (`2:top`, or `2:top#2` when the
    /// name is shared by several pieces).
    pub fn reference_name(&self, solid: usize, entity: Entity) -> Option<String> {
        let bodies = self.built_bodies();
        chamfera_doc::naming::reference_name(
            &bodies,
            Found {
                body: solid,
                entity,
            },
        )
    }

    /// Every named entity of `kind` in the solids built last, with its unique name.
    pub fn reference_names(&self, kind: Kind) -> Vec<(Found, String)> {
        chamfera_doc::naming::reference_names(&self.built_bodies(), kind)
    }

    /// Records what `reference` of item `owner` names now, with `point` (where it was
    /// picked) when it lies on it. A reference that does not resolve to one entity (a
    /// whole name of a split face, or a name not built yet) keeps only the point.
    pub(crate) fn record_signature(
        &mut self,
        owner: u32,
        kind: Kind,
        reference: &str,
        point: Option<Point3>,
    ) {
        let bodies = self.built_bodies();
        let one = chamfera_doc::naming::resolve(&bodies, kind, reference, None, Want::All)
            .ok()
            .filter(|f| f.len() == 1)
            .and_then(|f| f.first().copied());
        let sig = match one {
            Some(f) => chamfera_doc::naming::signature_of(&bodies, f, point),
            None => point.filter(|p| p.is_valid()).map(|p| Signature {
                point: Some([p.x, p.y, p.z]),
                ..Signature::default()
            }),
        };
        if let Some(sig) = sig {
            // Over the limits the reference still works by name alone.
            let _ = self.doc.set_signature(owner, reference, sig);
        }
    }

    /// [`Self::record_signature`] for a face reference.
    pub(crate) fn record_face_signature(&mut self, owner: u32, r: &FaceRef, point: Option<Point3>) {
        self.record_signature(owner, Kind::Face, &face_ref_name(r), point);
    }
}

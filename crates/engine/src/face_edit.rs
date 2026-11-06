//! Direct face edits on the timeline (offset, delete and move faces) and Press Pull, the
//! one tool that picks the right operation for what is selected.
//!
//! An edit finds its faces by name in the bodies built so far, edits every body that has
//! any of them (kernel `face_edit`), and keeps the names of everything it does not delete,
//! so later features still find their faces. Every name must still be found.

use crate::solids::{name_edges, name_faces, split_face_key};
use chamfera_doc::expr::Dim;
use chamfera_doc::{BlendKind, DeleteFace, Document, FeatureKind, MoveFace, OffsetFace};
use chamfera_geom::{Transform, Vec3};
use chamfera_kernel::{Body, FaceId};

/// The bodies after an edit, and the names of those it changed.
type Made = (Vec<(u32, String, Body)>, Vec<String>);

/// Runs `edit` on every body holding one of `names`, refusing a name no body has.
fn edit_named(
    made: &[(u32, String, Body)],
    names: &[String],
    feature: u32,
    word: &str,
    edit: impl Fn(&Body, &[FaceId]) -> Result<Body, chamfera_kernel::KernelError>,
) -> Result<Made, String> {
    let mut found = vec![false; names.len()];
    let mut next = made.to_vec();
    let mut changed = Vec::new();
    for (_, name, body) in &mut next {
        let mut faces = Vec::new();
        for f in body.faces() {
            if let Some(n) = body.face_name(f).ok().flatten()
                && let Some(i) = names.iter().position(|x| x == n)
            {
                faces.push(f);
                if let Some(seen) = found.get_mut(i) {
                    *seen = true;
                }
            }
        }
        if faces.is_empty() {
            continue;
        }
        let mut out = edit(body, &faces).map_err(|x| format!("{word}: {x}"))?;
        name_faces(&mut out, feature);
        name_edges(&mut out, feature);
        *body = out;
        changed.push(name.clone());
    }
    if let Some(i) = found.iter().position(|f| !f) {
        let name = names.get(i).map_or("", String::as_str);
        return Err(format!("{word}: the face {name:?} is no longer there"));
    }
    Ok((next, changed))
}

pub(crate) fn build_offset_face(
    doc: &Document,
    o: &OffsetFace,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<Made, String> {
    let d = doc
        .evaluate_feature_value(&o.distance, Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    edit_named(made, &o.faces, feature, "offset face", |b, f| {
        chamfera_kernel::offset_faces(b, f, d)
    })
}

pub(crate) fn build_delete_face(
    d: &DeleteFace,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<Made, String> {
    edit_named(
        made,
        &d.faces,
        feature,
        "delete face",
        chamfera_kernel::delete_faces,
    )
}

/// The rigid motion of a Move Face feature: the translation, then the turn.
pub fn move_face_transform(doc: &Document, m: &MoveFace) -> Result<Transform, String> {
    let d = doc
        .evaluate_feature_value(&m.distance, Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    let shift = if d.abs() > chamfera_geom::LINEAR_TOLERANCE {
        m.direction
            .normalized()
            .ok_or("move face: the direction has no length")?
            * d
    } else {
        Vec3::new(0.0, 0.0, 0.0)
    };
    let mut t = Transform::translation(shift).ok_or("move face: the distance is too large")?;
    if let Some(turn) = &m.turn {
        let a = doc
            .evaluate_feature_value(&turn.angle, Dim::ANGLE)
            .map_err(|x| x.to_string())?;
        let r = Transform::rotation_about(turn.origin, turn.axis, a)
            .ok_or("move face: the turn needs an axis direction")?;
        t = t.then(&r);
    }
    Ok(t)
}

pub(crate) fn build_move_face(
    doc: &Document,
    m: &MoveFace,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<Made, String> {
    let t = move_face_transform(doc, m)?;
    edit_named(made, &m.faces, feature, "move face", |b, f| {
        chamfera_kernel::move_faces(b, f, &t)
    })
}

/// What Press Pull does with a selection.
#[derive(Debug, Clone, PartialEq)]
pub enum PressPull {
    /// A sketch profile: extrude it.
    Extrude,
    /// A face: offset it (planar faces, and curved faces that are not fillets).
    OffsetFace { face: String },
    /// A face that a fillet feature made: change that fillet's radius.
    FilletRadius {
        face: String,
        feature: u32,
        radius: String,
    },
    /// An edge: fillet it.
    Fillet { edge: String },
}

impl PressPull {
    /// The word for the operation, for the dialog title and the reply.
    pub fn label(&self) -> &'static str {
        match self {
            PressPull::Extrude => "Extrude",
            PressPull::OffsetFace { .. } => "Offset Face",
            PressPull::FilletRadius { .. } => "Fillet radius",
            PressPull::Fillet { .. } => "Fillet",
        }
    }
}

/// The fillet feature that made face `name` (`5:fillet of 2:sweep 0`), with its radius
/// expression, if it is one.
pub fn fillet_of_face(doc: &Document, name: &str) -> Option<(u32, String)> {
    let (id, role) = split_face_key(name)?;
    if !role.starts_with("fillet of ") && !role.starts_with("fillet corner at ") {
        return None;
    }
    match &doc.feature(id).ok()?.kind {
        FeatureKind::Blend(b) if b.blend == BlendKind::Fillet => Some((id, b.size.clone())),
        _ => None,
    }
}

/// Press Pull on a face: the fillet's radius if a fillet made it, else an offset.
pub fn press_pull_face(doc: &Document, face: &str) -> PressPull {
    match fillet_of_face(doc, face) {
        Some((feature, radius)) => PressPull::FilletRadius {
            face: face.to_owned(),
            feature,
            radius,
        },
        None => PressPull::OffsetFace {
            face: face.to_owned(),
        },
    }
}

/// What Press Pull would work on under a pixel, for the tool's drag arrow: the
/// `solid.press_pull` parameters naming it (without `distance`), the operation, where the
/// arrow starts and which way a positive distance goes.
#[derive(Debug, Clone, PartialEq)]
pub struct PressPullTarget {
    pub params: serde_json::Value,
    pub operation: PressPull,
    pub anchor: chamfera_geom::Point3,
    pub direction: Vec3,
}

/// The outward normal of `face` at the point of its surface nearest `p`.
fn outward_normal(body: &Body, face: FaceId, p: chamfera_geom::Point3) -> Option<Vec3> {
    let (surface, same_sense) = body.face_surface(face).ok()??;
    let q = surface.parameters(p);
    let n = surface.normal_at(q.u, q.v);
    Some(if same_sense { n } else { -n })
}

impl crate::Session {
    /// The edge, face or sketch profile under pixel `(x, y)` that Press Pull would use:
    /// an edge near the pointer first, then a profile in front of (or on) the face under
    /// it, then that face.
    pub fn press_pull_target(&self, x: f64, y: f64, w: f64, h: f64) -> Option<PressPullTarget> {
        use serde_json::json;
        if let Some((si, e, name)) = self.edge_at(x, y, w, h) {
            let solid = self.solids().get(si)?;
            let body = &solid.body;
            let (c, t0, t1) = body.edge_curve(e).ok()??;
            let mid = c.point_at(0.5 * (t0 + t1));
            let [a, b] = body.edge_faces(e).ok()?;
            let n = outward_normal(body, a, mid)? + outward_normal(body, b, mid)?;
            return Some(PressPullTarget {
                params: json!({ "edge": name }),
                operation: PressPull::Fillet { edge: name },
                anchor: mid,
                direction: n.normalized()?,
            });
        }
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.scene.pick(&ray);
        // A profile wins over a face it lies on or in front of (a sketch on a face).
        let profile = self.profile_at(x, y, w, h).and_then(|(sketch, at, _)| {
            let plane = self.doc.sketch(sketch).ok()?.plane;
            let p = plane.from_plane(at);
            let t = (p - ray.origin).dot(ray.direction);
            let in_front = hit
                .as_ref()
                .is_none_or(|h| t <= h.distance + 1e-6 * h.distance.abs().max(1.0));
            in_front.then_some((sketch, at, plane))
        });
        if let Some((sketch, at, plane)) = profile {
            return Some(PressPullTarget {
                params: json!({ "sketch": sketch, "profile": { "x": at.x, "y": at.y } }),
                operation: PressPull::Extrude,
                anchor: plane.from_plane(at),
                direction: plane.z(),
            });
        }
        if let Some(hit) = hit {
            let solid = self
                .solids()
                .iter()
                .find(|s| s.scene_id == Some(hit.body))?;
            let face = *solid.faces.get(hit.triangle)?;
            let name = solid.body.face_name(face).ok().flatten()?.to_owned();
            return Some(PressPullTarget {
                params: json!({ "face": name }),
                operation: press_pull_face(&self.doc, &name),
                anchor: hit.point,
                direction: outward_normal(&solid.body, face, hit.point)?,
            });
        }
        None
    }
}

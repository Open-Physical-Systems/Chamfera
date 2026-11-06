//! Direct face edits (`solid.offset_face`, `solid.delete_face`, `solid.move_face`) and
//! `solid.press_pull`, which picks the operation from what it is given: a sketch profile
//! is extruded, a face offset (or, for a fillet's face, that fillet's radius changed), an
//! edge filleted.

use super::CommandSpec;
use super::feature::feature_reply;
use crate::face_edit::{PressPull, press_pull_face};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{Blend, BlendKind, DeleteFace, FaceTurn, FeatureKind, MoveFace, OffsetFace};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "solid.delete_face",
        label: "Delete Face",
        params: "faces: list of strings (optional) - face names like \"5:fillet of 2:sweep 0\"\n\
                 pick: {x, y, width, height} (optional) - the face under this pixel, instead",
        mutates: true,
        run: delete_face,
    },
    CommandSpec {
        id: "solid.move_face",
        label: "Move Face",
        params: "faces: list of strings (optional) - planar face names like \"2:top\"\n\
                 pick: {x, y, width, height} (optional) - the face under this pixel, instead\n\
                 direction: {x, y, z} (optional) - which way to move, default the face's normal\n\
                 distance: string or number (optional) - how far, default 0\n\
                 origin, axis: {x, y, z} (optional) - a point and direction of the axis to turn about\n\
                 angle: string or number (optional) - the turn; a plain number is degrees",
        mutates: true,
        run: move_face,
    },
    CommandSpec {
        id: "solid.offset_face",
        label: "Offset Face",
        params: "faces: list of strings (optional) - face names like \"2:top\"\n\
                 pick: {x, y, width, height} (optional) - the face under this pixel, instead\n\
                 distance: string or number (required) - along the outward normal; positive adds material",
        mutates: true,
        run: offset_face,
    },
    CommandSpec {
        id: "solid.press_pull",
        label: "Press Pull",
        params: "face: string (optional) - a face: offset it, or change its fillet's radius\n\
                 edge: string (optional) - an edge: fillet it\n\
                 sketch: integer and profile: {x, y} (optional) - a sketch profile: extrude it\n\
                 pick: {x, y, width, height} (optional) - whatever is under this pixel: an edge, else a profile on or in front of the face, else the face\n\
                 distance: string or number (required) - offset or extrude distance; the radius for an edge or a fillet face\n\
                 operation: string (optional) - for a profile: new_body (default), join, cut or intersect\n\
                 preview: boolean (optional) - show the result translucent and change nothing\n\
                 clear: boolean (optional) - true removes the preview instead",
        mutates: true,
        run: press_pull,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// A length or angle given as a number (in `unit`) or an expression.
fn expression(p: &Value, key: &str, unit: &str) -> Result<Option<String>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => Ok(Some(format!("{} {unit}", params::req_f64(p, key)?))),
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

fn point(p: &Value, key: &str) -> Result<Option<chamfera_geom::Vec3>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(chamfera_geom::Vec3::new(
            params::req_f64(v, "x")?,
            params::req_f64(v, "y")?,
            params::req_f64(v, "z")?,
        ))),
    }
}

/// The faces a command edits: a non-empty list, or the one under `pick`.
fn faces(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    let list = match (p.get("faces"), p.get("pick")) {
        (Some(Value::Array(list)), None | Some(Value::Null)) => list
            .iter()
            .map(|f| {
                f.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad("`faces` must be a list of face names"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y, w, h) = params::pixel(pick)?;
            vec![s.face_ref_at(x, y, w, h).ok_or_else(|| {
                CommandError::Unavailable("there is no face of a solid under that pixel".into())
            })?]
        }
        (None | Some(Value::Null), None) => return Err(bad("give `faces` or `pick`")),
        (Some(_), None | Some(Value::Null)) => {
            return Err(bad("`faces` must be a list of face names"));
        }
        _ => return Err(bad("give either `faces` (a list) or `pick`, not both")),
    };
    if list.is_empty() {
        return Err(bad("`faces` must name at least one face"));
    }
    Ok(list)
}

/// Adds `kind` to the timeline and rebuilds; the feature's error is the command's.
fn add(s: &mut Session, kind: FeatureKind) -> CmdResult {
    s.scene.previews.clear();
    let named: Vec<String> = match &kind {
        FeatureKind::OffsetFace(o) => o.faces.clone(),
        FeatureKind::DeleteFace(d) => d.faces.clone(),
        FeatureKind::MoveFace(m) => m.faces.clone(),
        _ => Vec::new(),
    };
    let id = s.doc.add_feature(kind)?;
    for n in &named {
        s.record_signature(id, crate::naming::Kind::Face, n, None);
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn offset_face(s: &mut Session, p: &Value) -> CmdResult {
    let faces = faces(s, p)?;
    let distance = expression(p, "distance", "mm")?.ok_or_else(|| bad("`distance` is required"))?;
    add(s, FeatureKind::OffsetFace(OffsetFace { faces, distance }))
}

fn delete_face(s: &mut Session, p: &Value) -> CmdResult {
    let faces = faces(s, p)?;
    add(s, FeatureKind::DeleteFace(DeleteFace { faces }))
}

/// The outward normal of the named flat face among the built solids (through the
/// resolver: pieces of a split face share their plane).
fn face_normal(s: &Session, name: &str) -> Option<chamfera_geom::Vec3> {
    let found = s
        .resolve_reference(
            None,
            crate::naming::Kind::Face,
            name,
            crate::naming::Want::All,
        )
        .ok()?;
    let f = found.first()?;
    crate::solids::face_plane(&s.solids().get(f.body)?.body, f.face()?).map(|p| p.z())
}

fn move_face(s: &mut Session, p: &Value) -> CmdResult {
    let faces = faces(s, p)?;
    let distance = expression(p, "distance", "mm")?.unwrap_or_else(|| "0 mm".into());
    let direction = match point(p, "direction")? {
        Some(d) => d,
        None => faces
            .first()
            .and_then(|f| face_normal(s, f))
            .unwrap_or(chamfera_geom::Vec3::new(0.0, 0.0, 1.0)),
    };
    let turn = match expression(p, "angle", "deg")? {
        None => None,
        Some(angle) => {
            let o = point(p, "origin")?.ok_or_else(|| bad("a turn needs `origin`"))?;
            let axis = point(p, "axis")?.ok_or_else(|| bad("a turn needs `axis`"))?;
            Some(FaceTurn {
                origin: chamfera_geom::Point3::new(o.x, o.y, o.z),
                axis,
                angle,
            })
        }
    };
    add(
        s,
        FeatureKind::MoveFace(MoveFace {
            faces,
            direction,
            distance,
            turn,
        }),
    )
}

/// True for the feature kinds this file edits through `feature.edit`.
pub(super) fn is_face_edit(kind: &FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::OffsetFace(_) | FeatureKind::DeleteFace(_) | FeatureKind::MoveFace(_)
    )
}

/// `feature.edit` of a face edit: `faces`, `distance`, `angle`, `suppressed`.
pub(super) fn edit(s: &mut Session, id: u32, p: &Value) -> CmdResult {
    let allowed = ["feature", "faces", "distance", "angle", "suppressed"];
    if let Some(obj) = p.as_object()
        && let Some(k) = obj.keys().find(|k| !allowed.contains(&k.as_str()))
    {
        return Err(bad(format!(
            "a face edit takes faces, distance, angle and suppressed, not `{k}`"
        )));
    }
    let mut kind = s.doc.feature(id)?.kind.clone();
    let new_faces = match p.get("faces") {
        None | Some(Value::Null) => None,
        Some(_) => Some(faces(s, &json!({ "faces": p.get("faces") }))?),
    };
    let distance = expression(p, "distance", "mm")?;
    let angle = expression(p, "angle", "deg")?;
    match &mut kind {
        FeatureKind::OffsetFace(o) => {
            if angle.is_some() {
                return Err(bad("an offset face has no angle"));
            }
            if let Some(d) = distance {
                o.distance = d;
            }
            if let Some(f) = new_faces {
                o.faces = f;
            }
        }
        FeatureKind::DeleteFace(d) => {
            if distance.is_some() || angle.is_some() {
                return Err(bad("a delete face has no distance or angle"));
            }
            if let Some(f) = new_faces {
                d.faces = f;
            }
        }
        FeatureKind::MoveFace(m) => {
            if let Some(d) = distance {
                m.distance = d;
            }
            if let Some(a) = angle {
                let t = m
                    .turn
                    .as_mut()
                    .ok_or_else(|| bad("this move face has no turn to change"))?;
                t.angle = a;
            }
            if let Some(f) = new_faces {
                m.faces = f;
            }
        }
        _ => return Err(bad("not a face edit")),
    }
    let suppressed = match p.get("suppressed") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "suppressed")?),
    };
    s.doc.check_feature(&kind)?;
    let f = s.doc.feature_mut(id)?;
    f.kind = kind;
    if let Some(v) = suppressed {
        s.doc.set_suppressed(id, v)?;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

/// What Press Pull was given, resolved: the operation and the change to the design.
enum Change {
    Add(FeatureKind),
    Edit(u32, FeatureKind),
}

/// Works out what `solid.press_pull` does with its parameters.
fn press_pull_plan(s: &Session, p: &Value) -> Result<(PressPull, Change), CommandError> {
    let value = expression(p, "distance", "mm")?.ok_or_else(|| bad("`distance` is required"))?;
    let given = ["face", "edge", "sketch", "pick"]
        .iter()
        .filter(|k| !matches!(p.get(**k), None | Some(Value::Null)))
        .count();
    if given != 1 {
        return Err(bad(
            "give one of `face`, `edge`, `sketch` with `profile`, or `pick`",
        ));
    }
    let profile = |sketch: u32, x: f64, y: f64| -> Result<(PressPull, Change), CommandError> {
        let mut q = json!({
            "sketch": sketch,
            "profiles": [{ "x": x, "y": y }],
            "distance": value,
        });
        if let Some(op) = p.get("operation") {
            q["operation"] = op.clone();
        }
        let e = super::feature::new_extrude(s, &q)?;
        Ok((PressPull::Extrude, Change::Add(FeatureKind::Extrude(e))))
    };
    let target = if let Some(Value::String(face)) = p.get("face") {
        press_pull_face(&s.doc, face)
    } else if let Some(Value::String(edge)) = p.get("edge") {
        PressPull::Fillet { edge: edge.clone() }
    } else if !matches!(p.get("sketch"), None | Some(Value::Null)) {
        let sketch = params::req_id(p, "sketch")?;
        let at = p
            .get("profile")
            .ok_or_else(|| bad("`profile` {x, y} is required"))?;
        return profile(sketch, params::req_f64(at, "x")?, params::req_f64(at, "y")?);
    } else if let Some(pick) = p.get("pick") {
        let (x, y, w, h) = params::pixel(pick)?;
        let t = s.press_pull_target(x, y, w, h).ok_or_else(|| {
            CommandError::Unavailable("there is no profile, face or edge under that pixel".into())
        })?;
        if let Some(sketch) = t.params.get("sketch").and_then(Value::as_u64) {
            let at = &t.params["profile"];
            let sketch = u32::try_from(sketch).map_err(|_| bad("bad sketch id"))?;
            return profile(sketch, params::req_f64(at, "x")?, params::req_f64(at, "y")?);
        }
        t.operation
    } else {
        return Err(bad(
            "`face`, `edge` and `sketch` take a string, a string and an id",
        ));
    };
    let change = match &target {
        PressPull::OffsetFace { face } => Change::Add(FeatureKind::OffsetFace(OffsetFace {
            faces: vec![face.clone()],
            distance: value,
        })),
        PressPull::Fillet { edge } => Change::Add(FeatureKind::Blend(Blend {
            blend: BlendKind::Fillet,
            edges: vec![edge.clone()],
            size: value,
        })),
        PressPull::FilletRadius { feature, .. } => {
            let mut kind = s.doc.feature(*feature)?.kind.clone();
            if let FeatureKind::Blend(b) = &mut kind {
                b.size = value;
            }
            Change::Edit(*feature, kind)
        }
        PressPull::Extrude => return Err(bad("no profile given")),
    };
    Ok((target, change))
}

/// Applies a Press Pull change to `doc`, returning the feature it made or changed.
fn apply(doc: &mut chamfera_doc::Document, change: Change) -> Result<u32, CommandError> {
    Ok(match change {
        Change::Add(kind) => doc.add_feature(kind)?,
        Change::Edit(id, kind) => {
            doc.check_feature(&kind)?;
            doc.feature_mut(id)?.kind = kind;
            id
        }
    })
}

fn press_pull(s: &mut Session, p: &Value) -> CmdResult {
    if matches!(p.get("clear"), Some(Value::Bool(true))) {
        s.scene.previews.clear();
        return Ok(json!({ "cleared": true }));
    }
    let (target, change) = press_pull_plan(s, p)?;
    let face = match &target {
        PressPull::OffsetFace { face } | PressPull::FilletRadius { face, .. } => Some(face.clone()),
        _ => None,
    };
    if matches!(p.get("preview"), Some(Value::Bool(true))) {
        return press_pull_preview(s, target, change, face);
    }
    s.scene.previews.clear();
    let id = apply(&mut s.doc, change)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    let mut reply = feature_reply(s, id);
    reply["operation"] = json!(target.label());
    Ok(reply)
}

/// Builds the Press Pull result in a scratch session and shows the changed solids as
/// translucent previews; the design does not change.
fn press_pull_preview(
    s: &mut Session,
    target: PressPull,
    change: Change,
    face: Option<String>,
) -> CmdResult {
    s.scene.previews.clear();
    let mut scratch = Session {
        doc: s.doc.clone(),
        ..Session::default()
    };
    let id = apply(&mut scratch.doc, change)?;
    scratch.rebuild();
    if let Some(e) = scratch.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    let mut volume = 0.0;
    for x in scratch.solids() {
        let has_face = face.as_ref().is_some_and(|n| {
            x.body
                .faces()
                .into_iter()
                .any(|f| x.body.face_name(f).ok().flatten() == Some(n.as_str()))
        });
        if !(x.feature == id || crate::solids::body_has_feature(&x.body, id) || has_face) {
            continue;
        }
        volume += x.body.volume().unwrap_or(0.0);
        let mesh = chamfera_kernel::tessellate(&x.body, &chamfera_kernel::TessOptions::default())
            .map_err(|e| {
            CommandError::Unavailable(format!("the preview cannot be shown: {e}"))
        })?;
        s.scene.previews.push(mesh.mesh);
    }
    Ok(json!({
        "operation": target.label(),
        "volume": volume,
        "previews": s.scene.previews.len(),
    }))
}

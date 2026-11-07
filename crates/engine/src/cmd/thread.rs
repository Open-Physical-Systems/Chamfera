//! `solid.thread` and `solid.threads`: cosmetic threads on cylindrical faces.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::thread::Thread;
use chamfera_doc::{FaceRef, FeatureKind};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "solid.thread",
        label: "Thread",
        params: "face: string (optional) - the cylindrical face by name, such as \"3:hole 0 wall\"\n\
                 pick: {x, y, width, height} (optional) - the face under this pixel, instead\n\
                 size: string (optional) - the designation in callouts, such as \"M6\"; default M and the diameter\n\
                 pitch: string or number (required) - such as 1 or \"1 mm\"\n\
                 length: string or number (optional) - from the start; default the whole face (\"full\")\n\
                 flip: boolean (optional) - start at the face's lower end instead of its upper one",
        mutates: true,
        run: thread,
    },
    CommandSpec {
        id: "solid.threads",
        label: "List Threads",
        params: "(none) - every cosmetic thread: its feature, face, callout, start, direction, radius, pitch and length",
        mutates: false,
        run: threads,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// The face a thread goes on: `face` by name, or the solid face under `pick`.
fn face(s: &Session, p: &Value) -> Result<FaceRef, CommandError> {
    let name = match (p.get("face"), p.get("pick")) {
        (Some(Value::String(name)), None | Some(Value::Null)) => name.clone(),
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y, w, h) = params::pixel(pick)?;
            s.face_ref_at(x, y, w, h).ok_or_else(|| {
                CommandError::Unavailable("there is no face of a solid under that pixel".into())
            })?
        }
        (None | Some(Value::Null), None) => return Err(bad("give `face` or `pick`")),
        _ => return Err(bad("give either `face` (a name) or `pick`, not both")),
    };
    let (feature, role) = crate::solids::split_face_key(&name).ok_or_else(|| {
        bad("`face` must be a face name such as \"3:hole 0 wall\" (feature:role)")
    })?;
    Ok(FaceRef {
        feature,
        face: role.to_owned(),
    })
}

/// Applies the thread parameters given in `p` to `t` (for `solid.thread` and
/// `feature.edit`).
pub(super) fn apply(t: &mut Thread, p: &Value) -> Result<(), CommandError> {
    if let Some(size) = params::opt_str(p, "size")? {
        if !chamfera_doc::thread::is_valid_thread_size(size) {
            return Err(bad(
                "`size` holds at most 40 letters, digits, spaces and . , / - characters",
            ));
        }
        t.size = size.trim().to_owned();
    }
    if let Some(pitch) = super::feature::expression(p, "pitch", "mm")? {
        t.pitch = pitch;
    }
    match p.get("length") {
        None | Some(Value::Null) => {}
        Some(Value::String(l)) if l.trim() == "full" => t.length = None,
        Some(_) => t.length = super::feature::expression(p, "length", "mm")?,
    }
    match p.get("flip") {
        None | Some(Value::Null) => {}
        Some(_) => t.flip = params::req_bool(p, "flip")?,
    }
    Ok(())
}

/// The keys `feature.edit` takes for a thread.
const EDIT_KEYS: &[&str] = &[
    "feature",
    "face",
    "size",
    "pitch",
    "length",
    "flip",
    "suppressed",
];

/// `feature.edit` of thread `id`: new values, a new face, or suppression.
pub(super) fn edit(s: &mut Session, id: u32, p: &Value) -> CmdResult {
    if let Some(obj) = p.as_object()
        && let Some(k) = obj.keys().find(|k| !EDIT_KEYS.contains(&k.as_str()))
    {
        return Err(bad(format!(
            "a thread takes face, size, pitch, length, flip and suppressed, not `{k}`"
        )));
    }
    let FeatureKind::Thread(mut t) = s.doc.feature(id)?.kind.clone() else {
        return Err(bad("not a thread"));
    };
    if p.get("face").is_some_and(|v| !v.is_null()) {
        t.face = face(s, &json!({ "face": p.get("face") }))?;
        let on = t.face.clone();
        s.record_face_signature(id, &on, None);
    }
    apply(&mut t, p)?;
    let kind = FeatureKind::Thread(t);
    s.doc.check_feature(&kind)?;
    s.doc.feature_mut(id)?.kind = kind;
    match p.get("suppressed") {
        None | Some(Value::Null) => {}
        Some(_) => s
            .doc
            .set_suppressed(id, params::req_bool(p, "suppressed")?)?,
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(super::feature::feature_reply(s, id))
}

fn thread(s: &mut Session, p: &Value) -> CmdResult {
    let on = face(s, p)?;
    if super::feature::expression(p, "pitch", "mm")?.is_none() {
        return Err(bad("`pitch` is required"));
    }
    let mut t = Thread {
        face: on,
        size: String::new(),
        pitch: String::new(),
        length: None,
        flip: false,
    };
    apply(&mut t, p)?;
    let on = t.face.clone();
    let id = s.doc.add_feature(FeatureKind::Thread(t))?;
    s.record_face_signature(id, &on, None);
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    let mut r = super::feature::feature_reply(s, id);
    if let Some(t) = s.threads().get(&id) {
        r["callout"] = json!(t.callout);
        r["length"] = json!(t.length);
    }
    Ok(r)
}

fn threads(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let pt = |q: chamfera_geom::Point3| json!([q.x, q.y, q.z]);
    let list: Vec<Value> = s
        .doc
        .features
        .iter()
        .filter_map(|f| {
            let FeatureKind::Thread(t) = &f.kind else {
                return None;
            };
            let built = s.threads().get(&f.id);
            Some(json!({
                "feature": f.id,
                "name": f.name,
                "face": crate::naming::face_ref_name(&t.face),
                "suppressed": f.suppressed,
                "callout": built.map(|b| b.callout.clone()),
                "internal": built.map(|b| b.internal),
                "start": built.map(|b| pt(b.start)),
                "direction": built.map(|b| json!([b.direction.x, b.direction.y, b.direction.z])),
                "diameter": built.map(|b| 2.0 * b.radius),
                "pitch": built.map(|b| b.pitch),
                "length": built.map(|b| b.length),
                "error": s.feature_errors().get(&f.id),
            }))
        })
        .collect();
    Ok(json!({ "threads": list }))
}

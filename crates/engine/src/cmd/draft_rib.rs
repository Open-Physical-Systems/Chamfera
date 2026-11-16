//! `solid.draft` tilts planar faces about a neutral plane; `solid.rib` grows a rib or web
//! from an open sketch chain to a solid. Both add a timeline feature, and with `preview`
//! show the result translucent instead, changing nothing.

use super::CommandSpec;
use super::face_edit::{expression, faces};
use super::feature::feature_reply;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{Draft, FeatureKind, Rib};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "solid.draft",
        label: "Draft",
        params: "faces: list of strings (optional) - planar face names like \"2:side 0\"\n\
                 pick: {x, y, width, height} (optional) - the face under this pixel, instead\n\
                 neutral: string (required) - the neutral plane: a planar face like \"2:bottom\" (pull into the solid) or xy, yz, xz (pull along its normal)\n\
                 angle: string or number (required) - the draft from the pull direction; a plain number is degrees; positive narrows along the pull\n\
                 flip: boolean (optional) - pull the other way\n\
                 preview: boolean (optional) - show the result translucent and change nothing\n\
                 clear: boolean (optional) - true removes the preview instead",
        mutates: true,
        run: draft,
    },
    CommandSpec {
        id: "solid.rib",
        label: "Rib",
        params: "sketch: integer (required) - the sketch holding the chain\n\
                 curves: list of integers (required) - lines and arcs joined end to end, in order; open\n\
                 thickness: string or number (required) - the wall, across the sketch plane, half each side\n\
                 web: boolean (optional) - go through the solid, filling every gap, instead of stopping at its first faces\n\
                 flip: boolean (optional) - grow away from the solid's side of the chain\n\
                 body: string (optional) - the body to join; default the solid nearest the chain\n\
                 preview: boolean (optional) - show the result translucent and change nothing\n\
                 clear: boolean (optional) - true removes the preview instead",
        mutates: true,
        run: rib,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_bool(p, key).map(Some),
    }
}

fn curves(p: &Value) -> Result<Option<Vec<u32>>, CommandError> {
    match p.get("curves") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| bad("`curves` must be a list of curve ids"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(bad("`curves` must be a list of curve ids")),
    }
}

fn body_name(p: &Value) -> Result<Option<String>, CommandError> {
    Ok(params::opt_str(p, "body")?.map(str::to_owned))
}

fn is_clear(p: &Value) -> bool {
    matches!(p.get("clear"), Some(Value::Bool(true)))
}

fn is_preview(p: &Value) -> bool {
    matches!(p.get("preview"), Some(Value::Bool(true)))
}

fn draft(s: &mut Session, p: &Value) -> CmdResult {
    if is_clear(p) {
        s.scene.previews.clear();
        return Ok(json!({ "cleared": true }));
    }
    let faces = faces(s, p)?;
    let neutral = params::req_str(p, "neutral")?.to_owned();
    let angle = expression(p, "angle", "deg")?.ok_or_else(|| bad("`angle` is required"))?;
    let flip = opt_bool(p, "flip")?.unwrap_or(false);
    let kind = FeatureKind::Draft(Draft {
        faces,
        neutral,
        angle,
        flip,
    });
    if is_preview(p) {
        return preview(s, kind);
    }
    add(s, kind)
}

fn rib(s: &mut Session, p: &Value) -> CmdResult {
    if is_clear(p) {
        s.scene.previews.clear();
        return Ok(json!({ "cleared": true }));
    }
    let sketch = params::req_id(p, "sketch")?;
    let curves = curves(p)?.ok_or_else(|| bad("`curves` is required"))?;
    let thickness =
        expression(p, "thickness", "mm")?.ok_or_else(|| bad("`thickness` is required"))?;
    let kind = FeatureKind::Rib(Rib {
        sketch,
        curves,
        thickness,
        web: opt_bool(p, "web")?.unwrap_or(false),
        flip: opt_bool(p, "flip")?.unwrap_or(false),
        body: body_name(p)?,
    });
    if is_preview(p) {
        return preview(s, kind);
    }
    add(s, kind)
}

/// The face names a draft holds, for their signatures.
fn named_faces(kind: &FeatureKind) -> Vec<String> {
    match kind {
        FeatureKind::Draft(d) => {
            let mut v = d.faces.clone();
            if d.neutral.contains(':') {
                v.push(d.neutral.clone());
            }
            v
        }
        _ => Vec::new(),
    }
}

/// Adds `kind` to the timeline and rebuilds; the feature's error is the command's.
fn add(s: &mut Session, kind: FeatureKind) -> CmdResult {
    s.scene.previews.clear();
    let named = named_faces(&kind);
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

/// Builds `kind` in a scratch copy of the design and shows the bodies it changes as
/// translucent previews; the design does not change.
fn preview(s: &mut Session, kind: FeatureKind) -> CmdResult {
    s.scene.previews.clear();
    let mut scratch = Session {
        doc: s.doc.clone(),
        ..Session::default()
    };
    let id = scratch.doc.add_feature(kind)?;
    scratch.rebuild();
    if let Some(e) = scratch.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    let changed = scratch.edited_solids(id).to_vec();
    let mut volume = 0.0;
    for x in scratch.solids() {
        if !changed.contains(&x.name) {
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
        "volume": volume,
        "previews": s.scene.previews.len(),
    }))
}

/// True for the feature kinds this file edits through `feature.edit`.
pub(super) fn is_draft_or_rib(kind: &FeatureKind) -> bool {
    matches!(kind, FeatureKind::Draft(_) | FeatureKind::Rib(_))
}

/// `feature.edit` of a draft (`faces`, `neutral`, `angle`, `flip`) or a rib (`curves`,
/// `thickness`, `web`, `flip`, `body`), and `suppressed`.
pub(super) fn edit(s: &mut Session, id: u32, p: &Value) -> CmdResult {
    let mut kind = s.doc.feature(id)?.kind.clone();
    let allowed: &[&str] = match &kind {
        FeatureKind::Draft(_) => &["feature", "faces", "neutral", "angle", "flip", "suppressed"],
        FeatureKind::Rib(_) => &[
            "feature",
            "curves",
            "thickness",
            "web",
            "flip",
            "body",
            "suppressed",
        ],
        _ => return Err(bad("not a draft, rib or web")),
    };
    if let Some(obj) = p.as_object()
        && let Some(k) = obj.keys().find(|k| !allowed.contains(&k.as_str()))
    {
        return Err(bad(format!(
            "this feature takes {}, not `{k}`",
            allowed.get(1..).unwrap_or_default().join(", ")
        )));
    }
    let flip = opt_bool(p, "flip")?;
    match &mut kind {
        FeatureKind::Draft(d) => {
            if !matches!(p.get("faces"), None | Some(Value::Null)) {
                d.faces = faces(s, &json!({ "faces": p.get("faces") }))?;
            }
            if let Some(n) = params::opt_str(p, "neutral")? {
                d.neutral = n.to_owned();
            }
            if let Some(a) = expression(p, "angle", "deg")? {
                d.angle = a;
            }
            if let Some(f) = flip {
                d.flip = f;
            }
        }
        FeatureKind::Rib(r) => {
            if let Some(c) = curves(p)? {
                r.curves = c;
            }
            if let Some(t) = expression(p, "thickness", "mm")? {
                r.thickness = t;
            }
            if let Some(w) = opt_bool(p, "web")? {
                r.web = w;
            }
            if let Some(f) = flip {
                r.flip = f;
            }
            if let Some(b) = body_name(p)? {
                r.body = Some(b);
            }
        }
        _ => return Err(bad("not a draft, rib or web")),
    }
    let suppressed = opt_bool(p, "suppressed")?;
    s.doc.check_feature(&kind)?;
    s.doc.feature_mut(id)?.kind = kind;
    if let Some(v) = suppressed {
        s.doc.set_suppressed(id, v)?;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

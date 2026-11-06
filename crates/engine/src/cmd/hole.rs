//! `solid.hole`: simple, counterbore and countersink holes on a flat face.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{FaceRef, FeatureKind, Hole, HoleKind};
use chamfera_geom::Point2;
use serde_json::Value;

pub static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "solid.hole",
    label: "Hole",
    params: "face: string (optional) - the flat face by name, such as \"1:top\"\n\
             pick: {x, y, width, height} (optional) - the face under this pixel, instead\n\
             points: list (optional) - hole centres: {x, y} in the face's plane coordinates, or\n\
             {x, y, width, height} pixels on the face\n\
             sketch: integer (optional) - also drill at this sketch's standalone points\n\
             kind: string (optional) - simple (default), counterbore or countersink\n\
             diameter: string or number (required) - such as 6 or \"6 mm\"\n\
             depth: string or number (optional) - the drilled depth; or\n\
             through_all: boolean (optional) - through the whole body instead\n\
             cb_diameter, cb_depth: string or number (counterbore) - its diameter and depth\n\
             cs_diameter: string or number (countersink) - its diameter at the face\n\
             cs_angle: string or number (optional) - countersink angle, default 90 deg\n\
             tip_angle: string or number (optional) - drill point, default 118 deg; \"flat\" for none",
    mutates: true,
    run: hole,
}];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// An expression given as a number (in `unit`) or a string.
fn expression(p: &Value, key: &str, unit: &str) -> Result<Option<String>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => Ok(Some(format!("{} {unit}", params::req_f64(p, key)?))),
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

/// The face a hole goes on: `face` by name, or the one under `pick`.
fn face(s: &Session, p: &Value) -> Result<FaceRef, CommandError> {
    match (p.get("face"), p.get("pick")) {
        (Some(Value::String(name)), None | Some(Value::Null)) => {
            let (feature, role) = crate::solids::split_face_key(name).ok_or_else(|| {
                bad("`face` must be a face name such as \"1:top\" (feature:role)")
            })?;
            Ok(FaceRef {
                feature,
                face: role.to_owned(),
            })
        }
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y, w, h) = params::pixel(pick)?;
            let (feature, face, _) = s.face_at(x, y, w, h).ok_or_else(|| {
                CommandError::Unavailable("no flat face of a solid is under that pixel".into())
            })?;
            Ok(FaceRef { feature, face })
        }
        (None | Some(Value::Null), None) => Err(bad("give `face` or `pick`")),
        _ => Err(bad("give either `face` (a name) or `pick`, not both")),
    }
}

/// The hole centres: face coordinates, or pixels on the face `on`.
fn points(s: &Session, p: &Value, on: &FaceRef) -> Result<Option<Vec<Point2>>, CommandError> {
    let list = match p.get("points") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(bad("`points` must be a list of {x, y}")),
    };
    if list.len() > chamfera_doc::feature::MAX_HOLE_POINTS {
        return Err(bad("too many hole points"));
    }
    list.iter()
        .map(|v| {
            let pt = if v.get("width").is_some() {
                let (x, y, w, h) = params::pixel(v)?;
                match s.face_point_at(x, y, w, h) {
                    // Any piece of a split face will do: the centres pick the piece.
                    Some((f, role, at))
                        if f == on.feature
                            && chamfera_doc::naming::split_piece(&role).0
                                == chamfera_doc::naming::split_piece(&on.face).0 =>
                    {
                        at
                    }
                    _ => {
                        return Err(CommandError::Unavailable(format!(
                            "the pixel ({x:.0}, {y:.0}) is not on the face {:?}",
                            on.face
                        )));
                    }
                }
            } else {
                Point2::new(params::req_f64(v, "x")?, params::req_f64(v, "y")?)
            };
            if !pt.is_valid() {
                return Err(bad("a hole point is outside the working range"));
            }
            Ok(pt)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// `depth` or `through_all` (given one of them); `None` when neither is given.
fn depth(p: &Value) -> Result<Option<Option<String>>, CommandError> {
    let through = match p.get("through_all") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "through_all")?),
    };
    match (expression(p, "depth", "mm")?, through) {
        (Some(_), Some(true)) => Err(bad("give `depth` or `through_all`, not both")),
        (Some(d), _) => Ok(Some(Some(d))),
        (None, Some(true)) => Ok(Some(None)),
        (None, Some(false) | None) => Ok(None),
    }
}

fn kind(p: &Value) -> Result<Option<HoleKind>, CommandError> {
    match params::opt_str(p, "kind")? {
        None => Ok(None),
        Some(k) => HoleKind::parse(k).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown hole kind {k:?}: use simple, counterbore or countersink"
            ))
        }),
    }
}

/// The drill point: an angle, or `"flat"` for none.
fn tip(p: &Value) -> Result<Option<Option<String>>, CommandError> {
    if p.get("tip_angle").and_then(Value::as_str) == Some("flat") {
        return Ok(Some(None));
    }
    Ok(expression(p, "tip_angle", "deg")?.map(Some))
}

/// Applies the hole parameters given in `p` to `h` (for `solid.hole` and `feature.edit`).
pub(super) fn apply(s: &Session, h: &mut Hole, p: &Value) -> Result<(), CommandError> {
    if let Some(pts) = points(s, p, &h.face)? {
        h.points = pts;
    }
    if p.get("sketch").is_some_and(|v| !v.is_null()) {
        h.sketch = Some(params::req_id(p, "sketch")?);
    }
    if let Some(k) = kind(p)? {
        h.hole = k;
    }
    if let Some(d) = expression(p, "diameter", "mm")? {
        h.diameter = d;
    }
    if let Some(d) = depth(p)? {
        h.depth = d;
    }
    for (key, unit, slot) in [
        ("cb_diameter", "mm", &mut h.cb_diameter),
        ("cb_depth", "mm", &mut h.cb_depth),
        ("cs_diameter", "mm", &mut h.cs_diameter),
        ("cs_angle", "deg", &mut h.cs_angle),
    ] {
        if let Some(v) = expression(p, key, unit)? {
            *slot = Some(v);
        }
    }
    if let Some(t) = tip(p)? {
        h.tip_angle = t;
    }
    Ok(())
}

/// The keys `feature.edit` hands to [`apply`] for a hole.
pub(super) const EDIT_KEYS: &[&str] = &[
    "points",
    "kind",
    "diameter",
    "depth",
    "through_all",
    "cb_diameter",
    "cb_depth",
    "cs_diameter",
    "cs_angle",
    "tip_angle",
];

fn hole(s: &mut Session, p: &Value) -> CmdResult {
    let on = face(s, p)?;
    let mut h = Hole {
        face: on,
        points: Vec::new(),
        sketch: None,
        hole: HoleKind::Simple,
        diameter: String::new(),
        depth: None,
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle: None,
        tip_angle: Some("118 deg".into()),
    };
    if expression(p, "diameter", "mm")?.is_none() {
        return Err(bad("`diameter` is required"));
    }
    if depth(p)?.is_none() {
        return Err(bad("give `depth` or `through_all: true`"));
    }
    apply(s, &mut h, p)?;
    let on = h.face.clone();
    let id = s.doc.add_feature(FeatureKind::Hole(h))?;
    s.record_face_signature(id, &on, None);
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(super::feature::feature_reply(s, id))
}

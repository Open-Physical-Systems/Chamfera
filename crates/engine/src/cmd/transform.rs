//! `solid.mirror` and `solid.move`: timeline features that mirror, move or copy bodies.

use super::CommandSpec;
use super::feature::{expression, feature_reply, operation, vec3};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{FeatureKind, Mirror, MirrorPlane, Move, MoveTransform, Operation};
use chamfera_geom::{Point3, Vec3};
use serde_json::Value;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "solid.mirror",
        label: "Mirror",
        params: "bodies: list of strings (optional) - body names; default the selected body\n\
                 features: list of integers (optional) - features whose shapes to mirror\n\
                 plane: string or object (required) - xy, xz or yz; {feature, face} for a\n\
                 flat face; or {origin: {x, y, z}, normal: {x, y, z}}\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: mirror,
    },
    CommandSpec {
        id: "solid.move",
        label: "Move",
        params: "bodies: list of strings (optional) - body names; default the selected body\n\
                 dx, dy, dz: string or number (optional) - a translation (plain = mm)\n\
                 angle: string or number (optional) - a rotation instead (plain = degrees)\n\
                 axis: {x, y, z} (optional) - rotation axis, default z\n\
                 origin: {x, y, z} (optional) - a point on the rotation axis, default 0\n\
                 from, to: {x, y, z} (optional) - move by the vector between two points\n\
                 copy: boolean (optional) - keep the originals and add moved copies",
        mutates: true,
        run: move_bodies,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn point(p: &Value, key: &str) -> Result<Point3, CommandError> {
    let v = vec3(p, key)?;
    let q = Point3::new(v.x, v.y, v.z);
    if !q.is_valid() {
        return Err(bad(format!("`{key}` is outside the working range")));
    }
    Ok(q)
}

/// `bodies` as given, or the selection when it is left out.
fn bodies(s: &Session, p: &Value, required: bool) -> Result<Vec<String>, CommandError> {
    match p.get("bodies") {
        None | Some(Value::Null) => {
            let sel = s.selected_body_names();
            if sel.is_empty() && required {
                return Err(bad("give `bodies` (body names) or select a body first"));
            }
            Ok(sel)
        }
        Some(Value::Array(list)) => {
            if list.len() > chamfera_doc::feature::MAX_TRANSFORM_ITEMS {
                return Err(bad("too many bodies"));
            }
            list.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| bad("`bodies` must be a list of body names"))
                })
                .collect()
        }
        Some(_) => Err(bad("`bodies` must be a list of body names")),
    }
}

fn features(p: &Value) -> Result<Vec<u32>, CommandError> {
    match p.get("features") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => {
            if list.len() > chamfera_doc::feature::MAX_TRANSFORM_ITEMS {
                return Err(bad("too many features"));
            }
            list.iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| bad("`features` must be a list of feature ids"))
                })
                .collect()
        }
        Some(_) => Err(bad("`features` must be a list of feature ids")),
    }
}

/// The `plane` parameter of a mirror.
pub(super) fn mirror_plane(p: &Value) -> Result<Option<MirrorPlane>, CommandError> {
    match p.get("plane") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => MirrorPlane::origin(name)
            .map(Some)
            .ok_or_else(|| bad(format!("unknown plane {name:?}: use xy, xz or yz"))),
        Some(v @ Value::Object(o)) if o.contains_key("face") => Ok(Some(MirrorPlane::Face {
            feature: params::req_id(v, "feature")?,
            face: params::req_str(v, "face")?.to_owned(),
        })),
        Some(v @ Value::Object(_)) => {
            let normal = vec3(v, "normal")?;
            if normal.normalized().is_none() {
                return Err(bad("the plane's `normal` has no length"));
            }
            Ok(Some(MirrorPlane::Plane {
                origin: point(v, "origin")?,
                normal,
            }))
        }
        Some(_) => Err(bad(
            "`plane` must be xy, xz, yz, {feature, face} or {origin, normal}",
        )),
    }
}

/// Adds the feature, rebuilds and replies, or fails (the session rolls back).
fn add(s: &mut Session, kind: FeatureKind) -> CmdResult {
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn mirror(s: &mut Session, p: &Value) -> CmdResult {
    let features = features(p)?;
    let bodies = bodies(s, p, features.is_empty())?;
    let plane = mirror_plane(p)?.ok_or_else(|| bad("`plane` is required"))?;
    add(
        s,
        FeatureKind::Mirror(Mirror {
            bodies,
            features,
            plane,
            operation: operation(p)?.unwrap_or(Operation::NewBody),
        }),
    )
}

/// The transform a move's parameters describe, or `None` when they name none.
pub(super) fn move_transform(p: &Value) -> Result<Option<MoveTransform>, CommandError> {
    let has = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if has("angle") {
        let axis = if has("axis") {
            vec3(p, "axis")?
        } else {
            Vec3::Z
        };
        if axis.normalized().is_none() {
            return Err(bad("the rotation `axis` has no length"));
        }
        let origin = if has("origin") {
            point(p, "origin")?
        } else {
            Point3::ORIGIN
        };
        let angle = expression(p, "angle", "deg")?.unwrap_or_default();
        return Ok(Some(MoveTransform::Rotate {
            origin,
            axis,
            angle,
        }));
    }
    if has("from") || has("to") {
        return Ok(Some(MoveTransform::PointToPoint {
            from: point(p, "from")?,
            to: point(p, "to")?,
        }));
    }
    if has("dx") || has("dy") || has("dz") {
        let d = |k: &str| -> Result<String, CommandError> {
            Ok(expression(p, k, "mm")?.unwrap_or_else(|| "0 mm".to_owned()))
        };
        return Ok(Some(MoveTransform::Translate {
            dx: d("dx")?,
            dy: d("dy")?,
            dz: d("dz")?,
        }));
    }
    Ok(None)
}

/// `feature.edit` for mirrors and moves: a new plane, transform or copy flag. Those keys
/// on any other feature are refused.
pub(super) fn edit_transform(kind: &mut FeatureKind, p: &Value) -> Result<(), CommandError> {
    if !matches!(kind, FeatureKind::Mirror(_) | FeatureKind::Move(_)) {
        // `angle` is shared with planes at an angle, which feature.edit handles itself.
        let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
        // So is `to`, which an extrude's options and a hole use.
        let extrude = matches!(kind, FeatureKind::Extrude(_) | FeatureKind::Hole(_));
        // And `plane`, which splits use (body_tools handles it).
        let split = matches!(kind, FeatureKind::SplitBody(_) | FeatureKind::SplitFace(_));
        if [
            "plane", "dx", "dy", "dz", "axis", "origin", "from", "to", "copy",
        ]
        .into_iter()
        .filter(|k| !(extrude && *k == "to"))
        .filter(|k| !(split && *k == "plane"))
        .any(given)
        {
            return Err(bad(
                "plane belongs to mirrors; dx, dy, dz, angle, from, to and copy to moves",
            ));
        }
        return Ok(());
    }
    let plane = mirror_plane(p)?;
    let transform = move_transform(p)?;
    let copy = match p.get("copy") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "copy")?),
    };
    match kind {
        FeatureKind::Mirror(m) if transform.is_none() && copy.is_none() => {
            if let Some(plane) = plane {
                m.plane = plane;
            }
        }
        FeatureKind::Move(m) if plane.is_none() => {
            if let Some(t) = transform {
                m.transform = t;
            }
            if let Some(c) = copy {
                m.copy = c;
            }
        }
        _ if plane.is_none() && transform.is_none() && copy.is_none() => {}
        _ => {
            return Err(bad(
                "plane belongs to mirrors; dx, dy, dz, angle, from, to and copy to moves",
            ));
        }
    }
    Ok(())
}

fn move_bodies(s: &mut Session, p: &Value) -> CmdResult {
    let bodies = bodies(s, p, true)?;
    let transform =
        move_transform(p)?.ok_or_else(|| bad("give dx/dy/dz, an angle, or from and to"))?;
    let copy = match p.get("copy") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "copy")?,
    };
    add(
        s,
        FeatureKind::Move(Move {
            bodies,
            transform,
            copy,
        }),
    )
}

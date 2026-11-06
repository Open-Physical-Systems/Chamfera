//! `joint.*`: joints between components, and driving them.

use super::CommandSpec;
use super::component::{angle, bad, component_id, component_json, xyz};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{EdgeEnd, Joint, JointKind, JointOrigin, OriginAt, Unit};
use chamfera_geom::{MAX_COORDINATE, Point3};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "joint.create",
        label: "Joint",
        params: "type: string (required) - rigid, revolute, slider, cylindrical or planar\n\
                 a, b: object (required) - the joint origins; B moves to A unless B is grounded. Each is {component?, face} (a flat face's centre and normal, by name like \"2:top\"), {component?, edge} (a circle's centre and axis or a line's midpoint and direction), {component?, edge, end: \"start\"|\"end\"} (a vertex), {component, point: {x, y, z}, axis: {x, y, z}} (in the component's coordinates) or {pick: {x, y, width, height}} (what is under a pixel)\n\
                 value: number or expression (optional) - the angle (degrees) or, for a slider, the distance (mm); default 0\n\
                 distance: number (optional) - cylindrical: the slide along the axis, mm\n\
                 offset: {x, y} (optional) - planar: the slide across the axis, mm\n\
                 min, max: number or expression (optional) - limits for the value\n\
                 flip: boolean (optional) - turn B over so the two origins face each other\n\
                 name: string (optional)",
        mutates: true,
        run: create,
    },
    CommandSpec {
        id: "joint.delete",
        label: "Delete Joint",
        params: "joint: integer (required) - the joint id; its component stays where the joint left it",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "joint.origin_at",
        label: "Joint Origin at Pixel",
        params: "x, y: number (required) - pixel, origin top-left\n\
                 width, height: number (required) - viewport size in pixels",
        mutates: false,
        run: origin_at,
    },
    CommandSpec {
        id: "joint.set_value",
        label: "Drive Joint",
        params: "joint: integer (required)\n\
                 value: number or expression (optional) - the new angle (degrees) or distance (mm); it must be inside the joint's limits\n\
                 distance: number (optional) - cylindrical: the slide along the axis, mm\n\
                 offset: {x, y} (optional) - planar: the slide across the axis, mm",
        mutates: true,
        run: set_value,
    },
];

/// Everything about one joint, for replies.
pub(crate) fn joint_json(s: &Session, id: u32) -> Value {
    let Ok(j) = s.doc.assembly.joint(id) else {
        return Value::Null;
    };
    let shown = |v: f64| match j.kind.value_unit() {
        Some(Unit::Angle) => v.to_degrees(),
        _ => v,
    };
    json!({
        "joint": j.id,
        "name": j.name,
        "type": j.kind.key(),
        "a": j.a,
        "b": j.b,
        "value": shown(j.value),
        "value_unit": match j.kind.value_unit() {
            Some(Unit::Angle) => Some("deg"),
            Some(_) => Some("mm"),
            None => None,
        },
        "distance": j.distance,
        "offset": j.offset,
        "limits": j.limits.map(|[lo, hi]| [shown(lo), shown(hi)]),
        "flip": j.flip,
        "error": s.placements().errors.get(&id),
    })
}

/// A value in the joint's unit: degrees or an angle expression for turning joints, mm or
/// a length expression for a slider.
fn joint_value(
    s: &Session,
    kind: JointKind,
    v: Option<&Value>,
    what: &str,
) -> Result<f64, CommandError> {
    match kind.value_unit() {
        Some(Unit::Angle) => angle(s, v, what),
        None => match v {
            None | Some(Value::Null) => Ok(0.0),
            Some(_) => Err(bad("a rigid joint has no value")),
        },
        Some(_) => {
            let d = match v {
                Some(Value::Number(n)) => n
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| bad(format!("{what} must be a finite number")))?,
                Some(Value::String(e)) => s
                    .doc
                    .evaluate_feature_value(e, chamfera_doc::expr::Dim::LENGTH)?,
                None | Some(Value::Null) => return Err(bad(format!("{what} is required"))),
                Some(_) => return Err(bad(format!("{what} must be a number or an expression"))),
            };
            if d.abs() > MAX_COORDINATE {
                return Err(bad(format!("{what} is at most {MAX_COORDINATE} mm")));
            }
            Ok(d)
        }
    }
}

fn mm(p: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    let v = params::opt_f64(p, key)?;
    if v.is_some_and(|v| v.abs() > MAX_COORDINATE) {
        return Err(bad(format!("`{key}` is at most {MAX_COORDINATE} mm")));
    }
    Ok(v)
}

fn offset(p: &Value) -> Result<Option<[f64; 2]>, CommandError> {
    match p.get("offset") {
        None | Some(Value::Null) => Ok(None),
        Some(o) if o.is_object() => Ok(Some([
            mm(o, "x")?.unwrap_or(0.0),
            mm(o, "y")?.unwrap_or(0.0),
        ])),
        Some(_) => Err(bad("`offset` must be {x, y}")),
    }
}

/// The component whose solid has the named face or edge.
fn component_with(s: &Session, at: &OriginAt) -> Option<u32> {
    s.solids().iter().find_map(|x| {
        let has = match at {
            OriginAt::Face { face } => x
                .body
                .faces()
                .into_iter()
                .any(|f| x.body.face_name(f).ok().flatten() == Some(face.as_str())),
            OriginAt::Edge { edge } | OriginAt::Vertex { edge, .. } => x
                .body
                .edges()
                .into_iter()
                .any(|e| crate::solids::edge_ref(&x.body, e).as_deref() == Some(edge.as_str())),
            OriginAt::Point { .. } => false,
        };
        has.then_some(x.component)
    })
}

/// Reads a joint origin parameter (`a` or `b`).
fn origin(s: &Session, p: &Value, key: &str) -> Result<JointOrigin, CommandError> {
    let o = p.get(key).filter(|o| o.is_object()).ok_or_else(|| {
        bad(format!(
            "`{key}` is required: an object naming a joint origin"
        ))
    })?;
    if let Some(px) = o.get("pick").filter(|v| !v.is_null()) {
        let (x, y, w, h) = params::pixel(px)?;
        return s
            .joint_origin_at(x, y, w, h)
            .map(|(origin, _)| origin)
            .ok_or_else(|| {
                CommandError::Unavailable(format!(
                    "`{key}`: nothing to joint to under that pixel (pick a flat face or an edge)"
                ))
            });
    }
    let name = |k: &str| -> Result<Option<String>, CommandError> {
        let v = params::opt_str(o, k)?;
        if v.is_some_and(|v| v.is_empty() || v.chars().count() > 1024) {
            return Err(bad(format!("`{key}.{k}` must be 1 to 1024 characters")));
        }
        Ok(v.map(str::to_owned))
    };
    let at = if let Some(face) = name("face")? {
        OriginAt::Face { face }
    } else if let Some(edge) = name("edge")? {
        match params::opt_str(o, "end")? {
            None => OriginAt::Edge { edge },
            Some("start") => OriginAt::Vertex {
                edge,
                end: EdgeEnd::Start,
            },
            Some("end") => OriginAt::Vertex {
                edge,
                end: EdgeEnd::End,
            },
            Some(_) => return Err(bad(format!("`{key}.end` must be \"start\" or \"end\""))),
        }
    } else if let Some(point) = o.get("point").filter(|v| !v.is_null()) {
        let point = Point3::ORIGIN + xyz(point, &format!("{key}.point"))?;
        let axis = xyz(
            o.get("axis")
                .ok_or_else(|| bad(format!("`{key}.axis` is required with a point")))?,
            &format!("{key}.axis"),
        )?;
        if axis.normalized().is_none() {
            return Err(bad(format!("`{key}.axis` must not be zero")));
        }
        OriginAt::Point { point, axis }
    } else {
        return Err(bad(format!(
            "`{key}` needs a face, an edge, a point and axis, or a pick"
        )));
    };
    let component = match o.get("component") {
        None | Some(Value::Null) => match &at {
            OriginAt::Point { .. } => {
                return Err(bad(format!("`{key}.component` is required with a point")));
            }
            _ => component_with(s, &at).ok_or_else(|| {
                CommandError::Unavailable(format!("`{key}`: no body has that face or edge"))
            })?,
        },
        Some(_) => component_id(s, o, "component")?,
    };
    Ok(JointOrigin { component, at })
}

fn create(s: &mut Session, p: &Value) -> CmdResult {
    let kind = JointKind::parse(params::req_str(p, "type")?)
        .ok_or_else(|| bad("`type` must be rigid, revolute, slider, cylindrical or planar"))?;
    let a = origin(s, p, "a")?;
    let b = origin(s, p, "b")?;
    let value = match p.get("value") {
        None | Some(Value::Null) => 0.0,
        v => joint_value(s, kind, v, "`value`")?,
    };
    let limit = |k: &str| -> Result<Option<f64>, CommandError> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            v => joint_value(s, kind, v, &format!("`{k}`")).map(Some),
        }
    };
    let limits = match (limit("min")?, limit("max")?) {
        (None, None) => None,
        (Some(lo), Some(hi)) => Some([lo, hi]),
        _ => return Err(bad("give both `min` and `max`, or neither")),
    };
    let flip = match p.get("flip") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "flip")?,
    };
    let joint = Joint {
        id: 0,
        name: params::opt_str(p, "name")?.unwrap_or_default().to_owned(),
        kind,
        a,
        b,
        value,
        distance: mm(p, "distance")?.unwrap_or(0.0),
        offset: offset(p)?.unwrap_or([0.0, 0.0]),
        limits,
        flip,
    };
    let id = s.doc.assembly.add_joint(joint)?;
    s.rebuild();
    if let Some(e) = s.placements().errors.get(&id) {
        return Err(CommandError::Unavailable(format!(
            "the joint cannot be solved: {e}"
        )));
    }
    Ok(reply(s, id))
}

/// A joint and both its components, as placed now.
fn reply(s: &Session, id: u32) -> Value {
    let mut out = joint_json(s, id);
    if let Ok(j) = s.doc.assembly.joint(id) {
        out["components"] = json!([
            component_json(s, j.a.component),
            component_json(s, j.b.component)
        ]);
    }
    out
}

fn set_value(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "joint")?;
    let kind = s.doc.assembly.joint(id)?.kind;
    let value = match p.get("value") {
        None | Some(Value::Null) => None,
        v => Some(joint_value(s, kind, v, "`value`")?),
    };
    let distance = mm(p, "distance")?;
    let offset = offset(p)?;
    if value.is_none() && distance.is_none() && offset.is_none() {
        return Err(bad(
            "give a `value` (or a `distance` or `offset`) to drive the joint to",
        ));
    }
    if distance.is_some() && kind != JointKind::Cylindrical {
        return Err(bad("only a cylindrical joint takes a `distance`"));
    }
    if offset.is_some() && kind != JointKind::Planar {
        return Err(bad("only a planar joint takes an `offset`"));
    }
    let j = s.doc.assembly.joint_mut(id)?;
    if let Some(v) = value {
        j.check_value(v)?;
        j.value = v;
    }
    if let Some(d) = distance {
        j.distance = d;
    }
    if let Some(o) = offset {
        j.offset = o;
    }
    s.rebuild();
    if let Some(e) = s.placements().errors.get(&id) {
        return Err(CommandError::Unavailable(format!(
            "the joint cannot be solved: {e}"
        )));
    }
    Ok(reply(s, id))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "joint")?;
    // The component stays where the joint put it: that becomes its placement.
    let asm = &s.doc.assembly;
    let moved = asm.joint(id).ok().and_then(|j| {
        [j.a.component, j.b.component]
            .into_iter()
            .find(|c| asm.driver_of(*c).is_some_and(|d| d.joint == id))
    });
    let removed = s.doc.assembly.remove_joint(id)?;
    if let Some(c) = moved {
        let world = s.placements().of(c);
        let parent = s.doc.assembly.component(c).map(|x| x.parent).unwrap_or(0);
        let parent_world = s.placements().of(parent);
        if let Some(local) = parent_world
            .inverse_rigid()
            .map(|inv| world.then(&inv))
            .filter(chamfera_doc::component::is_valid_placement)
            && let Ok(x) = s.doc.assembly.component_mut(c)
        {
            x.placement = local;
        }
    }
    s.rebuild();
    Ok(json!({ "deleted": removed.id, "name": removed.name }))
}

fn origin_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    Ok(match s.joint_origin_at(x, y, w, h) {
        Some((o, frame)) => json!({
            "origin": o,
            "point": frame.origin(),
            "axis": frame.z(),
        }),
        None => json!({ "origin": null }),
    })
}

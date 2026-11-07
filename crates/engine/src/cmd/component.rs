//! `component.*`: the component tree of a design (the start of assemblies).

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::ROOT;
use chamfera_geom::{MAX_COORDINATE, Point3, Transform, Vec3};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "component.activate",
        label: "Activate Component",
        params: "component: integer (required) - the component new features go into; 0 is the root (the design)",
        mutates: true,
        run: activate,
    },
    CommandSpec {
        id: "component.copy",
        label: "Copy Component",
        params: "component: integer (required) - the component to copy; the copy is linked: it shows the same bodies\n\
                 transform: object (optional) - where the copy goes relative to the original, as in component.move",
        mutates: true,
        run: copy,
    },
    CommandSpec {
        id: "component.delete",
        label: "Delete Component",
        params: "component: integer (required) - the component; its bodies and child components move to its parent, its linked copies and joints are removed",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "component.ground",
        label: "Ground",
        params: "component: integer (required)\n\
                 grounded: boolean (optional) - default true; a grounded component stays put and joints move the other side",
        mutates: true,
        run: ground,
    },
    CommandSpec {
        id: "component.list",
        label: "List Components",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "component.move",
        label: "Move Component",
        params: "component: integer (required)\n\
                 transform: object (required) - {translate?: {x, y, z} mm, rotate?: {axis: {x, y, z}, angle: degrees or an expression, origin?: {x, y, z}}, absolute?: boolean}; the turn comes first, then the move; relative to the current placement unless absolute",
        mutates: true,
        run: move_component,
    },
    CommandSpec {
        id: "component.new",
        label: "New Component",
        params: "name: string (optional) - default ComponentN\n\
                 from_bodies: array of integers (optional) - bodies to move into it, by the feature (or imported body) that made each; without any, the new empty component becomes active\n\
                 parent: integer (optional) - the component to put it in; default the active one",
        mutates: true,
        run: new,
    },
    CommandSpec {
        id: "component.rename",
        label: "Rename Component",
        params: "component: integer (required)\nname: string (required)",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "component.tint",
        label: "Tint Component",
        params: "component: integer (required)\ntint: boolean (required) - draw its bodies with a subtle colour of their own",
        mutates: true,
        run: tint,
    },
];

pub(crate) fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// A component id: 0 (the root) or a component's id.
pub(crate) fn component_id(s: &Session, p: &Value, key: &str) -> Result<u32, CommandError> {
    let v = p
        .as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))?
        .get(key);
    let id = match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| bad(format!("`{key}` must be a whole number id")))?,
        None | Some(Value::Null) => return Err(bad(format!("`{key}` is required"))),
        Some(_) => return Err(bad(format!("`{key}` must be a whole number id"))),
    };
    if !s.doc.assembly.exists(id) {
        return Err(CommandError::Unavailable(format!("no component {id}")));
    }
    Ok(id)
}

/// A point or vector `{x, y, z}`, each finite and within the working range.
pub(crate) fn xyz(v: &Value, what: &str) -> Result<Vec3, CommandError> {
    let read = |k: &str| params::req_f64(v, k).map_err(|e| bad(format!("{what}: {e}")));
    let out = Vec3::new(read("x")?, read("y")?, read("z")?);
    if [out.x, out.y, out.z]
        .iter()
        .any(|c| c.abs() > MAX_COORDINATE)
    {
        return Err(bad(format!(
            "{what}: coordinates are at most {MAX_COORDINATE} mm"
        )));
    }
    Ok(out)
}

/// An angle: a number of degrees, or an expression such as `"pi / 2"` or `"30 deg"`.
pub(crate) fn angle(s: &Session, v: Option<&Value>, what: &str) -> Result<f64, CommandError> {
    let a = match v {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| bad(format!("{what} must be a finite number")))?
            .to_radians(),
        Some(Value::String(e)) => s
            .doc
            .evaluate_feature_value(e, chamfera_doc::expr::Dim::ANGLE)?,
        None | Some(Value::Null) => return Err(bad(format!("{what} is required"))),
        Some(_) => return Err(bad(format!("{what} must be a number or an expression"))),
    };
    if a.abs() > chamfera_doc::joint::MAX_JOINT_ANGLE {
        return Err(bad(format!("{what} is too large")));
    }
    Ok(a)
}

/// Reads `transform`: `{translate?, rotate?: {axis, angle, origin?}, absolute?}`.
/// Returns the move and whether it replaces the placement.
fn transform(s: &Session, p: &Value) -> Result<(Transform, bool), CommandError> {
    let t = p
        .get("transform")
        .ok_or_else(|| bad("`transform` is required"))?;
    if !t.is_object() {
        return Err(bad("`transform` must be an object"));
    }
    let mut out = Transform::IDENTITY;
    if let Some(r) = t.get("rotate").filter(|r| !r.is_null()) {
        if !r.is_object() {
            return Err(bad("`rotate` must be an object"));
        }
        let axis = xyz(
            r.get("axis")
                .ok_or_else(|| bad("`rotate.axis` is required"))?,
            "rotate.axis",
        )?;
        let a = angle(s, r.get("angle"), "`rotate.angle`")?;
        let origin = match r.get("origin").filter(|o| !o.is_null()) {
            Some(o) => Point3::ORIGIN + xyz(o, "rotate.origin")?,
            None => Point3::ORIGIN,
        };
        out = Transform::rotation_about(origin, axis, a)
            .ok_or_else(|| bad("`rotate.axis` must not be zero"))?;
    }
    if let Some(v) = t.get("translate").filter(|v| !v.is_null()) {
        let d = xyz(v, "translate")?;
        out = out.then(&Transform::translation(d).ok_or_else(|| bad("bad translation"))?);
    }
    let absolute = match t.get("absolute") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(t, "absolute")?,
    };
    Ok((out, absolute))
}

fn placement_json(t: &Transform) -> Value {
    json!({ "matrix": t.matrix(), "translation": t.translation_part() })
}

/// Everything about one component, for replies.
pub(crate) fn component_json(s: &Session, id: u32) -> Value {
    let asm = &s.doc.assembly;
    let c = asm.component(id).ok();
    let bodies: Vec<Value> = s
        .solids()
        .iter()
        .filter(|x| x.component == id)
        .map(|x| {
            json!({
                "name": x.name,
                "feature": x.feature,
                "body": x.scene_id,
                "volume": x.body.volume().ok(),
                "bounds": x.body.bounding_box(),
            })
        })
        .collect();
    json!({
        "component": id,
        "name": c.map_or_else(|| "(root)".to_owned(), |c| c.name.clone()),
        "parent": c.map(|c| c.parent),
        "active": asm.active == id,
        "grounded": c.is_none_or(|c| c.grounded),
        "occurrence_of": c.and_then(|c| c.occurrence_of),
        "tint": c.is_some_and(|c| c.tint),
        "placement": c.map(|c| placement_json(&c.placement)),
        "world": placement_json(&s.placements().of(id)),
        "driven_by": asm.driver_of(id).map(|d| d.joint),
        "bodies": bodies,
    })
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let mut components = vec![component_json(s, ROOT)];
    components.extend(
        s.doc
            .assembly
            .components
            .iter()
            .map(|c| component_json(s, c.id)),
    );
    let joints: Vec<Value> = s
        .doc
        .assembly
        .joints
        .iter()
        .map(|j| super::joint::joint_json(s, j.id))
        .collect();
    Ok(json!({
        "active": s.doc.assembly.active,
        "components": components,
        "joints": joints,
    }))
}

fn new(s: &mut Session, p: &Value) -> CmdResult {
    let bodies: Vec<u32> = match p.get("from_bodies") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => {
            if a.len() > chamfera_doc::component::MAX_COMPONENT_FEATURES {
                return Err(bad("`from_bodies` lists too many bodies"));
            }
            a.iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|v| u32::try_from(v).ok())
                        .filter(|v| *v > 0)
                        .ok_or_else(|| bad("`from_bodies` holds whole number ids"))
                })
                .collect::<Result<_, _>>()?
        }
        Some(_) => return Err(bad("`from_bodies` must be an array of ids")),
    };
    for b in &bodies {
        let known = s.doc.feature(*b).is_ok() || s.doc.imported.iter().any(|i| i.id == *b);
        if !known {
            return Err(CommandError::Unavailable(format!(
                "no body made by feature {b}"
            )));
        }
    }
    let parent = match p.get("parent") {
        None | Some(Value::Null) => s.doc.assembly.active,
        Some(_) => component_id(s, p, "parent")?,
    };
    let name = match params::opt_str(p, "name")? {
        Some(n) => n.to_owned(),
        None => format!("Component{}", s.doc.assembly.components.len() + 1),
    };
    let id = s.doc.assembly.add_component(&name, parent, &bodies)?;
    if bodies.is_empty() {
        s.doc.assembly.active = id;
    }
    s.rebuild();
    Ok(component_json(s, id))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    let name = params::req_str(p, "name")?;
    chamfera_doc::component::check_name(name)?;
    if id == ROOT {
        return Err(CommandError::Unavailable(
            "the root component is the design: save it under a new name instead".into(),
        ));
    }
    s.doc.assembly.component_mut(id)?.name = name.to_owned();
    Ok(component_json(s, id))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    let gone = s.doc.assembly.remove_component(id)?;
    s.rebuild();
    Ok(json!({ "deleted": gone }))
}

fn activate(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    if s.doc
        .assembly
        .component(id)
        .is_ok_and(|c| c.occurrence_of.is_some())
    {
        return Err(CommandError::Unavailable(
            "a linked copy shows another component: activate the original to change it".into(),
        ));
    }
    s.doc.assembly.active = id;
    Ok(component_json(s, id))
}

fn copy(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    // A copy starts where the original is shown: a jointed original's own placement is
    // where it was before its joint moved it. (A nested original's placement is in its
    // parent's coordinates, as the copy's is.)
    let c = s.doc.assembly.component(id)?;
    let shown = if c.parent == ROOT {
        s.placements().of(id)
    } else {
        c.placement
    };
    let placement = match p.get("transform") {
        None | Some(Value::Null) => shown,
        Some(_) => {
            let (t, absolute) = transform(s, p)?;
            if absolute { t } else { shown.then(&t) }
        }
    };
    let copy = s.doc.assembly.add_occurrence(id, placement)?;
    s.rebuild();
    Ok(component_json(s, copy))
}

fn move_component(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    if id == ROOT {
        return Err(CommandError::Unavailable(
            "the root component is the design's origin and does not move".into(),
        ));
    }
    if let Some(d) = s.doc.assembly.driver_of(id) {
        let name = s
            .doc
            .assembly
            .joint(d.joint)
            .map_or_else(|_| d.joint.to_string(), |j| j.name.clone());
        return Err(CommandError::Unavailable(format!(
            "joint {name:?} positions this component: drive the joint (joint.set_value) or delete it"
        )));
    }
    let (t, absolute) = transform(s, p)?;
    let c = s.doc.assembly.component_mut(id)?;
    let placement = if absolute { t } else { c.placement.then(&t) };
    if !chamfera_doc::component::is_valid_placement(&placement) {
        return Err(bad("the move takes the component out of the working range"));
    }
    c.placement = placement;
    s.rebuild();
    Ok(component_json(s, id))
}

fn ground(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    let grounded = match p.get("grounded") {
        None | Some(Value::Null) => true,
        Some(_) => params::req_bool(p, "grounded")?,
    };
    if id == ROOT {
        return if grounded {
            Ok(component_json(s, id))
        } else {
            Err(CommandError::Unavailable(
                "the root component is the design's origin and is always grounded".into(),
            ))
        };
    }
    s.doc.assembly.component_mut(id)?.grounded = grounded;
    chamfera_doc::joint::check_tree(&s.doc.assembly)?;
    s.rebuild();
    Ok(component_json(s, id))
}

fn tint(s: &mut Session, p: &Value) -> CmdResult {
    let id = component_id(s, p, "component")?;
    let on = params::req_bool(p, "tint")?;
    if id == ROOT {
        return Err(CommandError::Unavailable(
            "the root component keeps the plain body colour".into(),
        ));
    }
    s.doc.assembly.component_mut(id)?.tint = on;
    s.rebuild();
    Ok(component_json(s, id))
}

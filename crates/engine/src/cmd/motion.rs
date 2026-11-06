//! `motion.*`: the motion study (keyframes of joint values), plus the assembly tools that
//! only change what is shown: `joint.drag`, `inspect.interference`, `view.explode` and
//! `view.explode_clear` (registered in their own areas, run from here).
//!
//! Showing a motion time, dragging and exploding are display state: they never add undo
//! steps. Adding and removing keys changes the design and can be undone.

use super::CommandSpec;
use super::component::{bad, xyz};
use super::joint::joint_value;
use crate::motion::{InterferenceReport, MAX_EXPLODE_FACTOR, MAX_INTERFERENCE_BODIES};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::motion::{MAX_MOTION_TIME, MotionKey, check_time};
use chamfera_doc::{JointKind, Unit};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "motion.add_key",
        label: "Add Motion Key",
        params: "joint: integer (required) - the joint the key drives\n\
                 time: number (required) - seconds from the start, 0 to 100000\n\
                 value: number or expression (optional) - the joint's angle (degrees) or distance (mm) then, inside its limits; default the value it shows now\n\
                 A key at the same joint and time is replaced.",
        mutates: true,
        run: add_key,
    },
    CommandSpec {
        id: "motion.list",
        label: "Motion Keys",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "motion.remove_key",
        label: "Remove Motion Key",
        params: "joint: integer (required)\ntime: number (required) - the key's time in seconds",
        mutates: true,
        run: remove_key,
    },
    CommandSpec {
        id: "motion.set_time",
        label: "Motion Time",
        params: "t: number or null (required) - show the study at this time in seconds (keyed joints take their interpolated values); null leaves the study and shows the joints' own values\n\
                 Display only: it adds no undo step.",
        mutates: false,
        run: set_time,
    },
];

/// A joint value in what the user types: degrees for turning joints, mm for a slider.
fn shown(kind: JointKind, v: f64) -> f64 {
    match kind.value_unit() {
        Some(Unit::Angle) => v.to_degrees(),
        _ => v,
    }
}

fn kind_of(s: &Session, joint: u32) -> Option<JointKind> {
    s.doc.assembly.joint(joint).ok().map(|j| j.kind)
}

fn key_json(s: &Session, k: &MotionKey) -> Value {
    let kind = kind_of(s, k.joint).unwrap_or(JointKind::Rigid);
    json!({
        "joint": k.joint,
        "name": s.doc.assembly.joint(k.joint).map(|j| j.name.clone()).unwrap_or_default(),
        "time": k.time,
        "value": shown(kind, k.value),
    })
}

fn time_param(p: &Value, key: &str) -> Result<f64, CommandError> {
    let t = params::req_f64(p, key)?;
    check_time(t).map_err(|_| {
        bad(format!(
            "`{key}` must be a time in seconds from 0 to {MAX_MOTION_TIME}"
        ))
    })?;
    Ok(t)
}

/// The study as a reply: keys, joints, duration and the time shown.
fn study_json(s: &Session) -> Value {
    let m = &s.doc.motion;
    let joints: Vec<Value> = m
        .joints()
        .into_iter()
        .map(|j| {
            let kind = kind_of(s, j).unwrap_or(JointKind::Rigid);
            json!({
                "joint": j,
                "name": s.doc.assembly.joint(j).map(|x| x.name.clone()).unwrap_or_default(),
                "value": s.shown_joint_value(j).map(|v| shown(kind, v)),
            })
        })
        .collect();
    json!({
        "keys": m.keys.iter().map(|k| key_json(s, k)).collect::<Vec<_>>(),
        "times": m.times(),
        "duration": m.duration(),
        "time": s.view_state.motion_time,
        "joints": joints,
    })
}

fn add_key(s: &mut Session, p: &Value) -> CmdResult {
    let joint = params::req_id(p, "joint")?;
    let kind = s.doc.assembly.joint(joint)?.kind;
    if kind == JointKind::Rigid {
        return Err(bad("a rigid joint has no value to key"));
    }
    let time = time_param(p, "time")?;
    let value = match p.get("value") {
        None | Some(Value::Null) => s
            .shown_joint_value(joint)
            .ok_or_else(|| bad(format!("no joint {joint}")))?,
        v => joint_value(s, kind, v, "`value`")?,
    };
    let key = MotionKey { joint, time, value };
    let asm = s.doc.assembly.clone();
    s.doc.motion.add_key(&asm, key)?;
    let mut out = study_json(s);
    out["key"] = key_json(s, &key);
    Ok(out)
}

fn remove_key(s: &mut Session, p: &Value) -> CmdResult {
    let joint = params::req_id(p, "joint")?;
    let time = time_param(p, "time")?;
    let k = s.doc.motion.remove_key(joint, time)?;
    let mut out = study_json(s);
    out["removed"] = key_json(s, &k);
    Ok(out)
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(study_json(s))
}

fn set_time(s: &mut Session, p: &Value) -> CmdResult {
    let o = p
        .as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))?;
    let t = match o.get("t") {
        None => return Err(bad("`t` is required: a time in seconds, or null")),
        Some(Value::Null) => None,
        Some(_) => Some(time_param(p, "t")?),
    };
    s.view_state.motion_time = t;
    s.view_state.drag.clear();
    s.rebuild();
    s.refresh_overlays();
    Ok(study_json(s))
}

/// `joint.drag`: shows a joint moved by a drag in the viewport, without changing the
/// design. The UI sends `joint.set_value` with the reply's value when the drag ends.
pub(crate) fn joint_drag(s: &mut Session, p: &Value) -> CmdResult {
    let joint = params::req_id(p, "joint")?;
    let j = s.doc.assembly.joint(joint)?.clone();
    if p.get("cancel").is_some_and(|v| !v.is_null()) && params::req_bool(p, "cancel")? {
        s.view_state.drag.remove(&joint);
        s.rebuild();
        s.refresh_overlays();
        return Ok(json!({ "joint": joint, "value": shown(j.kind, j.value), "cancelled": true }));
    }
    if j.kind == JointKind::Rigid {
        return Err(bad("a rigid joint does not move"));
    }
    let (w, h) = params::viewport(p)?;
    let point = |key: &str| -> Result<(f64, f64), CommandError> {
        let v = p
            .get(key)
            .filter(|v| v.is_object())
            .ok_or_else(|| bad(format!("`{key}` is required: {{x, y}} in pixels")))?;
        let (x, y) = (params::req_f64(v, "x")?, params::req_f64(v, "y")?);
        if !(-w..=2.0 * w).contains(&x) || !(-h..=2.0 * h).contains(&y) {
            return Err(bad(format!("`{key}` must be near the viewport")));
        }
        Ok((x, y))
    };
    let (from, to) = (point("from")?, point("to")?);
    let start = match p.get("start") {
        None | Some(Value::Null) => j.value,
        v => joint_value(s, j.kind, v, "`start`")?,
    };
    // Measure the drag against the joint as it was when the drag began.
    s.view_state.drag.insert(joint, start);
    s.rebuild();
    let delta = s.joint_drag_delta(joint, from, to, w, h).ok_or_else(|| {
        CommandError::Unavailable(
            "the joint's axis cannot be dragged from this view (it points at the screen, \
             or its origin is gone)"
                .into(),
        )
    })?;
    let mut value = start + delta;
    let mut clamped = false;
    if let Some([lo, hi]) = j.limits {
        let c = value.clamp(lo, hi);
        clamped = c != value;
        value = c;
    }
    if !value.is_finite() || j.check_value(value).is_err() {
        value = start;
    }
    s.view_state.drag.insert(joint, value);
    s.rebuild();
    s.refresh_overlays();
    Ok(json!({
        "joint": joint,
        "value": shown(j.kind, value),
        "start": shown(j.kind, start),
        "clamped": clamped,
    }))
}

/// A list of component ids or body names from `key`, if given.
fn id_list(p: &Value, key: &str) -> Result<Option<Vec<u32>>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() <= MAX_INTERFERENCE_BODIES => a
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| bad(format!("`{key}` must list component ids")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(bad(format!(
            "`{key}` must be a list of at most {MAX_INTERFERENCE_BODIES} component ids"
        ))),
    }
}

fn name_list(p: &Value, key: &str) -> Result<Option<Vec<String>>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() <= MAX_INTERFERENCE_BODIES => a
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|n| n.chars().count() <= 256)
                    .map(str::to_owned)
                    .ok_or_else(|| bad(format!("`{key}` must list body names")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(bad(format!(
            "`{key}` must be a list of at most {MAX_INTERFERENCE_BODIES} body names"
        ))),
    }
}

fn check_now(
    s: &Session,
    components: Option<&[u32]>,
    bodies: Option<&[String]>,
) -> Result<InterferenceReport, CommandError> {
    let list = s.interference_bodies(components, bodies);
    if list.len() > MAX_INTERFERENCE_BODIES {
        return Err(bad(format!(
            "an interference check compares at most {MAX_INTERFERENCE_BODIES} bodies: \
             name some `components` or `bodies`"
        )));
    }
    Ok(crate::motion::interference(&list))
}

fn report_json(r: &InterferenceReport) -> Value {
    json!({
        "count": r.pairs.len(),
        "pairs": r.pairs.iter().map(|x| json!({
            "a": x.a,
            "b": x.b,
            "a_component": x.a_component,
            "b_component": x.b_component,
            "volume": x.volume,
            "bounds": x.overlap.bounding_box(),
        })).collect::<Vec<_>>(),
        "bodies": r.bodies,
        "candidates": r.candidates,
        "errors": r.errors.iter().map(|(a, b, e)| json!({"a": a, "b": b, "error": e})).collect::<Vec<_>>(),
    })
}

/// `inspect.interference`: pairs of bodies that overlap, with the overlap volume, drawn
/// highlighted. With `at_keys`, also at each motion keyframe.
pub(crate) fn interference(s: &mut Session, p: &Value) -> CmdResult {
    if p.get("clear").is_some_and(|v| !v.is_null()) && params::req_bool(p, "clear")? {
        s.view_state.overlaps.clear();
        s.refresh_overlays();
        return Ok(json!({ "count": 0, "pairs": [], "cleared": true }));
    }
    let components = id_list(p, "components")?;
    let bodies = name_list(p, "bodies")?;
    let at_keys = match p.get("at_keys") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "at_keys")?,
    };
    let mut per_key = Vec::new();
    if at_keys {
        let saved = (s.view_state.motion_time, s.view_state.drag.clone());
        let mut failed = None;
        for t in s.doc.motion.times() {
            s.view_state.motion_time = Some(t);
            s.view_state.drag.clear();
            s.rebuild();
            match check_now(s, components.as_deref(), bodies.as_deref()) {
                Ok(r) => {
                    let mut v = report_json(&r);
                    v["time"] = json!(t);
                    per_key.push(v);
                }
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        (s.view_state.motion_time, s.view_state.drag) = saved;
        s.rebuild();
        if let Some(e) = failed {
            s.refresh_overlays();
            return Err(e);
        }
    }
    let now = check_now(s, components.as_deref(), bodies.as_deref())?;
    s.view_state.overlaps = now
        .pairs
        .iter()
        .filter_map(|x| crate::motion::overlap_shape(&x.overlap))
        .collect();
    s.refresh_overlays();
    let mut out = report_json(&now);
    if at_keys {
        out["keys"] = json!(per_key);
    }
    Ok(out)
}

fn explode_json(s: &Session) -> Value {
    let comps: Vec<Value> = s
        .exploded()
        .iter()
        .map(|(c, e)| {
            json!({
                "component": c,
                "name": s.doc.assembly.name_of(*c),
                "centre": e.centre,
                "offset": e.offset,
                "distance": e.offset.length(),
            })
        })
        .collect();
    json!({
        "factor": s.view_state.explode_factor,
        "components": comps,
    })
}

/// `view.explode`: shows the components moved apart, without changing the model.
pub(crate) fn explode(s: &mut Session, p: &Value) -> CmdResult {
    let factor = params::opt_f64(p, "factor")?;
    if let Some(f) = factor
        && !(1.0..=MAX_EXPLODE_FACTOR).contains(&f)
    {
        return Err(bad(format!(
            "`factor` must be from 1 (assembled) to {MAX_EXPLODE_FACTOR}"
        )));
    }
    let manual = match p.get("component") {
        None | Some(Value::Null) => None,
        Some(_) => {
            let c = super::component::component_id(s, p, "component")?;
            let o = xyz(
                p.get("offset")
                    .ok_or_else(|| bad("`offset` {x, y, z} is required with a `component`"))?,
                "`offset`",
            )?;
            Some((c, o))
        }
    };
    if factor.is_none() && manual.is_none() {
        return Err(bad(
            "give a `factor`, or a `component` and its `offset`, to explode",
        ));
    }
    if let Some(f) = factor {
        s.view_state.explode_factor = Some(f);
    }
    if let Some((c, o)) = manual {
        if o.length() > chamfera_geom::LINEAR_TOLERANCE {
            s.view_state.explode_manual.insert(c, o);
        } else {
            s.view_state.explode_manual.remove(&c);
        }
    }
    s.rebuild();
    s.refresh_overlays();
    Ok(explode_json(s))
}

/// `view.explode_clear`: shows the assembly as it is again.
pub(crate) fn explode_clear(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.view_state.explode_factor = None;
    s.view_state.explode_manual.clear();
    s.rebuild();
    s.refresh_overlays();
    Ok(explode_json(s))
}

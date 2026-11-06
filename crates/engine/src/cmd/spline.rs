//! `sketch.spline`: a spline through (fit) or guided by (control) points in the active
//! sketch (rung K10; `chamfera_sketch::spline`). Its points are ordinary sketch points,
//! so `sketch.constrain` and `sketch.drag` on them shape it.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::Point2;
use chamfera_sketch::{MAX_SPLINE_POINTS, PointId, Sketch, SplineForm};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "sketch.spline",
    label: "Spline",
    params: "points: array (required) - in order, each {x, y} for a new point or {point: id} for an \
             existing one; 2 or more (3 or more when closed)\n\
             kind: string (optional) - fit (default: through the points) or control (the points \
             are the control polygon)\n\
             closed: boolean (optional) - close the spline on itself (default false)",
    mutates: true,
    run: spline,
}];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn unavailable(e: impl std::fmt::Display) -> CommandError {
    CommandError::Unavailable(e.to_string())
}

/// One entry of `points`: a new point at `(x, y)` or an existing point.
enum Entry {
    At(Point2),
    Existing(PointId),
}

fn entry(v: &Value, i: usize) -> Result<Entry, CommandError> {
    let Some(obj) = v.as_object() else {
        return Err(bad(format!(
            "`points[{i}]` must be an object {{x, y}} or {{point}}"
        )));
    };
    if obj.contains_key("point") {
        if obj.len() != 1 {
            return Err(bad(format!(
                "`points[{i}]` names an existing point and must have no other keys"
            )));
        }
        return Ok(Entry::Existing(PointId(params::req_id(v, "point")?)));
    }
    if obj.keys().any(|k| k != "x" && k != "y") {
        return Err(bad(format!("`points[{i}]` may only have `x` and `y`")));
    }
    let (x, y) = (params::req_f64(v, "x")?, params::req_f64(v, "y")?);
    let at = Point2::new(x, y);
    if !at.is_valid() {
        return Err(bad(format!("`points[{i}]` is outside the working range")));
    }
    Ok(Entry::At(at))
}

fn spline(s: &mut Session, p: &Value) -> CmdResult {
    let list = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`points` must be an array of {x, y} or {point}"))?;
    if list.len() > MAX_SPLINE_POINTS {
        return Err(bad(format!(
            "`points` may hold at most {MAX_SPLINE_POINTS} points"
        )));
    }
    let entries = list
        .iter()
        .enumerate()
        .map(|(i, v)| entry(v, i))
        .collect::<Result<Vec<_>, _>>()?;
    let form = match params::opt_str(p, "kind")? {
        None | Some("fit") => SplineForm::Fit,
        Some("control") => SplineForm::Control,
        Some(other) => {
            return Err(bad(format!("unknown `kind` {other:?}: use fit or control")));
        }
    };
    let closed = match p.get("closed") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "closed")?,
    };
    let sketch = active(s)?;
    let mut ids = Vec::with_capacity(entries.len());
    for e in entries {
        let id = match e {
            Entry::At(at) => sketch.add_point(at).map_err(unavailable)?,
            Entry::Existing(id) => {
                if sketch.point(id).is_none() {
                    return Err(bad(format!("no {id} in the sketch")));
                }
                id
            }
        };
        ids.push(id);
    }
    let curve = sketch
        .add_spline(ids.clone(), form, closed)
        .map_err(unavailable)?;
    if sketch.spline_curve(curve).is_none() {
        return Err(bad(
            "the points do not make a spline: two in a row are at the same place",
        ));
    }
    sketch.solve().map_err(unavailable)?;
    Ok(json!({
        "curve": curve.0,
        "points": ids.iter().map(|p| p.0).collect::<Vec<_>>(),
        "kind": form.name(),
        "closed": closed,
    }))
}

fn active(s: &mut Session) -> Result<&mut Sketch, CommandError> {
    let id = s.active_sketch().map(|f| f.id).ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Create Sketch or Edit Sketch".into())
    })?;
    Ok(&mut s.doc.sketch_mut(id)?.sketch)
}

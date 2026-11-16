//! More sketch commands: copy and paste of sketch geometry (`chamfera_sketch::clipboard`),
//! intersection curves (`crate::intersection`), spline tangent handles, polygons by an
//! edge and arc slots.

use super::CommandSpec;
use super::sketch::{active, active_id, bad, count, pt, run_tool, sketch_err, targets};
use crate::sketching::SketchItem;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_sketch::{CurveId, ToolOp};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sketch.copy",
        label: "Copy Sketch Geometry",
        params: "curves, points: array of integers (optional) - what to copy; default the \
                 selection. The curves' points come along, and the constraints among what is \
                 copied. The copy is kept for sketch.paste (in this session, not the design)",
        mutates: false,
        run: copy,
    },
    CommandSpec {
        id: "sketch.intersect",
        label: "Intersect",
        params: "bodies: list of strings (optional) - bodies by name; default the selected \
                 bodies, else every body the sketch plane cuts\n\
                 linked: boolean (optional) - follow the bodies when the model rebuilds \
                 (default true)",
        mutates: true,
        run: intersect,
    },
    CommandSpec {
        id: "sketch.paste",
        label: "Paste Sketch Geometry",
        params: "dx, dy: number (optional) - move the copy by this much (mm); default 0\n\
                 x, y: number (optional) - instead, put the middle of the copy here\n\
                 Pastes what sketch.copy kept into the active sketch, with its constraints, and \
                 selects it",
        mutates: true,
        run: paste,
    },
    CommandSpec {
        id: "sketch.polygon.edge",
        label: "Edge Polygon",
        params: "x1, y1, x2, y2: number (required) - the ends of one side; the polygon lies \
                 to its left\n\
                 sides: integer (required) - 3 to 64",
        mutates: true,
        run: polygon_edge,
    },
    CommandSpec {
        id: "sketch.slot.arc",
        label: "Arc Slot",
        params: "cx, cy: number (required) - the centre of the arc\n\
                 x1, y1: number (required) - the start of the centre line (sets the radius)\n\
                 x2, y2: number (required) - the direction of its end, counter-clockwise from \
                 the start\n\
                 width: number (required) - mm, less than twice the radius",
        mutates: true,
        run: arc_slot,
    },
    CommandSpec {
        id: "sketch.spline.handles",
        label: "Spline Tangent Handles",
        params: "curve: integer (required) - an open fit spline\n\
                 start, end: boolean (optional) - give that end a handle (default both); \
                 the spline leaves its first point toward the start handle and reaches its \
                 last point heading away from the end handle\n\
                 remove: boolean (optional) - take the handles away instead",
        mutates: true,
        run: spline_handles,
    },
];

fn opt_bool(p: &Value, key: &str, default: bool) -> Result<bool, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => params::req_bool(p, key),
    }
}

fn opt_f64(p: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_f64(p, key).map(Some),
    }
}

fn copy(s: &mut Session, p: &Value) -> CmdResult {
    let (curves, points) = targets(s, p)?;
    let f = s.active_sketch().ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Edit Sketch first".into())
    })?;
    let clip = f.sketch.extract(&curves, &points).map_err(sketch_err)?;
    let reply = json!({
        "curves": clip.curves().count(),
        "points": clip.points().count(),
        "constraints": clip.constraints().count(),
    });
    s.sketch_clipboard = Some(clip);
    Ok(reply)
}

fn paste(s: &mut Session, p: &Value) -> CmdResult {
    let clip = s.sketch_clipboard.clone().ok_or_else(|| {
        CommandError::Unavailable("there is nothing to paste: copy some sketch geometry".into())
    })?;
    let (dx, dy) = match (opt_f64(p, "x")?, opt_f64(p, "y")?) {
        (Some(x), Some(y)) => {
            let c = clip
                .centre()
                .ok_or_else(|| CommandError::Unavailable("the copy is empty".into()))?;
            (x - c.x, y - c.y)
        }
        (None, None) => (
            opt_f64(p, "dx")?.unwrap_or(0.0),
            opt_f64(p, "dy")?.unwrap_or(0.0),
        ),
        _ => return Err(bad("give both `x` and `y`, or neither")),
    };
    if !(dx.is_finite() && dy.is_finite()) {
        return Err(bad("the offset must be finite numbers"));
    }
    let id = active_id(s)?;
    let r = s
        .doc
        .sketch_mut(id)?
        .sketch
        .paste(&clip, dx, dy)
        .map_err(sketch_err)?;
    s.sketch_selection = r
        .created_curves
        .iter()
        .map(|c| SketchItem::Curve(c.0))
        .collect();
    let profiles: Vec<f64> = s
        .doc
        .sketch(id)?
        .sketch
        .profiles()
        .iter()
        .map(|r| r.area)
        .collect();
    Ok(json!({
        "created_curves": r.created_curves.iter().map(|c| c.0).collect::<Vec<_>>(),
        "created_points": r.created_points.iter().map(|c| c.0).collect::<Vec<_>>(),
        "created_constraints": r.created_constraints.iter().map(|c| c.0).collect::<Vec<_>>(),
        "profiles": profiles,
    }))
}

fn intersect(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = active_id(s)?;
    let linked = opt_bool(p, "linked", true)?;
    let names: Vec<String> = match p.get("bodies") {
        None | Some(Value::Null) => {
            let picked: Vec<String> = s
                .scene
                .selected_bodies()
                .into_iter()
                .filter_map(|b| {
                    s.solids()
                        .iter()
                        .find(|x| x.scene_id == Some(b))
                        .map(|x| x.name.clone())
                })
                .collect();
            if picked.is_empty() {
                let plane = s.doc.sketch(sketch)?.plane;
                s.solids()
                    .iter()
                    .filter(|x| crate::intersection::section_curves(&x.body, &plane).is_ok())
                    .map(|x| x.name.clone())
                    .collect()
            } else {
                picked
            }
        }
        Some(Value::Array(list)) => {
            if list.len() > chamfera_doc::MAX_INTERSECTIONS {
                return Err(bad("`bodies` names too many bodies"));
            }
            list.iter()
                .map(|v| {
                    v.as_str()
                        .filter(|n| !n.is_empty() && n.len() <= 512)
                        .map(str::to_owned)
                        .ok_or_else(|| bad("`bodies` must be a list of body names"))
                })
                .collect::<Result<_, _>>()?
        }
        Some(_) => return Err(bad("`bodies` must be a list of body names")),
    };
    if names.is_empty() {
        return Err(CommandError::Unavailable(
            "the sketch plane cuts no body".into(),
        ));
    }
    let made = s
        .intersect_bodies(sketch, &names, linked)
        .map_err(CommandError::Unavailable)?;
    let mut reply = Vec::new();
    for (body, curves) in &made {
        let f = s.doc.sketch(sketch)?;
        let kinds: Vec<Value> = curves
            .iter()
            .filter_map(|c| f.sketch.curve(CurveId(*c)))
            .map(|c| json!(c.kind_name()))
            .collect();
        let radii: Vec<Value> = curves
            .iter()
            .map(|c| json!(f.sketch.radius(CurveId(*c))))
            .collect();
        reply.push(json!({"body": body, "curves": curves, "kinds": kinds, "radii": radii}));
    }
    Ok(json!({ "intersections": reply, "linked": linked }))
}

fn polygon_edge(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    let sides = count(p, "sides", chamfera_sketch::MAX_POLYGON_SIDES)?;
    run_tool(s, ToolOp::PolygonEdge { a, b, sides })
}

fn arc_slot(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let start = pt(p, "x1", "y1")?;
    let end = pt(p, "x2", "y2")?;
    let width = params::req_f64(p, "width")?;
    run_tool(
        s,
        ToolOp::ArcSlot {
            centre,
            start,
            end,
            width,
        },
    )
}

fn spline_handles(s: &mut Session, p: &Value) -> CmdResult {
    let curve = CurveId(params::req_id(p, "curve")?);
    let remove = opt_bool(p, "remove", false)?;
    let start = opt_bool(p, "start", true)?;
    let end = opt_bool(p, "end", true)?;
    let sk = active(s)?;
    if remove {
        sk.remove_spline_handles(curve).map_err(sketch_err)?;
        sk.solve().map_err(sketch_err)?;
        return Ok(json!({ "curve": curve.0, "start_handle": null, "end_handle": null }));
    }
    let (a, b) = sk
        .add_spline_handles(curve, start, end)
        .map_err(sketch_err)?;
    sk.solve().map_err(sketch_err)?;
    Ok(json!({
        "curve": curve.0,
        "start_handle": a.map(|x| x.0),
        "end_handle": b.map(|x| x.0),
    }))
}

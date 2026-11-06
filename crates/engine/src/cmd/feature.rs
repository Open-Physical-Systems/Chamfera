//! `solid.*` and `feature.*`: features on the timeline that build solids.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{Blend, BlendKind, Extrude, ExtrudeDirection, FeatureKind, Operation, Revolve};
use chamfera_geom::Point2;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "feature.delete",
        label: "Delete Feature",
        params: "feature: integer (required)",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "feature.edit",
        label: "Edit Feature",
        params: "feature: integer (required)\n\
                 distance: string or number (optional) - new extrude distance expression\n\
                 direction: string (optional) - normal, reversed or symmetric\n\
                 operation: string (optional) - new_body, join, cut or intersect\n\
                 size: string or number (optional) - new fillet radius or chamfer distance\n\
                 plane: string or object (optional) - a mirror's new plane, as in solid.mirror\n\
                 dx, dy, dz, angle, axis, origin, from, to (optional) - a move's new transform\n\
                 copy: boolean (optional) - whether a move keeps the originals\n\
                 suppressed: boolean (optional)",
        mutates: true,
        run: edit,
    },
    CommandSpec {
        id: "feature.list",
        label: "List Features",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "solid.chamfer",
        label: "Chamfer",
        params: "edges: list of strings (optional) - edge names from solid.edges or solid.edge_at\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, instead\n\
                 distance: string or number (required) - such as 1 or \"1 mm\"",
        mutates: true,
        run: chamfer,
    },
    CommandSpec {
        id: "solid.edge_at",
        label: "Edge Under Cursor",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: edge_at,
    },
    CommandSpec {
        id: "solid.edges",
        label: "List Edges",
        params: "(none)",
        mutates: false,
        run: edges,
    },
    CommandSpec {
        id: "solid.extrude",
        label: "Extrude",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (required) - a point inside each profile, in sketch mm\n\
                 distance: string or number (required) - such as 10, \"10 mm\" or \"thickness * 2\"\n\
                 direction: string (optional) - normal (default), reversed or symmetric\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: extrude,
    },
    CommandSpec {
        id: "solid.revolve",
        label: "Revolve",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (required) - a point inside each profile, in sketch mm\n\
                 axis: integer (required) - a line of the same sketch to turn around\n\
                 angle: string or number (optional) - default 360 deg; a plain number is degrees\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: revolve,
    },
    CommandSpec {
        id: "solid.fillet",
        label: "Fillet",
        params: "edges: list of strings (optional) - edge names from solid.edges or solid.edge_at\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, instead\n\
                 radius: string or number (required) - such as 2 or \"2 mm\"",
        mutates: true,
        run: fillet,
    },
    CommandSpec {
        id: "solid.pattern_circular",
        label: "Circular Pattern",
        params: "feature: integer (required) - the feature to repeat\n\
                 count: string or number (required) - copies including the original, 2 to 1000\n\
                 origin: {x, y, z} (required) - a point on the axis\n\
                 axis: {x, y, z} (required) - the axis direction\n\
                 angle: string or number (optional) - spread, default 360 deg (plain = degrees)",
        mutates: true,
        run: pattern_circular,
    },
    CommandSpec {
        id: "solid.pattern_linear",
        label: "Rectangular Pattern",
        params: "feature: integer (required) - the feature to repeat\n\
                 count: string or number (required) - copies including the original, 2 to 1000\n\
                 direction: {x, y, z} (required) - which way the copies go\n\
                 spacing: string or number (required) - distance between copies",
        mutates: true,
        run: pattern_linear,
    },
    CommandSpec {
        id: "solid.profile_at",
        label: "Profile Under Cursor",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: profile_at,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// A distance given as a number (mm) or an expression string.
fn distance(p: &Value) -> Result<Option<String>, CommandError> {
    match p.get("distance") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => Ok(Some(format!("{} mm", params::req_f64(p, "distance")?))),
        Some(_) => Err(bad("`distance` must be a number or an expression")),
    }
}

fn direction(p: &Value) -> Result<Option<ExtrudeDirection>, CommandError> {
    Ok(match params::opt_str(p, "direction")? {
        None => None,
        Some("normal") => Some(ExtrudeDirection::Normal),
        Some("reversed") => Some(ExtrudeDirection::Reversed),
        Some("symmetric") => Some(ExtrudeDirection::Symmetric),
        Some(o) => {
            return Err(bad(format!(
                "unknown direction {o:?}: use normal, reversed or symmetric"
            )));
        }
    })
}

pub(super) fn operation(p: &Value) -> Result<Option<Operation>, CommandError> {
    match params::opt_str(p, "operation")? {
        None => Ok(None),
        Some(o) => Operation::parse(o).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown operation {o:?}: use new_body, join, cut or intersect"
            ))
        }),
    }
}

fn seeds(p: &Value) -> Result<Vec<Point2>, CommandError> {
    let list = p
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`profiles` must be a list of {x, y}"))?;
    if list.len() > chamfera_doc::feature::MAX_FEATURE_PROFILES {
        return Err(bad("too many profiles"));
    }
    list.iter()
        .map(|v| {
            let x = params::req_f64(v, "x")?;
            let y = params::req_f64(v, "y")?;
            let pt = Point2::new(x, y);
            if !pt.is_valid() {
                return Err(bad("a profile point is outside the working range"));
            }
            Ok(pt)
        })
        .collect()
}

/// The reply for a feature: its solids' exact volumes from the kernel, and any error.
pub(super) fn feature_reply(s: &Session, id: u32) -> Value {
    let solids: Vec<Value> = s
        .solids()
        .iter()
        .filter(|x| x.feature == id || crate::solids::body_has_feature(&x.body, id))
        .map(|x| {
            json!({
                "name": x.name,
                "body": x.scene_id,
                "volume": x.body.volume().ok(),
                "area": x.body.area().ok(),
            })
        })
        .collect();
    let f = s.doc.feature(id).ok();
    json!({
        "feature": id,
        "name": f.map(|f| f.name.clone()),
        "definition": f.map(|f| &f.kind),
        "suppressed": f.map(|f| f.suppressed),
        "solids": solids,
        "error": s.feature_errors().get(&id),
        "display_error": s.display_errors().get(&id),
    })
}

fn extrude(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = params::req_id(p, "sketch")?;
    let distance = distance(p)?.ok_or_else(|| bad("`distance` is required"))?;
    let kind = FeatureKind::Extrude(Extrude {
        sketch,
        profiles: seeds(p)?,
        distance,
        direction: direction(p)?.unwrap_or_default(),
        operation: operation(p)?.unwrap_or_default(),
    });
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn revolve(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = params::req_id(p, "sketch")?;
    let axis = params::req_id(p, "axis")?;
    let angle = match p.get("angle") {
        None | Some(Value::Null) => "360 deg".to_owned(),
        Some(Value::String(a)) => a.clone(),
        Some(Value::Number(_)) => format!("{} deg", params::req_f64(p, "angle")?),
        Some(_) => return Err(bad("`angle` must be a number or an expression")),
    };
    let kind = FeatureKind::Revolve(Revolve {
        sketch,
        profiles: seeds(p)?,
        axis,
        angle,
        operation: operation(p)?.unwrap_or_default(),
    });
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

/// The edges a fillet or chamfer names: a list, or the one under a pixel.
fn blend_edges(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    match (p.get("edges"), p.get("pick")) {
        (Some(Value::Array(list)), None | Some(Value::Null)) => list
            .iter()
            .map(|e| {
                e.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad("`edges` must be a list of edge names"))
            })
            .collect(),
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y) = (params::req_f64(pick, "x")?, params::req_f64(pick, "y")?);
            let (w, h) = (
                params::req_f64(pick, "width")?,
                params::req_f64(pick, "height")?,
            );
            s.edge_at(x, y, w, h)
                .map(|(_, _, r)| vec![r])
                .ok_or_else(|| {
                    CommandError::Unavailable("there is no edge under that pixel".into())
                })
        }
        (None | Some(Value::Null), None) => Err(bad("give `edges` or `pick`")),
        _ => Err(bad("give either `edges` (a list) or `pick`, not both")),
    }
}

fn blend(s: &mut Session, p: &Value, kind: BlendKind, key: &str) -> CmdResult {
    let edges = blend_edges(s, p)?;
    let size = expression(p, key, "mm")?.ok_or_else(|| bad(format!("`{key}` is required")))?;
    let id = s.doc.add_feature(FeatureKind::Blend(Blend {
        blend: kind,
        edges,
        size,
    }))?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn fillet(s: &mut Session, p: &Value) -> CmdResult {
    blend(s, p, BlendKind::Fillet, "radius")
}

fn chamfer(s: &mut Session, p: &Value) -> CmdResult {
    blend(s, p, BlendKind::Chamfer, "distance")
}

fn edge_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y) = (params::req_f64(p, "x")?, params::req_f64(p, "y")?);
    let (w, h) = (params::req_f64(p, "width")?, params::req_f64(p, "height")?);
    Ok(match s.edge_at(x, y, w, h) {
        Some((si, _, r)) => json!({
            "edge": r,
            "solid": s.solids().get(si).map(|x| x.name.clone()),
            "body": s.solids().get(si).and_then(|x| x.scene_id),
        }),
        None => json!({ "edge": null }),
    })
}

/// Every edge of every solid, by name, with its end points.
fn edges(s: &mut Session, _p: &Value) -> CmdResult {
    let pt = |q: chamfera_geom::Point3| json!([q.x, q.y, q.z]);
    let solids: Vec<Value> = s
        .solids()
        .iter()
        .map(|x| {
            let edges: Vec<Value> = x
                .body
                .edges()
                .into_iter()
                .filter_map(|e| {
                    let name = crate::solids::edge_ref(&x.body, e)?;
                    let [a, b] = x.body.edge_vertices(e).ok()?;
                    Some(json!({
                        "edge": name,
                        "from": pt(x.body.vertex_point(a).ok()?),
                        "to": pt(x.body.vertex_point(b).ok()?),
                    }))
                })
                .collect();
            json!({ "solid": x.name, "body": x.scene_id, "edges": edges })
        })
        .collect();
    Ok(json!({ "solids": solids }))
}

pub(super) fn expression(p: &Value, key: &str, unit: &str) -> Result<Option<String>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => {
            let v = params::req_f64(p, key)?;
            Ok(Some(if unit.is_empty() {
                format!("{v}")
            } else {
                format!("{v} {unit}")
            }))
        }
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

pub(super) fn vec3(p: &Value, key: &str) -> Result<chamfera_geom::Vec3, CommandError> {
    let v = p
        .get(key)
        .ok_or_else(|| bad(format!("`{key}` is required")))?;
    Ok(chamfera_geom::Vec3::new(
        params::req_f64(v, "x")?,
        params::req_f64(v, "y")?,
        params::req_f64(v, "z")?,
    ))
}

fn add_pattern(s: &mut Session, pattern: chamfera_doc::Pattern) -> CmdResult {
    let id = s.doc.add_feature(FeatureKind::Pattern(pattern))?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn pattern_linear(s: &mut Session, p: &Value) -> CmdResult {
    let source = params::req_id(p, "feature")?;
    let count = expression(p, "count", "")?.ok_or_else(|| bad("`count` is required"))?;
    let spacing = expression(p, "spacing", "mm")?.ok_or_else(|| bad("`spacing` is required"))?;
    let direction = vec3(p, "direction")?;
    add_pattern(
        s,
        chamfera_doc::Pattern {
            source,
            count,
            layout: chamfera_doc::PatternLayout::Linear { direction, spacing },
        },
    )
}

fn pattern_circular(s: &mut Session, p: &Value) -> CmdResult {
    let source = params::req_id(p, "feature")?;
    let count = expression(p, "count", "")?.ok_or_else(|| bad("`count` is required"))?;
    let angle = expression(p, "angle", "deg")?.unwrap_or_else(|| "360 deg".to_owned());
    let o = vec3(p, "origin")?;
    let axis = vec3(p, "axis")?;
    add_pattern(
        s,
        chamfera_doc::Pattern {
            source,
            count,
            layout: chamfera_doc::PatternLayout::Circular {
                origin: chamfera_geom::Point3::new(o.x, o.y, o.z),
                axis,
                angle,
            },
        },
    )
}

fn edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "feature")?;
    let new_distance = distance(p)?;
    let new_direction = direction(p)?;
    let new_operation = operation(p)?;
    let suppressed = match p.get("suppressed") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "suppressed")?),
    };
    let mut kind = s.doc.feature(id)?.kind.clone();
    if let Some(o) = new_operation {
        match &mut kind {
            FeatureKind::Extrude(e) => e.operation = o,
            FeatureKind::Revolve(r) => r.operation = o,
            FeatureKind::Pattern(_) => {
                return Err(bad("a pattern does what the feature it repeats does"));
            }
            FeatureKind::Blend(_) => {
                return Err(bad("a fillet or chamfer has no operation"));
            }
            FeatureKind::Mirror(m) => m.operation = o,
            FeatureKind::Move(_) => return Err(bad("a move has no operation")),
        }
    }
    super::transform::edit_transform(&mut kind, p)?;
    let new_size = expression(p, "size", "mm")?;
    if let Some(size) = new_size {
        let FeatureKind::Blend(b) = &mut kind else {
            return Err(bad("size belongs to fillets and chamfers"));
        };
        b.size = size;
    }
    if let FeatureKind::Extrude(e) = &mut kind {
        if let Some(d) = new_distance {
            e.distance = d;
        }
        if let Some(d) = new_direction {
            e.direction = d;
        }
    } else if new_distance.is_some() || new_direction.is_some() {
        return Err(bad("distance and direction belong to extrudes"));
    }
    s.doc.check_feature(&kind)?;
    let f = s.doc.feature_mut(id)?;
    f.kind = kind;
    if let Some(v) = suppressed {
        f.suppressed = v;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "feature")?;
    let users = s.doc.feature_users(id);
    if !users.is_empty() {
        return Err(CommandError::Unavailable(format!(
            "{} is drawn on this feature's faces; delete it first",
            users.join(", ")
        )));
    }
    s.doc.remove_feature(id)?;
    s.rebuild();
    Ok(json!({ "deleted": id }))
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let ids: Vec<u32> = s.doc.features.iter().map(|f| f.id).collect();
    let features: Vec<Value> = ids.into_iter().map(|id| feature_reply(s, id)).collect();
    Ok(json!({ "features": features }))
}

fn profile_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    Ok(match s.profile_at(x, y, w, h) {
        Some((sketch, at, area)) => json!({
            "sketch": sketch, "x": at.x, "y": at.y, "area": area,
        }),
        None => json!({ "sketch": null }),
    })
}

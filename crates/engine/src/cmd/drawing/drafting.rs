//! `drawing.layer.*`, `drawing.draw.*` and `drawing.entity.*`: 2D drafting on a sheet.
//!
//! Each sheet holds user layers (colour, line type, line weight, on/off, frozen, locked),
//! a current layer, a line type scale, and lines, circles, arcs and polylines drawn on
//! those layers (`chamfera_doc::drafting`). Grips (`drawing.entity.grip`) move one point of
//! an entity exactly. Entities on a locked layer refuse every edit; a frozen layer cannot
//! be current. Positions are sheet mm, angles radians.

use super::{bad, coordinate, sheet_id};
use crate::cmd::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::drafting::{
    self as df, DEFAULT_LAYER, DraftLayer, Drafting, Entity, EntityKind, LineType,
};
use chamfera_drawing::drafting as dr;
use chamfera_drawing::linetype;
use chamfera_geom::{Point2, Vec2};
use serde_json::{Value, json};

/// Most entities one command moves, deletes or changes.
const MAX_SELECTION: usize = 10_000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "drawing.draw.arc",
        label: "Sheet Arc",
        params: "cx, cy: number (required) - the centre, sheet mm\n\
                 radius: number (required) - mm, above 0\n\
                 start, end: number (required) - radians, counter-clockwise from start to end, \
                 at most a full turn\n\
                 layer: string (optional) - default the current layer\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: draw_arc,
    },
    CommandSpec {
        id: "drawing.draw.circle",
        label: "Sheet Circle",
        params: "cx, cy: number (required) - the centre, sheet mm\n\
                 radius: number (required) - mm, above 0\n\
                 layer: string (optional) - default the current layer\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: draw_circle,
    },
    CommandSpec {
        id: "drawing.draw.line",
        label: "Sheet Line",
        params: "x1, y1, x2, y2: number (required) - the two ends, sheet mm from the lower-left \
                 corner\n\
                 layer: string (optional) - default the current layer\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: draw_line,
    },
    CommandSpec {
        id: "drawing.draw.polyline",
        label: "Sheet Polyline",
        params: "points: [{x, y}] (required) - 2 to 2000 sheet points (3 when closed)\n\
                 closed: bool (optional) - join the last point back to the first\n\
                 layer: string (optional) - default the current layer\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: draw_polyline,
    },
    CommandSpec {
        id: "drawing.draw.rectangle",
        label: "Sheet Rectangle",
        params: "x1, y1, x2, y2: number (required) - two opposite corners, sheet mm\n\
                 layer: string (optional) - default the current layer\n\
                 sheet: id (optional) - default the last sheet\n\
                 Makes a closed polyline of four points",
        mutates: true,
        run: draw_rectangle,
    },
    CommandSpec {
        id: "drawing.entity.delete",
        label: "Delete Sheet Entities",
        params: "entities: [id] (required) - drafted entities, none on a locked layer",
        mutates: true,
        run: entity_delete,
    },
    CommandSpec {
        id: "drawing.entity.grip",
        label: "Drag Grip",
        params: "entity: id (required) - a drafted entity, not on a locked layer\n\
                 grip: integer (required) - which grip, as drawing.entity.grips lists them \
                 (line: start, end, middle; circle: centre, quadrants at 0, 90, 180, 270 \
                 degrees; arc: centre, start, end, middle; polyline: vertices, then segment \
                 middles)\n\
                 x, y: number (required) - where the grip goes, sheet mm",
        mutates: true,
        run: entity_grip,
    },
    CommandSpec {
        id: "drawing.entity.grips",
        label: "Entity Grips",
        params: "entity: id (required) - a drafted entity\n\
                 Replies with its grips in order: kind and position",
        mutates: false,
        run: entity_grips,
    },
    CommandSpec {
        id: "drawing.entity.layer",
        label: "Change Layer",
        params: "entities: [id] (required) - drafted entities, none on a locked layer\n\
                 layer: string (required) - the layer to put them on (on their sheet)",
        mutates: true,
        run: entity_layer,
    },
    CommandSpec {
        id: "drawing.entity.move",
        label: "Move Sheet Entities",
        params: "entities: [id] (required) - drafted entities, none on a locked layer\n\
                 dx, dy: number (required) - sheet mm\n\
                 copy: bool (optional) - move copies and keep the originals",
        mutates: true,
        run: entity_move,
    },
    CommandSpec {
        id: "drawing.entity.pick",
        label: "Pick Sheet Entity",
        params: "x, y: number (required) - a sheet point, mm\n\
                 tolerance: number (optional) - mm, 0 to 50; default 2\n\
                 sheet: id (optional) - default the last sheet\n\
                 Replies with the nearest drafted entity shown and not locked, or null",
        mutates: false,
        run: entity_pick,
    },
    CommandSpec {
        id: "drawing.layer.add",
        label: "New Layer",
        params: "name: string (required) - 1 to 31 letters, digits, _, - or $; not VISIBLE, \
                 HIDDEN, CENTER, HATCH, DIM, DANGLING or BORDER\n\
                 colour: string (optional) - #rrggbb, or ink (the sheet's line colour); \
                 default ink\n\
                 line_type: string (optional) - continuous, dashed, hidden, center, phantom or \
                 dot; default continuous\n\
                 weight: number (optional) - plotted width in mm, 0.05 to 2.5; default 0.25\n\
                 current: bool (optional) - make it the current layer\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: layer_add,
    },
    CommandSpec {
        id: "drawing.layer.current",
        label: "Set Current Layer",
        params: "layer: string (required) - a layer that is not frozen; new entities go on it\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: layer_current,
    },
    CommandSpec {
        id: "drawing.layer.delete",
        label: "Delete Layer",
        params: "layer: string (required) - an empty layer that is not 0 and not current\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: layer_delete,
    },
    CommandSpec {
        id: "drawing.layer.edit",
        label: "Layer Properties",
        params: "layer: string (required) - the layer to change\n\
                 name: string (optional) - a new name (not for layer 0); its entities follow\n\
                 colour: string (optional) - #rrggbb, or ink\n\
                 line_type: string (optional) - continuous, dashed, hidden, center, phantom or dot\n\
                 weight: number (optional) - mm, 0.05 to 2.5\n\
                 on: bool (optional) - shown; off hides its entities on screen and in exports\n\
                 frozen: bool (optional) - hidden like off, and it cannot be current\n\
                 locked: bool (optional) - shown, but its entities cannot be edited\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: layer_edit,
    },
    CommandSpec {
        id: "drawing.layer.list",
        label: "Layers",
        params: "sheet: id (optional) - default the last sheet\n\
                 Replies with the layers, the current one, the line type scale, each line type's \
                 dash pattern and how many entities each layer holds",
        mutates: false,
        run: layer_list,
    },
    CommandSpec {
        id: "drawing.linetype_scale",
        label: "Line Type Scale",
        params: "scale: number (required) - multiplies every dash and gap of the user layers' \
                 line types, 0.01 to 100\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: linetype_scale,
    },
];

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_bool(p, key).map(Some),
    }
}

fn drafting(s: &Session, sheet: u32) -> Result<&Drafting, CommandError> {
    Ok(&s.doc.drawing.sheet(sheet)?.drafting)
}

fn drafting_mut(s: &mut Session, sheet: u32) -> Result<&mut Drafting, CommandError> {
    Ok(&mut s.doc.drawing.sheet_mut(sheet)?.drafting)
}

/// A colour: `#rrggbb` (or `rrggbb`), or `ink`/null for the sheet's line colour.
fn colour_param(p: &Value) -> Result<Option<Option<[u8; 3]>>, CommandError> {
    let Some(v) = p.get("colour") else {
        return Ok(None);
    };
    let text = match v {
        Value::Null => return Ok(Some(None)),
        Value::String(t) => t.trim(),
        _ => return Err(bad("`colour` must be a string: #rrggbb or ink")),
    };
    if text.eq_ignore_ascii_case("ink") {
        return Ok(Some(None));
    }
    parse_colour(text).map(|c| Some(Some(c))).ok_or_else(|| {
        bad(format!(
            "`colour` {text:?} must be #rrggbb (hex digits) or ink"
        ))
    })
}

/// `#rrggbb` or `rrggbb` as sRGB.
pub fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
    };
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn colour_text(c: Option<[u8; 3]>) -> String {
    match c {
        None => "ink".into(),
        Some([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

fn line_type_param(p: &Value) -> Result<Option<LineType>, CommandError> {
    match params::opt_str(p, "line_type")? {
        None => Ok(None),
        Some(t) => LineType::parse(t).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown line type {t:?}: use continuous, dashed, hidden, center, phantom or dot"
            ))
        }),
    }
}

fn weight_param(p: &Value) -> Result<Option<f64>, CommandError> {
    match params::opt_f64(p, "weight")? {
        None => Ok(None),
        Some(w) if df::is_valid_weight(w) => Ok(Some(w)),
        Some(_) => Err(bad(format!(
            "`weight` must be {} to {} mm",
            df::MIN_WEIGHT,
            df::MAX_WEIGHT
        ))),
    }
}

fn layer_name_param(p: &Value, key: &str) -> Result<String, CommandError> {
    let name = params::req_str(p, key)?.trim();
    if name == DEFAULT_LAYER || df::is_valid_layer_name(name) {
        Ok(name.to_owned())
    } else {
        Err(bad(format!(
            "`{key}` must be 1 to {} letters, digits, _, - or $, and not one of {}",
            df::MAX_LAYER_NAME,
            df::RESERVED_LAYERS.join(", ")
        )))
    }
}

/// The layer a new entity goes on: `layer` if given (it must exist), else the current.
fn target_layer(d: &Drafting, p: &Value) -> Result<String, CommandError> {
    match params::opt_str(p, "layer")? {
        None => Ok(d.current.clone()),
        Some(name) => d
            .layer(name)
            .map(|l| l.name.clone())
            .ok_or_else(|| bad(format!("no layer {name:?} on this sheet"))),
    }
}

/// Adds a checked entity to a sheet's drafting.
fn add_entity(s: &mut Session, p: &Value, kind: EntityKind) -> CmdResult {
    kind.check().map_err(bad)?;
    let sheet = sheet_id(s, p)?;
    let d = drafting(s, sheet)?;
    if d.entities.len() >= df::MAX_ENTITIES {
        return Err(bad(format!(
            "a sheet holds at most {} drafted entities",
            df::MAX_ENTITIES
        )));
    }
    let layer = target_layer(d, p)?;
    let id = s.doc.drawing.next_id()?;
    let entity = Entity {
        id,
        layer: layer.clone(),
        kind: kind.clone(),
    };
    drafting_mut(s, sheet)?.entities.push(entity);
    Ok(json!({ "entity": id, "sheet": sheet, "layer": layer, "kind": kind }))
}

fn point(p: &Value, x: &str, y: &str) -> Result<Point2, CommandError> {
    Ok(Point2::new(coordinate(p, x)?, coordinate(p, y)?))
}

fn radius(p: &Value) -> Result<f64, CommandError> {
    let r = params::req_f64(p, "radius")?;
    if r > 1e-6 && r <= chamfera_doc::drawing::MAX_SHEET_COORDINATE {
        Ok(r)
    } else {
        Err(bad(format!(
            "`radius` must be above 0 and at most {} mm",
            chamfera_doc::drawing::MAX_SHEET_COORDINATE
        )))
    }
}

fn draw_line(s: &mut Session, p: &Value) -> CmdResult {
    let kind = EntityKind::Line {
        a: point(p, "x1", "y1")?,
        b: point(p, "x2", "y2")?,
    };
    add_entity(s, p, kind)
}

fn draw_circle(s: &mut Session, p: &Value) -> CmdResult {
    let kind = EntityKind::Circle {
        centre: point(p, "cx", "cy")?,
        radius: radius(p)?,
    };
    add_entity(s, p, kind)
}

fn draw_arc(s: &mut Session, p: &Value) -> CmdResult {
    let kind = EntityKind::Arc {
        centre: point(p, "cx", "cy")?,
        radius: radius(p)?,
        start: params::req_f64(p, "start")?,
        end: params::req_f64(p, "end")?,
    };
    add_entity(s, p, kind)
}

fn points_param(p: &Value) -> Result<Vec<Point2>, CommandError> {
    let list = match p.get("points") {
        Some(Value::Array(a)) => a,
        _ => return Err(bad("`points` must be a list of {x, y}")),
    };
    if list.len() > df::MAX_POLYLINE_POINTS {
        return Err(bad(format!(
            "a polyline has at most {} points",
            df::MAX_POLYLINE_POINTS
        )));
    }
    list.iter()
        .map(|q| {
            if !q.is_object() {
                return Err(bad("each point must be {x, y}"));
            }
            point(q, "x", "y")
        })
        .collect()
}

fn draw_polyline(s: &mut Session, p: &Value) -> CmdResult {
    let kind = EntityKind::Polyline {
        points: points_param(p)?,
        closed: opt_bool(p, "closed")?.unwrap_or(false),
    };
    add_entity(s, p, kind)
}

fn draw_rectangle(s: &mut Session, p: &Value) -> CmdResult {
    let a = point(p, "x1", "y1")?;
    let b = point(p, "x2", "y2")?;
    if (a.x - b.x).abs() <= 1e-6 || (a.y - b.y).abs() <= 1e-6 {
        return Err(bad("a rectangle's corners must differ in both x and y"));
    }
    let kind = EntityKind::Polyline {
        points: vec![a, Point2::new(b.x, a.y), b, Point2::new(a.x, b.y)],
        closed: true,
    };
    add_entity(s, p, kind)
}

/// Where entity `id` is: (sheet id, index in its sheet's entities).
fn locate(s: &Session, id: u32) -> Result<(u32, usize), CommandError> {
    for sh in &s.doc.drawing.sheets {
        if let Some(i) = sh.drafting.entities.iter().position(|e| e.id == id) {
            return Ok((sh.id, i));
        }
    }
    Err(bad(format!("no drafted entity {id}")))
}

/// Refuses an edit of an entity on a locked layer.
fn editable(d: &Drafting, e: &Entity) -> Result<(), CommandError> {
    if d.layer(&e.layer).is_some_and(|l| l.locked) {
        return Err(CommandError::Unavailable(format!(
            "entity {} is on locked layer {:?}: unlock the layer first",
            e.id, e.layer
        )));
    }
    Ok(())
}

fn ids_param(p: &Value) -> Result<Vec<u32>, CommandError> {
    let list = match p.get("entities") {
        Some(Value::Array(a)) => a,
        _ => return Err(bad("`entities` must be a list of entity ids")),
    };
    if list.is_empty() || list.len() > MAX_SELECTION {
        return Err(bad(format!("`entities` holds 1 to {MAX_SELECTION} ids")));
    }
    let mut out: Vec<u32> = Vec::with_capacity(list.len());
    for v in list {
        let id = v
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .filter(|x| *x > 0)
            .ok_or_else(|| bad("`entities` must be whole number ids"))?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn entity_grip(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "entity")?;
    let grip = match p.get("grip").and_then(Value::as_u64) {
        Some(g) => usize::try_from(g).map_err(|_| bad("`grip` is too large"))?,
        None => return Err(bad("`grip` must be a whole number from 0")),
    };
    let to = point(p, "x", "y")?;
    let (sheet, i) = locate(s, id)?;
    let d = drafting(s, sheet)?;
    let e = d
        .entities
        .get(i)
        .ok_or_else(|| bad(format!("no drafted entity {id}")))?;
    editable(d, e)?;
    let kind = dr::drag_grip(&e.kind, grip, to).map_err(bad)?;
    if let Some(e) = drafting_mut(s, sheet)?.entities.get_mut(i) {
        e.kind = kind.clone();
    }
    Ok(json!({ "entity": id, "grip": grip, "kind": kind }))
}

fn entity_grips(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "entity")?;
    let (sheet, i) = locate(s, id)?;
    let e = drafting(s, sheet)?
        .entities
        .get(i)
        .ok_or_else(|| bad(format!("no drafted entity {id}")))?;
    let grips: Vec<Value> = dr::grips(&e.kind)
        .iter()
        .enumerate()
        .map(|(i, g)| json!({ "grip": i, "kind": g.kind.id(), "x": g.at.x, "y": g.at.y }))
        .collect();
    Ok(json!({ "entity": id, "grips": grips }))
}

/// The entities named, grouped by sheet as (sheet, index), all editable.
fn selection(s: &Session, ids: &[u32]) -> Result<Vec<(u32, usize)>, CommandError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let (sheet, i) = locate(s, *id)?;
        let d = drafting(s, sheet)?;
        if let Some(e) = d.entities.get(i) {
            editable(d, e)?;
        }
        out.push((sheet, i));
    }
    Ok(out)
}

fn entity_move(s: &mut Session, p: &Value) -> CmdResult {
    let ids = ids_param(p)?;
    let dx = params::req_f64(p, "dx")?;
    let dy = params::req_f64(p, "dy")?;
    let copy = opt_bool(p, "copy")?.unwrap_or(false);
    let shift = Vec2::new(dx, dy);
    let picked = selection(s, &ids)?;
    let mut moved_ids = Vec::new();
    for (sheet, i) in picked {
        let Some(e) = drafting(s, sheet)?.entities.get(i).cloned() else {
            continue;
        };
        let kind = dr::moved(&e.kind, shift);
        kind.check()
            .map_err(|m| bad(format!("entity {}: {m}", e.id)))?;
        if copy {
            if drafting(s, sheet)?.entities.len() >= df::MAX_ENTITIES {
                return Err(bad(format!(
                    "a sheet holds at most {} drafted entities",
                    df::MAX_ENTITIES
                )));
            }
            let id = s.doc.drawing.next_id()?;
            drafting_mut(s, sheet)?.entities.push(Entity {
                id,
                layer: e.layer.clone(),
                kind,
            });
            moved_ids.push(id);
        } else {
            if let Some(x) = drafting_mut(s, sheet)?.entities.get_mut(i) {
                x.kind = kind;
            }
            moved_ids.push(e.id);
        }
    }
    Ok(json!({ "entities": moved_ids, "copied": copy, "dx": dx, "dy": dy }))
}

fn entity_delete(s: &mut Session, p: &Value) -> CmdResult {
    let ids = ids_param(p)?;
    selection(s, &ids)?;
    for sh in &mut s.doc.drawing.sheets {
        sh.drafting.entities.retain(|e| !ids.contains(&e.id));
    }
    Ok(json!({ "deleted": ids }))
}

fn entity_layer(s: &mut Session, p: &Value) -> CmdResult {
    let ids = ids_param(p)?;
    let name = params::req_str(p, "layer")?;
    let picked = selection(s, &ids)?;
    for (sheet, i) in picked {
        let d = drafting_mut(s, sheet)?;
        let layer = d
            .layer(name)
            .map(|l| l.name.clone())
            .ok_or_else(|| bad(format!("no layer {name:?} on sheet {sheet}")))?;
        if let Some(e) = d.entities.get_mut(i) {
            e.layer = layer;
        }
    }
    Ok(json!({ "entities": ids, "layer": name }))
}

fn entity_pick(s: &mut Session, p: &Value) -> CmdResult {
    let at = point(p, "x", "y")?;
    let tolerance = match params::opt_f64(p, "tolerance")? {
        None => 2.0,
        Some(t) if (0.0..=50.0).contains(&t) => t,
        Some(_) => return Err(bad("`tolerance` must be 0 to 50 mm")),
    };
    let sheet = sheet_id(s, p)?;
    let (styles, shapes) = dr::draft(drafting(s, sheet)?);
    let hit = dr::pick_entity(&styles, &shapes, at, tolerance);
    let layer = hit
        .and_then(|id| drafting(s, sheet).ok()?.entity(id))
        .map(|e| e.layer.clone());
    Ok(json!({ "entity": hit, "layer": layer, "sheet": sheet }))
}

fn layer_json(d: &Drafting, l: &DraftLayer) -> Value {
    json!({
        "name": l.name,
        "colour": colour_text(l.colour),
        "line_type": l.line_type.id(),
        "weight": l.weight,
        "on": l.on,
        "frozen": l.frozen,
        "locked": l.locked,
        "current": l.name.eq_ignore_ascii_case(&d.current),
        "entities": d.entities.iter().filter(|e| e.layer.eq_ignore_ascii_case(&l.name)).count(),
    })
}

fn layer_add(s: &mut Session, p: &Value) -> CmdResult {
    let name = layer_name_param(p, "name")?;
    let colour = colour_param(p)?.flatten();
    let line_type = line_type_param(p)?.unwrap_or_default();
    let weight = weight_param(p)?.unwrap_or(0.25);
    let current = opt_bool(p, "current")?.unwrap_or(false);
    let sheet = sheet_id(s, p)?;
    let d = drafting_mut(s, sheet)?;
    if d.layer(&name).is_some() {
        return Err(bad(format!("layer {name:?} already exists")));
    }
    if d.layers.len() >= df::MAX_LAYERS {
        return Err(bad(format!(
            "a sheet has at most {} layers",
            df::MAX_LAYERS
        )));
    }
    let mut layer = DraftLayer::new(&name);
    layer.colour = colour;
    layer.line_type = line_type;
    layer.weight = weight;
    d.layers.push(layer);
    if current {
        d.current = name.clone();
    }
    let reply = d.layer(&name).map(|l| layer_json(d, l));
    Ok(json!({ "sheet": sheet, "layer": reply }))
}

fn layer_edit(s: &mut Session, p: &Value) -> CmdResult {
    let which = params::req_str(p, "layer")?.to_owned();
    let new_name = match p.get("name") {
        None | Some(Value::Null) => None,
        Some(_) => Some(layer_name_param(p, "name")?),
    };
    let colour = colour_param(p)?;
    let line_type = line_type_param(p)?;
    let weight = weight_param(p)?;
    let on = opt_bool(p, "on")?;
    let frozen = opt_bool(p, "frozen")?;
    let locked = opt_bool(p, "locked")?;
    let sheet = sheet_id(s, p)?;
    let d = drafting_mut(s, sheet)?;
    let old = d
        .layer(&which)
        .map(|l| l.name.clone())
        .ok_or_else(|| bad(format!("no layer {which:?} on sheet {sheet}")))?;
    let is_current = old.eq_ignore_ascii_case(&d.current);
    if frozen == Some(true) && is_current {
        return Err(CommandError::Unavailable(format!(
            "layer {old:?} is current: make another layer current before freezing it"
        )));
    }
    let mut name = old.clone();
    if let Some(n) = new_name
        && n != old
    {
        if old == DEFAULT_LAYER || n == DEFAULT_LAYER {
            return Err(bad(
                "layer 0 is always there: it cannot be renamed, nor a layer renamed to 0",
            ));
        }
        if d.layer(&n)
            .is_some_and(|l| !l.name.eq_ignore_ascii_case(&old))
        {
            return Err(bad(format!("layer {n:?} already exists")));
        }
        for e in &mut d.entities {
            if e.layer.eq_ignore_ascii_case(&old) {
                e.layer = n.clone();
            }
        }
        if is_current {
            d.current = n.clone();
        }
        name = n;
    }
    if let Some(l) = d.layer_mut(&old) {
        l.name = name.clone();
        if let Some(c) = colour {
            l.colour = c;
        }
        if let Some(t) = line_type {
            l.line_type = t;
        }
        if let Some(w) = weight {
            l.weight = w;
        }
        if let Some(v) = on {
            l.on = v;
        }
        if let Some(v) = frozen {
            l.frozen = v;
        }
        if let Some(v) = locked {
            l.locked = v;
        }
    }
    let reply = d.layer(&name).map(|l| layer_json(d, l));
    Ok(json!({ "sheet": sheet, "layer": reply }))
}

fn layer_delete(s: &mut Session, p: &Value) -> CmdResult {
    let which = params::req_str(p, "layer")?;
    let sheet = sheet_id(s, p)?;
    let d = drafting_mut(s, sheet)?;
    let name = d
        .layer(which)
        .map(|l| l.name.clone())
        .ok_or_else(|| bad(format!("no layer {which:?} on sheet {sheet}")))?;
    if name == DEFAULT_LAYER {
        return Err(bad("layer 0 is always there and cannot be deleted"));
    }
    if name.eq_ignore_ascii_case(&d.current) {
        return Err(CommandError::Unavailable(format!(
            "layer {name:?} is current: make another layer current first"
        )));
    }
    let held = d
        .entities
        .iter()
        .filter(|e| e.layer.eq_ignore_ascii_case(&name))
        .count();
    if held > 0 {
        return Err(CommandError::Unavailable(format!(
            "layer {name:?} holds {held} entities: move them to another layer or delete them first"
        )));
    }
    d.layers.retain(|l| l.name != name);
    Ok(json!({ "sheet": sheet, "deleted": name }))
}

fn layer_current(s: &mut Session, p: &Value) -> CmdResult {
    let which = params::req_str(p, "layer")?;
    let sheet = sheet_id(s, p)?;
    let d = drafting_mut(s, sheet)?;
    let l = d
        .layer(which)
        .ok_or_else(|| bad(format!("no layer {which:?} on sheet {sheet}")))?;
    if l.frozen {
        return Err(CommandError::Unavailable(format!(
            "layer {:?} is frozen: thaw it before making it current",
            l.name
        )));
    }
    let name = l.name.clone();
    d.current = name.clone();
    Ok(json!({ "sheet": sheet, "current": name }))
}

fn layer_list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let sheet = sheet_id(s, p)?;
    let d = drafting(s, sheet)?;
    let layers: Vec<Value> = d.layers.iter().map(|l| layer_json(d, l)).collect();
    let types: Vec<Value> = LineType::ALL
        .iter()
        .map(|t| {
            json!({
                "id": t.id(),
                "pattern": linetype::pattern(*t),
                "scaled": linetype::scaled(*t, d.linetype_scale),
                "dxf": linetype::dxf_name(*t),
            })
        })
        .collect();
    Ok(json!({
        "sheet": sheet,
        "current": d.current,
        "linetype_scale": d.linetype_scale,
        "layers": layers,
        "line_types": types,
        "weights": df::WEIGHTS,
        "entities": d.entities,
    }))
}

fn linetype_scale(s: &mut Session, p: &Value) -> CmdResult {
    let k = params::req_f64(p, "scale")?;
    if !df::is_valid_linetype_scale(k) {
        return Err(bad(format!(
            "`scale` must be {} to {}",
            df::MIN_LINETYPE_SCALE,
            df::MAX_LINETYPE_SCALE
        )));
    }
    let sheet = sheet_id(s, p)?;
    drafting_mut(s, sheet)?.linetype_scale = k;
    Ok(json!({ "sheet": sheet, "linetype_scale": k }))
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use serde_json::{Value, json};

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn session_with_sheet() -> Session {
        let mut s = Session::default();
        run(&mut s, "drawing.new_sheet", json!({}));
        s
    }

    #[test]
    fn layers_are_added_edited_made_current_and_deleted() {
        let mut s = session_with_sheet();
        let r = run(
            &mut s,
            "drawing.layer.add",
            json!({"name": "Axes", "colour": "#FF0000", "line_type": "centre", "weight": 0.35, "current": true}),
        );
        assert_eq!(r["layer"]["colour"], "#ff0000");
        assert_eq!(r["layer"]["line_type"], "center");
        assert_eq!(r["layer"]["current"], true);
        // A line goes on the current layer.
        let line = run(
            &mut s,
            "drawing.draw.line",
            json!({"x1": 10, "y1": 10, "x2": 60, "y2": 10}),
        );
        assert_eq!(line["layer"], "Axes");
        // Duplicate, reserved and malformed names are refused.
        for bad in ["axes", "HIDDEN", "a b", "", "0"] {
            assert!(
                s.execute("drawing.layer.add", &json!({"name": bad}))
                    .is_err(),
                "{bad:?}"
            );
        }
        // The current layer cannot be frozen or deleted.
        assert!(
            s.execute(
                "drawing.layer.edit",
                &json!({"layer": "Axes", "frozen": true})
            )
            .is_err()
        );
        run(&mut s, "drawing.layer.current", json!({"layer": "0"}));
        run(
            &mut s,
            "drawing.layer.edit",
            json!({"layer": "Axes", "frozen": true}),
        );
        assert!(
            s.execute("drawing.layer.current", &json!({"layer": "axes"}))
                .is_err(),
            "a frozen layer cannot be current"
        );
        // A layer holding entities is not deleted; renaming carries its entities.
        assert!(
            s.execute("drawing.layer.delete", &json!({"layer": "Axes"}))
                .is_err()
        );
        run(
            &mut s,
            "drawing.layer.edit",
            json!({"layer": "Axes", "name": "CentreLines"}),
        );
        let list = run(&mut s, "drawing.layer.list", json!({}));
        assert_eq!(list["layers"][1]["name"], "CentreLines");
        assert_eq!(list["layers"][1]["entities"], 1);
        assert_eq!(list["entities"][0]["layer"], "CentreLines");
        assert!(
            s.execute("drawing.layer.edit", &json!({"layer": "0", "name": "Zero"}))
                .is_err()
        );
        assert!(
            s.execute("drawing.layer.delete", &json!({"layer": "0"}))
                .is_err()
        );
        run(&mut s, "drawing.layer.add", json!({"name": "Spare"}));
        run(&mut s, "drawing.layer.delete", json!({"layer": "spare"}));
        // Undo takes back the deletion only.
        s.execute("edit.undo", &json!({})).unwrap();
        let list = run(&mut s, "drawing.layer.list", json!({}));
        assert_eq!(list["layers"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn a_grip_drag_moves_exactly_the_dragged_point() {
        let mut s = session_with_sheet();
        let r = run(
            &mut s,
            "drawing.draw.rectangle",
            json!({"x1": 20, "y1": 20, "x2": 80, "y2": 50}),
        );
        let id = r["entity"].as_u64().unwrap();
        let grips = run(&mut s, "drawing.entity.grips", json!({"entity": id}));
        // Four corners, then four midpoints; grip 2 is the corner (80, 50).
        assert_eq!(grips["grips"].as_array().map(Vec::len), Some(8));
        assert_eq!(grips["grips"][2]["x"], 80.0);
        assert_eq!(grips["grips"][2]["y"], 50.0);
        let r = run(
            &mut s,
            "drawing.entity.grip",
            json!({"entity": id, "grip": 2, "x": 90.5, "y": 61.25}),
        );
        let pts = &r["kind"]["polyline"]["points"];
        // Only that corner moved, to exactly the point given.
        assert_eq!(pts[2], json!({"x": 90.5, "y": 61.25}));
        assert_eq!(pts[0], json!({"x": 20.0, "y": 20.0}));
        assert_eq!(pts[1], json!({"x": 80.0, "y": 20.0}));
        assert_eq!(pts[3], json!({"x": 20.0, "y": 50.0}));
        // A locked layer refuses the drag and leaves the entity as it was.
        run(
            &mut s,
            "drawing.layer.edit",
            json!({"layer": "0", "locked": true}),
        );
        assert!(
            s.execute(
                "drawing.entity.grip",
                &json!({"entity": id, "grip": 0, "x": 0, "y": 0})
            )
            .is_err()
        );
        assert!(
            s.execute("drawing.entity.delete", &json!({"entities": [id]}))
                .is_err()
        );
        let list = run(&mut s, "drawing.layer.list", json!({}));
        assert_eq!(
            list["entities"][0]["kind"]["polyline"]["points"][0]["x"],
            20.0
        );
    }

    #[test]
    fn entities_move_copy_change_layer_and_delete() {
        let mut s = session_with_sheet();
        run(&mut s, "drawing.layer.add", json!({"name": "Notes"}));
        let c = run(
            &mut s,
            "drawing.draw.circle",
            json!({"cx": 100, "cy": 100, "radius": 10}),
        )["entity"]
            .as_u64()
            .unwrap();
        let a = run(
            &mut s,
            "drawing.draw.arc",
            json!({"cx": 0, "cy": 0, "radius": 5, "start": 0, "end": std::f64::consts::PI, "layer": "notes"}),
        );
        assert_eq!(a["layer"], "Notes");
        let r = run(
            &mut s,
            "drawing.entity.move",
            json!({"entities": [c], "dx": 5, "dy": -2.5, "copy": true}),
        );
        let copy = r["entities"][0].as_u64().unwrap();
        assert_ne!(copy, c);
        let list = run(&mut s, "drawing.layer.list", json!({}));
        assert_eq!(
            list["entities"][2]["kind"]["circle"]["centre"],
            json!({"x": 105.0, "y": 97.5})
        );
        // Picking: the copy's rightmost point (115, 97.5).
        let hit = run(
            &mut s,
            "drawing.entity.pick",
            json!({"x": 115.2, "y": 97.5}),
        );
        assert_eq!(hit["entity"].as_u64(), Some(copy));
        run(
            &mut s,
            "drawing.entity.layer",
            json!({"entities": [c, copy], "layer": "Notes"}),
        );
        // Turning Notes off hides them from picking.
        run(
            &mut s,
            "drawing.layer.edit",
            json!({"layer": "Notes", "on": false}),
        );
        let miss = run(
            &mut s,
            "drawing.entity.pick",
            json!({"x": 115.2, "y": 97.5}),
        );
        assert!(miss["entity"].is_null());
        run(
            &mut s,
            "drawing.entity.delete",
            json!({"entities": [c, copy]}),
        );
        let list = run(&mut s, "drawing.layer.list", json!({}));
        assert_eq!(list["entities"].as_array().map(Vec::len), Some(1));
        // The design saves and opens again with its layers.
        let text = s.doc.to_json().unwrap();
        let back = chamfera_doc::Document::from_json(&text).unwrap();
        assert_eq!(back.drawing, s.doc.drawing);
    }

    #[test]
    fn the_line_type_scale_reaches_the_svg() {
        let mut s = session_with_sheet();
        run(
            &mut s,
            "drawing.layer.add",
            json!({"name": "Hid", "line_type": "hidden", "current": true}),
        );
        run(
            &mut s,
            "drawing.draw.line",
            json!({"x1": 10, "y1": 10, "x2": 60, "y2": 10}),
        );
        run(&mut s, "drawing.linetype_scale", json!({"scale": 4}));
        let r = run(&mut s, "drawing.export_bytes", json!({"format": "svg"}));
        let bytes =
            crate::cmd::design::decode_base64(r["data"].as_str().unwrap(), 1 << 24).unwrap();
        let svg = String::from_utf8(bytes).unwrap();
        // Hidden is 1.5, 0.75; at scale 4: 6, 3.
        assert!(svg.contains(r#"stroke-dasharray="6 3""#), "{svg}");
        assert!(
            s.execute("drawing.linetype_scale", &json!({"scale": 0}))
                .is_err()
        );
    }
}

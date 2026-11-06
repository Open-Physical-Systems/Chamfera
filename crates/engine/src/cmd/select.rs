//! `select.*`: what the user has picked in the viewport. The selection is a set of bodies;
//! the last pick also remembers where it hit. In an active sketch, `select.box` selects
//! the sketch's points and curves instead.

use super::CommandSpec;
use crate::box_select::{BoxMode, ScreenBox, bodies_in_box, sketch_items_in_box};
use crate::pick_filter::{PickFilter, Picked};
use crate::{CmdResult, CommandError, Session, params};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "select.box",
        label: "Box Select",
        params: "x1, y1, x2, y2: number (required) - the box's corners in pixels, origin top-left\n\
                 width, height: number (required) - viewport size in pixels\n\
                 mode: string (required) - window (what is fully inside) or crossing (what the \
                 box touches or contains)\n\
                 add: boolean (optional) - add to the selection instead of replacing it\n\
                 remove: boolean (optional) - remove what the box selects from the selection\n\
                 In an active sketch it selects the sketch's points and curves, else bodies",
        mutates: false,
        run: select_box,
    },
    CommandSpec {
        id: "select.clear",
        label: "Clear Selection",
        params: "(none)",
        mutates: false,
        run: clear,
    },
    CommandSpec {
        id: "select.get",
        label: "Get Selection",
        params: "(none)",
        mutates: false,
        run: get,
    },
    CommandSpec {
        id: "select.pick",
        label: "Pick",
        params: "x, y: number (required) - pixel, origin top-left\n\
                 width, height: number (required) - viewport size in pixels\n\
                 mode: string (optional) - replace (default: select only what is hit, nothing \
                 on a miss), add, toggle (add, or remove if selected) or remove; a miss \
                 changes nothing unless replacing\n\
                 filter: list of strings (optional) - what may be picked: bodies, faces, \
                 edges (an edge near the pixel comes first); without it, the body hit",
        mutates: false,
        run: pick,
    },
];

/// How a pick or a box changes the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Combine {
    Replace,
    Add,
    Toggle,
    Remove,
}

fn reply(s: &Session) -> Value {
    let mut r = body_reply(s);
    r["edges"] = json!(s.selected_edges);
    if let Some(face) = s.selected_face() {
        r["face"] = json!(face);
    }
    r
}

fn body_reply(s: &Session) -> Value {
    let bodies = s.scene.selected_bodies();
    let mut r = match s.scene.selection {
        Some(h) => json!({
            "body": h.body,
            "name": s.scene.body(h.body).map(|b| b.name.clone()),
            "triangle": h.triangle,
            "point": h.point,
            "distance": h.distance,
            "normal": s.scene.body(h.body).and_then(|b| b.mesh.triangle_normal(h.triangle)),
        }),
        None => json!({ "body": null }),
    };
    r["count"] = json!(bodies.len());
    r["bodies"] = json!(bodies);
    r
}

fn combine(p: &Value) -> Result<Combine, CommandError> {
    match params::opt_str(p, "mode")? {
        None | Some("replace") => Ok(Combine::Replace),
        Some("add") => Ok(Combine::Add),
        Some("toggle") => Ok(Combine::Toggle),
        Some("remove") => Ok(Combine::Remove),
        Some(m) => Err(CommandError::BadParams(format!(
            "`mode` must be replace, add, toggle or remove, not {m:?}"
        ))),
    }
}

fn opt_bool(p: &Value, key: &str) -> Result<bool, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(_) => params::req_bool(p, key),
    }
}

fn pick(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    let how = combine(p)?;
    let bounds = s.scene.bounds();
    let ray = s
        .camera
        .ray_through(x, y, w, h, bounds.as_ref())
        .ok_or_else(|| CommandError::BadParams("no ray through that pixel".into()))?;
    let (hit, edge) = match PickFilter::from_params(p)? {
        None => (s.scene.pick(&ray), None),
        Some(filter) => match s.pick_filtered(x, y, w, h, &filter) {
            Picked::Edge { name, .. } => (None, Some(name)),
            Picked::Face { hit, .. } | Picked::Body(hit) => (Some(hit), None),
            Picked::Nothing => (None, None),
        },
    };
    if let Some(e) = edge {
        // An edge: replacing selects only it; the other modes change the edge list.
        let had = s.selected_edges.contains(&e);
        match how {
            Combine::Replace => {
                s.scene.clear_selection();
                s.selected_edges = vec![e];
            }
            Combine::Add if !had => s.selected_edges.push(e),
            Combine::Toggle if !had => s.selected_edges.push(e),
            Combine::Toggle | Combine::Remove => s.selected_edges.retain(|x| *x != e),
            Combine::Add => {}
        }
        return Ok(reply(s));
    }
    if how == Combine::Replace {
        s.selected_edges.clear();
    }
    // Keep the set complete before changing it: the picked body belongs to it.
    s.scene.selected = s.scene.selected_bodies();
    match (how, hit) {
        (Combine::Replace, _) => {
            s.scene.selected = hit.map(|h| vec![h.body]).unwrap_or_default();
            s.scene.selection = hit;
        }
        (_, None) => {}
        (Combine::Add, Some(h)) => add(s, h),
        (Combine::Toggle, Some(h)) => {
            if s.scene.selected.contains(&h.body) {
                s.scene.deselect(|b| b == h.body);
            } else {
                add(s, h);
            }
        }
        (Combine::Remove, Some(h)) => s.scene.deselect(|b| b == h.body),
    }
    Ok(reply(s))
}

fn add(s: &mut Session, h: chamfera_render::Hit) {
    if !s.scene.selected.contains(&h.body) {
        s.scene.selected.push(h.body);
    }
    s.scene.selection = Some(h);
}

fn select_box(s: &mut Session, p: &Value) -> CmdResult {
    let (w, h) = params::viewport(p)?;
    let corner = |kx: &str, ky: &str| -> Result<(f64, f64), CommandError> {
        let x = params::req_f64(p, kx)?;
        let y = params::req_f64(p, ky)?;
        if !(0.0..=w).contains(&x) || !(0.0..=h).contains(&y) {
            return Err(CommandError::BadParams(format!(
                "`{kx}`, `{ky}` must be inside the viewport"
            )));
        }
        Ok((x, y))
    };
    let a = corner("x1", "y1")?;
    let b = corner("x2", "y2")?;
    let mode = BoxMode::parse(params::req_str(p, "mode")?)
        .ok_or_else(|| CommandError::BadParams("`mode` must be window or crossing".into()))?;
    let (add, remove) = (opt_bool(p, "add")?, opt_bool(p, "remove")?);
    let how = match (add, remove) {
        (true, true) => {
            return Err(CommandError::BadParams(
                "`add` and `remove` cannot both be true".into(),
            ));
        }
        (true, false) => Combine::Add,
        (false, true) => Combine::Remove,
        (false, false) => Combine::Replace,
    };
    let area = ScreenBox::from_corners(a, b);
    if s.active_sketch.is_some() {
        let items = sketch_items_in_box(s, area, mode, w, h);
        match how {
            Combine::Replace => s.sketch_selection = items.clone(),
            Combine::Remove => s.sketch_selection.retain(|i| !items.contains(i)),
            Combine::Add | Combine::Toggle => {
                for i in &items {
                    if !s.sketch_selection.contains(i) {
                        s.sketch_selection.push(*i);
                    }
                }
            }
        }
        return Ok(json!({
            "mode": mode,
            "items": items,
            "selection": s.sketch_selection,
        }));
    }
    let ids = bodies_in_box(s, area, mode, w, h);
    s.scene.selected = s.scene.selected_bodies();
    match how {
        Combine::Replace => s.scene.clear_selection(),
        Combine::Remove => s.scene.deselect(|b| ids.contains(&b)),
        Combine::Add | Combine::Toggle => {}
    }
    if how != Combine::Remove {
        for id in &ids {
            if !s.scene.selected.contains(id) {
                s.scene.selected.push(*id);
            }
        }
    }
    let mut r = reply(s);
    r["mode"] = json!(mode);
    r["boxed"] = json!(ids);
    Ok(r)
}

fn clear(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.scene.clear_selection();
    s.selected_edges.clear();
    Ok(reply(s))
}

fn get(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(reply(s))
}

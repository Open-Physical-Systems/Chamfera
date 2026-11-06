//! `select.*`: what the user has picked in the viewport.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
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
                 width, height: number (required) - viewport size in pixels",
        mutates: false,
        run: pick,
    },
];

fn reply(s: &Session) -> Value {
    match s.scene.selection {
        Some(h) => json!({
            "body": h.body,
            "name": s.scene.body(h.body).map(|b| b.name.clone()),
            "triangle": h.triangle,
            "point": h.point,
            "distance": h.distance,
            "normal": s.scene.body(h.body).and_then(|b| b.mesh.triangle_normal(h.triangle)),
            "component": s.component_of_body(h.body),
        }),
        None => json!({ "body": null }),
    }
}

fn pick(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    let bounds = s.scene.bounds();
    let ray = s
        .camera
        .ray_through(x, y, w, h, bounds.as_ref())
        .ok_or_else(|| CommandError::BadParams("no ray through that pixel".into()))?;
    s.scene.selection = s.scene.pick(&ray);
    Ok(reply(s))
}

fn clear(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.scene.selection = None;
    Ok(reply(s))
}

fn get(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(reply(s))
}

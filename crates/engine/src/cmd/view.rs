//! `view.*`: the camera. The UI's mouse and keyboard navigation call these, so there is one
//! camera path for the UI, scripts and agents.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_render::{Projection, StandardView};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "view.fit",
        label: "Fit",
        params: "width, height: number (required) - viewport size in pixels",
        mutates: false,
        run: fit,
    },
    CommandSpec {
        id: "view.get",
        label: "Get View",
        params: "(none)",
        mutates: false,
        run: get,
    },
    CommandSpec {
        id: "view.home",
        label: "Home View",
        params: "width, height: number (required) - viewport size in pixels",
        mutates: false,
        run: home,
    },
    CommandSpec {
        id: "view.normal_to",
        label: "Look At",
        params: "x, y, z: number (required) - look against this direction (a face normal)",
        mutates: false,
        run: normal_to,
    },
    CommandSpec {
        id: "view.orbit",
        label: "Orbit",
        params: "yaw: number (required) - radians around +Z\npitch: number (required) - radians up",
        mutates: false,
        run: orbit,
    },
    CommandSpec {
        id: "view.pan",
        label: "Pan",
        params: "dx, dy: number (required) - pixels, right and down\n\
                 height: number (required) - viewport height in pixels",
        mutates: false,
        run: pan,
    },
    CommandSpec {
        id: "view.projection",
        label: "Projection",
        params: "mode: string (required) - perspective or orthographic",
        mutates: false,
        run: projection,
    },
    CommandSpec {
        id: "view.standard",
        label: "Standard View",
        params: "view: string (required) - front, back, left, right, top, bottom or iso",
        mutates: false,
        run: standard,
    },
    CommandSpec {
        id: "view.zoom",
        label: "Zoom",
        params: "factor: number (required) - below 1 zooms in, 0.01 to 100\n\
                 x, y, width, height: number (optional) - zoom toward this pixel",
        mutates: false,
        run: zoom,
    },
];

fn get(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(reply(s))
}

fn reply(s: &Session) -> Value {
    json!({ "camera": s.camera, "eye": s.camera.eye() })
}

fn fit(s: &mut Session, p: &Value) -> CmdResult {
    let (w, h) = params::viewport(p)?;
    if let Some(b) = s.scene.bounds() {
        s.camera.fit(&b, w / h);
    }
    Ok(reply(s))
}

fn home(s: &mut Session, p: &Value) -> CmdResult {
    let (w, h) = params::viewport(p)?;
    s.camera.set_view(StandardView::Iso);
    match s.scene.bounds() {
        Some(b) => s.camera.fit(&b, w / h),
        None => {
            let d = chamfera_render::Camera::default();
            s.camera.target = d.target;
            s.camera.distance = d.distance;
        }
    }
    Ok(reply(s))
}

/// One step turns at most a full circle, so a hostile value can't spin the view wildly.
fn angle(p: &Value, key: &str) -> Result<f64, CommandError> {
    let v = params::req_f64(p, key)?;
    if v.abs() > std::f64::consts::TAU {
        return Err(CommandError::BadParams(format!(
            "`{key}` must be within ±2π"
        )));
    }
    Ok(v)
}

fn normal_to(s: &mut Session, p: &Value) -> CmdResult {
    let n = chamfera_geom::Vec3::new(
        params::req_f64(p, "x")?,
        params::req_f64(p, "y")?,
        params::req_f64(p, "z")?,
    )
    .normalized()
    .ok_or_else(|| CommandError::BadParams("the direction has no length".into()))?;
    // The eye sits on the normal's side, looking back along it.
    s.camera.yaw = n.y.atan2(n.x);
    s.camera.pitch = n.z.clamp(-1.0, 1.0).asin();
    if n.x.abs() < 1e-9 && n.y.abs() < 1e-9 {
        // Straight up or down: keep +Y up on screen, as the Top view does.
        s.camera.yaw = -std::f64::consts::FRAC_PI_2;
    }
    s.camera.sanitize();
    Ok(reply(s))
}

fn orbit(s: &mut Session, p: &Value) -> CmdResult {
    let yaw = angle(p, "yaw")?;
    let pitch = angle(p, "pitch")?;
    s.camera.orbit(yaw, pitch);
    Ok(reply(s))
}

fn pan(s: &mut Session, p: &Value) -> CmdResult {
    let dx = params::req_f64(p, "dx")?;
    let dy = params::req_f64(p, "dy")?;
    let h = params::req_f64(p, "height")?;
    if dx.abs() > 1e5 || dy.abs() > 1e5 || !(1.0..=32768.0).contains(&h) {
        return Err(CommandError::BadParams(
            "`dx`, `dy` must be within ±100000 px and `height` 1 to 32768".into(),
        ));
    }
    s.camera.pan_pixels(dx, dy, h);
    Ok(reply(s))
}

fn zoom(s: &mut Session, p: &Value) -> CmdResult {
    let factor = params::req_f64(p, "factor")?;
    if !(0.01..=100.0).contains(&factor) {
        return Err(CommandError::BadParams(
            "`factor` must be 0.01 to 100".into(),
        ));
    }
    // Zoom toward what is under the cursor, or toward the cursor's ray at target depth.
    let toward = match params::opt_f64(p, "x")? {
        None => None,
        Some(_) => {
            let (x, y, w, h) = params::pixel(p)?;
            let bounds = s.scene.bounds();
            s.camera
                .ray_through(x, y, w, h, bounds.as_ref())
                .map(|ray| match s.scene.pick(&ray) {
                    Some(hit) => hit.point,
                    None => {
                        let to_target = s.camera.target - ray.origin;
                        ray.at(to_target.dot(ray.direction).max(0.0))
                    }
                })
        }
    };
    s.camera.zoom(factor, toward);
    Ok(reply(s))
}

fn projection(s: &mut Session, p: &Value) -> CmdResult {
    s.camera.projection = match params::req_str(p, "mode")? {
        "perspective" => Projection::Perspective,
        "orthographic" => Projection::Orthographic,
        other => {
            return Err(CommandError::BadParams(format!(
                "unknown mode {other:?}: use perspective or orthographic"
            )));
        }
    };
    Ok(reply(s))
}

fn standard(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "view")?;
    let view = StandardView::parse(name).ok_or_else(|| {
        CommandError::BadParams(format!(
            "unknown view {name:?}: use front, back, left, right, top, bottom or iso"
        ))
    })?;
    s.camera.set_view(view);
    Ok(reply(s))
}

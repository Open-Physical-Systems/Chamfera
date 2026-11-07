//! `render.*`: presentation images. The environment is display state (like the visual
//! style); materials are `body.appearance`; images come from the path tracer
//! (`chamfera-raytrace`) with the current view.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_raytrace::{MAX_SAMPLES, MAX_SIZE, Render, Settings};
use chamfera_render::EnvironmentPreset;
use chamfera_render::material::LIBRARY;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "render.environment",
        label: "Environment",
        params: "preset: string (required) - studio (soft key, fill and rim lights over a \
                 pale floor), outdoor (blue sky and a small sun) or dark (a dark room with \
                 a strong key and rim)\n\
                 The lighting of the realistic style and of renders; a view state, not part \
                 of the design.",
        mutates: false,
        run: environment,
    },
    CommandSpec {
        id: "render.image",
        label: "Render Image",
        params: "path: string (required) - the PNG file to write\n\
                 width, height: integer (required) - pixels, 1 to 4096\n\
                 samples: integer (required) - samples per pixel, 1 to 4096 (64 is a good preview, 512 a clean picture)\n\
                 time_limit: number (optional) - seconds: stop sooner, with the samples \
                 done by then (at least one)\n\
                 Path-traces the visible bodies with their materials in the environment, \
                 seen from the current view, and writes the picture.",
        mutates: false,
        run: image,
    },
    CommandSpec {
        id: "render.image_bytes",
        label: "Render Image",
        params: "width, height: integer (required) - pixels, 1 to 4096\n\
                 samples: integer (required) - samples per pixel, 1 to 4096 (64 is a good preview, 512 a clean picture)\n\
                 time_limit: number (optional) - seconds\n\
                 As render.image, but replies with the PNG as base64 `data`; writes no file",
        mutates: false,
        run: image_bytes,
    },
    CommandSpec {
        id: "render.materials",
        label: "Material Library",
        params: "(none) - every material of the library with its colour, metallic, \
                 roughness, transmission and index of refraction; give an id to \
                 body.appearance as `preset`",
        mutates: false,
        run: materials,
    },
    CommandSpec {
        id: "render.save",
        label: "Save Render",
        params: "path: string (required) - the PNG file to write\n\
                 Writes the picture of the render started with render.start, as far as \
                 it has got.",
        mutates: false,
        run: save,
    },
    CommandSpec {
        id: "render.save_bytes",
        label: "Save Render",
        params: "(none) - replies with the started render's picture as a PNG, base64 \
                 `data`; writes no file",
        mutates: false,
        run: save_bytes,
    },
    CommandSpec {
        id: "render.start",
        label: "Render",
        params: "width, height: integer (required) - pixels, 1 to 4096\n\
                 samples: integer (required) - samples per pixel to stop at, 1 to 4096\n\
                 Starts a progressive render of the current view that the app refines in \
                 its Render window; render.save writes it. Replaces an earlier one.",
        mutates: false,
        run: start,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn whole(p: &Value, key: &str, max: u32) -> Result<u32, CommandError> {
    let v = params::req_f64(p, key)?;
    if v.fract() != 0.0 || !(1.0..=f64::from(max)).contains(&v) {
        return Err(bad(format!("`{key}` must be a whole number 1 to {max}")));
    }
    Ok(v as u32)
}

fn settings(p: &Value) -> Result<Settings, CommandError> {
    Ok(Settings::new(
        whole(p, "width", MAX_SIZE)?,
        whole(p, "height", MAX_SIZE)?,
        whole(p, "samples", MAX_SAMPLES)?,
    ))
}

/// Seconds, if given: finite and positive.
fn time_limit(p: &Value) -> Result<Option<f64>, CommandError> {
    match params::opt_f64(p, "time_limit")? {
        Some(t) if !(t > 0.0 && t <= 86_400.0) => Err(bad(
            "`time_limit` must be more than 0 and at most 86400 seconds",
        )),
        t => Ok(t),
    }
}

fn environment(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "preset")?;
    let preset = EnvironmentPreset::parse(name).ok_or_else(|| {
        let all: Vec<&str> = EnvironmentPreset::ALL.iter().map(|e| e.id()).collect();
        bad(format!(
            "unknown environment {name:?}: use {}",
            all.join(", ")
        ))
    })?;
    s.scene.display.environment = preset;
    Ok(json!({ "environment": preset.id() }))
}

fn materials(_: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let list: Vec<Value> = LIBRARY
        .iter()
        .map(|m| {
            let c = m.colour;
            json!({
                "id": m.id,
                "label": m.label,
                "category": m.category.label(),
                "colour": format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2),
                "metallic": m.metallic,
                "roughness": m.roughness,
                "transmission": m.transmission,
                "ior": m.ior,
            })
        })
        .collect();
    Ok(json!({ "materials": list }))
}

/// Runs a render to its sample count or the time limit (at least one pass).
fn run_to_end(r: &Render, limit: Option<f64>) {
    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();
    // The browser has no clock in std, and renders there are run from the UI instead.
    #[cfg(target_arch = "wasm32")]
    let _ = limit;
    loop {
        r.step();
        if r.done() {
            break;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if limit.is_some_and(|t| start.elapsed().as_secs_f64() >= t) {
            break;
        }
    }
}

fn reply(r: &Render, bytes: usize) -> Value {
    let st = r.settings();
    json!({
        "width": st.width,
        "height": st.height,
        "samples": r.samples(),
        "target_samples": st.samples,
        "bytes": bytes,
    })
}

fn rendered(s: &Session, p: &Value) -> Result<(Render, Vec<u8>), CommandError> {
    let limit = time_limit(p)?;
    let r = s.prepare_render(settings(p)?)?;
    run_to_end(&r, limit);
    let png = r
        .png()
        .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    Ok((r, png))
}

fn png_path(p: &Value) -> Result<&str, CommandError> {
    let path = params::req_str(p, "path")?;
    let is_png = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(bad("`path` must end in .png"));
    }
    Ok(path)
}

fn image(s: &mut Session, p: &Value) -> CmdResult {
    let path = png_path(p)?;
    let (r, png) = rendered(s, p)?;
    std::fs::write(path, &png).map_err(chamfera_io::IoError::from)?;
    let mut out = reply(&r, png.len());
    out["path"] = json!(path);
    Ok(out)
}

fn image_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let (r, png) = rendered(s, p)?;
    let mut out = reply(&r, png.len());
    out["data"] = json!(super::design::encode_base64(&png));
    Ok(out)
}

fn start(s: &mut Session, p: &Value) -> CmdResult {
    let job = s.start_render(settings(p)?)?;
    Ok(reply(&job, 0))
}

fn job_png(s: &Session) -> Result<(std::sync::Arc<Render>, Vec<u8>), CommandError> {
    let job = s.render_job.clone().ok_or_else(|| {
        CommandError::Unavailable("no render: start one with render.start".into())
    })?;
    let png = job
        .png()
        .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    Ok((job, png))
}

fn save(s: &mut Session, p: &Value) -> CmdResult {
    let path = png_path(p)?;
    let (job, png) = job_png(s)?;
    std::fs::write(path, &png).map_err(chamfera_io::IoError::from)?;
    let mut out = reply(&job, png.len());
    out["path"] = json!(path);
    Ok(out)
}

fn save_bytes(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let (job, png) = job_png(s)?;
    let mut out = reply(&job, png.len());
    out["data"] = json!(super::design::encode_base64(&png));
    Ok(out)
}

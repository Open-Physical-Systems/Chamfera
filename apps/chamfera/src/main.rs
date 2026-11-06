//! The Chamfera desktop app.
//!
//! ```text
//! chamfera [part.stl ...] [--sample] [--theme light|dark] [--script steps.json]
//!          [--parameters] [--extrude] [--size WxH] [--zoom F] [--screenshot shot.png]
//!          [--sketch-input "tool line; click 0,0; hover 30,12; type 25"]
//! ```

// Release builds on Windows open no console window; chamfera-cli stays a console app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

const USAGE: &str = "usage: chamfera [part.stl ...] [--sample] [--theme light|dark] \
[--script steps.json] [--parameters] [--extrude] [--size WxH] [--zoom F] [--screenshot shot.png] \
[--sketch-input \"tool line; click X,Y; hover X,Y; shift; type 25; tab\"]";

fn main() -> ExitCode {
    let mut options = chamfera_ui::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => options.sample = true,
            "--parameters" => options.parameters = true,
            "--extrude" => options.extrude = true,
            "--size" => match args.next().as_deref().and_then(parse_size) {
                Some(size) => options.window_size = Some(size),
                None => return fail(USAGE),
            },
            "--zoom" => match args.next().and_then(|z| z.parse::<f32>().ok()) {
                Some(z) if z.is_finite() && (0.5..=4.0).contains(&z) => options.zoom = Some(z),
                _ => return fail("--zoom takes a number from 0.5 to 4"),
            },
            "--theme" => match args.next().as_deref() {
                Some("light") => options.theme = Some(chamfera_render::ThemeMode::Light),
                Some("dark") => options.theme = Some(chamfera_render::ThemeMode::Dark),
                _ => return fail(USAGE),
            },
            "--script" => match args.next().map(|p| read_script(&p)) {
                Some(Ok(steps)) => options.script = steps,
                Some(Err(e)) => return fail(&e),
                None => return fail(USAGE),
            },
            "--sketch-input" => match args.next() {
                Some(steps) => options.sketch_input = Some(steps),
                None => return fail(USAGE),
            },
            "--screenshot" => match args.next() {
                Some(p) => options.screenshot = Some(p.into()),
                None => return fail(USAGE),
            },
            "--version" => {
                println!("chamfera {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => return fail(USAGE),
            file => options.files.push(file.into()),
        }
    }
    match chamfera_ui::run_native(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("chamfera: {msg}");
    ExitCode::FAILURE
}

/// `1000x680` → [1000, 680], within sensible window sizes.
fn parse_size(text: &str) -> Option<[f32; 2]> {
    let (w, h) = text.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    let ok = |v: f32| v.is_finite() && (320.0..=8192.0).contains(&v);
    (ok(w) && ok(h)).then_some([w, h])
}

/// Reads a `chamfera-cli run` style script: a JSON list of {"command", "params"}.
fn read_script(path: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    v.as_array().cloned().ok_or(format!(
        "{path}: a script is a JSON list of {{\"command\", \"params\"}} steps"
    ))
}

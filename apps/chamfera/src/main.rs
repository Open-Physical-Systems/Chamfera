//! The Chamfera desktop app.
//!
//! ```text
//! chamfera [part.stl ...] [--sample] [--theme light|dark] [--screenshot shot.png]
//! ```

// Release builds on Windows open no console window; chamfera-cli stays a console app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

const USAGE: &str =
    "usage: chamfera [part.stl ...] [--sample] [--theme light|dark] [--screenshot shot.png]";

fn main() -> ExitCode {
    let mut options = chamfera_ui::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => options.sample = true,
            "--parameters" => options.parameters = true,
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

/// Reads a `chamfera-cli run` style script: a JSON list of {"command", "params"}.
fn read_script(path: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let steps = v
        .as_array()
        .ok_or(format!("{path}: a script is a JSON list"))?;
    steps
        .iter()
        .map(|s| {
            let id = s["command"]
                .as_str()
                .ok_or(format!("{path}: every step needs a \"command\""))?;
            let params = s.get("params").cloned().unwrap_or(serde_json::json!({}));
            Ok((id.to_owned(), params))
        })
        .collect()
}

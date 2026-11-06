//! `chamfera-cli`: run Chamfera without a window.
//!
//! ```text
//! chamfera-cli commands                 list every command
//! chamfera-cli exec <id> [json]         run one command, print its reply
//! chamfera-cli run <script.json>        run a script: a JSON list of {"command", "params"}
//! chamfera-cli snapshot [options]       render the viewport to a PNG, no window needed
//! ```

use chamfera_engine::{Session, registry};
use serde_json::{Value, json};
use std::process::ExitCode;

/// Largest script file accepted (bytes).
const MAX_SCRIPT_BYTES: u64 = 64 * 1024 * 1024;

const USAGE: &str = "usage:
  chamfera-cli commands
  chamfera-cli exec <command-id> [json-params]
  chamfera-cli run <script.json>
  chamfera-cli snapshot --out shot.png [--sample] [--stl part.stl]... [--script s.json]
                        [--view iso|front|back|left|right|top|bottom]
                        [--projection perspective|orthographic] [--theme light|dark]
                        [--size 1280x800] [--pick X,Y]
  chamfera-cli --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("commands") => {
            for c in registry() {
                println!("{:<24} {}", c.id, c.label);
            }
            Ok(())
        }
        Some("exec") => {
            let id = args.get(1).ok_or(USAGE)?;
            let params: Value = match args.get(2) {
                Some(text) => serde_json::from_str(text).map_err(|e| format!("params: {e}"))?,
                None => json!({}),
            };
            let reply = Session::new()
                .execute(id, &params)
                .map_err(|e| e.to_string())?;
            println!("{reply}");
            Ok(())
        }
        Some("run") => run_script(args.get(1).ok_or(USAGE)?, &mut Session::new()).map(|_| ()),
        Some("snapshot") => snapshot(&args[1..]),
        Some("--version") => {
            println!("chamfera-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// Runs each step in order and prints one JSON reply per line. Stops at the first failure.
fn run_script(path: &str, session: &mut Session) -> Result<(), String> {
    let bytes = chamfera_engine::read_limited(std::path::Path::new(path), MAX_SCRIPT_BYTES)
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))?;
    let script: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let steps = script
        .as_array()
        .ok_or("a script is a JSON list of {\"command\": ..., \"params\": {...}}")?;
    for (i, step) in steps.iter().enumerate() {
        let id = step
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("step {}: `command` must be a string", i + 1))?;
        let params = step.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = session
            .execute(id, &params)
            .map_err(|e| format!("step {} ({id}): {e}", i + 1))?;
        println!(
            "{}",
            json!({ "step": i + 1, "command": id, "reply": reply })
        );
    }
    Ok(())
}

/// `snapshot`: build a scene from the options, run optional view commands, render a PNG.
fn snapshot(args: &[String]) -> Result<(), String> {
    let mut session = Session::new();
    let mut out = None;
    let (mut width, mut height) = (1280.0, 800.0);
    let mut view = None;
    let mut projection = None;
    let mut theme = chamfera_render::Theme::light();
    let mut pick = None;
    let mut it = args.iter();
    let value = |it: &mut std::slice::Iter<String>, flag: &str| {
        it.next().cloned().ok_or(format!("{flag} needs a value"))
    };
    let run =
        |s: &mut Session, id: &str, p: Value| s.execute(id, &p).map_err(|e| format!("{id}: {e}"));
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--out" => out = Some(value(&mut it, flag)?),
            "--sample" => {
                run(&mut session, "body.sample", json!({}))?;
            }
            "--stl" => {
                let path = value(&mut it, flag)?;
                run(&mut session, "body.import_stl", json!({ "path": path }))?;
            }
            "--script" => run_script(&value(&mut it, flag)?, &mut session)?,
            "--view" => view = Some(value(&mut it, flag)?),
            "--projection" => projection = Some(value(&mut it, flag)?),
            "--theme" => {
                theme = match value(&mut it, flag)?.as_str() {
                    "light" => chamfera_render::Theme::light(),
                    "dark" => chamfera_render::Theme::dark(),
                    other => return Err(format!("unknown theme {other:?}: use light or dark")),
                }
            }
            "--size" => {
                let v = value(&mut it, flag)?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or("--size is WIDTHxHEIGHT, like 1280x800")?;
                width = w.parse().map_err(|_| "--size width must be a number")?;
                height = h.parse().map_err(|_| "--size height must be a number")?;
            }
            "--pick" => {
                let v = value(&mut it, flag)?;
                let (x, y) = v.split_once(',').ok_or("--pick is X,Y in pixels")?;
                let x: f64 = x.parse().map_err(|_| "--pick X must be a number")?;
                let y: f64 = y.parse().map_err(|_| "--pick Y must be a number")?;
                pick = Some((x, y));
            }
            other => return Err(format!("unknown option {other:?}\n{USAGE}")),
        }
    }
    let out = out.ok_or("snapshot needs --out <file.png>")?;
    let viewport = json!({ "width": width, "height": height });
    if let Some(p) = projection {
        run(&mut session, "view.projection", json!({ "mode": p }))?;
    }
    match view.as_deref() {
        Some("iso") | None => {
            run(&mut session, "view.home", viewport.clone())?;
        }
        Some(v) => {
            run(&mut session, "view.standard", json!({ "view": v }))?;
            run(&mut session, "view.fit", viewport.clone())?;
        }
    }
    if let Some((x, y)) = pick {
        let hit = run(
            &mut session,
            "select.pick",
            json!({ "x": x, "y": y, "width": width, "height": height }),
        )?;
        println!("{}", json!({ "pick": hit }));
    }
    let frame = chamfera_gpu::Frame {
        scene: &session.scene,
        camera: &session.camera,
        theme: &theme,
        width: width as u32,
        height: height as u32,
    };
    let png = chamfera_gpu::snapshot_png(&frame).map_err(|e| e.to_string())?;
    std::fs::write(&out, png).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "{}",
        json!({ "snapshot": out, "width": width, "height": height })
    );
    Ok(())
}

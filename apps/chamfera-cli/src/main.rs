//! `chamfera-cli`: run Chamfera without a window.
//!
//! ```text
//! chamfera-cli commands                 list every command
//! chamfera-cli exec <id> [json]         run one command, print its reply
//! chamfera-cli run <script.json>        run a script: a JSON list of {"command", "params"}
//! chamfera-cli snapshot [options]       render the viewport to a PNG, no window needed
//! chamfera-cli bench [--runs N] [--cam] [s.json] time rebuilds and edits of the examples (release)
//! chamfera-cli mcp [--allow-path DIR]... serve MCP on stdio for AI agents (docs/mcp.md)
//! ```

mod bench;

use chamfera_engine::{Session, registry};
use serde_json::{Value, json};
use std::io::Write;
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
  chamfera-cli bench [--runs N] [--cam] [script.json]...
  chamfera-cli mcp [--allow-path DIR]...
  chamfera-cli --version";

/// Why the CLI stopped early.
#[derive(Debug)]
enum Stop {
    /// Something went wrong: printed to stderr, exit status 1.
    Error(String),
    /// Whoever reads stdout closed it (`chamfera-cli commands | head`): nobody wants the
    /// rest, so stop quietly with status 0.
    PipeClosed,
}

impl From<String> for Stop {
    fn from(msg: String) -> Stop {
        Stop::Error(msg)
    }
}

impl From<&str> for Stop {
    fn from(msg: &str) -> Stop {
        Stop::Error(msg.to_owned())
    }
}

/// Writes one line to stdout. A closed pipe stops the CLI quietly; `println!` would
/// panic there.
fn out(line: impl std::fmt::Display) -> Result<(), Stop> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")
        .and_then(|()| stdout.flush())
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::BrokenPipe => Stop::PipeClosed,
            _ => Stop::Error(format!("stdout: {e}")),
        })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) | Err(Stop::PipeClosed) => ExitCode::SUCCESS,
        Err(Stop::Error(msg)) => {
            // stderr may be closed too; there is nobody left to tell.
            let _ = writeln!(std::io::stderr(), "error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Stop> {
    match args.first().map(String::as_str) {
        Some("commands") => {
            for c in registry() {
                out(format_args!("{:<24} {}", c.id, c.label))?;
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
            out(reply)
        }
        Some("run") => run_script(args.get(1).ok_or(USAGE)?, &mut Session::new()).map(|_| ()),
        Some("snapshot") => snapshot(&args[1..]),
        Some("bench") => bench::bench(&args[1..]),
        Some("mcp") => mcp(&args[1..]),
        Some("--version") => out(format_args!("chamfera-cli {}", env!("CARGO_PKG_VERSION"))),
        _ => Err(USAGE.into()),
    }
}

/// `mcp`: serve the MCP server on stdio. File commands may use only the `--allow-path`
/// folders and a scratch folder.
fn mcp(args: &[String]) -> Result<(), Stop> {
    let mut config = chamfera_mcp::Config::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--allow-path" => config
                .allow
                .push(it.next().ok_or("--allow-path needs a folder")?.into()),
            other => return Err(format!("unknown option {other:?}\n{USAGE}").into()),
        }
    }
    Ok(chamfera_mcp::serve_stdio(config)?)
}

/// A script file's steps.
fn load_script(path: &str) -> Result<Vec<Value>, String> {
    let bytes = chamfera_engine::read_limited(std::path::Path::new(path), MAX_SCRIPT_BYTES)
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))?;
    let script: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    match script {
        Value::Array(steps) => Ok(steps),
        _ => Err("a script is a JSON list of {\"command\": ..., \"params\": {...}}".into()),
    }
}

/// Runs each step in order and prints one JSON reply per line. Stops at the first failure.
fn run_script(path: &str, session: &mut Session) -> Result<(), Stop> {
    let steps = load_script(path)?;
    // Each reply is printed as its step finishes, so the steps before a failure show.
    let mut printed: Result<(), Stop> = Ok(());
    let ran = chamfera_engine::script::run_steps_each(session, &steps, |n, reply| {
        if printed.is_ok() {
            let command = steps.get(n - 1).and_then(|s| s.get("command"));
            printed = out(json!({ "step": n, "command": command, "reply": reply }));
        }
    });
    printed?;
    ran.map_err(|e| e.to_string())?;
    Ok(())
}

/// `snapshot`: build a scene from the options, run optional view commands, render a PNG.
fn snapshot(args: &[String]) -> Result<(), Stop> {
    let mut session = Session::new();
    let mut path = None;
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
            "--out" => path = Some(value(&mut it, flag)?),
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
                    other => {
                        return Err(format!("unknown theme {other:?}: use light or dark").into());
                    }
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
            other => return Err(format!("unknown option {other:?}\n{USAGE}").into()),
        }
    }
    let path = path.ok_or("snapshot needs --out <file.png>")?;
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
        out(json!({ "pick": hit }))?;
    }
    let frame = chamfera_gpu::Frame {
        scene: &session.scene,
        camera: &session.camera,
        theme: &theme,
        width: width as u32,
        height: height as u32,
        scale: 1.0,
    };
    let png = chamfera_gpu::snapshot_png(&frame).map_err(|e| e.to_string())?;
    std::fs::write(&path, png).map_err(|e| format!("{path}: {e}"))?;
    out(json!({ "snapshot": path, "width": width, "height": height }))
}

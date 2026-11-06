//! `chamfera-cli`: run Chamfera without a window.
//!
//! ```text
//! chamfera-cli commands                 list every command
//! chamfera-cli exec <id> [json]         run one command, print its reply
//! chamfera-cli run <script.json>        run a script: a JSON list of {"command", "params"}
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
        Some("run") => run_script(args.get(1).ok_or(USAGE)?),
        Some("--version") => {
            println!("chamfera-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// Runs each step in order and prints one JSON reply per line. Stops at the first failure.
fn run_script(path: &str) -> Result<(), String> {
    let bytes = chamfera_engine::read_limited(std::path::Path::new(path), MAX_SCRIPT_BYTES)
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))?;
    let script: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let steps = script
        .as_array()
        .ok_or("a script is a JSON list of {\"command\": ..., \"params\": {...}}")?;
    let mut session = Session::new();
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

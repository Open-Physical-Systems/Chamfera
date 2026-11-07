//! Repository checks. `cargo xtask ci` runs them all; each rule in AGENTS.md names the check
//! that enforces it.

mod notices;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask <ci|layers|skills|licences|notices [--out FILE]|wasm>";

/// The layer of every crate. A crate may depend only on crates in a lower layer.
/// Keep in step with docs/architecture.md.
const LAYERS: &[(&str, u32)] = &[
    ("chamfera-geom", 0),
    ("chamfera-kernel", 1),
    ("chamfera-render", 1),
    ("chamfera-sketch", 1),
    ("chamfera-doc", 2),
    ("chamfera-io", 3),
    ("chamfera-drawing", 3),
    ("chamfera-cam", 3),
    ("chamfera-raytrace", 3),
    ("chamfera-engine", 4),
    ("chamfera-gpu", 5),
    ("chamfera-assistant", 5),
    ("chamfera-ui", 6),
    ("chamfera-mcp", 6),
    ("chamfera", 7),
    ("chamfera-cli", 7),
    ("chamfera-web", 7),
];

/// Window, GPU and dialog crates: only layer 5 (gpu) and up may use them.
const UI_ONLY_DEPS: &[&str] = &["egui", "eframe", "winit", "wgpu", "rfd", "egui-wgpu"];

/// Crates that build for the browser (wasm32-unknown-unknown).
const WASM_CRATES: &[&str] = &[
    "chamfera-geom",
    "chamfera-sketch",
    "chamfera-kernel",
    "chamfera-render",
    "chamfera-doc",
    "chamfera-io",
    "chamfera-drawing",
    "chamfera-cam",
    "chamfera-raytrace",
    "chamfera-engine",
    "chamfera-gpu",
    "chamfera-assistant",
    "chamfera-ui",
    "chamfera-web",
];

/// Subcommands, so `cargo xtask skills` can check that docs name only real ones.
const SUBCOMMANDS: &[&str] = &["ci", "layers", "skills", "licences", "notices", "wasm"];

type CheckResult = Result<(), String>;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let root = match repo_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("xtask: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match task.as_deref() {
        Some("ci") => ci(&root),
        Some("layers") => layers(&root),
        Some("skills") => skills(&root),
        Some("licences") => licences(&root),
        Some("notices") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            match args.as_slice() {
                [] => notices::notices(&root, None),
                [flag, path] if flag == "--out" => notices::notices(&root, Some(path)),
                _ => Err(USAGE.to_owned()),
            }
        }
        Some("wasm") => wasm(&root),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| "run through cargo xtask")?;
    Path::new(&manifest)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask has no parent folder".to_owned())
}

fn run(root: &Path, program: &str, args: &[&str]) -> CheckResult {
    println!("==> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed", args.join(" ")))
    }
}

fn ci(root: &Path) -> CheckResult {
    run(root, "cargo", &["fmt", "--all", "--check"])?;
    run(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(root, "cargo", &["test", "--workspace", "--quiet"])?;
    println!("==> layers");
    layers(root)?;
    println!("==> skills");
    skills(root)?;
    licences(root)?;
    // Every shipped crate has its licence text (target/THIRD-PARTY-LICENSES.txt).
    notices::notices(root, None)?;
    wasm(root)?;
    println!("ci: all checks passed");
    Ok(())
}

/// Every member crate has a layer, and depends only on lower layers.
fn layers(root: &Path) -> CheckResult {
    let table: BTreeMap<&str, u32> = LAYERS.iter().copied().collect();
    let mut errors = Vec::new();
    for dir in ["crates", "apps"] {
        let entries = std::fs::read_dir(root.join(dir)).map_err(|e| format!("{dir}: {e}"))?;
        for entry in entries.flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if !manifest.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&manifest)
                .map_err(|e| format!("{}: {e}", manifest.display()))?;
            let value: toml::Table =
                toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
            let name = value
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| format!("{} has no package name", manifest.display()))?;
            let Some(&layer) = table.get(name) else {
                errors.push(format!("{name} has no layer: add it to LAYERS in xtask"));
                continue;
            };
            let deps = value
                .get("dependencies")
                .and_then(|d| d.as_table())
                .map(|t| t.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for dep in deps {
                if let Some(&dep_layer) = table.get(dep.as_str())
                    && dep_layer >= layer
                {
                    errors.push(format!(
                        "{name} (layer {layer}) depends on {dep} (layer {dep_layer})"
                    ));
                }
                if layer < 5 && UI_ONLY_DEPS.contains(&dep.as_str()) {
                    errors.push(format!("{name} (layer {layer}) uses UI-only crate {dep}"));
                }
            }
        }
    }
    report(errors, "layers")
}

/// AGENTS.md and every SKILL.md name only paths, skills and xtask commands that exist, and
/// AGENTS.md routes to every skill.
fn skills(root: &Path) -> CheckResult {
    let mut errors = Vec::new();
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).map_err(|e| e.to_string())?;
    let mut docs = vec![("AGENTS.md".to_owned(), agents.clone())];

    let skill_dir = root.join("skills");
    let mut skill_names = Vec::new();
    for entry in std::fs::read_dir(&skill_dir)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path().join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if !text.starts_with("---\n") || !text.contains("\ndescription:") {
            errors.push(format!(
                "skills/{dir_name}/SKILL.md lacks front matter with a description"
            ));
        }
        if !text.contains(&format!("\nname: {dir_name}\n")) {
            errors.push(format!(
                "skills/{dir_name}/SKILL.md: `name:` must be {dir_name}"
            ));
        }
        let rel = format!("skills/{dir_name}/SKILL.md");
        if !agents.contains(&rel) {
            errors.push(format!("AGENTS.md does not route to {rel}"));
        }
        skill_names.push(dir_name);
        docs.push((rel, text));
    }
    if skill_names.is_empty() {
        errors.push("no skills found in skills/".into());
    }

    for (doc, text) in &docs {
        for token in backticked(text) {
            if let Some(cmd) = token.strip_prefix("cargo xtask ") {
                let sub = cmd.split_whitespace().next().unwrap_or("");
                if !SUBCOMMANDS.contains(&sub) {
                    errors.push(format!(
                        "{doc} names `cargo xtask {sub}`, which does not exist"
                    ));
                }
            } else if looks_like_repo_path(&token)
                && !root.join(token.trim_end_matches('/')).exists()
            {
                errors.push(format!("{doc} names `{token}`, which does not exist"));
            }
        }
    }
    report(errors, "skills")
}

/// Inline `code` spans in a Markdown text.
fn backticked(text: &str) -> Vec<String> {
    text.split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.trim().to_owned())
        .collect()
}

/// Repository paths are written relative to the root and start with a known folder or file.
fn looks_like_repo_path(token: &str) -> bool {
    const ROOTS: &[&str] = &[
        "crates/", "apps/", "skills/", "docs/", "xtask/", "tests/", ".github/",
    ];
    const FILES: &[&str] = &[
        "AGENTS.md",
        "CLAUDE.md",
        "PLAN.md",
        "deny.toml",
        "Cargo.toml",
    ];
    !token.contains(' ')
        && !token.contains('<')
        && !token.contains('*')
        && (ROOTS.iter().any(|r| token.starts_with(r)) || FILES.contains(&token))
}

fn report(errors: Vec<String>, check: &str) -> CheckResult {
    if errors.is_empty() {
        println!("{check}: ok");
        Ok(())
    } else {
        for e in &errors {
            eprintln!("  {e}");
        }
        Err(format!("{check}: {} problem(s)", errors.len()))
    }
}

/// Only permissive licences, no unknown sources (deny.toml).
fn licences(root: &Path) -> CheckResult {
    let has_deny = Command::new("cargo")
        .args(["deny", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_deny {
        return Err("cargo-deny is missing: cargo install --locked cargo-deny".into());
    }
    run(
        root,
        "cargo",
        &[
            "deny",
            "--log-level",
            "error",
            "check",
            "licenses",
            "bans",
            "sources",
        ],
    )
}

/// The libraries below the UI build for the browser.
fn wasm(root: &Path) -> CheckResult {
    let mut args = vec!["build", "--quiet", "--target", "wasm32-unknown-unknown"];
    for c in WASM_CRATES {
        args.push("-p");
        args.push(c);
    }
    run(root, "cargo", &args)
}

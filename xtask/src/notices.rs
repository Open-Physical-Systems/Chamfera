//! `cargo xtask notices`: the licence texts of every third-party crate that ships inside the
//! apps (`chamfera`, `chamfera-cli`, `chamfera-web`), gathered from the crates' own sources
//! into one text file. The packages ship it next to `ATTRIBUTION.md`, as MIT, BSD, ISC and
//! Apache-2.0 ask (keep the copyright and licence notices with every copy).
//!
//! Shipped means: reachable from an app over normal (not build or dev) dependencies, on any
//! platform, and not a procedural macro (those run in the compiler and are not linked).

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The apps whose dependencies ship.
pub const APPS: &[&str] = &["chamfera", "chamfera-cli", "chamfera-web"];

/// The standard MIT licence text, for a crate that names MIT but ships no licence file.
const MIT_TEXT: &str = "Permission is hereby granted, free of charge, to any person obtaining a copy \
of this software and associated documentation files (the \"Software\"), to deal in the Software \
without restriction, including without limitation the rights to use, copy, modify, merge, \
publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to \
whom the Software is furnished to do so, subject to the following conditions:\n\n\
The above copyright notice and this permission notice shall be included in all copies or \
substantial portions of the Software.\n\n\
THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, \
INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR \
PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR \
ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, \
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE \
SOFTWARE.\n";

/// One shipped third-party crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipped {
    pub name: String,
    pub version: String,
    pub licence: String,
    pub authors: Vec<String>,
    pub dir: PathBuf,
    /// `license-file` from the crate's manifest, relative to `dir`.
    pub licence_file: Option<String>,
}

/// The third-party crates the apps ship, sorted by name and version, from `cargo metadata`
/// output.
pub fn shipped(metadata: &Value) -> Result<Vec<Shipped>, String> {
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?;
    let by_id: BTreeMap<&str, &Value> = packages
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("cargo metadata: no resolve graph")?;
    let deps: BTreeMap<&str, Vec<&str>> = nodes
        .iter()
        .filter_map(|n| {
            let id = n["id"].as_str()?;
            let normal = n["deps"]
                .as_array()?
                .iter()
                .filter(|d| {
                    d["dep_kinds"]
                        .as_array()
                        .is_some_and(|k| k.iter().any(|k| k["kind"].is_null()))
                })
                .filter_map(|d| d["pkg"].as_str())
                .collect();
            Some((id, normal))
        })
        .collect();

    let mut queue: VecDeque<&str> = by_id
        .iter()
        .filter(|(_, p)| {
            p["source"].is_null() && p["name"].as_str().is_some_and(|n| APPS.contains(&n))
        })
        .map(|(id, _)| *id)
        .collect();
    if queue.len() != APPS.len() {
        return Err(format!("cargo metadata: expected the apps {APPS:?}"));
    }
    let mut seen: BTreeSet<&str> = queue.iter().copied().collect();
    let mut out = Vec::new();
    while let Some(id) = queue.pop_front() {
        let Some(p) = by_id.get(id) else { continue };
        let proc_macro = p["targets"].as_array().is_some_and(|ts| {
            ts.iter().any(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "proc-macro"))
            })
        });
        if proc_macro {
            continue;
        }
        if !p["source"].is_null() {
            let manifest = p["manifest_path"]
                .as_str()
                .ok_or("a package has no manifest")?;
            out.push(Shipped {
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                version: p["version"].as_str().unwrap_or_default().to_owned(),
                licence: p["license"].as_str().unwrap_or_default().to_owned(),
                authors: p["authors"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                dir: Path::new(manifest)
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
                licence_file: p["license_file"].as_str().map(str::to_owned),
            });
        }
        for d in deps.get(id).into_iter().flatten() {
            if seen.insert(d) {
                queue.push_back(d);
            }
        }
    }
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(out)
}

/// Whether a file name is a licence, notice or copyright file.
pub fn is_licence_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "LICENSE",
        "LICENCE",
        "COPYING",
        "NOTICE",
        "COPYRIGHT",
        "UNLICENSE",
    ]
    .iter()
    .any(|p| upper.starts_with(p))
}

/// The licence files of a crate: its top-level licence files, its `license-file`, the
/// licences of the fonts the font crate bundles, and those of code ring took from once_cell.
fn licence_files(c: &Shipped) -> Vec<PathBuf> {
    let mut files = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(&c.dir) {
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_uppercase();
            // A copyleft option of a dual-licensed crate is never the one we take.
            if path.is_file() && is_licence_file(&name) && !name.contains("GPL") {
                files.insert(path);
            }
        }
    }
    if let Some(f) = &c.licence_file {
        let path = c.dir.join(f);
        if path.is_file() {
            files.insert(path);
        }
    }
    // Code ring took from once_cell, with its own licence files.
    if c.name == "ring" {
        for f in ["LICENSE-MIT", "LICENSE-APACHE"] {
            let path = c.dir.join("src/polyfill/once_cell").join(f);
            if path.is_file() {
                files.insert(path);
            }
        }
    }
    if c.name == "epaint_default_fonts"
        && let Ok(entries) = std::fs::read_dir(c.dir.join("fonts"))
    {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "txt") {
                files.insert(e.path());
            }
        }
    }
    files.into_iter().collect()
}

/// The notices file for these crates. Identical texts are written once, under the list of
/// crates that use them. A crate without licence files gets the standard text of the
/// licences it names (MIT here; Apache-2.0 and the rest are given once at the end).
pub fn render(crates: &[Shipped], apache: &str) -> Result<String, String> {
    let mut texts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut missing = Vec::new();
    for c in crates {
        let label = format!("{} {}", c.name, c.version);
        let files = licence_files(c);
        if files.is_empty() {
            if c.licence.is_empty() {
                missing.push(label);
                continue;
            }
            let who = if c.authors.is_empty() {
                format!("the {} authors", c.name)
            } else {
                c.authors.join(", ")
            };
            let text = if c.licence.contains("MIT") {
                format!(
                    "{} is licensed under {}. No licence file ships with it; the MIT terms:\n\n\
                     Copyright (c) {who}\n\n{MIT_TEXT}",
                    c.name, c.licence
                )
            } else {
                format!(
                    "{} is licensed under {} (copyright {who}). No licence file ships with \
                     it; the standard text is at the end of this file or at \
                     https://spdx.org/licenses/.\n",
                    c.name, c.licence
                )
            };
            texts.entry(text).or_default().push(label);
            continue;
        }
        for f in files {
            let text = std::fs::read_to_string(&f)
                .map_err(|e| format!("{}: {e}", f.display()))?
                .replace("\r\n", "\n");
            let file = f.file_name().unwrap_or_default().to_string_lossy();
            texts
                .entry(text)
                .or_default()
                .push(format!("{label} ({file})"));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "no licence and no licence file: {}",
            missing.join(", ")
        ));
    }

    let rule = "=".repeat(78);
    let mut s = String::new();
    s.push_str(
        "Third-party licences\n\n\
         Chamfera is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). The apps include\n\
         the third-party Rust crates below, under their own licences.\n\
         Fonts and icons: ATTRIBUTION.md.\n\
         Generated by `cargo xtask notices` from Cargo.lock; do not edit by hand.\n\n",
    );
    s.push_str(&format!("{rule}\nCrates ({})\n{rule}\n", crates.len()));
    for c in crates {
        s.push_str(&format!("{} {}: {}\n", c.name, c.version, c.licence));
    }
    for (text, users) in &texts {
        s.push_str(&format!("\n{rule}\n"));
        for u in users {
            s.push_str(u);
            s.push('\n');
        }
        s.push_str(&format!("{rule}\n\n"));
        s.push_str(text.trim_end());
        s.push('\n');
    }
    s.push_str(&format!(
        "\n{rule}\nApache License 2.0 (the full text, for crates that name it)\n{rule}\n\n"
    ));
    s.push_str(apache.trim_end());
    s.push('\n');
    Ok(s)
}

/// Writes the notices file (default `target/THIRD-PARTY-LICENSES.txt`).
pub fn notices(root: &Path, out: Option<&str>) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not start cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata: bad JSON: {e}"))?;
    let crates = shipped(&metadata)?;
    let apache = std::fs::read_to_string(root.join("LICENSE-APACHE"))
        .map_err(|e| format!("LICENSE-APACHE: {e}"))?;
    let mut text = render(&crates, &apache)?;
    // The font Chamfera bundles itself (sketch text), with its licence as the OFL asks.
    let lato = std::fs::read_to_string(root.join("assets/fonts/OFL-Lato.txt"))
        .map_err(|e| format!("assets/fonts/OFL-Lato.txt: {e}"))?;
    text.push_str(
        "\n================================================================\n\
         Bundled font: Lato Regular (assets/fonts/Lato-Regular.ttf), used for sketch text\n\
         ================================================================\n\n",
    );
    text.push_str(&lato);
    let path = match out {
        Some(p) => PathBuf::from(p),
        None => root.join("target").join("THIRD-PARTY-LICENSES.txt"),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "notices: {} third-party crates, written to {}",
        crates.len(),
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn normal(pkg: &str) -> Value {
        json!({ "pkg": pkg, "dep_kinds": [{ "kind": null, "target": null }] })
    }

    fn package(id: &str, source: Option<&str>, kind: &str) -> Value {
        json!({
            "id": id, "name": id, "version": "1.0.0", "license": "MIT",
            "authors": ["A. Author"], "source": source,
            "manifest_path": format!("/reg/{id}/Cargo.toml"),
            "targets": [{ "kind": [kind] }],
        })
    }

    /// The three apps; `chamfera` uses `lib` (normal) and `builder` (build only), `lib`
    /// uses the macro `derive` and the crate `inner`, `derive` uses `syn`. Only `lib` and
    /// `inner` ship: build dependencies and macros (with what they use) do not.
    fn metadata() -> Value {
        let reg = Some("registry+https://github.com/rust-lang/crates.io-index");
        json!({
            "packages": [
                package("chamfera", None, "bin"),
                package("chamfera-cli", None, "bin"),
                package("chamfera-web", None, "cdylib"),
                package("lib", reg, "lib"),
                package("inner", reg, "lib"),
                package("builder", reg, "lib"),
                package("derive", reg, "proc-macro"),
                package("syn", reg, "lib"),
                package("unused", reg, "lib"),
            ],
            "resolve": { "nodes": [
                { "id": "chamfera", "deps": [
                    normal("lib"),
                    { "pkg": "builder", "dep_kinds": [{ "kind": "build", "target": null }] },
                ]},
                { "id": "chamfera-cli", "deps": [] },
                { "id": "chamfera-web", "deps": [normal("lib")] },
                { "id": "lib", "deps": [normal("derive"), normal("inner")] },
                { "id": "derive", "deps": [normal("syn")] },
                { "id": "inner", "deps": [] },
                { "id": "builder", "deps": [] },
                { "id": "syn", "deps": [] },
                { "id": "unused", "deps": [] },
            ]},
        })
    }

    #[test]
    fn only_normal_non_macro_dependencies_of_the_apps_ship() {
        let names: Vec<String> = shipped(&metadata())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["inner", "lib"]);
    }

    #[test]
    fn licence_files_are_recognised_by_name() {
        for yes in [
            "LICENSE",
            "LICENSE-MIT",
            "license-apache",
            "LICENCE.txt",
            "COPYING",
            "NOTICE",
            "UNLICENSE",
        ] {
            assert!(is_licence_file(yes), "{yes}");
        }
        for no in ["README.md", "Cargo.toml", "src", "build.rs"] {
            assert!(!is_licence_file(no), "{no}");
        }
    }

    #[test]
    fn a_crate_without_licence_files_gets_the_standard_text_with_its_authors() {
        let c = Shipped {
            name: "tiny".into(),
            version: "0.1.0".into(),
            licence: "MIT OR Apache-2.0".into(),
            authors: vec!["Jo Bloggs".into()],
            dir: PathBuf::from("/nonexistent/tiny"),
            licence_file: None,
        };
        let s = render(&[c.clone(), c], "APACHE TEXT").unwrap();
        assert!(s.contains("tiny 0.1.0: MIT OR Apache-2.0\n"));
        assert!(s.contains("Copyright (c) Jo Bloggs"));
        assert!(s.contains("The above copyright notice and this permission notice"));
        // The same text is written once, under both users.
        assert_eq!(s.matches("Copyright (c) Jo Bloggs").count(), 1);
        assert!(s.trim_end().ends_with("APACHE TEXT"));
    }

    #[test]
    fn a_crate_with_neither_licence_nor_file_is_an_error() {
        let c = Shipped {
            name: "mystery".into(),
            version: "1.0.0".into(),
            licence: String::new(),
            authors: vec![],
            dir: PathBuf::from("/nonexistent/mystery"),
            licence_file: None,
        };
        let e = render(&[c], "").unwrap_err();
        assert!(e.contains("mystery 1.0.0"), "{e}");
    }
}

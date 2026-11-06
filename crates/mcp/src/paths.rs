//! File safety: commands that take a path may only touch the allowed folders.
//!
//! The CLI runs as the user and may use any path. Over MCP an agent names the paths, so
//! each one is resolved before the command runs: `..` is refused outright, relative paths
//! go to the scratch folder, and the path is canonicalized (following every symlink) and
//! must then lie inside an allowed folder or the scratch folder. The command receives the
//! canonical path. Any command whose parameters mention a path but which is not listed in
//! [`PATH_COMMANDS`] is refused, so a new file command cannot slip past these rules.
//!
//! Limit: a symlink swapped in between the check and the command's own open is not
//! caught; the folders are the user's own, so that needs someone already inside them.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

pub(crate) use chamfera_engine::agent::takes_path;
pub use chamfera_engine::agent::{Access, PATH_COMMANDS};

/// Longest path accepted (bytes).
const MAX_PATH_BYTES: usize = 4096;

/// The allowed folders, canonical.
#[derive(Debug, Clone)]
pub struct PathGuard {
    scratch: PathBuf,
    allowed: Vec<PathBuf>,
}

/// Makes a private scratch folder in the system temp folder, one per server process.
pub(crate) fn make_scratch() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("chamfera-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not make the scratch folder {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not make {} private: {e}", dir.display()))?;
    }
    Ok(dir)
}

fn canonical_dir(dir: &Path) -> Result<PathBuf, String> {
    let c = dir
        .canonicalize()
        .map_err(|e| format!("folder {}: {e}", dir.display()))?;
    if !c.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    Ok(c)
}

impl PathGuard {
    /// Canonicalizes the folders. Each must exist.
    pub fn new(allow: &[PathBuf], scratch: &Path) -> Result<Self, String> {
        let scratch = canonical_dir(scratch)?;
        let mut allowed = vec![scratch.clone()];
        for dir in allow {
            let c = canonical_dir(dir).map_err(|e| format!("--allow-path: {e}"))?;
            if !allowed.contains(&c) {
                allowed.push(c);
            }
        }
        Ok(Self { scratch, allowed })
    }

    pub fn scratch(&self) -> &Path {
        &self.scratch
    }

    /// The scratch folder first, then the `--allow-path` folders.
    pub fn allowed(&self) -> &[PathBuf] {
        &self.allowed
    }

    /// The folders, for messages.
    pub fn describe(&self) -> String {
        self.allowed
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn inside(&self, p: &Path) -> bool {
        // `starts_with` compares whole components: /a/bc is not inside /a/b.
        self.allowed.iter().any(|root| p.starts_with(root))
    }

    fn outside(&self, raw: &str) -> String {
        format!(
            "refused: `{raw}` is outside the folders this server may use ({}); relative \
             paths go to the scratch folder",
            self.describe()
        )
    }

    /// Resolves a path an agent gave to a canonical path inside the allowed folders.
    pub fn resolve(&self, raw: &str, access: Access) -> Result<PathBuf, String> {
        if raw.is_empty() {
            return Err("refused: the path is empty".into());
        }
        if raw.len() > MAX_PATH_BYTES || raw.contains('\0') {
            return Err(format!(
                "refused: a path is at most {MAX_PATH_BYTES} bytes, without NUL characters"
            ));
        }
        let given = Path::new(raw);
        if given.components().any(|c| c == Component::ParentDir) {
            return Err(format!(
                "refused: `{raw}` uses `..`; name the file directly"
            ));
        }
        let full = if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.scratch.join(given)
        };
        let resolved = match access {
            Access::Read => full
                .canonicalize()
                .map_err(|e| format!("cannot read `{raw}`: {e}"))?,
            Access::Write => {
                if std::fs::symlink_metadata(&full).is_ok() {
                    // It exists (maybe as a symlink): check where it really is. A dangling
                    // symlink fails here, so a write cannot create a file through it.
                    full.canonicalize()
                        .map_err(|e| format!("refused: `{raw}` cannot be resolved: {e}"))?
                } else {
                    let name = match full.components().next_back() {
                        Some(Component::Normal(n)) => n.to_owned(),
                        _ => return Err(format!("refused: `{raw}` does not name a file")),
                    };
                    let parent = full
                        .parent()
                        .ok_or_else(|| format!("refused: `{raw}` has no folder"))?;
                    parent
                        .canonicalize()
                        .map_err(|e| format!("cannot write `{raw}`: its folder: {e}"))?
                        .join(name)
                }
            }
        };
        if !self.inside(&resolved) {
            return Err(self.outside(raw));
        }
        Ok(resolved)
    }

    /// Checks a command's parameters before it runs. Returns the parameters to run with
    /// (the path replaced by its canonical form), or why the command is refused.
    pub fn check_command(&self, id: &str, params: &Value) -> Result<Value, String> {
        let Some(&(_, access)) = PATH_COMMANDS.iter().find(|(c, _)| *c == id) else {
            let takes_path = chamfera_engine::cmd::find(id).is_some_and(|c| takes_path(c.params));
            if takes_path {
                return Err(format!(
                    "refused: `{id}` takes a file path and is not enabled over MCP"
                ));
            }
            return Ok(params.clone());
        };
        // A missing or non-string path (or folder) is left for the command to reject.
        let mut out = params.clone();
        for key in ["path", "dir"] {
            let Some(raw) = params.get(key).and_then(Value::as_str) else {
                continue;
            };
            let resolved = self.resolve(raw, access)?;
            let text = resolved
                .to_str()
                .ok_or_else(|| format!("refused: `{raw}` resolves to a path that is not UTF-8"))?
                .to_owned();
            if let Some(obj) = out.as_object_mut() {
                obj.insert(key.into(), Value::String(text));
            }
        }
        Ok(out)
    }
}

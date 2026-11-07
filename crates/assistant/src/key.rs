//! The API key. It comes from the `ANTHROPIC_API_KEY` environment variable or from a key
//! file of its own in the user's config folder, readable by the user only (0600 on Unix),
//! never from or into a design. [`ApiKey`] has no `Display` or serde, and its `Debug`
//! shows only the last four characters, so it cannot end up in a log by accident.

/// The environment variable read for the key.
pub const ENV_VAR: &str = "ANTHROPIC_API_KEY";
/// Longest key accepted (bytes).
pub const MAX_KEY_BYTES: usize = 512;

/// A checked API key.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

/// Where the key came from, for the Preferences page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Environment,
    File,
}

impl ApiKey {
    /// Checks a key: 8 to 512 visible ASCII characters, no spaces. Surrounding white
    /// space (a pasted line ending) is dropped.
    pub fn new(raw: &str) -> Result<ApiKey, String> {
        let k = raw.trim();
        if k.len() < 8 || k.len() > MAX_KEY_BYTES {
            return Err(format!("an API key has 8 to {MAX_KEY_BYTES} characters"));
        }
        if !k.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("an API key has only letters, digits and punctuation, no spaces".into());
        }
        Ok(ApiKey(k.to_owned()))
    }

    /// The key itself, for the request header only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, as "…abcd", to show which key is set.
    pub fn hint(&self) -> String {
        let tail: String = self
            .0
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    }

    /// The key from `ANTHROPIC_API_KEY`, if it is set and valid.
    pub fn from_env() -> Option<ApiKey> {
        std::env::var(ENV_VAR)
            .ok()
            .and_then(|k| ApiKey::new(&k).ok())
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ApiKey({})", self.hint())
    }
}

/// Reads the key file. A missing, unreadable or invalid file gives `None`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &std::path::Path) -> Option<ApiKey> {
    let bytes = chamfera_engine::read_limited(path, MAX_KEY_BYTES as u64 + 64).ok()?;
    ApiKey::new(std::str::from_utf8(&bytes).ok()?).ok()
}

/// Writes the key file, readable and writable by the user only, creating its folder.
/// The file is made private before the key is written into it.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &std::path::Path, key: &ApiKey) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(key.expose().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Removes the key file; a missing one is fine.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

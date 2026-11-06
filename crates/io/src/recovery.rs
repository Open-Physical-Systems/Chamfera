//! Recovery files: the unsaved design, written every so often while it has changes, so a
//! crash costs at most that much work.
//!
//! Each running app has a key (`<pid>-<milliseconds>`) and writes two files into the
//! recovery folder (in the user's data folder): `<key>.chamfera`, the design, and
//! `<key>.json`, which file the work belongs to and when it was written. Both are written
//! atomically ([`crate::atomic`]), the design first. A clean save or quit removes them; a
//! crash leaves them for the next start to offer.

use crate::IoError;
use crate::atomic::write_atomic;
use chamfera_doc::Document;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The most folder entries looked at.
pub const MAX_ENTRIES: usize = 10_000;
/// Largest metadata file read (bytes).
pub const MAX_META_BYTES: u64 = 64 * 1024;
/// Longest key.
const MAX_KEY: usize = 64;
/// Longest path kept.
const MAX_PATH: usize = 4096;

/// A recovery file found at start.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    /// When it was written, milliseconds since 1970 UTC.
    pub time_ms: u64,
    /// The process that wrote it.
    pub pid: u32,
    /// The design file the work belongs to, or `None` for a design never saved.
    pub design_path: Option<PathBuf>,
    /// The recovery design file.
    pub file: PathBuf,
    pub bytes: u64,
}

impl Entry {
    /// What the prompt calls the design.
    pub fn name(&self) -> String {
        self.design_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Untitled".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            )
    }

    /// When it was written, seconds since 1970.
    pub fn time(&self) -> u64 {
        self.time_ms / 1000
    }
}

/// A key is 1 to 64 ASCII letters, digits and `-`.
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Milliseconds since 1970 now.
pub fn now_ms() -> u64 {
    crate::atomic::now_ms()
}

/// A key for this run of the app.
pub fn new_key() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static RUN: AtomicU32 = AtomicU32::new(0);
    let n = RUN.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{n}", crate::atomic::pid(), now_ms())
}

fn check_folder(dir: &Path) -> Result<bool, IoError> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(m) if m.is_dir() => Ok(true),
        Ok(_) => Err(IoError::Format(format!(
            "{} is not a folder (a recovery folder cannot be a link)",
            dir.display()
        ))),
    }
}

fn paths(dir: &Path, key: &str) -> Result<(PathBuf, PathBuf), IoError> {
    if !is_valid_key(key) {
        return Err(IoError::Format(format!("{key:?} is not a recovery key")));
    }
    Ok((
        dir.join(format!("{key}.chamfera")),
        dir.join(format!("{key}.json")),
    ))
}

/// Writes the design's recovery files (the folder is made if missing).
pub fn write(
    dir: &Path,
    key: &str,
    doc: &Document,
    design_path: Option<&Path>,
) -> Result<Entry, IoError> {
    let (file, meta) = paths(dir, key)?;
    if !check_folder(dir)? {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let text = doc.to_json().map_err(|e| IoError::Format(e.to_string()))?;
    write_atomic(&file, text.as_bytes())?;
    let time_ms = now_ms();
    let design = design_path
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| p.len() <= MAX_PATH);
    let m = json!({
        "format": "chamfera-recovery",
        "version": 1,
        "key": key,
        "time_ms": time_ms,
        "pid": crate::atomic::pid(),
        "design_path": design,
    });
    write_atomic(
        &meta,
        serde_json::to_string_pretty(&m)
            .unwrap_or_default()
            .as_bytes(),
    )?;
    Ok(Entry {
        key: key.to_owned(),
        time_ms,
        pid: crate::atomic::pid(),
        design_path: design.map(PathBuf::from),
        file,
        bytes: text.len() as u64,
    })
}

/// Removes a key's recovery files (missing ones are fine).
pub fn remove(dir: &Path, key: &str) -> Result<(), IoError> {
    let (file, meta) = paths(dir, key)?;
    for p in [meta, file] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn parse_meta(text: &str, key: &str) -> Result<(u64, u32, Option<PathBuf>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("format").and_then(Value::as_str) != Some("chamfera-recovery") {
        return Err("not a recovery file".into());
    }
    if v.get("key").and_then(Value::as_str) != Some(key) {
        return Err("its key does not match its file name".into());
    }
    let time = v.get("time_ms").and_then(Value::as_u64).ok_or("no time")?;
    let pid = v
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .unwrap_or(0);
    let design = v
        .get("design_path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty() && p.len() <= MAX_PATH && !p.contains('\0'))
        .map(PathBuf::from);
    Ok((time, pid, design))
}

/// The recovery files in the folder, newest first, and the entries skipped (broken ones,
/// with the reason). A missing folder has none.
pub fn scan(dir: &Path) -> Result<(Vec<Entry>, Vec<String>), IoError> {
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    if !check_folder(dir)? {
        return Ok((found, skipped));
    }
    for entry in std::fs::read_dir(dir)?.take(MAX_ENTRIES).flatten() {
        let name = entry.file_name();
        let Some(key) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
            continue;
        };
        if !is_valid_key(key) {
            continue;
        }
        let Ok((file, meta)) = paths(dir, key) else {
            continue;
        };
        let plain = |p: &Path| std::fs::symlink_metadata(p).ok().filter(|m| m.is_file());
        if plain(&meta).is_none() {
            skipped.push(format!("{key}: not a plain file"));
            continue;
        }
        let text = match crate::read_limited(&meta, MAX_META_BYTES) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                skipped.push(format!("{key}: {e}"));
                continue;
            }
        };
        let (time_ms, pid, design_path) = match parse_meta(&text, key) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(format!("{key}: {e}"));
                continue;
            }
        };
        let Some(m) = plain(&file) else {
            skipped.push(format!("{key}: its design file is missing"));
            continue;
        };
        found.push(Entry {
            key: key.to_owned(),
            time_ms,
            pid,
            design_path,
            file,
            bytes: m.len(),
        });
    }
    found.sort_by(|a, b| b.time_ms.cmp(&a.time_ms).then(a.key.cmp(&b.key)));
    skipped.sort();
    Ok((found, skipped))
}

/// True when the work was saved after the recovery file was written: the design file
/// exists and was changed at or after the recovery time.
pub fn saved_since(entry: &Entry) -> bool {
    let Some(p) = &entry.design_path else {
        return false;
    };
    std::fs::metadata(p)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .is_some_and(|saved| saved >= entry.time_ms)
}

/// True when the app that wrote the entry still runs (so its work is not lost): another
/// process of that id whose name starts with `chamfera`. Only known on Linux; elsewhere
/// every other process's entry counts as left by a crash.
pub fn writer_running(entry: &Entry) -> bool {
    if entry.pid == crate::atomic::pid() || entry.pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let comm = Path::new("/proc").join(entry.pid.to_string()).join("comm");
        crate::read_limited(&comm, 256).is_ok_and(|b| {
            String::from_utf8_lossy(&b)
                .trim_start()
                .starts_with("chamfera")
        })
    }
    #[cfg(not(target_os = "linux"))]
    false
}

/// Reads a recovery design, checked like any design file.
pub fn read(entry: &Entry) -> Result<Document, IoError> {
    crate::open_design(&entry.file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("chamfera-recovery-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_recovery_file_is_found_after_a_crash_and_gone_after_removal() {
        let dir = temp_dir("found");
        let mut doc = Document::default();
        doc.set_parameter("w", 50.0, chamfera_doc::Unit::Length)
            .unwrap();
        let key = new_key();
        write(&dir, &key, &doc, None).unwrap();
        // The "crash": nothing removes the files. The next start finds them.
        let (found, skipped) = scan(&dir).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, key);
        assert_eq!(found[0].name(), "Untitled");
        assert!(!saved_since(&found[0]));
        assert_eq!(read(&found[0]).unwrap().parameters["w"].value, 50.0);
        remove(&dir, &key).unwrap();
        assert!(scan(&dir).unwrap().0.is_empty());
    }

    #[test]
    fn work_saved_after_the_recovery_file_is_not_offered() {
        let dir = temp_dir("saved");
        let design =
            std::env::temp_dir().join(format!("chamfera-rec-{}.chamfera", std::process::id()));
        let e = write(&dir, "k-1", &Document::default(), Some(&design)).unwrap();
        // The design file does not exist yet: not saved since.
        let _ = std::fs::remove_file(&design);
        assert!(!saved_since(&e));
        std::thread::sleep(std::time::Duration::from_millis(5));
        crate::save_design(&Document::default(), &design).unwrap();
        assert!(saved_since(&e));
    }

    #[test]
    fn broken_recovery_folders_never_panic() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), b"{{").unwrap();
        std::fs::write(
            dir.join("b.json"),
            r#"{"format":"chamfera-recovery","key":"c","time_ms":1}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("d.json"),
            r#"{"format":"chamfera-recovery","key":"d","time_ms":1}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("e.json")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("f.json", dir.join("f.json")).unwrap();
        let (found, skipped) = scan(&dir).unwrap();
        assert!(found.is_empty());
        assert!(skipped.len() >= 4, "{skipped:?}");
        assert!(write(&dir, "../escape", &Document::default(), None).is_err());
        assert!(remove(&dir, "").is_err());
        // A recovery "folder" that is a file.
        let file = dir.join("plain");
        std::fs::write(&file, b"x").unwrap();
        assert!(scan(&file).is_err());
    }
}

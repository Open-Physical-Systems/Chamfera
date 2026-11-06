//! Writing a file so that a crash never leaves it broken.
//!
//! The new contents go to a temporary file in the same folder, which is flushed to the
//! disk (`fsync`) and then renamed over the old file. A rename within one file system is
//! atomic: after a crash the file holds either all of the old contents or all of the new,
//! never a mix or zeros. The folder is flushed too on Unix, so the rename itself survives
//! a power cut.

use crate::IoError;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The suffix of temporary files, so readers of a folder can skip them.
pub const TEMP_SUFFIX: &str = ".chamfera-tmp";

/// Leftover temporary files older than this (seconds) are removed after a write.
const STALE_TEMP_SECS: u64 = 3600;

/// Writes `bytes` to `path` atomically (see the module docs). A symlink at `path` is
/// followed, so the file it points at is replaced and the link stays.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IoError> {
    let target = real_target(path)?;
    let temp = stage(&target, bytes)?;
    commit(&temp, &target)?;
    remove_stale_temps(&target);
    Ok(())
}

/// The file a write really replaces: the target of a symlink, else the path itself.
fn real_target(path: &Path) -> Result<PathBuf, IoError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Ok(path.canonicalize()?),
        Ok(m) if m.is_dir() => Err(IoError::Format(format!(
            "{} is a folder, not a file",
            path.display()
        ))),
        _ => Ok(path.to_path_buf()),
    }
}

/// This process's id (0 in the browser, which has none: `std::process::id` would panic).
pub fn pid() -> u32 {
    #[cfg(target_arch = "wasm32")]
    return 0;
    #[cfg(not(target_arch = "wasm32"))]
    std::process::id()
}

/// Milliseconds since 1970 UTC (0 in the browser, where `SystemTime::now` would panic,
/// or when the clock is before 1970).
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return 0;
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The temporary file `stage` writes for `target`: hidden, beside it, unique per process
/// and call.
fn temp_path(target: &Path) -> Result<PathBuf, IoError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .ok_or_else(|| IoError::Format(format!("{} does not name a file", target.display())))?
        .to_string_lossy();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = format!(".{name}.{}-{n}{TEMP_SUFFIX}", pid());
    Ok(target.with_file_name(temp))
}

/// The first half of an atomic write: the new contents in a temporary file beside
/// `target`, flushed to the disk. The old file is not touched. Returns the temporary
/// file, which [`commit`] renames over the target.
pub fn stage(target: &Path, bytes: &[u8]) -> Result<PathBuf, IoError> {
    let temp = temp_path(target)?;
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(e.into());
    }
    Ok(temp)
}

/// The second half: renames the staged file over the target and flushes the folder.
pub fn commit(temp: &Path, target: &Path) -> Result<(), IoError> {
    if let Err(e) = std::fs::rename(temp, target) {
        let _ = std::fs::remove_file(temp);
        return Err(e.into());
    }
    sync_folder(target);
    Ok(())
}

/// Flushes the folder holding `path`, so a rename in it is on the disk (Unix only; on
/// Windows a folder cannot be opened this way and the rename is journalled anyway).
fn sync_folder(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Removes temporary files of earlier writes to `target` that a crash left behind (only
/// old ones: a newer one may belong to a write still running in another process).
fn remove_stale_temps(target: &Path) {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return;
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let prefix = format!(".{}.", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Bounded: a folder with very many files is not walked to the end.
    for entry in entries.take(100_000).flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if !(file.starts_with(&prefix) && file.ends_with(TEMP_SUFFIX)) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age.as_secs() >= STALE_TEMP_SECS);
        if meta.is_file() && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chamfera-atomic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_crash_after_the_temporary_file_is_written_leaves_the_old_file_intact() {
        let dir = temp_dir("crash");
        let path = dir.join("part.chamfera");
        write_atomic(&path, b"old contents").unwrap();
        // The crash: the new contents are staged but never renamed into place.
        let temp = stage(&path, b"new contents that never arrived").unwrap();
        assert!(temp.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"old contents");
        // The next write replaces the file whole, and the leftover is not the file.
        write_atomic(&path, b"newer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"newer");
        assert!(
            temp.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(TEMP_SUFFIX)
        );
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary_file() {
        let dir = temp_dir("plain");
        let path = dir.join("a.json");
        write_atomic(&path, b"1").unwrap();
        write_atomic(&path, b"22").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"22");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.json".to_owned()]);
    }

    #[test]
    fn a_folder_cannot_be_written_over() {
        let dir = temp_dir("folder");
        assert!(write_atomic(&dir, b"x").is_err());
        assert!(dir.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_replaced_through_the_link() {
        let dir = temp_dir("link");
        let real = dir.join("real.chamfera");
        let link = dir.join("link.chamfera");
        std::fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

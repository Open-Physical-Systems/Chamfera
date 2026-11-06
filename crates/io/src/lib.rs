//! Layer 3: design files, import and export.
//!
//! Design documents are versioned JSON. STL import and export arrived in M1 (mesh bodies);
//! STEP export and import of B-rep bodies arrived with K9 (`step`). Meshes also go out as
//! 3MF, OBJ and glTF binary and come in from 3MF and OBJ; sketches go out to and come in
//! from 2D DXF.

pub mod dxf;
pub mod gltf;
pub mod mesh;
pub mod obj;
pub mod step;
pub mod stl;
pub mod threemf;
mod xml;
pub mod zip;

use chamfera_doc::{DocError, Document};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("could not read or write the file: {0}")]
    File(#[from] std::io::Error),
    /// The file's content is wrong; the message names the format and the problem.
    #[error("{0}")]
    Format(String),
    #[error(transparent)]
    Doc(#[from] DocError),
}

/// Largest design file we accept (bytes). Hostile or broken files beyond this are refused.
pub const MAX_DESIGN_BYTES: u64 = 256 * 1024 * 1024;

pub fn save_design(doc: &Document, path: &std::path::Path) -> Result<(), IoError> {
    let text = doc.to_json().map_err(|e| IoError::Format(e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

pub fn open_design(path: &std::path::Path) -> Result<Document, IoError> {
    let bytes = read_limited(path, MAX_DESIGN_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| IoError::Format("a design file is UTF-8 text".into()))?;
    Ok(Document::from_json(&text)?)
}

/// Reads a regular file of at most `limit` bytes. Devices, pipes and folders are refused,
/// and the read stops at the limit even if the file grows or lies about its size.
pub fn read_limited(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, IoError> {
    use std::io::Read;
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(IoError::Format(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if meta.len() > limit {
        return Err(IoError::Format(format!(
            "{} is {} bytes, the limit is {limit}",
            path.display(),
            meta.len()
        )));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(IoError::Format(format!(
            "{} is larger than {limit} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_and_folders_are_not_read() {
        for p in ["/dev/zero", "/dev/null", "/"] {
            let path = std::path::Path::new(p);
            if path.exists() {
                assert!(read_limited(path, 1024).is_err(), "{p}");
                assert!(open_design(path).is_err(), "{p}");
            }
        }
    }

    #[test]
    fn files_over_the_limit_are_refused() {
        let path = std::env::temp_dir().join(format!("chamfera-limit-{}", std::process::id()));
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        assert!(read_limited(&path, 99).is_err());
        assert_eq!(read_limited(&path, 100).unwrap().len(), 100);
        std::fs::remove_file(&path).unwrap();
    }
}

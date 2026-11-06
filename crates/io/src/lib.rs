//! Layer 3: design files, import and export.
//!
//! M0 saves and opens the design document as versioned JSON. STEP, STL and 3MF arrive with
//! the first solids (M3).

use chamfera_doc::{DocError, Document};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("could not read or write the file: {0}")]
    File(#[from] std::io::Error),
    #[error("not a valid Chamfera design: {0}")]
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
    let size = std::fs::metadata(path)?.len();
    if size > MAX_DESIGN_BYTES {
        return Err(IoError::Format(format!(
            "file is {size} bytes, the limit is {MAX_DESIGN_BYTES}"
        )));
    }
    let text = std::fs::read_to_string(path)?;
    Ok(Document::from_json(&text)?)
}

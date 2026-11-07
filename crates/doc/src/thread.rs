//! Cosmetic threads (format 38): a thread recorded on a cylindrical face, its size, pitch
//! and length, shown in the viewport and called out on drawings. The solid stays a plain
//! cylinder; a modelled helical thread comes later.

use crate::{DocError, Document, FaceRef, expr};
use serde::{Deserialize, Serialize};

/// Longest thread designation (`M6`, `1/4-20 UNC`).
pub const MAX_THREAD_SIZE: usize = 40;

/// A cosmetic thread on a cylindrical face of the solids built so far, found again by
/// name at every rebuild.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub face: FaceRef,
    /// The designation shown in callouts, such as `M6`; empty: `M` and the diameter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub size: String,
    /// The pitch, an expression such as `1 mm`.
    pub pitch: String,
    /// How far the thread runs from its start; `None`: the whole face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<String>,
    /// Start at the face's lower end instead of its upper one (see the engine's thread
    /// module for which end is which).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// A thread's evaluated values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadValues {
    /// mm.
    pub pitch: f64,
    /// mm; `None`: the whole face.
    pub length: Option<f64>,
}

/// Whether `s` may be a thread designation: at most [`MAX_THREAD_SIZE`] characters,
/// letters, digits, spaces and `.,/-x×` only.
pub fn is_valid_thread_size(s: &str) -> bool {
    s.chars().count() <= MAX_THREAD_SIZE
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || " .,/-×".contains(c))
}

impl Document {
    /// A thread's pitch and length, checked: both above 0, a valid designation.
    pub fn thread_values(&self, t: &Thread) -> Result<ThreadValues, DocError> {
        let bad = |reason: &str| DocError::BadFeature {
            reason: reason.into(),
        };
        if !is_valid_thread_size(&t.size) {
            return Err(bad(
                "a thread size has at most 40 letters, digits, spaces and . , / - characters",
            ));
        }
        self.feature(t.face.feature)?;
        if t.face.face.is_empty() || t.face.face.chars().count() > 512 {
            return Err(bad("the face name is empty or too long"));
        }
        let length = |src: &str, what: &str| -> Result<f64, DocError> {
            let v = self.evaluate_feature_value(src, expr::Dim::LENGTH)?;
            if !v.is_finite()
                || v <= chamfera_geom::LINEAR_TOLERANCE
                || v > chamfera_geom::MAX_COORDINATE
            {
                return Err(DocError::BadFeature {
                    reason: format!("the thread {what} must be above 0"),
                });
            }
            Ok(v)
        };
        Ok(ThreadValues {
            pitch: length(&t.pitch, "pitch")?,
            length: t
                .length
                .as_deref()
                .map(|l| length(l, "length"))
                .transpose()?,
        })
    }
}

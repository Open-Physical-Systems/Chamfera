//! Layer 2: the design document: parameters and sketches now, the feature timeline from M4.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// The design format version this build writes. Bump it with a migration for older files.
///
/// - 1: parameters.
/// - 2: sketches (each on a plane). Version 1 files open with no sketches.
pub const FORMAT_VERSION: u32 = 2;
/// The tag every design file starts with, so other JSON is not mistaken for a design.
pub const FORMAT_TAG: &str = "chamfera";
/// Longest parameter name.
pub const MAX_NAME_LEN: usize = 64;
/// Most parameters one design may hold.
pub const MAX_PARAMETERS: usize = 10_000;
/// Most sketches one design may hold.
pub const MAX_SKETCHES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DocError {
    #[error("bad parameter name {0:?}: use a letter or _ then letters, digits or _ (at most 64)")]
    BadName(String),
    #[error("bad value for {name}: {reason}")]
    BadValue { name: String, reason: String },
    #[error("no parameter named {0:?}")]
    NoSuchParameter(String),
    #[error("a design holds at most {MAX_PARAMETERS} parameters")]
    TooManyParameters,
    #[error("not a Chamfera design: {0}")]
    Format(String),
    #[error("the design uses format {found}, this build reads up to {FORMAT_VERSION}")]
    TooNew { found: u32 },
    #[error("no sketch {0}")]
    NoSuchSketch(u32),
    #[error("a design holds at most {MAX_SKETCHES} sketches")]
    TooManySketches,
    #[error("sketch {sketch}: {reason}")]
    BadSketch { sketch: u32, reason: String },
}

/// What a parameter measures. Values are stored in internal units: mm and radians.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Length,
    Angle,
    Unitless,
}

impl Unit {
    pub fn parse(s: &str) -> Option<Unit> {
        match s {
            "length" => Some(Unit::Length),
            "angle" => Some(Unit::Angle),
            "unitless" => Some(Unit::Unitless),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub value: f64,
    pub unit: Unit,
}

/// A sketch and the plane it is drawn on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchFeature {
    pub id: u32,
    pub name: String,
    /// Sketch coordinates (x, y) map to `plane.from_plane(p)`.
    pub plane: chamfera_geom::Frame,
    pub sketch: chamfera_sketch::Sketch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: String,
    pub version: u32,
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub sketches: Vec<SketchFeature>,
    /// The last feature id handed out; ids are never reused.
    #[serde(default)]
    pub last_feature_id: u32,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            format: FORMAT_TAG.to_owned(),
            version: FORMAT_VERSION,
            parameters: BTreeMap::new(),
            sketches: Vec::new(),
            last_feature_id: 0,
        }
    }
}

/// True for names like `width`, `hole_d2`, `_t`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name.len() <= MAX_NAME_LEN
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Document {
    /// Creates or replaces a parameter. Fails without changing the document.
    pub fn set_parameter(&mut self, name: &str, value: f64, unit: Unit) -> Result<(), DocError> {
        if !is_valid_name(name) {
            return Err(DocError::BadName(name.to_owned()));
        }
        let bad = |reason: &str| DocError::BadValue {
            name: name.to_owned(),
            reason: reason.to_owned(),
        };
        if !value.is_finite() {
            return Err(bad("must be a finite number"));
        }
        let limit = match unit {
            Unit::Length => chamfera_geom::MAX_COORDINATE,
            Unit::Angle => 1e3 * std::f64::consts::TAU,
            Unit::Unitless => 1e9,
        };
        if value.abs() > limit {
            return Err(bad(&format!("magnitude must be at most {limit}")));
        }
        if !self.parameters.contains_key(name) && self.parameters.len() >= MAX_PARAMETERS {
            return Err(DocError::TooManyParameters);
        }
        self.parameters
            .insert(name.to_owned(), Parameter { value, unit });
        Ok(())
    }

    pub fn remove_parameter(&mut self, name: &str) -> Result<Parameter, DocError> {
        self.parameters
            .remove(name)
            .ok_or_else(|| DocError::NoSuchParameter(name.to_owned()))
    }

    /// Adds an empty sketch on `plane` and returns its id.
    pub fn add_sketch(&mut self, plane: chamfera_geom::Frame) -> Result<u32, DocError> {
        if self.sketches.len() >= MAX_SKETCHES {
            return Err(DocError::TooManySketches);
        }
        let id = self
            .last_feature_id
            .checked_add(1)
            .ok_or(DocError::TooManySketches)?;
        self.last_feature_id = id;
        self.sketches.push(SketchFeature {
            id,
            name: format!("Sketch{}", self.sketches.len() + 1),
            plane,
            sketch: chamfera_sketch::Sketch::new(),
        });
        Ok(id)
    }

    pub fn sketch(&self, id: u32) -> Result<&SketchFeature, DocError> {
        self.sketches
            .iter()
            .find(|s| s.id == id)
            .ok_or(DocError::NoSuchSketch(id))
    }

    pub fn sketch_mut(&mut self, id: u32) -> Result<&mut SketchFeature, DocError> {
        self.sketches
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(DocError::NoSuchSketch(id))
    }

    pub fn remove_sketch(&mut self, id: u32) -> Result<SketchFeature, DocError> {
        let i = self
            .sketches
            .iter()
            .position(|s| s.id == id)
            .ok_or(DocError::NoSuchSketch(id))?;
        Ok(self.sketches.remove(i))
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Reads a design, refusing other JSON, newer formats and invalid content.
    pub fn from_json(text: &str) -> Result<Document, DocError> {
        let doc: Document =
            serde_json::from_str(text).map_err(|e| DocError::Format(e.to_string()))?;
        if doc.format != FORMAT_TAG {
            return Err(DocError::Format(format!("format tag is {:?}", doc.format)));
        }
        if doc.version > FORMAT_VERSION {
            return Err(DocError::TooNew { found: doc.version });
        }
        // Re-validate everything: a file is hostile input.
        let mut checked = Document::default();
        for (name, p) in &doc.parameters {
            checked.set_parameter(name, p.value, p.unit)?;
        }
        if doc.sketches.len() > MAX_SKETCHES {
            return Err(DocError::TooManySketches);
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &doc.sketches {
            let bad = |reason: String| DocError::BadSketch {
                sketch: s.id,
                reason,
            };
            if !seen.insert(s.id) || s.id == 0 || s.id > doc.last_feature_id {
                return Err(bad("duplicate or out-of-range id".into()));
            }
            if s.name.chars().count() > 256 {
                return Err(bad("name is longer than 256 characters".into()));
            }
            s.sketch.validate().map_err(|e| bad(e.to_string()))?;
        }
        checked.sketches = doc.sketches;
        checked.last_feature_id = doc.last_feature_id;
        Ok(checked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_identifier_rules() {
        for ok in ["w", "width", "_t", "hole_d2", &"a".repeat(64)] {
            assert!(is_valid_name(ok), "{ok}");
        }
        for bad in ["", "2w", "a-b", "a b", "ä", &"a".repeat(65)] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn non_finite_and_huge_values_are_refused_and_change_nothing() {
        let mut doc = Document::default();
        doc.set_parameter("w", 10.0, Unit::Length).unwrap();
        for v in [f64::NAN, f64::INFINITY, 1e12] {
            assert!(doc.set_parameter("w", v, Unit::Length).is_err());
        }
        assert_eq!(doc.parameters["w"].value, 10.0);
    }

    #[test]
    fn a_design_survives_a_round_trip() {
        let mut doc = Document::default();
        doc.set_parameter("width", 50.0, Unit::Length).unwrap();
        doc.set_parameter("draft", 0.05, Unit::Angle).unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn sketches_survive_a_round_trip_and_version_1_files_still_open() {
        let mut doc = Document::default();
        let id = doc.add_sketch(chamfera_geom::Frame::XZ).unwrap();
        let s = &mut doc.sketch_mut(id).unwrap().sketch;
        let a = s.add_point(chamfera_geom::Point2::new(0.1, 0.2)).unwrap();
        let b = s
            .add_point(chamfera_geom::Point2::new(10.0, 1.0 / 3.0))
            .unwrap();
        s.add_line(a, b).unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc, "float_roundtrip keeps every coordinate exact");

        let v1 = r#"{"format":"chamfera","version":1,"parameters":{}}"#;
        let old = Document::from_json(v1).unwrap();
        assert!(old.sketches.is_empty());
    }

    #[test]
    fn bad_sketch_ids_are_refused() {
        let mut doc = Document::default();
        doc.add_sketch(chamfera_geom::Frame::XY).unwrap();
        let mut text: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        text["last_feature_id"] = serde_json::json!(0);
        assert!(Document::from_json(&text.to_string()).is_err());
    }

    #[test]
    fn other_json_newer_formats_and_bad_content_are_refused() {
        assert!(Document::from_json("{}").is_err());
        assert!(Document::from_json("not json").is_err());
        let newer = r#"{"format":"chamfera","version":999,"parameters":{}}"#;
        assert!(matches!(
            Document::from_json(newer),
            Err(DocError::TooNew { .. })
        ));
        let bad =
            r#"{"format":"chamfera","version":1,"parameters":{"2x":{"value":1,"unit":"length"}}}"#;
        assert!(Document::from_json(bad).is_err());
    }
}

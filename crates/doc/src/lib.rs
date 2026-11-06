//! Layer 2: the design document: parameters and sketches now, the feature timeline from M4.

pub mod expr;
pub mod feature;
pub mod imported;

pub use feature::{
    Blend, BlendKind, Extrude, ExtrudeDirection, Feature, FeatureKind, Operation, Pattern,
    PatternLayout, Revolve,
};
pub use imported::ImportedBody;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// The design format version this build writes. Bump it with a migration for older files.
///
/// - 1: parameters.
/// - 2: sketches (each on a plane). Version 1 files open with no sketches.
/// - 3: parameter expressions, and sketch dimensions driven by expressions.
/// - 4: the feature timeline (extrude, revolve).
/// - 5: sketches attached to a face of a feature's solid.
/// - 6: feature operations (new body, join, cut, intersect).
/// - 7: linear and circular patterns of features.
/// - 8: imported solid bodies.
/// - 9: fillets and chamfers of edges.
pub const FORMAT_VERSION: u32 = 9;
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
    #[error("{name}: {error}")]
    Expression {
        name: String,
        error: expr::ExprError,
    },
    #[error("parameters refer to each other in a loop: {}", .0.join(" → "))]
    Cycle(Vec<String>),
    #[error("{name} is used by {}; change those first", .users.join(", "))]
    InUse { name: String, users: Vec<String> },
    #[error("sketch {sketch} cannot take the new dimensions: {reason}")]
    Solve { sketch: u32, reason: String },
    #[error("no feature {0}")]
    NoSuchFeature(u32),
    #[error("{reason}")]
    BadFeature { reason: String },
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

impl Unit {
    /// The expression dimension this unit expects.
    pub fn dim(self) -> expr::Dim {
        match self {
            Unit::Length => expr::Dim::LENGTH,
            Unit::Angle => expr::Dim::ANGLE,
            Unit::Unitless => expr::Dim::NONE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// The current value in internal units (mm, radians), kept up to date with
    /// `expression` by [`Document::evaluate`].
    pub value: f64,
    pub unit: Unit,
    /// What the user typed, such as `width / 2 + 3 mm`. `None` for a plain value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// A face of a feature's solid, by the face's role name (`top`, `side 2`, ...). Names
/// come from the kernel's sweeps and stay the same when the feature is rebuilt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceRef {
    pub feature: u32,
    pub face: String,
}

/// A sketch and the plane it is drawn on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchFeature {
    pub id: u32,
    pub name: String,
    /// Sketch coordinates (x, y) map to `plane.from_plane(p)`.
    pub plane: chamfera_geom::Frame,
    pub sketch: chamfera_sketch::Sketch,
    /// Dimensions driven by expressions: constraint id → expression. Re-evaluated when a
    /// parameter changes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expressions: BTreeMap<u32, String>,
    /// The face this sketch lies on, if any: its plane follows that face when the design
    /// rebuilds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached: Option<FaceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: String,
    pub version: u32,
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub sketches: Vec<SketchFeature>,
    /// The timeline, in build order.
    #[serde(default)]
    pub features: Vec<Feature>,
    /// The last feature id handed out; ids are never reused.
    #[serde(default)]
    pub last_feature_id: u32,
    /// Solid bodies imported from other CAD files (STEP), kept as exact B-rep bodies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imported: Vec<ImportedBody>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            format: FORMAT_TAG.to_owned(),
            version: FORMAT_VERSION,
            parameters: BTreeMap::new(),
            sketches: Vec::new(),
            features: Vec::new(),
            last_feature_id: 0,
            imported: Vec::new(),
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
        let before = self.clone();
        self.parameters.insert(
            name.to_owned(),
            Parameter {
                value,
                unit,
                expression: None,
            },
        );
        if let Err(e) = self.evaluate() {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Creates or replaces a parameter defined by an expression, then re-evaluates the
    /// design. Fails without changing the document.
    pub fn set_parameter_expression(
        &mut self,
        name: &str,
        expression: &str,
        unit: Unit,
    ) -> Result<(), DocError> {
        if !is_valid_name(name) {
            return Err(DocError::BadName(name.to_owned()));
        }
        if !self.parameters.contains_key(name) && self.parameters.len() >= MAX_PARAMETERS {
            return Err(DocError::TooManyParameters);
        }
        let before = self.clone();
        self.parameters.insert(
            name.to_owned(),
            Parameter {
                value: 0.0,
                unit,
                expression: Some(expression.to_owned()),
            },
        );
        if let Err(e) = self.evaluate() {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Who refers to a parameter: other parameters and sketch dimensions.
    pub fn parameter_users(&self, name: &str) -> Vec<String> {
        let mut users: Vec<String> = self
            .parameters
            .iter()
            .filter(|(n, p)| {
                n.as_str() != name
                    && p.expression
                        .as_deref()
                        .is_some_and(|e| expr::names_in(e).iter().any(|x| x == name))
            })
            .map(|(n, _)| n.clone())
            .collect();
        for s in &self.sketches {
            for (c, e) in &s.expressions {
                if expr::names_in(e).iter().any(|x| x == name) {
                    users.push(format!("{} dimension {c}", s.name));
                }
            }
        }
        users.extend(self.feature_parameter_users(name));
        users
    }

    /// Recomputes every parameter from its expression (in dependency order), then drives
    /// sketch dimensions from theirs and solves those sketches. On error the document may
    /// be partly updated: callers keep a copy and restore it (the session does).
    pub fn evaluate(&mut self) -> Result<(), DocError> {
        use expr::Quantity;
        let mut values: BTreeMap<String, Quantity> = self
            .parameters
            .iter()
            .filter(|(_, p)| p.expression.is_none())
            .map(|(n, p)| {
                (
                    n.clone(),
                    Quantity {
                        value: p.value,
                        dim: p.unit.dim(),
                    },
                )
            })
            .collect();
        // Depth-first evaluation with an explicit path, so a loop is reported by name.
        fn visit(
            name: &str,
            doc: &Document,
            values: &mut BTreeMap<String, expr::Quantity>,
            path: &mut Vec<String>,
        ) -> Result<(), DocError> {
            if values.contains_key(name) {
                return Ok(());
            }
            if let Some(i) = path.iter().position(|n| n == name) {
                let mut cycle: Vec<String> = path.get(i..).unwrap_or_default().to_vec();
                cycle.push(name.to_owned());
                return Err(DocError::Cycle(cycle));
            }
            let Some(p) = doc.parameters.get(name) else {
                return Ok(()); // An unknown name is reported when the expression runs.
            };
            let Some(e) = p.expression.as_deref() else {
                return Ok(());
            };
            path.push(name.to_owned());
            for dep in expr::names_in(e) {
                visit(&dep, doc, values, path)?;
            }
            path.pop();
            let v = expr::evaluate_as(e, p.unit.dim(), values).map_err(|error| {
                DocError::Expression {
                    name: name.to_owned(),
                    error,
                }
            })?;
            let limit = match p.unit {
                Unit::Length => chamfera_geom::MAX_COORDINATE,
                Unit::Angle => 1e3 * std::f64::consts::TAU,
                Unit::Unitless => 1e9,
            };
            if v.abs() > limit {
                return Err(DocError::BadValue {
                    name: name.to_owned(),
                    reason: format!("magnitude must be at most {limit}"),
                });
            }
            values.insert(
                name.to_owned(),
                expr::Quantity {
                    value: v,
                    dim: p.unit.dim(),
                },
            );
            Ok(())
        }
        let names: Vec<String> = self.parameters.keys().cloned().collect();
        for n in &names {
            visit(n, self, &mut values, &mut Vec::new())?;
        }
        for (n, p) in self.parameters.iter_mut() {
            if let Some(q) = values.get(n) {
                p.value = q.value;
            }
        }
        // Dimensions that follow expressions.
        for s in &mut self.sketches {
            if s.expressions.is_empty() {
                continue;
            }
            let mut changed = false;
            for (c, e) in &s.expressions {
                let id = chamfera_sketch::ConstraintId(*c);
                let Some(constraint) = s.sketch.constraint(id) else {
                    continue;
                };
                let dim = if constraint.kind_name() == "angle" {
                    expr::Dim::ANGLE
                } else {
                    expr::Dim::LENGTH
                };
                let v =
                    expr::evaluate_as(e, dim, &values).map_err(|error| DocError::Expression {
                        name: format!("{} dimension {c}", s.name),
                        error,
                    })?;
                if constraint.value() != Some(v) {
                    s.sketch
                        .set_dimension(id, v)
                        .map_err(|err| DocError::Solve {
                            sketch: s.id,
                            reason: err.to_string(),
                        })?;
                    changed = true;
                }
            }
            if changed {
                s.sketch.solve().map_err(|err| DocError::Solve {
                    sketch: s.id,
                    reason: err.to_string(),
                })?;
            }
        }
        Ok(())
    }

    pub fn remove_parameter(&mut self, name: &str) -> Result<Parameter, DocError> {
        let users = self.parameter_users(name);
        if !users.is_empty() {
            return Err(DocError::InUse {
                name: name.to_owned(),
                users,
            });
        }
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
            expressions: BTreeMap::new(),
            attached: None,
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
            if !is_valid_name(name) {
                return Err(DocError::BadName(name.clone()));
            }
            if checked.parameters.len() >= MAX_PARAMETERS {
                return Err(DocError::TooManyParameters);
            }
            if !p.value.is_finite() {
                return Err(DocError::BadValue {
                    name: name.clone(),
                    reason: "must be a finite number".into(),
                });
            }
            if p.expression
                .as_ref()
                .is_some_and(|e| e.chars().count() > expr::MAX_EXPRESSION_LEN)
            {
                return Err(DocError::BadValue {
                    name: name.clone(),
                    reason: "the expression is too long".into(),
                });
            }
            checked.parameters.insert(name.clone(), p.clone());
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
            for (c, e) in &s.expressions {
                if s.sketch
                    .constraint(chamfera_sketch::ConstraintId(*c))
                    .is_none()
                    || e.chars().count() > expr::MAX_EXPRESSION_LEN
                {
                    return Err(bad(format!("expression for missing constraint {c}")));
                }
            }
        }
        checked.sketches = doc.sketches;
        checked.last_feature_id = doc.last_feature_id;
        if doc.features.len() > feature::MAX_FEATURES {
            return Err(DocError::BadFeature {
                reason: "too many features".into(),
            });
        }
        for f in &doc.features {
            if !seen.insert(f.id) || f.id == 0 || f.id > checked.last_feature_id {
                return Err(DocError::BadFeature {
                    reason: format!("feature {} has a duplicate or out-of-range id", f.id),
                });
            }
            if f.name.chars().count() > 256 {
                return Err(DocError::BadFeature {
                    reason: format!("feature {} has a name longer than 256", f.id),
                });
            }
        }
        checked.features = doc.features;
        imported::check(&doc.imported, &mut seen, checked.last_feature_id)?;
        checked.imported = doc.imported;
        // Values in the file must agree with their expressions.
        checked.evaluate()?;
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
    fn parameters_follow_their_expressions_in_dependency_order() {
        let mut doc = Document::default();
        doc.set_parameter_expression("width", "80 mm", Unit::Length)
            .unwrap();
        doc.set_parameter_expression("hole", "width / 10", Unit::Length)
            .unwrap();
        doc.set_parameter_expression("margin", "hole * 1.5", Unit::Length)
            .unwrap();
        assert_eq!(doc.parameters["margin"].value, 12.0);
        doc.set_parameter_expression("width", "100", Unit::Length)
            .unwrap();
        assert_eq!(doc.parameters["hole"].value, 10.0);
        assert_eq!(doc.parameters["margin"].value, 15.0);
        let a = doc.set_parameter_expression("tilt", "15 deg * 2", Unit::Angle);
        assert!(a.is_ok());
        assert!((doc.parameters["tilt"].value - std::f64::consts::FRAC_PI_6).abs() < 1e-15);
    }

    #[test]
    fn loops_units_and_unknown_names_are_refused_and_change_nothing() {
        let mut doc = Document::default();
        doc.set_parameter_expression("a", "10", Unit::Length)
            .unwrap();
        doc.set_parameter_expression("b", "a * 2", Unit::Length)
            .unwrap();
        let before = doc.clone();
        assert!(matches!(
            doc.set_parameter_expression("a", "b + 1", Unit::Length),
            Err(DocError::Cycle(_))
        ));
        assert!(
            doc.set_parameter_expression("c", "a + 5 deg", Unit::Length)
                .is_err()
        );
        assert!(
            doc.set_parameter_expression("c", "a * a", Unit::Length)
                .is_err()
        );
        assert!(
            doc.set_parameter_expression("c", "nope + 1", Unit::Length)
                .is_err()
        );
        assert_eq!(doc, before);
        assert!(matches!(
            doc.remove_parameter("a"),
            Err(DocError::InUse { .. })
        ));
        doc.remove_parameter("b").unwrap();
        doc.remove_parameter("a").unwrap();
    }

    #[test]
    fn a_dimension_follows_its_parameter() {
        use chamfera_geom::Point2;
        use chamfera_sketch::Constraint;
        let mut doc = Document::default();
        doc.set_parameter_expression("len", "40 mm", Unit::Length)
            .unwrap();
        let id = doc.add_sketch(chamfera_geom::Frame::XY).unwrap();
        let f = doc.sketch_mut(id).unwrap();
        let a = f.sketch.add_point(Point2::new(0.0, 0.0)).unwrap();
        let b = f.sketch.add_point(Point2::new(30.0, 0.0)).unwrap();
        let l = f.sketch.add_line(a, b).unwrap();
        f.sketch.fix_point(a).unwrap();
        f.sketch
            .add_constraint(Constraint::Horizontal { line: l })
            .unwrap();
        let c = f
            .sketch
            .add_constraint(Constraint::Length {
                line: l,
                value: 30.0,
            })
            .unwrap();
        f.expressions.insert(c.0, "len / 2".into());
        doc.evaluate().unwrap();
        let end = doc.sketch(id).unwrap().sketch.point(b).unwrap();
        assert!((end.x - 20.0).abs() < 1e-9);
        doc.set_parameter_expression("len", "70", Unit::Length)
            .unwrap();
        let end = doc.sketch(id).unwrap().sketch.point(b).unwrap();
        assert!((end.x - 35.0).abs() < 1e-9);
        // A value that the sketch cannot take (a zero length) is refused, nothing changes.
        let before = doc.clone();
        assert!(
            doc.set_parameter_expression("len", "0", Unit::Length)
                .is_err()
        );
        assert_eq!(doc, before);
        // Round trip keeps the expressions and their values.
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
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

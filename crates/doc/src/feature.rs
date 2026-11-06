//! The feature timeline: what turns sketches into solids, in order.
//!
//! A feature refers to sketch profiles by a point inside each one (a seed), not by an
//! index: indices change when a sketch changes, a point inside the plate stays inside the
//! plate. Distances are expressions, so a parameter can drive them.

use crate::{DocError, Document, expr};
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// Most features one design may hold.
pub const MAX_FEATURES: usize = 10_000;
/// Most profiles one feature may use.
pub const MAX_FEATURE_PROFILES: usize = 1_000;

/// Which way an extrude goes from its start plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeDirection {
    /// Along the plane normal.
    #[default]
    Normal,
    /// Against the plane normal.
    Reversed,
    /// Half each way.
    Symmetric,
    /// `distance` along the normal and `distance2` against it.
    TwoSides,
}

impl ExtrudeDirection {
    pub fn parse(s: &str) -> Option<ExtrudeDirection> {
        Some(match s {
            "normal" => ExtrudeDirection::Normal,
            "reversed" => ExtrudeDirection::Reversed,
            "symmetric" => ExtrudeDirection::Symmetric,
            "two_sides" => ExtrudeDirection::TwoSides,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            ExtrudeDirection::Normal => "normal",
            ExtrudeDirection::Reversed => "reversed",
            ExtrudeDirection::Symmetric => "symmetric",
            ExtrudeDirection::TwoSides => "two_sides",
        }
    }
}

/// How far an extrude goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeExtent {
    /// By `distance` (and `distance2` for two sides).
    #[default]
    Distance,
    /// Past every body built before it (from their bounding boxes), so a cut goes
    /// through everything.
    ThroughAll,
    /// Up to the flat face or plane in `to`, which must be parallel to the profile's
    /// plane. The direction follows the object.
    ToObject,
}

impl ExtrudeExtent {
    pub fn parse(s: &str) -> Option<ExtrudeExtent> {
        Some(match s {
            "distance" => ExtrudeExtent::Distance,
            "through_all" => ExtrudeExtent::ThroughAll,
            "to_object" => ExtrudeExtent::ToObject,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            ExtrudeExtent::Distance => "distance",
            ExtrudeExtent::ThroughAll => "through_all",
            ExtrudeExtent::ToObject => "to_object",
        }
    }
}

/// What a to-object extrude goes up to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeTo {
    /// A flat face of an earlier feature's solid, by name, found again at every rebuild.
    Face(crate::FaceRef),
    /// A fixed plane (such as a construction plane's frame).
    Plane(chamfera_geom::Frame),
}

/// What a feature's new shape does to the bodies it overlaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// A separate body.
    #[default]
    NewBody,
    /// Merged with the bodies it overlaps.
    Join,
    /// Removed from the bodies it overlaps.
    Cut,
    /// Only the overlap is kept.
    Intersect,
}

impl Operation {
    pub fn parse(s: &str) -> Option<Operation> {
        Some(match s {
            "new_body" => Operation::NewBody,
            "join" => Operation::Join,
            "cut" => Operation::Cut,
            "intersect" => Operation::Intersect,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            Operation::NewBody => "new_body",
            Operation::Join => "join",
            Operation::Cut => "cut",
            Operation::Intersect => "intersect",
        }
    }
}

/// Pull sketch profiles straight out. Every option after `operation` came in format 13
/// and defaults to what older designs did: by distance, from the sketch plane, no taper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    pub sketch: u32,
    /// A point inside each profile to extrude (sketch coordinates).
    pub profiles: Vec<Point2>,
    /// The distance, an expression such as `10 mm` or `thickness * 2` (side one for two
    /// sides; unused for through all and to object).
    pub distance: String,
    #[serde(default)]
    pub direction: ExtrudeDirection,
    #[serde(default)]
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "is_default")]
    pub extent: ExtrudeExtent,
    /// The second side's distance, against the normal (two sides only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance2: Option<String>,
    /// Where the extrude starts: this far from the sketch plane along its normal
    /// (negative: the other way). `None` starts on the sketch plane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<String>,
    /// The draft angle of the side faces, an angle expression: positive narrows the
    /// profile away from the start plane, negative widens it. `None` is no taper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper: Option<String>,
    /// The face or plane a to-object extrude goes up to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<ExtrudeTo>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl Default for Extrude {
    /// No sketch or profiles yet, 10 mm along the normal as a new body.
    fn default() -> Self {
        Extrude {
            sketch: 0,
            profiles: Vec::new(),
            distance: "10 mm".into(),
            direction: ExtrudeDirection::Normal,
            operation: Operation::NewBody,
            extent: ExtrudeExtent::Distance,
            distance2: None,
            start_offset: None,
            taper: None,
            to: None,
        }
    }
}

/// The steepest extrude taper, in degrees (side faces at 90 degrees would lie flat).
pub const MAX_TAPER_DEGREES: f64 = 89.0;

/// An extrude's evaluated options (see [`Document::extrude_values`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ExtrudeValues {
    /// mm; 0 when the extent does not use it.
    pub distance: f64,
    /// mm, two sides only.
    pub distance2: f64,
    /// mm along the sketch plane's normal.
    pub start_offset: f64,
    /// Radians; positive narrows.
    pub taper: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revolve {
    pub sketch: u32,
    pub profiles: Vec<Point2>,
    /// The axis: a sketch line (its curve id) the profile turns around.
    pub axis: u32,
    /// The angle, an expression such as `360 deg` or `90`.
    pub angle: String,
    #[serde(default)]
    pub operation: Operation,
}

/// Where a pattern puts its copies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "layout", rename_all = "snake_case")]
pub enum PatternLayout {
    /// `count` copies along `direction`, `spacing` apart (the first is the original).
    Linear {
        direction: chamfera_geom::Vec3,
        spacing: String,
    },
    /// `count` copies around the axis through `origin` along `axis`, spread evenly over
    /// `angle` (360 deg: a full circle, the last copy not on top of the first).
    Circular {
        origin: chamfera_geom::Point3,
        axis: chamfera_geom::Vec3,
        angle: String,
    },
}

/// Copies of an earlier feature's shape, each doing that feature's operation (a pattern
/// of a cut drills more holes; of a join, adds more bosses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub source: u32,
    /// How many, including the original: an expression such as `4` or `holes`.
    pub count: String,
    #[serde(flatten)]
    pub layout: PatternLayout,
}

/// Most copies one pattern may make.
pub const MAX_PATTERN_COUNT: f64 = 1000.0;

/// Rounded (fillet) or bevelled (chamfer) edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendKind {
    Fillet,
    Chamfer,
}

/// A fillet or chamfer of edges of the solids built so far. Edges are named like faces
/// (`2:top 0` is edge `top 0` of feature 2's shape), so the blend finds them again after
/// every rebuild.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blend {
    pub blend: BlendKind,
    pub edges: Vec<String>,
    /// The radius (fillet) or distance (chamfer), an expression such as `2 mm`.
    pub size: String,
}

/// Most edges one blend may hold.
pub const MAX_BLEND_EDGES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureKind {
    Extrude(Extrude),
    Revolve(Revolve),
    Pattern(Pattern),
    Blend(Blend),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: u32,
    pub name: String,
    /// A suppressed feature is kept but not built.
    #[serde(default)]
    pub suppressed: bool,
    #[serde(flatten)]
    pub kind: FeatureKind,
}

impl FeatureKind {
    /// The sketch the feature's profiles come from (patterns have none).
    pub fn sketch(&self) -> Option<u32> {
        match self {
            FeatureKind::Extrude(e) => Some(e.sketch),
            FeatureKind::Revolve(r) => Some(r.sketch),
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) => None,
        }
    }

    /// The operation; a pattern repeats its source's (see the rebuild).
    pub fn operation(&self) -> Operation {
        match self {
            FeatureKind::Extrude(e) => e.operation,
            FeatureKind::Revolve(r) => r.operation,
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) => Operation::NewBody,
        }
    }

    pub fn profiles(&self) -> &[Point2] {
        match self {
            FeatureKind::Extrude(e) => &e.profiles,
            FeatureKind::Revolve(r) => &r.profiles,
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) => &[],
        }
    }

    fn label(&self) -> &'static str {
        match self {
            FeatureKind::Extrude(_) => "Extrude",
            FeatureKind::Revolve(_) => "Revolve",
            FeatureKind::Pattern(_) => "Pattern",
            FeatureKind::Blend(b) => match b.blend {
                BlendKind::Fillet => "Fillet",
                BlendKind::Chamfer => "Chamfer",
            },
        }
    }

    /// The expressions this feature evaluates.
    fn expressions(&self) -> Vec<&str> {
        match self {
            FeatureKind::Extrude(e) => std::iter::once(e.distance.as_str())
                .chain(e.distance2.as_deref())
                .chain(e.start_offset.as_deref())
                .chain(e.taper.as_deref())
                .collect(),
            FeatureKind::Revolve(r) => vec![r.angle.as_str()],
            FeatureKind::Blend(b) => vec![b.size.as_str()],
            FeatureKind::Pattern(p) => match &p.layout {
                PatternLayout::Linear { spacing, .. } => vec![p.count.as_str(), spacing.as_str()],
                PatternLayout::Circular { angle, .. } => vec![p.count.as_str(), angle.as_str()],
            },
        }
    }
}

impl Document {
    /// Checks a feature against the design: its sketch exists, every seed lies in a
    /// profile of that sketch, its expressions evaluate with the right unit.
    pub fn check_feature(&self, kind: &FeatureKind) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        if let FeatureKind::Pattern(p) = kind {
            return self.check_pattern(p);
        }
        if let FeatureKind::Blend(b) = kind {
            if b.edges.is_empty() || b.edges.len() > MAX_BLEND_EDGES {
                return Err(bad(format!("pick 1 to {MAX_BLEND_EDGES} edges")));
            }
            if b.edges
                .iter()
                .any(|e| e.is_empty() || e.chars().count() > 512)
            {
                return Err(bad("an edge name is empty or too long".into()));
            }
            let s = self.evaluate_feature_value(&b.size, expr::Dim::LENGTH)?;
            if !s.is_finite() || s <= chamfera_geom::LINEAR_TOLERANCE {
                return Err(bad("the size must be above 0".into()));
            }
            return Ok(());
        }
        let Some(sketch) = kind.sketch() else {
            return Err(bad("the feature has no sketch".into()));
        };
        let f = self.sketch(sketch)?;
        let seeds = kind.profiles();
        if seeds.is_empty() {
            return Err(bad("select at least one profile".into()));
        }
        if seeds.len() > MAX_FEATURE_PROFILES {
            return Err(bad(format!("at most {MAX_FEATURE_PROFILES} profiles")));
        }
        for p in seeds {
            if !p.is_valid() {
                return Err(bad("a profile point is not a finite coordinate".into()));
            }
            if f.sketch.region_at(*p).is_none() {
                return Err(bad(format!(
                    "no closed profile of {} contains ({:.3}, {:.3})",
                    f.name, p.x, p.y
                )));
            }
        }
        match kind {
            FeatureKind::Extrude(e) => self.check_extrude(e)?,
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) => {}
            FeatureKind::Revolve(r) => {
                let a = self.evaluate_feature_value(&r.angle, expr::Dim::ANGLE)?;
                if !a.is_finite()
                    || a <= 0.0
                    || a > std::f64::consts::TAU + chamfera_geom::ANGULAR_TOLERANCE
                {
                    return Err(bad(
                        "the angle must be above 0 and at most 360 degrees".into()
                    ));
                }
                match f.sketch.curve(chamfera_sketch::CurveId(r.axis)) {
                    Some(c) if c.is_line() => {}
                    _ => {
                        return Err(bad(format!(
                            "the axis {} is not a line of the sketch",
                            r.axis
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// The values of an extrude's options, checked: (distance, second distance, start
    /// offset, taper in radians). Distances are only checked (and only non-zero) where
    /// the extent uses them; missing options are 0.
    pub fn extrude_values(&self, e: &Extrude) -> Result<ExtrudeValues, DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        let length = |src: &str| self.evaluate_feature_value(src, expr::Dim::LENGTH);
        let tol = chamfera_geom::LINEAR_TOLERANCE;
        let by_distance = e.extent == ExtrudeExtent::Distance;
        let mut v = ExtrudeValues::default();
        if by_distance {
            v.distance = length(&e.distance)?;
            if !v.distance.is_finite()
                || v.distance.abs() <= tol
                || v.distance.abs() > chamfera_geom::MAX_COORDINATE
            {
                return Err(bad(format!(
                    "the distance must not be 0 and at most {} mm",
                    chamfera_geom::MAX_COORDINATE
                )));
            }
        }
        if by_distance && e.direction == ExtrudeDirection::TwoSides {
            let Some(d2) = &e.distance2 else {
                return Err(bad("two sides needs a second distance".into()));
            };
            v.distance2 = length(d2)?;
            let ok = |d: f64| d.is_finite() && d > tol && d <= chamfera_geom::MAX_COORDINATE;
            if !ok(v.distance) || !ok(v.distance2) {
                return Err(bad(format!(
                    "both distances of two sides must be above 0 and at most {} mm",
                    chamfera_geom::MAX_COORDINATE
                )));
            }
        }
        if let Some(s) = &e.start_offset {
            v.start_offset = length(s)?;
            if !v.start_offset.is_finite() || v.start_offset.abs() > chamfera_geom::MAX_COORDINATE {
                return Err(bad(format!(
                    "the start offset must be at most {} mm",
                    chamfera_geom::MAX_COORDINATE
                )));
            }
        }
        if let Some(t) = &e.taper {
            v.taper = self.evaluate_feature_value(t, expr::Dim::ANGLE)?;
            if !v.taper.is_finite() || v.taper.abs().to_degrees() >= MAX_TAPER_DEGREES {
                return Err(bad(format!(
                    "the taper must be less than {MAX_TAPER_DEGREES} degrees either way"
                )));
            }
        }
        if e.extent == ExtrudeExtent::ToObject {
            match &e.to {
                None => return Err(bad("pick the face or plane to extrude up to".into())),
                Some(ExtrudeTo::Face(r)) => {
                    self.feature(r.feature)?;
                    if r.face.is_empty() || r.face.chars().count() > 512 {
                        return Err(bad("the face name is empty or too long".into()));
                    }
                }
                Some(ExtrudeTo::Plane(f)) => {
                    if !f.origin().is_valid() {
                        return Err(bad("the plane lies outside the working range".into()));
                    }
                }
            }
        }
        Ok(v)
    }

    fn check_extrude(&self, e: &Extrude) -> Result<(), DocError> {
        self.extrude_values(e).map(|_| ())
    }

    /// The number of copies a pattern makes, checked (2 to [`MAX_PATTERN_COUNT`]).
    pub fn pattern_count(&self, p: &Pattern) -> Result<usize, DocError> {
        let n = self.evaluate_feature_value(&p.count, expr::Dim::NONE)?;
        if !(n.is_finite() && n.fract() == 0.0 && (2.0..=MAX_PATTERN_COUNT).contains(&n)) {
            return Err(DocError::BadFeature {
                reason: format!("the count must be a whole number from 2 to {MAX_PATTERN_COUNT}"),
            });
        }
        Ok(n as usize)
    }

    fn check_pattern(&self, p: &Pattern) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        let source = self.feature(p.source)?;
        if matches!(source.kind, FeatureKind::Pattern(_)) {
            return Err(bad("pattern a feature, not another pattern".into()));
        }
        self.pattern_count(p)?;
        match &p.layout {
            PatternLayout::Linear { direction, spacing } => {
                if direction.normalized().is_none() {
                    return Err(bad("the direction has no length".into()));
                }
                let s = self.evaluate_feature_value(spacing, expr::Dim::LENGTH)?;
                if !s.is_finite() || s.abs() <= chamfera_geom::LINEAR_TOLERANCE {
                    return Err(bad("the spacing must not be 0".into()));
                }
            }
            PatternLayout::Circular {
                origin,
                axis,
                angle,
            } => {
                if !origin.is_valid() || axis.normalized().is_none() {
                    return Err(bad("the axis needs a point and a direction".into()));
                }
                let a = self.evaluate_feature_value(angle, expr::Dim::ANGLE)?;
                if !a.is_finite()
                    || a.abs() <= chamfera_geom::ANGULAR_TOLERANCE
                    || a.abs() > std::f64::consts::TAU + chamfera_geom::ANGULAR_TOLERANCE
                {
                    return Err(bad(
                        "the angle must be above 0 and at most 360 degrees".into()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Evaluates a feature's expression against the current parameters.
    pub fn evaluate_feature_value(&self, src: &str, dim: expr::Dim) -> Result<f64, DocError> {
        let values: std::collections::BTreeMap<String, expr::Quantity> = self
            .parameters
            .iter()
            .map(|(n, p)| {
                (
                    n.clone(),
                    expr::Quantity {
                        value: p.value,
                        dim: p.unit.dim(),
                    },
                )
            })
            .collect();
        expr::evaluate_as(src, dim, &values).map_err(|error| DocError::Expression {
            name: "feature".into(),
            error,
        })
    }

    /// Adds a feature at the end of the timeline and returns its id.
    pub fn add_feature(&mut self, kind: FeatureKind) -> Result<u32, DocError> {
        if self.features.len() >= MAX_FEATURES {
            return Err(DocError::BadFeature {
                reason: format!("a design holds at most {MAX_FEATURES} features"),
            });
        }
        self.check_feature(&kind)?;
        let id = self
            .last_feature_id
            .checked_add(1)
            .ok_or(DocError::TooManySketches)?;
        self.last_feature_id = id;
        let n = self
            .features
            .iter()
            .filter(|f| f.kind.label() == kind.label())
            .count();
        self.features.push(Feature {
            id,
            name: format!("{}{}", kind.label(), n + 1),
            suppressed: false,
            kind,
        });
        Ok(id)
    }

    pub fn feature(&self, id: u32) -> Result<&Feature, DocError> {
        self.features
            .iter()
            .find(|f| f.id == id)
            .ok_or(DocError::NoSuchFeature(id))
    }

    pub fn feature_mut(&mut self, id: u32) -> Result<&mut Feature, DocError> {
        self.features
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or(DocError::NoSuchFeature(id))
    }

    pub fn remove_feature(&mut self, id: u32) -> Result<Feature, DocError> {
        let i = self
            .features
            .iter()
            .position(|f| f.id == id)
            .ok_or(DocError::NoSuchFeature(id))?;
        Ok(self.features.remove(i))
    }

    /// Sketches attached to a face of a feature, patterns of it and extrudes up to one of its
    /// faces (so deleting the feature can be refused).
    pub fn feature_users(&self, feature: u32) -> Vec<String> {
        let sketches = self
            .sketches
            .iter()
            .filter(|s| s.attached.as_ref().is_some_and(|a| a.feature == feature))
            .map(|s| s.name.clone());
        let patterns = self
            .features
            .iter()
            .filter(|f| match &f.kind {
                FeatureKind::Pattern(p) => p.source == feature,
                FeatureKind::Extrude(e) => {
                    matches!(&e.to, Some(ExtrudeTo::Face(r)) if r.feature == feature)
                }
                _ => false,
            })
            .map(|f| f.name.clone());
        sketches.chain(patterns).collect()
    }

    /// Features that use a sketch (so deleting the sketch can be refused).
    pub fn sketch_users(&self, sketch: u32) -> Vec<String> {
        self.features
            .iter()
            .filter(|f| f.kind.sketch() == Some(sketch))
            .map(|f| f.name.clone())
            .collect()
    }

    /// The parameters a feature's expressions refer to are users of those parameters.
    pub(crate) fn feature_parameter_users(&self, name: &str) -> Vec<String> {
        self.features
            .iter()
            .filter(|f| {
                f.kind
                    .expressions()
                    .iter()
                    .any(|e| expr::names_in(e).iter().any(|n| n == name))
            })
            .map(|f| f.name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Unit;

    fn plate_design() -> (Document, u32) {
        let mut doc = Document::default();
        let id = doc.add_sketch(chamfera_geom::Frame::XY).unwrap();
        let s = &mut doc.sketch_mut(id).unwrap().sketch;
        let p: Vec<_> = [(0.0, 0.0), (80.0, 0.0), (80.0, 50.0), (0.0, 50.0)]
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4]).unwrap();
        }
        (doc, id)
    }

    #[test]
    fn an_extrude_needs_a_real_profile_and_a_nonzero_distance() {
        let (mut doc, sketch) = plate_design();
        doc.set_parameter_expression("t", "8 mm", Unit::Length)
            .unwrap();
        let ok = FeatureKind::Extrude(Extrude {
            sketch,
            profiles: vec![Point2::new(10.0, 10.0)],
            distance: "t * 2".into(),
            direction: ExtrudeDirection::Normal,
            operation: Operation::NewBody,
            ..Extrude::default()
        });
        let id = doc.add_feature(ok).unwrap();
        assert_eq!(doc.feature(id).unwrap().name, "Extrude1");
        for (seed, distance) in [
            ((500.0, 0.0), "10"),
            ((10.0, 10.0), "0"),
            ((10.0, 10.0), "5 deg"),
        ] {
            let bad = FeatureKind::Extrude(Extrude {
                sketch,
                profiles: vec![Point2::new(seed.0, seed.1)],
                distance: distance.into(),
                direction: ExtrudeDirection::Normal,
                operation: Operation::NewBody,
                ..Extrude::default()
            });
            assert!(doc.add_feature(bad).is_err(), "{seed:?} {distance}");
        }
        // The parameter is now in use by the extrude.
        assert!(doc.remove_parameter("t").is_err());
        assert_eq!(doc.sketch_users(sketch), vec!["Extrude1"]);
    }

    #[test]
    fn features_survive_a_round_trip() {
        let (mut doc, sketch) = plate_design();
        doc.add_feature(FeatureKind::Extrude(Extrude {
            sketch,
            profiles: vec![Point2::new(10.0, 10.0)],
            distance: "12.5".into(),
            direction: ExtrudeDirection::Symmetric,
            operation: Operation::Cut,
            ..Extrude::default()
        }))
        .unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
    }

    fn extrude(sketch: u32) -> Extrude {
        Extrude {
            sketch,
            profiles: vec![Point2::new(10.0, 10.0)],
            ..Extrude::default()
        }
    }

    #[test]
    fn extrude_options_round_trip_and_older_extrudes_read_as_before() {
        let (mut doc, sketch) = plate_design();
        let first = doc
            .add_feature(FeatureKind::Extrude(extrude(sketch)))
            .unwrap();
        doc.add_feature(FeatureKind::Extrude(Extrude {
            direction: ExtrudeDirection::TwoSides,
            distance2: Some("4 mm".into()),
            start_offset: Some("-2 mm".into()),
            taper: Some("5 deg".into()),
            ..extrude(sketch)
        }))
        .unwrap();
        doc.add_feature(FeatureKind::Extrude(Extrude {
            extent: ExtrudeExtent::ToObject,
            to: Some(ExtrudeTo::Face(crate::FaceRef {
                feature: first,
                face: "top".into(),
            })),
            ..extrude(sketch)
        }))
        .unwrap();
        doc.add_feature(FeatureKind::Extrude(Extrude {
            extent: ExtrudeExtent::ThroughAll,
            operation: Operation::Cut,
            ..extrude(sketch)
        }))
        .unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        // The extrude up to its top face uses feature 1.
        assert_eq!(doc.feature_users(first), vec!["Extrude3"]);
        // An extrude as format 12 and older wrote it: no options, which mean by
        // distance from the sketch plane, without a taper.
        let old: Extrude = serde_json::from_str(
            r#"{"sketch":1,"profiles":[{"x":1,"y":1}],"distance":"10 mm","direction":"symmetric"}"#,
        )
        .unwrap();
        assert_eq!(
            old,
            Extrude {
                sketch: 1,
                profiles: vec![Point2::new(1.0, 1.0)],
                direction: ExtrudeDirection::Symmetric,
                ..Extrude::default()
            }
        );
        // And a plain extrude still writes only those fields.
        let text = serde_json::to_string(&old).unwrap();
        for key in ["extent", "distance2", "start_offset", "taper", "\"to\""] {
            assert!(!text.contains(key), "{text}");
        }
    }

    #[test]
    fn bad_extrude_options_are_refused() {
        let (mut doc, sketch) = plate_design();
        let bad = [
            // Two sides without a second distance, or with a zero or negative one.
            Extrude {
                direction: ExtrudeDirection::TwoSides,
                ..extrude(sketch)
            },
            Extrude {
                direction: ExtrudeDirection::TwoSides,
                distance2: Some("0".into()),
                ..extrude(sketch)
            },
            Extrude {
                direction: ExtrudeDirection::TwoSides,
                distance2: Some("-3".into()),
                ..extrude(sketch)
            },
            // A taper of 89 degrees or more, or one in mm.
            Extrude {
                taper: Some("89 deg".into()),
                ..extrude(sketch)
            },
            Extrude {
                taper: Some("5 mm".into()),
                ..extrude(sketch)
            },
            // A start offset that is an angle or out of range.
            Extrude {
                start_offset: Some("5 deg".into()),
                ..extrude(sketch)
            },
            Extrude {
                start_offset: Some("1e9 mm".into()),
                ..extrude(sketch)
            },
            // To object without an object, or with a missing feature.
            Extrude {
                extent: ExtrudeExtent::ToObject,
                ..extrude(sketch)
            },
            Extrude {
                extent: ExtrudeExtent::ToObject,
                to: Some(ExtrudeTo::Face(crate::FaceRef {
                    feature: 99,
                    face: "top".into(),
                })),
                ..extrude(sketch)
            },
        ];
        for e in bad {
            assert!(
                doc.add_feature(FeatureKind::Extrude(e.clone())).is_err(),
                "{e:?}"
            );
        }
        assert!(doc.features.is_empty());
        // Through all needs no distance at all.
        doc.add_feature(FeatureKind::Extrude(Extrude {
            extent: ExtrudeExtent::ThroughAll,
            distance: "0".into(),
            ..extrude(sketch)
        }))
        .unwrap();
    }
}

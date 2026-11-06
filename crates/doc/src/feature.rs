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

/// Which way an extrude goes from its sketch plane.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    pub sketch: u32,
    /// A point inside each profile to extrude (sketch coordinates).
    pub profiles: Vec<Point2>,
    /// The distance, an expression such as `10 mm` or `thickness * 2`.
    pub distance: String,
    #[serde(default)]
    pub direction: ExtrudeDirection,
    #[serde(default)]
    pub operation: Operation,
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

/// A shell: the solids built so far hollowed to walls of `thickness`, with the named
/// faces removed to open them. Faces are named like everywhere else (`2:top` is face
/// `top` of feature 2's shape), so the shell finds them again after every rebuild. A
/// negative thickness grows the walls outward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shell {
    /// The faces to remove (open); may be empty for a closed, hollow body.
    #[serde(default)]
    pub faces: Vec<String>,
    /// The wall thickness, an expression such as `2 mm`.
    pub thickness: String,
}

/// Most faces one shell may open.
pub const MAX_SHELL_FACES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureKind {
    Extrude(Extrude),
    Revolve(Revolve),
    Pattern(Pattern),
    Blend(Blend),
    Shell(Shell),
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
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) | FeatureKind::Shell(_) => None,
        }
    }

    /// The operation; a pattern repeats its source's (see the rebuild).
    pub fn operation(&self) -> Operation {
        match self {
            FeatureKind::Extrude(e) => e.operation,
            FeatureKind::Revolve(r) => r.operation,
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) | FeatureKind::Shell(_) => {
                Operation::NewBody
            }
        }
    }

    pub fn profiles(&self) -> &[Point2] {
        match self {
            FeatureKind::Extrude(e) => &e.profiles,
            FeatureKind::Revolve(r) => &r.profiles,
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) | FeatureKind::Shell(_) => &[],
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
            FeatureKind::Shell(_) => "Shell",
        }
    }

    /// The expressions this feature evaluates.
    fn expressions(&self) -> Vec<&str> {
        match self {
            FeatureKind::Extrude(e) => vec![e.distance.as_str()],
            FeatureKind::Revolve(r) => vec![r.angle.as_str()],
            FeatureKind::Blend(b) => vec![b.size.as_str()],
            FeatureKind::Shell(s) => vec![s.thickness.as_str()],
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
        if let FeatureKind::Shell(s) = kind {
            if s.faces.len() > MAX_SHELL_FACES {
                return Err(bad(format!("open at most {MAX_SHELL_FACES} faces")));
            }
            if s.faces
                .iter()
                .any(|f| f.is_empty() || f.chars().count() > 512)
            {
                return Err(bad("a face name is empty or too long".into()));
            }
            let t = self.evaluate_feature_value(&s.thickness, expr::Dim::LENGTH)?;
            if !t.is_finite() || t.abs() <= chamfera_geom::LINEAR_TOLERANCE {
                return Err(bad("the thickness must not be 0".into()));
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
            FeatureKind::Extrude(e) => {
                let d = self.evaluate_feature_value(&e.distance, expr::Dim::LENGTH)?;
                if !d.is_finite() || d.abs() <= chamfera_geom::LINEAR_TOLERANCE {
                    return Err(bad("the distance must not be 0".into()));
                }
            }
            FeatureKind::Pattern(_) | FeatureKind::Blend(_) | FeatureKind::Shell(_) => {}
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
        self.assembly.adopt(id);
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

    /// Sketches attached to a face of a feature, and patterns of it (so deleting the
    /// feature can be refused).
    pub fn feature_users(&self, feature: u32) -> Vec<String> {
        let sketches = self
            .sketches
            .iter()
            .filter(|s| s.attached.as_ref().is_some_and(|a| a.feature == feature))
            .map(|s| s.name.clone());
        let patterns = self
            .features
            .iter()
            .filter(|f| matches!(&f.kind, FeatureKind::Pattern(p) if p.source == feature))
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
        }))
        .unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
    }
}

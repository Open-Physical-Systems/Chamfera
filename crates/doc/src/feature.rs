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
    /// The axis: a sketch line (its curve id) the profile turns around. Ignored when
    /// `axis_feature` is set.
    pub axis: u32,
    /// A construction axis (its feature id) to turn around instead of a sketch line. It
    /// must lie in the sketch plane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_feature: Option<u32>,
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
        /// A construction axis (its feature id) to turn around instead of `origin` and
        /// `axis`, followed when the design rebuilds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        axis_feature: Option<u32>,
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

/// The plane a mirror reflects in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MirrorPlane {
    /// The XY (top) origin plane, z = 0.
    Xy,
    /// The XZ (front) origin plane, y = 0.
    Xz,
    /// The YZ (right) origin plane, x = 0.
    Yz,
    /// A flat face of a feature's solid, found by name at rebuild (like a sketch on a face).
    Face { feature: u32, face: String },
    /// Any plane, by a point on it and its normal (construction planes will refer here).
    Plane {
        origin: chamfera_geom::Point3,
        normal: chamfera_geom::Vec3,
    },
}

impl MirrorPlane {
    /// The origin plane named `xy`, `xz` or `yz`.
    pub fn origin(name: &str) -> Option<MirrorPlane> {
        Some(match name {
            "xy" => MirrorPlane::Xy,
            "xz" => MirrorPlane::Xz,
            "yz" => MirrorPlane::Yz,
            _ => return None,
        })
    }
}

/// Mirror images of bodies or of features' shapes. Mirrored bodies are separate (new body)
/// or joined to what they touch, so a half joined to its mirror makes a symmetric part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mirror {
    /// Bodies built so far, by name (`Extrude1`), mirrored as they are at this point.
    #[serde(default)]
    pub bodies: Vec<String>,
    /// Features whose own shapes are mirrored (like a pattern of them).
    #[serde(default)]
    pub features: Vec<u32>,
    pub plane: MirrorPlane,
    #[serde(default)]
    pub operation: Operation,
}

/// How a move places bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transform", rename_all = "snake_case")]
pub enum MoveTransform {
    /// Along x, y and z, each an expression such as `10 mm`.
    Translate { dx: String, dy: String, dz: String },
    /// Turned by `angle` about the axis through `origin` along `axis` (right hand rule).
    Rotate {
        origin: chamfera_geom::Point3,
        axis: chamfera_geom::Vec3,
        angle: String,
    },
    /// Moved by the vector from `from` to `to`.
    PointToPoint {
        from: chamfera_geom::Point3,
        to: chamfera_geom::Point3,
    },
}

/// Moves (or copies) bodies built so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Move {
    /// The bodies, by name.
    pub bodies: Vec<String>,
    #[serde(flatten)]
    pub transform: MoveTransform,
    /// Keep the originals and add moved copies.
    #[serde(default)]
    pub copy: bool,
}

/// Most bodies or features one mirror or move may name.
pub const MAX_TRANSFORM_ITEMS: usize = 1_000;
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
    /// A construction plane, axis or point.
    Construct(crate::construct::Construction),
    Blend(Blend),
    Mirror(Mirror),
    Move(Move),
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
            FeatureKind::Pattern(_)
            | FeatureKind::Construct(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Move(_) => None,
        }
    }

    /// The operation; a pattern repeats its source's (see the rebuild).
    pub fn operation(&self) -> Operation {
        match self {
            FeatureKind::Extrude(e) => e.operation,
            FeatureKind::Revolve(r) => r.operation,
            FeatureKind::Mirror(m) => m.operation,
            FeatureKind::Pattern(_)
            | FeatureKind::Construct(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Move(_) => Operation::NewBody,
        }
    }

    pub fn profiles(&self) -> &[Point2] {
        match self {
            FeatureKind::Extrude(e) => &e.profiles,
            FeatureKind::Revolve(r) => &r.profiles,
            FeatureKind::Pattern(_)
            | FeatureKind::Construct(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Move(_) => &[],
        }
    }

    fn label(&self) -> &'static str {
        match self {
            FeatureKind::Extrude(_) => "Extrude",
            FeatureKind::Revolve(_) => "Revolve",
            FeatureKind::Pattern(_) => "Pattern",
            FeatureKind::Construct(c) => c.kind().label(),
            FeatureKind::Blend(b) => match b.blend {
                BlendKind::Fillet => "Fillet",
                BlendKind::Chamfer => "Chamfer",
            },
            FeatureKind::Mirror(_) => "Mirror",
            FeatureKind::Move(m) => {
                if m.copy {
                    "Copy"
                } else {
                    "Move"
                }
            }
            FeatureKind::Shell(_) => "Shell",
        }
    }

    /// The expressions this feature evaluates.
    fn expressions(&self) -> Vec<&str> {
        match self {
            FeatureKind::Extrude(e) => vec![e.distance.as_str()],
            FeatureKind::Revolve(r) => vec![r.angle.as_str()],
            FeatureKind::Blend(b) => vec![b.size.as_str()],
            FeatureKind::Mirror(_) => Vec::new(),
            FeatureKind::Move(m) => match &m.transform {
                MoveTransform::Translate { dx, dy, dz } => {
                    vec![dx.as_str(), dy.as_str(), dz.as_str()]
                }
                MoveTransform::Rotate { angle, .. } => vec![angle.as_str()],
                MoveTransform::PointToPoint { .. } => Vec::new(),
            },
            FeatureKind::Shell(s) => vec![s.thickness.as_str()],
            FeatureKind::Pattern(p) => match &p.layout {
                PatternLayout::Linear { spacing, .. } => vec![p.count.as_str(), spacing.as_str()],
                PatternLayout::Circular { angle, .. } => vec![p.count.as_str(), angle.as_str()],
            },
            FeatureKind::Construct(c) => c.expressions(),
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
        if let FeatureKind::Mirror(m) = kind {
            return self.check_mirror(m);
        }
        if let FeatureKind::Move(m) = kind {
            return self.check_move(m);
        }
        if let FeatureKind::Construct(c) = kind {
            return self.check_construction(c);
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
            FeatureKind::Pattern(_)
            | FeatureKind::Construct(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Move(_) => {}
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
                if let Some(axis) = r.axis_feature {
                    self.check_construct_axis(axis)?;
                    return Ok(());
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
                axis_feature,
            } => {
                if let Some(a) = axis_feature {
                    self.check_construct_axis(*a)?;
                }
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

    fn check_body_names(&self, bodies: &[String]) -> Result<(), DocError> {
        if bodies
            .iter()
            .any(|b| b.trim().is_empty() || b.chars().count() > 256)
        {
            return Err(DocError::BadFeature {
                reason: "a body name is empty or longer than 256 characters".into(),
            });
        }
        Ok(())
    }

    fn check_mirror(&self, m: &Mirror) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        let n = m.bodies.len() + m.features.len();
        if n == 0 || n > MAX_TRANSFORM_ITEMS {
            return Err(bad(format!(
                "pick 1 to {MAX_TRANSFORM_ITEMS} bodies or features to mirror"
            )));
        }
        self.check_body_names(&m.bodies)?;
        for id in &m.features {
            self.feature(*id)?;
        }
        match &m.plane {
            MirrorPlane::Xy | MirrorPlane::Xz | MirrorPlane::Yz => {}
            MirrorPlane::Face { feature, face } => {
                self.feature(*feature)?;
                if face.is_empty() || face.chars().count() > 256 {
                    return Err(bad("the mirror face name is empty or too long".into()));
                }
            }
            MirrorPlane::Plane { origin, normal } => {
                if !origin.is_valid() || normal.normalized().is_none() {
                    return Err(bad("the mirror plane needs a point and a normal".into()));
                }
            }
        }
        Ok(())
    }

    /// The rigid motion a move makes, checked (finite, within the working range).
    pub fn move_transform(&self, m: &Move) -> Result<chamfera_geom::Transform, DocError> {
        use chamfera_geom::{MAX_COORDINATE, Transform, Vec3};
        let bad = |reason: &str| DocError::BadFeature {
            reason: reason.into(),
        };
        let t = match &m.transform {
            MoveTransform::Translate { dx, dy, dz } => {
                let mut v = [0.0; 3];
                for (slot, e) in v.iter_mut().zip([dx, dy, dz]) {
                    let d = self.evaluate_feature_value(e, expr::Dim::LENGTH)?;
                    if !d.is_finite() || d.abs() > MAX_COORDINATE {
                        return Err(bad("each distance must be within the working range"));
                    }
                    *slot = d;
                }
                Transform::translation(Vec3::new(v[0], v[1], v[2]))
            }
            MoveTransform::Rotate {
                origin,
                axis,
                angle,
            } => {
                let a = self.evaluate_feature_value(angle, expr::Dim::ANGLE)?;
                if !origin.is_valid() || !a.is_finite() || a.abs() > 1e6 {
                    return Err(bad("the rotation needs a point and a finite angle"));
                }
                Transform::rotation_about(*origin, *axis, a)
            }
            MoveTransform::PointToPoint { from, to } => {
                if !from.is_valid() || !to.is_valid() {
                    return Err(bad("both points must be within the working range"));
                }
                Transform::translation(*to - *from)
            }
        };
        t.ok_or_else(|| bad("the move needs a non-zero axis and finite values"))
    }

    fn check_move(&self, m: &Move) -> Result<(), DocError> {
        if m.bodies.is_empty() || m.bodies.len() > MAX_TRANSFORM_ITEMS {
            return Err(DocError::BadFeature {
                reason: format!("pick 1 to {MAX_TRANSFORM_ITEMS} bodies to move"),
            });
        }
        self.check_body_names(&m.bodies)?;
        self.move_transform(m).map(|_| ())
    }

    /// Checks that `id` is a construction axis.
    fn check_construct_axis(&self, id: u32) -> Result<(), DocError> {
        match &self.feature(id)?.kind {
            FeatureKind::Construct(c) if c.kind() == crate::construct::ConstructKind::Axis => {
                Ok(())
            }
            _ => Err(DocError::BadFeature {
                reason: format!("feature {id} is not a construction axis"),
            }),
        }
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

    /// Sketches attached to a face of a feature or drawn on a construction plane, patterns
    /// of it, and constructions and revolves that use it (so deleting the feature can be
    /// refused).
    pub fn feature_users(&self, feature: u32) -> Vec<String> {
        let sketches = self
            .sketches
            .iter()
            .filter(|s| {
                s.attached.as_ref().is_some_and(|a| a.feature == feature)
                    || s.on_plane == Some(feature)
            })
            .map(|s| s.name.clone());
        let features = self
            .features
            .iter()
            .filter(|f| match &f.kind {
                FeatureKind::Pattern(p) => {
                    p.source == feature
                        || matches!(
                            p.layout,
                            PatternLayout::Circular { axis_feature: Some(a), .. } if a == feature
                        )
                }
                FeatureKind::Revolve(r) => r.axis_feature == Some(feature),
                FeatureKind::Construct(c) => c.features_used().contains(&feature),
                FeatureKind::Mirror(m) => {
                    m.features.contains(&feature)
                        || matches!(&m.plane, MirrorPlane::Face { feature: g, .. } if *g == feature)
                }
                FeatureKind::Extrude(_)
                | FeatureKind::Blend(_)
                | FeatureKind::Shell(_)
                | FeatureKind::Move(_) => false,
            })
            .map(|f| f.name.clone());
        sketches.chain(features).collect()
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
    fn mirrors_and_moves_are_checked_and_survive_a_round_trip() {
        use chamfera_geom::{Point3, Vec3};
        let (mut doc, sketch) = plate_design();
        let ext = doc
            .add_feature(FeatureKind::Extrude(Extrude {
                sketch,
                profiles: vec![Point2::new(10.0, 10.0)],
                distance: "5".into(),
                direction: ExtrudeDirection::Normal,
                operation: Operation::NewBody,
            }))
            .unwrap();
        let mirror = FeatureKind::Mirror(Mirror {
            bodies: vec!["Extrude1".into()],
            features: vec![],
            plane: MirrorPlane::Yz,
            operation: Operation::Join,
        });
        assert_eq!(doc.add_feature(mirror).map(|_| ()), Ok(()));
        let by_feature = FeatureKind::Mirror(Mirror {
            bodies: vec![],
            features: vec![ext],
            plane: MirrorPlane::Face {
                feature: ext,
                face: "top".into(),
            },
            operation: Operation::NewBody,
        });
        doc.add_feature(by_feature).unwrap();
        // The second mirror uses the extrude's shape and face, so the extrude cannot be
        // deleted from under it; the first names a body only.
        assert_eq!(doc.feature_users(ext), vec!["Mirror2"]);
        let moved = FeatureKind::Move(Move {
            bodies: vec!["Extrude1".into()],
            transform: MoveTransform::Translate {
                dx: "10 mm".into(),
                dy: "0".into(),
                dz: "-2".into(),
            },
            copy: true,
        });
        let id = doc.add_feature(moved).unwrap();
        assert_eq!(doc.feature(id).unwrap().name, "Copy1");
        let t = doc
            .move_transform(&Move {
                bodies: vec!["a".into()],
                transform: MoveTransform::Rotate {
                    origin: Point3::ORIGIN,
                    axis: Vec3::Z,
                    angle: "90 deg".into(),
                },
                copy: false,
            })
            .unwrap();
        // A quarter turn about z takes (50, 0, 0) to (0, 50, 0).
        let p = t.apply_point(Point3::new(50.0, 0.0, 0.0));
        assert!(p.distance(Point3::new(0.0, 50.0, 0.0)) < 1e-9);
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        // Bad ones are refused.
        for bad in [
            FeatureKind::Mirror(Mirror {
                bodies: vec![],
                features: vec![],
                plane: MirrorPlane::Xy,
                operation: Operation::NewBody,
            }),
            FeatureKind::Mirror(Mirror {
                bodies: vec!["".into()],
                features: vec![],
                plane: MirrorPlane::Xy,
                operation: Operation::NewBody,
            }),
            FeatureKind::Mirror(Mirror {
                bodies: vec![],
                features: vec![99],
                plane: MirrorPlane::Xy,
                operation: Operation::NewBody,
            }),
            FeatureKind::Mirror(Mirror {
                bodies: vec!["a".into()],
                features: vec![],
                plane: MirrorPlane::Plane {
                    origin: Point3::ORIGIN,
                    normal: Vec3::ZERO,
                },
                operation: Operation::NewBody,
            }),
            FeatureKind::Move(Move {
                bodies: vec!["a".into()],
                transform: MoveTransform::Translate {
                    dx: "5 deg".into(),
                    dy: "0".into(),
                    dz: "0".into(),
                },
                copy: false,
            }),
            FeatureKind::Move(Move {
                bodies: vec!["a".into()],
                transform: MoveTransform::Rotate {
                    origin: Point3::ORIGIN,
                    axis: Vec3::ZERO,
                    angle: "1".into(),
                },
                copy: false,
            }),
            FeatureKind::Move(Move {
                bodies: vec![],
                transform: MoveTransform::PointToPoint {
                    from: Point3::ORIGIN,
                    to: Point3::ORIGIN,
                },
                copy: false,
            }),
        ] {
            assert!(doc.add_feature(bad.clone()).is_err(), "{bad:?}");
        }
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

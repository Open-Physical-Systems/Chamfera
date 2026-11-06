//! Surface features (format 28): surface bodies (open shells) made by sweeping sketch
//! profiles or open sketch curves without caps, planar patches, stitching and trimming.
//!
//! A swept surface takes closed profiles (a point inside each, as an extrude does) and
//! open curves (their ids, joined end to end in the order given), or both. Bodies are
//! named as everywhere else and found by name when the timeline builds; edges are named
//! `2:top 0` (edge `top 0` of feature 2's shape).

use crate::construct::PlaneRef;
use crate::{DocError, Document, expr};
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// Most profiles, curves, edges or bodies one surface feature may name.
pub const MAX_SURFACE_ITEMS: usize = 1_000;

/// Sketch profiles and open curves swept as surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceShapes {
    pub sketch: u32,
    /// A point inside each closed profile (sketch coordinates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Point2>,
    /// Open curves (ids in the sketch), joined end to end into one chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curves: Vec<u32>,
}

/// The sides of profiles or curves pulled straight out, without caps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceExtrude {
    #[serde(flatten)]
    pub shapes: SurfaceShapes,
    /// An expression such as `10 mm`; negative goes against the sketch normal.
    pub distance: String,
    /// Half the distance each way.
    #[serde(default)]
    pub symmetric: bool,
}

/// Profiles or curves turned about an axis, without caps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRevolve {
    #[serde(flatten)]
    pub shapes: SurfaceShapes,
    /// A sketch line (its curve id); ignored when `axis_feature` is set.
    pub axis: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_feature: Option<u32>,
    /// An angle expression such as `360 deg`.
    pub angle: String,
}

/// Profiles or curves swept along a path, without caps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSweep {
    #[serde(flatten)]
    pub shapes: SurfaceShapes,
    pub path_sketch: u32,
    pub path: Vec<u32>,
}

/// A planar face filling closed sketch profiles, or a closed planar loop of edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfacePatch {
    /// The sketch of the profiles (with `profiles`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Point2>,
    /// Model edges by name, all of one body (instead of a sketch).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<String>,
}

/// Surface bodies joined along the edges they share; a closed result is a solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceStitch {
    pub bodies: Vec<String>,
}

/// What a trim cuts with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrimTool {
    /// A plane (origin plane, flat face or construction plane).
    Plane { plane: PlaneRef },
    /// Another body: a closed solid, or a surface of one flat face (its plane).
    Body { body: String },
}

/// Which part of a trimmed surface is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimKeep {
    /// Behind the plane (against its normal), or outside a solid tool.
    #[default]
    Below,
    /// In front of the plane, or inside a solid tool.
    Above,
}

impl TrimKeep {
    pub fn parse(s: &str) -> Option<TrimKeep> {
        match s {
            "below" | "outside" => Some(TrimKeep::Below),
            "above" | "inside" => Some(TrimKeep::Above),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            TrimKeep::Below => "below",
            TrimKeep::Above => "above",
        }
    }
}

/// A surface trimmed: the part on one side of a tool is kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceTrim {
    pub body: String,
    pub tool: TrimTool,
    #[serde(default)]
    pub keep: TrimKeep,
}

/// A surface feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "surface", rename_all = "snake_case")]
pub enum SurfaceFeature {
    Extrude(SurfaceExtrude),
    Revolve(SurfaceRevolve),
    Sweep(SurfaceSweep),
    Patch(SurfacePatch),
    Stitch(SurfaceStitch),
    Trim(SurfaceTrim),
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

fn check_names(what: &str, names: &[String], min: usize) -> Result<(), DocError> {
    if names.len() < min || names.len() > MAX_SURFACE_ITEMS {
        return Err(bad(format!("pick {min} to {MAX_SURFACE_ITEMS} {what}")));
    }
    if names
        .iter()
        .any(|n| n.trim().is_empty() || n.chars().count() > 512)
    {
        return Err(bad(format!("a name among the {what} is empty or too long")));
    }
    Ok(())
}

/// The feature ids an edge or face name refers to (`2:top 0` names 2).
fn named_features(name: &str) -> impl Iterator<Item = u32> + '_ {
    name.split_whitespace()
        .filter_map(|w| w.split_once(':'))
        .filter_map(|(id, _)| id.parse::<u32>().ok())
}

impl SurfaceFeature {
    /// The sketch its profiles or curves come from.
    pub fn sketch(&self) -> Option<u32> {
        match self {
            SurfaceFeature::Extrude(e) => Some(e.shapes.sketch),
            SurfaceFeature::Revolve(r) => Some(r.shapes.sketch),
            SurfaceFeature::Sweep(s) => Some(s.shapes.sketch),
            SurfaceFeature::Patch(p) => p.sketch,
            SurfaceFeature::Stitch(_) | SurfaceFeature::Trim(_) => None,
        }
    }

    /// Every sketch it uses (a sweep's path sketch too).
    pub fn sketches(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.sketch().into_iter().collect();
        if let SurfaceFeature::Sweep(s) = self
            && !out.contains(&s.path_sketch)
        {
            out.push(s.path_sketch);
        }
        out
    }

    /// The profile seeds in its sketch.
    pub fn profiles(&self) -> &[Point2] {
        match self {
            SurfaceFeature::Extrude(e) => &e.shapes.profiles,
            SurfaceFeature::Revolve(r) => &r.shapes.profiles,
            SurfaceFeature::Sweep(s) => &s.shapes.profiles,
            SurfaceFeature::Patch(p) => &p.profiles,
            SurfaceFeature::Stitch(_) | SurfaceFeature::Trim(_) => &[],
        }
    }

    /// The default name stem, such as `SurfaceExtrude` (so `SurfaceExtrude1`).
    pub fn label(&self) -> &'static str {
        match self {
            SurfaceFeature::Extrude(_) => "SurfaceExtrude",
            SurfaceFeature::Revolve(_) => "SurfaceRevolve",
            SurfaceFeature::Sweep(_) => "SurfaceSweep",
            SurfaceFeature::Patch(_) => "Patch",
            SurfaceFeature::Stitch(_) => "Stitch",
            SurfaceFeature::Trim(_) => "Trim",
        }
    }

    /// The expressions it evaluates.
    pub fn expressions(&self) -> Vec<&str> {
        match self {
            SurfaceFeature::Extrude(e) => vec![e.distance.as_str()],
            SurfaceFeature::Revolve(r) => vec![r.angle.as_str()],
            _ => Vec::new(),
        }
    }

    /// Its length expressions, for changing the design's unit.
    pub fn length_expressions_mut(&mut self) -> Vec<&mut String> {
        match self {
            SurfaceFeature::Extrude(e) => vec![&mut e.distance],
            _ => Vec::new(),
        }
    }

    /// The features it refers to by id (axes, planes, the features of named edges).
    pub fn features_used(&self) -> Vec<u32> {
        let mut out: Vec<u32> = match self {
            SurfaceFeature::Revolve(r) => r.axis_feature.into_iter().collect(),
            SurfaceFeature::Patch(p) => p.edges.iter().flat_map(|e| named_features(e)).collect(),
            SurfaceFeature::Trim(t) => match &t.tool {
                TrimTool::Plane { plane } => match plane {
                    PlaneRef::Origin { .. } => Vec::new(),
                    PlaneRef::Face(f) => vec![f.feature],
                    PlaneRef::Construct { feature } => vec![*feature],
                },
                TrimTool::Body { .. } => Vec::new(),
            },
            _ => Vec::new(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl Document {
    fn check_surface_shapes(&self, s: &SurfaceShapes) -> Result<(), DocError> {
        let f = self.sketch(s.sketch)?;
        if s.profiles.is_empty() && s.curves.is_empty() {
            return Err(bad("pick at least one profile or curve"));
        }
        if s.profiles.len() + s.curves.len() > MAX_SURFACE_ITEMS {
            return Err(bad(format!(
                "at most {MAX_SURFACE_ITEMS} profiles and curves"
            )));
        }
        for p in &s.profiles {
            if !p.is_valid() {
                return Err(bad("a profile point is not a finite coordinate"));
            }
            if f.sketch.region_at(*p).is_none() {
                return Err(bad(format!(
                    "no closed profile of {} contains ({:.3}, {:.3})",
                    f.name, p.x, p.y
                )));
            }
        }
        for c in &s.curves {
            if f.sketch.curve(chamfera_sketch::CurveId(*c)).is_none() {
                return Err(bad(format!("curve {c} is not in {}", f.name)));
            }
        }
        Ok(())
    }

    /// Checks a surface feature against the design.
    pub fn check_surface(&self, s: &SurfaceFeature) -> Result<(), DocError> {
        match s {
            SurfaceFeature::Extrude(e) => {
                self.check_surface_shapes(&e.shapes)?;
                let d = self.evaluate_feature_value(&e.distance, expr::Dim::LENGTH)?;
                if !d.is_finite() || d.abs() <= chamfera_geom::LINEAR_TOLERANCE {
                    return Err(bad("the distance must not be 0"));
                }
                if d.abs() > chamfera_geom::MAX_COORDINATE {
                    return Err(bad("the distance leaves the working range"));
                }
                Ok(())
            }
            SurfaceFeature::Revolve(r) => {
                self.check_surface_shapes(&r.shapes)?;
                let a = self.evaluate_feature_value(&r.angle, expr::Dim::ANGLE)?;
                if !a.is_finite()
                    || a <= 0.0
                    || a > std::f64::consts::TAU + chamfera_geom::ANGULAR_TOLERANCE
                {
                    return Err(bad("the angle must be above 0 and at most 360 degrees"));
                }
                if let Some(axis) = r.axis_feature {
                    return self.check_construct_axis(axis);
                }
                let f = self.sketch(r.shapes.sketch)?;
                match f.sketch.curve(chamfera_sketch::CurveId(r.axis)) {
                    Some(c) if c.is_line() => {}
                    _ => {
                        return Err(bad(format!(
                            "the axis {} is not a line of the sketch",
                            r.axis
                        )));
                    }
                }
                if r.shapes.curves.contains(&r.axis) {
                    return Err(bad("the axis cannot also be a curve to revolve"));
                }
                Ok(())
            }
            SurfaceFeature::Sweep(w) => {
                self.check_surface_shapes(&w.shapes)?;
                let p = self.sketch(w.path_sketch)?;
                if w.path.is_empty() || w.path.len() > crate::feature::MAX_SWEEP_PATH_CURVES {
                    return Err(bad(format!(
                        "pick 1 to {} path curves",
                        crate::feature::MAX_SWEEP_PATH_CURVES
                    )));
                }
                for c in &w.path {
                    if p.sketch.curve(chamfera_sketch::CurveId(*c)).is_none() {
                        return Err(bad(format!("path curve {c} is not in {}", p.name)));
                    }
                }
                Ok(())
            }
            SurfaceFeature::Patch(p) => match (p.sketch, p.edges.is_empty()) {
                (Some(sketch), true) => self
                    .check_surface_shapes(&SurfaceShapes {
                        sketch,
                        profiles: p.profiles.clone(),
                        curves: Vec::new(),
                    })
                    .and_then(|()| {
                        if p.profiles.is_empty() {
                            Err(bad("pick at least one closed profile to patch"))
                        } else {
                            Ok(())
                        }
                    }),
                (None, false) => check_names("edges", &p.edges, 1),
                _ => Err(bad(
                    "a patch fills either sketch profiles or a loop of edges, not both",
                )),
            },
            SurfaceFeature::Stitch(s) => check_names("bodies", &s.bodies, 1),
            SurfaceFeature::Trim(t) => {
                check_names("body", std::slice::from_ref(&t.body), 1)?;
                match &t.tool {
                    TrimTool::Plane { plane } => self.check_trim_plane(plane),
                    TrimTool::Body { body } => {
                        check_names("tool body", std::slice::from_ref(body), 1)?;
                        if *body == t.body {
                            return Err(bad("a surface cannot trim itself"));
                        }
                        Ok(())
                    }
                }
            }
        }
    }

    fn check_trim_plane(&self, p: &PlaneRef) -> Result<(), DocError> {
        match p {
            PlaneRef::Origin { .. } => Ok(()),
            PlaneRef::Face(f) => {
                self.feature(f.feature)?;
                if f.face.is_empty() || f.face.chars().count() > 512 {
                    return Err(bad("the face name is empty or too long"));
                }
                Ok(())
            }
            PlaneRef::Construct { feature } => match &self.feature(*feature)?.kind {
                crate::FeatureKind::Construct(c)
                    if c.kind() == crate::construct::ConstructKind::Plane =>
                {
                    Ok(())
                }
                _ => Err(bad(format!(
                    "feature {feature} is not a construction plane"
                ))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeatureKind;
    use crate::construct::OriginPlane;

    #[test]
    fn surface_features_survive_a_round_trip_and_bad_ones_are_refused() {
        let mut doc = Document::default();
        let stitch = FeatureKind::Surface(SurfaceFeature::Stitch(SurfaceStitch {
            bodies: vec!["A".into(), "B".into()],
        }));
        let trim = FeatureKind::Surface(SurfaceFeature::Trim(SurfaceTrim {
            body: "A".into(),
            tool: TrimTool::Plane {
                plane: PlaneRef::Origin {
                    plane: OriginPlane::Xy,
                },
            },
            keep: TrimKeep::Above,
        }));
        let patch = FeatureKind::Surface(SurfaceFeature::Patch(SurfacePatch {
            sketch: None,
            profiles: Vec::new(),
            edges: vec!["2:top 0".into()],
        }));
        for k in [stitch, trim, patch] {
            doc.add_feature(k).unwrap();
        }
        let text = doc.to_json().unwrap();
        assert!(text.contains("\"surface\": \"trim\"") || text.contains("\"surface\":\"trim\""));
        let back = Document::from_json(&text).unwrap();
        assert_eq!(back.features, doc.features);
        let refused = [
            FeatureKind::Surface(SurfaceFeature::Stitch(SurfaceStitch { bodies: vec![] })),
            FeatureKind::Surface(SurfaceFeature::Trim(SurfaceTrim {
                body: "A".into(),
                tool: TrimTool::Body { body: "A".into() },
                keep: TrimKeep::Below,
            })),
            FeatureKind::Surface(SurfaceFeature::Patch(SurfacePatch {
                sketch: Some(1),
                profiles: Vec::new(),
                edges: vec!["2:top 0".into()],
            })),
            FeatureKind::Surface(SurfaceFeature::Extrude(SurfaceExtrude {
                shapes: SurfaceShapes {
                    sketch: 99,
                    profiles: vec![],
                    curves: vec![1],
                },
                distance: "10".into(),
                symmetric: false,
            })),
        ];
        for k in refused {
            assert!(doc.check_feature(&k).is_err(), "{k:?}");
        }
    }
}

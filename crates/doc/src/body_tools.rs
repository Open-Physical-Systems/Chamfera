//! Body tools on the timeline: combine bodies, split a body or faces by a plane, thicken
//! faces, and remove bodies from the result (format 22).
//!
//! Bodies are named as everywhere else (`Extrude1`, `Extrude1 (2)`) and found by name when
//! the timeline builds; faces are named `2:top` (face `top` of feature 2's shape).

use crate::construct::PlaneRef;
use crate::{DocError, Document, FeatureKind, Operation, expr};
use serde::{Deserialize, Serialize};

/// Most bodies or faces one body tool may name.
pub const MAX_BODY_TOOL_ITEMS: usize = 1_000;

/// A boolean between bodies built so far: `target` joined with, cut by or intersected
/// with each of `tools`. The tools are used up unless `keep_tools`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Combine {
    pub target: String,
    pub tools: Vec<String>,
    /// Join, cut or intersect (never new body).
    pub operation: Operation,
    #[serde(default)]
    pub keep_tools: bool,
}

/// Each named body cut in two by a plane: the part behind the plane keeps the body's
/// name, the part in front is a new body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitBody {
    pub bodies: Vec<String>,
    pub plane: PlaneRef,
}

/// What a split face is cut with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SplitFaceTool {
    /// A plane (origin plane, flat face or construction plane).
    Plane { plane: PlaneRef },
    /// A sketch line, carried along the sketch's normal onto the faces (so the plane
    /// through the line, square to its sketch).
    SketchLine { sketch: u32, curve: u32 },
}

/// Flat faces cut along a plane, so the body has more faces but the same shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitFace {
    pub faces: Vec<String>,
    pub tool: SplitFaceTool,
}

/// Which way a thicken grows from its faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThickenDirection {
    /// Along the face's outward normal (a negative thickness goes inward).
    #[default]
    OneSide,
    /// Half the thickness each way.
    Symmetric,
    /// The whole thickness each way.
    BothSides,
}

impl ThickenDirection {
    pub fn parse(s: &str) -> Option<ThickenDirection> {
        Some(match s {
            "one_side" => ThickenDirection::OneSide,
            "symmetric" => ThickenDirection::Symmetric,
            "both_sides" => ThickenDirection::BothSides,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            ThickenDirection::OneSide => "one_side",
            ThickenDirection::Symmetric => "symmetric",
            ThickenDirection::BothSides => "both_sides",
        }
    }

    /// The offsets `(lo, hi)` along the outward normal that thickness `t` spans.
    pub fn span(self, t: f64) -> (f64, f64) {
        match self {
            ThickenDirection::OneSide => (t.min(0.0), t.max(0.0)),
            ThickenDirection::Symmetric => (-t.abs() / 2.0, t.abs() / 2.0),
            ThickenDirection::BothSides => (-t.abs(), t.abs()),
        }
    }
}

/// Faces given a thickness: one solid per face, a new body or combined like an extrude.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thicken {
    pub faces: Vec<String>,
    /// An expression such as `3 mm`.
    pub thickness: String,
    #[serde(default)]
    pub direction: ThickenDirection,
    #[serde(default)]
    pub operation: Operation,
}

/// Bodies taken out of the result from this point on; the features that made them stay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveBodies {
    pub bodies: Vec<String>,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

fn check_names(what: &str, names: &[String], min: usize) -> Result<(), DocError> {
    if names.len() < min || names.len() > MAX_BODY_TOOL_ITEMS {
        return Err(bad(format!("pick {min} to {MAX_BODY_TOOL_ITEMS} {what}")));
    }
    if names
        .iter()
        .any(|n| n.trim().is_empty() || n.chars().count() > 512)
    {
        return Err(bad(format!("a name among the {what} is empty or too long")));
    }
    Ok(())
}

/// The feature ids a face name refers to (`2:top` names 2).
fn face_features(face: &str) -> impl Iterator<Item = u32> + '_ {
    face.split_whitespace()
        .filter_map(|w| w.split_once(':'))
        .filter_map(|(id, _)| id.parse::<u32>().ok())
}

/// The features a plane reference uses.
fn plane_features(p: &PlaneRef) -> Option<u32> {
    match p {
        PlaneRef::Origin { .. } => None,
        PlaneRef::Face(f) => Some(f.feature),
        PlaneRef::Construct { feature } => Some(*feature),
    }
}

impl FeatureKind {
    /// True for the body tools of this module.
    pub fn is_body_tool(&self) -> bool {
        matches!(
            self,
            FeatureKind::Combine(_)
                | FeatureKind::SplitBody(_)
                | FeatureKind::SplitFace(_)
                | FeatureKind::Thicken(_)
                | FeatureKind::Remove(_)
        )
    }

    /// The features a body tool uses (its plane's feature, the features of the faces it
    /// names), for the timeline's order rules and for refusing deletes. Bodies are named,
    /// so a body tool finds them when it is built, like a move.
    pub fn body_tool_features(&self) -> Vec<u32> {
        let mut out: Vec<u32> = match self {
            FeatureKind::SplitBody(s) => plane_features(&s.plane).into_iter().collect(),
            FeatureKind::SplitFace(s) => {
                let mut v: Vec<u32> = s.faces.iter().flat_map(|f| face_features(f)).collect();
                if let SplitFaceTool::Plane { plane } = &s.tool {
                    v.extend(plane_features(plane));
                }
                v
            }
            FeatureKind::Thicken(t) => t.faces.iter().flat_map(|f| face_features(f)).collect(),
            _ => Vec::new(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl Document {
    fn check_plane_ref(&self, p: &PlaneRef) -> Result<(), DocError> {
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
                FeatureKind::Construct(c) if c.kind() == crate::construct::ConstructKind::Plane => {
                    Ok(())
                }
                _ => Err(bad(format!(
                    "feature {feature} is not a construction plane"
                ))),
            },
        }
    }

    /// Checks a body tool against the design.
    pub(crate) fn check_body_tool(&self, kind: &FeatureKind) -> Result<(), DocError> {
        match kind {
            FeatureKind::Combine(c) => {
                check_names("target", std::slice::from_ref(&c.target), 1)?;
                check_names("tool bodies", &c.tools, 1)?;
                if c.operation == Operation::NewBody {
                    return Err(bad(
                        "combine joins, cuts or intersects: new_body is not one",
                    ));
                }
                if c.tools.contains(&c.target) {
                    return Err(bad("the target cannot also be a tool"));
                }
                Ok(())
            }
            FeatureKind::SplitBody(s) => {
                check_names("bodies", &s.bodies, 1)?;
                self.check_plane_ref(&s.plane)
            }
            FeatureKind::SplitFace(s) => {
                check_names("faces", &s.faces, 1)?;
                match &s.tool {
                    SplitFaceTool::Plane { plane } => self.check_plane_ref(plane),
                    SplitFaceTool::SketchLine { sketch, curve } => {
                        let f = self.sketch(*sketch)?;
                        match f.sketch.curve(chamfera_sketch::CurveId(*curve)) {
                            Some(c) if c.is_line() => Ok(()),
                            _ => Err(bad(format!("curve {curve} is not a line of {}", f.name))),
                        }
                    }
                }
            }
            FeatureKind::Thicken(t) => {
                check_names("faces", &t.faces, 1)?;
                let v = self.evaluate_feature_value(&t.thickness, expr::Dim::LENGTH)?;
                if !v.is_finite() || v.abs() <= chamfera_geom::LINEAR_TOLERANCE {
                    return Err(bad("the thickness must not be 0"));
                }
                if v.abs() > chamfera_geom::MAX_COORDINATE {
                    return Err(bad("the thickness leaves the working range"));
                }
                Ok(())
            }
            FeatureKind::Remove(r) => check_names("bodies", &r.bodies, 1),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construct::OriginPlane;

    #[test]
    fn body_tools_are_checked_and_survive_a_round_trip() {
        let mut doc = Document::default();
        let combine = FeatureKind::Combine(Combine {
            target: "Extrude1".into(),
            tools: vec!["Extrude2".into()],
            operation: Operation::Cut,
            keep_tools: true,
        });
        let split = FeatureKind::SplitBody(SplitBody {
            bodies: vec!["Extrude1".into()],
            plane: PlaneRef::Origin {
                plane: OriginPlane::Yz,
            },
        });
        let thicken = FeatureKind::Thicken(Thicken {
            faces: vec!["2:top".into()],
            thickness: "3 mm".into(),
            direction: ThickenDirection::Symmetric,
            operation: Operation::NewBody,
        });
        let remove = FeatureKind::Remove(RemoveBodies {
            bodies: vec!["Extrude1".into()],
        });
        for k in [combine, split, thicken, remove] {
            doc.add_feature(k).unwrap();
        }
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back.features, doc.features);
        // Refusals.
        let refused = [
            FeatureKind::Combine(Combine {
                target: "A".into(),
                tools: vec!["A".into()],
                operation: Operation::Join,
                keep_tools: false,
            }),
            FeatureKind::Combine(Combine {
                target: "A".into(),
                tools: vec!["B".into()],
                operation: Operation::NewBody,
                keep_tools: false,
            }),
            FeatureKind::Combine(Combine {
                target: "A".into(),
                tools: vec![],
                operation: Operation::Join,
                keep_tools: false,
            }),
            FeatureKind::SplitBody(SplitBody {
                bodies: vec![" ".into()],
                plane: PlaneRef::Construct { feature: 99 },
            }),
            FeatureKind::Thicken(Thicken {
                faces: vec!["2:top".into()],
                thickness: "0 mm".into(),
                direction: ThickenDirection::OneSide,
                operation: Operation::NewBody,
            }),
            FeatureKind::Remove(RemoveBodies { bodies: vec![] }),
        ];
        for k in refused {
            assert!(doc.check_feature(&k).is_err(), "{k:?}");
        }
    }

    #[test]
    fn thicken_spans_follow_the_direction() {
        assert_eq!(ThickenDirection::OneSide.span(3.0), (0.0, 3.0));
        assert_eq!(ThickenDirection::OneSide.span(-3.0), (-3.0, 0.0));
        assert_eq!(ThickenDirection::Symmetric.span(3.0), (-1.5, 1.5));
        assert_eq!(ThickenDirection::BothSides.span(3.0), (-3.0, 3.0));
    }
}

//! Direct face edits on the timeline: offset, delete and move faces of the solids built so
//! far. Faces are named like everywhere else (`2:top` is face `top` of feature 2's shape),
//! so an edit finds its faces again after every rebuild, and its distances are
//! expressions a parameter can drive.

use crate::{DocError, Document, expr};
use chamfera_geom::{Point3, Vec3};
use serde::{Deserialize, Serialize};

/// Most faces one face edit may hold.
pub const MAX_EDIT_FACES: usize = 10_000;

/// Faces moved along their outward normals by `distance` (positive grows the solid
/// there; a hole's wall offset by a positive distance gets narrower).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OffsetFace {
    pub faces: Vec<String>,
    /// An expression such as `5 mm` or `-t`.
    pub distance: String,
}

/// Faces removed, the gap closed by extending the faces around them (a fillet or
/// chamfer taken off, a hole filled).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteFace {
    pub faces: Vec<String>,
}

/// A turn of moved faces about an axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceTurn {
    pub origin: Point3,
    pub axis: Vec3,
    /// An expression such as `10 deg`.
    pub angle: String,
}

/// Planar faces moved by `distance` along `direction`, then turned by `turn`, their
/// neighbours following.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveFace {
    pub faces: Vec<String>,
    pub direction: Vec3,
    /// An expression such as `5 mm`; may be `0` when the faces only turn.
    pub distance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<FaceTurn>,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

fn check_faces(faces: &[String]) -> Result<(), DocError> {
    if faces.is_empty() || faces.len() > MAX_EDIT_FACES {
        return Err(bad(format!("pick 1 to {MAX_EDIT_FACES} faces")));
    }
    if faces
        .iter()
        .any(|f| f.is_empty() || f.chars().count() > 512)
    {
        return Err(bad("a face name is empty or too long"));
    }
    Ok(())
}

impl Document {
    pub(crate) fn check_offset_face(&self, o: &OffsetFace) -> Result<(), DocError> {
        check_faces(&o.faces)?;
        let d = self.evaluate_feature_value(&o.distance, expr::Dim::LENGTH)?;
        if !d.is_finite() || d.abs() <= chamfera_geom::LINEAR_TOLERANCE {
            return Err(bad("the distance must not be 0"));
        }
        Ok(())
    }

    pub(crate) fn check_delete_face(&self, d: &DeleteFace) -> Result<(), DocError> {
        check_faces(&d.faces)
    }

    pub(crate) fn check_move_face(&self, m: &MoveFace) -> Result<(), DocError> {
        check_faces(&m.faces)?;
        let d = self.evaluate_feature_value(&m.distance, expr::Dim::LENGTH)?;
        if !d.is_finite() || d.abs() > chamfera_geom::MAX_COORDINATE {
            return Err(bad("the distance must be a finite length"));
        }
        if d.abs() > chamfera_geom::LINEAR_TOLERANCE && m.direction.normalized().is_none() {
            return Err(bad("the direction has no length"));
        }
        let mut turns = false;
        if let Some(t) = &m.turn {
            if !t.origin.is_valid() || t.axis.normalized().is_none() {
                return Err(bad("the turn needs a point and an axis direction"));
            }
            let a = self.evaluate_feature_value(&t.angle, expr::Dim::ANGLE)?;
            if !a.is_finite() || a.abs() > std::f64::consts::PI {
                return Err(bad("the angle must be at most 180 degrees either way"));
            }
            turns = a.abs() > chamfera_geom::ANGULAR_TOLERANCE;
        }
        if d.abs() <= chamfera_geom::LINEAR_TOLERANCE && !turns {
            return Err(bad("move the faces by a distance or an angle"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeatureKind;

    #[test]
    fn face_edits_check_their_faces_and_values_and_round_trip() {
        let mut doc = Document::default();
        let ok = FeatureKind::OffsetFace(OffsetFace {
            faces: vec!["900:top".into()],
            distance: "5 mm".into(),
        });
        let id = doc.add_feature(ok).unwrap();
        assert_eq!(doc.feature(id).unwrap().name, "Offset Face1");
        for (faces, distance) in [
            (vec![], "5"),
            (vec!["900:top".to_owned()], "0"),
            (vec!["".to_owned()], "1"),
        ] {
            let k = FeatureKind::OffsetFace(OffsetFace {
                faces,
                distance: distance.into(),
            });
            assert!(doc.add_feature(k).is_err());
        }
        doc.add_feature(FeatureKind::DeleteFace(DeleteFace {
            faces: vec!["900:fillet of 899:sweep 0".into()],
        }))
        .unwrap();
        assert!(
            doc.add_feature(FeatureKind::DeleteFace(DeleteFace { faces: vec![] }))
                .is_err()
        );
        let turn = FeatureKind::MoveFace(MoveFace {
            faces: vec!["900:top".into()],
            direction: Vec3::new(0.0, 0.0, 1.0),
            distance: "0".into(),
            turn: Some(FaceTurn {
                origin: Point3::new(0.0, 15.0, 10.0),
                axis: Vec3::new(1.0, 0.0, 0.0),
                angle: "10 deg".into(),
            }),
        });
        doc.add_feature(turn).unwrap();
        let still = FeatureKind::MoveFace(MoveFace {
            faces: vec!["900:top".into()],
            direction: Vec3::new(0.0, 0.0, 0.0),
            distance: "0".into(),
            turn: None,
        });
        assert!(doc.add_feature(still).is_err());
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
    }
}

//! Construction geometry: planes, axes and points on the timeline that sketches and
//! features can use.
//!
//! A construction refers to what it is built from by name, never by position: origin
//! planes and axes by their name, faces of solids by [`FaceRef`] (the feature and the
//! face's role name), edges by the two named faces they join, vertices by the named faces
//! that meet there, and earlier constructions by their feature id. So a plane offset from a
//! face follows that face when the design rebuilds.

use crate::{DocError, Document, FaceRef, FeatureKind, expr};
use serde::{Deserialize, Serialize};

/// Most faces a vertex reference may name.
pub const MAX_VERTEX_FACES: usize = 8;
/// Longest face role name a reference may hold.
pub const MAX_FACE_NAME: usize = 256;

/// One of the three origin planes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginPlane {
    Xy,
    Xz,
    Yz,
}

impl OriginPlane {
    pub fn parse(s: &str) -> Option<OriginPlane> {
        Some(match s {
            "xy" => OriginPlane::Xy,
            "xz" => OriginPlane::Xz,
            "yz" => OriginPlane::Yz,
            _ => return None,
        })
    }

    pub fn frame(self) -> chamfera_geom::Frame {
        match self {
            OriginPlane::Xy => chamfera_geom::Frame::XY,
            OriginPlane::Xz => chamfera_geom::Frame::XZ,
            OriginPlane::Yz => chamfera_geom::Frame::YZ,
        }
    }
}

/// One of the three origin axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginAxis {
    X,
    Y,
    Z,
}

impl OriginAxis {
    pub fn parse(s: &str) -> Option<OriginAxis> {
        Some(match s {
            "x" => OriginAxis::X,
            "y" => OriginAxis::Y,
            "z" => OriginAxis::Z,
            _ => return None,
        })
    }

    pub fn direction(self) -> chamfera_geom::Vec3 {
        match self {
            OriginAxis::X => chamfera_geom::Vec3::X,
            OriginAxis::Y => chamfera_geom::Vec3::Y,
            OriginAxis::Z => chamfera_geom::Vec3::Z,
        }
    }
}

/// An edge of a solid: the edge where two named faces meet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub faces: [FaceRef; 2],
}

/// A plane a construction starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaneRef {
    Origin {
        plane: OriginPlane,
    },
    /// A flat face of a solid; its normal points out of the solid.
    Face(FaceRef),
    /// An earlier construction plane.
    Construct {
        feature: u32,
    },
}

/// A line a construction starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AxisRef {
    Origin {
        axis: OriginAxis,
    },
    /// A straight edge of a solid.
    Edge(EdgeRef),
    /// An earlier construction axis.
    Construct {
        feature: u32,
    },
}

/// A point a construction starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PointRef {
    /// The origin (0, 0, 0).
    Origin,
    /// The vertex where all these faces meet.
    Vertex { faces: Vec<FaceRef> },
    /// The centre of a circular edge (a circle or an arc).
    Centre { edge: EdgeRef },
    /// Coordinates in mm, each an expression such as `10` or `width / 2`.
    Coordinates { x: String, y: String, z: String },
    /// An earlier construction point.
    Construct { feature: u32 },
}

/// What a construction feature builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "construct", rename_all = "snake_case")]
pub enum Construction {
    /// Parallel to `base`, `distance` along its normal (negative: the other way).
    PlaneOffset { base: PlaneRef, distance: String },
    /// Through `axis`, turned by `angle` about it from `base` (or, without one, from the
    /// origin plane that holds the axis: XY, else XZ, else YZ, turned to contain it).
    PlaneAngle {
        axis: AxisRef,
        angle: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<PlaneRef>,
    },
    /// Halfway between two parallel planes, facing like the first.
    PlaneMidplane { planes: [PlaneRef; 2] },
    /// Through three points that are not on one line; its normal is
    /// `(p1 - p0) × (p2 - p0)`.
    PlaneThreePoints { points: [PointRef; 3] },
    /// Touching a cylindrical face along the line nearest to `point`.
    PlaneTangent { face: FaceRef, point: PointRef },
    /// The axis of a cylindrical or conical face.
    AxisCylinder { face: FaceRef },
    /// Along a straight edge.
    AxisEdge { edge: EdgeRef },
    /// Through two different points, from the first to the second.
    AxisTwoPoints { points: [PointRef; 2] },
    /// Where two planes that are not parallel cross.
    AxisTwoPlanes { planes: [PlaneRef; 2] },
    /// A point.
    Point { at: PointRef },
}

/// The three kinds of construction geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructKind {
    Plane,
    Axis,
    Point,
}

impl ConstructKind {
    pub fn label(self) -> &'static str {
        match self {
            ConstructKind::Plane => "Plane",
            ConstructKind::Axis => "Axis",
            ConstructKind::Point => "Point",
        }
    }
}

impl PlaneRef {
    fn visit<'a>(&'a self, out: &mut Refs<'a>) {
        match self {
            PlaneRef::Origin { .. } => {}
            PlaneRef::Face(f) => out.faces.push(f),
            PlaneRef::Construct { feature } => {
                out.constructs.push((*feature, ConstructKind::Plane))
            }
        }
    }
}

impl AxisRef {
    fn visit<'a>(&'a self, out: &mut Refs<'a>) {
        match self {
            AxisRef::Origin { .. } => {}
            AxisRef::Edge(e) => out.faces.extend(e.faces.iter()),
            AxisRef::Construct { feature } => out.constructs.push((*feature, ConstructKind::Axis)),
        }
    }
}

impl PointRef {
    fn visit<'a>(&'a self, out: &mut Refs<'a>) {
        match self {
            PointRef::Origin => {}
            PointRef::Vertex { faces } => {
                out.faces.extend(faces.iter());
                out.vertex_lists.push(faces.len());
            }
            PointRef::Centre { edge } => out.faces.extend(edge.faces.iter()),
            PointRef::Coordinates { x, y, z } => {
                out.lengths.extend([x.as_str(), y.as_str(), z.as_str()]);
            }
            PointRef::Construct { feature } => {
                out.constructs.push((*feature, ConstructKind::Point))
            }
        }
    }
}

/// Everything a construction refers to, gathered for checks.
#[derive(Default)]
struct Refs<'a> {
    faces: Vec<&'a FaceRef>,
    constructs: Vec<(u32, ConstructKind)>,
    lengths: Vec<&'a str>,
    angles: Vec<&'a str>,
    vertex_lists: Vec<usize>,
}

impl Construction {
    pub fn kind(&self) -> ConstructKind {
        match self {
            Construction::PlaneOffset { .. }
            | Construction::PlaneAngle { .. }
            | Construction::PlaneMidplane { .. }
            | Construction::PlaneThreePoints { .. }
            | Construction::PlaneTangent { .. } => ConstructKind::Plane,
            Construction::AxisCylinder { .. }
            | Construction::AxisEdge { .. }
            | Construction::AxisTwoPoints { .. }
            | Construction::AxisTwoPlanes { .. } => ConstructKind::Axis,
            Construction::Point { .. } => ConstructKind::Point,
        }
    }

    /// A short description, such as `Offset 15 mm from XY`.
    pub fn describe(&self) -> String {
        fn plane(r: &PlaneRef) -> String {
            match r {
                PlaneRef::Origin { plane } => match plane {
                    OriginPlane::Xy => "XY".into(),
                    OriginPlane::Xz => "XZ".into(),
                    OriginPlane::Yz => "YZ".into(),
                },
                PlaneRef::Face(f) => format!("face {} of feature {}", f.face, f.feature),
                PlaneRef::Construct { feature } => format!("plane {feature}"),
            }
        }
        match self {
            Construction::PlaneOffset { base, distance } => {
                format!("Offset {distance} from {}", plane(base))
            }
            Construction::PlaneAngle { angle, .. } => format!("At {angle} about an axis"),
            Construction::PlaneMidplane { planes } => {
                format!(
                    "Midplane of {} and {}",
                    plane(&planes[0]),
                    plane(&planes[1])
                )
            }
            Construction::PlaneThreePoints { .. } => "Through three points".into(),
            Construction::PlaneTangent { face, .. } => {
                format!("Tangent to face {} of feature {}", face.face, face.feature)
            }
            Construction::AxisCylinder { face } => {
                format!("Axis of face {} of feature {}", face.face, face.feature)
            }
            Construction::AxisEdge { .. } => "Along an edge".into(),
            Construction::AxisTwoPoints { .. } => "Through two points".into(),
            Construction::AxisTwoPlanes { planes } => {
                format!("Where {} meets {}", plane(&planes[0]), plane(&planes[1]))
            }
            Construction::Point { at } => match at {
                PointRef::Origin => "At the origin".into(),
                PointRef::Vertex { .. } => "At a vertex".into(),
                PointRef::Centre { .. } => "At the centre of an arc".into(),
                PointRef::Coordinates { x, y, z } => format!("At ({x}, {y}, {z})"),
                PointRef::Construct { feature } => format!("At point {feature}"),
            },
        }
    }

    fn refs(&self) -> Refs<'_> {
        let mut r = Refs::default();
        match self {
            Construction::PlaneOffset { base, distance } => {
                base.visit(&mut r);
                r.lengths.push(distance);
            }
            Construction::PlaneAngle { axis, angle, base } => {
                axis.visit(&mut r);
                if let Some(b) = base {
                    b.visit(&mut r);
                }
                r.angles.push(angle);
            }
            Construction::PlaneMidplane { planes } | Construction::AxisTwoPlanes { planes } => {
                planes.iter().for_each(|p| p.visit(&mut r));
            }
            Construction::PlaneThreePoints { points } => {
                points.iter().for_each(|p| p.visit(&mut r));
            }
            Construction::AxisTwoPoints { points } => points.iter().for_each(|p| p.visit(&mut r)),
            Construction::PlaneTangent { face, point } => {
                r.faces.push(face);
                point.visit(&mut r);
            }
            Construction::AxisCylinder { face } => r.faces.push(face),
            Construction::AxisEdge { edge } => r.faces.extend(edge.faces.iter()),
            Construction::Point { at } => at.visit(&mut r),
        }
        r
    }

    /// The expressions this construction evaluates.
    pub fn expressions(&self) -> Vec<&str> {
        let r = self.refs();
        r.lengths.into_iter().chain(r.angles).collect()
    }

    /// Every feature this construction refers to: features whose faces it uses and
    /// earlier constructions.
    pub fn features_used(&self) -> Vec<u32> {
        let r = self.refs();
        let mut ids: Vec<u32> = r
            .faces
            .iter()
            .map(|f| f.feature)
            .chain(r.constructs.iter().map(|c| c.0))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The checks that need nothing but the construction itself (a design file is checked
    /// with these before anything is built).
    pub fn check_shape(&self) -> Result<(), String> {
        let r = self.refs();
        for f in &r.faces {
            if f.face.is_empty() || f.face.chars().count() > MAX_FACE_NAME {
                return Err(format!(
                    "a face name must have 1 to {MAX_FACE_NAME} characters"
                ));
            }
        }
        for n in &r.vertex_lists {
            if !(2..=MAX_VERTEX_FACES).contains(n) {
                return Err(format!(
                    "a vertex is named by 2 to {MAX_VERTEX_FACES} of the faces that meet there"
                ));
            }
        }
        for e in r.lengths.iter().chain(r.angles.iter()) {
            if e.chars().count() > expr::MAX_EXPRESSION_LEN {
                return Err("an expression is too long".into());
            }
        }
        Ok(())
    }
}

impl Document {
    /// Checks a construction against the design: what it refers to exists (faces of
    /// earlier solid features, earlier constructions of the right kind) and its expressions
    /// evaluate with the right unit.
    pub fn check_construction(&self, c: &Construction) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        c.check_shape().map_err(bad)?;
        let r = c.refs();
        for f in &r.faces {
            let used = self.feature(f.feature)?;
            if matches!(used.kind, FeatureKind::Construct(_)) {
                return Err(bad(format!(
                    "feature {} is construction geometry, which has no faces",
                    f.feature
                )));
            }
        }
        for (id, want) in &r.constructs {
            let used = self.feature(*id)?;
            match &used.kind {
                FeatureKind::Construct(k) if k.kind() == *want => {}
                _ => {
                    return Err(bad(format!(
                        "feature {id} is not a construction {}",
                        want.label().to_lowercase()
                    )));
                }
            }
        }
        for e in &r.lengths {
            let v = self.evaluate_feature_value(e, expr::Dim::LENGTH)?;
            if !v.is_finite() || v.abs() > chamfera_geom::MAX_COORDINATE {
                return Err(bad(format!(
                    "a length must be at most {} mm",
                    chamfera_geom::MAX_COORDINATE
                )));
            }
        }
        for e in &r.angles {
            let v = self.evaluate_feature_value(e, expr::Dim::ANGLE)?;
            if !v.is_finite() || v.abs() > std::f64::consts::TAU + chamfera_geom::ANGULAR_TOLERANCE
            {
                return Err(bad("the angle must be from -360 to 360 degrees".into()));
            }
        }
        Ok(())
    }

    /// The construction planes, axes or points of the timeline: (id, name).
    pub fn constructions(&self, kind: ConstructKind) -> Vec<(u32, String)> {
        self.features
            .iter()
            .filter(|f| matches!(&f.kind, FeatureKind::Construct(c) if c.kind() == kind))
            .map(|f| (f.id, f.name.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Extrude, ExtrudeDirection, Operation, Unit};
    use chamfera_geom::Point2;

    fn plate() -> (Document, u32) {
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
        let f = doc
            .add_feature(FeatureKind::Extrude(Extrude {
                sketch: id,
                profiles: vec![Point2::new(5.0, 5.0)],
                distance: "10".into(),
                direction: ExtrudeDirection::Normal,
                operation: Operation::NewBody,
                ..Extrude::default()
            }))
            .unwrap();
        (doc, f)
    }

    #[test]
    fn constructions_are_named_by_kind_and_survive_a_round_trip() {
        let (mut doc, plate) = plate();
        doc.set_parameter_expression("gap", "15 mm", Unit::Length)
            .unwrap();
        let p = doc
            .add_feature(FeatureKind::Construct(Construction::PlaneOffset {
                base: PlaneRef::Face(FaceRef {
                    feature: plate,
                    face: "top".into(),
                }),
                distance: "gap".into(),
            }))
            .unwrap();
        let a = doc
            .add_feature(FeatureKind::Construct(Construction::AxisTwoPlanes {
                planes: [
                    PlaneRef::Construct { feature: p },
                    PlaneRef::Origin {
                        plane: OriginPlane::Xz,
                    },
                ],
            }))
            .unwrap();
        let pt = doc
            .add_feature(FeatureKind::Construct(Construction::Point {
                at: PointRef::Coordinates {
                    x: "1".into(),
                    y: "gap * 2".into(),
                    z: "-3 mm".into(),
                },
            }))
            .unwrap();
        assert_eq!(doc.feature(p).unwrap().name, "Plane1");
        assert_eq!(doc.feature(a).unwrap().name, "Axis1");
        assert_eq!(doc.feature(pt).unwrap().name, "Point1");
        assert_eq!(
            doc.constructions(ConstructKind::Plane),
            vec![(p, "Plane1".into())]
        );
        // The parameter and the plate are in use now.
        assert!(doc.remove_parameter("gap").is_err());
        let users = doc.feature_users(plate);
        assert!(users.contains(&"Plane1".to_owned()), "{users:?}");
        assert_eq!(doc.feature_users(p), vec!["Axis1".to_owned()]);
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn references_must_exist_and_be_of_the_right_kind() {
        let (mut doc, plate) = plate();
        let p = doc
            .add_feature(FeatureKind::Construct(Construction::PlaneOffset {
                base: PlaneRef::Origin {
                    plane: OriginPlane::Xy,
                },
                distance: "5".into(),
            }))
            .unwrap();
        let before = doc.clone();
        let bad = [
            // An axis that is a plane.
            Construction::PlaneAngle {
                axis: AxisRef::Construct { feature: p },
                angle: "30 deg".into(),
                base: None,
            },
            // A face of a construction.
            Construction::AxisCylinder {
                face: FaceRef {
                    feature: p,
                    face: "top".into(),
                },
            },
            // A face of a feature that does not exist.
            Construction::AxisCylinder {
                face: FaceRef {
                    feature: 999,
                    face: "side 0".into(),
                },
            },
            // A distance in degrees, an angle of two turns, a vertex of one face.
            Construction::PlaneOffset {
                base: PlaneRef::Construct { feature: p },
                distance: "5 deg".into(),
            },
            Construction::PlaneAngle {
                axis: AxisRef::Origin {
                    axis: OriginAxis::X,
                },
                angle: "720 deg".into(),
                base: None,
            },
            Construction::Point {
                at: PointRef::Vertex {
                    faces: vec![FaceRef {
                        feature: plate,
                        face: "top".into(),
                    }],
                },
            },
            Construction::Point {
                at: PointRef::Coordinates {
                    x: "1e7".into(),
                    y: "0".into(),
                    z: "0".into(),
                },
            },
        ];
        for c in bad {
            assert!(
                doc.add_feature(FeatureKind::Construct(c.clone())).is_err(),
                "{c:?}"
            );
        }
        assert_eq!(doc, before);
    }
}

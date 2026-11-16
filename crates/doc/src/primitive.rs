//! Primitives and scale on the timeline (format 43).
//!
//! A primitive is a box, cylinder, sphere, torus or cone made in one step at a point,
//! with its axis along x, y or z, as a new body or joined, cut or intersected like an
//! extrude. Its sizes are expressions, so they are model parameters (`Box1.length`).
//!
//! A scale stretches bodies built so far about a point, by one factor or by a factor along
//! each world axis. Bodies are named as everywhere else and found by name when built.

use crate::{DocError, Document, Operation, expr};
use chamfera_geom::{LINEAR_TOLERANCE, MAX_COORDINATE, Point3};
use serde::{Deserialize, Serialize};

/// Which world axis a primitive's height (or axis of turning) runs along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveAxis {
    X,
    Y,
    #[default]
    Z,
}

impl PrimitiveAxis {
    pub fn parse(s: &str) -> Option<PrimitiveAxis> {
        Some(match s {
            "x" => PrimitiveAxis::X,
            "y" => PrimitiveAxis::Y,
            "z" => PrimitiveAxis::Z,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            PrimitiveAxis::X => "x",
            PrimitiveAxis::Y => "y",
            PrimitiveAxis::Z => "z",
        }
    }
}

/// The shape and its sizes, each an expression such as `20 mm`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum PrimitiveShape {
    /// Centred on the point across its base, `length` along the first axis square to the
    /// primitive's axis, `width` along the second, `height` along the axis.
    Box {
        length: String,
        width: String,
        height: String,
    },
    /// Its base circle centred on the point.
    Cylinder { radius: String, height: String },
    /// Centred on the point.
    Sphere { radius: String },
    /// Centred on the point, turned about the axis: the tube's centre circle has
    /// `major_radius`, the tube `minor_radius`.
    Torus {
        major_radius: String,
        minor_radius: String,
    },
    /// Its base circle (`radius`) centred on the point, its top (`top_radius`, 0 for a
    /// pointed cone) `height` along the axis.
    Cone {
        radius: String,
        #[serde(default = "zero")]
        top_radius: String,
        height: String,
    },
}

fn zero() -> String {
    "0 mm".into()
}

impl PrimitiveShape {
    /// The shape's name, which also names its features (`Box1`).
    pub fn label(&self) -> &'static str {
        match self {
            PrimitiveShape::Box { .. } => "Box",
            PrimitiveShape::Cylinder { .. } => "Cylinder",
            PrimitiveShape::Sphere { .. } => "Sphere",
            PrimitiveShape::Torus { .. } => "Torus",
            PrimitiveShape::Cone { .. } => "Cone",
        }
    }

    /// Every size, with its field name, for model parameters and unit changes.
    pub fn sizes_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            PrimitiveShape::Box {
                length,
                width,
                height,
            } => vec![("length", length), ("width", width), ("height", height)],
            PrimitiveShape::Cylinder { radius, height } => {
                vec![("radius", radius), ("height", height)]
            }
            PrimitiveShape::Sphere { radius } => vec![("radius", radius)],
            PrimitiveShape::Torus {
                major_radius,
                minor_radius,
            } => vec![
                ("major_radius", major_radius),
                ("minor_radius", minor_radius),
            ],
            PrimitiveShape::Cone {
                radius,
                top_radius,
                height,
            } => vec![
                ("radius", radius),
                ("top_radius", top_radius),
                ("height", height),
            ],
        }
    }
}

/// A box, cylinder, sphere, torus or cone at a point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Primitive {
    #[serde(flatten)]
    pub shape: PrimitiveShape,
    /// Where it sits (see each shape), in mm.
    pub origin: Point3,
    #[serde(default)]
    pub axis: PrimitiveAxis,
    #[serde(default)]
    pub operation: Operation,
}

/// A primitive's sizes evaluated, in mm, in the order of [`PrimitiveShape::sizes_mut`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrimitiveValues {
    Box {
        length: f64,
        width: f64,
        height: f64,
    },
    Cylinder {
        radius: f64,
        height: f64,
    },
    Sphere {
        radius: f64,
    },
    Torus {
        major: f64,
        minor: f64,
    },
    Cone {
        radius: f64,
        top: f64,
        height: f64,
    },
}

impl PrimitiveValues {
    /// The exact volume in mm³ (box `l w h`; cylinder `π r² h`; sphere `4/3 π r³`; torus
    /// `2 π² R r²` by Pappus; cone frustum `π h / 3 (r₁² + r₁ r₂ + r₂²)`).
    pub fn volume(&self) -> f64 {
        use std::f64::consts::PI;
        match *self {
            PrimitiveValues::Box {
                length,
                width,
                height,
            } => length * width * height,
            PrimitiveValues::Cylinder { radius, height } => PI * radius * radius * height,
            PrimitiveValues::Sphere { radius } => 4.0 / 3.0 * PI * radius.powi(3),
            PrimitiveValues::Torus { major, minor } => 2.0 * PI * PI * major * minor * minor,
            PrimitiveValues::Cone {
                radius,
                top,
                height,
            } => PI * height / 3.0 * (radius * radius + radius * top + top * top),
        }
    }
}

/// Bodies scaled about a point by a factor along x, y and z (equal for a uniform scale).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub bodies: Vec<String>,
    /// The point that stays put, in mm.
    pub origin: Point3,
    /// The factors along x, y and z, plain-number expressions such as `2` or `k * 1.5`.
    pub x: String,
    pub y: String,
    pub z: String,
}

/// Most bodies one scale may name.
pub const MAX_SCALE_BODIES: usize = 1_000;

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

impl Document {
    /// The primitive's sizes, checked: each above 0 (a cone's top may be 0) and within
    /// the working range; a torus's tube thinner than its centre circle; a cone's two
    /// radii different (equal radii make a cylinder).
    pub fn primitive_values(&self, p: &Primitive) -> Result<PrimitiveValues, DocError> {
        if !p.origin.is_valid() {
            return Err(bad("the position must be a point within the working range"));
        }
        let len = |name: &str, e: &String, may_be_zero: bool| -> Result<f64, DocError> {
            let v = self.evaluate_feature_value(e, expr::Dim::LENGTH)?;
            let low_ok = if may_be_zero {
                v >= 0.0
            } else {
                v > LINEAR_TOLERANCE
            };
            if !(v.is_finite() && low_ok && v <= MAX_COORDINATE) {
                return Err(bad(format!(
                    "the {name} must be {} and at most {MAX_COORDINATE} mm",
                    if may_be_zero { "0 or more" } else { "above 0" }
                )));
            }
            Ok(v)
        };
        Ok(match &p.shape {
            PrimitiveShape::Box {
                length,
                width,
                height,
            } => PrimitiveValues::Box {
                length: len("length", length, false)?,
                width: len("width", width, false)?,
                height: len("height", height, false)?,
            },
            PrimitiveShape::Cylinder { radius, height } => PrimitiveValues::Cylinder {
                radius: len("radius", radius, false)?,
                height: len("height", height, false)?,
            },
            PrimitiveShape::Sphere { radius } => PrimitiveValues::Sphere {
                radius: len("radius", radius, false)?,
            },
            PrimitiveShape::Torus {
                major_radius,
                minor_radius,
            } => {
                let major = len("major radius", major_radius, false)?;
                let minor = len("minor radius", minor_radius, false)?;
                if minor >= major - LINEAR_TOLERANCE {
                    return Err(bad(
                        "the minor radius must be smaller than the major radius",
                    ));
                }
                PrimitiveValues::Torus { major, minor }
            }
            PrimitiveShape::Cone {
                radius,
                top_radius,
                height,
            } => {
                let radius = len("radius", radius, false)?;
                let top = len("top radius", top_radius, true)?;
                if (radius - top).abs() <= LINEAR_TOLERANCE {
                    return Err(bad(
                        "a cone's radii must differ (equal radii make a cylinder)",
                    ));
                }
                PrimitiveValues::Cone {
                    radius,
                    top,
                    height: len("height", height, false)?,
                }
            }
        })
    }

    /// The scale's factors along x, y and z, each a plain number within the kernel's
    /// range.
    pub fn scale_factors(&self, s: &Scale) -> Result<[f64; 3], DocError> {
        let mut out = [1.0; 3];
        for (slot, (name, e)) in out.iter_mut().zip([("x", &s.x), ("y", &s.y), ("z", &s.z)]) {
            let v = self.evaluate_feature_value(e, expr::Dim::NONE)?;
            if !(v.is_finite()
                && (chamfera_kernel::MIN_SCALE_FACTOR..=chamfera_kernel::MAX_SCALE_FACTOR)
                    .contains(&v))
            {
                return Err(bad(format!(
                    "the {name} factor must be between {} and {}",
                    chamfera_kernel::MIN_SCALE_FACTOR,
                    chamfera_kernel::MAX_SCALE_FACTOR
                )));
            }
            *slot = v;
        }
        Ok(out)
    }

    pub(crate) fn check_primitive(&self, p: &Primitive) -> Result<(), DocError> {
        self.primitive_values(p).map(|_| ())
    }

    pub(crate) fn check_scale(&self, s: &Scale) -> Result<(), DocError> {
        if s.bodies.is_empty() || s.bodies.len() > MAX_SCALE_BODIES {
            return Err(bad(format!("pick 1 to {MAX_SCALE_BODIES} bodies to scale")));
        }
        if s.bodies
            .iter()
            .any(|b| b.trim().is_empty() || b.chars().count() > 256)
        {
            return Err(bad("a body name is empty or longer than 256 characters"));
        }
        if !s.origin.is_valid() {
            return Err(bad("the scale's point must be within the working range"));
        }
        self.scale_factors(s).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeatureKind;

    fn sphere(r: &str) -> Primitive {
        Primitive {
            shape: PrimitiveShape::Sphere { radius: r.into() },
            origin: Point3::ORIGIN,
            axis: PrimitiveAxis::Z,
            operation: Operation::NewBody,
        }
    }

    #[test]
    fn primitive_volumes_follow_their_formulas() {
        use std::f64::consts::PI;
        // Sphere r = 10: 4/3 pi 1000.
        let v = PrimitiveValues::Sphere { radius: 10.0 }.volume();
        assert!((v - 4000.0 / 3.0 * PI).abs() < 1e-9);
        // Torus R = 20, r = 5: 2 pi² 20 25 = 1000 pi².
        let v = PrimitiveValues::Torus {
            major: 20.0,
            minor: 5.0,
        }
        .volume();
        assert!((v - 1000.0 * PI * PI).abs() < 1e-9);
        // Cone r = 3, h = 9, pointed: pi 9 9 / 3 = 27 pi.
        let v = PrimitiveValues::Cone {
            radius: 3.0,
            top: 0.0,
            height: 9.0,
        }
        .volume();
        assert!((v - 27.0 * PI).abs() < 1e-9);
    }

    #[test]
    fn primitives_and_scales_are_checked_and_survive_a_round_trip() {
        let mut doc = Document::default();
        let id = doc
            .add_feature(FeatureKind::Primitive(sphere("10 mm")))
            .unwrap();
        assert_eq!(doc.feature(id).unwrap().name, "Sphere1");
        let scale = FeatureKind::Scale(Scale {
            bodies: vec!["Sphere1".into()],
            origin: Point3::ORIGIN,
            x: "2".into(),
            y: "2".into(),
            z: "2".into(),
        });
        let sid = doc.add_feature(scale).unwrap();
        assert_eq!(doc.feature(sid).unwrap().name, "Scale1");
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back.features, doc.features);
        let refused = [
            FeatureKind::Primitive(sphere("0 mm")),
            FeatureKind::Primitive(sphere("-1")),
            FeatureKind::Primitive(sphere("2 deg")),
            FeatureKind::Primitive(Primitive {
                shape: PrimitiveShape::Torus {
                    major_radius: "5 mm".into(),
                    minor_radius: "5 mm".into(),
                },
                ..sphere("1")
            }),
            FeatureKind::Primitive(Primitive {
                shape: PrimitiveShape::Cone {
                    radius: "5 mm".into(),
                    top_radius: "5 mm".into(),
                    height: "5 mm".into(),
                },
                ..sphere("1")
            }),
            FeatureKind::Scale(Scale {
                bodies: vec![],
                origin: Point3::ORIGIN,
                x: "1".into(),
                y: "1".into(),
                z: "1".into(),
            }),
            FeatureKind::Scale(Scale {
                bodies: vec!["A".into()],
                origin: Point3::ORIGIN,
                x: "0".into(),
                y: "1".into(),
                z: "1".into(),
            }),
            FeatureKind::Scale(Scale {
                bodies: vec!["A".into()],
                origin: Point3::ORIGIN,
                x: "2 mm".into(),
                y: "1".into(),
                z: "1".into(),
            }),
        ];
        for k in refused {
            assert!(doc.check_feature(&k).is_err(), "{k:?}");
        }
    }
}

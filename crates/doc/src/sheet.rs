//! Sheet metal in the design: named rules (thickness, bend radius, K-factor, relief) and
//! the sheet features (base flange, flange, flat pattern) on the timeline.

use crate::{DocError, Document, expr};
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// Most sheet metal rules one design may hold.
pub const MAX_SHEET_RULES: usize = 1_000;
/// Longest rule name.
pub const MAX_RULE_NAME_LEN: usize = 64;
/// Thickest sheet, mm.
pub const MAX_SHEET_THICKNESS: f64 = 1_000.0;

/// The cut that frees a bend where a flange stops short of its edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReliefShape {
    #[default]
    Rectangular,
    /// A notch with a round end.
    Obround,
    /// A slit, no material removed.
    Rip,
}

impl ReliefShape {
    pub fn parse(s: &str) -> Option<ReliefShape> {
        Some(match s {
            "rectangular" => ReliefShape::Rectangular,
            "obround" => ReliefShape::Obround,
            "rip" => ReliefShape::Rip,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            ReliefShape::Rectangular => "rectangular",
            ReliefShape::Obround => "obround",
            ReliefShape::Rip => "rip",
        }
    }
}

/// A named set of sheet metal values, in mm (a base flange uses one by name; its flanges
/// follow it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetRule {
    pub name: String,
    pub thickness: f64,
    /// The inner bend radius.
    pub bend_radius: f64,
    /// Where the neutral layer lies across the thickness, 0 (inside) to 1 (outside).
    pub k_factor: f64,
    #[serde(default)]
    pub relief: ReliefShape,
    /// The relief's width along the edge.
    pub relief_width: f64,
    /// How far the relief reaches past the bend's start.
    pub relief_depth: f64,
}

impl SheetRule {
    /// The rule a design starts with: 2 mm sheet, 2 mm inner radius, K = 0.44, a
    /// rectangular relief 2 mm wide and 2 mm deep.
    pub fn default_named(name: &str) -> SheetRule {
        SheetRule {
            name: name.to_owned(),
            thickness: 2.0,
            bend_radius: 2.0,
            k_factor: 0.44,
            relief: ReliefShape::Rectangular,
            relief_width: 2.0,
            relief_depth: 2.0,
        }
    }

    /// Checks every value; the message says what is allowed.
    pub fn check(&self) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        let n = self.name.trim();
        if n.is_empty() || self.name.chars().count() > MAX_RULE_NAME_LEN || n != self.name {
            return Err(bad(format!(
                "a rule name must be 1 to {MAX_RULE_NAME_LEN} characters without spaces at the ends"
            )));
        }
        let tol = chamfera_geom::LINEAR_TOLERANCE;
        let length = |v: f64, what: &str, zero_ok: bool| -> Result<(), DocError> {
            let ok = v.is_finite()
                && v <= MAX_SHEET_THICKNESS * 100.0
                && (v > tol || zero_ok && v >= 0.0);
            if ok {
                Ok(())
            } else {
                Err(bad(format!(
                    "the {what} must be {} and at most {} mm",
                    if zero_ok { "0 or more" } else { "above 0" },
                    MAX_SHEET_THICKNESS * 100.0
                )))
            }
        };
        if !(self.thickness.is_finite()
            && self.thickness > tol
            && self.thickness <= MAX_SHEET_THICKNESS)
        {
            return Err(bad(format!(
                "the thickness must be above 0 and at most {MAX_SHEET_THICKNESS} mm"
            )));
        }
        length(self.bend_radius, "bend radius", false)?;
        if !(self.k_factor.is_finite() && (0.0..=1.0).contains(&self.k_factor)) {
            return Err(bad("the K-factor must be from 0 to 1".into()));
        }
        length(self.relief_width, "relief width", false)?;
        length(self.relief_depth, "relief depth", true)?;
        Ok(())
    }
}

/// A base flange: one closed sketch profile extruded by its rule's thickness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetBase {
    pub sketch: u32,
    /// A point inside the profile (sketch coordinates); one profile per base.
    pub profiles: Vec<Point2>,
    /// The sheet metal rule, by name.
    pub rule: String,
    /// Against the sketch plane's normal instead of along it.
    #[serde(default)]
    pub reversed: bool,
}

/// Where a flange sits relative to its edge's side face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FlangePosition {
    /// The outer faces meet on the edge: the part keeps its outline.
    #[default]
    Inside,
    /// The inner faces meet on the edge.
    Outside,
    /// The bend starts at the edge.
    BendOutside,
}

impl FlangePosition {
    pub fn parse(s: &str) -> Option<FlangePosition> {
        Some(match s {
            "inside" => FlangePosition::Inside,
            "outside" => FlangePosition::Outside,
            "bend_outside" => FlangePosition::BendOutside,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            FlangePosition::Inside => "inside",
            FlangePosition::Outside => "outside",
            FlangePosition::BendOutside => "bend_outside",
        }
    }
}

/// Which way a flange rises from its edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FlangeSide {
    /// Toward the face the edge borders (a top edge bends up).
    #[default]
    Same,
    /// The other way.
    Opposite,
}

impl FlangeSide {
    pub fn parse(s: &str) -> Option<FlangeSide> {
        Some(match s {
            "same" => FlangeSide::Same,
            "opposite" => FlangeSide::Opposite,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            FlangeSide::Same => "same",
            FlangeSide::Opposite => "opposite",
        }
    }
}

/// A flange: a bend and a straight wall on a straight edge of a sheet, named like other
/// edges (`2:top 1`), so it follows the sheet when it rebuilds. Sizes are expressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetFlange {
    pub edge: String,
    /// From the outer virtual sharp to the flange's end, such as `30 mm`.
    pub height: String,
    /// The bend angle, such as `90 deg`.
    pub angle: String,
    #[serde(default)]
    pub position: FlangePosition,
    #[serde(default)]
    pub side: FlangeSide,
    /// How far the flange stops short of the edge's start and end (`None`: 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_end: Option<String>,
}

/// The flat pattern of a sheet (its base flange's id): shown as a separate flat body and
/// exported as DXF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetUnfold {
    pub base: u32,
}

/// A flange's sizes, evaluated and checked (mm, radians).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangeValues {
    pub height: f64,
    pub angle: f64,
    pub offset_start: f64,
    pub offset_end: f64,
}

/// The feature ids an edge or face reference names (`2:top 1`, `between 2:side 0 and
/// 5:top`): every number right before a `:`.
pub fn referenced_features(name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, _) in name.match_indices(':') {
        let head = name.get(..i).unwrap_or("");
        let digits: String = head
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(id) = digits.parse::<u32>()
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
    out
}

impl Document {
    /// The rule named `name`.
    pub fn sheet_rule(&self, name: &str) -> Result<&SheetRule, DocError> {
        self.sheet_rules
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| DocError::BadFeature {
                reason: format!("there is no sheet metal rule named {name:?}"),
            })
    }

    /// Adds or replaces a rule (by name). Fails without changing the design.
    pub fn set_sheet_rule(&mut self, rule: SheetRule) -> Result<(), DocError> {
        rule.check()?;
        if let Some(r) = self.sheet_rules.iter_mut().find(|r| r.name == rule.name) {
            *r = rule;
            return Ok(());
        }
        if self.sheet_rules.len() >= MAX_SHEET_RULES {
            return Err(DocError::BadFeature {
                reason: format!("a design holds at most {MAX_SHEET_RULES} sheet metal rules"),
            });
        }
        self.sheet_rules.push(rule);
        Ok(())
    }

    /// Removes a rule no base flange uses.
    pub fn remove_sheet_rule(&mut self, name: &str) -> Result<SheetRule, DocError> {
        let users: Vec<String> = self
            .features
            .iter()
            .filter(|f| matches!(&f.kind, crate::FeatureKind::SheetBase(b) if b.rule == name))
            .map(|f| f.name.clone())
            .collect();
        if !users.is_empty() {
            return Err(DocError::InUse {
                name: name.to_owned(),
                users,
            });
        }
        let i = self
            .sheet_rules
            .iter()
            .position(|r| r.name == name)
            .ok_or_else(|| DocError::BadFeature {
                reason: format!("there is no sheet metal rule named {name:?}"),
            })?;
        Ok(self.sheet_rules.remove(i))
    }

    /// A flange's sizes, checked: a height above 0, an angle above 0 and below 180
    /// degrees, offsets of 0 or more.
    pub fn flange_values(&self, f: &SheetFlange) -> Result<FlangeValues, DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        let tol = chamfera_geom::LINEAR_TOLERANCE;
        let max = chamfera_geom::MAX_COORDINATE;
        let height = self.evaluate_feature_value(&f.height, expr::Dim::LENGTH)?;
        if !height.is_finite() || height <= tol || height > max {
            return Err(bad(format!(
                "the flange height must be above 0 and at most {max} mm"
            )));
        }
        let angle = self.evaluate_feature_value(&f.angle, expr::Dim::ANGLE)?;
        let a = chamfera_geom::ANGULAR_TOLERANCE;
        if !angle.is_finite() || angle <= a || angle >= std::f64::consts::PI - a {
            return Err(bad(
                "the bend angle must be above 0 and below 180 degrees".into()
            ));
        }
        let offset = |v: &Option<String>| -> Result<f64, DocError> {
            let Some(src) = v else { return Ok(0.0) };
            let o = self.evaluate_feature_value(src, expr::Dim::LENGTH)?;
            if !o.is_finite() || o < 0.0 || o > max {
                return Err(bad(format!("a flange offset must be from 0 to {max} mm")));
            }
            Ok(o)
        };
        Ok(FlangeValues {
            height,
            angle,
            offset_start: offset(&f.offset_start)?,
            offset_end: offset(&f.offset_end)?,
        })
    }

    pub(crate) fn check_sheet_base(&self, b: &SheetBase) -> Result<(), DocError> {
        if b.profiles.len() != 1 {
            return Err(DocError::BadFeature {
                reason: "a base flange takes exactly one profile".into(),
            });
        }
        self.sheet_rule(&b.rule)?;
        Ok(())
    }

    pub(crate) fn check_sheet_flange(&self, f: &SheetFlange) -> Result<(), DocError> {
        let bad = |reason: String| DocError::BadFeature { reason };
        if f.edge.trim().is_empty() || f.edge.chars().count() > crate::MAX_EDGE_REF_LEN {
            return Err(bad("the flange edge name is empty or too long".into()));
        }
        let ids = referenced_features(&f.edge);
        if ids.is_empty() {
            return Err(bad(format!(
                "{:?} is not an edge name like \"2:top 1\"",
                f.edge
            )));
        }
        for id in ids {
            self.feature(id)?;
        }
        self.flange_values(f).map(|_| ())
    }

    pub(crate) fn check_sheet_unfold(&self, u: &SheetUnfold) -> Result<(), DocError> {
        match &self.feature(u.base)?.kind {
            crate::FeatureKind::SheetBase(_) => Ok(()),
            _ => Err(DocError::BadFeature {
                reason: format!("feature {} is not a base flange", u.base),
            }),
        }
    }

    /// Re-checks the rules read from a file.
    pub(crate) fn check_sheet_rules(rules: &[SheetRule]) -> Result<(), DocError> {
        if rules.len() > MAX_SHEET_RULES {
            return Err(DocError::BadFeature {
                reason: "too many sheet metal rules".into(),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in rules {
            r.check()?;
            if !seen.insert(r.name.as_str()) {
                return Err(DocError::BadFeature {
                    reason: format!("two sheet metal rules are named {:?}", r.name),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FeatureKind;

    fn plate() -> (Document, u32) {
        let mut doc = Document::default();
        let id = doc.add_sketch(chamfera_geom::Frame::XY).unwrap();
        let s = &mut doc.sketch_mut(id).unwrap().sketch;
        let p: Vec<_> = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)]
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4]).unwrap();
        }
        (doc, id)
    }

    #[test]
    fn rules_are_checked_named_once_and_survive_a_round_trip() {
        let (mut doc, sketch) = plate();
        doc.set_sheet_rule(SheetRule::default_named("Steel 2"))
            .unwrap();
        let mut thin = SheetRule::default_named("Thin");
        thin.thickness = 0.5;
        thin.relief = ReliefShape::Obround;
        doc.set_sheet_rule(thin.clone()).unwrap();
        for bad in [
            SheetRule {
                thickness: 0.0,
                ..thin.clone()
            },
            SheetRule {
                thickness: f64::NAN,
                ..thin.clone()
            },
            SheetRule {
                k_factor: 1.2,
                ..thin.clone()
            },
            SheetRule {
                bend_radius: -1.0,
                ..thin.clone()
            },
            SheetRule {
                name: " ".into(),
                ..thin.clone()
            },
        ] {
            assert!(doc.set_sheet_rule(bad).is_err());
        }
        assert_eq!(doc.sheet_rules.len(), 2);
        let base = doc
            .add_feature(FeatureKind::SheetBase(SheetBase {
                sketch,
                profiles: vec![Point2::new(50.0, 25.0)],
                rule: "Thin".into(),
                reversed: false,
            }))
            .unwrap();
        doc.add_feature(FeatureKind::SheetFlange(SheetFlange {
            edge: format!("{base}:top 1"),
            height: "30 mm".into(),
            angle: "90 deg".into(),
            position: FlangePosition::Outside,
            side: FlangeSide::Opposite,
            offset_start: Some("5".into()),
            offset_end: None,
        }))
        .unwrap();
        doc.add_feature(FeatureKind::SheetUnfold(SheetUnfold { base }))
            .unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        // A rule in use cannot go; the base cannot be deleted under its flange.
        assert!(doc.remove_sheet_rule("Thin").is_err());
        doc.remove_sheet_rule("Steel 2").unwrap();
        assert!(!doc.feature_users(base).is_empty());
        // A file with a bad rule is refused.
        let mut text: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        text["sheet_rules"][0]["k_factor"] = serde_json::json!(3.0);
        assert!(Document::from_json(&text.to_string()).is_err());
    }

    #[test]
    fn bad_sheet_features_are_refused() {
        let (mut doc, sketch) = plate();
        // No such rule.
        assert!(
            doc.add_feature(FeatureKind::SheetBase(SheetBase {
                sketch,
                profiles: vec![Point2::new(50.0, 25.0)],
                rule: "None".into(),
                reversed: false,
            }))
            .is_err()
        );
        doc.set_sheet_rule(SheetRule::default_named("R")).unwrap();
        let base = doc
            .add_feature(FeatureKind::SheetBase(SheetBase {
                sketch,
                profiles: vec![Point2::new(50.0, 25.0)],
                rule: "R".into(),
                reversed: false,
            }))
            .unwrap();
        let ok = SheetFlange {
            edge: format!("{base}:top 1"),
            height: "30".into(),
            angle: "90 deg".into(),
            position: FlangePosition::Inside,
            side: FlangeSide::Same,
            offset_start: None,
            offset_end: None,
        };
        for bad in [
            SheetFlange {
                edge: "".into(),
                ..ok.clone()
            },
            SheetFlange {
                edge: "top 1".into(),
                ..ok.clone()
            },
            SheetFlange {
                edge: "99:top 1".into(),
                ..ok.clone()
            },
            SheetFlange {
                height: "0".into(),
                ..ok.clone()
            },
            SheetFlange {
                angle: "180 deg".into(),
                ..ok.clone()
            },
            SheetFlange {
                angle: "0 deg".into(),
                ..ok.clone()
            },
            SheetFlange {
                offset_end: Some("-1".into()),
                ..ok.clone()
            },
        ] {
            assert!(doc.add_feature(FeatureKind::SheetFlange(bad)).is_err());
        }
        assert!(
            doc.add_feature(FeatureKind::SheetUnfold(SheetUnfold { base: 99 }))
                .is_err()
        );
        assert_eq!(
            referenced_features("between 12:side 0 and 3:top"),
            vec![12, 3]
        );
    }
}

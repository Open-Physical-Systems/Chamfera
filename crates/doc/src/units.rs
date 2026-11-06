//! The design's display units: which length unit values are shown and typed in, and how
//! many decimal places lengths and angles show.
//!
//! This is presentation only. Internally every length is mm and every angle radians, so
//! changing the units never changes the model: the stored values stay as they are and
//! only their labels change. A typed value without a unit is read in the design's length
//! unit (`10` in an inch design is 10 in = 254 mm); [`Document::set_units`] adds the old
//! unit to stored expressions that have none, so switching units keeps every size.

use crate::{DocError, Document, FeatureKind, Unit, expr};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most decimal places a length or angle may show.
pub const MAX_DECIMALS: u8 = 8;

/// A length unit values can be shown and typed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    #[default]
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl LengthUnit {
    /// Every unit, in menu order.
    pub const ALL: [LengthUnit; 5] = [
        LengthUnit::Mm,
        LengthUnit::Cm,
        LengthUnit::M,
        LengthUnit::In,
        LengthUnit::Ft,
    ];

    /// The unit's symbol, as typed in expressions and shown after values: `mm`, `in`.
    pub fn symbol(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Cm => "cm",
            LengthUnit::M => "m",
            LengthUnit::In => "in",
            LengthUnit::Ft => "ft",
        }
    }

    /// The unit's name for menus: "Millimetres (mm)".
    pub fn name(self) -> &'static str {
        match self {
            LengthUnit::Mm => "Millimetres (mm)",
            LengthUnit::Cm => "Centimetres (cm)",
            LengthUnit::M => "Metres (m)",
            LengthUnit::In => "Inches (in)",
            LengthUnit::Ft => "Feet (ft)",
        }
    }

    /// Parses a symbol (`mm`, `cm`, `m`, `in`, `ft`).
    pub fn parse(s: &str) -> Option<LengthUnit> {
        LengthUnit::ALL.into_iter().find(|u| u.symbol() == s)
    }

    /// How many mm one of this unit is (exact: 1 in = 25.4 mm by definition).
    pub fn mm(self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::Cm => 10.0,
            LengthUnit::M => 1000.0,
            LengthUnit::In => 25.4,
            LengthUnit::Ft => 304.8,
        }
    }
}

/// The design's display units and precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayUnits {
    pub length: LengthUnit,
    /// Decimal places shown for lengths (0 to [`MAX_DECIMALS`]).
    pub length_decimals: u8,
    /// Decimal places shown for angles, in degrees (0 to [`MAX_DECIMALS`]).
    pub angle_decimals: u8,
}

impl Default for DisplayUnits {
    fn default() -> Self {
        Self {
            length: LengthUnit::Mm,
            length_decimals: 2,
            angle_decimals: 2,
        }
    }
}

/// A number with a fixed count of decimals, without a minus sign on zero: `12.346`.
pub fn fixed(v: f64, decimals: u8) -> String {
    if !v.is_finite() {
        return "?".to_owned();
    }
    let d = usize::from(decimals.min(MAX_DECIMALS));
    let text = format!("{v:.d$}");
    // "-0.00" reads as a bug; a value that rounds to zero is zero.
    if text.starts_with('-') && text.bytes().all(|b| matches!(b, b'-' | b'0' | b'.')) {
        return text.trim_start_matches('-').to_owned();
    }
    text
}

impl DisplayUnits {
    pub fn is_default(&self) -> bool {
        *self == DisplayUnits::default()
    }

    /// Checks the precision (a file is hostile input).
    pub fn validate(&self) -> Result<(), String> {
        if self.length_decimals > MAX_DECIMALS || self.angle_decimals > MAX_DECIMALS {
            return Err(format!("decimal places must be 0 to {MAX_DECIMALS}"));
        }
        Ok(())
    }

    /// A length (mm) as a number in the display unit, with the set decimals: `10.00`.
    pub fn length_number(&self, mm: f64) -> String {
        fixed(mm / self.length.mm(), self.length_decimals)
    }

    /// A length (mm) with its unit: `10.00 in`.
    pub fn length(&self, mm: f64) -> String {
        format!("{} {}", self.length_number(mm), self.length.symbol())
    }

    /// An area (mm²) in the display unit: `6.45 in²`.
    pub fn area(&self, mm2: f64) -> String {
        let s = self.length.mm();
        format!(
            "{} {}²",
            fixed(mm2 / (s * s), self.length_decimals),
            self.length.symbol()
        )
    }

    /// A volume (mm³) in the display unit: `1.00 in³`.
    pub fn volume(&self, mm3: f64) -> String {
        let s = self.length.mm();
        format!(
            "{} {}³",
            fixed(mm3 / (s * s * s), self.length_decimals),
            self.length.symbol()
        )
    }

    /// An angle (radians) as degrees with the set decimals, no symbol: `45.00`.
    pub fn angle_number(&self, rad: f64) -> String {
        fixed(rad.to_degrees(), self.angle_decimals)
    }

    /// An angle (radians) in degrees: `45.00°`.
    pub fn angle(&self, rad: f64) -> String {
        format!("{}°", self.angle_number(rad))
    }
}

impl FeatureKind {
    /// The feature's length expressions, to give them a unit when the design's unit
    /// changes. Every kind lists all of its length expressions here.
    fn length_expressions_mut(&mut self) -> Vec<&mut String> {
        use crate::construct::{Construction as C, PointRef};
        fn point(p: &mut PointRef) -> Vec<&mut String> {
            match p {
                PointRef::Coordinates { x, y, z } => vec![x, y, z],
                PointRef::Origin
                | PointRef::Vertex { .. }
                | PointRef::Centre { .. }
                | PointRef::Construct { .. } => Vec::new(),
            }
        }
        match self {
            FeatureKind::Extrude(e) => std::iter::once(&mut e.distance)
                .chain(
                    [&mut e.distance2, &mut e.start_offset]
                        .into_iter()
                        .flatten(),
                )
                .collect(),
            FeatureKind::Revolve(_) => Vec::new(),
            FeatureKind::Blend(b) => vec![&mut b.size],
            FeatureKind::Hole(h) => std::iter::once(&mut h.diameter)
                .chain(
                    [
                        &mut h.depth,
                        &mut h.cb_diameter,
                        &mut h.cb_depth,
                        &mut h.cs_diameter,
                    ]
                    .into_iter()
                    .flatten(),
                )
                .collect(),
            FeatureKind::Mirror(_) => Vec::new(),
            FeatureKind::Move(m) => match &mut m.transform {
                crate::MoveTransform::Translate { dx, dy, dz } => vec![dx, dy, dz],
                crate::MoveTransform::Rotate { .. } | crate::MoveTransform::PointToPoint { .. } => {
                    Vec::new()
                }
            },
            FeatureKind::Shell(s) => vec![&mut s.thickness],
            FeatureKind::Pattern(p) => match &mut p.layout {
                crate::PatternLayout::Linear { spacing, .. } => vec![spacing],
                crate::PatternLayout::Circular { .. } => Vec::new(),
            },
            FeatureKind::Construct(c) => match c {
                C::PlaneOffset { distance, .. } => vec![distance],
                C::PlaneThreePoints { points } => points.iter_mut().flat_map(point).collect(),
                C::AxisTwoPoints { points } => points.iter_mut().flat_map(point).collect(),
                C::PlaneTangent { point: p, .. } => point(p),
                C::Point { at } => point(at),
                C::PlaneAngle { .. }
                | C::PlaneMidplane { .. }
                | C::AxisCylinder { .. }
                | C::AxisEdge { .. }
                | C::AxisTwoPlanes { .. } => Vec::new(),
            },
        }
    }
}

/// `expr` with `unit` added when it is a plain number (or a formula of plain numbers),
/// so it keeps its size when the design's unit changes: `10` → `10 mm`,
/// `10 + 5` → `(10 + 5) mm`. Expressions with units or lengths are left alone.
fn with_unit(src: &str, unit: LengthUnit, values: &BTreeMap<String, expr::Quantity>) -> String {
    match expr::evaluate(src, values) {
        Ok(q) if q.dim == expr::Dim::NONE => {
            let t = src.trim();
            if t.parse::<f64>().is_ok() {
                format!("{t} {}", unit.symbol())
            } else {
                format!("({t}) {}", unit.symbol())
            }
        }
        _ => src.to_owned(),
    }
}

impl Document {
    /// The length unit's size in mm: what a plain number in an expression means.
    pub fn length_scale(&self) -> f64 {
        self.units.length.mm()
    }

    /// Changes the display units. Every stored length expression without a unit gets the
    /// old unit written into it first, so the model keeps its size; only labels change.
    pub fn set_units(&mut self, units: DisplayUnits) -> Result<(), DocError> {
        units.validate().map_err(|reason| DocError::BadValue {
            name: "units".into(),
            reason,
        })?;
        let old = self.units.length;
        if old != units.length {
            let values: BTreeMap<String, expr::Quantity> = self
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
            for p in self.parameters.values_mut() {
                if p.unit == Unit::Length
                    && let Some(e) = p.expression.as_mut()
                {
                    *e = with_unit(e, old, &values);
                }
            }
            for s in &mut self.sketches {
                for (c, e) in s.expressions.iter_mut() {
                    let angle = s
                        .sketch
                        .constraint(chamfera_sketch::ConstraintId(*c))
                        .is_some_and(|k| k.kind_name() == "angle");
                    if !angle {
                        *e = with_unit(e, old, &values);
                    }
                }
            }
            for f in &mut self.features {
                for e in f.kind.length_expressions_mut() {
                    *e = with_unit(e, old, &values);
                }
            }
        }
        self.units = units;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_three_rounds_to_three_places() {
        // 12.3456 to three places: the fourth digit 6 rounds the third up, 12.346.
        let u = DisplayUnits {
            length_decimals: 3,
            ..DisplayUnits::default()
        };
        assert_eq!(u.length(12.3456), "12.346 mm");
        assert_eq!(fixed(12.3456, 3), "12.346");
        assert_eq!(fixed(-0.0001, 2), "0.00");
        assert_eq!(fixed(f64::NAN, 2), "?");
    }

    #[test]
    fn lengths_show_in_the_display_unit() {
        let inch = DisplayUnits {
            length: LengthUnit::In,
            ..DisplayUnits::default()
        };
        // 254 mm / 25.4 mm per inch = 10 in.
        assert_eq!(inch.length(254.0), "10.00 in");
        // 645.16 mm² = 1 in²; 16387.064 mm³ = 1 in³ (25.4² and 25.4³).
        assert_eq!(inch.area(645.16), "1.00 in²");
        assert_eq!(inch.volume(16_387.064), "1.00 in³");
        let feet = DisplayUnits {
            length: LengthUnit::Ft,
            length_decimals: 1,
            ..DisplayUnits::default()
        };
        assert_eq!(feet.length(609.6), "2.0 ft");
        let deg = DisplayUnits {
            angle_decimals: 1,
            ..DisplayUnits::default()
        };
        assert_eq!(deg.angle(std::f64::consts::FRAC_PI_4), "45.0°");
    }

    #[test]
    fn units_parse_by_symbol() {
        for u in LengthUnit::ALL {
            assert_eq!(LengthUnit::parse(u.symbol()), Some(u));
        }
        assert_eq!(LengthUnit::parse("furlong"), None);
        assert_eq!(LengthUnit::parse("MM"), None);
    }

    #[test]
    fn a_plain_number_in_an_inch_design_is_inches() {
        let mut doc = Document::default();
        doc.set_units(DisplayUnits {
            length: LengthUnit::In,
            ..DisplayUnits::default()
        })
        .unwrap();
        doc.set_parameter_expression("w", "10", Unit::Length)
            .unwrap();
        // 10 in × 25.4 mm/in = 254 mm.
        assert!((doc.parameters["w"].value - 254.0).abs() < 1e-9);
        assert_eq!(doc.units.length(doc.parameters["w"].value), "10.00 in");
        // A unit typed explicitly still wins: 10 mm is 10 mm.
        doc.set_parameter_expression("t", "10 mm", Unit::Length)
            .unwrap();
        assert!((doc.parameters["t"].value - 10.0).abs() < 1e-9);
        // Angles keep degrees: 90 is a right angle whatever the length unit.
        doc.set_parameter_expression("a", "90", Unit::Angle)
            .unwrap();
        assert!((doc.parameters["a"].value - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn changing_the_unit_keeps_every_size() {
        let mut doc = Document::default();
        doc.set_parameter_expression("w", "10", Unit::Length)
            .unwrap();
        doc.set_parameter_expression("h", "w / 2", Unit::Length)
            .unwrap();
        doc.set_parameter_expression("s", "2 + 3", Unit::Length)
            .unwrap();
        doc.set_units(DisplayUnits {
            length: LengthUnit::In,
            ..DisplayUnits::default()
        })
        .unwrap();
        doc.evaluate().unwrap();
        assert_eq!(doc.parameters["w"].expression.as_deref(), Some("10 mm"));
        assert_eq!(doc.parameters["h"].expression.as_deref(), Some("w / 2"));
        assert_eq!(
            doc.parameters["s"].expression.as_deref(),
            Some("(2 + 3) mm")
        );
        assert!((doc.parameters["w"].value - 10.0).abs() < 1e-9);
        assert!((doc.parameters["h"].value - 5.0).abs() < 1e-9);
        assert!((doc.parameters["s"].value - 5.0).abs() < 1e-9);
    }

    #[test]
    fn bad_precision_is_refused() {
        let mut doc = Document::default();
        let bad = DisplayUnits {
            length_decimals: 9,
            ..DisplayUnits::default()
        };
        assert!(doc.set_units(bad).is_err());
        assert!(doc.units.is_default());
    }

    #[test]
    fn units_survive_save_and_open_and_hostile_units_are_refused() {
        let mut doc = Document::default();
        doc.set_units(DisplayUnits {
            length: LengthUnit::Ft,
            length_decimals: 4,
            angle_decimals: 0,
        })
        .unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back.units, doc.units);
        // A file without `units` (format 23 and older) opens in mm with two decimals.
        let old = r#"{"format":"chamfera","version":11,"parameters":{}}"#;
        assert!(Document::from_json(old).unwrap().units.is_default());
        for bad in [
            r#""units":{"length":"furlong","length_decimals":2,"angle_decimals":2}"#,
            r#""units":{"length":"in","length_decimals":9,"angle_decimals":2}"#,
            r#""units":{"length":"in","length_decimals":300,"angle_decimals":2}"#,
            r#""units":{"length":"in","length_decimals":-1,"angle_decimals":2}"#,
            r#""units":{"length":5}"#,
            r#""units":"in""#,
        ] {
            let text = format!(r#"{{"format":"chamfera","version":24,"parameters":{{}},{bad}}}"#);
            assert!(Document::from_json(&text).is_err(), "{bad}");
        }
    }
}

//! 2D drafting on a drawing sheet (format 41): user layers, line types and weights, and
//! the lines, circles, arcs and polylines drawn on them.
//!
//! Every sheet has its own [`Drafting`]: a list of [`DraftLayer`]s (layer `0` always
//! exists and cannot be removed or renamed), the current layer new geometry goes on, a
//! line type scale for the whole sheet, and the [`Entity`]s. An entity takes its colour,
//! line type and line weight from its layer. A layer that is off or frozen hides its
//! entities on screen and in every export; a locked layer shows them but refuses edits;
//! a frozen layer cannot be the current one.
//!
//! Positions are sheet millimetres from the sheet's lower-left corner, y up; angles are
//! radians, counter-clockwise from +x. Entity ids come from the drawing's one id counter
//! (`Drawing::last_id`).

use crate::drawing::is_valid_sheet_coordinate;
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// Most layers on one sheet.
pub const MAX_LAYERS: usize = 256;
/// Most drafted entities on one sheet.
pub const MAX_ENTITIES: usize = 20_000;
/// Most points of one polyline.
pub const MAX_POLYLINE_POINTS: usize = 2_000;
/// Longest layer name, in characters (DXF R12 allows 31).
pub const MAX_LAYER_NAME: usize = 31;
/// Thinnest and thickest line weight (mm).
pub const MIN_WEIGHT: f64 = 0.05;
pub const MAX_WEIGHT: f64 = 2.5;
/// Smallest and largest line type scale.
pub const MIN_LINETYPE_SCALE: f64 = 0.01;
pub const MAX_LINETYPE_SCALE: f64 = 100.0;
/// The plotted line weights the layer panel offers (ISO 128-20 series, mm).
pub const WEIGHTS: [f64; 9] = [0.13, 0.18, 0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0];
/// The layer that always exists.
pub const DEFAULT_LAYER: &str = "0";
/// Layer names the sheet's own lines use in exports, which user layers may not take.
pub const RESERVED_LAYERS: [&str; 7] = [
    "VISIBLE", "HIDDEN", "CENTER", "HATCH", "DIM", "DANGLING", "BORDER",
];

/// How a line is broken into dashes. The patterns themselves (in `chamfera-drawing`) are
/// our own, in sheet mm at line type scale 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineType {
    #[default]
    Continuous,
    Dashed,
    Hidden,
    Center,
    Phantom,
    Dot,
}

impl LineType {
    pub const ALL: [LineType; 6] = [
        LineType::Continuous,
        LineType::Dashed,
        LineType::Hidden,
        LineType::Center,
        LineType::Phantom,
        LineType::Dot,
    ];

    /// The id commands and files use.
    pub fn id(self) -> &'static str {
        match self {
            LineType::Continuous => "continuous",
            LineType::Dashed => "dashed",
            LineType::Hidden => "hidden",
            LineType::Center => "center",
            LineType::Phantom => "phantom",
            LineType::Dot => "dot",
        }
    }

    /// The id, or `centre` for `center`.
    pub fn parse(s: &str) -> Option<LineType> {
        let s = s.trim().to_ascii_lowercase();
        if s == "centre" {
            return Some(LineType::Center);
        }
        LineType::ALL.into_iter().find(|t| t.id() == s)
    }
}

/// One user layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftLayer {
    pub name: String,
    /// sRGB; `None` draws in the sheet's ink colour (black on paper).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<[u8; 3]>,
    #[serde(default)]
    pub line_type: LineType,
    /// Plotted width, mm.
    pub weight: f64,
    /// Shown (off hides it, and its entities are not exported).
    #[serde(default = "yes")]
    pub on: bool,
    /// Frozen: hidden like off, and it cannot be the current layer.
    #[serde(default)]
    pub frozen: bool,
    /// Locked: shown, but its entities cannot be picked for editing.
    #[serde(default)]
    pub locked: bool,
}

fn yes() -> bool {
    true
}

impl DraftLayer {
    /// A continuous, 0.25 mm, ink-coloured layer that is on.
    pub fn new(name: &str) -> DraftLayer {
        DraftLayer {
            name: name.to_owned(),
            colour: None,
            line_type: LineType::Continuous,
            weight: 0.25,
            on: true,
            frozen: false,
            locked: false,
        }
    }

    /// Shown on screen and in exports.
    pub fn visible(&self) -> bool {
        self.on && !self.frozen
    }
}

/// What a drafted entity is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum EntityKind {
    Line {
        a: Point2,
        b: Point2,
    },
    Circle {
        centre: Point2,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` (radians, `start < end <= start + 2π`).
    Arc {
        centre: Point2,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// Straight segments through `points`; `closed` joins the last back to the first.
    Polyline {
        points: Vec<Point2>,
        #[serde(default)]
        closed: bool,
    },
}

impl EntityKind {
    /// The id commands use.
    pub fn id(&self) -> &'static str {
        match self {
            EntityKind::Line { .. } => "line",
            EntityKind::Circle { .. } => "circle",
            EntityKind::Arc { .. } => "arc",
            EntityKind::Polyline { .. } => "polyline",
        }
    }

    /// Refuses non-finite or out-of-range positions, non-positive radii, bad arc angles
    /// and polylines with too few or too many points.
    pub fn check(&self) -> Result<(), String> {
        let place = |p: &Point2| {
            if is_valid_sheet_coordinate(p.x) && is_valid_sheet_coordinate(p.y) {
                Ok(())
            } else {
                Err(format!(
                    "positions must be finite and within ±{} mm",
                    crate::drawing::MAX_SHEET_COORDINATE
                ))
            }
        };
        let radius_ok = |r: f64| {
            if r.is_finite() && r > 1e-6 && r <= crate::drawing::MAX_SHEET_COORDINATE {
                Ok(())
            } else {
                Err(format!(
                    "the radius must be a finite number above 0 and at most {} mm",
                    crate::drawing::MAX_SHEET_COORDINATE
                ))
            }
        };
        match self {
            EntityKind::Line { a, b } => {
                place(a)?;
                place(b)?;
                if a.distance(*b) <= 1e-6 {
                    return Err("a line needs two different points".into());
                }
            }
            EntityKind::Circle { centre, radius } => {
                place(centre)?;
                radius_ok(*radius)?;
            }
            EntityKind::Arc {
                centre,
                radius,
                start,
                end,
            } => {
                place(centre)?;
                radius_ok(*radius)?;
                let sweep = end - start;
                if !(start.is_finite()
                    && end.is_finite()
                    && start.abs() <= 64.0
                    && sweep > 1e-9
                    && sweep <= std::f64::consts::TAU + 1e-9)
                {
                    return Err(
                        "an arc runs counter-clockwise from `start` to `end` (radians), more \
                         than 0 and at most a full turn"
                            .into(),
                    );
                }
            }
            EntityKind::Polyline { points, closed } => {
                let least = if *closed { 3 } else { 2 };
                if !(least..=MAX_POLYLINE_POINTS).contains(&points.len()) {
                    return Err(format!(
                        "a polyline has {least} to {MAX_POLYLINE_POINTS} points"
                    ));
                }
                points.iter().try_for_each(place)?;
            }
        }
        Ok(())
    }
}

/// One drafted entity on a layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: u32,
    pub layer: String,
    pub kind: EntityKind,
}

/// A sheet's layers and drafted entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Drafting {
    pub layers: Vec<DraftLayer>,
    /// The layer new entities go on.
    pub current: String,
    /// Multiplies every dash and gap of every line type on the sheet.
    pub linetype_scale: f64,
    pub entities: Vec<Entity>,
}

impl Default for Drafting {
    fn default() -> Self {
        Drafting {
            layers: vec![DraftLayer::new(DEFAULT_LAYER)],
            current: DEFAULT_LAYER.to_owned(),
            linetype_scale: 1.0,
            entities: Vec::new(),
        }
    }
}

/// A layer name: 1 to 31 letters, digits, `_`, `-` or `$` (the characters DXF R12 takes),
/// not one of the sheet's own layer names.
pub fn is_valid_layer_name(s: &str) -> bool {
    let n = s.chars().count();
    (1..=MAX_LAYER_NAME).contains(&n)
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
        && !RESERVED_LAYERS.iter().any(|r| r.eq_ignore_ascii_case(s))
}

/// A line weight the sheet accepts.
pub fn is_valid_weight(w: f64) -> bool {
    w.is_finite() && (MIN_WEIGHT..=MAX_WEIGHT).contains(&w)
}

/// A line type scale the sheet accepts.
pub fn is_valid_linetype_scale(k: f64) -> bool {
    k.is_finite() && (MIN_LINETYPE_SCALE..=MAX_LINETYPE_SCALE).contains(&k)
}

impl Drafting {
    /// Nothing but the default layer, at scale 1, with nothing drawn (not saved).
    pub fn is_default(&self) -> bool {
        *self == Drafting::default()
    }

    /// The layer named `name` (names compare without case).
    pub fn layer(&self, name: &str) -> Option<&DraftLayer> {
        self.layers
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(name))
    }

    pub fn layer_mut(&mut self, name: &str) -> Option<&mut DraftLayer> {
        self.layers
            .iter_mut()
            .find(|l| l.name.eq_ignore_ascii_case(name))
    }

    pub fn entity(&self, id: u32) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }

    /// Checks what a file holds: names, counts, numbers, the current layer and where
    /// every entity is. Ids are checked with the drawing's other ids.
    pub fn check(&self) -> Result<(), String> {
        if self.layers.is_empty() || self.layers.len() > MAX_LAYERS {
            return Err(format!("a sheet has 1 to {MAX_LAYERS} layers"));
        }
        for (i, l) in self.layers.iter().enumerate() {
            if l.name != DEFAULT_LAYER && !is_valid_layer_name(&l.name) {
                return Err(format!(
                    "layer {:?}: names hold 1 to {MAX_LAYER_NAME} letters, digits, _, - or $ \
                     and are not one of {}",
                    l.name,
                    RESERVED_LAYERS.join(", ")
                ));
            }
            if self.layers[..i]
                .iter()
                .any(|o| o.name.eq_ignore_ascii_case(&l.name))
            {
                return Err(format!("layer {:?} is there twice", l.name));
            }
            if !is_valid_weight(l.weight) {
                return Err(format!(
                    "layer {:?}: the line weight must be {MIN_WEIGHT} to {MAX_WEIGHT} mm",
                    l.name
                ));
            }
        }
        if self.layer(DEFAULT_LAYER).is_none() {
            return Err("layer 0 must exist".into());
        }
        match self.layer(&self.current) {
            None => {
                return Err(format!(
                    "the current layer {:?} does not exist",
                    self.current
                ));
            }
            Some(l) if l.frozen => {
                return Err(format!(
                    "the current layer {:?} is frozen: a frozen layer cannot be current",
                    l.name
                ));
            }
            Some(_) => {}
        }
        if !is_valid_linetype_scale(self.linetype_scale) {
            return Err(format!(
                "the line type scale must be {MIN_LINETYPE_SCALE} to {MAX_LINETYPE_SCALE}"
            ));
        }
        if self.entities.len() > MAX_ENTITIES {
            return Err(format!(
                "a sheet holds at most {MAX_ENTITIES} drafted entities"
            ));
        }
        for e in &self.entities {
            if self.layer(&e.layer).is_none() {
                return Err(format!("entity {}: no layer {:?}", e.id, e.layer));
            }
            e.kind
                .check()
                .map_err(|m| format!("entity {}: {m}", e.id))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_layer_zero_current_at_scale_one() {
        let d = Drafting::default();
        assert!(d.is_default());
        assert_eq!(d.current, "0");
        assert!(d.check().is_ok());
        let json = serde_json::to_string(&d).unwrap();
        let back: Drafting = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        // An empty object reads as the default.
        let empty: Drafting = serde_json::from_str("{}").unwrap();
        assert!(empty.is_default());
    }

    #[test]
    fn bad_layers_and_entities_are_refused() {
        let mut d = Drafting::default();
        d.layers.push(DraftLayer::new("hidden"));
        assert!(d.check().is_err(), "a reserved name");
        d.layers.pop();
        d.layers.push(DraftLayer::new("A B"));
        assert!(d.check().is_err(), "a space");
        d.layers.pop();
        d.layers.push(DraftLayer::new("Axes"));
        d.layers.push(DraftLayer::new("AXES"));
        assert!(d.check().is_err(), "a duplicate");
        d.layers.pop();
        d.layers[1].frozen = true;
        d.current = "Axes".into();
        assert!(d.check().is_err(), "a frozen current layer");
        d.current = "0".into();
        assert!(d.check().is_ok());
        d.layers[1].weight = f64::NAN;
        assert!(d.check().is_err());
        d.layers[1].weight = 0.35;
        d.entities.push(Entity {
            id: 1,
            layer: "nope".into(),
            kind: EntityKind::Line {
                a: Point2::new(0.0, 0.0),
                b: Point2::new(1.0, 0.0),
            },
        });
        assert!(d.check().is_err(), "no such layer");
        d.entities[0].layer = "axes".into();
        assert!(d.check().is_ok(), "layer names compare without case");
        d.entities[0].kind = EntityKind::Arc {
            centre: Point2::new(0.0, 0.0),
            radius: 5.0,
            start: 1.0,
            end: 1.0,
        };
        assert!(d.check().is_err(), "an empty arc");
        d.entities[0].kind = EntityKind::Polyline {
            points: vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)],
            closed: true,
        };
        assert!(d.check().is_err(), "a closed polyline of two points");
        d.linetype_scale = 0.0;
        d.entities.clear();
        assert!(d.check().is_err());
    }

    #[test]
    fn line_types_parse_by_id() {
        for t in LineType::ALL {
            assert_eq!(LineType::parse(t.id()), Some(t));
        }
        assert_eq!(LineType::parse("Centre"), Some(LineType::Center));
        assert_eq!(LineType::parse("zigzag"), None);
    }
}

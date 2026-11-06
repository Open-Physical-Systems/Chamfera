//! Drawings: 2D sheets with views of the model and dimensions (format 18).
//!
//! A design holds a list of [`Sheet`]s. Each sheet has a paper size, a scale, a projection
//! angle and a title block, and holds views and dimensions. A view names the body it shows
//! (or none for every solid) and a viewing direction; its lines are worked out from the
//! model whenever it is drawn (`chamfera-drawing`), so views follow the model. A
//! dimension stores the points it measures in its view's own 2D coordinates (model
//! millimetres along the view's right and up axes), snapped to projected vertices and
//! edges when it was made.
//!
//! Sheets, views and dimensions take ids from one counter ([`Drawing::last_id`]), so an id
//! names one thing on any sheet.

use crate::DocError;
use chamfera_geom::Point2;
use serde::{Deserialize, Serialize};

/// Most sheets one design may hold.
pub const MAX_SHEETS: usize = 1_000;
/// Most views on one sheet.
pub const MAX_VIEWS: usize = 64;
/// Most dimensions on one sheet.
pub const MAX_DIMENSIONS: usize = 2_000;
/// Longest text in a title block field or a name, in characters.
pub const MAX_TEXT: usize = 64;
/// Smallest and largest drawing scale (sheet mm per model mm).
pub const MIN_SCALE: f64 = 1e-3;
pub const MAX_SCALE: f64 = 1e3;
/// Largest sheet coordinate or offset (mm): a view or dimension further away than this
/// is surely a mistake.
pub const MAX_SHEET_COORDINATE: f64 = 1e4;

/// A paper size (ISO 216 A sizes and ANSI Y14.1 sizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    A4,
    A3,
    A2,
    AnsiA,
    AnsiB,
}

impl Paper {
    pub const ALL: [Paper; 5] = [Paper::A4, Paper::A3, Paper::A2, Paper::AnsiA, Paper::AnsiB];

    /// The long and the short side in mm. ISO 216: A4 297 x 210, A3 420 x 297,
    /// A2 594 x 420. ANSI: A is 11 x 8.5 in, B is 17 x 11 in (25.4 mm per inch).
    pub fn size(self) -> (f64, f64) {
        match self {
            Paper::A4 => (297.0, 210.0),
            Paper::A3 => (420.0, 297.0),
            Paper::A2 => (594.0, 420.0),
            Paper::AnsiA => (279.4, 215.9),
            Paper::AnsiB => (431.8, 279.4),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::A3 => "a3",
            Paper::A2 => "a2",
            Paper::AnsiA => "ansi_a",
            Paper::AnsiB => "ansi_b",
        }
    }

    pub fn parse(s: &str) -> Option<Paper> {
        Paper::ALL.into_iter().find(|p| p.id() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Landscape,
    Portrait,
}

/// Where projected views go relative to their base view (ISO 128-30). Third angle: the
/// top view above the front view, the right view to its right. First angle: the top view
/// below, the right view to the left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionAngle {
    First,
    Third,
}

/// The title block's fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleBlock {
    #[serde(default)]
    pub part_name: String,
    #[serde(default)]
    pub drawn_by: String,
    /// The date as written, for example `2026-11-06`.
    #[serde(default)]
    pub date: String,
}

/// The direction a view looks from. Z is up: the front view looks along +Y (from -Y),
/// the top view down along -Z, the right view along -X; the isometric view looks from
/// the front-right-top corner, along (-1, 1, -1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewDirection {
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
    Iso,
}

impl ViewDirection {
    pub const ALL: [ViewDirection; 7] = [
        ViewDirection::Front,
        ViewDirection::Back,
        ViewDirection::Top,
        ViewDirection::Bottom,
        ViewDirection::Left,
        ViewDirection::Right,
        ViewDirection::Iso,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ViewDirection::Front => "front",
            ViewDirection::Back => "back",
            ViewDirection::Top => "top",
            ViewDirection::Bottom => "bottom",
            ViewDirection::Left => "left",
            ViewDirection::Right => "right",
            ViewDirection::Iso => "iso",
        }
    }

    pub fn parse(s: &str) -> Option<ViewDirection> {
        ViewDirection::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// One view on a sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetView {
    pub id: u32,
    /// The solid shown, by name; `None` shows every solid.
    #[serde(default)]
    pub body: Option<String>,
    pub direction: ViewDirection,
    /// Where the centre of the model's bounding box goes on the sheet (mm from the
    /// sheet's lower-left corner). A projected view keeps only the distance from its
    /// base view; its side and alignment come from the projection angle.
    pub x: f64,
    pub y: f64,
    /// The view's own scale; `None` uses the sheet's (a projected view uses its base's).
    #[serde(default)]
    pub scale: Option<f64>,
    /// The base view this one is projected from.
    #[serde(default)]
    pub parent: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    Horizontal,
    Vertical,
    Aligned,
    Diameter,
    Radius,
    Angular,
}

impl DimensionKind {
    pub const ALL: [DimensionKind; 6] = [
        DimensionKind::Horizontal,
        DimensionKind::Vertical,
        DimensionKind::Aligned,
        DimensionKind::Diameter,
        DimensionKind::Radius,
        DimensionKind::Angular,
    ];

    pub fn id(self) -> &'static str {
        match self {
            DimensionKind::Horizontal => "horizontal",
            DimensionKind::Vertical => "vertical",
            DimensionKind::Aligned => "aligned",
            DimensionKind::Diameter => "diameter",
            DimensionKind::Radius => "radius",
            DimensionKind::Angular => "angular",
        }
    }

    pub fn parse(s: &str) -> Option<DimensionKind> {
        DimensionKind::ALL.into_iter().find(|k| k.id() == s)
    }

    /// How many points the dimension stores: two for linear ones (the measured points)
    /// and for diameters and radii (the centre, then a point on the circle where the
    /// leader goes), three for an angle (the vertex, then a point on each leg).
    pub fn point_count(self) -> usize {
        match self {
            DimensionKind::Angular => 3,
            _ => 2,
        }
    }
}

/// A dimension on a view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetDimension {
    pub id: u32,
    pub view: u32,
    pub kind: DimensionKind,
    /// In the view's 2D coordinates (model mm), see [`DimensionKind::point_count`].
    pub points: Vec<Point2>,
    /// Sheet mm: how far the dimension line is from the measured points (linear: beyond
    /// the outer point, negative for the other side), the leader's length (diameter and
    /// radius), or the arc's radius (angle).
    pub offset: f64,
}

/// One drawing sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub id: u32,
    pub name: String,
    pub paper: Paper,
    pub orientation: Orientation,
    /// Sheet mm per model mm: 0.5 is 1:2, 2 is 2:1.
    pub scale: f64,
    pub projection: ProjectionAngle,
    #[serde(default)]
    pub title: TitleBlock,
    #[serde(default)]
    pub views: Vec<SheetView>,
    #[serde(default)]
    pub dimensions: Vec<SheetDimension>,
}

impl Sheet {
    /// Width and height in mm.
    pub fn size(&self) -> (f64, f64) {
        let (long, short) = self.paper.size();
        match self.orientation {
            Orientation::Landscape => (long, short),
            Orientation::Portrait => (short, long),
        }
    }

    pub fn view(&self, id: u32) -> Option<&SheetView> {
        self.views.iter().find(|v| v.id == id)
    }
}

/// Every sheet of a design.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drawing {
    #[serde(default)]
    pub sheets: Vec<Sheet>,
    /// The last id handed out to a sheet, view or dimension; ids are never reused.
    #[serde(default)]
    pub last_id: u32,
}

fn bad(reason: String) -> DocError {
    DocError::BadDrawing { reason }
}

/// Title block and name text: at most [`MAX_TEXT`] characters, no control characters.
pub fn is_valid_text(s: &str) -> bool {
    s.chars().count() <= MAX_TEXT && !s.chars().any(char::is_control)
}

/// A scale the drawing accepts.
pub fn is_valid_scale(s: f64) -> bool {
    s.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&s)
}

/// A sheet coordinate or offset the drawing accepts.
pub fn is_valid_sheet_coordinate(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_SHEET_COORDINATE
}

impl Drawing {
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty() && self.last_id == 0
    }

    /// A new id.
    pub fn next_id(&mut self) -> Result<u32, DocError> {
        let id = self
            .last_id
            .checked_add(1)
            .ok_or_else(|| bad("no more drawing ids".into()))?;
        self.last_id = id;
        Ok(id)
    }

    pub fn sheet(&self, id: u32) -> Result<&Sheet, DocError> {
        self.sheets
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| bad(format!("no sheet {id}")))
    }

    pub fn sheet_mut(&mut self, id: u32) -> Result<&mut Sheet, DocError> {
        self.sheets
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| bad(format!("no sheet {id}")))
    }

    /// Checks a drawing read from a file: counts, ids, references, text and numbers.
    pub fn check(&self) -> Result<(), DocError> {
        if self.sheets.len() > MAX_SHEETS {
            return Err(bad(format!("a design holds at most {MAX_SHEETS} sheets")));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut id_ok = |id: u32, what: &str| {
            if id == 0 || id > self.last_id || !seen.insert(id) {
                Err(bad(format!(
                    "{what} {id} has a duplicate or out-of-range id"
                )))
            } else {
                Ok(())
            }
        };
        for s in &self.sheets {
            id_ok(s.id, "sheet")?;
            if !is_valid_text(&s.name)
                || !is_valid_text(&s.title.part_name)
                || !is_valid_text(&s.title.drawn_by)
                || !is_valid_text(&s.title.date)
            {
                return Err(bad(format!(
                    "sheet {}: names and title block fields hold at most {MAX_TEXT} \
                     characters and no control characters",
                    s.id
                )));
            }
            if !is_valid_scale(s.scale) {
                return Err(bad(format!(
                    "sheet {}: the scale must be between {MIN_SCALE} and {MAX_SCALE}",
                    s.id
                )));
            }
            if s.views.len() > MAX_VIEWS || s.dimensions.len() > MAX_DIMENSIONS {
                return Err(bad(format!(
                    "sheet {}: at most {MAX_VIEWS} views and {MAX_DIMENSIONS} dimensions",
                    s.id
                )));
            }
            for v in &s.views {
                id_ok(v.id, "view")?;
                let body_ok = v
                    .body
                    .as_deref()
                    .is_none_or(|b| !b.is_empty() && b.chars().count() <= 256);
                if !body_ok
                    || !is_valid_sheet_coordinate(v.x)
                    || !is_valid_sheet_coordinate(v.y)
                    || v.scale.is_some_and(|k| !is_valid_scale(k))
                {
                    return Err(bad(format!(
                        "view {}: bad body name, position or scale",
                        v.id
                    )));
                }
            }
            for v in &s.views {
                if let Some(p) = v.parent {
                    // A base view is never itself projected: no chains, no loops.
                    let base = s.view(p);
                    if p == v.id || base.is_none_or(|b| b.parent.is_some()) {
                        return Err(bad(format!(
                            "view {}: its base view {p} must be a view on the same sheet \
                             that is not itself projected",
                            v.id
                        )));
                    }
                }
            }
            for d in &s.dimensions {
                id_ok(d.id, "dimension")?;
                if s.view(d.view).is_none() {
                    return Err(bad(format!(
                        "dimension {}: no view {} on sheet {}",
                        d.id, d.view, s.id
                    )));
                }
                let finite = d.points.iter().all(|p| {
                    chamfera_geom::is_valid_coordinate(p.x)
                        && chamfera_geom::is_valid_coordinate(p.y)
                });
                if d.points.len() != d.kind.point_count()
                    || !finite
                    || !is_valid_sheet_coordinate(d.offset)
                {
                    return Err(bad(format!(
                        "dimension {}: a {} dimension needs {} finite points and a finite offset",
                        d.id,
                        d.kind.id(),
                        d.kind.point_count()
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(id: u32) -> Sheet {
        Sheet {
            id,
            name: "Sheet 1".into(),
            paper: Paper::A3,
            orientation: Orientation::Landscape,
            scale: 1.0,
            projection: ProjectionAngle::Third,
            title: TitleBlock::default(),
            views: Vec::new(),
            dimensions: Vec::new(),
        }
    }

    #[test]
    fn paper_sizes_follow_iso_216_and_ansi() {
        // A4 is 297 x 210; ANSI A is 11 x 8.5 in = 279.4 x 215.9 mm.
        assert_eq!(Paper::A4.size(), (297.0, 210.0));
        assert_eq!(Paper::AnsiA.size(), (279.4, 215.9));
        let mut s = sheet(1);
        s.orientation = Orientation::Portrait;
        assert_eq!(s.size(), (297.0, 420.0));
        for p in Paper::ALL {
            assert_eq!(Paper::parse(p.id()), Some(p));
        }
    }

    #[test]
    fn a_checked_drawing_refuses_bad_ids_references_and_numbers() {
        let mut d = Drawing {
            sheets: vec![sheet(1)],
            last_id: 3,
        };
        assert!(d.check().is_ok());
        d.sheets[0].views.push(SheetView {
            id: 2,
            body: None,
            direction: ViewDirection::Front,
            x: 100.0,
            y: 100.0,
            scale: None,
            parent: None,
        });
        assert!(d.check().is_ok());
        let mut child = d.sheets[0].views[0].clone();
        child.id = 3;
        child.parent = Some(3);
        d.sheets[0].views.push(child);
        assert!(d.check().is_err(), "a view cannot be its own base");
        d.sheets[0].views[1].parent = Some(2);
        assert!(d.check().is_ok());
        d.sheets[0].views[1].id = 2;
        assert!(d.check().is_err(), "duplicate id");
        d.sheets[0].views[1].id = 4;
        assert!(d.check().is_err(), "id beyond last_id");
        d.sheets[0].views[1].id = 3;
        d.sheets[0].scale = f64::NAN;
        assert!(d.check().is_err());
        d.sheets[0].scale = 1.0;
        d.sheets[0].title.part_name = "a\nb".into();
        assert!(d.check().is_err(), "control characters");
        d.sheets[0].title.part_name = "plate".into();
        d.last_id = 4;
        d.sheets[0].dimensions.push(SheetDimension {
            id: 4,
            view: 2,
            kind: DimensionKind::Angular,
            points: vec![Point2::new(0.0, 0.0); 2],
            offset: 10.0,
        });
        assert!(d.check().is_err(), "an angle needs three points");
        d.sheets[0].dimensions[0].kind = DimensionKind::Horizontal;
        assert!(d.check().is_ok());
        d.sheets[0].dimensions[0].view = 9;
        assert!(d.check().is_err(), "dimension on a missing view");
    }
}

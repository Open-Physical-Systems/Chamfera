//! Drawings: 2D sheets with views of the model and dimensions (format 18, extended in 30).
//!
//! A design holds a list of [`Sheet`]s. Each sheet has a paper size, a scale, a projection
//! angle and a title block, and holds views, dimensions, annotations and a parts list. A
//! view names the body it shows (or none for every solid) and a viewing direction; its
//! lines are worked out from the model whenever it is drawn (`chamfera-drawing`), so views
//! follow the model. A view can also be a section of another view ([`SectionCut`]: the
//! model cut along a line drawn on that view) or an enlarged detail of part of it
//! ([`DetailCircle`]).
//!
//! A dimension stores the points it measures in its view's own 2D coordinates (model
//! millimetres along the view's right and up axes), snapped to projected vertices and
//! edges when it was made. Since format 30 it also stores what those points are on the
//! model ([`ModelRef`]: a vertex or an edge by its persistent name, with a K8
//! [`Signature`]), so it follows the model when it changes; the stored points are what it
//! falls back to, drawn as dangling, when a reference no longer resolves.
//!
//! Sheets, views, dimensions and annotations take ids from one counter
//! ([`Drawing::last_id`]), so an id names one thing on any sheet.

use crate::DocError;
use crate::naming::Signature;
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
/// Most annotations (notes, leaders, symbols, frames, centre lines, balloons) on a sheet.
pub const MAX_ANNOTATIONS: usize = 2_000;
/// Longest note or leader text, in characters.
pub const MAX_NOTE: usize = 256;
/// Most points of one leader.
pub const MAX_LEADER_POINTS: usize = 16;
/// Most datum references in one feature control frame (primary, secondary, tertiary).
pub const MAX_DATUMS: usize = 3;
/// Smallest and largest annotation text height (mm).
pub const MIN_TEXT_HEIGHT: f64 = 1.0;
pub const MAX_TEXT_HEIGHT: f64 = 50.0;
/// Largest geometric tolerance value (mm).
pub const MAX_TOLERANCE: f64 = 1e3;

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
    /// A section view: the model cut along a line on another view (format 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<SectionCut>,
    /// A detail view: part of another view inside a circle, enlarged (format 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<DetailCircle>,
}

impl SheetView {
    /// A plain view: not a section or a detail of another view.
    pub fn is_plain(&self) -> bool {
        self.section.is_none() && self.detail.is_none()
    }

    /// The view this one is made from: its base, the view its section is cut on or the
    /// view its detail enlarges.
    pub fn source(&self) -> Option<u32> {
        self.section
            .as_ref()
            .map(|c| c.from)
            .or(self.detail.as_ref().map(|d| d.from))
            .or(self.parent)
    }
}

/// Where a section view cuts the model (ISO 128-40 and ISO 128-44): a cutting line from
/// `a` to `b` drawn on view `from`, in that view's coordinates (model mm). The cutting
/// plane holds the line and that view's direction of sight. The section looks across the
/// line at the material on the side the arrows point to: the right of `a` to `b`, or the
/// left when `flip` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionCut {
    pub from: u32,
    pub a: Point2,
    pub b: Point2,
    /// The letter at both ends of the cutting line: the view is labelled `A-A`.
    pub label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// What a detail view enlarges: the circle of `radius` (model mm) about `centre` on view
/// `from`, in that view's coordinates. The detail's own scale is the view's `scale`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailCircle {
    pub from: u32,
    pub centre: Point2,
    pub radius: f64,
    /// The letter beside the circle: the view is labelled `B (2:1)`.
    pub label: String,
}

/// A view or datum label: one to three capital letters.
pub fn is_valid_label(s: &str) -> bool {
    (1..=3).contains(&s.chars().count()) && s.chars().all(|c| c.is_ascii_uppercase())
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

    /// How many model references an associative dimension stores: the two measured
    /// points (linear), the circle (diameter, radius: its centre), the two legs (angle).
    pub fn ref_count(self) -> usize {
        match self {
            DimensionKind::Diameter | DimensionKind::Radius => 1,
            _ => 2,
        }
    }

    /// The kind and point each of its references must have.
    pub fn ref_fits(self, r: &ModelRef) -> bool {
        match self {
            DimensionKind::Horizontal | DimensionKind::Vertical | DimensionKind::Aligned => {
                matches!(
                    r.at,
                    RefPoint::Point | RefPoint::Start | RefPoint::End | RefPoint::Centre
                )
            }
            DimensionKind::Diameter | DimensionKind::Radius => r.at == RefPoint::Centre,
            DimensionKind::Angular => r.at == RefPoint::Line,
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
    /// What the points are on the model (format 30), see [`DimensionKind::ref_count`];
    /// empty for a dimension that keeps its points (made before format 30, or on a point
    /// with no named vertex or edge).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<ModelRef>,
}

/// The kind of model entity a [`ModelRef`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Vertex,
    Edge,
}

/// Which point of the entity a reference stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefPoint {
    /// A vertex's position.
    Point,
    /// The start or end of an edge's curve.
    Start,
    End,
    /// The centre of a circular edge.
    Centre,
    /// A straight edge as a whole (a leg of an angle).
    Line,
}

/// A point or edge of the model that a dimension measures: a vertex or an edge by its
/// persistent name (rung K8, [`crate::naming`]), with what it was when it was picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub kind: RefKind,
    /// The unique name: `2:top 0`, `between 2:side 0 and 2:top`, `2:top 0#2`.
    pub name: String,
    pub at: RefPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl ModelRef {
    pub fn check(&self) -> Result<(), String> {
        let n = self.name.chars().count();
        if n == 0 || n > crate::naming::MAX_REFERENCE_LEN || self.name.chars().any(char::is_control)
        {
            return Err("a reference name is empty, too long or holds control characters".into());
        }
        let fits = matches!(
            (self.kind, self.at),
            (RefKind::Vertex, RefPoint::Point)
                | (
                    RefKind::Edge,
                    RefPoint::Start | RefPoint::End | RefPoint::Centre | RefPoint::Line
                )
        );
        if !fits {
            return Err(
                "a vertex reference stands for its point, an edge reference for its \
                        start, end, centre or line"
                    .into(),
            );
        }
        if let Some(s) = &self.signature {
            s.validate()?;
        }
        Ok(())
    }
}

/// A geometric characteristic of a feature control frame (ISO 1101, ASME Y14.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GdtCharacteristic {
    Straightness,
    Flatness,
    Circularity,
    Parallelism,
    Perpendicularity,
    Position,
}

impl GdtCharacteristic {
    pub const ALL: [GdtCharacteristic; 6] = [
        GdtCharacteristic::Straightness,
        GdtCharacteristic::Flatness,
        GdtCharacteristic::Circularity,
        GdtCharacteristic::Parallelism,
        GdtCharacteristic::Perpendicularity,
        GdtCharacteristic::Position,
    ];

    pub fn id(self) -> &'static str {
        match self {
            GdtCharacteristic::Straightness => "straightness",
            GdtCharacteristic::Flatness => "flatness",
            GdtCharacteristic::Circularity => "circularity",
            GdtCharacteristic::Parallelism => "parallelism",
            GdtCharacteristic::Perpendicularity => "perpendicularity",
            GdtCharacteristic::Position => "position",
        }
    }

    pub fn parse(s: &str) -> Option<GdtCharacteristic> {
        GdtCharacteristic::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Orientation tolerances are always relative to a datum.
    pub fn needs_datum(self) -> bool {
        matches!(
            self,
            GdtCharacteristic::Parallelism | GdtCharacteristic::Perpendicularity
        )
    }

    /// Form tolerances (straightness, flatness, circularity) refer to no datum.
    pub fn takes_datum(self) -> bool {
        !matches!(
            self,
            GdtCharacteristic::Straightness
                | GdtCharacteristic::Flatness
                | GdtCharacteristic::Circularity
        )
    }
}

/// What an annotation is. Positions are sheet mm unless they say otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnotationKind {
    /// Free text; `at` is the start of its baseline.
    Note {
        at: Point2,
        text: String,
        height: f64,
    },
    /// A leader: an arrow at `points[0]`, through the other points, and the text at the
    /// last one.
    Leader { points: Vec<Point2>, text: String },
    /// A surface texture symbol standing on `at`, with its requirement (`Ra 1.6`);
    /// `removal` adds the bar for material removal required.
    SurfaceFinish {
        at: Point2,
        #[serde(default)]
        text: String,
        #[serde(default)]
        removal: bool,
    },
    /// A feature control frame: its lower-left corner at `at`, with a leader from its
    /// left end to `leader` when given.
    Gdt {
        at: Point2,
        characteristic: GdtCharacteristic,
        /// mm.
        tolerance: f64,
        /// The tolerance zone is a cylinder: `Ø` before the value.
        #[serde(default)]
        diameter: bool,
        /// Datum letters, primary first.
        #[serde(default)]
        datums: Vec<String>,
        #[serde(default)]
        leader: Option<Point2>,
    },
    /// A centre line between the centres of two circles of view `view` (in its
    /// coordinates), following the model through `refs` (two circular edges at their
    /// centres) when it can.
    CentreLine {
        view: u32,
        points: [Point2; 2],
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        refs: Vec<ModelRef>,
    },
    /// A balloon with the parts-list item number of component `component` at `at`, its
    /// leader ending in a dot at `target` on view `view` (in its coordinates).
    Balloon {
        view: u32,
        component: u32,
        at: Point2,
        target: Point2,
    },
}

/// One annotation on a sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub id: u32,
    pub kind: AnnotationKind,
}

/// A parts list (bill of materials) on a sheet, its lower-right corner at `(x, y)`. Its
/// rows come from the design's components whenever the sheet is drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartsList {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

/// Note and leader text: 1 to [`MAX_NOTE`] characters, no control characters.
pub fn is_valid_note(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_NOTE && !s.chars().any(char::is_control)
}

fn sheet_point_ok(p: &Point2) -> bool {
    is_valid_sheet_coordinate(p.x) && is_valid_sheet_coordinate(p.y)
}

fn model_point_ok(p: &Point2) -> bool {
    chamfera_geom::is_valid_coordinate(p.x) && chamfera_geom::is_valid_coordinate(p.y)
}

impl AnnotationKind {
    /// The view it belongs to, if any.
    pub fn view(&self) -> Option<u32> {
        match self {
            AnnotationKind::CentreLine { view, .. } | AnnotationKind::Balloon { view, .. } => {
                Some(*view)
            }
            _ => None,
        }
    }

    /// Refuses what a sheet must not hold: bad text, positions and values.
    pub fn check(&self) -> Result<(), String> {
        let text_ok = |t: &str| {
            if is_valid_note(t) {
                Ok(())
            } else {
                Err(format!(
                    "text must hold 1 to {MAX_NOTE} characters and no control characters"
                ))
            }
        };
        let place = |p: &Point2| {
            if sheet_point_ok(p) {
                Ok(())
            } else {
                Err(format!(
                    "positions must be finite and within ±{MAX_SHEET_COORDINATE} mm"
                ))
            }
        };
        match self {
            AnnotationKind::Note { at, text, height } => {
                place(at)?;
                text_ok(text)?;
                if !(height.is_finite() && (MIN_TEXT_HEIGHT..=MAX_TEXT_HEIGHT).contains(height)) {
                    return Err(format!(
                        "the text height must be {MIN_TEXT_HEIGHT} to {MAX_TEXT_HEIGHT} mm"
                    ));
                }
            }
            AnnotationKind::Leader { points, text } => {
                if !(2..=MAX_LEADER_POINTS).contains(&points.len()) {
                    return Err(format!("a leader has 2 to {MAX_LEADER_POINTS} points"));
                }
                points.iter().try_for_each(place)?;
                text_ok(text)?;
            }
            AnnotationKind::SurfaceFinish { at, text, .. } => {
                place(at)?;
                if !text.is_empty() {
                    text_ok(text)?;
                }
            }
            AnnotationKind::Gdt {
                at,
                characteristic,
                tolerance,
                datums,
                leader,
                ..
            } => {
                place(at)?;
                if let Some(l) = leader {
                    place(l)?;
                }
                if !(tolerance.is_finite() && (0.0..=MAX_TOLERANCE).contains(tolerance)) {
                    return Err(format!(
                        "the tolerance must be a finite value from 0 to {MAX_TOLERANCE} mm"
                    ));
                }
                if datums.len() > MAX_DATUMS || datums.iter().any(|d| !is_valid_label(d)) {
                    return Err(format!(
                        "at most {MAX_DATUMS} datums, each one to three capital letters"
                    ));
                }
                if !characteristic.takes_datum() && !datums.is_empty() {
                    return Err(format!("{} takes no datum", characteristic.id()));
                }
                if characteristic.needs_datum() && datums.is_empty() {
                    return Err(format!("{} needs a datum", characteristic.id()));
                }
            }
            AnnotationKind::CentreLine { points, refs, .. } => {
                if !points.iter().all(model_point_ok) {
                    return Err("centre line points must be finite model coordinates".into());
                }
                if !(refs.is_empty() || refs.len() == 2) {
                    return Err("a centre line has two references or none".into());
                }
                refs.iter().try_for_each(ModelRef::check)?;
            }
            AnnotationKind::Balloon { at, target, .. } => {
                place(at)?;
                if !model_point_ok(target) {
                    return Err("a balloon's target must be finite model coordinates".into());
                }
            }
        }
        Ok(())
    }
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
    /// Notes, leaders, symbols, frames, centre lines and balloons (format 30).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// The parts list table, if the sheet shows one (format 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts_list: Option<PartsList>,
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
                    if p == v.id
                        || !v.is_plain()
                        || base.is_none_or(|b| b.parent.is_some() || !b.is_plain())
                    {
                        return Err(bad(format!(
                            "view {}: its base view {p} must be a plain view on the same \
                             sheet that is not itself projected",
                            v.id
                        )));
                    }
                }
                if v.section.is_some() && v.detail.is_some() {
                    return Err(bad(format!(
                        "view {}: a view is a section or a detail, not both",
                        v.id
                    )));
                }
                // A section is cut on a plain view; a detail enlarges a plain or section
                // view. So sources never chain further than two steps and never loop.
                if let Some(c) = &v.section {
                    let from = s.view(c.from);
                    let finite = [c.a.x, c.a.y, c.b.x, c.b.y]
                        .iter()
                        .all(|x| chamfera_geom::is_valid_coordinate(*x));
                    if c.from == v.id
                        || from.is_none_or(|f| !f.is_plain())
                        || !finite
                        || c.a.distance(c.b) <= 1e-6
                        || !is_valid_label(&c.label)
                    {
                        return Err(bad(format!(
                            "view {}: a section needs a plain view on the same sheet, a \
                             cutting line of two different finite points and a label of \
                             one to three capital letters",
                            v.id
                        )));
                    }
                }
                if let Some(d) = &v.detail {
                    let from = s.view(d.from);
                    let finite = [d.centre.x, d.centre.y]
                        .iter()
                        .all(|x| chamfera_geom::is_valid_coordinate(*x));
                    if d.from == v.id
                        || from.is_none_or(|f| f.detail.is_some())
                        || !finite
                        || !(d.radius.is_finite() && d.radius > 1e-6)
                        || d.radius > chamfera_geom::MAX_COORDINATE
                        || !is_valid_label(&d.label)
                    {
                        return Err(bad(format!(
                            "view {}: a detail needs a view on the same sheet that is not a \
                             detail itself, a finite centre, a positive radius and a label \
                             of one to three capital letters",
                            v.id
                        )));
                    }
                }
            }
            if s.annotations.len() > MAX_ANNOTATIONS {
                return Err(bad(format!(
                    "sheet {}: at most {MAX_ANNOTATIONS} annotations",
                    s.id
                )));
            }
            for a in &s.annotations {
                id_ok(a.id, "annotation")?;
                if let Some(view) = a.kind.view()
                    && s.view(view).is_none()
                {
                    return Err(bad(format!(
                        "annotation {}: no view {view} on sheet {}",
                        a.id, s.id
                    )));
                }
                a.kind
                    .check()
                    .map_err(|e| bad(format!("annotation {}: {e}", a.id)))?;
            }
            if let Some(p) = &s.parts_list {
                id_ok(p.id, "parts list")?;
                if !(is_valid_sheet_coordinate(p.x) && is_valid_sheet_coordinate(p.y)) {
                    return Err(bad(format!(
                        "parts list {}: its position must be finite and within ±{MAX_SHEET_COORDINATE} mm",
                        p.id
                    )));
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
                if !(d.refs.is_empty() || d.refs.len() == d.kind.ref_count())
                    || d.refs.iter().any(|r| !d.kind.ref_fits(r))
                {
                    return Err(bad(format!(
                        "dimension {}: a {} dimension has {} model references or none, of \
                         the right kind",
                        d.id,
                        d.kind.id(),
                        d.kind.ref_count()
                    )));
                }
                for r in &d.refs {
                    r.check()
                        .map_err(|e| bad(format!("dimension {}: {e}", d.id)))?;
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
            annotations: Vec::new(),
            parts_list: None,
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
            section: None,
            detail: None,
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
            refs: Vec::new(),
        });
        assert!(d.check().is_err(), "an angle needs three points");
        d.sheets[0].dimensions[0].kind = DimensionKind::Horizontal;
        assert!(d.check().is_ok());
        d.sheets[0].dimensions[0].view = 9;
        assert!(d.check().is_err(), "dimension on a missing view");
    }

    fn plain(id: u32) -> SheetView {
        SheetView {
            id,
            body: None,
            direction: ViewDirection::Front,
            x: 100.0,
            y: 100.0,
            scale: None,
            parent: None,
            section: None,
            detail: None,
        }
    }

    #[test]
    fn sections_details_references_and_annotations_are_checked() {
        let mut d = Drawing {
            sheets: vec![sheet(1)],
            last_id: 20,
        };
        let s = &mut d.sheets[0];
        s.views.push(plain(2));
        let mut sec = plain(3);
        sec.section = Some(SectionCut {
            from: 2,
            a: Point2::new(0.0, -10.0),
            b: Point2::new(0.0, 10.0),
            label: "A".into(),
            flip: false,
        });
        s.views.push(sec);
        let mut det = plain(4);
        det.scale = Some(2.0);
        det.detail = Some(DetailCircle {
            from: 3,
            centre: Point2::new(0.0, 0.0),
            radius: 5.0,
            label: "B".into(),
        });
        s.views.push(det);
        assert_eq!(d.check(), Ok(()));
        let ok = d.clone();
        // A section cut on a section, a detail of a detail, bad labels and radii.
        d.sheets[0].views[1].section.as_mut().unwrap().from = 3;
        assert!(d.check().is_err(), "a section is cut on a plain view");
        d = ok.clone();
        d.sheets[0].views[2].detail.as_mut().unwrap().from = 4;
        assert!(d.check().is_err(), "a detail of itself");
        d = ok.clone();
        d.sheets[0].views[1].section.as_mut().unwrap().label = "a".into();
        assert!(d.check().is_err(), "labels are capital letters");
        d = ok.clone();
        d.sheets[0].views[2].detail.as_mut().unwrap().radius = f64::NAN;
        assert!(d.check().is_err());
        d = ok.clone();
        d.sheets[0].views[1].section.as_mut().unwrap().b = Point2::new(0.0, -10.0);
        assert!(d.check().is_err(), "a cutting line of one point");
        d = ok.clone();
        // References: a horizontal dimension takes two point references.
        let r = ModelRef {
            kind: RefKind::Vertex,
            name: "2:top 0 x 2:side 1".into(),
            at: RefPoint::Point,
            signature: None,
        };
        d.sheets[0].dimensions.push(SheetDimension {
            id: 5,
            view: 2,
            kind: DimensionKind::Horizontal,
            points: vec![Point2::new(0.0, 0.0), Point2::new(50.0, 0.0)],
            offset: 10.0,
            refs: vec![r.clone(), r.clone()],
        });
        assert_eq!(d.check(), Ok(()));
        d.sheets[0].dimensions[0].refs.pop();
        assert!(d.check().is_err(), "one reference for two points");
        d.sheets[0].dimensions[0].refs = vec![
            ModelRef {
                at: RefPoint::Line,
                ..r.clone()
            },
            r.clone(),
        ];
        assert!(d.check().is_err(), "a vertex has no line");
        // Annotations: text, heights, datums.
        let add = |kind: AnnotationKind| {
            let mut x = ok.clone();
            x.sheets[0].annotations.push(Annotation { id: 6, kind });
            x.check()
        };
        assert_eq!(
            add(AnnotationKind::Note {
                at: Point2::new(20.0, 20.0),
                text: "ALL EDGES 0.5 x 45°".into(),
                height: 3.5,
            }),
            Ok(())
        );
        assert!(
            add(AnnotationKind::Note {
                at: Point2::new(20.0, 20.0),
                text: "a\u{7}".into(),
                height: 3.5,
            })
            .is_err()
        );
        assert!(
            add(AnnotationKind::Note {
                at: Point2::new(20.0, 20.0),
                text: "x".into(),
                height: 0.0,
            })
            .is_err()
        );
        let gdt = |c: GdtCharacteristic, datums: &[&str]| AnnotationKind::Gdt {
            at: Point2::new(50.0, 50.0),
            characteristic: c,
            tolerance: 0.05,
            diameter: false,
            datums: datums.iter().map(|x| x.to_string()).collect(),
            leader: None,
        };
        assert_eq!(add(gdt(GdtCharacteristic::Flatness, &[])), Ok(()));
        assert!(add(gdt(GdtCharacteristic::Flatness, &["A"])).is_err());
        assert!(add(gdt(GdtCharacteristic::Perpendicularity, &[])).is_err());
        assert_eq!(
            add(gdt(GdtCharacteristic::Position, &["A", "B", "C"])),
            Ok(())
        );
        assert!(add(gdt(GdtCharacteristic::Position, &["A", "B", "C", "D"])).is_err());
        assert!(
            add(AnnotationKind::Balloon {
                view: 9,
                component: 1,
                at: Point2::new(1.0, 1.0),
                target: Point2::new(0.0, 0.0),
            })
            .is_err(),
            "a balloon on a missing view"
        );
        // A view of a design before format 30 reads with none of the new fields.
        let v: SheetView =
            serde_json::from_str(r#"{"id": 2, "direction": "front", "x": 1, "y": 2}"#).unwrap();
        assert!(v.is_plain() && v.source().is_none());
    }
}

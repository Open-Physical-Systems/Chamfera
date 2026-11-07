//! `drawing.*`: 2D drawing sheets with views of the solids and dimensions.
//!
//! Sheets live in the design (`Document::drawing`); their lines are worked out from the
//! solids whenever a sheet is drawn ([`render`]), so views follow the model, and so do
//! dimensions, which keep model references. Sections, details, annotations, the parts
//! list and balloons are in `drawing/annot.rs`.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::drawing::{
    self as dd, DimensionKind, Orientation, Paper, ProjectionAngle, Sheet, SheetDimension,
    SheetView, TitleBlock, ViewDirection,
};
use chamfera_drawing::{Format, HoleNote, PartRow, SheetGeometry};
use chamfera_geom::Point2;
use serde_json::{Value, json};

mod annot;
mod dimset;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "drawing.add_balloons",
        label: "Balloons",
        params: "view: id (required) - put a balloon on every component this view shows, \
                 numbered as in the parts list; replaces the view's earlier balloons\n\
                 sheet: id (optional) - default the view's sheet",
        mutates: true,
        run: annot::add_balloons,
    },
    CommandSpec {
        id: "drawing.add_baseline",
        label: "Baseline Dimensions",
        params: "view: id (required) - the view to dimension\n\
                 kind: string (required) - horizontal or vertical\n\
                 points: [{x, y}] (required) - 2 to 64 sheet points, each near a vertex or edge \
                 of the view; the first is the baseline every other point is measured from\n\
                 offset: number (optional) - sheet mm from the points to the nearest dimension \
                 line (negative: the other side); default 10\n\
                 spacing: number (optional) - sheet mm between the stacked lines, 2 to 50; \
                 default 7\n\
                 sheet: id (optional) - default the view's sheet\n\
                 Replies with each dimension's id and value, shortest first",
        mutates: true,
        run: dimset::add_baseline,
    },
    CommandSpec {
        id: "drawing.add_centre_line",
        label: "Centre Line",
        params: "view: id (required) - the view\n\
                 x1, y1, x2, y2: number (required) - sheet mm near two circles or arcs of \
                 the view; the line joins their centres and follows the model",
        mutates: true,
        run: annot::add_centre_line,
    },
    CommandSpec {
        id: "drawing.add_chain",
        label: "Chain Dimensions",
        params: "view: id (required) - the view to dimension\n\
                 kind: string (required) - horizontal or vertical\n\
                 points: [{x, y}] (required) - 2 to 64 sheet points, each near a vertex or edge \
                 of the view; each neighbouring pair along the axis gets one dimension, all on \
                 one line\n\
                 offset: number (optional) - sheet mm from the points to the line; default 10\n\
                 sheet: id (optional) - default the view's sheet",
        mutates: true,
        run: dimset::add_chain,
    },
    CommandSpec {
        id: "drawing.add_detail",
        label: "Detail View",
        params: "view: id (required) - the view to enlarge part of (not a detail itself)\n\
                 cx, cy: number (required) - the detail circle's centre, sheet mm on that view\n\
                 radius: number (required) - the circle's radius, sheet mm on that view\n\
                 x, y: number (required) - where the detail's centre goes on the sheet\n\
                 scale: number (optional) - the detail's scale; default twice the view's\n\
                 label: string (optional) - one to three capital letters; default the next free",
        mutates: true,
        run: annot::add_detail,
    },
    CommandSpec {
        id: "drawing.add_dimension",
        label: "Add Dimension",
        params: "view: id (required) - the view to dimension\n\
                 kind: string (required) - horizontal, vertical, aligned, diameter, radius, angular, \
                 ordinate_x or ordinate_y\n\
                 x1, y1: number (required) - the first pick, sheet mm from the lower-left corner: \
                 a vertex or edge (linear; an ordinate's origin), a point on the circle \
                 (diameter, radius) or on the first edge (angular)\n\
                 x2, y2: number (required for linear, ordinate and angular) - the second vertex \
                 or edge (an ordinate's measured point; the origin itself reads 0)\n\
                 offset: number (optional) - sheet mm from the measured points to the dimension \
                 line (negative: the other side), the leader length, the arc radius, or where an \
                 ordinate's line ends from its origin (up for ordinate_x, right for ordinate_y); \
                 default 10\n\
                 sheet: id (optional) - default the view's sheet\n\
                 The dimension keeps what it measures on the model (vertices and edges by \
                 their persistent names) and follows edits; when that is gone it is shown \
                 dangling at its old place",
        mutates: true,
        run: add_dimension,
    },
    CommandSpec {
        id: "drawing.add_gdt",
        label: "Feature Control Frame",
        params: "characteristic: string (required) - straightness, flatness, circularity, \
                 parallelism, perpendicularity or position\n\
                 tolerance: number (required) - mm, 0 to 1000\n\
                 x, y: number (required) - the frame's lower-left corner, sheet mm\n\
                 diameter: bool (optional) - a cylindrical zone: Ø before the value\n\
                 datums: [string] (optional) - up to three datum letters, primary first; \
                 none for form tolerances, at least one for parallelism and perpendicularity\n\
                 leader_x, leader_y: number (optional) - a leader from the frame to this point\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: annot::add_gdt,
    },
    CommandSpec {
        id: "drawing.add_hole_callout",
        label: "Hole Callout",
        params: "view: id (required) - the view\n\
                 x1, y1: number (required) - sheet mm near the circle of a hole made by a Hole \
                 feature\n\
                 x, y: number (required) - where the callout's text goes, sheet mm\n\
                 feature: id (optional) - the hole feature, when the circle's name does not say\n\
                 The text (count, diameter, depth or THRU, counterbore or countersink) comes \
                 from the feature whenever the sheet is drawn, and the leader follows the hole",
        mutates: true,
        run: annot::add_hole_callout,
    },
    CommandSpec {
        id: "drawing.add_leader",
        label: "Leader",
        params: "points: [{x, y}] (required) - 2 to 16 sheet points: the arrow's tip first, the \
                 text at the last\n\
                 text: string (required unless thread) - 1 to 256 characters\n\
                 thread: integer (optional) - a cosmetic thread feature: its callout (such as \"M6x1 - 10 deep\") is the text\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: annot::add_leader,
    },
    CommandSpec {
        id: "drawing.add_note",
        label: "Note",
        params: "x, y: number (required) - the start of the text's baseline, sheet mm\n\
                 text: string (required) - 1 to 256 characters\n\
                 height: number (optional) - text height in mm, 1 to 50; default 3.5\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: annot::add_note,
    },
    CommandSpec {
        id: "drawing.add_ordinate",
        label: "Ordinate Dimensions",
        params: "view: id (required) - the view to dimension\n\
                 axis: string (required) - x (distances along the view's width) or y (its height)\n\
                 x0, y0: number (required) - the origin, sheet mm near a vertex or edge; it gets \
                 a 0 with the origin's circle\n\
                 points: [{x, y}] (optional) - up to 64 more sheet points, each near a vertex or \
                 edge; each gets its distance from the origin\n\
                 offset: number (optional) - where the lines end, sheet mm from the origin (up \
                 for x, right for y; negative the other way); default -10\n\
                 sheet: id (optional) - default the view's sheet",
        mutates: true,
        run: dimset::add_ordinate,
    },
    CommandSpec {
        id: "drawing.add_parts_list",
        label: "Parts List",
        params: "x, y: number (optional) - the table's lower-right corner, sheet mm; default \
                 on top of the title block\n\
                 sheet: id (optional) - default the last sheet\n\
                 Rows: item, name, quantity and material of each component; replaces the \
                 sheet's earlier parts list",
        mutates: true,
        run: annot::add_parts_list,
    },
    CommandSpec {
        id: "drawing.add_section",
        label: "Section View",
        params: "view: id (required) - the plain orthographic view to cut\n\
                 x1, y1, x2, y2: number (required) - the cutting line, sheet mm on that view\n\
                 x, y: number (required) - where the section's centre goes on the sheet\n\
                 flip: bool (optional) - look the other way across the line (default: the \
                 arrows point to the right of the line from point 1 to point 2)\n\
                 scale: number (optional) - default the view's\n\
                 label: string (optional) - one to three capital letters; default the next free",
        mutates: true,
        run: annot::add_section,
    },
    CommandSpec {
        id: "drawing.add_surface_finish",
        label: "Surface Texture",
        params: "x, y: number (required) - the symbol's point on the surface, sheet mm\n\
                 text: string (optional) - the requirement, for example \"Ra 1.6\"\n\
                 removal: bool (optional) - material removal required (default true)\n\
                 sheet: id (optional) - default the last sheet",
        mutates: true,
        run: annot::add_surface_finish,
    },
    CommandSpec {
        id: "drawing.add_view",
        label: "Add View",
        params: "view: string (required) - front, back, top, bottom, left, right or iso\n\
                 x, y: number (required) - where the model's centre goes, sheet mm from the \
                 lower-left corner; a projected view keeps only its distance from the base view\n\
                 body: string (optional) - the solid to show; default every solid\n\
                 scale: number (optional) - this view's scale; default the sheet's\n\
                 sheet: id (optional) - default the last sheet (one is made if there is none)",
        mutates: true,
        run: add_view,
    },
    CommandSpec {
        id: "drawing.delete",
        label: "Delete from Drawing",
        params: "sheet: id (required without view, dimension or annotation) - delete this sheet\n\
                 view: id (optional) - delete this view, its dimensions and annotations, and the \
                 views projected, sectioned or detailed from it\n\
                 dimension: id (optional) - delete this dimension\n\
                 annotation: id (optional) - delete this annotation or parts list",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "drawing.edit_sheet",
        label: "Sheet Settings",
        params: "sheet: id (optional) - default the last sheet\n\
                 and any of the drawing.new_sheet parameters",
        mutates: true,
        run: edit_sheet,
    },
    CommandSpec {
        id: "drawing.export",
        label: "Export Drawing",
        params: "path: string (required) - where to write the file\n\
                 format: string (optional) - svg, dxf or pdf; default from the path's extension\n\
                 sheet: id (optional) - default the last sheet",
        mutates: false,
        run: export,
    },
    CommandSpec {
        id: "drawing.export_bytes",
        label: "Export Drawing",
        params: "format: string (required) - svg, dxf or pdf\n\
                 sheet: id (optional) - default the last sheet\n\
                 Replies with the file as base64 `data`; writes no file",
        mutates: false,
        run: export_bytes,
    },
    CommandSpec {
        id: "drawing.list",
        label: "List Sheets",
        params: "(none) - every sheet with its views (and where they are on the sheet) and \
                 dimensions (with their values)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "drawing.move",
        label: "Move on Drawing",
        params: "view: id (optional) - move this view (views projected from it follow; a \
                 projected view moves only along its projection)\n\
                 dimension: id (optional) - move this dimension's line, leader or arc (its \
                 offset changes; what it measures stays)\n\
                 annotation: id (optional) - move this note, symbol, frame, balloon, leader \
                 text or parts list (what it points at stays)\n\
                 dx, dy: number (required) - sheet mm",
        mutates: true,
        run: move_item,
    },
    CommandSpec {
        id: "drawing.new_sheet",
        label: "New Sheet",
        params: "paper: string (optional) - a4, a3 (default), a2, ansi_a or ansi_b\n\
                 orientation: string (optional) - landscape (default) or portrait\n\
                 scale: number (optional) - sheet mm per model mm, default 1 (0.5 is 1:2)\n\
                 projection: string (optional) - third (default) or first angle\n\
                 name: string (optional) - the sheet's name\n\
                 part_name, drawing_number, revision, material, drawn_by, checked_by, date: \
                 string (optional) - title block fields, at most 64 characters; the date \
                 defaults to today",
        mutates: true,
        run: new_sheet,
    },
    CommandSpec {
        id: "drawing.pick",
        label: "Pick on Drawing",
        params: "x, y: number (required) - sheet mm from the lower-left corner\n\
                 tolerance: number (optional) - how near a dimension or annotation must be, \
                 0 to 50 mm, default 2\n\
                 sheet: id (optional) - default the last sheet\n\
                 Replies with `kind` (view, dimension, annotation or null) and `id`",
        mutates: false,
        run: pick,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn opt_id(p: &Value, key: &str) -> Result<Option<u32>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_id(p, key).map(Some),
    }
}

fn text_param(p: &Value, key: &str) -> Result<Option<String>, CommandError> {
    match params::opt_str(p, key)? {
        None => Ok(None),
        Some(t) if dd::is_valid_text(t) => Ok(Some(t.to_owned())),
        Some(_) => Err(bad(format!(
            "`{key}` holds at most {} characters and no control characters",
            dd::MAX_TEXT
        ))),
    }
}

fn scale_param(p: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match params::opt_f64(p, key)? {
        None => Ok(None),
        Some(k) if dd::is_valid_scale(k) => Ok(Some(k)),
        Some(_) => Err(bad(format!(
            "`{key}` must be between {} and {}",
            dd::MIN_SCALE,
            dd::MAX_SCALE
        ))),
    }
}

fn coordinate(p: &Value, key: &str) -> Result<f64, CommandError> {
    let v = params::req_f64(p, key)?;
    if dd::is_valid_sheet_coordinate(v) {
        Ok(v)
    } else {
        Err(bad(format!(
            "`{key}` must be within ±{} mm",
            dd::MAX_SHEET_COORDINATE
        )))
    }
}

/// The sheet a command names, or the last one.
fn sheet_id(s: &Session, p: &Value) -> Result<u32, CommandError> {
    match opt_id(p, "sheet")? {
        Some(id) => Ok(s.doc.drawing.sheet(id)?.id),
        None => s.doc.drawing.sheets.last().map(|x| x.id).ok_or_else(|| {
            CommandError::Unavailable("there are no sheets: make one with drawing.new_sheet".into())
        }),
    }
}

/// Today as `YYYY-MM-DD` where the computer is: the date in the time zone the system is
/// set to (UTC when it has none); empty in the browser.
fn today() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let zone = jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC);
        date_in(jiff::Timestamp::now(), zone)
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

/// The calendar date of an instant in a time zone, as `YYYY-MM-DD`.
#[cfg(not(target_arch = "wasm32"))]
fn date_in(at: jiff::Timestamp, zone: jiff::tz::TimeZone) -> String {
    let d = at.to_zoned(zone).date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

/// Applies the sheet settings in `p` to `sheet`.
fn apply_settings(sheet: &mut Sheet, p: &Value) -> Result<(), CommandError> {
    if let Some(v) = params::opt_str(p, "paper")? {
        sheet.paper = Paper::parse(v).ok_or_else(|| {
            bad(format!(
                "unknown paper {v:?}: use a4, a3, a2, ansi_a or ansi_b"
            ))
        })?;
    }
    if let Some(v) = params::opt_str(p, "orientation")? {
        sheet.orientation = match v {
            "landscape" => Orientation::Landscape,
            "portrait" => Orientation::Portrait,
            _ => return Err(bad("`orientation` is landscape or portrait")),
        };
    }
    if let Some(k) = scale_param(p, "scale")? {
        sheet.scale = k;
    }
    if let Some(v) = params::opt_str(p, "projection")? {
        sheet.projection = match v {
            "first" => ProjectionAngle::First,
            "third" => ProjectionAngle::Third,
            _ => return Err(bad("`projection` is first or third")),
        };
    }
    if let Some(t) = text_param(p, "name")? {
        sheet.name = t;
    }
    for name in TitleBlock::NAMES {
        if let Some(t) = text_param(p, name)?
            && let Some(f) = sheet.title.field_mut(name)
        {
            *f = t;
        }
    }
    Ok(())
}

fn make_sheet(s: &mut Session, p: &Value) -> Result<u32, CommandError> {
    if s.doc.drawing.sheets.len() >= dd::MAX_SHEETS {
        return Err(bad(format!(
            "a design holds at most {} sheets",
            dd::MAX_SHEETS
        )));
    }
    let n = s.doc.drawing.sheets.len() + 1;
    let part = s
        .path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|n| {
            n.to_string_lossy()
                .chars()
                .take(dd::MAX_TEXT)
                .collect::<String>()
        })
        .filter(|n| dd::is_valid_text(n))
        .unwrap_or_default();
    let mut sheet = Sheet {
        id: 0,
        name: format!("Sheet {n}"),
        paper: Paper::A3,
        orientation: Orientation::Landscape,
        scale: 1.0,
        projection: ProjectionAngle::Third,
        title: TitleBlock {
            part_name: part,
            drawn_by: String::new(),
            date: today(),
            ..Default::default()
        },
        views: Vec::new(),
        dimensions: Vec::new(),
        annotations: Vec::new(),
        parts_list: None,
    };
    apply_settings(&mut sheet, p)?;
    sheet.id = s.doc.drawing.next_id()?;
    let id = sheet.id;
    s.doc.drawing.sheets.push(sheet);
    Ok(id)
}

fn new_sheet(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let id = make_sheet(s, p)?;
    let sheet = s.doc.drawing.sheet(id)?;
    let (w, h) = sheet.size();
    Ok(json!({ "sheet": id, "width": w, "height": h }))
}

fn edit_sheet(s: &mut Session, p: &Value) -> CmdResult {
    let id = sheet_id(s, p)?;
    let sheet = s.doc.drawing.sheet_mut(id)?;
    apply_settings(sheet, p)?;
    let (w, h) = sheet.size();
    Ok(json!({
        "sheet": id,
        "width": w,
        "height": h,
        "paper": sheet.paper.id(),
        "orientation": sheet.orientation,
        "scale": sheet.scale,
        "projection": sheet.projection,
        "name": sheet.name,
        "title": sheet.title,
    }))
}

fn add_view(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "view")?;
    let direction = ViewDirection::parse(name).ok_or_else(|| {
        bad(format!(
            "unknown view {name:?}: use front, back, top, bottom, left, right or iso"
        ))
    })?;
    let x = coordinate(p, "x")?;
    let y = coordinate(p, "y")?;
    let scale = scale_param(p, "scale")?;
    let body = params::opt_str(p, "body")?.map(str::to_owned);
    match &body {
        Some(b) if !s.solids().iter().any(|x| &x.name == b) => {
            return Err(CommandError::Unavailable(format!(
                "there is no solid named {b:?}"
            )));
        }
        None if s.solids().is_empty() => {
            return Err(CommandError::Unavailable(
                "there are no solids to draw".into(),
            ));
        }
        _ => {}
    }
    let sheet = match opt_id(p, "sheet")? {
        Some(id) => s.doc.drawing.sheet(id)?.id,
        None => match s.doc.drawing.sheets.last() {
            Some(x) => x.id,
            None => make_sheet(s, &json!({}))?,
        },
    };
    let id = s.doc.drawing.next_id()?;
    let sh = s.doc.drawing.sheet_mut(sheet)?;
    if sh.views.len() >= dd::MAX_VIEWS {
        return Err(bad(format!(
            "a sheet holds at most {} views",
            dd::MAX_VIEWS
        )));
    }
    // A view that can be projected from an earlier base view of the same body is.
    let parent = sh
        .views
        .iter()
        .find(|v| {
            v.parent.is_none()
                && v.body == body
                && chamfera_drawing::projects_from(v.direction, direction)
        })
        .map(|v| v.id);
    sh.views.push(SheetView {
        id,
        body,
        direction,
        x,
        y,
        scale: if parent.is_some() { None } else { scale },
        parent,
        section: None,
        detail: None,
    });
    Ok(json!({ "sheet": sheet, "view": id, "parent": parent }))
}

/// The sheet holding view `view`.
fn sheet_of_view(s: &Session, view: u32) -> Result<u32, CommandError> {
    s.doc
        .drawing
        .sheets
        .iter()
        .find(|x| x.view(view).is_some())
        .map(|x| x.id)
        .ok_or_else(|| bad(format!("no view {view}")))
}

/// Lays out sheet `id` with the current solids.
pub fn render(s: &Session, id: u32) -> Result<SheetGeometry, CommandError> {
    let sheet = s.doc.drawing.sheet(id)?;
    chamfera_drawing::render_full(sheet, &bodies(s), &parts(s), &hole_notes(s))
        .map_err(|e| CommandError::Unavailable(e.to_string()))
}

/// A length on a drawing: two decimals at most, trailing zeros dropped.
fn mm(v: f64) -> String {
    chamfera_drawing::format_value(DimensionKind::Horizontal, v)
}

/// What a hole callout says about each built hole feature: `4 x Ø6.6 THRU` or
/// `Ø5 x 10 DEEP`, then `CBORE Ø11 x 6.4 DEEP` or `CSK Ø12 x 90°`. Suppressed holes and
/// holes whose sizes do not evaluate have none.
pub fn hole_notes(s: &Session) -> Vec<HoleNote> {
    let mut out = Vec::new();
    for f in &s.doc.features {
        let chamfera_doc::FeatureKind::Hole(h) = &f.kind else {
            continue;
        };
        if f.suppressed {
            continue;
        }
        let Ok(z) = s.doc.hole_sizes(h) else {
            continue;
        };
        let from_sketch = h
            .sketch
            .and_then(|k| s.doc.sketch(k).ok())
            .map_or(0, |k| crate::hole::sketch_hole_points(&k.sketch).len());
        let n = h.points.len() + from_sketch;
        let count = if n > 1 {
            format!("{n} x ")
        } else {
            String::new()
        };
        let depth = match z.depth {
            None => "THRU".to_owned(),
            Some(d) => format!("x {} DEEP", mm(d)),
        };
        let mut lines = vec![format!("{count}Ø{} {depth}", mm(z.diameter))];
        if let Some((d, depth)) = z.counterbore {
            lines.push(format!("CBORE Ø{} x {} DEEP", mm(d), mm(depth)));
        }
        if let Some((d, angle)) = z.countersink {
            lines.push(format!(
                "CSK Ø{} x {}",
                mm(d),
                chamfera_drawing::format_value(DimensionKind::Angular, angle.to_degrees())
            ));
        }
        out.push(HoleNote {
            feature: f.id,
            lines,
        });
    }
    out
}

/// The solids by name, as the drawing crate takes them.
fn bodies(s: &Session) -> Vec<(&str, &chamfera_kernel::Body)> {
    s.solids()
        .iter()
        .map(|x| (x.name.as_str(), &x.body))
        .collect()
}

/// The parts list of the design: one row per component that is not a linked copy, its
/// quantity counting the copies, its material the named appearance of its first feature
/// that has one.
pub fn parts(s: &Session) -> Vec<PartRow> {
    let a = &s.doc.assembly;
    a.components
        .iter()
        .filter(|c| c.occurrence_of.is_none())
        .map(|c| {
            let mut members = vec![c.id];
            members.extend(
                a.components
                    .iter()
                    .filter(|o| o.occurrence_of == Some(c.id))
                    .map(|o| o.id),
            );
            let material = c
                .features
                .iter()
                .find_map(|f| s.doc.appearances.get(f).and_then(|x| x.preset.clone()))
                .unwrap_or_default();
            PartRow {
                quantity: u32::try_from(members.len()).unwrap_or(u32::MAX),
                components: members,
                name: c.name.clone(),
                material,
            }
        })
        .collect()
}

fn add_dimension(s: &mut Session, p: &Value) -> CmdResult {
    let view = params::req_id(p, "view")?;
    let sheet = match opt_id(p, "sheet")? {
        Some(id) => id,
        None => sheet_of_view(s, view)?,
    };
    let kind_name = params::req_str(p, "kind")?;
    let kind = DimensionKind::parse(kind_name).ok_or_else(|| {
        bad(format!(
            "unknown kind {kind_name:?}: use horizontal, vertical, aligned, diameter, radius, \
             angular, ordinate_x or ordinate_y"
        ))
    })?;
    let mut picks = vec![Point2::new(coordinate(p, "x1")?, coordinate(p, "y1")?)];
    if !matches!(kind, DimensionKind::Diameter | DimensionKind::Radius) {
        picks.push(Point2::new(coordinate(p, "x2")?, coordinate(p, "y2")?));
    }
    let offset = match params::opt_f64(p, "offset")? {
        None => 10.0,
        Some(v) if dd::is_valid_sheet_coordinate(v) => v,
        Some(_) => return Err(bad("`offset` must be a finite number of mm")),
    };
    let mut made = push_dimensions(s, sheet, view, &[(kind, picks, offset)])?;
    let mut reply = made.pop().unwrap_or(Value::Null);
    reply["sheet"] = json!(sheet);
    Ok(reply)
}

/// Plans dimensions on view `view` of `sheet` from picks (sheet mm), each a kind, its
/// picks and its offset, and adds them all, or none when one fails. Replies with each
/// one's `dimension` id, `value`, `text` and `associative`.
pub(super) fn push_dimensions(
    s: &mut Session,
    sheet: u32,
    view: u32,
    specs: &[(DimensionKind, Vec<Point2>, f64)],
) -> Result<Vec<Value>, CommandError> {
    let geometry = render(s, sheet)?;
    let placed = geometry
        .view(view)
        .ok_or_else(|| bad(format!("no view {view} on sheet {sheet}")))?;
    if let Some(problem) = &placed.problem {
        return Err(CommandError::Unavailable(format!("view {view}: {problem}")));
    }
    let mut planned = Vec::new();
    for (kind, picks, offset) in specs {
        let plan = chamfera_drawing::plan_dimension(placed, *kind, picks)
            .map_err(|e| CommandError::Unavailable(e.to_string()))?;
        let refs = chamfera_drawing::dimension_refs(
            s.doc.drawing.sheet(sheet)?,
            &bodies(s),
            view,
            *kind,
            &plan.points,
        );
        planned.push((plan, refs, *offset));
    }
    if s.doc.drawing.sheet(sheet)?.dimensions.len() + planned.len() > dd::MAX_DIMENSIONS {
        return Err(bad(format!(
            "a sheet holds at most {} dimensions",
            dd::MAX_DIMENSIONS
        )));
    }
    let mut out = Vec::new();
    for (plan, refs, offset) in planned {
        let id = s.doc.drawing.next_id()?;
        let associative = !refs.is_empty();
        s.doc
            .drawing
            .sheet_mut(sheet)?
            .dimensions
            .push(SheetDimension {
                id,
                view,
                kind: plan.kind,
                points: plan.points.clone(),
                offset,
                refs,
            });
        out.push(json!({
            "dimension": id,
            "value": plan.value,
            "text": chamfera_drawing::format_value(plan.kind, plan.value),
            "associative": associative,
        }));
    }
    Ok(out)
}

/// `v`, with -0 (an empty sum) as 0.
fn positive_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// The sheet that holds dimension `id`.
fn sheet_of_dimension(s: &Session, id: u32) -> Result<u32, CommandError> {
    s.doc
        .drawing
        .sheets
        .iter()
        .find(|x| x.dimensions.iter().any(|d| d.id == id))
        .map(|x| x.id)
        .ok_or_else(|| bad(format!("no dimension {id}")))
}

/// A point moved by `(dx, dy)`, refused when it would leave the sheet's range.
fn shifted(p: Point2, dx: f64, dy: f64) -> Result<Point2, CommandError> {
    let q = Point2::new(p.x + dx, p.y + dy);
    if dd::is_valid_sheet_coordinate(q.x) && dd::is_valid_sheet_coordinate(q.y) {
        Ok(q)
    } else {
        Err(bad(format!(
            "that moves it beyond ±{} mm",
            dd::MAX_SHEET_COORDINATE
        )))
    }
}

fn move_item(s: &mut Session, p: &Value) -> CmdResult {
    let (dx, dy) = (coordinate(p, "dx")?, coordinate(p, "dy")?);
    if let Some(d) = opt_id(p, "dimension")? {
        let sheet = sheet_of_dimension(s, d)?;
        let g = render(s, sheet)?;
        let sh = s.doc.drawing.sheet(sheet)?;
        let dim = sh
            .dimensions
            .iter()
            .find(|x| x.id == d)
            .ok_or_else(|| bad(format!("no dimension {d}")))?;
        let view = g
            .view(dim.view)
            .ok_or_else(|| bad(format!("dimension {d} has no view on its sheet")))?;
        let offset = chamfera_drawing::moved_offset(dim, view, chamfera_geom::Vec2::new(dx, dy))
            .filter(|o| dd::is_valid_sheet_coordinate(*o))
            .ok_or_else(|| bad(format!("dimension {d} cannot be moved there")))?;
        let sh = s.doc.drawing.sheet_mut(sheet)?;
        if let Some(x) = sh.dimensions.iter_mut().find(|x| x.id == d) {
            x.offset = offset;
        }
        return Ok(json!({ "moved": "dimension", "id": d, "offset": offset }));
    }
    if let Some(a) = opt_id(p, "annotation")? {
        for sh in &mut s.doc.drawing.sheets {
            if let Some(pl) = sh.parts_list.as_mut().filter(|x| x.id == a) {
                let q = shifted(Point2::new(pl.x, pl.y), dx, dy)?;
                (pl.x, pl.y) = (q.x, q.y);
                return Ok(json!({ "moved": "annotation", "id": a, "x": q.x, "y": q.y }));
            }
            let Some(ann) = sh.annotations.iter_mut().find(|x| x.id == a) else {
                continue;
            };
            use chamfera_doc::drawing::AnnotationKind as K;
            // The point that places it: a note's, symbol's, frame's or balloon's
            // anchor, a leader's text end. What it points at on a view stays.
            let at = match &mut ann.kind {
                K::Note { at, .. }
                | K::SurfaceFinish { at, .. }
                | K::Gdt { at, .. }
                | K::Balloon { at, .. }
                | K::HoleCallout { at, .. } => at,
                K::Leader { points, .. } => points
                    .last_mut()
                    .ok_or_else(|| bad(format!("leader {a} has no points")))?,
                K::CentreLine { .. } => {
                    return Err(bad(
                        "a centre line joins its circles' centres and follows the model; \
                         it cannot be moved",
                    ));
                }
            };
            let q = shifted(*at, dx, dy)?;
            *at = q;
            return Ok(json!({ "moved": "annotation", "id": a, "x": q.x, "y": q.y }));
        }
        return Err(bad(format!("no annotation or parts list {a}")));
    }
    let v = params::req_id(p, "view")?;
    let sheet = sheet_of_view(s, v)?;
    let sh = s.doc.drawing.sheet_mut(sheet)?;
    let view = sh
        .views
        .iter_mut()
        .find(|x| x.id == v)
        .ok_or_else(|| bad(format!("no view {v}")))?;
    let q = shifted(Point2::new(view.x, view.y), dx, dy)?;
    (view.x, view.y) = (q.x, q.y);
    Ok(json!({ "moved": "view", "id": v, "x": q.x, "y": q.y }))
}

fn pick(s: &mut Session, p: &Value) -> CmdResult {
    let at = Point2::new(coordinate(p, "x")?, coordinate(p, "y")?);
    let tolerance = match params::opt_f64(p, "tolerance")? {
        None => 2.0,
        Some(t) if t.is_finite() && (0.0..=50.0).contains(&t) => t,
        Some(_) => return Err(bad("`tolerance` must be 0 to 50 mm")),
    };
    let sheet = sheet_id(s, p)?;
    let g = render(s, sheet)?;
    Ok(match g.pick(at, tolerance) {
        Some(o) => {
            let (kind, id) = o.key();
            json!({ "kind": kind, "id": id, "sheet": sheet })
        }
        None => json!({ "kind": null, "sheet": sheet }),
    })
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    if let Some(d) = opt_id(p, "dimension")? {
        for sh in &mut s.doc.drawing.sheets {
            if let Some(i) = sh.dimensions.iter().position(|x| x.id == d) {
                sh.dimensions.remove(i);
                return Ok(json!({ "deleted": d }));
            }
        }
        return Err(bad(format!("no dimension {d}")));
    }
    if let Some(a) = opt_id(p, "annotation")? {
        for sh in &mut s.doc.drawing.sheets {
            if let Some(i) = sh.annotations.iter().position(|x| x.id == a) {
                sh.annotations.remove(i);
                return Ok(json!({ "deleted": a }));
            }
            if sh.parts_list.as_ref().is_some_and(|x| x.id == a) {
                sh.parts_list = None;
                return Ok(json!({ "deleted": a }));
            }
        }
        return Err(bad(format!("no annotation or parts list {a}")));
    }
    if let Some(v) = opt_id(p, "view")? {
        let sheet = sheet_of_view(s, v)?;
        let sh = s.doc.drawing.sheet_mut(sheet)?;
        // The view, and every view made from it: projected, sectioned, detailed (two
        // steps at most, see `Drawing::check`).
        let mut gone: Vec<u32> = vec![v];
        for _ in 0..3 {
            for x in &sh.views {
                if !gone.contains(&x.id) && x.source().is_some_and(|f| gone.contains(&f)) {
                    gone.push(x.id);
                }
            }
        }
        sh.views.retain(|x| !gone.contains(&x.id));
        sh.dimensions.retain(|d| !gone.contains(&d.view));
        sh.annotations
            .retain(|a| a.kind.view().is_none_or(|w| !gone.contains(&w)));
        return Ok(json!({ "deleted": gone }));
    }
    let id = params::req_id(p, "sheet")?;
    let i = s
        .doc
        .drawing
        .sheets
        .iter()
        .position(|x| x.id == id)
        .ok_or_else(|| bad(format!("no sheet {id}")))?;
    s.doc.drawing.sheets.remove(i);
    Ok(json!({ "deleted": id }))
}

fn format_of(p: &Value, path: Option<&str>) -> Result<Format, CommandError> {
    match params::opt_str(p, "format")? {
        Some(f) => Format::parse(f)
            .ok_or_else(|| bad(format!("unknown format {f:?}: use svg, dxf or pdf"))),
        None => path
            .and_then(|x| std::path::Path::new(x).extension())
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .and_then(|e| Format::parse(&e))
            .ok_or_else(|| bad("give `format` (svg, dxf or pdf) or a path ending in one")),
    }
}

fn export(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let format = format_of(p, Some(path))?;
    let id = sheet_id(s, p)?;
    let bytes = chamfera_drawing::export(&render(s, id)?, format);
    std::fs::write(path, &bytes).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "sheet": id, "format": format.extension(), "bytes": bytes.len() }))
}

fn export_bytes(s: &mut Session, p: &Value) -> CmdResult {
    let format = format_of(p, None)?;
    let id = sheet_id(s, p)?;
    let bytes = chamfera_drawing::export(&render(s, id)?, format);
    Ok(json!({
        "sheet": id,
        "format": format.extension(),
        "bytes": bytes.len(),
        "data": super::design::encode_base64(&bytes),
    }))
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let mut sheets = Vec::new();
    let notes = hole_notes(s);
    for sheet in &s.doc.drawing.sheets {
        let geometry = render(s, sheet.id).ok();
        let (w, h) = sheet.size();
        let views: Vec<Value> = sheet
            .views
            .iter()
            .map(|v| {
                let placed = geometry.as_ref().and_then(|g| g.view(v.id));
                json!({
                    "id": v.id,
                    "view": v.direction.id(),
                    "body": v.body,
                    "parent": v.parent,
                    "section": v.section,
                    "detail": v.detail,
                    "hatched_area": placed.map(|pv| pv.geometry.regions.iter()
                        .map(|r| chamfera_drawing::section::region_area(r)).sum::<f64>()).map(positive_zero),
                    "x": placed.map_or(v.x, |pv| pv.centre.x),
                    "y": placed.map_or(v.y, |pv| pv.centre.y),
                    "scale": placed.map(|pv| pv.scale),
                    "bounds": placed.and_then(|pv| pv.bounds()).map(|(lo, hi)| [lo.x, lo.y, hi.x, hi.y]),
                    "visible_curves": placed.map(|pv| pv.geometry.curves.iter().filter(|c| !c.hidden).count()),
                    "hidden_curves": placed.map(|pv| pv.geometry.curves.iter().filter(|c| c.hidden).count()),
                    "problem": placed.and_then(|pv| pv.problem.clone()),
                })
            })
            .collect();
        let dims: Vec<Value> = sheet
            .dimensions
            .iter()
            .map(|d| {
                let state = geometry
                    .as_ref()
                    .and_then(|g| g.dimensions.iter().find(|x| x.id == d.id));
                let value = state
                    .and_then(|x| x.value)
                    .or_else(|| chamfera_drawing::measure(d.kind, &d.points));
                json!({
                    "id": d.id,
                    "view": d.view,
                    "kind": d.kind.id(),
                    "value": value,
                    "text": value.map(|v| chamfera_drawing::format_value(d.kind, v)),
                    "associative": !d.refs.is_empty(),
                    "dangling": state.and_then(|x| x.dangling.clone()),
                })
            })
            .collect();
        let annotations: Vec<Value> = sheet
            .annotations
            .iter()
            .map(|a| {
                let dangling = geometry.as_ref().and_then(|g| {
                    g.dangling_annotations
                        .iter()
                        .find(|x| x.0 == a.id)
                        .map(|x| x.1.clone())
                });
                let text = match &a.kind {
                    dd::AnnotationKind::HoleCallout { feature, .. } => notes
                        .iter()
                        .find(|n| n.feature == *feature)
                        .map(|n| json!(n.lines)),
                    _ => None,
                };
                json!({ "id": a.id, "annotation": a.kind, "dangling": dangling, "text": text })
            })
            .collect();
        let parts: Vec<Value> = match (&sheet.parts_list, geometry.as_ref()) {
            (Some(_), Some(g)) => g
                .parts
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    json!({
                        "item": i + 1,
                        "name": r.name,
                        "quantity": r.quantity,
                        "material": r.material,
                        "components": r.components,
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        sheets.push(json!({
            "id": sheet.id,
            "name": sheet.name,
            "paper": sheet.paper.id(),
            "orientation": sheet.orientation,
            "width": w,
            "height": h,
            "scale": sheet.scale,
            "projection": sheet.projection,
            "title": sheet.title,
            "views": views,
            "dimensions": dims,
            "annotations": annotations,
            "parts_list": sheet.parts_list.as_ref().map(|pl| json!({
                "id": pl.id, "x": pl.x, "y": pl.y, "rows": parts,
            })),
        }));
    }
    Ok(json!({ "sheets": sheets }))
}

#[cfg(test)]
mod tests {
    use super::date_in;

    #[test]
    fn the_title_block_date_is_the_local_date() {
        // QA user guide pass: a sheet made at 02:13 on 2026-11-07 at UTC+7 was dated
        // 2026-11-06, the UTC date. 02:13 at UTC+7 is 19:13 UTC the day before.
        let at: jiff::Timestamp = "2026-11-06T19:13:00Z".parse().unwrap();
        let plus7 = jiff::tz::TimeZone::fixed(jiff::tz::offset(7));
        assert_eq!(date_in(at, plus7), "2026-11-07");
        assert_eq!(date_in(at, jiff::tz::TimeZone::UTC), "2026-11-06");
        // West of Greenwich the date can be a day behind UTC: 03:00 UTC on 1 January
        // is still 31 December at UTC-5.
        let at: jiff::Timestamp = "2027-01-01T03:00:00Z".parse().unwrap();
        let minus5 = jiff::tz::TimeZone::fixed(jiff::tz::offset(-5));
        assert_eq!(date_in(at, minus5), "2026-12-31");
    }
}

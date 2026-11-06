//! `drawing.*`: 2D drawing sheets with views of the solids and dimensions.
//!
//! Sheets live in the design (`Document::drawing`); their lines are worked out from the
//! solids whenever a sheet is drawn ([`render`]), so views follow the model.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::drawing::{
    self as dd, DimensionKind, Orientation, Paper, ProjectionAngle, Sheet, SheetDimension,
    SheetView, TitleBlock, ViewDirection,
};
use chamfera_drawing::{Format, SheetGeometry};
use chamfera_geom::Point2;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "drawing.add_dimension",
        label: "Add Dimension",
        params: "view: id (required) - the view to dimension\n\
                 kind: string (required) - horizontal, vertical, aligned, diameter, radius or angular\n\
                 x1, y1: number (required) - the first pick, sheet mm from the lower-left corner: \
                 a vertex or edge (linear), a point on the circle (diameter, radius) or on the \
                 first edge (angular)\n\
                 x2, y2: number (required for linear and angular) - the second vertex or edge\n\
                 offset: number (optional) - sheet mm from the measured points to the dimension \
                 line (negative: the other side), the leader length, or the arc radius; default 10\n\
                 sheet: id (optional) - default the view's sheet",
        mutates: true,
        run: add_dimension,
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
        params: "sheet: id (required without view or dimension) - delete this sheet\n\
                 view: id (optional) - delete this view, its dimensions and views projected from it\n\
                 dimension: id (optional) - delete this dimension",
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
        id: "drawing.new_sheet",
        label: "New Sheet",
        params: "paper: string (optional) - a4, a3 (default), a2, ansi_a or ansi_b\n\
                 orientation: string (optional) - landscape (default) or portrait\n\
                 scale: number (optional) - sheet mm per model mm, default 1 (0.5 is 1:2)\n\
                 projection: string (optional) - third (default) or first angle\n\
                 name: string (optional) - the sheet's name\n\
                 part_name, drawn_by, date: string (optional) - title block fields; the date \
                 defaults to today",
        mutates: true,
        run: new_sheet,
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

/// Today as `YYYY-MM-DD` (UTC), from the system clock; empty in the browser.
fn today() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = i64::try_from(secs / 86_400).unwrap_or(0);
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}-{m:02}-{d:02}")
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

/// The proleptic Gregorian date of a day count from 1970-01-01 (the well-known
/// era-based conversion: 400-year eras of 146097 days).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
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
    if let Some(t) = text_param(p, "part_name")? {
        sheet.title.part_name = t;
    }
    if let Some(t) = text_param(p, "drawn_by")? {
        sheet.title.drawn_by = t;
    }
    if let Some(t) = text_param(p, "date")? {
        sheet.title.date = t;
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
        },
        views: Vec::new(),
        dimensions: Vec::new(),
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
    Ok(json!({ "sheet": id, "width": w, "height": h }))
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
    let bodies: Vec<(&str, &chamfera_kernel::Body)> = s
        .solids()
        .iter()
        .map(|x| (x.name.as_str(), &x.body))
        .collect();
    chamfera_drawing::render_sheet(sheet, &bodies)
        .map_err(|e| CommandError::Unavailable(e.to_string()))
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
            "unknown kind {kind_name:?}: use horizontal, vertical, aligned, diameter, radius or angular"
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
    let geometry = render(s, sheet)?;
    let placed = geometry
        .view(view)
        .ok_or_else(|| bad(format!("no view {view} on sheet {sheet}")))?;
    if let Some(problem) = &placed.problem {
        return Err(CommandError::Unavailable(format!("view {view}: {problem}")));
    }
    let plan = chamfera_drawing::plan_dimension(placed, kind, &picks)
        .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    let id = s.doc.drawing.next_id()?;
    let sh = s.doc.drawing.sheet_mut(sheet)?;
    if sh.dimensions.len() >= dd::MAX_DIMENSIONS {
        return Err(bad(format!(
            "a sheet holds at most {} dimensions",
            dd::MAX_DIMENSIONS
        )));
    }
    sh.dimensions.push(SheetDimension {
        id,
        view,
        kind,
        points: plan.points.clone(),
        offset,
    });
    Ok(json!({
        "sheet": sheet,
        "dimension": id,
        "value": plan.value,
        "text": chamfera_drawing::format_value(kind, plan.value),
    }))
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
    if let Some(v) = opt_id(p, "view")? {
        let sheet = sheet_of_view(s, v)?;
        let sh = s.doc.drawing.sheet_mut(sheet)?;
        let gone: Vec<u32> = sh
            .views
            .iter()
            .filter(|x| x.id == v || x.parent == Some(v))
            .map(|x| x.id)
            .collect();
        sh.views.retain(|x| !gone.contains(&x.id));
        sh.dimensions.retain(|d| !gone.contains(&d.view));
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
                let value = chamfera_drawing::measure(d.kind, &d.points);
                json!({
                    "id": d.id,
                    "view": d.view,
                    "kind": d.kind.id(),
                    "value": value,
                    "text": value.map(|v| chamfera_drawing::format_value(d.kind, v)),
                })
            })
            .collect();
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
        }));
    }
    Ok(json!({ "sheets": sheets }))
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn day_counts_become_dates() {
        // 1970-01-01 is day 0; 2000-03-01 is day 11017 (30 years of 365 days, 7 leap
        // days 1972..1996, plus 31 + 29 days of 2000); 2026-11-06 is day 20763.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(20_763), (2026, 11, 6));
    }
}

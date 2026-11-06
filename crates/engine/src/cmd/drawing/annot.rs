//! `drawing.add_section`, `drawing.add_detail`, the annotation commands, the parts list
//! and balloons. Positions given in sheet mm are turned into a view's own coordinates
//! here, so what is stored follows the view when it moves.

use super::{bad, bodies, coordinate, opt_id, parts, render, scale_param, sheet_of_view};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::drawing::{
    self as dd, Annotation, AnnotationKind, DetailCircle, GdtCharacteristic, PartsList, SectionCut,
    Sheet, SheetView, ViewDirection,
};
use chamfera_drawing::{Basis, Curve2, PlacedView};
use chamfera_geom::{Point2, Point3};
use serde_json::{Value, json};

/// How near a pick must be to a circle for a centre line (sheet mm).
const PICK: f64 = 3.0;
/// Balloons stand this far outside their view's box (mm).
const BALLOON_GAP: f64 = 14.0;

fn unavailable(msg: impl Into<String>) -> CommandError {
    CommandError::Unavailable(msg.into())
}

/// The sheet a sheet-level annotation goes on: `sheet`, or the last one.
fn target_sheet(s: &Session, p: &Value) -> Result<u32, CommandError> {
    super::sheet_id(s, p)
}

fn point(p: &Value, x: &str, y: &str) -> Result<Point2, CommandError> {
    Ok(Point2::new(coordinate(p, x)?, coordinate(p, y)?))
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_bool(p, key).map(Some),
    }
}

/// Adds an annotation to a sheet after checking it.
fn push(s: &mut Session, sheet: u32, kind: AnnotationKind) -> Result<u32, CommandError> {
    kind.check().map_err(bad)?;
    if s.doc.drawing.sheet(sheet)?.annotations.len() >= dd::MAX_ANNOTATIONS {
        return Err(bad(format!(
            "a sheet holds at most {} annotations",
            dd::MAX_ANNOTATIONS
        )));
    }
    let id = s.doc.drawing.next_id()?;
    s.doc
        .drawing
        .sheet_mut(sheet)?
        .annotations
        .push(Annotation { id, kind });
    Ok(id)
}

/// The view `view` as placed on its sheet, refusing one that could not be drawn.
fn placed(s: &Session, sheet: u32, view: u32) -> Result<PlacedView, CommandError> {
    let g = render(s, sheet)?;
    let v = g
        .view(view)
        .cloned()
        .ok_or_else(|| bad(format!("no view {view} on sheet {sheet}")))?;
    if let Some(problem) = &v.problem {
        return Err(unavailable(format!("view {view}: {problem}")));
    }
    Ok(v)
}

fn view_and_sheet(s: &Session, p: &Value) -> Result<(u32, u32), CommandError> {
    let view = params::req_id(p, "view")?;
    let sheet = match opt_id(p, "sheet")? {
        Some(id) => {
            if s.doc.drawing.sheet(id)?.view(view).is_none() {
                return Err(bad(format!("no view {view} on sheet {id}")));
            }
            id
        }
        None => sheet_of_view(s, view)?,
    };
    Ok((view, sheet))
}

/// Letters ISO 128-44 leaves out of view labels, as they read like digits.
const SKIPPED: [char; 3] = ['I', 'O', 'Q'];

fn used_labels(sheet: &Sheet) -> Vec<String> {
    sheet
        .views
        .iter()
        .filter_map(|v| {
            v.section
                .as_ref()
                .map(|c| c.label.clone())
                .or(v.detail.as_ref().map(|d| d.label.clone()))
        })
        .collect()
}

/// The label from `label`, or the next free letter (A, B, ... then AA, AB, ...).
fn label(sheet: &Sheet, p: &Value) -> Result<String, CommandError> {
    let used = used_labels(sheet);
    if let Some(l) = params::opt_str(p, "label")? {
        if !dd::is_valid_label(l) {
            return Err(bad("`label` is one to three capital letters"));
        }
        if used.iter().any(|u| u == l) {
            return Err(bad(format!("the label {l:?} is taken on this sheet")));
        }
        return Ok(l.to_owned());
    }
    let letters: Vec<char> = ('A'..='Z').filter(|c| !SKIPPED.contains(c)).collect();
    let singles = letters.iter().map(|c| c.to_string());
    let doubles = letters
        .iter()
        .flat_map(|a| letters.iter().map(move |b| format!("{a}{b}")));
    singles
        .chain(doubles)
        .find(|l| !used.contains(l))
        .ok_or_else(|| bad("this sheet has no free view labels"))
}

/// The standard direction a basis looks along, if it is one.
fn direction_of(basis: &Basis) -> Option<ViewDirection> {
    ViewDirection::ALL
        .into_iter()
        .find(|d| Basis::of(*d).toward.dot(basis.toward) > 1.0 - 1e-9)
}

/// Adds a view and checks the sheet still draws it; a view that cannot be drawn is an
/// error (and the session undoes the change).
fn add_checked_view(s: &mut Session, sheet: u32, v: SheetView) -> CmdResult {
    let id = v.id;
    {
        let sh = s.doc.drawing.sheet_mut(sheet)?;
        if sh.views.len() >= dd::MAX_VIEWS {
            return Err(bad(format!(
                "a sheet holds at most {} views",
                dd::MAX_VIEWS
            )));
        }
        sh.views.push(v);
    }
    s.doc.drawing.check()?;
    let g = render(s, sheet)?;
    let pv = g
        .view(id)
        .ok_or_else(|| unavailable("the view could not be placed"))?;
    if let Some(problem) = &pv.problem {
        return Err(unavailable(problem.clone()));
    }
    let area: f64 = pv
        .geometry
        .regions
        .iter()
        .map(|r| chamfera_drawing::section::region_area(r))
        .sum::<f64>()
        + 0.0;
    Ok(json!({
        "sheet": sheet,
        "view": id,
        "scale": pv.scale,
        "bounds": pv.bounds().map(|(lo, hi)| [lo.x, lo.y, hi.x, hi.y]),
        "hatched_area": area,
    }))
}

pub(super) fn add_section(s: &mut Session, p: &Value) -> CmdResult {
    let (view, sheet) = view_and_sheet(s, p)?;
    let src = s.doc.drawing.sheet(sheet)?.view(view).cloned();
    let src = src.ok_or_else(|| bad(format!("no view {view}")))?;
    if !src.is_plain() || src.direction == ViewDirection::Iso {
        return Err(bad(
            "a section is cut on a plain orthographic view (not iso, a section or a detail)",
        ));
    }
    let a = point(p, "x1", "y1")?;
    let b = point(p, "x2", "y2")?;
    if a.distance(b) < 1.0 {
        return Err(bad(
            "the cutting line must be at least 1 mm long on the sheet",
        ));
    }
    let at = point(p, "x", "y")?;
    let flip = opt_bool(p, "flip")?.unwrap_or(false);
    let scale = scale_param(p, "scale")?;
    let pv = placed(s, sheet, view)?;
    let label = label(s.doc.drawing.sheet(sheet)?, p)?;
    let cut = SectionCut {
        from: view,
        a: pv.to_view(a),
        b: pv.to_view(b),
        label: label.clone(),
        flip,
    };
    let direction = chamfera_drawing::section::cut_of(&pv.basis, &cut)
        .and_then(|c| direction_of(&c.basis))
        .unwrap_or(src.direction);
    let id = s.doc.drawing.next_id()?;
    let mut r = add_checked_view(
        s,
        sheet,
        SheetView {
            id,
            body: src.body.clone(),
            direction,
            x: at.x,
            y: at.y,
            scale,
            parent: None,
            section: Some(cut),
            detail: None,
        },
    )?;
    r["label"] = json!(label);
    Ok(r)
}

pub(super) fn add_detail(s: &mut Session, p: &Value) -> CmdResult {
    let (view, sheet) = view_and_sheet(s, p)?;
    let src = s.doc.drawing.sheet(sheet)?.view(view).cloned();
    let src = src.ok_or_else(|| bad(format!("no view {view}")))?;
    if src.detail.is_some() {
        return Err(bad("a detail enlarges a view that is not a detail itself"));
    }
    let c = point(p, "cx", "cy")?;
    let r = params::req_f64(p, "radius")?;
    if !(r.is_finite() && (0.5..=dd::MAX_SHEET_COORDINATE).contains(&r)) {
        return Err(bad(format!(
            "`radius` must be from 0.5 to {} mm on the sheet",
            dd::MAX_SHEET_COORDINATE
        )));
    }
    let at = point(p, "x", "y")?;
    let pv = placed(s, sheet, view)?;
    let scale = match scale_param(p, "scale")? {
        Some(k) => k,
        None => {
            let k = pv.scale * 2.0;
            if !dd::is_valid_scale(k) {
                return Err(bad("twice the view's scale is out of range: give `scale`"));
            }
            k
        }
    };
    let label = label(s.doc.drawing.sheet(sheet)?, p)?;
    let id = s.doc.drawing.next_id()?;
    let mut out = add_checked_view(
        s,
        sheet,
        SheetView {
            id,
            body: src.body.clone(),
            direction: src.direction,
            x: at.x,
            y: at.y,
            scale: Some(scale),
            parent: None,
            section: None,
            detail: Some(DetailCircle {
                from: view,
                centre: pv.to_view(c),
                radius: r / pv.scale,
                label: label.clone(),
            }),
        },
    )?;
    out["label"] = json!(label);
    Ok(out)
}

fn text_param(p: &Value, key: &str) -> Result<String, CommandError> {
    let t = params::req_str(p, key)?;
    if !dd::is_valid_note(t) {
        return Err(bad(format!(
            "`{key}` holds 1 to {} characters and no control characters",
            dd::MAX_NOTE
        )));
    }
    Ok(t.to_owned())
}

pub(super) fn add_note(s: &mut Session, p: &Value) -> CmdResult {
    let sheet = target_sheet(s, p)?;
    let at = point(p, "x", "y")?;
    let text = text_param(p, "text")?;
    let height = params::opt_f64(p, "height")?.unwrap_or(chamfera_drawing::annot::TEXT);
    let id = push(s, sheet, AnnotationKind::Note { at, text, height })?;
    Ok(json!({ "sheet": sheet, "annotation": id }))
}

pub(super) fn add_leader(s: &mut Session, p: &Value) -> CmdResult {
    let sheet = target_sheet(s, p)?;
    let list = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`points` must be a list of {x, y} sheet points"))?;
    if !(2..=dd::MAX_LEADER_POINTS).contains(&list.len()) {
        return Err(bad(format!(
            "a leader has 2 to {} points",
            dd::MAX_LEADER_POINTS
        )));
    }
    let points = list
        .iter()
        .map(|q| point(q, "x", "y"))
        .collect::<Result<Vec<_>, _>>()?;
    let text = text_param(p, "text")?;
    let id = push(s, sheet, AnnotationKind::Leader { points, text })?;
    Ok(json!({ "sheet": sheet, "annotation": id }))
}

pub(super) fn add_surface_finish(s: &mut Session, p: &Value) -> CmdResult {
    let sheet = target_sheet(s, p)?;
    let at = point(p, "x", "y")?;
    let text = match params::opt_str(p, "text")? {
        None | Some("") => String::new(),
        Some(t) if dd::is_valid_note(t) => t.to_owned(),
        Some(_) => {
            return Err(bad(format!(
                "`text` holds at most {} characters and no control characters",
                dd::MAX_NOTE
            )));
        }
    };
    let removal = opt_bool(p, "removal")?.unwrap_or(true);
    let id = push(
        s,
        sheet,
        AnnotationKind::SurfaceFinish { at, text, removal },
    )?;
    Ok(json!({ "sheet": sheet, "annotation": id }))
}

pub(super) fn add_gdt(s: &mut Session, p: &Value) -> CmdResult {
    let sheet = target_sheet(s, p)?;
    let name = params::req_str(p, "characteristic")?;
    let characteristic = GdtCharacteristic::parse(name).ok_or_else(|| {
        bad(format!(
            "unknown characteristic {name:?}: use straightness, flatness, circularity, \
             parallelism, perpendicularity or position"
        ))
    })?;
    let tolerance = params::req_f64(p, "tolerance")?;
    let at = point(p, "x", "y")?;
    let diameter = opt_bool(p, "diameter")?.unwrap_or(false);
    let datums = match p.get("datums") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.len() <= dd::MAX_DATUMS => a
            .iter()
            .map(|d| {
                d.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad("each datum is a string of capital letters"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(bad(format!(
                "`datums` is a list of at most {} letters",
                dd::MAX_DATUMS
            )));
        }
    };
    let leader = match (p.get("leader_x"), p.get("leader_y")) {
        (None | Some(Value::Null), None | Some(Value::Null)) => None,
        _ => Some(point(p, "leader_x", "leader_y")?),
    };
    let id = push(
        s,
        sheet,
        AnnotationKind::Gdt {
            at,
            characteristic,
            tolerance,
            diameter,
            datums,
            leader,
        },
    )?;
    Ok(json!({ "sheet": sheet, "annotation": id }))
}

/// The circle or arc of a view nearest a pick (sheet mm): its centre and radius in view
/// coordinates.
fn circle_near(v: &PlacedView, pick: Point2) -> Option<(Point2, f64)> {
    let pv = v.to_view(pick);
    v.geometry
        .curves
        .iter()
        .filter_map(|c| match c.curve {
            Curve2::Circle { centre, radius } | Curve2::Arc { centre, radius, .. } => {
                Some((centre, radius))
            }
            Curve2::Line { .. } => None,
        })
        .map(|(c, r)| {
            let d = (((pv - c).length() - r).abs() * v.scale).min(pv.distance(c) * v.scale);
            (d, c, r)
        })
        .filter(|(d, _, _)| *d <= PICK)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c, r)| (c, r))
}

pub(super) fn add_centre_line(s: &mut Session, p: &Value) -> CmdResult {
    let (view, sheet) = view_and_sheet(s, p)?;
    let picks = [point(p, "x1", "y1")?, point(p, "x2", "y2")?];
    let pv = placed(s, sheet, view)?;
    let mut found = Vec::new();
    for q in picks {
        found.push(circle_near(&pv, q).ok_or_else(|| {
            unavailable(format!(
                "no circle or arc of view {view} within {PICK} mm of ({:.2}, {:.2})",
                q.x, q.y
            ))
        })?);
    }
    let (Some(a), Some(b)) = (found.first().copied(), found.get(1).copied()) else {
        return Err(bad("two circles are needed"));
    };
    if a.0.distance(b.0) < 1e-6 {
        return Err(bad(
            "the two circles share their centre: pick two different ones",
        ));
    }
    let refs =
        chamfera_drawing::centre_line_refs(s.doc.drawing.sheet(sheet)?, &bodies(s), view, a, b);
    let associative = !refs.is_empty();
    let id = push(
        s,
        sheet,
        AnnotationKind::CentreLine {
            view,
            points: [a.0, b.0],
            refs,
        },
    )?;
    Ok(json!({
        "sheet": sheet,
        "annotation": id,
        "length": a.0.distance(b.0),
        "associative": associative,
    }))
}

pub(super) fn add_parts_list(s: &mut Session, p: &Value) -> CmdResult {
    let sheet = target_sheet(s, p)?;
    let rows = parts(s);
    if rows.is_empty() {
        return Err(unavailable(
            "the design has no components: make some with component.new first",
        ));
    }
    let (w, _) = s.doc.drawing.sheet(sheet)?.size();
    let x = match p.get("x") {
        None | Some(Value::Null) => w - chamfera_drawing::MARGIN,
        Some(_) => coordinate(p, "x")?,
    };
    let y = match p.get("y") {
        None | Some(Value::Null) => chamfera_drawing::MARGIN + chamfera_drawing::TITLE_HEIGHT,
        Some(_) => coordinate(p, "y")?,
    };
    let id = s.doc.drawing.next_id()?;
    s.doc.drawing.sheet_mut(sheet)?.parts_list = Some(PartsList { id, x, y });
    let list: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            json!({ "item": i + 1, "name": r.name, "quantity": r.quantity, "material": r.material })
        })
        .collect();
    Ok(json!({ "sheet": sheet, "parts_list": id, "rows": list }))
}

pub(super) fn add_balloons(s: &mut Session, p: &Value) -> CmdResult {
    let (view, sheet) = view_and_sheet(s, p)?;
    let rows = parts(s);
    if rows.is_empty() {
        return Err(unavailable(
            "the design has no components: make some with component.new first",
        ));
    }
    let pv = placed(s, sheet, view)?;
    let shown_body = s
        .doc
        .drawing
        .sheet(sheet)?
        .view(view)
        .and_then(|v| v.body.clone());
    let (lo, hi) = pv
        .bounds()
        .ok_or_else(|| unavailable("the view is empty"))?;
    let reach = 0.5 * lo.distance(hi) + BALLOON_GAP;
    let mut kinds = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        // The component's solids in this view, and the centre of their box.
        let mut bbox: Option<chamfera_geom::Aabb> = None;
        for sol in s.solids() {
            if !r.components.contains(&sol.component)
                || shown_body.as_ref().is_some_and(|b| *b != sol.name)
            {
                continue;
            }
            if let Some(x) = sol.body.bounding_box() {
                bbox = Some(match bbox {
                    None => x,
                    Some(o) => o.union(x),
                });
            }
        }
        let Some(bbox) = bbox else { continue };
        let c: Point3 = bbox.center();
        let target = pv.basis.project(c);
        let ts = pv.to_sheet(target);
        // Out from the view's centre through the target, else round the view.
        let fallback = {
            let a = std::f64::consts::FRAC_PI_4 + i as f64 * 0.7;
            chamfera_geom::Vec2::new(a.cos(), a.sin())
        };
        let dir = (ts - pv.centre)
            .normalized()
            .filter(|_| ts.distance(pv.centre) > 1.0)
            .unwrap_or(fallback);
        let at = pv.centre + dir * reach;
        let comp = r.components.first().copied().unwrap_or(0);
        kinds.push((
            i + 1,
            AnnotationKind::Balloon {
                view,
                component: comp,
                at,
                target,
            },
        ));
    }
    if kinds.is_empty() {
        return Err(unavailable(format!(
            "view {view} shows no component's bodies"
        )));
    }
    s.doc
        .drawing
        .sheet_mut(sheet)?
        .annotations
        .retain(|a| !matches!(a.kind, AnnotationKind::Balloon { view: w, .. } if w == view));
    let mut made = Vec::new();
    for (item, kind) in kinds {
        let id = push(s, sheet, kind)?;
        made.push(json!({ "annotation": id, "item": item }));
    }
    Ok(json!({ "sheet": sheet, "balloons": made }))
}

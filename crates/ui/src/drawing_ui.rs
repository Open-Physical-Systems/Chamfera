//! The DRAWING workspace: a 2D sheet canvas with the views of the model, and its ribbon.
//!
//! The sheet is laid out by `chamfera-drawing` (through `chamfera_engine::cmd::drawing::
//! render`) and painted here with egui shapes in the theme's sheet colours. Panning and
//! zooming the sheet is local to this canvas (it has no 3D camera). Everything that
//! changes the drawing runs a `drawing.*` command: placing a view is one click, a
//! dimension two (one for a diameter or radius).

use crate::icons::Icon;
use crate::widgets::{self, T_SMALL};
use crate::{ChamferaApp, color, label};
use chamfera_doc::Document;
use chamfera_doc::drawing::{DimensionKind, ProjectionAngle, ViewDirection};
use chamfera_drawing::{Align, Layer, Owner, Shape, SheetGeometry};
use chamfera_geom::Point2;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;

/// How near (sheet mm) a click must be to a dimension or annotation to pick it.
const PICK_MM: f64 = 2.0;

/// What a click on the sheet does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawTool {
    PlaceView(ViewDirection),
    Dimension(DimensionKind),
    /// Two clicks for the cutting line on a view, one to place the section.
    Section,
    /// The circle's centre, a point on it, then where the detail goes.
    Detail,
    Note,
    /// The arrow's tip, then where the text goes.
    Leader,
    SurfaceFinish,
    /// The feature the leader points at, then the frame's corner.
    Gdt,
    /// Two circles of a view.
    CentreLine,
    /// A view whose components get balloons.
    Balloons,
    /// The origin, then each point measured from it, one ordinate dimension a click.
    Ordinate,
    /// Points from a baseline (the first click) or along a chain; Enter places them.
    Baseline,
    Chain,
    /// A hole's circle, then where its callout goes.
    HoleCallout,
}

/// The drawing workspace's state.
#[derive(Default)]
pub(crate) struct DrawingUi {
    /// The DRAWING tab is open.
    pub active: bool,
    /// The sheet shown; `None` for the last one.
    pub sheet: Option<u32>,
    pub tool: Option<DrawTool>,
    /// Dimension picks so far, sheet mm.
    pub picks: Vec<Point2>,
    /// The sheet point at the canvas centre, and screen points per sheet mm; `None`
    /// fits the sheet on the next frame.
    view: Option<(Point2, f32)>,
    /// The last layout, and the design and sheet it was made for.
    cache: Option<(Document, u32, SheetGeometry)>,
    /// The canvas on screen, for turning clicks into sheet points.
    pub(crate) rect: Option<Rect>,
    /// The fields of the annotation tools (`drawing_tools_ui.rs`).
    pub options: crate::drawing_tools_ui::DrawOptions,
    /// The view, dimension or annotation picked with no tool (moved by a drag,
    /// removed by Delete).
    pub selected: Option<Owner>,
    /// A drag of the selection: where it started and where the pointer is, sheet mm.
    drag: Option<(Point2, Point2)>,
    /// The Sheet Settings dialog, while it is open (`drawing_sheet_ui.rs`).
    pub settings: Option<crate::drawing_sheet_ui::SheetForm>,
}

/// Sheet mm to screen points and back (y up on paper, down on screen).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mapping {
    pub centre_screen: Pos2,
    pub centre_sheet: Point2,
    pub zoom: f32,
}

impl Mapping {
    pub fn to_screen(self, p: Point2) -> Pos2 {
        egui::pos2(
            self.centre_screen.x + ((p.x - self.centre_sheet.x) as f32) * self.zoom,
            self.centre_screen.y - ((p.y - self.centre_sheet.y) as f32) * self.zoom,
        )
    }
    pub fn to_sheet(self, p: Pos2) -> Point2 {
        Point2::new(
            self.centre_sheet.x + f64::from((p.x - self.centre_screen.x) / self.zoom),
            self.centre_sheet.y - f64::from((p.y - self.centre_screen.y) / self.zoom),
        )
    }
}

/// Splits a polyline into dashes following `pattern` (dash, gap, dash, gap ... in
/// screen points); an empty pattern keeps it whole.
fn dashed(points: &[Pos2], pattern: &[f32]) -> Vec<[Pos2; 2]> {
    let mut out = Vec::new();
    if pattern.is_empty() || pattern.iter().any(|d| !(d.is_finite() && *d > 0.01)) {
        for w in points.windows(2) {
            out.push([w[0], w[1]]);
        }
        return out;
    }
    let mut k = 0usize;
    let mut left = pattern[0];
    let mut on = true;
    let mut pieces = 0usize;
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut len = a.distance(b);
        while len > 0.0 && pieces < 20_000 {
            pieces += 1;
            let step = left.min(len);
            let t = step / len;
            let c = a + (b - a) * t;
            if on {
                out.push([a, c]);
            }
            a = c;
            len -= step;
            left -= step;
            if left <= 1e-4 {
                k = (k + 1) % pattern.len();
                left = pattern[k];
                on = !on;
            }
        }
    }
    out
}

fn layer_colour(app: &ChamferaApp, layer: Layer) -> Color32 {
    let t = &app.theme;
    color(match layer {
        Layer::Visible => t.sheet_line,
        Layer::Hidden => t.sheet_hidden,
        Layer::Center => t.sheet_centre,
        Layer::Hatch => t.sheet_hatch,
        Layer::Dim => t.sheet_dimension,
        Layer::Dangling => t.sheet_dangling,
        Layer::Border => t.sheet_border,
    })
}

/// Points along an arc (counter-clockwise on paper), about one per 2 screen points.
fn arc_points(m: &Mapping, c: Point2, r: f64, start: f64, end: f64) -> Vec<Pos2> {
    let sweep = end - start;
    let len = (r * sweep).abs() as f32 * m.zoom;
    let n = ((len / 2.0).ceil() as usize).clamp(8, 720);
    (0..=n)
        .map(|i| {
            let a = start + sweep * i as f64 / n as f64;
            m.to_screen(Point2::new(c.x + r * a.cos(), c.y + r * a.sin()))
        })
        .collect()
}

impl ChamferaApp {
    /// The sheet the workspace shows.
    pub(crate) fn drawing_sheet(&self) -> Option<u32> {
        let sheets = &self.session.doc.drawing.sheets;
        self.drawing
            .sheet
            .filter(|id| sheets.iter().any(|s| s.id == *id))
            .or_else(|| sheets.last().map(|s| s.id))
    }

    /// The laid-out sheet, worked out again only when the design changed.
    fn drawing_geometry(&mut self) -> Option<&SheetGeometry> {
        let id = self.drawing_sheet()?;
        let fresh = self
            .drawing
            .cache
            .as_ref()
            .is_some_and(|(doc, sid, _)| *sid == id && *doc == self.session.doc);
        if !fresh {
            match chamfera_engine::cmd::drawing::render(&self.session, id) {
                Ok(g) => self.drawing.cache = Some((self.session.doc.clone(), id, g)),
                Err(e) => {
                    self.message = Some((format!("Drawing: {e}"), true));
                    self.drawing.cache = None;
                    return None;
                }
            }
        }
        self.drawing.cache.as_ref().map(|c| &c.2)
    }

    fn drawing_mapping(&self, rect: Rect, size: (f64, f64)) -> Mapping {
        let (centre_sheet, zoom) = self.drawing.view.unwrap_or_else(|| {
            let z =
                (rect.width() / (size.0 as f32 + 20.0)).min(rect.height() / (size.1 as f32 + 20.0));
            (Point2::new(size.0 * 0.5, size.1 * 0.5), z.max(0.05))
        });
        Mapping {
            centre_screen: rect.center(),
            centre_sheet,
            zoom,
        }
    }

    /// A click on the sheet at `p` (sheet mm) with the current tool. Tests call this
    /// instead of moving a mouse.
    pub(crate) fn drawing_click(&mut self, p: Point2) {
        let sheet = self.drawing_sheet();
        match self.drawing.tool {
            None => self.drawing_select(p),
            Some(
                t @ (DrawTool::Section
                | DrawTool::Detail
                | DrawTool::Note
                | DrawTool::Leader
                | DrawTool::SurfaceFinish
                | DrawTool::Gdt
                | DrawTool::CentreLine
                | DrawTool::Balloons
                | DrawTool::Ordinate
                | DrawTool::Baseline
                | DrawTool::Chain
                | DrawTool::HoleCallout),
            ) => self.drawing_tool_click(t, p),
            Some(DrawTool::PlaceView(d)) => {
                let mut params = json!({ "view": d.id(), "x": p.x, "y": p.y });
                if let Some(s) = sheet {
                    params["sheet"] = json!(s);
                }
                if self.run("drawing.add_view", params).is_some() {
                    self.drawing.tool = None;
                }
            }
            Some(DrawTool::Dimension(kind)) => {
                self.drawing.picks.push(p);
                let needed = match kind {
                    DimensionKind::Diameter | DimensionKind::Radius => 1,
                    _ => 2,
                };
                if self.drawing.picks.len() < needed {
                    return;
                }
                let picks = std::mem::take(&mut self.drawing.picks);
                let first = picks.first().copied().unwrap_or(p);
                let Some(view) = self
                    .drawing_geometry()
                    .and_then(|g| g.view_at(first, 6.0))
                    .map(|v| (v.id, v.centre))
                else {
                    self.message = Some(("Add Dimension: click on a view's edges".into(), true));
                    return;
                };
                let mut params =
                    json!({ "view": view.0, "kind": kind.id(), "x1": first.x, "y1": first.y });
                if let Some(second) = picks.get(1) {
                    params["x2"] = json!(second.x);
                    params["y2"] = json!(second.y);
                }
                // The dimension goes on the side of the view the picks are on.
                let n = picks.len() as f64;
                let mean = Point2::new(
                    picks.iter().map(|q| q.x).sum::<f64>() / n,
                    picks.iter().map(|q| q.y).sum::<f64>() / n,
                );
                let offset = match kind {
                    DimensionKind::Horizontal if mean.y < view.1.y => -10.0,
                    DimensionKind::Vertical if mean.x < view.1.x => -10.0,
                    DimensionKind::Angular => 15.0,
                    _ => 10.0,
                };
                params["offset"] = json!(offset);
                if let Some(r) = self.run("drawing.add_dimension", params) {
                    self.message = Some((
                        format!("Dimension {}", r["text"].as_str().unwrap_or("")),
                        false,
                    ));
                }
            }
        }
    }

    /// The sheet canvas, in place of the 3D viewport.
    pub(crate) fn drawing_canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        self.drawing.rect = Some(rect);
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color(self.theme.sheet_backdrop));
        if self.drawing_sheet().is_none() {
            let look = self.look();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No sheet yet: use New Sheet, or place a view",
                egui::FontId::proportional(widgets::T_BODY),
                look.c(|t| t.text_muted),
            );
        }
        let size = self
            .drawing_sheet()
            .and_then(|id| self.session.doc.drawing.sheet(id).ok())
            .map_or((420.0, 297.0), |s| s.size());
        let mut m = self.drawing_mapping(rect, size);
        // Pan with the middle or right button, zoom with the wheel about the pointer.
        if response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            let d = response.drag_delta();
            m.centre_sheet = Point2::new(
                m.centre_sheet.x - f64::from(d.x / m.zoom),
                m.centre_sheet.y + f64::from(d.y / m.zoom),
            );
            self.drawing.view = Some((m.centre_sheet, m.zoom));
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let before = m.to_sheet(hover);
                let z = (m.zoom * (scroll * 0.002).exp()).clamp(0.05, 200.0);
                m.zoom = z;
                let after = m.to_sheet(hover);
                m.centre_sheet = Point2::new(
                    m.centre_sheet.x + before.x - after.x,
                    m.centre_sheet.y + before.y - after.y,
                );
                self.drawing.view = Some((m.centre_sheet, m.zoom));
            }
        }
        if response.double_clicked_by(egui::PointerButton::Middle) {
            self.drawing.view = None;
        }
        let escape =
            !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            self.drawing.tool = None;
            self.drawing.picks.clear();
            self.drawing.selected = None;
        }
        let enter =
            !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter
            && matches!(
                self.drawing.tool,
                Some(DrawTool::Baseline | DrawTool::Chain)
            )
        {
            self.drawing_finish_set();
        }
        // The paper and its shadow.
        let lo = m.to_screen(Point2::new(0.0, size.1));
        let hi = m.to_screen(Point2::new(size.0, 0.0));
        let paper = Rect::from_min_max(lo, hi);
        painter.rect_filled(
            paper.translate(Vec2::new(3.0, 3.0)),
            0.0,
            color(self.theme.shadow),
        );
        painter.rect_filled(paper, 0.0, color(self.theme.sheet_paper));
        let zoom = m.zoom;
        let items = self
            .drawing_geometry()
            .map(|g| g.items.clone())
            .unwrap_or_default();
        for item in &items {
            let c = layer_colour(self, item.layer);
            let width = (item.layer.width() as f32 * zoom).max(if item.layer == Layer::Visible {
                1.4
            } else {
                0.8
            });
            let stroke = Stroke::new(width, c);
            let pattern: Vec<f32> = item
                .layer
                .dashes()
                .iter()
                .map(|d| (*d as f32 * zoom).max(2.0))
                .collect();
            let path = match &item.shape {
                Shape::Line { a, b } => vec![m.to_screen(*a), m.to_screen(*b)],
                Shape::Circle { centre, radius } => {
                    arc_points(&m, *centre, *radius, 0.0, std::f64::consts::TAU)
                }
                Shape::Arc {
                    centre,
                    radius,
                    start,
                    end,
                } => arc_points(&m, *centre, *radius, *start, *end),
                Shape::Fill { points } => {
                    painter.add(egui::Shape::convex_polygon(
                        points.iter().map(|p| m.to_screen(*p)).collect(),
                        c,
                        Stroke::NONE,
                    ));
                    continue;
                }
                Shape::Text {
                    at,
                    height,
                    angle,
                    align,
                    text,
                } => {
                    let size = (*height as f32 * zoom).clamp(1.0, 400.0);
                    if size < 3.0 {
                        continue;
                    }
                    let galley =
                        painter.layout_no_wrap(text.clone(), egui::FontId::proportional(size), c);
                    let w = galley.size().x;
                    let shift = match align {
                        Align::Left => 0.0,
                        Align::Centre => w * 0.5,
                        Align::Right => w,
                    };
                    // The galley's top-left, from the baseline anchor, turned with it
                    // (screen angles run clockwise).
                    let th = -(*angle as f32);
                    let (dx, dy) = (-shift, -size * 0.8);
                    let at = m.to_screen(*at);
                    let pos = egui::pos2(
                        at.x + dx * th.cos() - dy * th.sin(),
                        at.y + dx * th.sin() + dy * th.cos(),
                    );
                    painter.add(egui::epaint::TextShape::new(pos, galley, c).with_angle(th));
                    continue;
                }
            };
            for [a, b] in dashed(&path, &pattern) {
                painter.line_segment([a, b], stroke);
            }
        }
        // With no tool: a drag moves what it starts on, Delete removes the selection.
        if self.drawing.tool.is_none() {
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(o) = ui.input(|i| i.pointer.press_origin())
            {
                let p = m.to_sheet(o);
                self.drawing_select(p);
                self.drawing.drag = self.drawing.selected.map(|_| (p, p));
            }
            if let Some((from, _)) = self.drawing.drag
                && let Some(h) = response.interact_pointer_pos()
            {
                self.drawing.drag = Some((from, m.to_sheet(h)));
            }
            if response.drag_stopped_by(egui::PointerButton::Primary)
                && let Some((from, to)) = self.drawing.drag.take()
            {
                self.drawing_move_selected(to.x - from.x, to.y - from.y);
            }
            let delete = !ui.ctx().egui_wants_keyboard_input()
                && ui.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                });
            if delete {
                self.drawing_delete_selected();
            }
        } else {
            self.drawing.selected = None;
            self.drawing.drag = None;
        }
        let shift = self
            .drawing
            .drag
            .map_or(chamfera_geom::Vec2::new(0.0, 0.0), |(a, b)| b - a);
        self.paint_drawing_selection(&painter, &m, shift);
        // Tool feedback: picks so far and a cross at the pointer.
        let pick = color(self.theme.sheet_pick);
        for p in &self.drawing.picks {
            painter.circle_stroke(m.to_screen(*p), 5.0, Stroke::new(2.0, pick));
        }
        if self.drawing.tool.is_some()
            && let Some(h) = response.hover_pos()
        {
            let s = Stroke::new(1.0, pick);
            painter.line_segment([h - Vec2::new(10.0, 0.0), h + Vec2::new(10.0, 0.0)], s);
            painter.line_segment([h - Vec2::new(0.0, 10.0), h + Vec2::new(0.0, 10.0)], s);
            let p = m.to_sheet(h);
            painter.text(
                h + Vec2::new(12.0, 12.0),
                egui::Align2::LEFT_TOP,
                format!("{:.1}, {:.1}", p.x, p.y),
                egui::FontId::proportional(T_SMALL),
                pick,
            );
        }
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(h) = response.interact_pointer_pos()
        {
            let p = m.to_sheet(h);
            self.drawing_click(p);
        }
        self.drawing_options_dialog(ui.ctx(), rect);
        self.sheet_picker(ui.ctx(), rect);
        self.sheet_settings_dialog(ui.ctx());
    }

    /// Fits the sheet in the canvas on the next frame (after its size changed).
    pub(crate) fn drawing_refit(&mut self) {
        self.drawing.view = None;
    }

    /// The sheets of the design, by id and name, for the sheet picker.
    pub(crate) fn drawing_sheets(&self) -> Vec<(u32, String)> {
        self.session
            .doc
            .drawing
            .sheets
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect()
    }

    /// Shows sheet `id` on the canvas, fitted.
    pub(crate) fn drawing_show_sheet(&mut self, id: u32) {
        if self.drawing.sheet != Some(id) {
            self.drawing.sheet = Some(id);
            self.drawing.selected = None;
            self.drawing.view = None;
            self.drawing.settings = None;
        }
    }

    /// With more than one sheet, a list at the canvas's top-left to show another.
    fn sheet_picker(&mut self, ctx: &egui::Context, rect: Rect) {
        let sheets = self.drawing_sheets();
        if sheets.len() < 2 {
            return;
        }
        let shown = self.drawing_sheet();
        let current = sheets
            .iter()
            .find(|(id, _)| Some(*id) == shown)
            .map_or("Sheet", |(_, n)| n.as_str())
            .to_owned();
        let mut pick = None;
        egui::Area::new(egui::Id::unique("sheet picker"))
            .fixed_pos(rect.min + Vec2::new(8.0, 8.0))
            .show(ctx, |ui| {
                egui::ComboBox::from_id_salt("drawing sheet")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (id, name) in &sheets {
                            if ui.selectable_label(Some(*id) == shown, name).clicked() {
                                pick = Some(*id);
                            }
                        }
                    })
                    .response
                    .on_hover_text("The sheet shown");
            });
        if let Some(id) = pick {
            self.drawing_show_sheet(id);
        }
    }

    /// A click with no tool: selects the view, dimension or annotation under `p` (sheet
    /// mm) through `drawing.pick`, or clears the selection on empty paper.
    pub(crate) fn drawing_select(&mut self, p: Point2) {
        let mut params = json!({ "x": p.x, "y": p.y, "tolerance": PICK_MM });
        if let Some(s) = self.drawing_sheet() {
            params["sheet"] = json!(s);
        }
        self.drawing.selected = self.run("drawing.pick", params).and_then(|r| {
            let id = u32::try_from(r["id"].as_u64()?).ok()?;
            match r["kind"].as_str()? {
                "view" => Some(Owner::View(id)),
                "dimension" => Some(Owner::Dimension(id)),
                "annotation" => Some(Owner::Annotation(id)),
                _ => None,
            }
        });
    }

    /// Moves the selection by `(dx, dy)` sheet mm with one `drawing.move`.
    fn drawing_move_selected(&mut self, dx: f64, dy: f64) {
        let Some(owner) = self.drawing.selected else {
            return;
        };
        if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
            return;
        }
        let (key, id) = owner.key();
        let mut params = json!({ "dx": dx, "dy": dy });
        params[key] = json!(id);
        self.run("drawing.move", params);
    }

    /// A drag with no tool from `from` to `to` (sheet mm): selects what is under `from`
    /// and moves it (one undo step). Tests call this instead of a mouse.
    #[cfg(test)]
    pub(crate) fn drawing_drag(&mut self, from: Point2, to: Point2) {
        self.drawing_select(from);
        self.drawing_move_selected(to.x - from.x, to.y - from.y);
    }

    /// Selects what is at `from` and shows it being dragged to `to` without moving it
    /// (`--drawing-drag`, for screenshots).
    pub(crate) fn drawing_drag_preview(&mut self, from: Point2, to: Point2) {
        self.drawing_select(from);
        self.drawing.drag = self.drawing.selected.map(|_| (from, to));
    }

    /// Delete with a selection: removes it through `drawing.delete`.
    pub(crate) fn drawing_delete_selected(&mut self) {
        let Some(owner) = self.drawing.selected.take() else {
            return;
        };
        let (key, id) = owner.key();
        let mut params = json!({});
        params[key] = json!(id);
        self.run("drawing.delete", params);
    }

    /// The selection outlined in the pick colour: a view's box, or the lines and text a
    /// dimension or annotation drew; shifted by `shift` sheet mm while it is dragged.
    fn paint_drawing_selection(
        &mut self,
        painter: &egui::Painter,
        m: &Mapping,
        d: chamfera_geom::Vec2,
    ) {
        let Some(owner) = self.drawing.selected else {
            return;
        };
        let pick = color(self.theme.sheet_pick);
        let stroke = Stroke::new(2.0, pick);
        let Some(g) = self.drawing_geometry() else {
            return;
        };
        let at = |p: Point2| m.to_screen(p + d);
        if let Owner::View(id) = owner {
            if let Some((lo, hi)) = g.view(id).and_then(|v| v.bounds()) {
                let r = Rect::from_two_pos(at(lo), at(hi)).expand(4.0);
                painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Outside);
            }
            return;
        }
        for item in g.items_of(owner) {
            match &item.shape {
                Shape::Line { a, b } => {
                    painter.line_segment([at(*a), at(*b)], stroke);
                }
                Shape::Circle { centre, radius } => {
                    painter.circle_stroke(at(*centre), (*radius as f32) * m.zoom, stroke);
                }
                Shape::Arc {
                    centre,
                    radius,
                    start,
                    end,
                } => {
                    let pts = arc_points(m, *centre + d, *radius, *start, *end);
                    painter.add(egui::Shape::line(pts, stroke));
                }
                Shape::Fill { points } => {
                    painter.add(egui::Shape::convex_polygon(
                        points.iter().map(|p| at(*p)).collect(),
                        pick,
                        Stroke::NONE,
                    ));
                }
                Shape::Text {
                    at: a,
                    height,
                    angle,
                    align,
                    text,
                } => {
                    // An underline along the text's baseline marks it.
                    let w = chamfera_drawing::text_width(text, *height);
                    let x0 = match align {
                        chamfera_drawing::Align::Left => 0.0,
                        chamfera_drawing::Align::Centre => -w * 0.5,
                        chamfera_drawing::Align::Right => -w,
                    };
                    let u = chamfera_geom::Vec2::new(angle.cos(), angle.sin());
                    let p0 = *a + u * x0;
                    painter.line_segment([at(p0), at(p0 + u * w)], stroke);
                }
            }
        }
    }

    /// True when the shown sheet has a view to dimension or export.
    pub(crate) fn drawing_has_views(&self) -> bool {
        self.drawing_sheet()
            .and_then(|id| self.session.doc.drawing.sheet(id).ok())
            .is_some_and(|s| !s.views.is_empty())
    }

    /// True when the shown sheet (or a new one) uses third-angle projection.
    pub(crate) fn drawing_third_angle(&self) -> bool {
        self.drawing_sheet()
            .and_then(|id| self.session.doc.drawing.sheet(id).ok())
            .is_none_or(|s| s.projection == ProjectionAngle::Third)
    }

    /// New Sheet: adds a sheet and shows it.
    pub(crate) fn drawing_new_sheet(&mut self) {
        if let Some(r) = self.run("drawing.new_sheet", json!({})) {
            self.drawing.sheet = r["sheet"].as_u64().and_then(|v| u32::try_from(v).ok());
            self.drawing.view = None;
        }
    }

    /// Projection: switches the shown sheet between third- and first-angle projection.
    pub(crate) fn drawing_toggle_projection(&mut self) {
        let p = if self.drawing_third_angle() {
            "first"
        } else {
            "third"
        };
        if let Some(id) = self.drawing_sheet() {
            self.run(
                "drawing.edit_sheet",
                json!({ "sheet": id, "projection": p }),
            );
        }
    }

    /// Picks a sheet tool (placing a view or a dimension), or drops it when it is the one
    /// in use.
    pub(crate) fn drawing_pick_tool(&mut self, tool: DrawTool) {
        let next = if self.drawing.tool == Some(tool) {
            None
        } else {
            Some(tool)
        };
        if next != self.drawing.tool {
            self.drawing.tool = next;
            self.drawing.picks.clear();
        }
    }

    /// File > Export Drawing items, shown while the DRAWING tab is open. Returns the
    /// format picked.
    pub(crate) fn drawing_export_items(
        &self,
        ui: &mut egui::Ui,
        look: crate::widgets::Look,
    ) -> Option<&'static str> {
        if !self.drawing.active {
            return None;
        }
        let enabled = self.drawing_sheet().is_some();
        ui.separator();
        let mut chosen = None;
        for f in ["svg", "dxf", "pdf"] {
            let text = format!("{} as {}…", label("drawing.export"), f.to_uppercase());
            if widgets::menu_item(ui, look, Icon::Export, &text, enabled).clicked() {
                chosen = Some(f);
            }
        }
        chosen
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn export_drawing(&mut self, format: &str) {
        let kind = format!("{} drawing", format.to_uppercase());
        self.export(&kind, &[format], "drawing.export");
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn export_drawing(&mut self, format: &str) {
        let Some(v) = self.run("drawing.export_bytes", json!({ "format": format })) else {
            return;
        };
        let name = format!("{}.{format}", self.design_name());
        let mime = match format {
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            _ => "image/vnd.dxf",
        };
        match crate::web::download(&name, mime, v["data"].as_str().unwrap_or("")) {
            Ok(()) => self.message = Some((format!("Downloaded {name}"), false)),
            Err(e) => self.message = Some((format!("{}: {e}", label("drawing.export")), true)),
        }
    }

    /// The status bar hint for the drawing workspace.
    pub(crate) fn drawing_hint(&self) -> &'static str {
        match self.drawing.tool {
            Some(DrawTool::PlaceView(_)) => "Click on the sheet to place the view · Esc cancels",
            Some(DrawTool::Dimension(DimensionKind::Diameter | DimensionKind::Radius)) => {
                "Click a circle or arc · Esc cancels"
            }
            Some(DrawTool::Dimension(DimensionKind::Angular)) => {
                "Click two straight edges · Esc cancels"
            }
            Some(DrawTool::Dimension(_)) => "Click two vertices or edges · Esc cancels",
            Some(t) => crate::drawing_tools_ui::hint(t),
            None => {
                "Click to select · Drag to move · Del: delete · Middle- or right-drag: pan · Wheel: zoom · Middle double-click: fit"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mapping_round_trips_and_flips_y() {
        let m = Mapping {
            centre_screen: egui::pos2(500.0, 400.0),
            centre_sheet: Point2::new(210.0, 148.5),
            zoom: 2.0,
        };
        // 10 mm right and up of the centre is 20 points right and 20 up (smaller y).
        let s = m.to_screen(Point2::new(220.0, 158.5));
        assert_eq!((s.x, s.y), (520.0, 380.0));
        let p = m.to_sheet(s);
        assert!((p.x - 220.0).abs() < 1e-4 && (p.y - 158.5).abs() < 1e-4);
    }

    #[test]
    fn dashes_follow_the_pattern() {
        // A 10-point line with 3 on, 2 off: dashes at 0..3, 5..8 and the start of 10.
        let d = dashed(&[egui::pos2(0.0, 0.0), egui::pos2(10.0, 0.0)], &[3.0, 2.0]);
        assert_eq!(d.len(), 2);
        assert!((d[0][1].x - 3.0).abs() < 1e-4 && (d[1][0].x - 5.0).abs() < 1e-4);
        assert_eq!(
            dashed(&[egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0)], &[]).len(),
            1
        );
    }

    #[test]
    fn clicks_place_views_and_dimensions_through_commands() {
        let mut app = ChamferaApp::headless();
        let s = &mut app.session;
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
        )
        .unwrap();
        app.drawing.active = true;
        app.drawing.tool = Some(DrawTool::PlaceView(ViewDirection::Front));
        app.drawing_click(Point2::new(150.0, 150.0));
        // No sheet existed: placing the view made one, and the tool is done.
        assert_eq!(app.session.doc.drawing.sheets.len(), 1);
        assert_eq!(app.session.doc.drawing.sheets[0].views.len(), 1);
        assert!(app.drawing.tool.is_none());
        // The front view's lower corners are at (125, 145) and (175, 145).
        app.drawing.tool = Some(DrawTool::Dimension(DimensionKind::Horizontal));
        app.drawing_click(Point2::new(125.4, 145.2));
        assert_eq!(app.drawing.picks.len(), 1);
        app.drawing_click(Point2::new(174.7, 145.1));
        let dims = &app.session.doc.drawing.sheets[0].dimensions;
        assert_eq!(dims.len(), 1);
        assert!(
            (chamfera_drawing::measure(dims[0].kind, &dims[0].points).unwrap() - 50.0).abs() < 1e-9
        );
        assert!(
            dims[0].offset < 0.0,
            "picks below the view centre put it below"
        );
    }

    /// Earlier sheets can be shown again (QA user guide pass: only the newest was).
    #[test]
    fn an_earlier_sheet_can_be_shown_again() {
        let mut app = ChamferaApp::headless();
        app.run("drawing.new_sheet", json!({ "name": "First" }));
        app.run("drawing.new_sheet", json!({ "name": "Second" }));
        let sheets = app.drawing_sheets();
        assert_eq!(sheets.len(), 2);
        assert_eq!(
            app.drawing_sheet(),
            Some(sheets[1].0),
            "the newest at first"
        );
        app.drawing_show_sheet(sheets[0].0);
        assert_eq!(app.drawing_sheet(), Some(sheets[0].0));
    }

    /// With no tool, a click selects what is under it, a drag moves it (one command,
    /// one undo step) and Delete removes it. The front view of the 50 x 30 x 10 box at
    /// (150, 150) spans x 125..175, y 145..155.
    #[test]
    fn a_drawing_item_is_selected_dragged_and_deleted() {
        let mut app = ChamferaApp::headless();
        let s = &mut app.session;
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        )
        .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
        )
        .unwrap();
        s.execute(
            "drawing.add_view",
            &json!({"view": "front", "x": 150, "y": 150}),
        )
        .unwrap();
        app.drawing.active = true;
        app.drawing_click(Point2::new(160.0, 150.0));
        let view = app.session.doc.drawing.sheets[0].views[0].id;
        assert_eq!(app.drawing.selected, Some(Owner::View(view)));
        app.drawing_drag(Point2::new(160.0, 150.0), Point2::new(200.0, 120.0));
        let v = &app.session.doc.drawing.sheets[0].views[0];
        assert_eq!((v.x, v.y), (190.0, 120.0));
        app.run("edit.undo", json!({}));
        assert_eq!(app.session.doc.drawing.sheets[0].views[0].x, 150.0);
        // A click on empty paper clears the selection; Delete then does nothing.
        app.drawing_click(Point2::new(60.0, 260.0));
        assert_eq!(app.drawing.selected, None);
        app.drawing_delete_selected();
        assert_eq!(app.session.doc.drawing.sheets[0].views.len(), 1);
        app.drawing_click(Point2::new(160.0, 150.0));
        app.drawing_delete_selected();
        assert!(app.session.doc.drawing.sheets[0].views.is_empty());
    }
}

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
use chamfera_drawing::{Align, Layer, Shape, SheetGeometry};
use chamfera_geom::Point2;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;

/// What a click on the sheet does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawTool {
    PlaceView(ViewDirection),
    Dimension(DimensionKind),
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
    rect: Option<Rect>,
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
        Layer::Dim => t.sheet_dimension,
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
            None => {}
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
            None => {
                "Pick a view or dimension tool · Middle- or right-drag: pan · Wheel: zoom · Middle double-click: fit"
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
}

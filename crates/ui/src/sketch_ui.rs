//! Sketch mode: the sketch toolbar, drawing tools with previews, selection, constraint and
//! dimension buttons, dimension labels with inline editing, and point dragging.
//!
//! Like the rest of the UI it acts only through commands (`sketch.*`).

use crate::{ChamferaApp, color, label};
use chamfera_engine::sketching::SketchItem;
use chamfera_geom::{Point2, Point3};
use chamfera_render::{Overlay, OverlayStyle, mat_mul, transform_point};
use egui::{Color32, Pos2, Rect, Vec2};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => label("sketch.line"),
            Tool::Rectangle => label("sketch.rectangle"),
            Tool::Circle => label("sketch.circle"),
            Tool::Arc => label("sketch.arc"),
        }
    }

    fn hint(self, clicks: usize) -> &'static str {
        match (self, clicks) {
            (Tool::Select, _) => {
                "Click to select (Shift adds) · drag a point to move it · Del deletes"
            }
            (Tool::Line, 0) => "Click the start of the line",
            (Tool::Line, _) => {
                "Click the next point · click the first point to close · Esc or right-click ends"
            }
            (Tool::Rectangle, 0) => "Click the first corner",
            (Tool::Rectangle, _) => "Click the opposite corner",
            (Tool::Circle, 0) => "Click the centre",
            (Tool::Circle, _) => "Click to set the radius",
            (Tool::Arc, 0) => "Click the centre",
            (Tool::Arc, 1) => "Click the start (sets the radius)",
            (Tool::Arc, _) => "Click the end (counter-clockwise from the start)",
        }
    }
}

/// A placed click: where it is, and the existing point it snapped to.
#[derive(Debug, Clone, Copy)]
struct Click {
    at: Point2,
    point: Option<u32>,
}

#[derive(Default)]
pub struct SketchUi {
    pub tool: Option<Tool>,
    clicks: Vec<Click>,
    /// The first point of the current line chain, to close the loop on it.
    chain_start: Option<u32>,
    hover: Option<Point2>,
    dragging: Option<u32>,
    /// A dimension being edited: its constraint id, kind and the text typed so far.
    editing: Option<(u64, String, String)>,
    /// Remembered status line for the active sketch.
    pub status: Option<String>,
}

impl SketchUi {
    pub fn tool(&self) -> Tool {
        self.tool.unwrap_or(Tool::Select)
    }

    fn reset_tool(&mut self) {
        self.clicks.clear();
        self.chain_start = None;
    }
}

/// Plane names for the Sketch menu, with the view that looks straight at each.
pub const PLANES: [(&str, &str, &str); 3] = [
    ("xy", "Top plane (XY)", "top"),
    ("xz", "Front plane (XZ)", "front"),
    ("yz", "Right plane (YZ)", "right"),
];

fn as_id(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl ChamferaApp {
    /// Runs a sketch command and refreshes the sketch status line.
    pub(crate) fn run_sketch(&mut self, id: &str, params: Value) -> Option<Value> {
        let r = self.run(id, params);
        self.refresh_sketch_status();
        r
    }

    pub(crate) fn refresh_sketch_status(&mut self) {
        self.sketch.status = match self.session.execute("sketch.get", &json!({})) {
            Ok(g) => {
                let name = g["name"].as_str().unwrap_or("Sketch").to_owned();
                let state = match (g["status"].as_str(), g["dof"].as_u64()) {
                    (Some("FullyConstrained"), _) => "fully constrained".to_owned(),
                    (Some("OverConstrained"), _) => "over-constrained".to_owned(),
                    (_, Some(1)) => "1 degree of freedom".to_owned(),
                    (_, Some(n)) => format!("{n} degrees of freedom"),
                    _ => "not solved".to_owned(),
                };
                let regions = g["profiles"].as_array().map_or(0, Vec::len);
                Some(format!("{name} · {state} · {regions} closed profiles"))
            }
            Err(_) => None,
        };
    }

    pub(crate) fn create_sketch(&mut self, plane: &str, view: &str) {
        if self
            .run_sketch("sketch.create", json!({ "plane": plane }))
            .is_some()
        {
            self.run("view.standard", json!({ "view": view }));
            self.sketch.tool = Some(Tool::Line);
            self.sketch.reset_tool();
        }
    }

    pub(crate) fn finish_sketch(&mut self) {
        self.run("sketch.finish", json!({}));
        self.sketch = super::sketch_ui::SketchUi::default();
    }

    /// The sketch toolbar, shown while a sketch is being edited.
    pub(crate) fn sketch_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("SKETCH");
            ui.separator();
            for tool in [
                Tool::Select,
                Tool::Line,
                Tool::Rectangle,
                Tool::Circle,
                Tool::Arc,
            ] {
                if ui
                    .selectable_label(self.sketch.tool() == tool, tool.name())
                    .clicked()
                {
                    self.sketch.tool = Some(tool);
                    self.sketch.reset_tool();
                }
            }
            ui.separator();
            let has_selection = !self.session.sketch_selection.is_empty();
            for (kind, text) in [
                ("horizontal", "Horizontal"),
                ("vertical", "Vertical"),
                ("coincident", "Coincident"),
                ("point_on", "On Curve"),
                ("midpoint", "Midpoint"),
                ("parallel", "Parallel"),
                ("perpendicular", "Perpendicular"),
                ("tangent", "Tangent"),
                ("equal", "Equal"),
                ("fix", "Fix"),
            ] {
                if ui
                    .add_enabled(has_selection, egui::Button::new(text).small())
                    .clicked()
                {
                    self.run_sketch("sketch.constrain_selection", json!({ "kind": kind }));
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(label("sketch.dimension_selection")),
                )
                .clicked()
                && let Some(r) = self.run_sketch("sketch.dimension_selection", json!({}))
                && let (Some(id), Some(v), Some(kind)) = (
                    r["constraint"].as_u64(),
                    r["value"].as_f64(),
                    r["kind"].as_str(),
                )
            {
                self.sketch.editing = Some((id, kind.to_owned(), display_value(kind, v)));
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(label("sketch.construction")),
                )
                .on_hover_text("Turn the selected curves into construction geometry, or back")
                .clicked()
            {
                self.toggle_construction();
            }
            ui.separator();
            if ui.button(label("sketch.finish")).clicked() {
                self.finish_sketch();
            }
        });
    }

    fn toggle_construction(&mut self) {
        let curves: Vec<u32> = self
            .session
            .sketch_selection
            .iter()
            .filter_map(|i| match i {
                SketchItem::Curve(id) => Some(*id),
                SketchItem::Point(_) => None,
            })
            .collect();
        let Some(f) = self.session.active_sketch() else {
            return;
        };
        let states: Vec<(u32, bool)> = curves
            .iter()
            .filter_map(|id| {
                f.sketch
                    .curve(chamfera_sketch::CurveId(*id))
                    .map(|c| (*id, !c.is_construction()))
            })
            .collect();
        for (id, value) in states {
            self.run_sketch(
                "sketch.construction",
                json!({ "curve": id, "construction": value }),
            );
        }
    }

    /// The point a click lands on: an existing sketch point under the cursor, else the
    /// plane point.
    fn click_at(&mut self, x: f64, y: f64, w: f64, h: f64) -> Option<Click> {
        let px = json!({ "x": x, "y": y, "width": w, "height": h });
        if let Ok(r) = self.session.execute("sketch.pick", &px)
            && let Some(id) = as_id(&r["item"]["point"])
            && let Some(at) = self
                .session
                .active_sketch()
                .and_then(|f| f.sketch.point(chamfera_sketch::PointId(id)))
        {
            return Some(Click {
                at,
                point: Some(id),
            });
        }
        let r = self.session.execute("sketch.plane_point", &px).ok()?;
        Some(Click {
            at: Point2::new(r["x"].as_f64()?, r["y"].as_f64()?),
            point: None,
        })
    }

    /// Mouse input in sketch mode. Returns true when the event was used (so the viewport
    /// does not also treat it as a body pick).
    pub(crate) fn sketch_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
    ) -> bool {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let (shift, esc, delete) = ui.input(|i| {
            (
                i.modifiers.shift,
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        let typing = ui.ctx().egui_wants_keyboard_input();
        if esc && !typing {
            if self.sketch.clicks.is_empty() {
                self.sketch.tool = Some(Tool::Select);
                self.run_sketch("sketch.select_clear", json!({}));
            }
            self.sketch.reset_tool();
        }
        if delete && !typing && !self.session.sketch_selection.is_empty() {
            self.run_sketch("sketch.remove", json!({}));
        }
        // Hover: where the next click would land (for previews).
        self.sketch.hover = response
            .hover_pos()
            .and_then(|p| {
                let (x, y) = local(p);
                self.click_at(x, y, w, h)
            })
            .map(|c| c.at);

        let tool = self.sketch.tool();
        // Dragging a point with the Select tool.
        if tool == Tool::Select {
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(p) = response.interact_pointer_pos()
            {
                let (x, y) = local(p);
                let picked = self
                    .session
                    .execute(
                        "sketch.pick",
                        &json!({ "x": x, "y": y, "width": w, "height": h }),
                    )
                    .ok()
                    .and_then(|r| as_id(&r["item"]["point"]));
                self.sketch.dragging = picked;
            }
            if let Some(point) = self.sketch.dragging {
                if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(at) = self.sketch.hover
                {
                    self.run_sketch(
                        "sketch.drag",
                        json!({ "point": point, "x": at.x, "y": at.y }),
                    );
                }
                if response.drag_stopped() {
                    self.sketch.dragging = None;
                }
                return true;
            }
        }
        if response.secondary_clicked() && !self.sketch.clicks.is_empty() {
            self.sketch.reset_tool();
            return true;
        }
        if !response.clicked() {
            return false;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };
        let (x, y) = local(pos);
        self.sketch_click(x, y, w, h, shift);
        true
    }

    /// A left click at pixel `(x, y)` of a `w` x `h` viewport in sketch mode: selects
    /// with the Select tool, else places the next point of the current tool.
    pub(crate) fn sketch_click(&mut self, x: f64, y: f64, w: f64, h: f64, shift: bool) {
        let tool = self.sketch.tool();
        if tool == Tool::Select {
            self.run_sketch(
                "sketch.select",
                json!({ "x": x, "y": y, "width": w, "height": h, "add": shift }),
            );
            return;
        }
        let Some(click) = self.click_at(x, y, w, h) else {
            self.message = Some(("The sketch plane is edge-on: turn the view".into(), true));
            return;
        };
        self.sketch.clicks.push(click);
        self.place(tool);
    }

    /// Creates geometry once a tool has all its clicks.
    fn place(&mut self, tool: Tool) {
        let c = self.sketch.clicks.clone();
        match (tool, c.as_slice()) {
            (Tool::Line, [a, b]) => {
                if a.at.coincident(b.at) {
                    self.sketch.clicks.pop();
                    return;
                }
                let mut p = json!({
                    "x1": a.at.x, "y1": a.at.y, "x2": b.at.x, "y2": b.at.y, "snap": true,
                });
                if let Some(id) = a.point {
                    p["start"] = json!(id);
                }
                if let Some(id) = b.point {
                    p["end"] = json!(id);
                }
                match self.run_sketch("sketch.line", p) {
                    Some(r) => {
                        let start = as_id(&r["start"]);
                        let end = as_id(&r["end"]);
                        if self.sketch.chain_start.is_none() {
                            self.sketch.chain_start = start;
                        }
                        let closed = end.is_some() && end == self.sketch.chain_start;
                        self.sketch.clicks.clear();
                        if closed {
                            self.sketch.chain_start = None;
                        } else {
                            self.sketch.clicks.push(Click {
                                at: b.at,
                                point: end,
                            });
                        }
                    }
                    None => {
                        self.sketch.clicks.pop();
                    }
                }
            }
            (Tool::Rectangle, [a, b]) => {
                self.run_sketch(
                    "sketch.rectangle",
                    json!({ "x1": a.at.x, "y1": a.at.y, "x2": b.at.x, "y2": b.at.y }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::Circle, [a, b]) => {
                let r = a.at.distance(b.at);
                self.run_sketch(
                    "sketch.circle",
                    json!({ "cx": a.at.x, "cy": a.at.y, "radius": r }),
                );
                self.sketch.clicks.clear();
            }
            (Tool::Arc, [c0, s, e]) => {
                self.run_sketch(
                    "sketch.arc",
                    json!({
                        "cx": c0.at.x, "cy": c0.at.y,
                        "x1": s.at.x, "y1": s.at.y, "x2": e.at.x, "y2": e.at.y,
                    }),
                );
                self.sketch.clicks.clear();
            }
            _ => {}
        }
    }

    /// The preview overlay for the tool in progress, in world coordinates.
    pub(crate) fn sketch_preview(&self) -> Option<Overlay> {
        if self.sketch.tool() == Tool::Select {
            return None;
        }
        let f = self.session.active_sketch()?;
        let hover = self.sketch.hover?;
        let c = &self.sketch.clicks;
        let w = |p: Point2| f.plane.from_plane(p);
        let mut lines: Vec<[Point3; 2]> = Vec::new();
        let ring = |centre: Point2, r: f64, a0: f64, sweep: f64| -> Vec<[Point3; 2]> {
            let n = 64;
            (0..n)
                .map(|k| {
                    let t0 = a0 + sweep * k as f64 / n as f64;
                    let t1 = a0 + sweep * (k + 1) as f64 / n as f64;
                    [
                        w(Point2::new(
                            centre.x + r * t0.cos(),
                            centre.y + r * t0.sin(),
                        )),
                        w(Point2::new(
                            centre.x + r * t1.cos(),
                            centre.y + r * t1.sin(),
                        )),
                    ]
                })
                .collect()
        };
        match (self.sketch.tool(), c.as_slice()) {
            (Tool::Line, [a]) => lines.push([w(a.at), w(hover)]),
            (Tool::Rectangle, [a]) => {
                let q = [
                    a.at,
                    Point2::new(hover.x, a.at.y),
                    hover,
                    Point2::new(a.at.x, hover.y),
                ];
                for k in 0..4 {
                    lines.push([w(q[k]), w(q[(k + 1) % 4])]);
                }
            }
            (Tool::Circle, [a]) => {
                lines.extend(ring(a.at, a.at.distance(hover), 0.0, std::f64::consts::TAU));
            }
            (Tool::Arc, [a]) => lines.push([w(a.at), w(hover)]),
            (Tool::Arc, [a, s]) => {
                let a0 = (s.at - a.at).angle();
                let mut sweep = (hover - a.at).angle() - a0;
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                lines.extend(ring(a.at, a.at.distance(s.at), a0, sweep));
            }
            _ => {}
        }
        Some(Overlay {
            style: OverlayStyle::Preview,
            lines,
            points: vec![w(hover)],
            on_top: true,
        })
    }

    /// Projects a world point to the viewport rect.
    fn project(&self, rect: Rect, p: Point3) -> Option<Pos2> {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let bounds = self.session.scene.bounds();
        let vp = mat_mul(
            &self
                .session
                .camera
                .projection_matrix(w / h, bounds.as_ref()),
            &self.session.camera.view_matrix(),
        );
        let n = transform_point(&vp, p)?;
        if !(0.0..=1.0).contains(&n[2]) {
            return None;
        }
        Some(Pos2::new(
            rect.min.x + ((n[0] + 1.0) * 0.5 * w) as f32,
            rect.min.y + ((1.0 - n[1]) * 0.5 * h) as f32,
        ))
    }

    /// Dimension labels over the viewport; click one to edit its value.
    pub(crate) fn dimension_labels(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let labels = self.session.dimension_labels();
        let font = egui::FontId::proportional(13.0);
        let text_c = color(self.theme.text);
        let bg = color(self.theme.panel).gamma_multiply(0.92);
        let editing_id = self.sketch.editing.as_ref().map(|e| e.0);
        for l in labels {
            let Some(pos) = self.project(rect, l.anchor) else {
                continue;
            };
            if Some(u64::from(l.constraint)) == editing_id {
                continue;
            }
            let expression = self
                .session
                .active_sketch()
                .and_then(|f| f.expressions.get(&l.constraint).cloned());
            let text = match &expression {
                Some(e) => format!("{} = {e}", display_value(&l.kind, l.value)),
                None => display_value(&l.kind, l.value),
            };
            let galley = ui
                .painter()
                .layout_no_wrap(text.clone(), font.clone(), text_c);
            let r = Rect::from_center_size(pos, galley.size() + Vec2::new(10.0, 4.0));
            let id = ui.scope_id().with(("dimension", l.constraint));
            let resp = ui.interact(r, id, egui::Sense::click());
            let border = if resp.hovered() {
                color(self.theme.accent)
            } else {
                color(self.theme.text_muted)
            };
            ui.painter().rect_filled(r, 3.0, bg);
            ui.painter()
                .rect_stroke(r, 3.0, (1.0, border), egui::StrokeKind::Inside);
            ui.painter()
                .galley(r.center() - galley.size() * 0.5, galley, text_c);
            if resp.clicked() {
                let start = expression.unwrap_or_else(|| display_value(&l.kind, l.value));
                self.sketch.editing = Some((u64::from(l.constraint), l.kind.clone(), start));
            }
        }
        self.dimension_editor(ui, rect);
    }

    /// The inline value box for the dimension being edited. Enter applies, Esc cancels.
    fn dimension_editor(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let Some((id, kind, mut text)) = self.sketch.editing.take() else {
            return;
        };
        let anchor = self
            .session
            .dimension_labels()
            .into_iter()
            .find(|l| u64::from(l.constraint) == id)
            .and_then(|l| self.project(rect, l.anchor))
            .unwrap_or(rect.center());
        let mut keep = true;
        let area = egui::Area::new(ui.scope_id().with(("dimension editor", id)))
            .fixed_pos(anchor - Vec2::new(50.0, 12.0))
            .order(egui::Order::Foreground);
        area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(70.0)
                            .id(ui.scope_id().with(("dimension value", id))),
                    );
                    edit.request_focus();
                    ui.label(if kind == "angle" { "°" } else { "mm" });
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if esc {
                        keep = false;
                    } else if enter {
                        // A plain number sets the value; anything else (a parameter, a
                        // formula) becomes an expression the dimension follows.
                        let params = match parse_value(&kind, &text) {
                            Some(v) => json!({ "constraint": id, "value": v }),
                            None => json!({ "constraint": id, "expression": text.trim() }),
                        };
                        if self.run_sketch("sketch.dimension", params).is_some() {
                            keep = false;
                        }
                    }
                });
            });
        });
        if keep {
            self.sketch.editing = Some((id, kind, text));
        }
    }
}

/// A dimension value as the user reads it: mm with two decimals, angles in degrees.
fn display_value(kind: &str, v: f64) -> String {
    let text = if kind == "angle" {
        format!("{:.2}", v.to_degrees())
    } else {
        format!("{v:.2}")
    };
    let text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    match kind {
        "diameter" => format!("Ø{text}"),
        "radius" => format!("R{text}"),
        "angle" => format!("{text}°"),
        _ => text,
    }
}

/// Parses a typed dimension value (mm, or degrees for angles) to internal units.
fn parse_value(kind: &str, text: &str) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .trim_start_matches(['Ø', 'ø', 'R', 'r'])
        .trim_end_matches(['°', 'm'])
        .trim()
        .replace(',', ".");
    let v: f64 = cleaned.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(if kind == "angle" { v.to_radians() } else { v })
}

/// The muted colour for hints in the status bar.
pub(crate) fn hint_color(app: &ChamferaApp) -> Color32 {
    color(app.theme.text_muted)
}

pub(crate) fn tool_hint(app: &ChamferaApp) -> &'static str {
    app.sketch.tool().hint(app.sketch.clicks.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A top view of the XY plane, 800 x 600, centred on (40, 25) at a distance that makes
    /// the pixel scale easy: `px(x, y)` is the pixel of sketch point (x, y).
    fn app_on_top_plane() -> (ChamferaApp, impl Fn(f64, f64) -> (f64, f64)) {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.session.camera.target = chamfera_geom::Point3::new(40.0, 25.0, 0.0);
        app.session.camera.projection = chamfera_render::Projection::Orthographic;
        let cam = app.session.camera;
        // Orthographic: the view is 2 * half_height tall over 600 px.
        let per_mm = 600.0 / (2.0 * cam.half_height());
        let px = move |x: f64, y: f64| (400.0 + (x - 40.0) * per_mm, 300.0 - (y - 25.0) * per_mm);
        (app, px)
    }

    fn click(app: &mut ChamferaApp, p: (f64, f64)) {
        app.sketch_click(p.0, p.1, 800.0, 600.0, false);
    }

    #[test]
    fn four_line_clicks_ending_on_the_first_point_close_a_profile() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Line);
        for (x, y) in [
            (0.0, 0.0),
            (80.0, 0.0),
            (80.0, 50.0),
            (0.0, 50.0),
            (0.0, 0.0),
        ] {
            click(&mut app, px(x, y));
        }
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        assert_eq!(g["curves"].as_array().unwrap().len(), 4);
        // Shared corners: 4 points, not 8, and the chain ended on closing.
        assert_eq!(g["points"].as_array().unwrap().len(), 4);
        assert!(app.sketch.clicks.is_empty());
        let area = g["profiles"][0]["area"].as_f64().unwrap();
        // Clicks land on pixel centres of an orthographic view, so allow 0.5 px of error
        // per side: the rectangle is 80 x 50 to within that.
        assert!((area - 4000.0).abs() < 4000.0 * 0.02, "{area}");
        // Snapping made the four sides horizontal and vertical.
        let snaps = g["constraints"].as_array().unwrap().len();
        assert_eq!(snaps, 4);
    }

    #[test]
    fn rectangle_and_circle_tools_take_two_clicks_each() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Rectangle);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(80.0, 50.0));
        app.sketch.tool = Some(Tool::Circle);
        click(&mut app, px(20.0, 25.0));
        click(&mut app, px(28.0, 25.0));
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let profiles = g["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 2, "plate with a hole, and the disc");
        assert!(profiles.iter().any(|r| r["holes"] == 1));
    }

    #[test]
    fn clicking_selects_and_dimension_edits_resize() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Rectangle);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(80.0, 50.0));
        app.sketch.tool = Some(Tool::Select);
        // The bottom edge, away from its corners.
        click(&mut app, px(40.0, 0.0));
        assert_eq!(app.session.sketch_selection.len(), 1);
        let r = app
            .run_sketch("sketch.dimension_selection", json!({}))
            .unwrap();
        let id = r["constraint"].as_u64().unwrap();
        app.run_sketch(
            "sketch.dimension",
            json!({ "constraint": id, "value": 100 }),
        );
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let xs: Vec<f64> = g["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["x"].as_f64().unwrap())
            .collect();
        let width = xs.iter().cloned().fold(f64::MIN, f64::max)
            - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!((width - 100.0).abs() < 1e-6, "{width}");
        assert!(
            app.sketch
                .status
                .as_deref()
                .unwrap_or("")
                .contains("degrees of freedom")
        );
    }

    #[test]
    fn dimension_values_read_and_parse_both_ways() {
        assert_eq!(display_value("length", 50.0), "50");
        assert_eq!(display_value("length", 12.345), "12.35");
        assert_eq!(display_value("diameter", 16.0), "Ø16");
        assert_eq!(display_value("radius", 8.5), "R8.5");
        assert_eq!(display_value("angle", std::f64::consts::FRAC_PI_2), "90°");
        assert_eq!(parse_value("length", " 42,5 mm"), Some(42.5));
        assert_eq!(parse_value("diameter", "Ø16"), Some(16.0));
        let a = parse_value("angle", "45°").unwrap();
        assert!((a - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert_eq!(parse_value("length", "abc"), None);
        assert_eq!(parse_value("length", "inf"), None);
    }
}

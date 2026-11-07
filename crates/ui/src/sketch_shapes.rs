//! The shape tools of the sketch ribbon (Polygon, Slot, Ellipse, 3-Point Rectangle,
//! 2-Point, 3-Point and Tangent Circle), Project, which brings model edges into the
//! sketch, and Move/Copy, which moves, turns or scales the selection by typed values.
//!
//! `sketch_ui.rs` routes these tools here: their names and hints, the clicks that place
//! them, their previews and their option box. Like the rest of the UI they act only
//! through commands (`sketch.polygon`, `sketch.slot`, `sketch.ellipse`,
//! `sketch.rectangle.three_point`, `sketch.circle.two_point`, `sketch.circle.three_point`,
//! `sketch.circle.tangent`, `sketch.project`, `sketch.move`, `sketch.rotate`,
//! `sketch.scale`).

use crate::icons::Icon;
use crate::sketch_ui::Tool;
use crate::widgets;
use crate::{ChamferaApp, label};
use chamfera_engine::sketching::curve_polyline;
use chamfera_geom::Point2;
use chamfera_sketch::ToolOp;
use egui::{Rect, Vec2};
use serde_json::{Value, json};

/// What the Move/Copy tool does to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// The values the shape tools use, as set in their option box.
pub struct ShapeOptions {
    pub sides: String,
    pub circumscribed: bool,
    pub overall: bool,
    pub linked: bool,
    /// Tangent Circle: touch three curves (else two, with `radius`).
    pub tangent_three: bool,
    pub radius: String,
    /// The curves the Tangent Circle tool has picked, one per click so far.
    pub(crate) tangent_curves: Vec<u32>,
    pub mode: TransformMode,
    pub dx: String,
    pub dy: String,
    pub angle: String,
    pub factor: String,
    pub cx: String,
    pub cy: String,
    pub copy: bool,
}

impl Default for ShapeOptions {
    fn default() -> Self {
        Self {
            sides: "6".into(),
            circumscribed: false,
            overall: false,
            linked: true,
            tangent_three: true,
            radius: "5".into(),
            tangent_curves: Vec::new(),
            mode: TransformMode::Move,
            dx: "10".into(),
            dy: "0".into(),
            angle: "90".into(),
            factor: "2".into(),
            cx: "0".into(),
            cy: "0".into(),
            copy: false,
        }
    }
}

/// True for the tools this module handles.
pub(crate) fn is_shape(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Polygon
            | Tool::Slot
            | Tool::Ellipse
            | Tool::Project
            | Tool::ThreePointRectangle
            | Tool::TwoPointCircle
            | Tool::ThreePointCircle
            | Tool::TangentCircle
            | Tool::Transform
    )
}

/// True for the tools whose clicks pick existing geometry by pixel instead of placing
/// points.
pub(crate) fn picks(tool: Tool) -> bool {
    matches!(tool, Tool::Project | Tool::TangentCircle)
}

pub(crate) fn name(tool: Tool) -> &'static str {
    match tool {
        Tool::Polygon => label("sketch.polygon"),
        Tool::Slot => label("sketch.slot"),
        Tool::Ellipse => label("sketch.ellipse"),
        Tool::ThreePointRectangle => label("sketch.rectangle.three_point"),
        Tool::TwoPointCircle => label("sketch.circle.two_point"),
        Tool::ThreePointCircle => label("sketch.circle.three_point"),
        Tool::TangentCircle => label("sketch.circle.tangent"),
        Tool::Transform => "Move/Copy",
        _ => label("sketch.project"),
    }
}

pub(crate) fn icon(tool: Tool) -> Icon {
    match tool {
        Tool::Polygon => Icon::Polygon,
        Tool::Slot => Icon::Slot,
        Tool::Ellipse => Icon::Ellipse,
        Tool::ThreePointRectangle => Icon::RectangleThreePoint,
        Tool::TwoPointCircle => Icon::CircleTwoPoint,
        Tool::ThreePointCircle => Icon::CircleThreePoint,
        Tool::TangentCircle => Icon::CircleTangent,
        Tool::Transform => Icon::SketchTransform,
        _ => Icon::Project,
    }
}

pub(crate) fn tooltip(tool: Tool) -> &'static str {
    match tool {
        Tool::Polygon => {
            "Polygon\nA regular polygon by its centre and a corner, inscribed in a circle or around it"
        }
        Tool::Slot => "Slot\nA straight slot by the centres of its ends, then its width",
        Tool::Ellipse => {
            "Ellipse\nAn ellipse by its centre, the end of one axis, then the other axis"
        }
        Tool::ThreePointRectangle => {
            "3-Point Rectangle\nA rectangle at any angle: two clicks for one side, a third for the opposite side"
        }
        Tool::TwoPointCircle => "2-Point Circle\nTwo clicks: the ends of a diameter",
        Tool::ThreePointCircle => "3-Point Circle\nThree clicks: points the circle passes through",
        Tool::TangentCircle => {
            "Tangent Circle\nClick three lines, circles or arcs to touch; or two, with the radius in the tool box"
        }
        Tool::Transform => {
            "Move/Copy\nMove, turn or scale the selected geometry by typed values, or a moved copy of it"
        }
        _ => "Project\nCopy model edges into the sketch, linked so they follow the model",
    }
}

pub(crate) fn hint(tool: Tool, clicks: usize) -> &'static str {
    match (tool, clicks) {
        (Tool::Polygon, 0) => "Click the centre of the polygon",
        (Tool::Polygon, _) => "Click a corner (or the middle of a side when circumscribed)",
        (Tool::Slot, 0) => "Click the centre of one end",
        (Tool::Slot, 1) => "Click the centre of the other end",
        (Tool::Slot, _) => "Click to set the width",
        (Tool::Ellipse, 0) => "Click the centre",
        (Tool::Ellipse, 1) => "Click the end of the first axis",
        (Tool::Ellipse, _) => "Click to set the other axis",
        (Tool::ThreePointRectangle, 0) => "Click the start of one side",
        (Tool::ThreePointRectangle, 1) => "Click the end of that side",
        (Tool::ThreePointRectangle, _) => "Click the opposite side",
        (Tool::TwoPointCircle, 0) => "Click one end of a diameter",
        (Tool::TwoPointCircle, _) => "Click the other end of the diameter",
        (Tool::ThreePointCircle, 0) => "Click a point on the circle",
        (Tool::ThreePointCircle, 1) => "Click a second point on the circle",
        (Tool::ThreePointCircle, _) => "Click a third point on the circle",
        (Tool::TangentCircle, _) => {
            "Click the lines, circles or arcs to touch, near where the circle should be · Esc cancels"
        }
        (Tool::Transform, 0) => {
            "Select geometry, set the values and Apply · or click the base point (the centre to turn or scale about)"
        }
        (Tool::Transform, _) => "Click where the base point goes · Esc cancels",
        _ => "Click a model edge, or a face to project all its edges · Esc cancels",
    }
}

/// How many clicks a shape takes.
fn clicks_needed(tool: Tool, o: &ShapeOptions) -> usize {
    match tool {
        Tool::Polygon | Tool::TwoPointCircle => 2,
        Tool::Slot | Tool::Ellipse | Tool::ThreePointRectangle | Tool::ThreePointCircle => 3,
        Tool::TangentCircle if o.tangent_three => 3,
        Tool::TangentCircle => 2,
        Tool::Transform if o.mode == TransformMode::Move => 2,
        _ => 1,
    }
}

/// The distance of `p` from the line through `a` and `b` (0 when they coincide).
fn off_line(a: Point2, b: Point2, p: Point2) -> f64 {
    (b - a).normalized().map_or(0.0, |u| u.cross(p - a).abs())
}

impl ChamferaApp {
    /// The command, its parameters and the operation (for the preview) for a shape whose
    /// clicks are `pts`; `None` until it has them all or when they do not make a shape.
    fn shape_op(&self, tool: Tool, pts: &[Point2]) -> Option<(&'static str, Value, ToolOp)> {
        let o = &self.sketch.shapes;
        match (tool, pts) {
            (Tool::Polygon, [c, k]) => {
                let sides: u32 = o.sides.trim().parse().ok()?;
                let mode = if o.circumscribed {
                    "circumscribed"
                } else {
                    "inscribed"
                };
                Some((
                    "sketch.polygon",
                    json!({
                        "cx": c.x, "cy": c.y, "x": k.x, "y": k.y, "sides": sides, "mode": mode,
                    }),
                    ToolOp::Polygon {
                        centre: *c,
                        corner: *k,
                        sides,
                        inscribed: !o.circumscribed,
                    },
                ))
            }
            (Tool::Slot, [a, b, w]) => {
                let width = 2.0 * off_line(*a, *b, *w);
                Some((
                    "sketch.slot",
                    json!({
                        "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y,
                        "width": width, "overall": o.overall,
                    }),
                    ToolOp::Slot {
                        start: *a,
                        end: *b,
                        width,
                        overall: o.overall,
                    },
                ))
            }
            (Tool::Ellipse, [c, m, n]) => {
                let minor = off_line(*c, *m, *n);
                Some((
                    "sketch.ellipse",
                    json!({ "cx": c.x, "cy": c.y, "x": m.x, "y": m.y, "minor": minor }),
                    ToolOp::Ellipse {
                        centre: *c,
                        major: *m,
                        minor,
                    },
                ))
            }
            (Tool::ThreePointRectangle, [a, b, c]) => Some((
                "sketch.rectangle.three_point",
                json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y, "x3": c.x, "y3": c.y }),
                ToolOp::ThreePointRectangle {
                    a: *a,
                    b: *b,
                    c: *c,
                },
            )),
            (Tool::TwoPointCircle, [a, b]) => Some((
                "sketch.circle.two_point",
                json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y }),
                ToolOp::TwoPointCircle { a: *a, b: *b },
            )),
            (Tool::ThreePointCircle, [a, b, c]) => Some((
                "sketch.circle.three_point",
                json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y, "x3": c.x, "y3": c.y }),
                ToolOp::ThreePointCircle {
                    a: *a,
                    b: *b,
                    c: *c,
                },
            )),
            _ => None,
        }
    }

    /// A length typed in the shape tool box, in the design's unit unless it says another.
    fn typed(&self, kind: &str, text: &str) -> Option<f64> {
        crate::sketch_ui::parse_value(self.units(), kind, text)
    }

    /// The command and parameters of the Move/Copy tool with the values in its box (and
    /// for a move, `delta` from two clicks); `None` when a value does not read.
    fn transform_command(&self, delta: Option<(f64, f64)>) -> Option<(&'static str, Value)> {
        let o = &self.sketch.shapes;
        let centre = || Some((self.typed("length", &o.cx)?, self.typed("length", &o.cy)?));
        Some(match o.mode {
            TransformMode::Move => {
                let (dx, dy) = match delta {
                    Some(d) => d,
                    None => (self.typed("length", &o.dx)?, self.typed("length", &o.dy)?),
                };
                ("sketch.move", json!({ "dx": dx, "dy": dy, "copy": o.copy }))
            }
            TransformMode::Rotate => {
                let (cx, cy) = centre()?;
                let angle = self.typed("angle", &o.angle)?;
                (
                    "sketch.rotate",
                    json!({ "cx": cx, "cy": cy, "angle": angle, "copy": o.copy }),
                )
            }
            TransformMode::Scale => {
                let (cx, cy) = centre()?;
                let factor: f64 = o.factor.trim().replace(',', ".").parse().ok()?;
                (
                    "sketch.scale",
                    json!({ "cx": cx, "cy": cy, "factor": factor, "copy": o.copy }),
                )
            }
        })
    }

    /// Runs the Move/Copy tool on the selection.
    fn apply_transform(&mut self, delta: Option<(f64, f64)>) {
        match self.transform_command(delta) {
            Some((id, params)) => {
                self.run_sketch(id, params);
            }
            None => {
                self.message = Some((
                    "Move/Copy: type numbers for the values (a scale factor above 0)".into(),
                    true,
                ));
            }
        }
    }

    /// Places a shape once it has all its clicks. Returns true when the clicks are used
    /// up (placed, or refused with a message) and should be cleared.
    pub(crate) fn place_shape(&mut self, tool: Tool, pts: &[Point2]) -> bool {
        if pts.len() < clicks_needed(tool, &self.sketch.shapes) {
            return false;
        }
        if tool == Tool::Transform {
            match (self.sketch.shapes.mode, pts) {
                (TransformMode::Move, [a, b]) => self.apply_transform(Some((b.x - a.x, b.y - a.y))),
                (_, [c, ..]) => {
                    // The centre to turn or scale about; Apply runs it.
                    let per = self.units().length.mm();
                    let show = |v: f64| ((v / per) * 1e4).round() / 1e4;
                    self.sketch.shapes.cx = show(c.x).to_string();
                    self.sketch.shapes.cy = show(c.y).to_string();
                }
                _ => {}
            }
            return true;
        }
        match self.shape_op(tool, pts) {
            Some((id, params, _)) => {
                self.run_sketch(id, params);
            }
            None => {
                self.message = Some((
                    format!("{}: type a whole number of sides from 3 to 64", name(tool)),
                    true,
                ));
            }
        }
        true
    }

    /// The preview of a shape being placed: its clicks so far and the cursor as the next.
    pub(crate) fn shape_preview(
        &self,
        tool: Tool,
        pts: &[Point2],
        hover: Point2,
    ) -> Vec<Vec<Point2>> {
        let mut all = pts.to_vec();
        all.push(hover);
        if all.len() == 2 && clicks_needed(tool, &self.sketch.shapes) == 3 {
            return vec![all];
        }
        let Some((_, _, op)) = self.shape_op(tool, &all) else {
            return Vec::new();
        };
        let Some(f) = self.session.active_sketch() else {
            return Vec::new();
        };
        let Some((after, report)) = f.sketch.preview_tool(&op) else {
            return Vec::new();
        };
        report
            .created_curves
            .iter()
            .filter_map(|c| after.curve(*c))
            .filter(|c| !c.is_construction())
            .map(|c| curve_polyline(&after, c))
            .collect()
    }

    /// A click of a tool that picks geometry (`picks`) at pixel `(x, y)`, sketch point
    /// `at`.
    pub(crate) fn pick_click(&mut self, tool: Tool, x: f64, y: f64, w: f64, h: f64, at: Point2) {
        if tool != Tool::TangentCircle {
            self.project_click(x, y, w, h);
            return;
        }
        let Some(chamfera_engine::sketching::SketchItem::Curve(c)) =
            self.session.pick_sketch_item(x, y, w, h)
        else {
            self.message = Some(("Tangent Circle: click a line, circle or arc".into(), true));
            return;
        };
        // The picks so far follow the clicks (a reset clears both).
        let n = self.sketch.clicks.len();
        let o = &mut self.sketch.shapes;
        o.tangent_curves.truncate(n);
        if o.tangent_curves.contains(&c) {
            return;
        }
        o.tangent_curves.push(c);
        self.sketch.clicks.push(crate::sketch_ui::Click {
            at,
            point: None,
            snap: None,
        });
        if self.sketch.clicks.len() < clicks_needed(tool, &self.sketch.shapes) {
            return;
        }
        // The guess for the centre: the middle of the clicks, which are on the curves near
        // where the circle should touch them.
        let k = self.sketch.clicks.len() as f64;
        let (sx, sy) = self
            .sketch
            .clicks
            .iter()
            .fold((0.0, 0.0), |(x, y), c| (x + c.at.x, y + c.at.y));
        let curves = std::mem::take(&mut self.sketch.shapes.tangent_curves);
        self.sketch.clicks.clear();
        let mut params = json!({ "curves": curves, "x": sx / k, "y": sy / k });
        if !self.sketch.shapes.tangent_three {
            match self.typed("length", &self.sketch.shapes.radius.clone()) {
                Some(r) if r > 0.0 => params["radius"] = json!(r),
                _ => {
                    self.message = Some(("Tangent Circle: type a radius above 0".into(), true));
                    return;
                }
            }
        }
        self.run_sketch("sketch.circle.tangent", params);
    }

    /// A Project click at pixel `(x, y)`: the model edge there, or else every edge of the
    /// face there.
    pub(crate) fn project_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let params = json!({
            "pick": { "x": x, "y": y, "width": w, "height": h },
            "linked": self.sketch.shapes.linked,
        });
        if let Some(r) = self.run_sketch("sketch.project", params) {
            let n = r["curves"].as_array().map_or(0, Vec::len);
            self.message = Some((
                match n {
                    0 => "Project: those edges are in the sketch already".to_owned(),
                    1 => "Projected 1 edge".to_owned(),
                    n => format!("Projected {n} edges"),
                },
                false,
            ));
        }
    }

    /// The option box of the shape tools that take values, at the viewport's top left.
    pub(crate) fn shape_tool_box(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let tool = self.sketch.tool();
        if !matches!(
            tool,
            Tool::Polygon | Tool::Slot | Tool::Project | Tool::TangentCircle | Tool::Transform
        ) {
            return;
        }
        let look = widgets::Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let unit = self.session.doc.units.length.symbol();
        let selected = self.session.sketch_selection.len();
        let mut apply = false;
        let field = |ui: &mut egui::Ui, name: &str, text: &mut String, unit: &str| {
            ui.horizontal(|ui| {
                widgets::field_label(ui, look, name);
                ui.add(egui::TextEdit::singleline(text).desired_width(64.0));
                ui.label(egui::RichText::new(unit).color(look.c(|t| t.text_muted)));
            });
        };
        let o = &mut self.sketch.shapes;
        egui::Area::new(ui.scope_id().with("sketch shape box"))
            .fixed_pos(rect.min + Vec2::new(widgets::S5, widgets::S5))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::same(widgets::S4 as i8))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = widgets::S2;
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                            look.icon(ui, icon(tool), r, true);
                            ui.label(
                                egui::RichText::new(name(tool))
                                    .size(widgets::T_TITLE)
                                    .color(look.c(|t| t.text)),
                            );
                        });
                        match tool {
                            Tool::Polygon => {
                                ui.horizontal(|ui| {
                                    widgets::field_label(ui, look, "Sides");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut o.sides)
                                            .desired_width(64.0),
                                    );
                                });
                                ui.checkbox(
                                    &mut o.circumscribed,
                                    "Circumscribed (around the circle)",
                                );
                            }
                            Tool::Slot => {
                                ui.checkbox(&mut o.overall, "Points are the tips (overall length)");
                            }
                            Tool::TangentCircle => {
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut o.tangent_three, true, "Three curves");
                                    ui.radio_value(&mut o.tangent_three, false, "Two and a radius");
                                });
                                if !o.tangent_three {
                                    field(ui, "Radius", &mut o.radius, unit);
                                }
                            }
                            Tool::Transform => {
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut o.mode, TransformMode::Move, "Move");
                                    ui.radio_value(&mut o.mode, TransformMode::Rotate, "Rotate");
                                    ui.radio_value(&mut o.mode, TransformMode::Scale, "Scale");
                                });
                                match o.mode {
                                    TransformMode::Move => {
                                        field(ui, "Move X", &mut o.dx, unit);
                                        field(ui, "Move Y", &mut o.dy, unit);
                                    }
                                    TransformMode::Rotate => {
                                        field(ui, "Angle", &mut o.angle, "°");
                                        field(ui, "Centre X", &mut o.cx, unit);
                                        field(ui, "Centre Y", &mut o.cy, unit);
                                    }
                                    TransformMode::Scale => {
                                        field(ui, "Factor", &mut o.factor, "×");
                                        field(ui, "Centre X", &mut o.cx, unit);
                                        field(ui, "Centre Y", &mut o.cy, unit);
                                    }
                                }
                                ui.checkbox(&mut o.copy, "Copy (keep the original)");
                                ui.horizontal(|ui| {
                                    widgets::caption(ui, look, &format!("{selected} selected"));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            apply = widgets::primary_button(
                                                ui,
                                                look,
                                                "Apply",
                                                selected > 0,
                                            )
                                            .clicked();
                                        },
                                    );
                                });
                            }
                            _ => {
                                ui.checkbox(&mut o.linked, "Linked: follow the model");
                            }
                        }
                    });
            });
        if apply {
            self.apply_transform(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch_ui::tool_hint;

    /// A top view of the XY plane, 800 x 600, centred on (40, 25): `px(x, y)` is the
    /// pixel of sketch point (x, y).
    fn app_on_top_plane() -> (ChamferaApp, impl Fn(f64, f64) -> (f64, f64)) {
        let mut app = ChamferaApp::headless();
        // These tests measure the clicks themselves: no alignment guide may pull them
        // (the slot's width click at y = 13 would line up with the square's corner y = 15).
        app.prefs.snaps.alignment = false;
        app.create_sketch("xy", "top");
        app.session.camera.target = chamfera_geom::Point3::new(40.0, 25.0, 0.0);
        app.session.camera.projection = chamfera_render::Projection::Orthographic;
        let cam = app.session.camera;
        let per_mm = 600.0 / (2.0 * cam.half_height());
        let px = move |x: f64, y: f64| (400.0 + (x - 40.0) * per_mm, 300.0 - (y - 25.0) * per_mm);
        (app, px)
    }

    fn click(app: &mut ChamferaApp, p: (f64, f64)) {
        app.sketch_click(p.0, p.1, 800.0, 600.0, false);
    }

    fn areas(app: &mut ChamferaApp) -> Vec<f64> {
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let mut a: Vec<f64> = g["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["area"].as_f64().unwrap())
            .collect();
        a.sort_by(f64::total_cmp);
        a
    }

    #[test]
    fn polygon_slot_and_ellipse_tools_take_their_clicks() {
        let (mut app, px) = app_on_top_plane();
        let pi = std::f64::consts::PI;
        // A square inscribed in r = 10 (clicks land within half a pixel): area 2 r².
        app.sketch.tool = Some(Tool::Polygon);
        app.sketch.shapes.sides = "4".into();
        click(&mut app, px(20.0, 25.0));
        assert!(tool_hint(&app).contains("corner"));
        click(&mut app, px(30.0, 25.0));
        let a = areas(&mut app);
        assert!((a[0] - 200.0).abs() < 200.0 * 0.03, "{a:?}");
        // A slot with centres 20 apart and 6 wide: 120 + 9π.
        app.sketch.tool = Some(Tool::Slot);
        click(&mut app, px(45.0, 10.0));
        click(&mut app, px(65.0, 10.0));
        assert!(tool_hint(&app).contains("width"));
        // The preview shows the slot while the width is chosen.
        let preview = app.shape_preview(
            Tool::Slot,
            &[Point2::new(45.0, 10.0), Point2::new(65.0, 10.0)],
            Point2::new(55.0, 13.0),
        );
        assert_eq!(preview.len(), 4, "two lines and two arcs");
        click(&mut app, px(55.0, 13.0));
        // An ellipse a = 10, b = 5: 50π.
        app.sketch.tool = Some(Tool::Ellipse);
        click(&mut app, px(55.0, 35.0));
        click(&mut app, px(65.0, 35.0));
        click(&mut app, px(55.0, 40.0));
        let a = areas(&mut app);
        assert_eq!(a.len(), 3, "{a:?}");
        // Smallest first: the slot (148.3), the ellipse (157.1), the square.
        for (got, want) in a.iter().zip([120.0 + 9.0 * pi, 50.0 * pi, 200.0]) {
            assert!((got - want).abs() < want * 0.05, "{a:?}");
        }
        // A bad side count says so and draws nothing.
        app.sketch.tool = Some(Tool::Polygon);
        app.sketch.shapes.sides = "two".into();
        click(&mut app, px(20.0, 45.0));
        click(&mut app, px(25.0, 45.0));
        assert!(app.message.is_some());
        assert_eq!(areas(&mut app).len(), 3);
    }

    /// A 3-point rectangle with side (0, 0)-(20, 0) and the third click 10 up: 200. A
    /// 2-point circle across (40, 0)-(40, 10): r = 5, 25π. A 3-point circle through
    /// (60, 30), (70, 30), (60, 40): the right angle's hypotenuse (10√2) is a diameter, so
    /// r² = 50 and the area 50π. Clicks land within half a pixel, so 3% tolerance.
    #[test]
    fn rectangle_and_circle_tools_by_points_take_their_clicks() {
        let (mut app, px) = app_on_top_plane();
        let pi = std::f64::consts::PI;
        app.sketch.tool = Some(Tool::ThreePointRectangle);
        click(&mut app, px(0.0, 0.0));
        click(&mut app, px(20.0, 0.0));
        assert!(tool_hint(&app).contains("opposite"));
        click(&mut app, px(8.0, 10.0));
        app.sketch.tool = Some(Tool::TwoPointCircle);
        click(&mut app, px(40.0, 0.0));
        click(&mut app, px(40.0, 10.0));
        app.sketch.tool = Some(Tool::ThreePointCircle);
        click(&mut app, px(60.0, 30.0));
        click(&mut app, px(70.0, 30.0));
        click(&mut app, px(60.0, 40.0));
        let a = areas(&mut app);
        assert_eq!(a.len(), 3, "{a:?}");
        for (got, want) in a.iter().zip([25.0 * pi, 50.0 * pi, 200.0]) {
            assert!((got - want).abs() < want * 0.03, "{a:?}");
        }
    }

    /// Two lines along the axes from (0, 0) and the Tangent Circle tool with two curves
    /// and radius 5: one circle of radius 5 in the corner.
    #[test]
    fn the_tangent_circle_tool_picks_its_curves() {
        let (mut app, px) = app_on_top_plane();
        app.run_sketch("sketch.line", json!({"x1": 0, "y1": 0, "x2": 50, "y2": 0}));
        app.run_sketch("sketch.line", json!({"x1": 0, "y1": 0, "x2": 0, "y2": 40}));
        app.sketch.tool = Some(Tool::TangentCircle);
        app.sketch.shapes.tangent_three = false;
        app.sketch.shapes.radius = "5".into();
        click(&mut app, px(20.0, 0.0));
        click(&mut app, px(0.0, 20.0));
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let r: Vec<f64> = g["curves"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["geometry"]["radius"].as_f64())
            .collect();
        assert_eq!(r.len(), 1, "{g}");
        assert!((r[0] - 5.0).abs() < 1e-9);
        // Empty space picks nothing.
        click(&mut app, px(30.0, 30.0));
        assert!(app.message.is_some());
    }

    /// A 20 x 10 rectangle selected and copied by the Move/Copy tool's typed values
    /// (30, 0): two profiles of 200.
    #[test]
    fn the_move_copy_tool_applies_its_values_to_the_selection() {
        let (mut app, _) = app_on_top_plane();
        let r = app
            .run_sketch(
                "sketch.rectangle",
                json!({"x1": 0, "y1": 0, "x2": 20, "y2": 10}),
            )
            .unwrap();
        app.session.sketch_selection = r["curves"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| chamfera_engine::sketching::SketchItem::Curve(c.as_u64().unwrap() as u32))
            .collect();
        app.sketch.tool = Some(Tool::Transform);
        app.sketch.shapes.dx = "30".into();
        app.sketch.shapes.dy = "0".into();
        app.sketch.shapes.copy = true;
        app.apply_transform(None);
        let a = areas(&mut app);
        assert_eq!(a.len(), 2, "{a:?}");
        assert!(a.iter().all(|v| (v - 200.0).abs() < 1e-6), "{a:?}");
        app.sketch.shapes.mode = TransformMode::Scale;
        app.sketch.shapes.factor = "zero".into();
        app.apply_transform(None);
        assert!(app.message.is_some());
    }

    #[test]
    fn every_shape_tool_has_a_name_an_icon_a_tooltip_and_hints() {
        for tool in [
            Tool::Polygon,
            Tool::Slot,
            Tool::Ellipse,
            Tool::Project,
            Tool::ThreePointRectangle,
            Tool::TwoPointCircle,
            Tool::ThreePointCircle,
            Tool::TangentCircle,
            Tool::Transform,
        ] {
            assert!(is_shape(tool));
            assert!(tooltip(tool).starts_with(name(tool)), "{tool:?}");
            assert!(!hint(tool, 0).is_empty() && !hint(tool, 2).is_empty());
            let _ = icon(tool);
        }
        assert!(!is_shape(Tool::Line));
    }
}

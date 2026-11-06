//! The shape tools of the sketch ribbon (Polygon, Slot, Ellipse) and Project, which
//! brings model edges into the sketch.
//!
//! `sketch_ui.rs` routes these tools here: their names and hints, the clicks that place
//! them, their previews and their option box. Like the rest of the UI they act only
//! through commands (`sketch.polygon`, `sketch.slot`, `sketch.ellipse`, `sketch.project`).

use crate::icons::Icon;
use crate::sketch_ui::Tool;
use crate::widgets;
use crate::{ChamferaApp, label};
use chamfera_engine::sketching::curve_polyline;
use chamfera_geom::Point2;
use chamfera_sketch::ToolOp;
use egui::{Rect, Vec2};
use serde_json::{Value, json};

/// The values the shape tools use, as set in their option box.
pub struct ShapeOptions {
    pub sides: String,
    pub circumscribed: bool,
    pub overall: bool,
    pub linked: bool,
}

impl Default for ShapeOptions {
    fn default() -> Self {
        Self {
            sides: "6".into(),
            circumscribed: false,
            overall: false,
            linked: true,
        }
    }
}

/// True for the tools this module handles.
pub(crate) fn is_shape(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Polygon | Tool::Slot | Tool::Ellipse | Tool::Project
    )
}

pub(crate) fn name(tool: Tool) -> &'static str {
    match tool {
        Tool::Polygon => label("sketch.polygon"),
        Tool::Slot => label("sketch.slot"),
        Tool::Ellipse => label("sketch.ellipse"),
        _ => label("sketch.project"),
    }
}

pub(crate) fn icon(tool: Tool) -> Icon {
    match tool {
        Tool::Polygon => Icon::Polygon,
        Tool::Slot => Icon::Slot,
        Tool::Ellipse => Icon::Ellipse,
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
        _ => "Click a model edge, or a face to project all its edges · Esc cancels",
    }
}

/// How many clicks a shape takes.
fn clicks_needed(tool: Tool) -> usize {
    match tool {
        Tool::Polygon => 2,
        Tool::Slot | Tool::Ellipse => 3,
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
            _ => None,
        }
    }

    /// Places a shape once it has all its clicks. Returns true when the clicks are used
    /// up (placed, or refused with a message) and should be cleared.
    pub(crate) fn place_shape(&mut self, tool: Tool, pts: &[Point2]) -> bool {
        if pts.len() < clicks_needed(tool) {
            return false;
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
        if all.len() == 2 && clicks_needed(tool) == 3 {
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
        if !matches!(tool, Tool::Polygon | Tool::Slot | Tool::Project) {
            return;
        }
        let look = widgets::Look {
            theme: &self.theme,
            icons: &self.icons,
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
                            _ => {
                                ui.checkbox(&mut o.linked, "Linked: follow the model");
                            }
                        }
                    });
            });
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

    #[test]
    fn every_shape_tool_has_a_name_an_icon_a_tooltip_and_hints() {
        for tool in [Tool::Polygon, Tool::Slot, Tool::Ellipse, Tool::Project] {
            assert!(is_shape(tool));
            assert!(tooltip(tool).starts_with(name(tool)), "{tool:?}");
            assert!(!hint(tool, 0).is_empty() && !hint(tool, 2).is_empty());
            let _ = icon(tool);
        }
        assert!(!is_shape(Tool::Line));
    }
}

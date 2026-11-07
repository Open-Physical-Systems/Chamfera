//! The DRAWING workspace's section, detail and annotation tools: their clicks on the
//! sheet and their options dialog. Every tool ends in one `drawing.*` command; the
//! dialog only fills in its text and values (notes, leaders, surface texture, feature
//! control frames, which way a section looks, a detail's scale).

use crate::drawing_ui::DrawTool;
use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::drawing::GdtCharacteristic;
use chamfera_geom::Point2;
use serde_json::{Value, json};

/// The fields the annotation tools use.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DrawOptions {
    /// Note and leader text.
    pub text: String,
    /// The surface texture requirement and whether material removal is required.
    pub finish: String,
    pub removal: bool,
    pub characteristic: GdtCharacteristic,
    pub tolerance: String,
    pub diameter: bool,
    /// Datum letters separated by spaces or commas: `A B C`.
    pub datums: String,
    /// The section looks to the left of the line from the first click to the second.
    pub flip: bool,
    /// The detail's scale as typed (`2`, `2:1`, `5:1`); empty for twice the view's.
    pub scale: String,
    /// Ordinate, baseline and chain dimensions measure up the view, not across it.
    pub vertical: bool,
}

impl Default for DrawOptions {
    fn default() -> Self {
        DrawOptions {
            text: String::new(),
            finish: "Ra 1.6".into(),
            removal: true,
            characteristic: GdtCharacteristic::Flatness,
            tolerance: "0.05".into(),
            diameter: false,
            datums: String::new(),
            flip: false,
            scale: String::new(),
            vertical: false,
        }
    }
}

/// The command a tool ends in.
pub(crate) fn command(t: DrawTool) -> Option<&'static str> {
    Some(match t {
        DrawTool::Section => "drawing.add_section",
        DrawTool::Detail => "drawing.add_detail",
        DrawTool::Note => "drawing.add_note",
        DrawTool::Leader => "drawing.add_leader",
        DrawTool::SurfaceFinish => "drawing.add_surface_finish",
        DrawTool::Gdt => "drawing.add_gdt",
        DrawTool::CentreLine => "drawing.add_centre_line",
        DrawTool::Balloons => "drawing.add_balloons",
        DrawTool::Ordinate => "drawing.add_ordinate",
        DrawTool::Baseline => "drawing.add_baseline",
        DrawTool::Chain => "drawing.add_chain",
        DrawTool::HoleCallout => "drawing.add_hole_callout",
        DrawTool::PlaceView(_) | DrawTool::Dimension(_) => return None,
    })
}

/// How many clicks a tool takes.
fn clicks(t: DrawTool) -> usize {
    match t {
        DrawTool::Section | DrawTool::Detail => 3,
        DrawTool::Leader | DrawTool::Gdt | DrawTool::CentreLine | DrawTool::HoleCallout => 2,
        _ => 1,
    }
}

/// The status bar hint of a tool.
pub(crate) fn hint(t: DrawTool) -> &'static str {
    match t {
        DrawTool::Section => {
            "Click two points of the cutting line on a view, then where the section goes · Esc cancels"
        }
        DrawTool::Detail => {
            "Click the detail circle's centre on a view, a point on it, then where the detail goes · Esc cancels"
        }
        DrawTool::Note => "Type the note, then click where it goes · Esc cancels",
        DrawTool::Leader => "Click the arrow's tip, then where the text goes · Esc cancels",
        DrawTool::SurfaceFinish => "Click the surface the symbol stands on · Esc cancels",
        DrawTool::Gdt => "Click the feature, then where the frame goes · Esc cancels",
        DrawTool::CentreLine => "Click two circles of a view · Esc cancels",
        DrawTool::Balloons => "Click a view to balloon its components · Esc cancels",
        DrawTool::Ordinate => {
            "Click the origin on a view, then each point to measure from it · Esc ends"
        }
        DrawTool::Baseline => {
            "Click the baseline, then each point to measure from it · Enter places · Esc cancels"
        }
        DrawTool::Chain => "Click points along the view · Enter places · Esc cancels",
        DrawTool::HoleCallout => "Click a hole's circle, then where the callout goes · Esc ends",
        _ => "",
    }
}

/// `2`, `2:1` or `1:2` as a scale.
pub(crate) fn parse_scale(s: &str) -> Option<f64> {
    let s = s.trim();
    let v = match s.split_once(':') {
        Some((a, b)) => a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?,
        None => s.parse::<f64>().ok()?,
    };
    (v.is_finite() && v > 0.0).then_some(v)
}

/// The datum letters typed in `s`.
pub(crate) fn parse_datums(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_ascii_uppercase())
        .collect()
}

impl ChamferaApp {
    /// The view under a sheet point, generously padded.
    fn drawing_view_at(&mut self, p: Point2) -> Option<u32> {
        let sheet = self.drawing_sheet()?;
        let g = chamfera_engine::cmd::drawing::render(&self.session, sheet).ok()?;
        g.view_at(p, 10.0).map(|v| v.id)
    }

    /// A click with a section, detail or annotation tool. Tests call this with sheet
    /// points instead of a mouse.
    pub(crate) fn drawing_tool_click(&mut self, tool: DrawTool, p: Point2) {
        match tool {
            DrawTool::Ordinate => return self.drawing_ordinate_click(p),
            DrawTool::Baseline | DrawTool::Chain => {
                self.drawing.picks.push(p);
                return;
            }
            _ => {}
        }
        self.drawing.picks.push(p);
        if self.drawing.picks.len() < clicks(tool) {
            return;
        }
        let picks = std::mem::take(&mut self.drawing.picks);
        let Some(id) = command(tool) else { return };
        let o = self.drawing.options.clone();
        let at = |i: usize| picks.get(i).copied().unwrap_or(p);
        let (a, b, c) = (at(0), at(1), at(2));
        let needs_view = matches!(
            tool,
            DrawTool::Section
                | DrawTool::Detail
                | DrawTool::CentreLine
                | DrawTool::Balloons
                | DrawTool::HoleCallout
        );
        let view = if needs_view {
            match self.drawing_view_at(a) {
                Some(v) => Some(v),
                None => {
                    self.message = Some((format!("{}: click on a view", label(id)), true));
                    return;
                }
            }
        } else {
            None
        };
        let mut params: Value = match tool {
            DrawTool::Section => json!({
                "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y, "x": c.x, "y": c.y, "flip": o.flip,
            }),
            DrawTool::Detail => {
                let mut v = json!({
                    "cx": a.x, "cy": a.y, "radius": a.distance(b), "x": c.x, "y": c.y,
                });
                if let Some(k) = parse_scale(&o.scale) {
                    v["scale"] = json!(k);
                }
                v
            }
            DrawTool::Note => json!({ "x": a.x, "y": a.y, "text": o.text }),
            DrawTool::Leader => json!({
                "points": [{ "x": a.x, "y": a.y }, { "x": b.x, "y": b.y }], "text": o.text,
            }),
            DrawTool::SurfaceFinish => {
                json!({ "x": a.x, "y": a.y, "text": o.finish, "removal": o.removal })
            }
            DrawTool::Gdt => {
                let datums = if o.characteristic.takes_datum() {
                    parse_datums(&o.datums)
                } else {
                    Vec::new()
                };
                json!({
                    "characteristic": o.characteristic.id(),
                    "tolerance": o.tolerance.trim().parse::<f64>().unwrap_or(f64::NAN),
                    "diameter": o.diameter,
                    "datums": datums,
                    "x": b.x, "y": b.y, "leader_x": a.x, "leader_y": a.y,
                })
            }
            DrawTool::CentreLine => json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y }),
            DrawTool::Balloons => json!({}),
            DrawTool::HoleCallout => json!({ "x1": a.x, "y1": a.y, "x": b.x, "y": b.y }),
            DrawTool::PlaceView(_)
            | DrawTool::Dimension(_)
            | DrawTool::Ordinate
            | DrawTool::Baseline
            | DrawTool::Chain => return,
        };
        if let Some(v) = view {
            params["view"] = json!(v);
        } else if let Some(s) = self.drawing_sheet() {
            params["sheet"] = json!(s);
        }
        if let Some(r) = self.run(id, params) {
            let what = match tool {
                DrawTool::Section => format!("Section {0}-{0}", r["label"].as_str().unwrap_or("")),
                DrawTool::Detail => format!("Detail {}", r["label"].as_str().unwrap_or("")),
                DrawTool::HoleCallout => r["text"]
                    .as_array()
                    .map(|l| {
                        l.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" · ")
                    })
                    .unwrap_or_default(),
                _ => label(id).to_owned(),
            };
            self.message = Some((what, false));
            // Notes and symbols repeat; the view tools are done.
            if matches!(
                tool,
                DrawTool::Section | DrawTool::Detail | DrawTool::Balloons
            ) {
                self.drawing.tool = None;
            }
        }
    }

    /// A click with the Ordinate tool: the first sets the origin (and its 0), each next
    /// one adds the distance of the point clicked from it.
    fn drawing_ordinate_click(&mut self, p: Point2) {
        let vertical = self.drawing.options.vertical;
        let Some(&origin) = self.drawing.picks.first() else {
            let Some(view) = self.drawing_view_at(p) else {
                self.message = Some((
                    format!("{}: click on a view", label("drawing.add_ordinate")),
                    true,
                ));
                return;
            };
            let axis = if vertical { "y" } else { "x" };
            let params = json!({ "view": view, "axis": axis, "x0": p.x, "y0": p.y });
            if self.run("drawing.add_ordinate", params).is_some() {
                self.drawing.picks.push(p);
                self.message = Some(("Origin 0: now click the points to measure".into(), false));
            }
            return;
        };
        let Some(view) = self.drawing_view_at(origin) else {
            self.drawing.picks.clear();
            return;
        };
        let kind = if vertical { "ordinate_y" } else { "ordinate_x" };
        let params = json!({
            "view": view, "kind": kind, "x1": origin.x, "y1": origin.y, "x2": p.x, "y2": p.y,
            "offset": -10.0,
        });
        if let Some(r) = self.run("drawing.add_dimension", params) {
            self.message = Some((
                format!("Ordinate {}", r["text"].as_str().unwrap_or("")),
                false,
            ));
        }
    }

    /// Enter or Place with the Baseline or Chain tool: one command for the points
    /// clicked so far; the tool stays on for the next set.
    pub(crate) fn drawing_finish_set(&mut self) {
        let Some(tool @ (DrawTool::Baseline | DrawTool::Chain)) = self.drawing.tool else {
            return;
        };
        let Some(id) = command(tool) else { return };
        if self.drawing.picks.len() < 2 {
            self.message = Some((format!("{}: click at least two points", label(id)), true));
            return;
        }
        let picks = std::mem::take(&mut self.drawing.picks);
        let first = picks.first().copied().unwrap_or(Point2::ORIGIN);
        let Some(view) = self.drawing_view_at(first) else {
            self.message = Some((format!("{}: click on a view", label(id)), true));
            return;
        };
        let vertical = self.drawing.options.vertical;
        // The lines go on the side of the view the points are on, as for one dimension.
        let centre = self
            .drawing_sheet()
            .and_then(|s| chamfera_engine::cmd::drawing::render(&self.session, s).ok())
            .and_then(|g| g.view(view).map(|v| v.centre));
        let n = picks.len() as f64;
        let mean = Point2::new(
            picks.iter().map(|q| q.x).sum::<f64>() / n,
            picks.iter().map(|q| q.y).sum::<f64>() / n,
        );
        let below = centre.is_some_and(|c| if vertical { mean.x < c.x } else { mean.y < c.y });
        let params = json!({
            "view": view,
            "kind": if vertical { "vertical" } else { "horizontal" },
            "points": picks.iter().map(|q| json!({ "x": q.x, "y": q.y })).collect::<Vec<_>>(),
            "offset": if below { -10.0 } else { 10.0 },
        });
        if let Some(r) = self.run(id, params) {
            let texts: Vec<&str> = r["dimensions"]
                .as_array()
                .map(|d| d.iter().filter_map(|x| x["text"].as_str()).collect())
                .unwrap_or_default();
            self.message = Some((format!("{}: {}", label(id), texts.join(", ")), false));
        }
    }

    /// Parts List: the components' table on the shown sheet, above the title block.
    pub(crate) fn drawing_parts_list(&mut self) {
        let mut params = json!({});
        if let Some(s) = self.drawing_sheet() {
            params["sheet"] = json!(s);
        }
        if let Some(r) = self.run("drawing.add_parts_list", params) {
            let n = r["rows"].as_array().map_or(0, Vec::len);
            self.message = Some((format!("Parts list: {n} items"), false));
        }
    }

    /// The options of the tool in use, beside the sheet.
    pub(crate) fn drawing_options_dialog(&mut self, ctx: &egui::Context, canvas: egui::Rect) {
        let Some(tool) = self.drawing.tool else {
            return;
        };
        let Some(id) = command(tool) else { return };
        if matches!(
            tool,
            DrawTool::CentreLine | DrawTool::Balloons | DrawTool::HoleCallout
        ) {
            return;
        }
        let picked = self.drawing.picks.len();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let title = label(id);
        let icon = match tool {
            DrawTool::Section | DrawTool::Detail => Icon::ViewFront,
            DrawTool::Gdt | DrawTool::Ordinate | DrawTool::Baseline | DrawTool::Chain => {
                Icon::Dimension
            }
            _ => Icon::Edit,
        };
        let o = &mut self.drawing.options;
        let mut close = false;
        let mut place = false;
        const WIDTH: f32 = 300.0;
        egui::Window::new(title)
            .id(egui::Id::unique("drawing tool options"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(canvas.right_top() + egui::vec2(-WIDTH - 24.0, 16.0))
            .fade_in(false)
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, icon, title) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("drawing tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| match tool {
                            DrawTool::Note | DrawTool::Leader => {
                                widgets::field_label(ui, look, "Text");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut o.text)
                                        .hint_text("BREAK SHARP EDGES"),
                                );
                                ui.end_row();
                            }
                            DrawTool::SurfaceFinish => {
                                widgets::field_label(ui, look, "Requirement");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut o.finish).hint_text("Ra 1.6"),
                                );
                                ui.end_row();
                                widgets::field_label(ui, look, "Machined");
                                ui.checkbox(&mut o.removal, "Material removal required");
                                ui.end_row();
                            }
                            DrawTool::Gdt => {
                                widgets::field_label(ui, look, "Tolerance of");
                                egui::ComboBox::from_id_salt("gdt characteristic")
                                    .width(widgets::FIELD_WIDTH)
                                    .selected_text(name(o.characteristic))
                                    .show_ui(ui, |ui| {
                                        for k in GdtCharacteristic::ALL {
                                            ui.selectable_value(&mut o.characteristic, k, name(k));
                                        }
                                    });
                                ui.end_row();
                                widgets::field_label(ui, look, "Value");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut o.tolerance).hint_text("0.05"),
                                );
                                ui.end_row();
                                widgets::field_label(ui, look, "Zone");
                                ui.checkbox(&mut o.diameter, "Cylindrical (Ø)");
                                ui.end_row();
                                if o.characteristic.takes_datum() {
                                    widgets::field_label(ui, look, "Datums");
                                    ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(&mut o.datums)
                                            .hint_text("A B C"),
                                    );
                                    ui.end_row();
                                }
                            }
                            DrawTool::Section => {
                                widgets::field_label(ui, look, "Direction");
                                ui.checkbox(&mut o.flip, "Look the other way");
                                ui.end_row();
                            }
                            DrawTool::Ordinate | DrawTool::Baseline | DrawTool::Chain => {
                                widgets::field_label(ui, look, "Measure");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut o.vertical, false, "Across (x)");
                                    ui.radio_value(&mut o.vertical, true, "Up (y)");
                                });
                                ui.end_row();
                            }
                            DrawTool::Detail => {
                                widgets::field_label(ui, look, "Scale");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut o.scale)
                                        .hint_text("twice the view's (2:1)"),
                                );
                                ui.end_row();
                            }
                            _ => {}
                        });
                    ui.label(
                        egui::RichText::new(hint(tool))
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                });
                widgets::dialog_footer(ui, look, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if matches!(tool, DrawTool::Baseline | DrawTool::Chain)
                            && widgets::primary_button(
                                ui,
                                look,
                                &format!("Place ({picked} points)"),
                                picked >= 2,
                            )
                            .clicked()
                        {
                            place = true;
                        }
                        if widgets::secondary_button(ui, look, "Close").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.drawing.tool = None;
            self.drawing.picks.clear();
        } else if place {
            self.drawing_finish_set();
        }
    }
}

/// A characteristic's name in the dialog.
fn name(k: GdtCharacteristic) -> &'static str {
    match k {
        GdtCharacteristic::Straightness => "Straightness",
        GdtCharacteristic::Flatness => "Flatness",
        GdtCharacteristic::Circularity => "Circularity",
        GdtCharacteristic::Parallelism => "Parallelism",
        GdtCharacteristic::Perpendicularity => "Perpendicularity",
        GdtCharacteristic::Position => "Position",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_and_datums_parse_as_typed() {
        assert_eq!(parse_scale("2"), Some(2.0));
        assert_eq!(parse_scale(" 5:1 "), Some(5.0));
        assert_eq!(parse_scale("1:2"), Some(0.5));
        assert_eq!(parse_scale("0"), None);
        assert_eq!(parse_scale("1:0"), None);
        assert_eq!(parse_scale("x"), None);
        assert_eq!(parse_datums("a, B  c"), vec!["A", "B", "C"]);
        assert!(parse_datums("  ").is_empty());
    }

    #[test]
    fn clicks_make_a_section_a_detail_and_annotations_through_commands() {
        let mut app = ChamferaApp::headless();
        let s = &mut app.session;
        s.execute("sketch.create", &json!({"plane": "xy"})).unwrap();
        s.execute(
            "sketch.rectangle",
            &json!({"x1": 0, "y1": 0, "x2": 50, "y2": 30}),
        )
        .unwrap();
        s.execute("sketch.circle", &json!({"cx": 25, "cy": 15, "radius": 3}))
            .unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute(
            "solid.extrude",
            &json!({"sketch": 1, "profiles": [{"x": 5, "y": 5}], "distance": 10}),
        )
        .unwrap();
        app.drawing.active = true;
        app.drawing.tool = Some(DrawTool::PlaceView(
            chamfera_doc::drawing::ViewDirection::Top,
        ));
        app.drawing_click(Point2::new(150.0, 200.0));
        // Top view: the plate's centre (25, 15) at (150, 200). A cutting line along
        // y = 15 drawn right to left, then the section placed below.
        app.drawing.tool = Some(DrawTool::Section);
        app.drawing_click(Point2::new(180.0, 200.0));
        app.drawing_click(Point2::new(120.0, 200.0));
        app.drawing_click(Point2::new(150.0, 110.0));
        let sheet = &app.session.doc.drawing.sheets[0];
        assert_eq!(sheet.views.len(), 2, "{:?}", app.message);
        assert_eq!(
            sheet.views[1].section.as_ref().map(|c| c.label.as_str()),
            Some("A")
        );
        assert!(app.drawing.tool.is_none());
        // A detail about the hole: centre, a point 8 mm out, then its place.
        app.drawing.tool = Some(DrawTool::Detail);
        app.drawing_click(Point2::new(150.0, 200.0));
        app.drawing_click(Point2::new(158.0, 200.0));
        app.drawing_click(Point2::new(300.0, 200.0));
        let d = app.session.doc.drawing.sheets[0].views[2]
            .detail
            .clone()
            .unwrap();
        assert!((d.radius - 8.0).abs() < 1e-9 && d.label == "B");
        // A note with its text from the dialog, and a flatness frame.
        app.drawing.options.text = "DEBURR".into();
        app.drawing.tool = Some(DrawTool::Note);
        app.drawing_click(Point2::new(40.0, 50.0));
        app.drawing.options.characteristic = GdtCharacteristic::Position;
        app.drawing.options.datums = "a b".into();
        app.drawing.options.diameter = true;
        app.drawing.tool = Some(DrawTool::Gdt);
        app.drawing_click(Point2::new(153.0, 200.0));
        app.drawing_click(Point2::new(200.0, 240.0));
        let notes = &app.session.doc.drawing.sheets[0].annotations;
        assert_eq!(notes.len(), 2, "{:?}", app.message);
    }

    /// The hole plate (`examples/hole-plate.json`) with its top view at (150, 200):
    /// model (x, y) is sheet (110 + x, 175 + y).
    fn hole_plate_top() -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        let steps: Value =
            serde_json::from_str(include_str!("../../../examples/hole-plate.json")).unwrap();
        chamfera_engine::script::run_steps(&mut app.session, steps.as_array().unwrap()).unwrap();
        app.drawing.active = true;
        app.run(
            "drawing.add_view",
            json!({"view": "top", "x": 150, "y": 200}),
        );
        app
    }

    fn values(app: &ChamferaApp) -> Vec<f64> {
        app.session.doc.drawing.sheets[0]
            .dimensions
            .iter()
            .map(|d| chamfera_drawing::measure(d.kind, &d.points).unwrap())
            .collect()
    }

    #[test]
    fn clicks_make_ordinate_baseline_and_chain_dimensions_and_a_hole_callout() {
        let mut app = hole_plate_top();
        // Ordinate: the corner (110, 175) is the origin (0), then the hole centres at
        // x = 10 and 40 (sheet 120 and 150).
        app.drawing.tool = Some(DrawTool::Ordinate);
        app.drawing_click(Point2::new(110.0, 175.0));
        app.drawing_click(Point2::new(120.0, 185.0));
        app.drawing_click(Point2::new(150.0, 200.0));
        assert_eq!(values(&app), vec![0.0, 10.0, 40.0], "{:?}", app.message);
        // Baseline up the view: the corner, then the centres at y = 10 and 40; nothing
        // until Enter (here: Place).
        app.drawing.tool = Some(DrawTool::Baseline);
        app.drawing.picks.clear();
        app.drawing.options.vertical = true;
        app.drawing_click(Point2::new(110.0, 175.0));
        app.drawing_click(Point2::new(120.0, 215.0));
        app.drawing_click(Point2::new(120.0, 185.0));
        assert_eq!(values(&app).len(), 3);
        app.drawing_finish_set();
        assert_eq!(values(&app)[3..], [10.0, 40.0], "{:?}", app.message);
        assert!(app.drawing.picks.is_empty() && app.drawing.tool == Some(DrawTool::Baseline));
        // Chain across: centres at x = 25, 40 and 55 (sheet 135, 150, 165): 15, 15.
        app.drawing.tool = Some(DrawTool::Chain);
        app.drawing.options.vertical = false;
        for x in [165.0, 135.0, 150.0] {
            app.drawing_click(Point2::new(x, 200.0));
        }
        app.drawing_finish_set();
        assert_eq!(values(&app)[5..], [15.0, 15.0], "{:?}", app.message);
        // Hole callout: the countersunk hole's rim (156, 200), then the text's place.
        app.drawing.tool = Some(DrawTool::HoleCallout);
        app.drawing_click(Point2::new(156.0, 200.0));
        app.drawing_click(Point2::new(200.0, 250.0));
        let a = &app.session.doc.drawing.sheets[0].annotations;
        assert_eq!(a.len(), 1, "{:?}", app.message);
        assert_eq!(
            app.message.as_ref().map(|m| m.0.as_str()),
            Some("Ø6 THRU · CSK Ø12 x 90°")
        );
    }
}

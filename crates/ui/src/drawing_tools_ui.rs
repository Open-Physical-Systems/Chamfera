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
        DrawTool::PlaceView(_) | DrawTool::Dimension(_) => return None,
    })
}

/// How many clicks a tool takes.
fn clicks(t: DrawTool) -> usize {
    match t {
        DrawTool::Section | DrawTool::Detail => 3,
        DrawTool::Leader | DrawTool::Gdt | DrawTool::CentreLine => 2,
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
            DrawTool::Section | DrawTool::Detail | DrawTool::CentreLine | DrawTool::Balloons
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
            DrawTool::PlaceView(_) | DrawTool::Dimension(_) => return,
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
        if matches!(tool, DrawTool::CentreLine | DrawTool::Balloons) {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let title = label(id);
        let icon = match tool {
            DrawTool::Section | DrawTool::Detail => Icon::ViewFront,
            DrawTool::Gdt => Icon::Dimension,
            _ => Icon::Edit,
        };
        let o = &mut self.drawing.options;
        let mut close = false;
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
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.drawing.tool = None;
            self.drawing.picks.clear();
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
}

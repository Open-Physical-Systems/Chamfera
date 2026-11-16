//! The Layers panel of the DRAWING workspace: the shown sheet's user layers, one row each,
//! with the current layer, on/off, freeze, lock, colour, line type and line weight, and
//! below them New, Rename, Delete and the line type scale. Every change is one
//! `drawing.layer.*` command (one undo step); the panel holds only what is being typed.

use crate::icons::Icon;
use crate::widgets::{self, Look, T_SMALL};
use crate::{ChamferaApp, color, label};
use chamfera_doc::drafting::{DraftLayer, LineType, WEIGHTS};
use chamfera_drawing::linetype;
use egui::{Color32, Sense, Stroke, Vec2};
use serde_json::{Value, json};

/// The colours the panel offers (our own picks; any `#rrggbb` can be typed with
/// `drawing.layer.edit`). `None` is the sheet's ink colour.
pub(crate) const PALETTE: [(&str, Option<[u8; 3]>); 9] = [
    ("Ink", None),
    ("Red", Some([220, 40, 40])),
    ("Orange", Some([235, 130, 20])),
    ("Yellow", Some([200, 170, 0])),
    ("Green", Some([40, 150, 60])),
    ("Cyan", Some([0, 150, 175])),
    ("Blue", Some([40, 90, 220])),
    ("Magenta", Some([185, 50, 165])),
    ("Grey", Some([128, 128, 128])),
];

/// The panel's own state.
#[derive(Debug, Clone, Default)]
pub(crate) struct LayerPanel {
    pub open: bool,
    /// The row picked for Rename and Delete, by layer name.
    pub picked: Option<String>,
    /// The name typed for New or Rename.
    pub name: String,
    /// The line type scale as typed.
    pub scale: String,
}

fn colour_name(c: Option<[u8; 3]>) -> String {
    PALETTE
        .iter()
        .find(|(_, p)| *p == c)
        .map(|(n, _)| (*n).to_owned())
        .unwrap_or_else(|| match c {
            Some([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
            None => "Ink".into(),
        })
}

fn colour_text(c: Option<[u8; 3]>) -> String {
    match c {
        Some([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
        None => "ink".into(),
    }
}

/// A swatch with a sample of the line type, in the layer's colour.
fn sample(ui: &mut egui::Ui, look: Look, layer: &DraftLayer, scale: f64, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 14.0), Sense::hover());
    let ink = match layer.colour {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => look.c(|t| t.sheet_line),
    };
    ui.painter()
        .rect_filled(rect, 2.0, look.c(|t| t.sheet_paper));
    // 1 mm of pattern (at line type scale 1) per 2.5 points: a 60-point sample shows 22 mm.
    let k = 2.5;
    let y = rect.center().y;
    let pts = [
        chamfera_geom::Point2::new(0.0, 0.0),
        chamfera_geom::Point2::new(f64::from((rect.width() - 6.0) / k), 0.0),
    ];
    let stroke = Stroke::new((layer.weight as f32 * k).clamp(1.5, 4.0), ink);
    for [a, b] in linetype::dash_polyline(&pts, &linetype::scaled(layer.line_type, scale)) {
        // Dots are drawn at least 1.5 points long so they show.
        let x0 = rect.min.x + 3.0 + a.x as f32 * k;
        let x1 = (rect.min.x + 3.0 + b.x as f32 * k).max(x0 + 1.5);
        ui.painter()
            .line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], stroke);
    }
}

impl ChamferaApp {
    /// Layers: opens the panel on the shown sheet, or closes it.
    pub(crate) fn toggle_layer_panel(&mut self) {
        let p = &mut self.drawing.drafting.layers;
        p.open = !p.open;
        p.scale.clear();
    }

    /// Runs one `drawing.layer.*` command on the shown sheet.
    fn layer_command(&mut self, id: &str, mut params: Value) -> Option<Value> {
        if let (Some(s), Value::Object(map)) = (self.drawing_sheet(), &mut params) {
            map.insert("sheet".into(), json!(s));
        }
        self.run(id, params)
    }

    /// The panel, when it is open.
    pub(crate) fn layer_panel(&mut self, ctx: &egui::Context, canvas: egui::Rect) {
        if !self.drawing.drafting.layers.open {
            return;
        }
        let Some(sheet) = self
            .drawing_sheet()
            .and_then(|s| self.session.doc.drawing.sheet(s).ok())
        else {
            return;
        };
        let d = sheet.drafting.clone();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let mut panel = self.drawing.drafting.layers.clone();
        if panel.scale.is_empty() {
            panel.scale = format!("{}", d.linetype_scale);
        }
        // Commands to run once the panel is drawn: (id, params).
        let mut todo: Vec<(&'static str, Value)> = Vec::new();
        let mut close = false;
        let title = label("drawing.layer.list");
        egui::Window::new(title)
            .id(egui::Id::unique("layer panel"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .fixed_pos(egui::pos2(canvas.max.x - 12.0, canvas.min.y + 12.0))
            .pivot(egui::Align2::RIGHT_TOP)
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                if widgets::dialog_title(ui, look, Icon::Eye, title) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("layer rows")
                        .num_columns(9)
                        .spacing([widgets::S3, 6.0])
                        .show(ui, |ui| {
                            for h in ["", "Name", "On", "Freeze", "Lock", "Colour", "Line type", "Weight", "Items"] {
                                ui.label(
                                    egui::RichText::new(h)
                                        .size(T_SMALL)
                                        .color(look.c(|t| t.text_muted)),
                                );
                            }
                            ui.end_row();
                            for l in &d.layers {
                                let name = l.name.clone();
                                let current = name.eq_ignore_ascii_case(&d.current);
                                let r = ui
                                    .radio(current, "")
                                    .on_hover_text("Current: new geometry goes on this layer");
                                if r.clicked() && !current {
                                    todo.push(("drawing.layer.current", json!({ "layer": name })));
                                }
                                let picked = panel.picked.as_deref() == Some(name.as_str());
                                let text = egui::RichText::new(&name).color(if current {
                                    look.c(|t| t.text)
                                } else {
                                    look.c(|t| t.text_muted)
                                });
                                if ui
                                    .add_sized([110.0, 20.0], egui::Button::selectable(picked, text))
                                    .on_hover_text("Pick for Rename or Delete")
                                    .clicked()
                                {
                                    panel.picked = Some(name.clone());
                                    panel.name = name.clone();
                                }
                                let eye = if l.on { Icon::Eye } else { Icon::EyeOff };
                                if widgets::icon_button(ui, look, eye, 22.0, 16.0, true, false)
                                    .on_hover_text(if l.on { "On: click to hide" } else { "Off: click to show" })
                                    .clicked()
                                {
                                    todo.push(("drawing.layer.edit", json!({ "layer": name, "on": !l.on })));
                                }
                                let mut frozen = l.frozen;
                                if ui
                                    .checkbox(&mut frozen, "")
                                    .on_hover_text("Frozen: hidden, and it cannot be current")
                                    .changed()
                                {
                                    todo.push(("drawing.layer.edit", json!({ "layer": name, "frozen": frozen })));
                                }
                                let mut locked = l.locked;
                                if ui
                                    .checkbox(&mut locked, "")
                                    .on_hover_text("Locked: shown, but cannot be edited")
                                    .changed()
                                {
                                    todo.push(("drawing.layer.edit", json!({ "layer": name, "locked": locked })));
                                }
                                ui.horizontal(|ui| {
                                    let (sw, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                                    let fill = match l.colour {
                                        Some([r, g, b]) => Color32::from_rgb(r, g, b),
                                        None => color(self.theme.sheet_line),
                                    };
                                    ui.painter().rect_filled(sw, 2.0, fill);
                                    ui.painter().rect_stroke(sw, 2.0, Stroke::new(1.0, look.c(|t| t.border)), egui::StrokeKind::Inside);
                                    egui::ComboBox::from_id_salt(("layer colour", &name))
                                        .width(78.0)
                                        .selected_text(colour_name(l.colour))
                                        .show_ui(ui, |ui| {
                                            for (n, c) in PALETTE {
                                                if ui.selectable_label(l.colour == c, n).clicked() && l.colour != c {
                                                    todo.push(("drawing.layer.edit", json!({ "layer": name, "colour": colour_text(c) })));
                                                }
                                            }
                                        });
                                });
                                ui.horizontal(|ui| {
                                    sample(ui, look, l, 1.0, 60.0);
                                    egui::ComboBox::from_id_salt(("layer type", &name))
                                        .width(86.0)
                                        .selected_text(l.line_type.id())
                                        .show_ui(ui, |ui| {
                                            for t in LineType::ALL {
                                                if ui
                                                    .selectable_label(l.line_type == t, t.id())
                                                    .on_hover_text(linetype::describe(t))
                                                    .clicked()
                                                    && l.line_type != t
                                                {
                                                    todo.push(("drawing.layer.edit", json!({ "layer": name, "line_type": t.id() })));
                                                }
                                            }
                                        });
                                });
                                egui::ComboBox::from_id_salt(("layer weight", &name))
                                    .width(64.0)
                                    .selected_text(format!("{} mm", l.weight))
                                    .show_ui(ui, |ui| {
                                        for w in WEIGHTS {
                                            if ui
                                                .selectable_label((l.weight - w).abs() < 1e-9, format!("{w} mm"))
                                                .clicked()
                                            {
                                                todo.push(("drawing.layer.edit", json!({ "layer": name, "weight": w })));
                                            }
                                        }
                                    });
                                let count = d.entities.iter().filter(|e| e.layer.eq_ignore_ascii_case(&name)).count();
                                ui.label(egui::RichText::new(count.to_string()).color(look.c(|t| t.text_muted)));
                                ui.end_row();
                            }
                        });
                    ui.add_space(widgets::S3);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [140.0, 24.0],
                            egui::TextEdit::singleline(&mut panel.name).hint_text("Layer name"),
                        );
                        if ui.button("New").on_hover_text("A new layer with this name, made current").clicked() {
                            todo.push(("drawing.layer.add", json!({ "name": panel.name.trim(), "current": true })));
                        }
                        let picked = panel.picked.clone();
                        if ui
                            .add_enabled(picked.is_some(), egui::Button::new("Rename"))
                            .on_hover_text("Give the picked layer this name")
                            .clicked()
                            && let Some(p) = &picked
                        {
                            todo.push(("drawing.layer.edit", json!({ "layer": p, "name": panel.name.trim() })));
                        }
                        if ui
                            .add_enabled(picked.is_some(), egui::Button::new("Delete"))
                            .on_hover_text("Delete the picked layer (it must be empty and not current)")
                            .clicked()
                            && let Some(p) = &picked
                        {
                            todo.push(("drawing.layer.delete", json!({ "layer": p })));
                        }
                    });
                    ui.horizontal(|ui| {
                        widgets::field_label(ui, look, "Line type scale");
                        let r = ui.add_sized([70.0, 24.0], egui::TextEdit::singleline(&mut panel.scale));
                        if r.lost_focus()
                            && let Some(k) = crate::command_line::parse_number(&panel.scale)
                            && (k - d.linetype_scale).abs() > 1e-12
                        {
                            todo.push(("drawing.linetype_scale", json!({ "scale": k })));
                        }
                    });
                });
            });
        if close {
            panel.open = false;
        }
        self.drawing.drafting.layers = panel;
        for (id, params) in todo {
            let renamed = id == "drawing.layer.edit" && params.get("name").is_some();
            let deleted = id == "drawing.layer.delete" || id == "drawing.layer.add";
            if self.layer_command(id, params).is_some() && (renamed || deleted) {
                let p = &mut self.drawing.drafting.layers;
                p.picked = None;
                p.name.clear();
            }
            if id == "drawing.linetype_scale" {
                self.drawing.drafting.layers.scale.clear();
            }
        }
    }
}

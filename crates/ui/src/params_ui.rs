//! The Parameters window: named values and expressions that dimensions refer to.
//! Edits go through `param.set` and `param.remove`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ParamsUi {
    pub open: bool,
    /// Text being edited per parameter, before it is applied.
    edits: BTreeMap<String, String>,
    new_name: String,
    new_expression: String,
    new_unit: usize,
}

const UNITS: [(&str, &str); 3] = [
    ("length", "Length"),
    ("angle", "Angle"),
    ("unitless", "Unitless"),
];

fn unit_index(u: chamfera_doc::Unit) -> usize {
    match u {
        chamfera_doc::Unit::Length => 0,
        chamfera_doc::Unit::Angle => 1,
        chamfera_doc::Unit::Unitless => 2,
    }
}

/// A parameter's value as a user reads it.
pub(crate) fn show_value(u: chamfera_doc::DisplayUnits, p: &chamfera_doc::Parameter) -> String {
    let v = match p.unit {
        chamfera_doc::Unit::Angle => p.value.to_degrees(),
        chamfera_doc::Unit::Length => p.value / u.length.mm(),
        chamfera_doc::Unit::Unitless => p.value,
    };
    let text = format!("{v:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A plain value as an expression with its unit, for the edit box.
fn value_expression(u: chamfera_doc::DisplayUnits, p: &chamfera_doc::Parameter) -> String {
    match p.unit {
        chamfera_doc::Unit::Length => format!("{} {}", show_value(u, p), u.length.symbol()),
        chamfera_doc::Unit::Angle => format!("{} deg", show_value(u, p)),
        chamfera_doc::Unit::Unitless => show_value(u, p),
    }
}

impl ChamferaApp {
    pub(crate) fn parameters_window(&mut self, ctx: &egui::Context) {
        if !self.params.open {
            return;
        }
        let mut open = true;
        let units = self.session.doc.units;
        let vr = self.viewport_rect;
        let pos = egui::pos2(vr.min.x + widgets::S5, vr.min.y + widgets::S5);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let rows: Vec<(String, chamfera_doc::Parameter)> = self
            .session
            .doc
            .parameters
            .iter()
            .map(|(n, p)| (n.clone(), p.clone()))
            .collect();
        let error = match &self.message {
            Some((m, true)) => Some(m.clone()),
            _ => None,
        };
        // Commands to run once the window is drawn: (name, expression, unit) to set, or a
        // name to remove.
        let mut set: Vec<(String, String, &str, bool)> = Vec::new();
        let mut remove: Option<String> = None;
        let params = &mut self.params;
        egui::Window::new("Parameters")
            .id(egui::Id::unique("parameters window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(200.0)
            .fade_in(false)
            .default_width(560.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(560.0);
                if widgets::dialog_title(ui, look, Icon::Parameters, "Parameters") {
                    open = false;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Named values that dimensions and features can use. Expressions take units (mm, cm, in, deg), + − × ÷ ^, other parameters and functions such as sqrt or min.",
                        )
                        .size(widgets::T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                    );
                    ui.add_space(widgets::S4);
                    egui::Grid::new("parameter grid")
                        .num_columns(5)
                        .spacing([widgets::S3, widgets::S2 + 2.0])
                        .min_col_width(0.0)
                        .show(ui, |ui| {
                            for (h, align) in [
                                ("NAME", egui::Align::Min),
                                ("EXPRESSION", egui::Align::Min),
                                ("VALUE", egui::Align::Max),
                                ("UNIT", egui::Align::Min),
                            ] {
                                let text = egui::RichText::new(h)
                                    .size(widgets::T_CAPTION)
                                    .color(look.c(|t| t.text_muted));
                                let width = match h {
                                    "NAME" => 110.0,
                                    "EXPRESSION" => 230.0,
                                    "VALUE" => 72.0,
                                    _ => 40.0,
                                };
                                widgets::fixed_text(ui, width, text, align);
                            }
                            ui.label("");
                            ui.end_row();
                            if rows.is_empty() {
                                ui.label(
                                    egui::RichText::new("No parameters yet: add one below")
                                        .color(look.c(|t| t.text_muted)),
                                );
                                ui.end_row();
                            }
                            for (name, p) in &rows {
                                widgets::fixed_text(
                                    ui,
                                    110.0,
                                    egui::RichText::new(name).color(look.c(|t| t.text)),
                                    egui::Align::Min,
                                );
                                let current =
                                    p.expression.clone().unwrap_or_else(|| value_expression(units, p));
                                let text = params.edits.entry(name.clone()).or_insert(current);
                                let edit = ui.add_sized(
                                    [230.0, 26.0],
                                    egui::TextEdit::singleline(text).align(egui::Align2::LEFT_CENTER),
                                );
                                // Enter or clicking elsewhere applies the text.
                                if edit.lost_focus() {
                                    let unit = UNITS.get(unit_index(p.unit)).map_or("length", |u| u.0);
                                    set.push((name.clone(), text.clone(), unit, true));
                                    params.edits.remove(name);
                                }
                                widgets::fixed_text(
                                    ui,
                                    72.0,
                                    egui::RichText::new(show_value(units, p)).color(look.c(|t| t.text)),
                                    egui::Align::Max,
                                );
                                ui.label(
                                    egui::RichText::new(match p.unit {
                                        chamfera_doc::Unit::Length => units.length.symbol(),
                                        chamfera_doc::Unit::Angle => "deg",
                                        chamfera_doc::Unit::Unitless => "—",
                                    })
                                    .color(look.c(|t| t.text_muted)),
                                );
                                if widgets::icon_button(ui, look, Icon::Delete, 26.0, 16.0, true, false)
                                    .on_hover_text(format!("{name}: {}", label("param.remove")))
                                    .clicked()
                                {
                                    remove = Some(name.clone());
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(widgets::S3);
                    ui.separator();
                    ui.add_space(widgets::S3);
                    // A row to add a parameter.
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [110.0, 26.0],
                            egui::TextEdit::singleline(&mut params.new_name)
                                .hint_text("name")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        ui.add_sized(
                            [230.0, 26.0],
                            egui::TextEdit::singleline(&mut params.new_expression)
                                .hint_text("e.g. 80 mm or width / 2")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        egui::ComboBox::from_id_salt("new parameter unit")
                            .selected_text(UNITS.get(params.new_unit).map_or("", |u| u.1))
                            .width(72.0)
                            .show_ui(ui, |ui| {
                                for (i, (_, text)) in UNITS.iter().enumerate() {
                                    ui.selectable_value(&mut params.new_unit, i, *text);
                                }
                            });
                        let can_add = !params.new_name.trim().is_empty()
                            && !params.new_expression.trim().is_empty();
                        if widgets::primary_button(ui, look, "Add", can_add).clicked() {
                            let unit = UNITS.get(params.new_unit).map_or("length", |u| u.0);
                            set.push((
                                params.new_name.trim().to_owned(),
                                params.new_expression.trim().to_owned(),
                                unit,
                                false,
                            ));
                        }
                    });
                    if let Some(m) = &error {
                        ui.add_space(widgets::S3);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                            let e = look.c(|t| t.error);
                            look.icons.paint(ui.painter(), Icon::Error, r, e, e);
                            ui.label(egui::RichText::new(m).color(e));
                        });
                    }
                });
            });
        for (name, expression, unit, existing) in set {
            if self
                .run(
                    "param.set",
                    json!({ "name": name, "expression": expression, "unit": unit }),
                )
                .is_some()
            {
                self.refresh_sketch_status();
                self.message = Some((format!("{name} = {expression}"), false));
                if !existing {
                    self.params.new_name.clear();
                    self.params.new_expression.clear();
                }
            }
        }
        if let Some(name) = remove
            && self.run("param.remove", json!({ "name": name })).is_some()
        {
            self.params.edits.remove(&name);
        }
        self.params.open = open && self.params.open;
    }
}

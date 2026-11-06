//! The Parameters window: named values and expressions that dimensions refer to.
//! Edits go through `param.set` and `param.remove`.

use crate::{ChamferaApp, color, label};
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

const UNITS: [(&str, &str); 3] = [("length", "mm"), ("angle", "deg"), ("unitless", "—")];

fn unit_index(u: chamfera_doc::Unit) -> usize {
    match u {
        chamfera_doc::Unit::Length => 0,
        chamfera_doc::Unit::Angle => 1,
        chamfera_doc::Unit::Unitless => 2,
    }
}

/// A parameter's value as a user reads it.
pub(crate) fn show_value(p: &chamfera_doc::Parameter) -> String {
    let v = match p.unit {
        chamfera_doc::Unit::Angle => p.value.to_degrees(),
        _ => p.value,
    };
    let text = format!("{v:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A plain value as an expression with its unit, for the edit box.
fn value_expression(p: &chamfera_doc::Parameter) -> String {
    match p.unit {
        chamfera_doc::Unit::Length => format!("{} mm", show_value(p)),
        chamfera_doc::Unit::Angle => format!("{} deg", show_value(p)),
        chamfera_doc::Unit::Unitless => show_value(p),
    }
}

impl ChamferaApp {
    pub(crate) fn parameters_window(&mut self, ctx: &egui::Context) {
        if !self.params.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Parameters")
            .open(&mut open)
            .default_pos([240.0, 110.0])
            .default_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.weak("Expressions may use units (mm, cm, in, deg), + − × ÷ ^, other parameters and functions such as sqrt or min.");
                ui.separator();
                let rows: Vec<(String, chamfera_doc::Parameter)> = self
                    .session
                    .doc
                    .parameters
                    .iter()
                    .map(|(n, p)| (n.clone(), p.clone()))
                    .collect();
                egui::Grid::new("parameter grid")
                    .striped(true)
                    .num_columns(5)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("Expression");
                        ui.strong("Value");
                        ui.strong("Unit");
                        ui.label("");
                        ui.end_row();
                        for (name, p) in rows {
                            ui.label(&name);
                            let current = p.expression.clone().unwrap_or_else(|| value_expression(&p));
                            let text = self.params.edits.entry(name.clone()).or_insert(current);
                            let edit = ui.add_sized([200.0, 20.0], egui::TextEdit::singleline(text));
                            // Enter or clicking elsewhere applies the text.
                            if edit.lost_focus() {
                                let expression = text.clone();
                                let unit = UNITS.get(unit_index(p.unit)).map_or("length", |u| u.0);
                                if self
                                    .run(
                                        "param.set",
                                        json!({ "name": name, "expression": expression, "unit": unit }),
                                    )
                                    .is_some()
                                {
                                    self.refresh_sketch_status();
                                    self.message = Some((format!("{name} = {expression}"), false));
                                }
                                self.params.edits.remove(&name);
                            }
                            ui.label(show_value(&p));
                            ui.label(UNITS.get(unit_index(p.unit)).map_or("", |u| u.1));
                            if ui
                                .small_button("Delete")
                                .on_hover_text(label("param.remove"))
                                .clicked()
                                && self.run("param.remove", json!({ "name": name })).is_some()
                            {
                                self.params.edits.remove(&name);
                            }
                            ui.end_row();
                        }
                        // A row to add a parameter.
                        ui.add_sized(
                            [90.0, 20.0],
                            egui::TextEdit::singleline(&mut self.params.new_name).hint_text("name"),
                        );
                        ui.add_sized(
                            [200.0, 20.0],
                            egui::TextEdit::singleline(&mut self.params.new_expression)
                                .hint_text("e.g. 80 mm"),
                        );
                        ui.label("");
                        egui::ComboBox::from_id_salt("new parameter unit")
                            .selected_text(UNITS.get(self.params.new_unit).map_or("", |u| u.1))
                            .show_ui(ui, |ui| {
                                for (i, (_, text)) in UNITS.iter().enumerate() {
                                    ui.selectable_value(&mut self.params.new_unit, i, *text);
                                }
                            });
                        let can_add = !self.params.new_name.trim().is_empty()
                            && !self.params.new_expression.trim().is_empty();
                        if ui.add_enabled(can_add, egui::Button::new("Add")).clicked() {
                            let unit = UNITS.get(self.params.new_unit).map_or("length", |u| u.0);
                            let name = self.params.new_name.trim().to_owned();
                            if self
                                .run(
                                    "param.set",
                                    json!({
                                        "name": name,
                                        "expression": self.params.new_expression.trim(),
                                        "unit": unit,
                                    }),
                                )
                                .is_some()
                            {
                                self.params.new_name.clear();
                                self.params.new_expression.clear();
                            }
                        }
                        ui.end_row();
                    });
                if let Some((m, true)) = &self.message {
                    ui.separator();
                    ui.colored_label(color(self.theme.error), m);
                }
            });
        self.params.open = open;
    }
}

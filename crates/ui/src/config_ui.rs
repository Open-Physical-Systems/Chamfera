//! Configurations: the design table window (configurations × overridden parameters and
//! suppressed features) and the active-configuration menu in the app bar. Everything
//! runs `config.*` commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ConfigUi {
    pub open: bool,
    new_name: String,
    /// Cell text being edited: (configuration, parameter) → expression.
    cells: BTreeMap<(String, String), String>,
    /// The export format: 0 STEP, 1 STL (desktop only).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    format: usize,
}

/// Something to run once the window is drawn.
enum Act {
    Activate(Option<String>),
    Add(String, Option<String>),
    Delete(String),
    Set(String, String, Option<String>),
    Suppress(String, u32, bool),
    AddColumn(String),
    AddFeature(u32),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ExportAll(&'static str),
}

const W_NAME: f32 = 120.0;
const W_CELL: f32 = 96.0;

impl ChamferaApp {
    /// The active-configuration menu, in the app bar, once the design has configurations.
    pub(crate) fn configuration_menu(&mut self, ui: &mut egui::Ui) {
        let c = &self.session.doc.configurations;
        if c.list.is_empty() {
            return;
        }
        let names: Vec<String> = c.list.iter().map(|k| k.name.clone()).collect();
        let active = c.active.clone();
        let mut pick: Option<Option<String>> = None;
        let mut edit = false;
        widgets::bar_divider(ui, self.look(), 20.0);
        let shown = active.clone().unwrap_or_else(|| "Base design".to_owned());
        egui::ComboBox::from_id_salt("active configuration")
            .selected_text(shown)
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(active.is_none(), "Base design")
                    .clicked()
                {
                    pick = Some(None);
                }
                for n in &names {
                    if ui
                        .selectable_label(active.as_deref() == Some(n.as_str()), n)
                        .clicked()
                    {
                        pick = Some(Some(n.clone()));
                    }
                }
                ui.separator();
                if ui.selectable_label(false, "Edit Configurations…").clicked() {
                    edit = true;
                }
            })
            .response
            .on_hover_text("Configuration\nWhich variant of the design to build");
        if edit {
            self.configs.open = true;
        }
        if let Some(p) = pick
            && p != active
        {
            self.run_config(Act::Activate(p));
        }
    }

    pub(crate) fn configurations_window(&mut self, ctx: &egui::Context) {
        if !self.configs.open {
            return;
        }
        let mut open = true;
        let vr = self.viewport_rect;
        let pos = egui::pos2(vr.max.x - 720.0, vr.min.y + widgets::S5);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let doc = &self.session.doc;
        let table = &doc.configurations;
        let columns = table.columns();
        let features = table.feature_columns();
        let feature_name = |id: u32| doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
        let base = |p: &str| {
            doc.parameters
                .get(p)
                .map(|x| {
                    x.expression
                        .clone()
                        .unwrap_or_else(|| format!("{}", x.value))
                })
                .unwrap_or_default()
        };
        let free_params: Vec<String> = doc
            .parameters
            .keys()
            .filter(|p| !columns.contains(p))
            .cloned()
            .collect();
        let free_features: Vec<(u32, String)> = doc
            .features
            .iter()
            .filter(|f| !features.contains(&f.id))
            .map(|f| (f.id, f.name.clone()))
            .collect();
        let rows: Vec<chamfera_doc::Configuration> = table.list.clone();
        let active = table.active.clone();
        let error = match &self.message {
            Some((m, true)) => Some(m.clone()),
            _ => None,
        };
        let mut acts: Vec<Act> = Vec::new();
        let ui_state = &mut self.configs;
        egui::Window::new("Configurations")
            .id(egui::Id::unique("configurations window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                if widgets::dialog_title(ui, look, Icon::Parameters, "Configurations") {
                    open = false;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Each row is a variant of the design: it overrides the parameters in \
                             the columns (empty: the base value) and can suppress features. \
                             The active row is built.",
                        )
                        .size(widgets::T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                    );
                    ui.add_space(widgets::S4);
                    egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                        egui::Grid::new("configuration grid")
                            .spacing([widgets::S3, widgets::S2])
                            .min_col_width(0.0)
                            .show(ui, |ui| {
                                let head = |ui: &mut egui::Ui, t: &str, w: f32| {
                                    widgets::fixed_text(
                                        ui,
                                        w,
                                        egui::RichText::new(t)
                                            .size(widgets::T_CAPTION)
                                            .color(look.c(|t| t.text_muted)),
                                        egui::Align::Min,
                                    );
                                };
                                ui.label("");
                                head(ui, "CONFIGURATION", W_NAME);
                                for c in &columns {
                                    head(ui, c, W_CELL);
                                }
                                for f in &features {
                                    head(ui, &format!("{} ON", feature_name(*f)), W_CELL);
                                }
                                ui.label("");
                                ui.end_row();
                                // The base design, for reference.
                                if ui
                                    .radio(active.is_none(), "")
                                    .on_hover_text("Build the base design")
                                    .clicked()
                                {
                                    acts.push(Act::Activate(None));
                                }
                                widgets::fixed_text(
                                    ui,
                                    W_NAME,
                                    egui::RichText::new("Base design")
                                        .italics()
                                        .color(look.c(|t| t.text_muted)),
                                    egui::Align::Min,
                                );
                                for c in &columns {
                                    widgets::fixed_text(
                                        ui,
                                        W_CELL,
                                        egui::RichText::new(base(c))
                                            .color(look.c(|t| t.text_muted)),
                                        egui::Align::Min,
                                    );
                                }
                                for f in &features {
                                    let on = doc.feature(*f).is_ok_and(|x| !x.suppressed);
                                    widgets::fixed_text(
                                        ui,
                                        W_CELL,
                                        egui::RichText::new(if on { "yes" } else { "no" })
                                            .color(look.c(|t| t.text_muted)),
                                        egui::Align::Min,
                                    );
                                }
                                ui.label("");
                                ui.end_row();
                                for cfg in &rows {
                                    let (name, overrides, suppressed) =
                                        (&cfg.name, &cfg.parameters, &cfg.suppressed);
                                    let is_active = active.as_deref() == Some(name.as_str());
                                    if ui
                                        .radio(is_active, "")
                                        .on_hover_text(label("config.activate"))
                                        .clicked()
                                        && !is_active
                                    {
                                        acts.push(Act::Activate(Some(name.clone())));
                                    }
                                    let text = egui::RichText::new(name).color(if is_active {
                                        look.c(|t| t.accent)
                                    } else {
                                        look.c(|t| t.text)
                                    });
                                    widgets::fixed_text(
                                        ui,
                                        W_NAME,
                                        if is_active { text.strong() } else { text },
                                        egui::Align::Min,
                                    );
                                    for c in &columns {
                                        let current = overrides.get(c).cloned().unwrap_or_default();
                                        let cell = ui_state
                                            .cells
                                            .entry((name.clone(), c.clone()))
                                            .or_insert_with(|| current.clone());
                                        let resp = ui.add_sized(
                                            [W_CELL, 26.0],
                                            egui::TextEdit::singleline(cell)
                                                .hint_text(base(c))
                                                .align(egui::Align2::LEFT_CENTER),
                                        );
                                        if resp.lost_focus() && *cell != current {
                                            let e = cell.trim();
                                            acts.push(Act::Set(
                                                name.clone(),
                                                c.clone(),
                                                (!e.is_empty()).then(|| e.to_owned()),
                                            ));
                                        } else if !resp.has_focus() && *cell != current {
                                            *cell = current;
                                        }
                                    }
                                    for f in &features {
                                        let base_on = doc.feature(*f).is_ok_and(|x| !x.suppressed);
                                        let mut on = suppressed.get(f).map_or(base_on, |s| !s);
                                        let resp = ui
                                            .add_sized(
                                                [W_CELL, 26.0],
                                                egui::Checkbox::without_text(&mut on),
                                            )
                                            .on_hover_text(format!(
                                                "{} built in {name}",
                                                feature_name(*f)
                                            ));
                                        if resp.changed() {
                                            acts.push(Act::Suppress(name.clone(), *f, !on));
                                        }
                                    }
                                    if widgets::icon_button(
                                        ui,
                                        look,
                                        Icon::Delete,
                                        26.0,
                                        16.0,
                                        true,
                                        false,
                                    )
                                    .on_hover_text(format!("{name}: {}", label("config.delete")))
                                    .clicked()
                                    {
                                        acts.push(Act::Delete(name.clone()));
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                    ui.add_space(widgets::S3);
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("add configuration column")
                            .selected_text("Add parameter…")
                            .width(130.0)
                            .show_ui(ui, |ui| {
                                if free_params.is_empty() {
                                    ui.label("Every parameter is a column");
                                }
                                for p in &free_params {
                                    if ui.selectable_label(false, p).clicked() {
                                        acts.push(Act::AddColumn(p.clone()));
                                    }
                                }
                            });
                        egui::ComboBox::from_id_salt("add configuration feature")
                            .selected_text("Add feature…")
                            .width(130.0)
                            .show_ui(ui, |ui| {
                                for (id, n) in &free_features {
                                    if ui.selectable_label(false, n).clicked() {
                                        acts.push(Act::AddFeature(*id));
                                    }
                                }
                            });
                    });
                    ui.add_space(widgets::S3);
                    ui.separator();
                    ui.add_space(widgets::S3);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [W_NAME + 30.0, 26.0],
                            egui::TextEdit::singleline(&mut ui_state.new_name)
                                .hint_text("new configuration")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        let can = !ui_state.new_name.trim().is_empty();
                        if widgets::primary_button(ui, look, "Add", can).clicked() {
                            acts.push(Act::Add(ui_state.new_name.trim().to_owned(), None));
                        }
                        if let Some(a) = &active
                            && widgets::secondary_button(ui, look, &format!("Copy {a}")).clicked()
                            && can
                        {
                            acts.push(Act::Add(
                                ui_state.new_name.trim().to_owned(),
                                Some(a.clone()),
                            ));
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let format = ["step", "stl"];
                            if widgets::secondary_button(ui, look, "Export All…").clicked() {
                                acts.push(Act::ExportAll(
                                    format.get(ui_state.format).copied().unwrap_or("step"),
                                ));
                            }
                            egui::ComboBox::from_id_salt("export all format")
                                .selected_text(if ui_state.format == 1 { "STL" } else { "STEP" })
                                .width(60.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut ui_state.format, 0, "STEP");
                                    ui.selectable_value(&mut ui_state.format, 1, "STL");
                                });
                        });
                    });
                    if let Some(m) = &error {
                        ui.add_space(widgets::S3);
                        ui.horizontal(|ui| {
                            let (r, _) = ui
                                .allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                            let e = look.c(|t| t.error);
                            look.icons.paint(ui.painter(), Icon::Error, r, e, e);
                            ui.label(egui::RichText::new(m).color(e));
                        });
                    }
                });
            });
        self.configs.open = open && self.configs.open;
        for a in acts {
            self.run_config(a);
        }
    }

    fn run_config(&mut self, a: Act) {
        let doc = &self.session.doc;
        let r: Option<Value> = match a {
            Act::Activate(c) => {
                let r = self.run("config.activate", json!({ "config": c }));
                if r.is_some() {
                    let what = c.unwrap_or_else(|| "the base design".to_owned());
                    self.message = Some((format!("Built {what}"), false));
                }
                r
            }
            Act::Add(name, from) => {
                let r = self.run("config.add", json!({ "name": name, "from": from }));
                if r.is_some() {
                    self.configs.new_name.clear();
                }
                r
            }
            Act::Delete(name) => self.run("config.delete", json!({ "config": name })),
            Act::Set(config, parameter, expression) => {
                let r = self.run(
                    "config.set",
                    json!({ "config": config, "parameter": parameter, "expression": expression }),
                );
                self.configs.cells.remove(&(config, parameter));
                r
            }
            Act::Suppress(config, feature, suppressed) => self.run(
                "config.suppress",
                json!({ "config": config, "feature": feature, "suppressed": suppressed }),
            ),
            Act::AddColumn(p) => {
                // The column starts at the base value in every configuration.
                let e = doc.parameters.get(&p).map(|x| {
                    x.expression
                        .clone()
                        .unwrap_or_else(|| format!("{}", x.value))
                });
                let names: Vec<String> = doc
                    .configurations
                    .list
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                let mut last = None;
                for n in names {
                    last = self.run(
                        "config.set",
                        json!({ "config": n, "parameter": p, "expression": e }),
                    );
                }
                last
            }
            Act::AddFeature(id) => {
                let on = doc.feature(id).is_ok_and(|f| f.suppressed);
                let names: Vec<String> = doc
                    .configurations
                    .list
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                let mut last = None;
                for n in names {
                    last = self.run(
                        "config.suppress",
                        json!({ "config": n, "feature": id, "suppressed": on }),
                    );
                }
                last
            }
            Act::ExportAll(format) => {
                self.export_configurations(format);
                None
            }
        };
        if r.is_some() {
            self.refresh_sketch_status();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_configurations(&mut self, format: &str) {
        let Some(dir) = rfd::FileDialog::new()
            .set_title("Export Every Configuration")
            .pick_folder()
        else {
            return;
        };
        let text = dir.to_string_lossy().into_owned();
        if let Some(r) = self.run(
            "config.export_all",
            json!({ "dir": text, "format": format }),
        ) {
            let n = r["files"].as_array().map_or(0, Vec::len);
            self.message = Some((format!("Wrote {n} files to {text}"), false));
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn export_configurations(&mut self, _format: &str) {}
}

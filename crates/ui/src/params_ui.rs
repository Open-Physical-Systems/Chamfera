//! The Parameters window (v2): user parameters and every model parameter (each feature's
//! and dimension's value, `Extrude1.distance`, `Sketch1.d13`) in one table.
//!
//! Every name, expression and comment is edited in place; a star pins a parameter at the
//! top. Search and a filter narrow the table. A refused edit (a loop, an unknown name, a
//! unit mismatch) is shown under its row. Edits go through `param.set`, `param.rename`,
//! `param.annotate` and `param.remove`; CSV through `param.import_csv` and
//! `param.export_csv`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Which rows the table shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    User,
    Model,
    Favourites,
    Errors,
}

impl Filter {
    const ALL: [Filter; 5] = [
        Filter::All,
        Filter::User,
        Filter::Model,
        Filter::Favourites,
        Filter::Errors,
    ];

    fn name(self) -> &'static str {
        match self {
            Filter::All => "All parameters",
            Filter::User => "User parameters",
            Filter::Model => "Model parameters",
            Filter::Favourites => "Favourites",
            Filter::Errors => "With errors",
        }
    }
}

#[derive(Default)]
pub struct ParamsUi {
    pub open: bool,
    /// Expression text being edited, by parameter name, before it is applied.
    edits: BTreeMap<String, String>,
    /// Name text being edited (a rename), by the parameter's current name.
    names: BTreeMap<String, String>,
    /// Comment text being edited.
    comments: BTreeMap<String, String>,
    new_name: String,
    new_expression: String,
    new_unit: usize,
    pub search: String,
    pub filter: Filter,
    /// The last refused edit: the row's name and why.
    row_error: Option<(String, String)>,
}

const UNITS: [(&str, &str); 3] = [
    ("length", "Length"),
    ("angle", "Angle"),
    ("unitless", "Unitless"),
];

/// A value (mm, radians or plain) in the display unit, trailing zeros trimmed.
fn show_number(u: chamfera_doc::DisplayUnits, unit: &str, v: f64) -> String {
    let (v, decimals) = match unit {
        "angle" => (v.to_degrees(), u.angle_decimals.max(4)),
        "length" => (v / u.length.mm(), u.length_decimals.max(4)),
        _ => (v, 4),
    };
    let text = chamfera_doc::units::fixed(v, decimals);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

fn unit_symbol(u: chamfera_doc::DisplayUnits, unit: &str) -> &'static str {
    match unit {
        "length" => u.length.symbol(),
        "angle" => "deg",
        _ => "—",
    }
}

/// One row of the table, read from `param::table`.
#[derive(Debug, Clone)]
struct Row {
    name: String,
    model: bool,
    owner: String,
    expression: Option<String>,
    value: Option<f64>,
    unit: String,
    comment: String,
    favourite: bool,
    used_by: Vec<String>,
    error: Option<String>,
    overridden: bool,
}

impl Row {
    fn from(v: &Value) -> Row {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        Row {
            name: s("name"),
            model: v["kind"] == "model",
            owner: s("owner_name"),
            expression: v["expression"].as_str().map(str::to_owned),
            value: v["value"].as_f64(),
            unit: s("unit"),
            comment: s("comment"),
            favourite: v["favourite"].as_bool().unwrap_or(false),
            used_by: v["used_by"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            error: v["error"].as_str().map(str::to_owned),
            overridden: v["overridden"].as_bool().unwrap_or(false),
        }
    }

    /// What the expression box shows: the expression, or the plain value with its unit.
    fn text(&self, u: chamfera_doc::DisplayUnits) -> String {
        match (&self.expression, self.value) {
            (Some(e), _) => e.clone(),
            (None, Some(v)) => {
                let n = show_number(u, &self.unit, v);
                match self.unit.as_str() {
                    "length" => format!("{n} {}", u.length.symbol()),
                    "angle" => format!("{n} deg"),
                    _ => n,
                }
            }
            (None, None) => String::new(),
        }
    }

    fn matches(&self, filter: Filter, search: &str) -> bool {
        let kind = match filter {
            Filter::All => true,
            Filter::User => !self.model,
            Filter::Model => self.model,
            Filter::Favourites => self.favourite,
            Filter::Errors => self.error.is_some(),
        };
        if !kind {
            return false;
        }
        let q = search.trim().to_lowercase();
        q.is_empty()
            || [
                self.name.as_str(),
                self.expression.as_deref().unwrap_or(""),
                self.comment.as_str(),
                self.owner.as_str(),
            ]
            .iter()
            .any(|t| t.to_lowercase().contains(&q))
    }
}

/// An edit to run once the window is drawn.
enum Edit {
    Set {
        name: String,
        expression: String,
        unit: Option<&'static str>,
        new: bool,
    },
    Rename {
        name: String,
        new_name: String,
    },
    Comment {
        name: String,
        comment: String,
    },
    Favourite {
        name: String,
        on: bool,
    },
    Remove(String),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ImportCsv,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ExportCsv,
}

/// A five-pointed star, filled when `on`, that toggles on click.
fn star(ui: &mut egui::Ui, look: Look, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    let c = rect.center();
    let (outer, inner) = (7.5_f32, 3.2_f32);
    let pts: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = std::f32::consts::PI * (i as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
            egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    let colour = if on {
        look.c(|t| t.accent)
    } else if resp.hovered() {
        look.c(|t| t.text)
    } else {
        look.c(|t| t.text_muted)
    };
    let fill = if on {
        colour
    } else {
        egui::Color32::TRANSPARENT
    };
    // Fill as a fan from the centre: the star is not convex.
    if on {
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            ui.painter().add(egui::Shape::convex_polygon(
                vec![c, a, b],
                fill,
                egui::Stroke::NONE,
            ));
        }
    }
    ui.painter().add(egui::Shape::closed_line(
        pts,
        egui::Stroke::new(1.2, colour),
    ));
    resp
}

const W_NAME: f32 = 150.0;
const W_EXPR: f32 = 190.0;
const W_VALUE: f32 = 70.0;
const W_UNIT: f32 = 32.0;
const W_COMMENT: f32 = 150.0;
const W_USED: f32 = 150.0;

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
        let mut rows: Vec<Row> = chamfera_engine::cmd::param::table(&self.session.doc)
            .iter()
            .map(Row::from)
            .collect();
        // Favourites first, then user parameters, then model parameters.
        rows.sort_by_key(|r| (!r.favourite, r.model));
        let config = self.session.doc.configurations.active.clone();
        let error = match &self.message {
            Some((m, true)) => Some(m.clone()),
            _ => None,
        };
        let mut edits: Vec<Edit> = Vec::new();
        let params = &mut self.params;
        let shown: Vec<&Row> = rows
            .iter()
            .filter(|r| r.matches(params.filter, &params.search))
            .collect();
        let total = rows.len();
        egui::Window::new("Parameters")
            .id(egui::Id::unique("parameters window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(1000.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(1000.0);
                if widgets::dialog_title(ui, look, Icon::Parameters, "Parameters") {
                    open = false;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [260.0, 26.0],
                            egui::TextEdit::singleline(&mut params.search)
                                .hint_text("Search names, expressions, comments")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        egui::ComboBox::from_id_salt("parameter filter")
                            .selected_text(params.filter.name())
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for f in Filter::ALL {
                                    ui.selectable_value(&mut params.filter, f, f.name());
                                }
                            });
                        ui.label(
                            egui::RichText::new(format!("{} of {total}", shown.len()))
                                .size(widgets::T_SMALL)
                                .color(look.c(|t| t.text_muted)),
                        );
                        if let Some(c) = &config {
                            ui.label(
                                egui::RichText::new(format!("Configuration: {c}"))
                                    .size(widgets::T_SMALL)
                                    .color(look.c(|t| t.accent)),
                            );
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::secondary_button(ui, look, "Export CSV…").clicked() {
                                edits.push(Edit::ExportCsv);
                            }
                            if widgets::secondary_button(ui, look, "Import CSV…").clicked() {
                                edits.push(Edit::ImportCsv);
                            }
                        });
                    });
                    ui.add_space(widgets::S3);
                    egui::ScrollArea::vertical()
                        .max_height(440.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            parameter_grid(ui, look, units, params, &shown, &mut edits);
                        });
                    ui.add_space(widgets::S3);
                    ui.separator();
                    ui.add_space(widgets::S3);
                    // A row to add a user parameter.
                    ui.horizontal(|ui| {
                        ui.add_space(22.0 + widgets::S3);
                        ui.add_sized(
                            [W_NAME, 26.0],
                            egui::TextEdit::singleline(&mut params.new_name)
                                .hint_text("name")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        ui.add_sized(
                            [W_EXPR, 26.0],
                            egui::TextEdit::singleline(&mut params.new_expression)
                                .hint_text("e.g. 80 mm or if(w > 40 mm, 2, 1)")
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        egui::ComboBox::from_id_salt("new parameter unit")
                            .selected_text(UNITS.get(params.new_unit).map_or("", |u| u.1))
                            .width(80.0)
                            .show_ui(ui, |ui| {
                                for (i, (_, text)) in UNITS.iter().enumerate() {
                                    ui.selectable_value(&mut params.new_unit, i, *text);
                                }
                            });
                        let can_add = !params.new_name.trim().is_empty()
                            && !params.new_expression.trim().is_empty();
                        if widgets::primary_button(ui, look, "Add", can_add).clicked() {
                            edits.push(Edit::Set {
                                name: params.new_name.trim().to_owned(),
                                expression: params.new_expression.trim().to_owned(),
                                unit: Some(UNITS.get(params.new_unit).map_or("length", |u| u.0)),
                                new: true,
                            });
                        }
                    });
                    ui.add_space(widgets::S2);
                    ui.label(
                        egui::RichText::new(
                            "Expressions take units (mm, cm, in, ft, deg, rad), + − × ÷ ^, comparisons, \
                             if(c, a, b), sqrt, abs, min, max, round, floor, ceil, sin … atan2, pi, e, \
                             and any parameter, model ones too (Extrude1.distance).",
                        )
                        .size(widgets::T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                    );
                    if params.row_error.is_none()
                        && let Some(m) = &error
                    {
                        ui.add_space(widgets::S3);
                        error_line(ui, look, m);
                    }
                });
            });
        self.params.open = open && self.params.open;
        self.apply_parameter_edits(edits);
    }

    fn apply_parameter_edits(&mut self, edits: Vec<Edit>) {
        for e in edits {
            let (row, ok) = match e {
                Edit::Set {
                    name,
                    expression,
                    unit,
                    new,
                } => {
                    let mut p = json!({ "name": name, "expression": expression });
                    if let Some(u) = unit {
                        p["unit"] = json!(u);
                    }
                    let ok = self.run("param.set", p).is_some();
                    if ok {
                        self.params.edits.remove(&name);
                        if new {
                            self.params.new_name.clear();
                            self.params.new_expression.clear();
                        }
                        self.message = Some((format!("{name} = {expression}"), false));
                    }
                    (if new { None } else { Some(name) }, ok)
                }
                Edit::Rename { name, new_name } => {
                    let ok = self
                        .run(
                            "param.rename",
                            json!({ "name": name, "new_name": new_name }),
                        )
                        .is_some();
                    if ok {
                        self.params.names.remove(&name);
                        self.message = Some((format!("{name} is now {new_name}"), false));
                    }
                    (Some(name), ok)
                }
                Edit::Comment { name, comment } => {
                    let ok = self
                        .run(
                            "param.annotate",
                            json!({ "name": name, "comment": comment }),
                        )
                        .is_some();
                    if ok {
                        self.params.comments.remove(&name);
                    }
                    (Some(name), ok)
                }
                Edit::Favourite { name, on } => {
                    let ok = self
                        .run("param.annotate", json!({ "name": name, "favourite": on }))
                        .is_some();
                    (Some(name), ok)
                }
                Edit::Remove(name) => {
                    let ok = self.run("param.remove", json!({ "name": name })).is_some();
                    if ok {
                        self.params.edits.remove(&name);
                    }
                    (Some(name), ok)
                }
                Edit::ImportCsv => {
                    self.parameters_csv(false);
                    (None, true)
                }
                Edit::ExportCsv => {
                    self.parameters_csv(true);
                    (None, true)
                }
            };
            self.refresh_sketch_status();
            match (row, ok) {
                (Some(name), false) => {
                    let m = self
                        .message
                        .as_ref()
                        .map(|m| m.0.clone())
                        .unwrap_or_default();
                    // Show the reason without the command's label in front.
                    let m = m.split_once(": ").map_or(m.clone(), |(_, r)| r.to_owned());
                    self.params.row_error = Some((name, m));
                }
                (_, true) => self.params.row_error = None,
                (None, false) => {}
            }
        }
    }

    /// Imports or exports the user parameters as CSV through a file dialog.
    #[cfg(not(target_arch = "wasm32"))]
    fn parameters_csv(&mut self, export: bool) {
        let d = rfd::FileDialog::new().add_filter("CSV", &["csv"]);
        let path = if export {
            d.set_title("Export Parameters")
                .set_file_name(format!("{}-parameters.csv", self.design_name()))
                .save_file()
        } else {
            d.set_title("Import Parameters").pick_file()
        };
        let Some(path) = path else {
            return;
        };
        let id = if export {
            "param.export_csv"
        } else {
            "param.import_csv"
        };
        let text = path.to_string_lossy().into_owned();
        if self.run(id, json!({ "path": text })).is_some() {
            self.message = Some((format!("{}: {text}", label(id)), false));
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn parameters_csv(&mut self, _export: bool) {}
}

fn error_line(ui: &mut egui::Ui, look: Look, m: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
        let e = look.c(|t| t.error);
        look.icons.paint(ui.painter(), Icon::Error, r, e, e);
        ui.label(egui::RichText::new(m).color(e).size(widgets::T_SMALL));
    });
}

fn header(ui: &mut egui::Ui, look: Look, text: &str, width: f32, align: egui::Align) {
    let t = egui::RichText::new(text)
        .size(widgets::T_CAPTION)
        .color(look.c(|t| t.text_muted));
    widgets::fixed_text(ui, width, t, align);
}

fn parameter_grid(
    ui: &mut egui::Ui,
    look: Look,
    units: chamfera_doc::DisplayUnits,
    params: &mut ParamsUi,
    rows: &[&Row],
    edits: &mut Vec<Edit>,
) {
    egui::Grid::new("parameter grid")
        .num_columns(8)
        .spacing([widgets::S3, widgets::S2])
        .min_col_width(0.0)
        .show(ui, |ui| {
            ui.label("");
            header(ui, look, "NAME", W_NAME, egui::Align::Min);
            header(ui, look, "EXPRESSION", W_EXPR, egui::Align::Min);
            header(ui, look, "VALUE", W_VALUE, egui::Align::Max);
            header(ui, look, "UNIT", W_UNIT, egui::Align::Min);
            header(ui, look, "COMMENT", W_COMMENT, egui::Align::Min);
            header(ui, look, "USED BY", W_USED, egui::Align::Min);
            ui.label("");
            ui.end_row();
            if rows.is_empty() {
                ui.label("");
                ui.label(
                    egui::RichText::new("No parameters match: add one below or clear the search")
                        .color(look.c(|t| t.text_muted)),
                );
                ui.end_row();
            }
            let mut section = "";
            for r in rows {
                let this = if r.favourite {
                    "FAVOURITES"
                } else if r.model {
                    "MODEL PARAMETERS"
                } else {
                    "USER PARAMETERS"
                };
                if this != section {
                    section = this;
                    ui.label("");
                    ui.label(
                        egui::RichText::new(this)
                            .size(widgets::T_CAPTION)
                            .strong()
                            .color(look.c(|t| t.accent)),
                    );
                    ui.end_row();
                }
                parameter_row(ui, look, units, params, r, edits);
            }
        });
}

fn parameter_row(
    ui: &mut egui::Ui,
    look: Look,
    units: chamfera_doc::DisplayUnits,
    params: &mut ParamsUi,
    r: &Row,
    edits: &mut Vec<Edit>,
) {
    let name = &r.name;
    let tip = if r.favourite {
        "Unpin"
    } else {
        "Pin at the top"
    };
    if star(ui, look, r.favourite).on_hover_text(tip).clicked() {
        edits.push(Edit::Favourite {
            name: name.clone(),
            on: !r.favourite,
        });
    }
    // Name: edited in place, renamed on Enter or when focus leaves.
    let text = params
        .names
        .entry(name.clone())
        .or_insert_with(|| name.clone());
    let resp = ui.add_sized(
        [W_NAME, 26.0],
        egui::TextEdit::singleline(text)
            .font(egui::TextStyle::Monospace)
            .align(egui::Align2::LEFT_CENTER),
    );
    let resp = if r.model {
        resp.on_hover_text(format!(
            "A value of {}; rename it to give it a name of your own",
            r.owner
        ))
    } else {
        resp
    };
    if resp.lost_focus() {
        let new_name = text.trim().to_owned();
        if new_name != *name && !new_name.is_empty() {
            edits.push(Edit::Rename {
                name: name.clone(),
                new_name,
            });
        } else {
            params.names.remove(name);
        }
    }
    // Expression.
    let current = r.text(units);
    let text = params
        .edits
        .entry(name.clone())
        .or_insert_with(|| current.clone());
    let failed = params.row_error.as_ref().is_some_and(|(n, _)| n == name) || r.error.is_some();
    let mut edit = egui::TextEdit::singleline(text).align(egui::Align2::LEFT_CENTER);
    if failed {
        edit = edit.text_color(look.c(|t| t.error));
    }
    let resp = ui.add_sized([W_EXPR, 26.0], edit);
    if resp.lost_focus() {
        if *text != current {
            edits.push(Edit::Set {
                name: name.clone(),
                expression: text.clone(),
                unit: None,
                new: false,
            });
        } else {
            params.edits.remove(name);
        }
    } else if !resp.has_focus() && *text != current && !failed {
        // The design changed underneath (undo, another edit): follow it.
        *text = current;
    }
    // Value, in display units.
    let value = r
        .value
        .map_or_else(|| "?".to_owned(), |v| show_number(units, &r.unit, v));
    let colour = if r.overridden {
        look.c(|t| t.accent)
    } else {
        look.c(|t| t.text)
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(W_VALUE, 26.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.max.x, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        egui::FontId::proportional(widgets::T_BODY),
        colour,
    );
    if r.overridden {
        resp.on_hover_text("Set by the active configuration");
    }
    ui.label(egui::RichText::new(unit_symbol(units, &r.unit)).color(look.c(|t| t.text_muted)));
    // Comment.
    let text = params
        .comments
        .entry(name.clone())
        .or_insert_with(|| r.comment.clone());
    let resp = ui.add_sized(
        [W_COMMENT, 26.0],
        egui::TextEdit::singleline(text)
            .hint_text("comment")
            .align(egui::Align2::LEFT_CENTER),
    );
    if resp.lost_focus() && *text != r.comment {
        edits.push(Edit::Comment {
            name: name.clone(),
            comment: text.clone(),
        });
    } else if !resp.has_focus() && *text != r.comment {
        *text = r.comment.clone();
    }
    // Used by.
    let used = if r.used_by.is_empty() {
        "—".to_owned()
    } else {
        r.used_by.join(", ")
    };
    let short: String = if used.chars().count() > 24 {
        format!("{}…", used.chars().take(23).collect::<String>())
    } else {
        used.clone()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(W_USED, 26.0), egui::Sense::hover());
    ui.painter().text(
        egui::pos2(rect.min.x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        short,
        egui::FontId::proportional(widgets::T_SMALL),
        look.c(|t| t.text_muted),
    );
    if !r.used_by.is_empty() {
        resp.on_hover_text(used);
    }
    if r.model {
        ui.label("");
    } else if widgets::icon_button(
        ui,
        look,
        Icon::Delete,
        26.0,
        16.0,
        r.used_by.is_empty(),
        false,
    )
    .on_hover_text(if r.used_by.is_empty() {
        format!("{name}: {}", label("param.remove"))
    } else {
        format!("{name} is in use: change its users first")
    })
    .clicked()
    {
        edits.push(Edit::Remove(name.clone()));
    }
    ui.end_row();
    // A refused edit or a broken expression, under its row.
    let why = params
        .row_error
        .as_ref()
        .filter(|(n, _)| n == name)
        .map(|(_, m)| m.clone())
        .or_else(|| r.error.clone());
    if let Some(m) = why {
        ui.label("");
        ui.label("");
        error_line(ui, look, &m);
        ui.end_row();
    }
}

//! The SHEET METAL workspace: Base Flange (click a closed profile), Flange (click a
//! straight edge, set height, angle, position and side), Flat Pattern, DXF export and the
//! sheet metal rules. Acts only through `sheet.*` and `feature.edit`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{FeatureKind, FlangePosition, FlangeSide, ReliefShape, SheetFlange};
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::{Value, json};

/// The Base Flange tool: the rule to use; a click inside a profile makes the flange.
pub struct BaseTool {
    pub rule: String,
}

/// The Flange tool, new or editing a flange.
pub struct FlangeTool {
    /// The flange being edited, if any.
    pub editing: Option<u32>,
    pub edge: Option<String>,
    pub hover: Option<String>,
    pub height: String,
    pub angle: String,
    pub position: FlangePosition,
    pub side: FlangeSide,
    pub offset_start: String,
    pub offset_end: String,
}

impl FlangeTool {
    pub fn new() -> FlangeTool {
        FlangeTool {
            editing: None,
            edge: None,
            hover: None,
            height: "20 mm".into(),
            angle: "90 deg".into(),
            position: FlangePosition::Inside,
            side: FlangeSide::Same,
            offset_start: "0 mm".into(),
            offset_end: "0 mm".into(),
        }
    }

    fn editing(id: u32, f: &SheetFlange) -> FlangeTool {
        FlangeTool {
            editing: Some(id),
            edge: Some(f.edge.clone()),
            hover: None,
            height: f.height.clone(),
            angle: f.angle.clone(),
            position: f.position,
            side: f.side,
            offset_start: f.offset_start.clone().unwrap_or_else(|| "0 mm".into()),
            offset_end: f.offset_end.clone().unwrap_or_else(|| "0 mm".into()),
        }
    }

    /// The `sheet.flange` (or `feature.edit`) parameters.
    fn params(&self) -> Value {
        let mut p = json!({
            "height": self.height.trim(),
            "angle": self.angle.trim(),
            "position": self.position.key(),
            "side": self.side.key(),
            "offset_start": self.offset_start.trim(),
            "offset_end": self.offset_end.trim(),
        });
        if let Some(e) = &self.edge {
            p["edge"] = json!(e);
        }
        if let Some(id) = self.editing {
            p["feature"] = json!(id);
        }
        p
    }
}

impl Default for FlangeTool {
    fn default() -> Self {
        FlangeTool::new()
    }
}

/// The rule being edited in the Rules dialog (values as typed).
#[derive(Clone)]
pub struct RuleForm {
    pub name: String,
    pub thickness: String,
    pub bend_radius: String,
    pub k_factor: String,
    pub relief: ReliefShape,
    pub relief_width: String,
    pub relief_depth: String,
}

impl RuleForm {
    fn from_rule(r: &chamfera_doc::SheetRule) -> RuleForm {
        RuleForm {
            name: r.name.clone(),
            thickness: r.thickness.to_string(),
            bend_radius: r.bend_radius.to_string(),
            k_factor: r.k_factor.to_string(),
            relief: r.relief,
            relief_width: r.relief_width.to_string(),
            relief_depth: r.relief_depth.to_string(),
        }
    }
}

/// The SHEET METAL workspace's state.
#[derive(Default)]
pub struct SheetUi {
    /// The SHEET METAL tab is the active workspace.
    pub active: bool,
    pub base: Option<BaseTool>,
    pub flange: Option<FlangeTool>,
    pub rules: Option<RuleForm>,
}

fn position_name(p: FlangePosition) -> &'static str {
    match p {
        FlangePosition::Inside => "Inside (keep outline)",
        FlangePosition::Outside => "Outside",
        FlangePosition::BendOutside => "Bend outside",
    }
}

fn relief_name(r: ReliefShape) -> &'static str {
    match r {
        ReliefShape::Rectangular => "Rectangular",
        ReliefShape::Obround => "Obround",
        ReliefShape::Rip => "Rip",
    }
}

/// The timeline's icon and line for a sheet metal feature, if it is one.
pub(crate) fn feature_summary(kind: &FeatureKind) -> Option<(Icon, String)> {
    Some(match kind {
        FeatureKind::SheetBase(b) => (
            Icon::SheetBase,
            format!("Base flange · rule {} · sketch {}", b.rule, b.sketch),
        ),
        FeatureKind::SheetFlange(f) => (
            Icon::SheetFlange,
            format!("Flange {} at {} · {}", f.height, f.angle, f.edge),
        ),
        FeatureKind::SheetUnfold(u) => (
            Icon::SheetUnfold,
            format!("Flat pattern of feature {}", u.base),
        ),
        _ => return None,
    })
}

/// A text field in a dialog grid.
fn field(ui: &mut egui::Ui, look: Look, name: &str, value: &mut String, hint: &str) -> bool {
    widgets::field_label(ui, look, name);
    let edit = ui.add_sized(
        widgets::FIELD,
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.end_row();
    edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

impl ChamferaApp {
    /// True while a sheet metal tool takes viewport clicks.
    pub(crate) fn sheet_tool_open(&self) -> bool {
        self.sheet_metal.base.is_some() || self.sheet_metal.flange.is_some()
    }

    /// Base Flange: waits for a click inside a closed profile, with the first rule.
    pub(crate) fn open_base_flange(&mut self) {
        let rule = self
            .session
            .doc
            .sheet_rules
            .first()
            .map_or_else(|| "Default".to_owned(), |r| r.name.clone());
        self.sheet_metal.base = Some(BaseTool { rule });
        self.message = Some(("Click inside a closed profile".into(), false));
    }

    /// Flange: waits for a click on a straight edge of the sheet.
    pub(crate) fn open_flange(&mut self) {
        self.sheet_metal.flange = Some(FlangeTool::new());
        self.message = Some(("Click a straight edge of the sheet".into(), false));
    }

    /// A start-up script that used sheet metal commands opens the SHEET METAL tab, and
    /// reports its flat pattern's size if it made one.
    pub(crate) fn sheet_from_script(&mut self, steps: &[Value], replies: &[Value]) {
        let sheet = |v: &Value| {
            v["command"]
                .as_str()
                .is_some_and(|c| c.starts_with("sheet."))
        };
        if !steps.iter().any(sheet) {
            return;
        }
        self.sheet_metal.active = true;
        let flat = steps
            .iter()
            .zip(replies)
            .rev()
            .find(|(s, _)| s["command"] == "sheet.flat_pattern");
        if let Some((_, r)) = flat {
            let (l, w) = (
                r["length"].as_f64().unwrap_or(0.0),
                r["width"].as_f64().unwrap_or(0.0),
            );
            let u = self.units();
            self.message = Some((
                format!("Flat pattern {} × {}", u.length(l), u.length(w)),
                false,
            ));
        }
    }

    /// Runs `sheet.flat_pattern` and reports its size.
    pub(crate) fn make_flat_pattern(&mut self) {
        if let Some(r) = self.run("sheet.flat_pattern", json!({})) {
            let (l, w) = (
                r["length"].as_f64().unwrap_or(0.0),
                r["width"].as_f64().unwrap_or(0.0),
            );
            let n = r["bends"].as_array().map_or(0, Vec::len);
            self.message = Some((
                format!(
                    "Flat pattern {} × {} · {n} bend{}",
                    self.units().length(l),
                    self.units().length(w),
                    if n == 1 { "" } else { "s" }
                ),
                false,
            ));
            self.fit_pending = true;
        }
    }

    pub(crate) fn open_rules(&mut self) {
        let form = match self.session.doc.sheet_rules.first() {
            Some(r) => RuleForm::from_rule(r),
            None => RuleForm::from_rule(&chamfera_doc::SheetRule::default_named("Default")),
        };
        self.sheet_metal.rules = Some(form);
    }

    /// A viewport click or hover while a sheet tool is open. True when it used the click.
    pub(crate) fn sheet_input(
        &mut self,
        at: Option<(f64, f64)>,
        clicked: bool,
        w: f64,
        h: f64,
    ) -> bool {
        if self.sheet_metal.flange.is_some() {
            let hover = at.and_then(|(x, y)| self.session.edge_at(x, y, w, h).map(|e| e.2));
            if let Some(t) = self.sheet_metal.flange.as_mut() {
                t.hover = hover.clone();
                if clicked && hover.is_some() {
                    t.edge = hover;
                }
            }
            return clicked;
        }
        if let (Some(tool), true, Some((x, y))) = (self.sheet_metal.base.as_ref(), clicked, at) {
            let rule = tool.rule.clone();
            if let Some((sketch, seed, _)) = self.session.profile_at(x, y, w, h) {
                let mut p = json!({ "sketch": sketch, "profile": { "x": seed.x, "y": seed.y } });
                if self.session.doc.sheet_rule(&rule).is_ok() {
                    p["rule"] = json!(rule);
                }
                if let Some(r) = self.run("sheet.base", p) {
                    let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                    self.message = Some((
                        format!(
                            "{} · volume {}",
                            r["name"].as_str().unwrap_or("Base flange"),
                            self.units().volume(v)
                        ),
                        false,
                    ));
                    self.sheet_metal.base = None;
                }
            }
            return true;
        }
        false
    }

    /// The picked flange edge (highlighted) and the one under the pointer.
    pub(crate) fn sheet_overlays(&self) -> Vec<Overlay> {
        let Some(t) = self.sheet_metal.flange.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(e) = &t.edge {
            out.push(Overlay {
                style: OverlayStyle::Highlight,
                lines: self.edge_lines(std::iter::once(e)),
                points: Vec::new(),
                on_top: true,
            });
        }
        if let Some(h) = t.hover.as_ref().filter(|h| Some(*h) != t.edge.as_ref()) {
            out.push(Overlay {
                style: OverlayStyle::Preview,
                lines: self.edge_lines(std::iter::once(h)),
                points: Vec::new(),
                on_top: true,
            });
        }
        out
    }

    /// Opens the Flange tool on an existing flange (from the timeline).
    pub(crate) fn edit_sheet_feature(&mut self, id: u32) -> bool {
        let Ok(f) = self.session.doc.feature(id) else {
            return false;
        };
        match &f.kind {
            FeatureKind::SheetFlange(fl) => {
                self.sheet_metal.flange = Some(FlangeTool::editing(id, fl));
                self.sheet_metal.active = true;
                true
            }
            FeatureKind::SheetBase(_) | FeatureKind::SheetUnfold(_) => {
                self.sheet_metal.active = true;
                self.open_rules();
                true
            }
            _ => false,
        }
    }

    /// The sheet metal dialogs (Base Flange, Flange, Rules).
    pub(crate) fn sheet_dialogs(&mut self, ctx: &egui::Context) {
        self.base_dialog(ctx);
        self.flange_dialog(ctx);
        self.rules_dialog(ctx);
    }

    fn base_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let names: Vec<String> = self
            .session
            .doc
            .sheet_rules
            .iter()
            .map(|r| r.name.clone())
            .collect();
        let Some(tool) = self.sheet_metal.base.as_mut() else {
            return;
        };
        let mut cancel = false;
        let title = label("sheet.base");
        egui::Window::new(title)
            .id(egui::Id::unique("sheet base dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::SheetBase, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sheet base grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Profile");
                            ui.label(
                                egui::RichText::new("Click inside a closed profile")
                                    .color(look.c(|t| t.text_muted)),
                            );
                            ui.end_row();
                            widgets::field_label(ui, look, "Rule");
                            if names.is_empty() {
                                ui.label("Default (2 mm, made now)");
                            } else {
                                egui::ComboBox::from_id_salt("sheet base rule")
                                    .selected_text(tool.rule.clone())
                                    .width(widgets::FIELD_WIDTH)
                                    .show_ui(ui, |ui| {
                                        for n in &names {
                                            ui.selectable_value(&mut tool.rule, n.clone(), n);
                                        }
                                    });
                            }
                            ui.end_row();
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if cancel {
            self.sheet_metal.base = None;
            self.message = None;
        }
    }

    fn flange_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.sheet_metal.flange.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        let title = label("sheet.flange");
        let ready = tool.edge.is_some() && !tool.height.trim().is_empty();
        egui::Window::new(title)
            .id(egui::Id::unique("sheet flange dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::SheetFlange, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sheet flange grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Edge");
                            let (text, c) = match &tool.edge {
                                Some(e) => (e.clone(), look.c(|t| t.text)),
                                None => ("Click a straight edge".into(), look.c(|t| t.text_muted)),
                            };
                            ui.label(egui::RichText::new(text).color(c));
                            ui.end_row();
                            ok |= field(ui, look, "Height", &mut tool.height, "20 mm");
                            ok |= field(ui, look, "Angle", &mut tool.angle, "90 deg");
                            widgets::field_label(ui, look, "Position");
                            egui::ComboBox::from_id_salt("flange position")
                                .selected_text(position_name(tool.position))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for p in [
                                        FlangePosition::Inside,
                                        FlangePosition::Outside,
                                        FlangePosition::BendOutside,
                                    ] {
                                        ui.selectable_value(
                                            &mut tool.position,
                                            p,
                                            position_name(p),
                                        );
                                    }
                                });
                            ui.end_row();
                            widgets::field_label(ui, look, "Side");
                            egui::ComboBox::from_id_salt("flange side")
                                .selected_text(match tool.side {
                                    FlangeSide::Same => "Toward the face",
                                    FlangeSide::Opposite => "Flipped",
                                })
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut tool.side,
                                        FlangeSide::Same,
                                        "Toward the face",
                                    );
                                    ui.selectable_value(
                                        &mut tool.side,
                                        FlangeSide::Opposite,
                                        "Flipped",
                                    );
                                });
                            ui.end_row();
                            ok |= field(ui, look, "Start offset", &mut tool.offset_start, "0 mm");
                            ok |= field(ui, look, "End offset", &mut tool.offset_end, "0 mm");
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "OK", ready).clicked() {
                        ok = true;
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if ok && ready {
            let params = tool.params();
            let command = if tool.editing.is_some() {
                "feature.edit"
            } else {
                "sheet.flange"
            };
            if let Some(r) = self.run(command, params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {}",
                        r["name"].as_str().unwrap_or(title),
                        self.units().volume(v)
                    ),
                    false,
                ));
                self.sheet_metal.flange = None;
            }
        }
        if cancel {
            self.sheet_metal.flange = None;
            self.message = None;
        }
    }

    /// Apply in the Rules dialog: saves the rule through `sheet.rule`. Lengths are read
    /// in the design's unit unless they name one (`3 in`); a field that cannot be read
    /// stops it with an error naming the field, instead of keeping the old value.
    pub(crate) fn apply_rule_form(&mut self) {
        let Some(form) = self.sheet_metal.rules.clone() else {
            return;
        };
        use chamfera_doc::expr::{self, Dim};
        let unit = self.session.doc.units.length.mm();
        let mut p = json!({ "name": form.name.trim(), "relief": form.relief.key() });
        for (key, name, v, dim) in [
            ("thickness", "Thickness", &form.thickness, Dim::LENGTH),
            ("bend_radius", "Bend radius", &form.bend_radius, Dim::LENGTH),
            ("k_factor", "K-factor", &form.k_factor, Dim::NONE),
            (
                "relief_width",
                "Relief width",
                &form.relief_width,
                Dim::LENGTH,
            ),
            (
                "relief_depth",
                "Relief depth",
                &form.relief_depth,
                Dim::LENGTH,
            ),
        ] {
            if v.trim().is_empty() {
                continue;
            }
            match expr::evaluate_as_in(v.trim(), dim, unit, &expr::NoNames) {
                Ok(x) => p[key] = json!(x),
                Err(e) => {
                    self.message = Some((format!("{name} {:?}: {e}", v.trim()), true));
                    return;
                }
            }
        }
        if self.run("sheet.rule", p).is_some() {
            self.message = Some(("Sheet metal rule saved".into(), false));
        }
    }

    fn rules_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let rules: Vec<chamfera_doc::SheetRule> = self.session.doc.sheet_rules.clone();
        let Some(form) = self.sheet_metal.rules.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        let mut pick: Option<chamfera_doc::SheetRule> = None;
        let title = label("sheet.rule");
        egui::Window::new(title)
            .id(egui::Id::unique("sheet rules dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Parameters, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sheet rules grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Rule");
                            egui::ComboBox::from_id_salt("sheet rule pick")
                                .selected_text(form.name.clone())
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for r in &rules {
                                        if ui
                                            .selectable_label(r.name == form.name, &r.name)
                                            .clicked()
                                        {
                                            pick = Some(r.clone());
                                        }
                                    }
                                });
                            ui.end_row();
                            ok |= field(ui, look, "Name", &mut form.name, "Steel 2 mm");
                            ok |= field(ui, look, "Thickness", &mut form.thickness, "2");
                            ok |= field(ui, look, "Bend radius", &mut form.bend_radius, "2");
                            ok |= field(ui, look, "K-factor", &mut form.k_factor, "0.44");
                            widgets::field_label(ui, look, "Relief");
                            egui::ComboBox::from_id_salt("sheet relief")
                                .selected_text(relief_name(form.relief))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for r in [
                                        ReliefShape::Rectangular,
                                        ReliefShape::Obround,
                                        ReliefShape::Rip,
                                    ] {
                                        ui.selectable_value(&mut form.relief, r, relief_name(r));
                                    }
                                });
                            ui.end_row();
                            ok |= field(ui, look, "Relief width", &mut form.relief_width, "2");
                            ok |= field(ui, look, "Relief depth", &mut form.relief_depth, "2");
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Apply", true).clicked() {
                        ok = true;
                    }
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        cancel = true;
                    }
                });
            });
        if let Some(r) = pick {
            *form = RuleForm::from_rule(&r);
        }
        if ok {
            self.apply_rule_form();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if cancel {
            self.sheet_metal.rules = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Rules dialog reads lengths with their units and refuses what it cannot read
    /// (QA user guide pass, sheet metal 6: "3 in" and "abc" were skipped silently and
    /// "saved" shown). 3 in is 76.2 mm; a plain 2 is 2 mm in a millimetre design.
    #[test]
    fn the_rules_dialog_reads_units_and_refuses_unreadable_values() {
        let mut app = ChamferaApp::headless();
        let form = |thickness: &str| RuleForm {
            name: "Plate".into(),
            thickness: thickness.into(),
            bend_radius: "2".into(),
            k_factor: "0.4".into(),
            relief: chamfera_doc::ReliefShape::default(),
            relief_width: String::new(),
            relief_depth: String::new(),
        };
        app.sheet_metal.rules = Some(form("3 in"));
        app.apply_rule_form();
        let r = app.session.doc.sheet_rule("Plate").unwrap().clone();
        assert!((r.thickness - 76.2).abs() < 1e-9, "{}", r.thickness);
        assert!((r.bend_radius - 2.0).abs() < 1e-9);
        app.sheet_metal.rules = Some(form("abc"));
        app.apply_rule_form();
        let (text, error) = app.message.clone().unwrap();
        assert!(error && text.contains("Thickness"), "{text}");
        let after = app.session.doc.sheet_rule("Plate").unwrap();
        assert!(
            (after.thickness - 76.2).abs() < 1e-9,
            "the rule is unchanged"
        );
    }

    #[test]
    fn the_flange_tool_builds_the_flange_command() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 100, "y2": 50 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "sheet.base",
            json!({ "sketch": 1, "profile": { "x": 50, "y": 25 } }),
        );
        let edges = app.run("solid.edges", json!({})).unwrap();
        let edge = edges["solids"][0]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["from"][0] == 100.0 && e["to"][0] == 100.0 && e["from"][2] == 2.0)
            .map(|e| e["edge"].as_str().unwrap().to_owned())
            .unwrap();
        let mut tool = FlangeTool::new();
        tool.edge = Some(edge);
        tool.height = "30 mm".into();
        let r = app.run("sheet.flange", tool.params()).unwrap();
        // (100 - 4) * 50 * 2 + (π/4)(4² - 2²) * 50 + 26 * 2 * 50 for the default rule.
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        let expected = 9600.0 + std::f64::consts::PI / 4.0 * 12.0 * 50.0 + 2600.0;
        assert!((v - expected).abs() < 1e-6, "{v}");
        // Editing it from the timeline opens the tool on its values.
        let id = r["feature"].as_u64().unwrap() as u32;
        assert!(app.edit_sheet_feature(id));
        let t = app.sheet_metal.flange.as_ref().unwrap();
        assert_eq!(t.height, "30 mm");
        assert_eq!(t.params()["feature"], id);
    }
}

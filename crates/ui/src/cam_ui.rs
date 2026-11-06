//! The MANUFACTURE workspace: its ribbon (setup, tools, 2D operations, generate, post),
//! the setup and operations tree in the left panel, the operation dialog (pick faces or
//! edges in the viewport, choose the tool, depths and stepover) and Post G-code in the
//! File menu. Acts only through `cam.*` commands; toolpaths are drawn by the engine as
//! viewport overlays (rapids dashed, feeds solid, theme tokens `toolpath_*`).

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::{Value, json};

/// The Manufacture workspace's state.
#[derive(Default)]
pub struct CamUi {
    /// The MANUFACTURE tab is showing.
    pub active: bool,
    pub dialog: Option<OpDialog>,
    pub tool_dialog: Option<ToolDialog>,
}

/// The operation dialog while it is open (adding, or editing `edit`).
pub struct OpDialog {
    pub kind: &'static str,
    pub edit: Option<u32>,
    pub tool: Option<u32>,
    pub face: Option<String>,
    pub edges: Vec<String>,
    /// Contour: clicks pick edges instead of a face.
    pub pick_edges: bool,
    pub stepover: String,
    pub stepdown: String,
    pub outside: bool,
    pub entry: &'static str,
    pub peck: String,
}

/// The Add Tool dialog.
pub struct ToolDialog {
    pub kind: &'static str,
    pub diameter: String,
    pub feed: String,
    pub rpm: String,
}

/// A number for a text field: at most 3 decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.3}", (v * 1000.0).round() / 1000.0);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

const KINDS: [(&str, &str, Icon); 4] = [
    ("facing", "Facing", Icon::CamFacing),
    ("contour", "2D Contour", Icon::CamContour),
    ("pocket", "2D Pocket", Icon::CamPocket),
    ("drill", "Drill", Icon::CamDrill),
];

fn kind_icon(kind: &str) -> Icon {
    KINDS
        .iter()
        .find(|k| k.0 == kind)
        .map_or(Icon::CamGenerate, |k| k.2)
}

fn kind_title(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|k| k.0 == kind)
        .map_or("Operation", |k| k.1)
}

impl OpDialog {
    pub fn new(kind: &'static str, tool: Option<u32>) -> OpDialog {
        OpDialog {
            kind,
            edit: None,
            tool,
            face: None,
            edges: Vec::new(),
            pick_edges: false,
            stepover: match kind {
                "facing" => "50",
                _ => "40",
            }
            .into(),
            stepdown: match kind {
                "facing" => "1 mm",
                _ => "2 mm",
            }
            .into(),
            outside: true,
            entry: "helix",
            peck: String::new(),
        }
    }

    /// Whether viewport clicks pick something for this operation.
    fn picks(&self) -> bool {
        matches!(self.kind, "contour" | "pocket")
    }

    fn ready(&self) -> bool {
        self.tool.is_some()
            && match self.kind {
                "pocket" => self.face.is_some(),
                "contour" => self.face.is_some() || !self.edges.is_empty(),
                _ => true,
            }
    }

    /// The command parameters.
    fn params(&self) -> Value {
        let mut p = json!({});
        match self.edit {
            Some(id) => p["operation"] = json!(id),
            None => p["kind"] = json!(self.kind),
        }
        if let Some(t) = self.tool {
            p["tool"] = json!(t);
        }
        if let Ok(v) = self
            .stepover
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            && matches!(self.kind, "facing" | "pocket")
        {
            p["stepover"] = json!(v);
        }
        if self.kind != "drill" && !self.stepdown.trim().is_empty() {
            p["stepdown"] = json!(self.stepdown.trim());
        }
        match self.kind {
            "contour" => {
                if self.pick_edges {
                    p["edges"] = json!(self.edges);
                } else if let Some(f) = &self.face {
                    p["face"] = json!(f);
                }
                p["side"] = json!(if self.outside { "outside" } else { "inside" });
            }
            "pocket" => {
                if let Some(f) = &self.face {
                    p["face"] = json!(f);
                }
                p["entry"] = json!(self.entry);
            }
            "drill" => {
                if !self.peck.trim().is_empty() {
                    p["peck"] = json!(self.peck.trim());
                } else if self.edit.is_some() {
                    p["peck"] = Value::Null;
                }
            }
            _ => {}
        }
        p
    }
}

impl ChamferaApp {
    /// Opens or closes the Manufacture workspace (toolpaths show only there).
    pub(crate) fn set_manufacture(&mut self, on: bool) {
        self.cam.active = on;
        if !on {
            self.cam.dialog = None;
            self.cam.tool_dialog = None;
        }
        self.run("cam.show", json!({ "visible": on }));
    }

    /// The last tool of the library, for a new operation.
    fn last_tool(&self) -> Option<u32> {
        self.session.doc.cam.tools.last().map(|t| t.id)
    }

    /// Setup: stock from the model's box, part zero at the stock's top corner.
    pub(crate) fn cam_setup(&mut self) {
        if let Some(r) = self.run("cam.setup", json!({})) {
            self.message = Some((
                format!("Added {}", r["name"].as_str().unwrap_or("a setup")),
                false,
            ));
        }
    }

    /// Add Tool: opens its dialog with a 6 mm flat end mill filled in.
    pub(crate) fn open_cam_tool_dialog(&mut self) {
        self.cam.tool_dialog = Some(ToolDialog {
            kind: "flat_end_mill",
            diameter: "6 mm".into(),
            feed: "600".into(),
            rpm: "10000".into(),
        });
    }

    /// Opens the dialog for a new operation of this kind (`facing`, `contour`, `pocket`,
    /// `drill`), with the last tool of the library.
    pub(crate) fn open_cam_op(&mut self, kind: &'static str) {
        self.cam.dialog = Some(OpDialog::new(kind, self.last_tool()));
    }

    pub(crate) fn generate_toolpaths(&mut self) {
        if let Some(r) = self.run("cam.generate", json!({})) {
            let errors = r["errors"].as_u64().unwrap_or(0);
            let minutes = r["minutes"].as_f64().unwrap_or(0.0);
            self.message = Some(if errors == 0 {
                (
                    format!("Toolpaths generated · cycle time {minutes:.1} min"),
                    false,
                )
            } else {
                (
                    format!("{errors} operation(s) could not be generated: see the tree"),
                    true,
                )
            });
        }
    }

    /// File > Post G-code: asks where, then runs `cam.post`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn post_dialog(&mut self, dialect: &str) {
        let stem = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "design".to_owned(), |s| s.to_string_lossy().into_owned());
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Post G-code")
            .add_filter("G-code", &["nc", "ngc", "gcode"])
            .set_file_name(format!("{stem}.nc"))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("nc");
        }
        if let Some(r) = self.run(
            "cam.post",
            json!({ "path": path.to_string_lossy(), "dialect": dialect }),
        ) {
            self.message = Some((
                format!("Posted {} lines to {}", r["lines"], path.display()),
                false,
            ));
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn post_dialog(&mut self, _dialect: &str) {
        self.message = Some(("Posting G-code is in the desktop app for now".into(), true));
    }

    /// The left panel in Manufacture: setups with their operations, then the tools.
    pub(crate) fn cam_tree(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            widgets::caption(ui, look, "MANUFACTURE");
        });
        ui.add_space(widgets::S2);
        let cam = self.session.doc.cam.clone();
        let mut open: Option<u32> = None;
        let mut toggle: Option<(u32, bool)> = None;
        let mut delete: Option<u32> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if cam.setups.is_empty() {
                    ui.label(
                        egui::RichText::new("No setup yet: click Setup in the ribbon")
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                }
                for st in &cam.setups {
                    widgets::menu_item(ui, look, Icon::CamSetup, &st.name, true);
                    for o in cam.operations.iter().filter(|o| o.setup == st.id) {
                        let status = match self.session.toolpath(o.id) {
                            Some(Ok(tp)) => {
                                format!("{:.1} min", tp.stats(st.rapid_rate).minutes)
                            }
                            Some(Err(e)) if o.suppressed => {
                                let _ = e;
                                "off".to_owned()
                            }
                            Some(Err(_)) => "error".to_owned(),
                            None => String::new(),
                        };
                        let tip = match self.session.toolpath(o.id) {
                            Some(Err(e)) => e.clone(),
                            _ => format!("{} · {status}", kind_title(o.op.key())),
                        };
                        let colour = if status == "error" {
                            look.c(|t| t.error)
                        } else {
                            look.c(|t| t.text_muted)
                        };
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::icon_button(
                                        ui,
                                        look,
                                        Icon::Delete,
                                        22.0,
                                        14.0,
                                        true,
                                        false,
                                    )
                                    .on_hover_text(format!(
                                        "{}\nRemove it from the setup",
                                        label("cam.op_delete")
                                    ))
                                    .clicked()
                                    {
                                        delete = Some(o.id);
                                    }
                                    let eye = if o.suppressed {
                                        Icon::EyeOff
                                    } else {
                                        Icon::Eye
                                    };
                                    if widgets::icon_button(ui, look, eye, 22.0, 14.0, true, false)
                                        .on_hover_text(if o.suppressed {
                                            "Restore\nGenerate and post this operation again"
                                        } else {
                                            "Suppress\nSkip this operation when generating and posting"
                                        })
                                        .clicked()
                                    {
                                        toggle = Some((o.id, !o.suppressed));
                                    }
                                    ui.label(
                                        egui::RichText::new(&status)
                                            .size(widgets::T_SMALL)
                                            .color(colour),
                                    );
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            let r = widgets::menu_item(
                                                ui,
                                                look,
                                                kind_icon(o.op.key()),
                                                &o.name,
                                                !o.suppressed,
                                            );
                                            if r.on_hover_text(tip).clicked() {
                                                open = Some(o.id);
                                            }
                                        },
                                    );
                                },
                            );
                        });
                    }
                }
                ui.add_space(widgets::S3);
                widgets::caption(ui, look, "TOOLS");
                for t in &cam.tools {
                    widgets::menu_item(
                        ui,
                        look,
                        Icon::CamTool,
                        &format!("T{} {}", t.number, t.name),
                        true,
                    );
                }
            });
        if let Some(id) = open {
            self.edit_operation(id);
        }
        if let Some((id, s)) = toggle {
            self.run("cam.op_edit", json!({ "operation": id, "suppressed": s }));
        }
        if let Some(id) = delete {
            self.run("cam.op_delete", json!({ "operation": id }));
        }
    }

    /// Opens the dialog on an existing operation.
    fn edit_operation(&mut self, id: u32) {
        use chamfera_doc::cam::{Entry, OpKind, Side};
        let Some(o) = self.session.doc.cam.operation(id).cloned() else {
            return;
        };
        let kind = KINDS
            .iter()
            .find(|k| k.0 == o.op.key())
            .map_or("facing", |k| k.0);
        let mut d = OpDialog::new(kind, Some(o.tool));
        d.edit = Some(id);
        let pct = |v: f64| num(v * 100.0);
        let mm = |v: f64| format!("{} mm", num(v));
        match o.op {
            OpKind::Facing { stepover, stepdown } => {
                d.stepover = pct(stepover);
                d.stepdown = mm(stepdown);
            }
            OpKind::Contour {
                face,
                edges,
                side,
                stepdown,
                ..
            } => {
                d.pick_edges = face.is_none();
                d.face = face;
                d.edges = edges;
                d.outside = side == Side::Right;
                d.stepdown = mm(stepdown);
            }
            OpKind::Pocket {
                face,
                stepover,
                stepdown,
                entry,
                ..
            } => {
                d.face = Some(face);
                d.stepover = pct(stepover);
                d.stepdown = mm(stepdown);
                d.entry = match entry {
                    Entry::Helix => "helix",
                    Entry::Ramp => "ramp",
                    Entry::Plunge => "plunge",
                };
            }
            OpKind::Drill { peck, .. } => {
                d.peck = peck.map(mm).unwrap_or_default();
            }
        }
        self.cam.dialog = Some(d);
    }

    /// True when a viewport click goes to the operation dialog.
    pub(crate) fn cam_picking(&self) -> bool {
        self.cam.active && self.cam.dialog.as_ref().is_some_and(OpDialog::picks)
    }

    /// A viewport click while the dialog picks: a face (its name), or for a contour in
    /// edge mode an edge (click again to unpick).
    pub(crate) fn cam_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let edge_mode = self.cam.dialog.as_ref().is_some_and(|d| d.pick_edges);
        if edge_mode {
            let Some((_, _, name)) = self.session.edge_at(x, y, w, h) else {
                return;
            };
            if let Some(d) = self.cam.dialog.as_mut() {
                if let Some(i) = d.edges.iter().position(|e| *e == name) {
                    d.edges.remove(i);
                } else {
                    d.edges.push(name);
                }
            }
            return;
        }
        let Some((feature, role, _)) = self.session.face_at(x, y, w, h) else {
            self.message = Some(("Click a flat face of the model".into(), true));
            return;
        };
        let name = chamfera_engine::solids::face_key(feature, &role);
        if let Some(d) = self.cam.dialog.as_mut() {
            d.face = Some(name);
        }
    }

    /// The operation dialog and the Add Tool dialog.
    pub(crate) fn cam_dialogs(&mut self, ctx: &egui::Context) {
        if !self.cam.active {
            return;
        }
        self.tool_window(ctx);
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let tools: Vec<(u32, String)> = self
            .session
            .doc
            .cam
            .tools
            .iter()
            .map(|t| (t.id, format!("T{} {}", t.number, t.name)))
            .collect();
        let Some(d) = self.cam.dialog.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = kind_title(d.kind);
        let ready = d.ready();
        egui::Window::new(title)
            .id(egui::Id::unique("cam operation dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, kind_icon(d.kind), title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("cam op grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Tool");
                            let current = tools
                                .iter()
                                .find(|t| Some(t.0) == d.tool)
                                .map_or("Pick a tool", |t| t.1.as_str());
                            egui::ComboBox::from_id_salt("cam tool")
                                .selected_text(current)
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for (id, text) in &tools {
                                        ui.selectable_value(&mut d.tool, Some(*id), text);
                                    }
                                });
                            ui.end_row();
                            if d.kind == "contour" {
                                widgets::field_label(ui, look, "Follow");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.pick_edges, false, "Face");
                                    ui.radio_value(&mut d.pick_edges, true, "Edges");
                                });
                                ui.end_row();
                            }
                            if d.picks() {
                                let edges = d.kind == "contour" && d.pick_edges;
                                widgets::field_label(
                                    ui,
                                    look,
                                    if edges { "Edges" } else { "Face" },
                                );
                                let text = if edges {
                                    match d.edges.len() {
                                        0 => "Click edges".to_owned(),
                                        n => format!("{n} selected"),
                                    }
                                } else {
                                    d.face.clone().unwrap_or_else(|| {
                                        if d.kind == "pocket" {
                                            "Click the pocket floor".into()
                                        } else {
                                            "Click a flat face".into()
                                        }
                                    })
                                };
                                ui.label(text);
                                ui.end_row();
                            }
                            if d.kind == "contour" {
                                widgets::field_label(ui, look, "Side");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut d.outside, true, "Outside");
                                    ui.radio_value(&mut d.outside, false, "Inside");
                                });
                                ui.end_row();
                            }
                            if matches!(d.kind, "facing" | "pocket") {
                                widgets::field_label(ui, look, "Stepover %");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.stepover),
                                );
                                ui.end_row();
                            }
                            if d.kind != "drill" {
                                widgets::field_label(ui, look, "Stepdown");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.stepdown),
                                );
                                ui.end_row();
                            }
                            if d.kind == "pocket" {
                                widgets::field_label(ui, look, "Entry");
                                ui.horizontal(|ui| {
                                    for e in ["helix", "ramp", "plunge"] {
                                        ui.radio_value(&mut d.entry, e, e);
                                    }
                                });
                                ui.end_row();
                            }
                            if d.kind == "drill" {
                                widgets::field_label(ui, look, "Peck");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut d.peck).hint_text("none (G81)"),
                                );
                                ui.end_row();
                            }
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
            let (id, params) = match d.edit {
                Some(_) => ("cam.op_edit", d.params()),
                None => ("cam.op_add", d.params()),
            };
            if let Some(r) = self.run(id, params) {
                self.message = Some(match r.get("error").and_then(Value::as_str) {
                    Some(e) => (
                        format!("{}: {e}", r["name"].as_str().unwrap_or(title)),
                        true,
                    ),
                    None => (
                        format!(
                            "{} · {:.1} min",
                            r["name"].as_str().unwrap_or(title),
                            r["minutes"].as_f64().unwrap_or(0.0)
                        ),
                        false,
                    ),
                });
                self.cam.dialog = None;
            }
        }
        if cancel {
            self.cam.dialog = None;
        }
    }

    fn tool_window(&mut self, ctx: &egui::Context) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let pos = self.tool_dialog_pos(300.0);
        let Some(t) = self.cam.tool_dialog.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Add Tool")
            .id(egui::Id::unique("cam tool dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(300.0)
            .resizable(false)
            .show(ctx, |ui| {
                if widgets::dialog_title(ui, look, Icon::CamTool, label("cam.tool_add")) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("cam tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Kind");
                            egui::ComboBox::from_id_salt("cam tool kind")
                                .selected_text(t.kind.replace('_', " "))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for k in
                                        ["flat_end_mill", "ball_end_mill", "drill", "chamfer_mill"]
                                    {
                                        ui.selectable_value(&mut t.kind, k, k.replace('_', " "));
                                    }
                                });
                            ui.end_row();
                            for (text, field) in [
                                ("Diameter", &mut t.diameter),
                                ("Feed mm/min", &mut t.feed),
                                ("Spindle rpm", &mut t.rpm),
                            ] {
                                widgets::field_label(ui, look, text);
                                ui.add_sized(widgets::FIELD, egui::TextEdit::singleline(field));
                                ui.end_row();
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    ok = widgets::primary_button(ui, look, "OK", true).clicked();
                    cancel |= widgets::secondary_button(ui, look, "Cancel").clicked();
                });
            });
        if ok {
            let mut p = json!({ "kind": t.kind, "diameter": t.diameter.trim() });
            if let Ok(f) = t.feed.trim().parse::<f64>() {
                p["feed"] = json!(f);
            }
            if let Ok(r) = t.rpm.trim().parse::<f64>() {
                p["spindle_rpm"] = json!(r);
            }
            if let Some(r) = self.run("cam.tool_add", p) {
                self.message = Some((
                    format!(
                        "Added T{} {}",
                        r["number"],
                        r["name"].as_str().unwrap_or("")
                    ),
                    false,
                ));
                self.cam.tool_dialog = None;
            }
        }
        if cancel {
            self.cam.tool_dialog = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pocket_dialog_adds_a_pocket_on_the_picked_floor() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 60, "y2": 40 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": 10 }),
        );
        app.run(
            "sketch.create_on_face",
            json!({ "feature": 2, "face": "top" }),
        );
        app.run(
            "sketch.rectangle",
            json!({ "x1": 10, "y1": 10, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 3, "profiles": [{ "x": 30, "y": 20 }], "distance": 4,
                    "direction": "reversed", "operation": "cut" }),
        );
        app.set_manufacture(true);
        app.run(
            "cam.tool_add",
            json!({ "kind": "flat_end_mill", "diameter": 6 }),
        );
        app.cam.dialog = Some(OpDialog::new("pocket", app.last_tool()));
        // Look straight down: the pocket floor (z = 6) fills the middle of the view.
        let (w, h) = (800.0, 600.0);
        app.run("view.standard", json!({ "view": "top" }));
        app.run("view.fit", json!({ "width": w, "height": h }));
        assert!(app.cam_picking());
        app.cam_click(w / 2.0, h / 2.0, w, h);
        let d = app.cam.dialog.as_ref().unwrap();
        assert_eq!(d.face.as_deref(), Some("4:top"));
        let r = app.run("cam.op_add", d.params()).unwrap();
        assert!(r.get("error").is_none(), "{r}");
        // The toolpath shows as overlays in both styles.
        let styles: Vec<_> = app.session.scene.overlays.iter().map(|o| o.style).collect();
        assert!(styles.contains(&chamfera_render::OverlayStyle::ToolpathFeed));
        assert!(styles.contains(&chamfera_render::OverlayStyle::ToolpathRapid));
        // Leaving Manufacture hides them.
        app.set_manufacture(false);
        assert!(
            !app.session
                .scene
                .overlays
                .iter()
                .any(|o| o.style == chamfera_render::OverlayStyle::ToolpathFeed)
        );
    }
}

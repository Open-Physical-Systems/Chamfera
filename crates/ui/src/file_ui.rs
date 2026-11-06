//! The File menu: New, Open, Save, Save As, Export, Insert, and the question before
//! unsaved changes are thrown away. Keyboard shortcuts for files and undo live here too.
//!
//! Export writes STEP, STL, 3MF, OBJ or glTF binary; Insert Mesh takes STL, OBJ or 3MF;
//! Insert DXF draws a DXF file into the sketch being edited.
//!
//! The desktop uses file dialogs and paths. In the browser (wasm32) Open and the Insert
//! items use the browser's file picker, Save and Export download the file, and the design is
//! kept in local storage (`crate::web`); there is no Save As or Quit.

use crate::icons::Icon;
use crate::widgets;
use crate::{ChamferaApp, label};
use serde_json::json;

/// The mesh exports after STEP and STL: what the dialog calls the file, its extensions,
/// and the command.
pub(crate) const MESH_EXPORTS: [(&str, &[&str], &str); 3] = [
    ("3MF", &["3mf"], "design.export_3mf"),
    ("OBJ mesh", &["obj"], "design.export_obj"),
    ("glTF binary", &["glb"], "design.export_glb"),
];

/// The command that inserts a mesh file, by its name's extension (STL when unknown).
pub(crate) fn mesh_import_command(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    if ext.eq_ignore_ascii_case("obj") {
        "body.import_obj"
    } else if ext.eq_ignore_ascii_case("3mf") {
        "body.import_3mf"
    } else {
        "body.import_stl"
    }
}

/// Something that would discard unsaved changes, waiting for the user's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    New,
    Open,
    /// Open this design from File > Open Recent.
    /// (Desktop only: the browser has no paths.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    OpenPath(std::path::PathBuf),
    Quit,
}

#[derive(Default)]
pub struct FileUi {
    pub pending: Option<Pending>,
    /// The window title last sent, so it is only sent when it changes.
    title: String,
    /// Closing was confirmed (or nothing was unsaved): let the window close.
    pub allow_close: bool,
}

impl ChamferaApp {
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        // Where each item is, for tests in the browser (`crate::web`).
        let mut drawn: Vec<(String, egui::Rect)> = Vec::new();
        let mut item_if = |ui: &mut egui::Ui, icon: Icon, text: &str, keys: &str, on: bool| {
            let r = widgets::menu_item(ui, look, icon, text, on);
            drawn.push((text.to_owned(), r.rect));
            if !keys.is_empty() {
                ui.painter().text(
                    egui::pos2(r.rect.max.x - 8.0, r.rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    keys,
                    egui::FontId::proportional(widgets::T_SMALL),
                    look.c(|t| t.text_muted),
                );
            }
            r.clicked()
        };
        ui.set_min_width(240.0);
        let new = item_if(ui, Icon::New, "New", "Ctrl+N", true);
        let open = item_if(ui, Icon::Open, "Open…", "Ctrl+O", true);
        ui.separator();
        let save = item_if(ui, Icon::Save, "Save", "Ctrl+S", true);
        #[cfg(not(target_arch = "wasm32"))]
        let mut recent_pick: Option<std::path::PathBuf> = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let recent = self.prefs.recent_shown().to_vec();
            if !recent.is_empty() {
                ui.menu_button("Open Recent", |ui| {
                    for r in &recent {
                        let name = std::path::Path::new(r)
                            .file_name()
                            .map_or_else(|| r.clone(), |n| n.to_string_lossy().into_owned());
                        if ui.button(name).on_hover_text(r).clicked() {
                            recent_pick = Some(std::path::PathBuf::from(r));
                            ui.close();
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let save_as = item_if(ui, Icon::Save, "Save As…", "Ctrl+Shift+S", true);
        #[cfg(not(target_arch = "wasm32"))]
        let save_version = item_if(
            ui,
            Icon::Save,
            &format!("{}…", label("version.save")),
            "Ctrl+Alt+S",
            true,
        );
        #[cfg(not(target_arch = "wasm32"))]
        let history = item_if(
            ui,
            Icon::Undo,
            &format!("{}…", label("version.list")),
            "",
            true,
        );
        ui.separator();
        let step = item_if(
            ui,
            Icon::Export,
            &format!("{}…", label("design.export_step")),
            "",
            true,
        );
        let stl = item_if(
            ui,
            Icon::Export,
            &format!("{}…", label("design.export_stl")),
            "",
            true,
        );
        let mut mesh_export = None;
        for (i, (_, _, command)) in MESH_EXPORTS.iter().enumerate() {
            if item_if(ui, Icon::Export, &format!("{}…", label(command)), "", true) {
                mesh_export = Some(i);
            }
        }
        let post_linuxcnc = item_if(ui, Icon::CamPost, "Post G-code (LinuxCNC)…", "", true);
        let post_grbl = item_if(ui, Icon::CamPost, "Post G-code (GRBL)…", "", true);
        // While the DRAWING tab is open: the sheet's exports (`drawing_ui.rs`).
        let drawing_export = self.drawing_export_items(ui, look);
        ui.separator();
        let import = item_if(
            ui,
            Icon::ImportMesh,
            "Insert Mesh (STL, OBJ, 3MF)…",
            "",
            true,
        );
        let step_in = item_if(ui, Icon::ImportMesh, "Insert STEP…", "", true);
        let sketching = self.session.active_sketch.is_some();
        let dxf_text = format!("{} into Sketch…", label("sketch.import_dxf"));
        let dxf = item_if(ui, Icon::ImportMesh, &dxf_text, "", sketching);
        let dxf_out_text = format!("{}…", label("sketch.export_dxf"));
        let dxf_out = item_if(ui, Icon::Export, &dxf_out_text, "", sketching);
        let sample = item_if(ui, Icon::Body, label("body.sample"), "", true);
        ui.separator();
        let preferences = item_if(ui, Icon::Parameters, "Preferences…", "Ctrl+,", true);
        #[cfg(not(target_arch = "wasm32"))]
        let quit = {
            ui.separator();
            item_if(ui, Icon::Close, "Quit", "", true)
        };
        #[cfg(target_arch = "wasm32")]
        for (text, rect) in &drawn {
            self.web.target(text, *rect);
        }
        #[cfg(not(target_arch = "wasm32"))]
        drop(drawn);
        if new {
            ui.close();
            self.ask_or(Pending::New);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = recent_pick {
            ui.close();
            self.ask_or(Pending::OpenPath(path));
        }
        if preferences {
            ui.close();
            self.prefs_ui.open = true;
        }
        if open {
            ui.close();
            self.ask_or(Pending::Open);
        }
        if save {
            ui.close();
            self.save(false);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if save_as {
            ui.close();
            self.save(true);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if save_version {
            ui.close();
            self.ask_version_note();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if history {
            ui.close();
            self.open_history();
        }
        if step {
            ui.close();
            self.export("STEP", &["step", "stp"], "design.export_step");
        }
        if stl {
            ui.close();
            self.export("STL mesh", &["stl"], "design.export_stl");
        }
        if let Some((kind, extensions, command)) = mesh_export.and_then(|i| MESH_EXPORTS.get(i)) {
            ui.close();
            self.export(kind, extensions, command);
        }
        if post_linuxcnc || post_grbl {
            ui.close();
            self.post_dialog(if post_grbl { "grbl" } else { "linuxcnc" });
        }
        if let Some(f) = drawing_export {
            ui.close();
            self.export_drawing(f);
        }
        if import {
            ui.close();
            self.import_dialog();
        }
        if step_in {
            ui.close();
            self.step_dialog();
        }
        if dxf {
            ui.close();
            self.dxf_dialog();
        }
        if dxf_out {
            ui.close();
            self.export("DXF drawing", &["dxf"], "sketch.export_dxf");
        }
        if sample {
            ui.close();
            self.run("body.sample", json!({}));
            self.fit_pending = true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if quit {
            ui.close();
            self.ask_or(Pending::Quit);
        }
    }

    /// Does `action` now, or first asks when there are unsaved changes.
    pub(crate) fn ask_or(&mut self, action: Pending) {
        if self.session.has_unsaved_changes() {
            self.file.pending = Some(action);
        } else {
            self.do_pending(action);
        }
    }

    fn do_pending(&mut self, action: Pending) {
        match action {
            Pending::New => {
                let length = self.prefs.default_length.symbol();
                self.run("design.new", json!({ "length": length }));
                self.after_new_design();
                #[cfg(target_arch = "wasm32")]
                self.web.clear();
            }
            Pending::Open => self.open_dialog(),
            #[cfg(not(target_arch = "wasm32"))]
            Pending::OpenPath(path) => self.open_path(&path),
            #[cfg(target_arch = "wasm32")]
            Pending::OpenPath(_) => {}
            Pending::Quit => {
                // Quitting on purpose (changes saved or let go): no recovery file stays.
                self.recovery_remove_own();
                self.file.allow_close = true;
                self.ctx_close = true;
            }
        }
    }

    fn after_new_design(&mut self) {
        self.sketch = crate::sketch_ui::SketchUi::default();
        self.solid = crate::solid_ui::SolidUi::default();
        self.fit_pending = true;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open Design")
            .add_filter("Chamfera design", &["chamfera"])
            .pick_file()
        else {
            return;
        };
        self.open_path(&path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_path(&mut self, path: &std::path::Path) {
        if self
            .run("design.open", json!({ "path": path.to_string_lossy() }))
            .is_some()
        {
            self.after_new_design();
            self.message = Some((format!("Opened {}", path.display()), false));
        }
    }

    /// Remembers the design's file in the recent list once it is opened or saved.
    pub(crate) fn track_recent(&mut self) {
        if self.session.path == self.recent_seen {
            return;
        }
        self.recent_seen = self.session.path.clone();
        if let Some(p) = &self.recent_seen {
            let text = p.to_string_lossy().into_owned();
            self.prefs.add_recent(&text);
            self.preferences_changed();
        }
    }

    /// The browser's file picker; the design is opened in `browser_files` once read.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self) {
        self.web.pick(crate::web::Kind::Design, ".chamfera", false);
    }

    /// The browser's file picker for meshes, inserted in `browser_files` once read.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn import_dialog(&mut self) {
        self.web.pick(
            crate::web::Kind::Mesh,
            ".stl,.STL,.obj,.OBJ,.3mf,.3MF",
            true,
        );
    }

    /// The browser's file picker for a DXF drawing, drawn into the active sketch once read.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn dxf_dialog(&mut self) {
        self.web.pick(crate::web::Kind::Dxf, ".dxf,.DXF", false);
    }

    /// Asks for a DXF file and draws it into the sketch being edited.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn dxf_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(label("sketch.import_dxf"))
            .add_filter("DXF drawing", &["dxf", "DXF"])
            .pick_file()
        else {
            return;
        };
        let reply = self.run(
            "sketch.import_dxf",
            json!({ "path": path.to_string_lossy() }),
        );
        self.after_dxf(reply.as_ref());
    }

    /// STEP files are read from a path, which the browser build has none of.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn step_dialog(&mut self) {
        self.message = Some((
            "Insert STEP needs the desktop app: the browser build cannot read STEP files yet"
                .to_owned(),
            true,
        ));
    }

    /// Asks for a STEP file and adds its solids as exact bodies.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn step_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(label("body.import_step"))
            .add_filter("STEP model", &["step", "stp", "STEP", "STP"])
            .pick_file()
        else {
            return;
        };
        let reply = self.run(
            "body.import_step",
            json!({ "path": path.to_string_lossy() }),
        );
        self.after_step(reply.as_ref());
    }

    /// Reports a STEP import: how many solids came in and how many were left out.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn after_step(&mut self, reply: Option<&serde_json::Value>) {
        let Some(v) = reply else {
            return;
        };
        let solids = v["solids"].as_array().map_or(0, Vec::len);
        let skipped = v["skipped"].as_array().map_or(0, Vec::len);
        let mut text = format!(
            "Inserted {solids} STEP solid{}",
            if solids == 1 { "" } else { "s" }
        );
        if skipped > 0 {
            text.push_str(&format!("; {skipped} left out (not supported yet)"));
        }
        self.message = Some((text, skipped > 0 && solids == 0));
        self.fit_pending = true;
    }

    /// Reports a DXF import.
    pub(crate) fn after_dxf(&mut self, reply: Option<&serde_json::Value>) {
        let Some(v) = reply else {
            return;
        };
        let skipped: u64 = v["skipped"].as_object().map_or(0, |m| {
            m.values().filter_map(serde_json::Value::as_u64).sum()
        });
        let mut text = format!(
            "Inserted {} curves ({}), {} profiles",
            v["curves"],
            v["units"].as_str().unwrap_or("mm"),
            v["profiles"].as_array().map_or(0, Vec::len)
        );
        if skipped > 0 {
            text.push_str(&format!("; {skipped} entities left out"));
        }
        self.message = Some((text, false));
        self.refresh_sketch_status();
    }

    /// Opens the design this browser kept (local storage), if any, at start.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn restore_from_browser(&mut self) {
        let Some((text, name)) = crate::web::WebFiles::stored_design() else {
            return;
        };
        let Some(data) = crate::web::to_base64(text.as_bytes()) else {
            return;
        };
        if self
            .run("design.open_bytes", json!({ "data": data }))
            .is_some()
        {
            self.session.path = name.map(std::path::PathBuf::from);
            self.web.restored(&self.session.doc);
            self.after_new_design();
            self.message = Some((
                format!("Restored {} from this browser", self.design_name()),
                false,
            ));
        }
    }

    /// Every frame in the browser: opens or inserts the files the picker has read, and
    /// keeps the design in local storage.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_files(&mut self, ctx: &egui::Context) {
        self.web.set_context(ctx);
        for picked in self.web.take_picked() {
            let data = match picked.data {
                Ok(d) => d,
                Err(e) => {
                    self.message = Some((e, true));
                    continue;
                }
            };
            match picked.kind {
                crate::web::Kind::Design => {
                    if self
                        .run("design.open_bytes", json!({ "data": data }))
                        .is_some()
                    {
                        // Only the name, for the title and downloads: there is no file path.
                        self.session.path = Some(std::path::PathBuf::from(&picked.name));
                        self.after_new_design();
                        self.message = Some((format!("Opened {}", picked.name), false));
                    }
                }
                crate::web::Kind::Dxf => {
                    let reply = self.run("sketch.import_dxf_bytes", json!({ "data": data }));
                    self.after_dxf(reply.as_ref());
                }
                crate::web::Kind::Mesh => {
                    let name = picked.name.as_str();
                    let command = format!("{}_bytes", mesh_import_command(name));
                    if let Some(v) = self.run(&command, json!({ "data": data, "name": name })) {
                        self.message = Some((
                            format!(
                                "Inserted {} ({} triangles)",
                                v["name"].as_str().unwrap_or("mesh"),
                                v["triangles"]
                            ),
                            false,
                        ));
                        self.fit_pending = true;
                    }
                }
            }
        }
        let name = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(problem) = self.web.autosave(ctx, &self.session.doc, name.as_deref()) {
            self.message = Some((problem, true));
        }
        let message = self.message.as_ref().map_or("", |(m, _)| m.as_str());
        self.web
            .show_status(message, self.session.scene.bodies.len());
        self.web.show_targets(ctx.zoom_factor());
    }

    /// Saves to the current file, or asks where (always, with `choose`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn save(&mut self, choose: bool) {
        let path = match (&self.session.path, choose) {
            (Some(p), false) => p.clone(),
            _ => {
                let Some(mut p) = rfd::FileDialog::new()
                    .set_title("Save Design")
                    .add_filter("Chamfera design", &["chamfera"])
                    .set_file_name("design.chamfera")
                    .save_file()
                else {
                    return;
                };
                if p.extension().is_none() {
                    p.set_extension("chamfera");
                }
                p
            }
        };
        if let Some(reply) = self.run("design.save", json!({ "path": path.to_string_lossy() })) {
            self.message = Some((format!("Saved {}", path.display()), false));
            if let Some(e) = reply["history_error"].as_str() {
                self.message = Some((
                    format!("Saved {}; no version kept: {e}", path.display()),
                    true,
                ));
            }
            self.recovery_remove_own();
            self.after_version_saved(&reply);
        }
    }

    /// Downloads the design (the browser has no Save As: it asks where, or not, itself).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save(&mut self, _choose: bool) {
        let Some(v) = self.run("design.save_bytes", json!({})) else {
            return;
        };
        let name = format!("{}.chamfera", self.design_name());
        match crate::web::download(
            &name,
            "application/octet-stream",
            v["data"].as_str().unwrap_or(""),
        ) {
            Ok(()) => self.message = Some((format!("Downloaded {name}"), false)),
            Err(e) => self.message = Some((format!("Save: {e}"), true)),
        }
    }

    /// Downloads a STEP or STL export.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn export(&mut self, _kind: &str, extensions: &[&str], command: &str) {
        let ext = extensions.first().copied().unwrap_or("out");
        let format = command
            .strip_prefix("design.export_")
            .or_else(|| command.strip_prefix("sketch.export_"))
            .unwrap_or("stl");
        let reply = if command == "sketch.export_dxf" {
            self.run("sketch.export_dxf_bytes", json!({}))
        } else {
            let mut params = json!({ "format": format });
            if format == "step" {
                params["timestamp"] = json!(crate::web::now_iso8601());
            }
            self.run("design.export_bytes", params)
        };
        let Some(v) = reply else {
            return;
        };
        let name = format!("{}.{ext}", self.design_name());
        let mime = match format {
            "step" => "model/step",
            "3mf" => "model/3mf",
            "obj" => "model/obj",
            "glb" => "model/gltf-binary",
            "dxf" => "image/vnd.dxf",
            _ => "model/stl",
        };
        match crate::web::download(&name, mime, v["data"].as_str().unwrap_or("")) {
            Ok(()) => self.message = Some((format!("Downloaded {name}"), false)),
            Err(e) => self.message = Some((format!("{}: {e}", label(command)), true)),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn export(&mut self, kind: &str, extensions: &[&str], command: &str) {
        let stem = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "design".to_owned(), |s| s.to_string_lossy().into_owned());
        let ext = extensions.first().copied().unwrap_or("out");
        let Some(mut path) = rfd::FileDialog::new()
            .set_title(format!("Export {kind}"))
            .add_filter(kind, extensions)
            .set_file_name(format!("{stem}.{ext}"))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension(ext);
        }
        if self
            .run(command, json!({ "path": path.to_string_lossy() }))
            .is_some()
        {
            self.message = Some((format!("Exported {}", path.display()), false));
        }
    }

    /// The "unsaved changes" question.
    pub(crate) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.file.pending.clone() else {
            return;
        };
        let mut answer: Option<&str> = None;
        let mut buttons = [egui::Rect::NOTHING; 3];
        let look = self.look();
        let name = self.design_name();
        let what = match action {
            Pending::New => "starting a new design",
            Pending::Open | Pending::OpenPath(_) => "opening another design",
            Pending::Quit => "quitting",
        };
        let modal = egui::Modal::new(egui::Id::unique("unsaved changes"))
            .frame(widgets::dialog_frame(look))
            .backdrop_color(look.c(|t| t.shadow))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                if widgets::dialog_title(ui, look, Icon::Warning, "Unsaved changes") {
                    answer = Some("cancel");
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(format!(
                        "{name} has changes that are not saved. Save them before {what}?"
                    ));
                });
                widgets::dialog_footer(ui, look, |ui| {
                    let save = widgets::primary_button(ui, look, "Save", true);
                    let cancel = widgets::secondary_button(ui, look, "Cancel");
                    let discard = widgets::secondary_button(ui, look, "Don't Save");
                    if save.clicked() {
                        answer = Some("save");
                    }
                    if cancel.clicked() {
                        answer = Some("cancel");
                    }
                    if discard.clicked() {
                        answer = Some("discard");
                    }
                    buttons = [save.rect, cancel.rect, discard.rect];
                });
            });
        // The buttons by text, for the browser checks (`web.rs`).
        #[cfg(target_arch = "wasm32")]
        for (text, rect) in ["Save", "Cancel", "Don't Save"].into_iter().zip(buttons) {
            self.web.target(text, rect);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = buttons;
        if modal.should_close() && answer.is_none() {
            answer = Some("cancel");
        }
        match answer {
            Some("save") => {
                self.save(false);
                if !self.session.has_unsaved_changes() {
                    self.file.pending = None;
                    self.do_pending(action);
                }
            }
            Some("discard") => {
                self.file.pending = None;
                self.do_pending(action);
            }
            Some(_) => self.file.pending = None,
            None => {}
        }
    }

    /// Keyboard shortcuts for files and undo, when no text box has the keyboard.
    pub(crate) fn file_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let pressed = |m: Modifiers, k: Key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        // Longer shortcuts first, so Ctrl+Shift+S isn't also read as Ctrl+S.
        if pressed(Modifiers::COMMAND | Modifiers::ALT, Key::S) {
            self.ask_version_note();
        } else if pressed(cmd_shift, Key::S) {
            self.save(true);
        } else if pressed(cmd, Key::S) {
            self.save(false);
        }
        if pressed(cmd_shift, Key::Z) || pressed(cmd, Key::Y) {
            self.run("edit.redo", json!({}));
        } else if pressed(cmd, Key::Z) {
            self.run("edit.undo", json!({}));
        }
        if pressed(cmd, Key::N) {
            self.ask_or(Pending::New);
        }
        if pressed(cmd, Key::O) {
            self.ask_or(Pending::Open);
        }
        if pressed(cmd, Key::Comma) {
            self.prefs_ui.open = !self.prefs_ui.open;
        }
    }

    /// Keeps the window title as "name — Chamfera", with a dot for unsaved changes.
    pub(crate) fn update_title(&mut self, ctx: &egui::Context) {
        let name = self
            .session
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Untitled".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
        let dot = if self.session.has_unsaved_changes() {
            " •"
        } else {
            ""
        };
        let title = format!("{name}{dot} — Chamfera");
        if title != self.file.title {
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            #[cfg(target_arch = "wasm32")]
            {
                let _ = ctx;
                crate::web::set_title(&title);
            }
            self.file.title = title;
        }
    }

    /// The window's close button: ask first when there are unsaved changes.
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.file.allow_close
            && self.session.has_unsaved_changes()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.file.pending = Some(Pending::Quit);
        }
        if self.ctx_close {
            self.ctx_close = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_forced_at_start_survives_opening_a_design_until_the_user_picks_one() {
        // `chamfera --theme dark` with light preferences: opening a design records it in
        // the recent list, which applies the preferences again; that must not undo the
        // forced theme (screenshots of scripts that open a design came out light).
        let options = crate::Options {
            theme: Some(chamfera_render::ThemeMode::Dark),
            ..Default::default()
        };
        let prefs = crate::prefs::Preferences {
            theme: crate::ThemeChoice::Light,
            ..Default::default()
        };
        let mut app = ChamferaApp::with_gpu(None, &options, prefs);
        assert_eq!(app.theme_choice, crate::ThemeChoice::Dark);
        let dir = std::env::temp_dir().join(format!("chamfera-ui-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.chamfera");
        app.run("design.save", json!({ "path": path.to_string_lossy() }));
        app.run("design.open", json!({ "path": path.to_string_lossy() }));
        app.track_recent();
        assert_eq!(app.theme_choice, crate::ThemeChoice::Dark);
        // Choosing a theme in the preferences wins over the start-up option.
        app.prefs.theme = crate::ThemeChoice::System;
        app.preferences_changed();
        assert_eq!(app.theme_choice, crate::ThemeChoice::System);
    }

    #[test]
    fn insert_step_is_in_the_solid_insert_group_and_reports_what_came_in() {
        let t = crate::ribbon::by_id("body.import_step").expect("an Insert STEP tool");
        assert_eq!(t.tab, crate::ribbon::Tab::Solid);
        assert_eq!(t.group, "INSERT");
        // A 10 mm cube written as STEP reads back as one solid of volume 1000.
        let mut source = ChamferaApp::headless();
        source.run("sketch.create", json!({ "plane": "xy" }));
        source.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 10, "y2": 10 }),
        );
        source.run("sketch.finish", json!({}));
        source.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": 10 }),
        );
        let dir = std::env::temp_dir().join(format!("chamfera-ui-step-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cube.step");
        source.run(
            "design.export_step",
            json!({ "path": path.to_string_lossy() }),
        );
        let mut app = ChamferaApp::headless();
        let reply = app.run(
            "body.import_step",
            json!({ "path": path.to_string_lossy() }),
        );
        let v = reply.as_ref().unwrap()["solids"][0]["volume"]
            .as_f64()
            .unwrap();
        assert!((v - 1000.0).abs() < 1e-6, "{v}");
        app.after_step(reply.as_ref());
        assert_eq!(
            app.message,
            Some(("Inserted 1 STEP solid".to_owned(), false))
        );
    }

    #[test]
    fn new_asks_first_only_when_there_are_unsaved_changes() {
        let mut app = ChamferaApp::headless();
        // Nothing changed: New happens at once.
        app.ask_or(Pending::New);
        assert!(app.file.pending.is_none());
        // A change: New waits for an answer, and the design is still there.
        app.run("param.set", json!({ "name": "w", "value": 5 }));
        app.ask_or(Pending::New);
        assert_eq!(app.file.pending, Some(Pending::New));
        assert!(app.session.doc.parameters.contains_key("w"));
        // "Don't Save" goes ahead.
        let action = app.file.pending.take().unwrap();
        app.do_pending(action);
        assert!(app.session.doc.parameters.is_empty());
        assert!(!app.session.has_unsaved_changes());
    }

    #[test]
    fn mesh_files_go_to_the_command_for_their_extension() {
        assert_eq!(mesh_import_command("a.OBJ"), "body.import_obj");
        assert_eq!(mesh_import_command("dir.v2/part.3mf"), "body.import_3mf");
        assert_eq!(mesh_import_command("part.stl"), "body.import_stl");
        assert_eq!(mesh_import_command("noext"), "body.import_stl");
        for (_, _, command) in MESH_EXPORTS {
            assert!(chamfera_engine::cmd::find(command).is_some(), "{command}");
        }
    }
}

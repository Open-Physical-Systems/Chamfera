//! The File menu: New, Open, Save, Save As, Export, Insert, and the question before
//! unsaved changes are thrown away. Keyboard shortcuts for files and undo live here too.
//!
//! The desktop uses file dialogs and paths. In the browser (wasm32) Open and Insert Mesh
//! use the browser's file picker, Save and Export download the file, and the design is
//! kept in local storage (`crate::web`); there is no Save As or Quit.

use crate::icons::Icon;
use crate::widgets;
use crate::{ChamferaApp, label};
use serde_json::json;

/// Something that would discard unsaved changes, waiting for the user's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    New,
    Open,
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
        let mut item = |ui: &mut egui::Ui, icon: Icon, text: &str, keys: &str| {
            let r = widgets::menu_item(ui, look, icon, text, true);
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
        let new = item(ui, Icon::New, "New", "Ctrl+N");
        let open = item(ui, Icon::Open, "Open…", "Ctrl+O");
        ui.separator();
        let save = item(ui, Icon::Save, "Save", "Ctrl+S");
        #[cfg(not(target_arch = "wasm32"))]
        let save_as = item(ui, Icon::Save, "Save As…", "Ctrl+Shift+S");
        ui.separator();
        let step = item(
            ui,
            Icon::Export,
            &format!("{}…", label("design.export_step")),
            "",
        );
        let stl = item(
            ui,
            Icon::Export,
            &format!("{}…", label("design.export_stl")),
            "",
        );
        ui.separator();
        let import = item(
            ui,
            Icon::ImportMesh,
            &format!("{}…", label("body.import_stl")),
            "",
        );
        let sample = item(ui, Icon::Body, label("body.sample"), "");
        #[cfg(not(target_arch = "wasm32"))]
        let quit = {
            ui.separator();
            item(ui, Icon::Close, "Quit", "")
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
        if step {
            ui.close();
            self.export("STEP", &["step", "stp"], "design.export_step");
        }
        if stl {
            ui.close();
            self.export("STL mesh", &["stl"], "design.export_stl");
        }
        if import {
            ui.close();
            self.import_dialog();
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
    fn ask_or(&mut self, action: Pending) {
        if self.session.has_unsaved_changes() {
            self.file.pending = Some(action);
        } else {
            self.do_pending(action);
        }
    }

    fn do_pending(&mut self, action: Pending) {
        match action {
            Pending::New => {
                self.run("design.new", json!({}));
                self.after_new_design();
                #[cfg(target_arch = "wasm32")]
                self.web.clear();
            }
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
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
        if self
            .run("design.open", json!({ "path": path.to_string_lossy() }))
            .is_some()
        {
            self.after_new_design();
            self.message = Some((format!("Opened {}", path.display()), false));
        }
    }

    /// The browser's file picker; the design is opened in `browser_files` once read.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self) {
        self.web.pick(crate::web::Kind::Design, ".chamfera", false);
    }

    /// The browser's file picker for STL meshes, inserted in `browser_files` once read.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn import_dialog(&mut self) {
        self.web.pick(crate::web::Kind::Mesh, ".stl,.STL", true);
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
                crate::web::Kind::Mesh => {
                    let name = picked.name.as_str();
                    if let Some(v) = self.run(
                        "body.import_stl_bytes",
                        json!({ "data": data, "name": name }),
                    ) {
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
        if self
            .run("design.save", json!({ "path": path.to_string_lossy() }))
            .is_some()
        {
            self.message = Some((format!("Saved {}", path.display()), false));
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
    fn export(&mut self, _kind: &str, extensions: &[&str], command: &str) {
        let ext = extensions.first().copied().unwrap_or("out");
        let format = if command == "design.export_step" {
            "step"
        } else {
            "stl"
        };
        let params = json!({ "format": format, "timestamp": crate::web::now_iso8601() });
        let Some(v) = self.run("design.export_bytes", params) else {
            return;
        };
        let name = format!("{}.{ext}", self.design_name());
        let mime = if format == "step" {
            "model/step"
        } else {
            "model/stl"
        };
        match crate::web::download(&name, mime, v["data"].as_str().unwrap_or("")) {
            Ok(()) => self.message = Some((format!("Downloaded {name}"), false)),
            Err(e) => self.message = Some((format!("{}: {e}", label(command)), true)),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export(&mut self, kind: &str, extensions: &[&str], command: &str) {
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
        let look = self.look();
        let name = self.design_name();
        let what = match action {
            Pending::New => "starting a new design",
            Pending::Open => "opening another design",
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
                    if widgets::primary_button(ui, look, "Save", true).clicked() {
                        answer = Some("save");
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        answer = Some("cancel");
                    }
                    if widgets::secondary_button(ui, look, "Don't Save").clicked() {
                        answer = Some("discard");
                    }
                });
            });
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
        if pressed(cmd_shift, Key::S) {
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
}

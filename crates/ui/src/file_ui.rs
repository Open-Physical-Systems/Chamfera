//! The File menu: New, Open, Save, Save As, Export, Insert, and the question before
//! unsaved changes are thrown away. Keyboard shortcuts for files and undo live here too.

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
        let item = |ui: &mut egui::Ui, icon: Icon, text: &str, keys: &str| {
            let r = widgets::menu_item(ui, look, icon, text, true);
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
        #[cfg(not(target_arch = "wasm32"))]
        let (open, save, save_as, step, stl, import) = {
            let open = item(ui, Icon::Open, "Open…", "Ctrl+O");
            ui.separator();
            let save = item(ui, Icon::Save, "Save", "Ctrl+S");
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
            (open, save, save_as, step, stl, import)
        };
        let sample = item(ui, Icon::Body, label("body.sample"), "");
        #[cfg(not(target_arch = "wasm32"))]
        let quit = {
            ui.separator();
            item(ui, Icon::Close, "Quit", "")
        };
        if new {
            ui.close();
            self.ask_or(Pending::New);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if open {
                ui.close();
                self.ask_or(Pending::Open);
            }
            if save {
                ui.close();
                self.save(false);
            }
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

    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self) {}

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

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save(&mut self, _choose: bool) {}

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
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
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

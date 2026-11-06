//! The Shell tool: click the faces to remove (click again to keep), set the wall
//! thickness, OK. Acts only through `solid.shell` (faces are picked with the same ray
//! as `Session::face_at`, by `Session::face_ref_at`).

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::json;

/// The Shell tool while it is open.
pub struct ShellTool {
    /// The faces to remove, by name (`2:top`).
    pub faces: Vec<String>,
    /// The wall thickness expression.
    pub thickness: String,
}

impl Default for ShellTool {
    fn default() -> Self {
        ShellTool {
            faces: Vec::new(),
            thickness: "2 mm".into(),
        }
    }
}

impl ChamferaApp {
    /// Opens the Shell tool.
    pub(crate) fn open_shell_tool(&mut self) {
        self.solid.shell = Some(ShellTool::default());
        self.message = Some((
            "Click the faces to remove, or none for a closed hollow solid".into(),
            false,
        ));
    }

    /// A click in the viewport while the tool is open: picks or unpicks the face there.
    pub(crate) fn shell_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(name) = self.session.face_ref_at(x, y, w, h) else {
            return;
        };
        let Some(tool) = self.solid.shell.as_mut() else {
            return;
        };
        if let Some(i) = tool.faces.iter().position(|f| *f == name) {
            tool.faces.remove(i);
        } else {
            tool.faces.push(name);
        }
    }

    /// The Shell dialog.
    pub(crate) fn shell_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.shell.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = label("solid.shell");
        let ready = !tool.thickness.trim().is_empty();
        egui::Window::new(title)
            .id(egui::Id::unique("shell dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(160.0)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Shell, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("shell grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Open faces");
                            let text = match tool.faces.len() {
                                0 => "None (closed)".to_owned(),
                                1 => "1 selected".to_owned(),
                                n => format!("{n} selected"),
                            };
                            let colour = if tool.faces.is_empty() {
                                look.c(|t| t.text_muted)
                            } else {
                                look.c(|t| t.text)
                            };
                            ui.label(egui::RichText::new(text).color(colour))
                                .on_hover_text("Click a picked face again to keep it");
                            ui.end_row();
                            widgets::field_label(ui, look, "Thickness");
                            let edit = ui.add_sized(
                                widgets::FIELD,
                                egui::TextEdit::singleline(&mut tool.thickness)
                                    .hint_text("2 mm")
                                    .align(egui::Align2::LEFT_CENTER),
                            );
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                ok = true;
                            }
                            ui.end_row();
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
            let params = json!({ "faces": tool.faces, "thickness": tool.thickness.trim() });
            if let Some(r) = self.run("solid.shell", params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {}",
                        r["name"].as_str().unwrap_or(title),
                        self.units().volume(v),
                    ),
                    false,
                ));
                self.solid.shell = None;
            }
        }
        if cancel {
            self.solid.shell = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_the_top_and_ok_shells_the_block() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        );
        let (w, h) = (800.0, 600.0);
        app.run("view.home", json!({ "width": w, "height": h }));
        // Where the middle of the top face (25, 15, 10) is on screen.
        let vp = chamfera_render::mat_mul(
            &app.session
                .camera
                .projection_matrix(w / h, app.session.scene.bounds().as_ref()),
            &app.session.camera.view_matrix(),
        );
        let n = chamfera_render::transform_point(&vp, chamfera_geom::Point3::new(25.0, 15.0, 10.0))
            .unwrap();
        let (x, y) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h);
        app.open_shell_tool();
        app.shell_click(x, y, w, h);
        assert_eq!(app.solid.shell.as_ref().unwrap().faces, vec!["2:top"]);
        // A second click unpicks it, a third picks it again.
        app.shell_click(x, y, w, h);
        assert!(app.solid.shell.as_ref().unwrap().faces.is_empty());
        app.shell_click(x, y, w, h);
        let tool = app.solid.shell.as_ref().unwrap();
        let r = app
            .run(
                "solid.shell",
                json!({ "faces": tool.faces, "thickness": tool.thickness }),
            )
            .unwrap();
        // 2 mm walls, top open: 15000 - 46·26·8.
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - (15_000.0 - 46.0 * 26.0 * 8.0)).abs() < 1e-6, "{v}");
    }
}

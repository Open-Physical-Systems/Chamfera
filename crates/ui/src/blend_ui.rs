//! The Fillet and Chamfer tools: click edges of solids (click again to unpick), set the
//! size, OK. Acts only through `solid.edge_at`, `solid.fillet` and `solid.chamfer`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_engine::solids::edge_ref;
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::json;

/// The Fillet or Chamfer tool while it is open.
pub struct BlendTool {
    /// Chamfer instead of fillet.
    pub chamfer: bool,
    /// The picked edges, by name.
    pub edges: Vec<String>,
    /// The radius (fillet) or distance (chamfer) expression.
    pub size: String,
    /// The edge under the pointer, shown as a preview.
    pub hover: Option<String>,
}

impl BlendTool {
    pub fn new(chamfer: bool) -> BlendTool {
        BlendTool {
            chamfer,
            edges: Vec::new(),
            size: if chamfer { "1 mm" } else { "2 mm" }.into(),
            hover: None,
        }
    }

    fn command(&self) -> &'static str {
        if self.chamfer {
            "solid.chamfer"
        } else {
            "solid.fillet"
        }
    }
}

impl ChamferaApp {
    /// A click in the viewport while the tool is open: picks or unpicks the edge there.
    pub(crate) fn blend_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some((_, _, name)) = self.session.edge_at(x, y, w, h) else {
            return;
        };
        let Some(tool) = self.solid.blend.as_mut() else {
            return;
        };
        if let Some(i) = tool.edges.iter().position(|e| *e == name) {
            tool.edges.remove(i);
        } else {
            tool.edges.push(name);
        }
    }

    /// The pointer moved over the viewport while the tool is open.
    pub(crate) fn blend_hover(&mut self, at: Option<(f64, f64)>, w: f64, h: f64) {
        let hover = at.and_then(|(x, y)| self.session.edge_at(x, y, w, h).map(|e| e.2));
        if let Some(tool) = self.solid.blend.as_mut() {
            tool.hover = hover;
        }
    }

    /// The lines of the named edges, as drawn on the solids.
    fn edge_lines<'a>(
        &self,
        names: impl Iterator<Item = &'a String> + Clone,
    ) -> Vec<[chamfera_geom::Point3; 2]> {
        let mut lines = Vec::new();
        for s in self.session.solids() {
            for (e, line) in &s.edges {
                let Some(r) = edge_ref(&s.body, *e) else {
                    continue;
                };
                if names.clone().any(|n| *n == r) {
                    lines.extend(line.windows(2).filter_map(|p| match p {
                        [a, b] => Some([*a, *b]),
                        _ => None,
                    }));
                }
            }
        }
        lines
    }

    /// The picked edges (highlighted) and the one under the pointer (preview).
    pub(crate) fn blend_overlays(&self) -> Vec<Overlay> {
        let Some(tool) = self.solid.blend.as_ref() else {
            return Vec::new();
        };
        let mut out = vec![Overlay {
            style: OverlayStyle::Highlight,
            lines: self.edge_lines(tool.edges.iter()),
            points: Vec::new(),
            on_top: true,
        }];
        if let Some(h) = tool.hover.as_ref().filter(|h| !tool.edges.contains(h)) {
            out.push(Overlay {
                style: OverlayStyle::Preview,
                lines: self.edge_lines(std::iter::once(h)),
                points: Vec::new(),
                on_top: true,
            });
        }
        out
    }

    /// The Fillet (or Chamfer) dialog.
    pub(crate) fn blend_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.blend.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let command = tool.command();
        let title = label(command);
        let ready = !tool.edges.is_empty() && !tool.size.trim().is_empty();
        egui::Window::new(title)
            .id(egui::Id::unique("blend dialog"))
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
                if widgets::dialog_title(ui, look, Icon::Edit, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("blend grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Edges");
                            let picked = !tool.edges.is_empty();
                            let text = match tool.edges.len() {
                                0 => "Click an edge".to_owned(),
                                1 => "1 selected".to_owned(),
                                n => format!("{n} selected"),
                            };
                            let colour = if picked {
                                look.c(|t| t.text)
                            } else {
                                look.c(|t| t.text_muted)
                            };
                            ui.label(egui::RichText::new(text).color(colour))
                                .on_hover_text("Click a picked edge again to unpick it");
                            ui.end_row();
                            widgets::field_label(
                                ui,
                                look,
                                if tool.chamfer { "Distance" } else { "Radius" },
                            );
                            let edit = ui.add_sized(
                                [180.0, 26.0],
                                egui::TextEdit::singleline(&mut tool.size)
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
            let key = if tool.chamfer { "distance" } else { "radius" };
            let mut params = json!({ "edges": tool.edges });
            params[key] = json!(tool.size.trim());
            if let Some(r) = self.run(command, params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {v:.2} mm³",
                        r["name"].as_str().unwrap_or(title)
                    ),
                    false,
                ));
                self.solid.blend = None;
            }
        }
        if cancel {
            self.solid.blend = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_an_edge_and_ok_fillets_it() {
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
        // Where the middle of the top front edge (25, 0, 10) is on screen.
        let vp = chamfera_render::mat_mul(
            &app.session
                .camera
                .projection_matrix(w / h, app.session.scene.bounds().as_ref()),
            &app.session.camera.view_matrix(),
        );
        let n = chamfera_render::transform_point(&vp, chamfera_geom::Point3::new(25.0, 0.0, 10.0))
            .unwrap();
        let (x, y) = ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h);
        app.solid.blend = Some(BlendTool::new(false));
        app.blend_click(x, y, w, h);
        assert_eq!(app.solid.blend.as_ref().unwrap().edges.len(), 1);
        assert!(!app.blend_overlays()[0].lines.is_empty());
        // A second click unpicks it, a third picks it again.
        app.blend_click(x, y, w, h);
        assert!(app.solid.blend.as_ref().unwrap().edges.is_empty());
        app.blend_click(x, y, w, h);
        let tool = app.solid.blend.as_ref().unwrap();
        let r = app
            .run(
                "solid.fillet",
                json!({ "edges": tool.edges, "radius": tool.size }),
            )
            .unwrap();
        // 15000 less (1 - π/4) r² along the 50 mm edge, r = 2.
        let want = 15_000.0 - (1.0 - std::f64::consts::FRAC_PI_4) * 4.0 * 50.0;
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - want).abs() < 1e-6, "{v}");
    }
}

//! The Fillet and Chamfer tools: click edges of solids (click again to unpick), set the
//! size, OK. With Tangent chain on, each picked edge brings every edge that continues it
//! smoothly (highlighted before OK). A chamfer takes equal distances, two distances or a
//! distance and an angle, with Flip swapping the faces they go on. Acts only through
//! `solid.edge_at`, `solid.tangent_chain`, `solid.fillet` and `solid.chamfer`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_engine::naming::{Kind, Want};
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::json;

/// How a chamfer's second size is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChamferType {
    Equal,
    TwoDistances,
    DistanceAngle,
}

impl ChamferType {
    fn name(self) -> &'static str {
        match self {
            ChamferType::Equal => "Equal distance",
            ChamferType::TwoDistances => "Two distances",
            ChamferType::DistanceAngle => "Distance and angle",
        }
    }
}

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
    /// Also blend the edges that continue the picked ones tangentially.
    pub tangent_chain: bool,
    pub chamfer_type: ChamferType,
    /// The second distance or the angle.
    pub second: String,
    /// Swap the faces the two sizes go on.
    pub flip: bool,
}

impl BlendTool {
    pub fn new(chamfer: bool) -> BlendTool {
        BlendTool {
            chamfer,
            edges: Vec::new(),
            size: if chamfer { "1 mm" } else { "2 mm" }.into(),
            hover: None,
            tangent_chain: true,
            chamfer_type: ChamferType::Equal,
            second: "2 mm".into(),
            flip: false,
        }
    }

    /// The command's parameters.
    pub fn params(&self) -> serde_json::Value {
        let key = if self.chamfer { "distance" } else { "radius" };
        let mut params = json!({ "edges": self.edges, "tangent_chain": self.tangent_chain });
        params[key] = json!(self.size.trim());
        if self.chamfer {
            match self.chamfer_type {
                ChamferType::Equal => {}
                ChamferType::TwoDistances => params["distance2"] = json!(self.second.trim()),
                ChamferType::DistanceAngle => params["angle"] = json!(self.second.trim()),
            }
            if self.chamfer_type != ChamferType::Equal && self.flip {
                params["flip"] = json!(true);
            }
        }
        params
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
    /// `--chamfer`: opens the Chamfer tool with the edges and options of the design's
    /// last chamfer, or empty.
    pub(crate) fn open_chamfer_on_last(&mut self) {
        let mut tool = BlendTool::new(true);
        let last = self
            .session
            .doc
            .features
            .iter()
            .rev()
            .find_map(|f| match &f.kind {
                chamfera_doc::FeatureKind::Blend(b)
                    if b.blend == chamfera_doc::BlendKind::Chamfer =>
                {
                    Some(b.clone())
                }
                _ => None,
            });
        if let Some(b) = last {
            tool.edges = b.edges;
            tool.size = b.size;
            tool.tangent_chain = b.tangent_chain;
            tool.flip = b.flip;
            if let Some(d) = b.distance2 {
                tool.chamfer_type = ChamferType::TwoDistances;
                tool.second = d;
            } else if let Some(a) = b.angle {
                tool.chamfer_type = ChamferType::DistanceAngle;
                tool.second = a;
            }
        }
        self.solid.blend = Some(tool);
    }

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
    pub(crate) fn edge_lines<'a>(
        &self,
        names: impl Iterator<Item = &'a String> + Clone,
    ) -> Vec<[chamfera_geom::Point3; 2]> {
        // Each name through the engine's resolver: every piece of a split edge.
        let wanted: Vec<(usize, _)> = names
            .flat_map(|n| {
                self.session
                    .resolve_reference(None, Kind::Edge, n, Want::All)
                    .unwrap_or_default()
            })
            .filter_map(|f| Some((f.body, f.edge()?)))
            .collect();
        let mut lines = Vec::new();
        for (si, s) in self.session.solids().iter().enumerate() {
            for (e, line) in &s.edges {
                if wanted.contains(&(si, *e)) {
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
        // With the tangent chain on, everything the chain adds is highlighted too.
        let chain = if tool.tangent_chain && !tool.edges.is_empty() {
            self.session
                .tangent_chain_names(&tool.edges)
                .unwrap_or_else(|_| tool.edges.clone())
        } else {
            tool.edges.clone()
        };
        let mut out = vec![Overlay {
            style: OverlayStyle::Highlight,
            lines: self.edge_lines(chain.iter()),
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
        let icon = if tool.chamfer {
            Icon::Chamfer
        } else {
            Icon::Fillet
        };
        let ready = !tool.edges.is_empty()
            && !tool.size.trim().is_empty()
            && (!tool.chamfer
                || tool.chamfer_type == ChamferType::Equal
                || !tool.second.trim().is_empty());
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
                if widgets::dialog_title(ui, look, icon, title) {
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
                                widgets::FIELD,
                                egui::TextEdit::singleline(&mut tool.size)
                                    .hint_text("2 mm")
                                    .align(egui::Align2::LEFT_CENTER),
                            );
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                ok = true;
                            }
                            ui.end_row();
                            if tool.chamfer {
                                widgets::field_label(ui, look, "Type");
                                egui::ComboBox::from_id_salt("chamfer type")
                                    .selected_text(tool.chamfer_type.name())
                                    .width(widgets::FIELD_WIDTH)
                                    .show_ui(ui, |ui| {
                                        for t in [
                                            ChamferType::Equal,
                                            ChamferType::TwoDistances,
                                            ChamferType::DistanceAngle,
                                        ] {
                                            ui.selectable_value(
                                                &mut tool.chamfer_type,
                                                t,
                                                t.name(),
                                            );
                                        }
                                    });
                                ui.end_row();
                                if tool.chamfer_type != ChamferType::Equal {
                                    let angle = tool.chamfer_type == ChamferType::DistanceAngle;
                                    widgets::field_label(
                                        ui,
                                        look,
                                        if angle { "Angle" } else { "Distance 2" },
                                    );
                                    let edit = ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(&mut tool.second)
                                            .hint_text(if angle { "45 deg" } else { "2 mm" })
                                            .align(egui::Align2::LEFT_CENTER),
                                    );
                                    if edit.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        ok = true;
                                    }
                                    ui.end_row();
                                    widgets::field_label(ui, look, "Flip");
                                    ui.checkbox(&mut tool.flip, "")
                                        .on_hover_text("Swap the faces the two sizes go on");
                                    ui.end_row();
                                }
                            }
                            widgets::field_label(ui, look, "Tangent chain");
                            ui.checkbox(&mut tool.tangent_chain, "").on_hover_text(
                                "Also take every edge that continues a picked one smoothly",
                            );
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
            let params = tool.params();
            if let Some(r) = self.run(command, params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {}",
                        r["name"].as_str().unwrap_or(title),
                        self.units().volume(v),
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
        let params = app.solid.blend.as_ref().unwrap().params();
        let r = app.run("solid.fillet", params).unwrap();
        // 15000 less (1 - π/4) r² along the 50 mm edge, r = 2.
        let want = 15_000.0 - (1.0 - std::f64::consts::FRAC_PI_4) * 4.0 * 50.0;
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - want).abs() < 1e-6, "{v}");
    }

    #[test]
    fn a_chamfer_of_a_slot_line_takes_its_whole_tangent_chain() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.slot",
            json!({ "x1": -10, "y1": 0, "x2": 10, "y2": 0, "width": 20 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 0, "y": 0 }], "distance": 10 }),
        );
        let edges = app.run("solid.edges", json!({})).unwrap();
        // The straight top edge at y = -10, z = 10.
        let at = |v: &serde_json::Value, x: f64| {
            (v[0].as_f64().unwrap() - x).abs() < 1e-9
                && (v[1].as_f64().unwrap() + 10.0).abs() < 1e-9
                && (v[2].as_f64().unwrap() - 10.0).abs() < 1e-9
        };
        let line = edges["solids"][0]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                (at(&e["from"], -10.0) && at(&e["to"], 10.0))
                    || (at(&e["from"], 10.0) && at(&e["to"], -10.0))
            })
            .unwrap()["edge"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut tool = BlendTool::new(true);
        tool.edges.push(line);
        tool.chamfer_type = ChamferType::TwoDistances;
        tool.second = "1 mm".into();
        let one = app.edge_lines(tool.edges.iter()).len();
        app.solid.blend = Some(tool);
        // The highlight covers the whole rim, not just the line.
        assert!(app.blend_overlays()[0].lines.len() > one);
        let params = app.solid.blend.as_ref().unwrap().params();
        let r = app.run("solid.chamfer", params).unwrap();
        // Legs of 1: the two 20 mm lines lose 1²/2 each mm; the two half-turn cones
        // together are one turn of the boss rim's triangle, centroid at ρ = 10 - 1/3:
        // 2π (10 - 1/3) / 2 (Pappus).
        let pi = std::f64::consts::PI;
        let v0 = (2.0 * 10.0 * 20.0 + pi * 100.0) * 10.0;
        let want = v0 - 2.0 * 20.0 * 0.5 - pi * (10.0 - 1.0 / 3.0);
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - want).abs() < 1e-6, "{v} vs {want}");
    }
}

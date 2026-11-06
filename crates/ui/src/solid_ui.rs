//! Solid modelling in the UI: the Extrude tool (pick profiles, set a distance) and the
//! timeline list of features. Acts only through `solid.*` and `feature.*` commands.

use crate::{ChamferaApp, color, label};
use chamfera_doc::{ExtrudeDirection, FeatureKind};
use chamfera_geom::Point2;
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::json;

/// The Extrude tool while it is open.
pub struct ExtrudeTool {
    sketch: Option<u32>,
    /// A point inside each picked profile.
    seeds: Vec<Point2>,
    pub distance: String,
    direction: ExtrudeDirection,
}

impl Default for ExtrudeTool {
    fn default() -> Self {
        ExtrudeTool {
            sketch: None,
            seeds: Vec::new(),
            distance: "10 mm".into(),
            direction: ExtrudeDirection::Normal,
        }
    }
}

#[derive(Default)]
pub struct SolidUi {
    pub extrude: Option<ExtrudeTool>,
    /// Feature distance being edited in the timeline: (feature id, text).
    editing: Option<(u32, String)>,
}

impl ChamferaApp {
    pub(crate) fn solid_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("SOLID");
            ui.separator();
            ui.menu_button(label("sketch.create"), |ui| {
                for (plane, text, view) in crate::sketch_ui::PLANES {
                    if ui.button(text).clicked() {
                        ui.close();
                        self.create_sketch(plane, view);
                    }
                }
            });
            let has_profiles = self
                .session
                .doc
                .sketches
                .iter()
                .any(|s| !s.sketch.profiles().is_empty());
            if ui
                .add_enabled(
                    has_profiles && self.solid.extrude.is_none(),
                    egui::Button::new(label("solid.extrude")),
                )
                .on_disabled_hover_text("Draw a closed profile in a sketch first")
                .clicked()
            {
                self.solid.extrude = Some(ExtrudeTool::default());
                self.message = Some(("Click inside the profiles to extrude".into(), false));
            }
        });
    }

    /// A click in the viewport while the Extrude tool is open: picks or unpicks a profile.
    pub(crate) fn extrude_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Ok(r) = self.session.execute(
            "solid.profile_at",
            &json!({ "x": x, "y": y, "width": w, "height": h }),
        ) else {
            return;
        };
        let (Some(sketch), Some(px), Some(py)) = (
            r["sketch"].as_u64().and_then(|v| u32::try_from(v).ok()),
            r["x"].as_f64(),
            r["y"].as_f64(),
        ) else {
            return;
        };
        let p = Point2::new(px, py);
        let Some(tool) = self.solid.extrude.as_mut() else {
            return;
        };
        if tool.sketch.is_some_and(|s| s != sketch) {
            self.message = Some((
                "One extrude takes profiles from one sketch: finish this one first".into(),
                true,
            ));
            return;
        }
        tool.sketch = Some(sketch);
        // Clicking a picked profile again unpicks it.
        let region_of = |q: Point2| {
            self.session
                .doc
                .sketch(sketch)
                .ok()
                .and_then(|f| f.sketch.region_at(q))
        };
        let clicked = region_of(p);
        if let Some(i) = tool
            .seeds
            .iter()
            .position(|s| region_of(*s).is_some() && region_of(*s) == clicked)
        {
            tool.seeds.remove(i);
            if tool.seeds.is_empty() {
                tool.sketch = None;
            }
        } else {
            tool.seeds.push(p);
        }
    }

    /// Outlines of the picked profiles, drawn highlighted.
    pub(crate) fn extrude_overlay(&self) -> Option<Overlay> {
        let tool = self.solid.extrude.as_ref()?;
        let sketch = tool.sketch?;
        let lines = tool
            .seeds
            .iter()
            .flat_map(|s| self.session.profile_outline(sketch, *s))
            .collect();
        Some(Overlay {
            style: OverlayStyle::Highlight,
            lines,
            points: Vec::new(),
            on_top: true,
        })
    }

    /// The Extrude dialog.
    pub(crate) fn extrude_dialog(&mut self, ctx: &egui::Context) {
        let Some(tool) = self.solid.extrude.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Window::new(label("solid.extrude"))
            .default_pos([240.0, 110.0])
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(match tool.seeds.len() {
                    0 => "Click inside a closed profile to pick it".to_owned(),
                    1 => "1 profile picked (click it again to unpick)".to_owned(),
                    n => format!("{n} profiles picked"),
                });
                egui::Grid::new("extrude grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Distance");
                        let edit = ui.add_sized(
                            [160.0, 20.0],
                            egui::TextEdit::singleline(&mut tool.distance)
                                .hint_text("10 mm or thickness * 2"),
                        );
                        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            ok = true;
                        }
                        ui.end_row();
                        ui.label("Direction");
                        egui::ComboBox::from_id_salt("extrude direction")
                            .selected_text(direction_name(tool.direction))
                            .show_ui(ui, |ui| {
                                for d in [
                                    ExtrudeDirection::Normal,
                                    ExtrudeDirection::Reversed,
                                    ExtrudeDirection::Symmetric,
                                ] {
                                    ui.selectable_value(&mut tool.direction, d, direction_name(d));
                                }
                            });
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!tool.seeds.is_empty(), egui::Button::new("OK"))
                        .clicked()
                    {
                        ok = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ok && !tool.seeds.is_empty() {
            let params = json!({
                "sketch": tool.sketch,
                "profiles": tool.seeds.iter().map(|p| json!({ "x": p.x, "y": p.y })).collect::<Vec<_>>(),
                "distance": tool.distance.trim(),
                "direction": direction_key(tool.direction),
            });
            if let Some(r) = self.run("solid.extrude", params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {v:.2} mm³",
                        r["name"].as_str().unwrap_or("Extrude")
                    ),
                    false,
                ));
                self.solid.extrude = None;
            }
        }
        if cancel {
            self.solid.extrude = None;
        }
    }

    /// The timeline in the Browser: features with their errors, suppress, edit, delete.
    pub(crate) fn timeline(&mut self, ui: &mut egui::Ui) {
        if self.session.doc.features.is_empty() {
            return;
        }
        ui.separator();
        ui.strong("Timeline");
        let rows: Vec<(u32, String, bool, Option<String>)> = self
            .session
            .doc
            .features
            .iter()
            .map(|f| {
                let distance = match &f.kind {
                    FeatureKind::Extrude(e) => Some(e.distance.clone()),
                    FeatureKind::Revolve(r) => Some(r.angle.clone()),
                };
                (f.id, f.name.clone(), f.suppressed, distance)
            })
            .collect();
        for (id, name, suppressed, distance) in rows {
            let error = self.session.feature_errors().get(&id).cloned();
            ui.horizontal(|ui| {
                let mut text = egui::RichText::new(&name);
                if suppressed {
                    text = text.strikethrough().weak();
                }
                if error.is_some() {
                    text = text.color(color(self.theme.error));
                }
                let resp = ui.label(text);
                if let Some(e) = &error {
                    resp.on_hover_text(e);
                }
                let mut on = !suppressed;
                if ui
                    .checkbox(&mut on, "")
                    .on_hover_text("Build this feature (off suppresses it)")
                    .changed()
                {
                    self.run("feature.edit", json!({ "feature": id, "suppressed": !on }));
                }
                if ui.small_button("Edit").clicked() {
                    self.solid.editing = Some((id, distance.clone().unwrap_or_default()));
                }
                if ui.small_button("Delete").clicked() {
                    self.run("feature.delete", json!({ "feature": id }));
                }
            });
            if let Some((eid, text)) = self.solid.editing.as_mut()
                && *eid == id
            {
                let mut done = false;
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    let edit = ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(text));
                    if edit.lost_focus() {
                        done = true;
                    }
                });
                if done {
                    let distance = text.trim().to_owned();
                    self.solid.editing = None;
                    self.run(
                        "feature.edit",
                        json!({ "feature": id, "distance": distance }),
                    );
                }
            }
        }
    }
}

fn direction_name(d: ExtrudeDirection) -> &'static str {
    match d {
        ExtrudeDirection::Normal => "One side",
        ExtrudeDirection::Reversed => "Flipped",
        ExtrudeDirection::Symmetric => "Symmetric",
    }
}

fn direction_key(d: ExtrudeDirection) -> &'static str {
    match d {
        ExtrudeDirection::Normal => "normal",
        ExtrudeDirection::Reversed => "reversed",
        ExtrudeDirection::Symmetric => "symmetric",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picking the plate by clicking its middle in a top view, then OK, builds the solid.
    #[test]
    fn picking_a_profile_and_ok_extrudes_it() {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 20 }),
        );
        app.finish_sketch();
        app.session.camera.target = chamfera_geom::Point3::new(20.0, 10.0, 0.0);
        app.solid.extrude = Some(ExtrudeTool::default());
        app.extrude_click(400.0, 300.0, 800.0, 600.0);
        assert_eq!(app.solid.extrude.as_ref().unwrap().seeds.len(), 1);
        // Clicking it again unpicks, a third click picks it again.
        app.extrude_click(400.0, 300.0, 800.0, 600.0);
        assert!(app.solid.extrude.as_ref().unwrap().seeds.is_empty());
        app.extrude_click(400.0, 300.0, 800.0, 600.0);
        let tool = app.solid.extrude.as_ref().unwrap();
        let params = json!({
            "sketch": tool.sketch,
            "profiles": tool.seeds.iter().map(|p| json!({ "x": p.x, "y": p.y })).collect::<Vec<_>>(),
            "distance": "5",
        });
        let r = app.run("solid.extrude", params).unwrap();
        assert!(
            (r["solids"][0]["volume"].as_f64().unwrap() - 4000.0).abs() < 1e-6,
            "40 x 20 x 5"
        );
    }
}

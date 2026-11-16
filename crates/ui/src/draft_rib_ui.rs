//! The Draft and Rib tools. Draft: click the faces to tilt, then (after clicking the
//! Neutral plane field) the face they turn about, set the angle, OK. Rib: click a sketch
//! line or arc (its chain is picked), set the thickness and whether it is a web, OK. Both
//! show the result as a translucent live preview while the dialog is open, through the
//! commands' `preview`, and act only through `solid.draft` and `solid.rib`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::{Value, json};

/// What a click in the viewport picks while the Draft tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftPick {
    Faces,
    Neutral,
}

/// The Draft tool while it is open.
pub struct DraftTool {
    /// The faces to tilt, by name (`2:side 0`).
    pub faces: Vec<String>,
    /// The neutral plane: a face name, or `xy` until one is picked.
    pub neutral: String,
    pub picking: DraftPick,
    /// The angle expression.
    pub angle: String,
    pub flip: bool,
    /// The parameters the preview shows, and what it said: the volume or why not.
    preview_for: Option<Value>,
    pub preview: Result<f64, String>,
}

impl Default for DraftTool {
    fn default() -> Self {
        DraftTool {
            faces: Vec::new(),
            neutral: "xy".into(),
            picking: DraftPick::Faces,
            angle: "3 deg".into(),
            flip: false,
            preview_for: None,
            preview: Err(String::new()),
        }
    }
}

impl DraftTool {
    /// The `solid.draft` parameters, once there is something to draft.
    pub fn params(&self) -> Option<Value> {
        (!self.faces.is_empty() && !self.angle.trim().is_empty()).then(|| {
            json!({
                "faces": self.faces,
                "neutral": self.neutral,
                "angle": self.angle.trim(),
                "flip": self.flip,
            })
        })
    }
}

/// The Rib tool while it is open.
pub struct RibTool {
    pub sketch: Option<u32>,
    /// The chain of lines and arcs, in order.
    pub curves: Vec<u32>,
    /// The thickness expression.
    pub thickness: String,
    pub web: bool,
    pub flip: bool,
    preview_for: Option<Value>,
    pub preview: Result<f64, String>,
}

impl Default for RibTool {
    fn default() -> Self {
        RibTool {
            sketch: None,
            curves: Vec::new(),
            thickness: "2 mm".into(),
            web: false,
            flip: false,
            preview_for: None,
            preview: Err(String::new()),
        }
    }
}

impl RibTool {
    /// The `solid.rib` parameters, once a chain is picked.
    pub fn params(&self) -> Option<Value> {
        let sketch = self.sketch?;
        (!self.curves.is_empty() && !self.thickness.trim().is_empty()).then(|| {
            json!({
                "sketch": sketch,
                "curves": self.curves,
                "thickness": self.thickness.trim(),
                "web": self.web,
                "flip": self.flip,
            })
        })
    }
}

/// The face a neutral plane names, for the dialog.
fn neutral_text(n: &str) -> String {
    match n {
        "xy" => "XY plane".into(),
        "yz" => "YZ plane".into(),
        "xz" => "XZ plane".into(),
        face => face.to_owned(),
    }
}

/// The preview's line: the volume, or why there is none.
fn preview_line(app_units: &chamfera_doc::DisplayUnits, p: &Result<f64, String>) -> (String, bool) {
    match p {
        Ok(v) => (format!("Preview: volume {}", app_units.volume(*v)), false),
        Err(e) if e.is_empty() => ("Pick to see a preview".into(), false),
        Err(e) => (e.clone(), true),
    }
}

impl ChamferaApp {
    /// Opens the Draft tool.
    pub(crate) fn open_draft_tool(&mut self) {
        self.solid.draft = Some(DraftTool::default());
        self.message = Some((
            "Click the faces to draft; the neutral plane is XY until you pick a face for it".into(),
            false,
        ));
    }

    /// Opens the Rib tool.
    pub(crate) fn open_rib_tool(&mut self) {
        self.solid.rib = Some(RibTool::default());
        self.message = Some((
            "Click a sketch line or arc: the rib grows from its chain to the solid".into(),
            false,
        ));
    }

    /// `--dialog draft:JSON` or `rib:JSON`: opens the tool filled in with those
    /// `solid.draft` or `solid.rib` parameters, its preview shown, for screenshots.
    pub(crate) fn open_draft_rib_with(&mut self, rib: bool, json: &str) -> Result<(), String> {
        let p: Value = serde_json::from_str(json)
            .map_err(|e| format!("--dialog takes the tool's parameters as JSON: {e}"))?;
        let text = |key: &str| match p.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        let flag = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(false);
        if rib {
            let mut t = RibTool {
                sketch: p
                    .get("sketch")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok()),
                curves: p
                    .get("curves")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
                            .collect()
                    })
                    .unwrap_or_default(),
                web: flag("web"),
                flip: flag("flip"),
                ..RibTool::default()
            };
            if let Some(th) = text("thickness") {
                t.thickness = th;
            }
            self.solid.rib = Some(t);
        } else {
            let mut t = DraftTool {
                faces: p
                    .get("faces")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                flip: flag("flip"),
                ..DraftTool::default()
            };
            if let Some(n) = text("neutral") {
                t.neutral = n;
            }
            if let Some(a) = text("angle") {
                t.angle = a;
            }
            self.solid.draft = Some(t);
        }
        self.update_draft_rib_preview();
        Ok(())
    }

    /// A click in the viewport while the Draft tool is open: picks or unpicks a face, or
    /// sets the neutral plane.
    pub(crate) fn draft_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(name) = self.session.face_ref_at(x, y, w, h) else {
            return;
        };
        let Some(tool) = self.solid.draft.as_mut() else {
            return;
        };
        match tool.picking {
            DraftPick::Neutral => {
                tool.neutral = name;
                tool.picking = DraftPick::Faces;
            }
            DraftPick::Faces => {
                if let Some(i) = tool.faces.iter().position(|f| *f == name) {
                    tool.faces.remove(i);
                } else {
                    tool.faces.push(name);
                }
            }
        }
    }

    /// A click in the viewport while the Rib tool is open: picks the chain of the sketch
    /// line or arc there (again: unpicks it).
    pub(crate) fn rib_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some((sketch, curve, chain)) = self.session.path_at(x, y, w, h) else {
            return;
        };
        let Some(tool) = self.solid.rib.as_mut() else {
            return;
        };
        if tool.sketch == Some(sketch) && tool.curves.contains(&curve) {
            tool.sketch = None;
            tool.curves.clear();
        } else {
            tool.sketch = Some(sketch);
            tool.curves = chain;
        }
    }

    /// Keeps the translucent preview in step with the open Draft or Rib tool; true when
    /// it changed.
    pub(crate) fn update_draft_rib_preview(&mut self) -> bool {
        let (command, wanted, shown) = if let Some(t) = self.solid.draft.as_ref() {
            ("solid.draft", t.params(), t.preview_for.clone())
        } else if let Some(t) = self.solid.rib.as_ref() {
            ("solid.rib", t.params(), t.preview_for.clone())
        } else {
            return false;
        };
        if wanted == shown {
            return false;
        }
        let result = match &wanted {
            Some(p) => {
                let mut p = p.clone();
                p["preview"] = json!(true);
                self.session
                    .execute(command, &p)
                    .map(|r| r["volume"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())
            }
            None => {
                let _ = self.session.execute(command, &json!({ "clear": true }));
                Err(String::new())
            }
        };
        self.preview_version += 1;
        if let Some(t) = self.solid.draft.as_mut() {
            t.preview = result;
            t.preview_for = wanted;
        } else if let Some(t) = self.solid.rib.as_mut() {
            t.preview = result;
            t.preview_for = wanted;
        }
        true
    }

    /// Closes the tool and its preview.
    fn close_draft_rib(&mut self) {
        self.solid.draft = None;
        self.solid.rib = None;
        let _ = self
            .session
            .execute("solid.draft", &json!({ "clear": true }));
        self.preview_version += 1;
    }

    /// Runs the open tool's command; true when the feature was built.
    pub(crate) fn draft_rib_ok(&mut self) -> bool {
        let (command, params) = if let Some(t) = self.solid.draft.as_ref() {
            ("solid.draft", t.params())
        } else if let Some(t) = self.solid.rib.as_ref() {
            ("solid.rib", t.params())
        } else {
            return false;
        };
        let Some(params) = params else {
            return false;
        };
        let Some(r) = self.run(command, params) else {
            return false;
        };
        let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
        self.message = Some((
            format!(
                "{} · volume {}",
                r["name"].as_str().unwrap_or(label(command)),
                self.units().volume(v),
            ),
            false,
        ));
        self.close_draft_rib();
        true
    }

    /// The Draft or Rib dialog, whichever is open.
    pub(crate) fn draft_rib_dialog(&mut self, ctx: &egui::Context) {
        if self.solid.draft.is_none() && self.solid.rib.is_none() {
            return;
        }
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let units = self.units();
        let sketch_name = self
            .solid
            .rib
            .as_ref()
            .and_then(|t| t.sketch)
            .and_then(|s| self.session.doc.sketch(s).ok().map(|f| f.name.clone()));
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let draft = self.solid.draft.is_some();
        let (icon, title) = if draft {
            (Icon::Draft, label("solid.draft"))
        } else {
            (Icon::Rib, label("solid.rib"))
        };
        let mut ok = false;
        let mut cancel = false;
        let ready = self
            .solid
            .draft
            .as_ref()
            .map(DraftTool::params)
            .or_else(|| self.solid.rib.as_ref().map(RibTool::params))
            .flatten()
            .is_some();
        let (draft_tool, rib_tool) = (&mut self.solid.draft, &mut self.solid.rib);
        egui::Window::new(title)
            .id(egui::Id::unique("draft rib dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(180.0)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, icon, title) {
                    cancel = true;
                }
                let (muted, text, error) = (
                    look.c(|t| t.text_muted),
                    look.c(|t| t.text),
                    look.c(|t| t.error),
                );
                let mut preview = Err(String::new());
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("draft rib grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            if let Some(tool) = draft_tool.as_mut() {
                                widgets::field_label(ui, look, "Faces");
                                let n = tool.faces.len();
                                let faces = match n {
                                    0 => "Click the faces to draft".to_owned(),
                                    1 => "1 selected".to_owned(),
                                    n => format!("{n} selected"),
                                };
                                if ui
                                    .selectable_label(
                                        tool.picking == DraftPick::Faces,
                                        egui::RichText::new(faces)
                                            .color(if n == 0 { muted } else { text }),
                                    )
                                    .on_hover_text("Click here, then faces; a picked face again unpicks it")
                                    .clicked()
                                {
                                    tool.picking = DraftPick::Faces;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Neutral plane");
                                if ui
                                    .selectable_label(
                                        tool.picking == DraftPick::Neutral,
                                        neutral_text(&tool.neutral),
                                    )
                                    .on_hover_text(
                                        "Click here, then the flat face the drafted faces turn about; the pull goes into the solid",
                                    )
                                    .clicked()
                                {
                                    tool.picking = DraftPick::Neutral;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Angle");
                                let edit = ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut tool.angle)
                                        .hint_text("3 deg")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if edit.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    ok = true;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Direction");
                                ui.checkbox(&mut tool.flip, "Flip the pull");
                                ui.end_row();
                                preview = tool.preview.clone();
                            } else if let Some(tool) = rib_tool.as_mut() {
                                widgets::field_label(ui, look, "Curves");
                                let n = tool.curves.len();
                                let chain = match (&sketch_name, n) {
                                    (Some(s), n) if n > 0 => format!(
                                        "{n} curve{} of {s}",
                                        if n == 1 { "" } else { "s" }
                                    ),
                                    _ => "Click a sketch line or arc".into(),
                                };
                                ui.label(
                                    egui::RichText::new(chain).color(if n == 0 { muted } else { text }),
                                )
                                .on_hover_text("The chain it is part of is picked; click it again to unpick");
                                ui.end_row();
                                widgets::field_label(ui, look, "Thickness");
                                let edit = ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut tool.thickness)
                                        .hint_text("2 mm")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if edit.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    ok = true;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Extent");
                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut tool.web, false, "To the solid")
                                        .on_hover_text("Rib: stop at the first faces it meets");
                                    ui.selectable_value(&mut tool.web, true, "Through")
                                        .on_hover_text("Web: fill every gap it crosses");
                                });
                                ui.end_row();
                                widgets::field_label(ui, look, "Direction");
                                ui.checkbox(&mut tool.flip, "Grow the other way");
                                ui.end_row();
                                preview = tool.preview.clone();
                            }
                        });
                    let (line, bad) = preview_line(&units, &preview);
                    ui.label(egui::RichText::new(line).color(if bad { error } else { muted }));
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
            self.draft_rib_ok();
        }
        if cancel {
            self.close_draft_rib();
        }
        if self.update_draft_rib_preview() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the 3D point `p` lands in an 800 x 600 viewport at the home view.
    fn pixel(app: &ChamferaApp, p: chamfera_geom::Point3) -> (f64, f64) {
        let (w, h) = (800.0, 600.0);
        let vp = chamfera_render::mat_mul(
            &app.session
                .camera
                .projection_matrix(w / h, app.session.scene.bounds().as_ref()),
            &app.session.camera.view_matrix(),
        );
        let n = chamfera_render::transform_point(&vp, p).unwrap();
        ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h)
    }

    #[test]
    fn drafting_two_clicked_sides_previews_and_builds_the_wedged_block() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 20 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 20, "y": 10 }], "distance": 10 }),
        );
        app.run("view.home", json!({ "width": 800, "height": 600 }));
        app.open_draft_tool();
        // The middles of the walls y = 0 and x = 40, both facing the home view.
        for p in [(20.0, 0.0, 5.0), (40.0, 10.0, 5.0)] {
            let (x, y) = pixel(&app, chamfera_geom::Point3::new(p.0, p.1, p.2));
            app.draft_click(x, y, 800.0, 600.0);
        }
        let tool = app.solid.draft.as_mut().unwrap();
        assert_eq!(tool.faces.len(), 2, "{:?}", tool.faces);
        tool.angle = "10 deg".into();
        assert!(app.update_draft_rib_preview());
        // Two adjacent walls lean in: the section at height z is (40 - z t)(20 - z t),
        // t = tan 10°, so V = ∫₀¹⁰ (800 - 60 t z + t² z²) dz = 8000 - 3000 t + (1000/3) t².
        let t = 10f64.to_radians().tan();
        let want = 8000.0 - 3000.0 * t + 1000.0 / 3.0 * t * t;
        let got = app.solid.draft.as_ref().unwrap().preview.clone().unwrap();
        assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        assert_eq!(app.session.scene.previews.len(), 1);
        assert!(app.draft_rib_ok());
        assert!(app.solid.draft.is_none());
        assert!(app.session.scene.previews.is_empty());
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - want).abs() < 1e-6, "{v} vs {want}");
    }

    #[test]
    fn a_rib_tool_picks_the_chain_and_flips() {
        let mut app = ChamferaApp::headless();
        app.open_rib_tool();
        let tool = app.solid.rib.as_mut().unwrap();
        assert!(tool.params().is_none());
        tool.sketch = Some(3);
        tool.curves = vec![4, 5];
        tool.web = true;
        let p = tool.params().unwrap();
        assert_eq!(p["curves"], json!([4, 5]));
        assert_eq!(p["web"], json!(true));
        // With no solid the preview says why.
        assert!(app.update_draft_rib_preview());
        assert!(app.solid.rib.as_ref().unwrap().preview.is_err());
        app.close_draft_rib();
        assert!(app.solid.rib.is_none());
    }
}

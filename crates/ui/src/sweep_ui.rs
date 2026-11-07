//! The Sweep and Loft tools. Sweep: click a closed profile, then a line or arc of the
//! path (its whole chain of lines and arcs is picked), OK. Loft: click the profiles in
//! order, OK. Acts only through `solid.profile_at`, `solid.path_at`, `solid.sweep` and
//! `solid.loft`.

use crate::icons::Icon;
use crate::solid_ui::operation_name;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::Operation;
use chamfera_geom::Point2;
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::{Value, json};

/// What a click in the viewport picks while the Sweep tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepPick {
    Profile,
    Path,
}

/// The Sweep or Loft tool while it is open.
pub struct SweepTool {
    /// Loft instead of sweep.
    pub loft: bool,
    /// Picked profiles as (sketch, a point inside): one for a sweep, in order for a loft.
    pub profiles: Vec<(u32, Point2)>,
    /// Sweep: the path's sketch and curves.
    pub path_sketch: Option<u32>,
    pub path: Vec<u32>,
    pub picking: SweepPick,
    pub operation: Operation,
}

impl SweepTool {
    pub fn new(loft: bool) -> SweepTool {
        SweepTool {
            loft,
            profiles: Vec::new(),
            path_sketch: None,
            path: Vec::new(),
            picking: SweepPick::Profile,
            operation: Operation::NewBody,
        }
    }

    /// True when OK can build the feature.
    pub fn ready(&self) -> bool {
        if self.loft {
            self.profiles.len() >= 2
        } else {
            !self.profiles.is_empty() && self.path_sketch.is_some() && !self.path.is_empty()
        }
    }

    /// The command and its parameters for OK.
    pub fn command(&self) -> (&'static str, Value) {
        if self.loft {
            let sections: Vec<Value> = self
                .profiles
                .iter()
                .map(|(s, p)| json!({ "sketch": s, "x": p.x, "y": p.y }))
                .collect();
            (
                "solid.loft",
                json!({ "sections": sections, "operation": self.operation.key() }),
            )
        } else {
            let sketch = self.profiles.first().map(|p| p.0);
            let profiles: Vec<Value> = self
                .profiles
                .iter()
                .map(|(_, p)| json!({ "x": p.x, "y": p.y }))
                .collect();
            (
                "solid.sweep",
                json!({
                    "sketch": sketch,
                    "profiles": profiles,
                    "path_sketch": self.path_sketch,
                    "path_curves": self.path,
                    "operation": self.operation.key(),
                }),
            )
        }
    }
}

impl ChamferaApp {
    /// Opens the Sweep tool, or the Loft tool.
    pub(crate) fn open_sweep(&mut self, loft: bool) {
        self.solid.sweep = Some(SweepTool::new(loft));
        self.message = Some((
            if loft {
                "Click the profiles to loft through, in order"
            } else {
                "Click the profile to sweep, then a line or arc of the path"
            }
            .into(),
            false,
        ));
    }

    /// A click in the viewport while the tool is open.
    pub(crate) fn sweep_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let at = json!({ "x": x, "y": y, "width": w, "height": h });
        let Some(picking) = self.solid.sweep.as_ref().map(|t| t.picking) else {
            return;
        };
        if picking == SweepPick::Path {
            let Ok(r) = self.session.execute("solid.path_at", &at) else {
                return;
            };
            let sketch = r["sketch"].as_u64().and_then(|v| u32::try_from(v).ok());
            let chain: Vec<u32> = r["chain"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
                        .collect()
                })
                .unwrap_or_default();
            let Some(tool) = self.solid.sweep.as_mut() else {
                return;
            };
            match sketch {
                Some(s) => {
                    // Clicking the picked path again unpicks it.
                    let curve = r["curve"].as_u64().and_then(|v| u32::try_from(v).ok());
                    if tool.path_sketch == Some(s) && curve.is_some_and(|c| tool.path.contains(&c))
                    {
                        tool.path.clear();
                        tool.path_sketch = None;
                    } else {
                        tool.path_sketch = Some(s);
                        tool.path = chain;
                    }
                }
                None => {
                    self.message = Some(("Click a line or arc of the path".into(), false));
                }
            }
            return;
        }
        let Ok(r) = self.session.execute("solid.profile_at", &at) else {
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
        let region_of = |s: u32, q: Point2| {
            self.session
                .doc
                .sketch(s)
                .ok()
                .and_then(|f| f.sketch.region_at(q))
        };
        let clicked = region_of(sketch, p);
        let picked = self.solid.sweep.as_ref().and_then(|t| {
            t.profiles
                .iter()
                .position(|(s, q)| *s == sketch && region_of(*s, *q) == clicked)
        });
        let Some(tool) = self.solid.sweep.as_mut() else {
            return;
        };
        if let Some(i) = picked {
            tool.profiles.remove(i);
        } else if tool.loft {
            tool.profiles.push((sketch, p));
        } else {
            tool.profiles = vec![(sketch, p)];
            tool.picking = SweepPick::Path;
            self.message = Some(("Now click a line or arc of the path".into(), false));
        }
    }

    /// The picked profiles' outlines and the picked path, drawn highlighted.
    pub(crate) fn sweep_overlay(&self) -> Option<Overlay> {
        let tool = self.solid.sweep.as_ref()?;
        let mut lines: Vec<[chamfera_geom::Point3; 2]> = tool
            .profiles
            .iter()
            .flat_map(|(s, p)| self.session.profile_outline(*s, *p))
            .collect();
        if let Some(f) = tool
            .path_sketch
            .and_then(|s| self.session.doc.sketch(s).ok())
        {
            for id in &tool.path {
                let Some(c) = f.sketch.curve(chamfera_sketch::CurveId(*id)) else {
                    continue;
                };
                let poly = chamfera_engine::sketching::curve_polyline(&f.sketch, c);
                for pair in poly.windows(2) {
                    if let [a, b] = pair {
                        lines.push([f.plane.from_plane(*a), f.plane.from_plane(*b)]);
                    }
                }
            }
        }
        Some(Overlay {
            style: OverlayStyle::Highlight,
            lines,
            points: Vec::new(),
            on_top: true,
        })
    }

    /// Builds the feature with the tool's picks; true when it was built.
    pub(crate) fn sweep_ok(&mut self) -> bool {
        let Some(tool) = self.solid.sweep.as_ref() else {
            return false;
        };
        if !tool.ready() {
            return false;
        }
        let (id, params) = tool.command();
        let Some(r) = self.run(id, params) else {
            return false;
        };
        let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
        self.message = Some((
            format!(
                "{} · volume {v:.2} mm³",
                r["name"].as_str().unwrap_or(label(id))
            ),
            false,
        ));
        self.solid.sweep = None;
        true
    }

    /// The Sweep or Loft dialog.
    pub(crate) fn sweep_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let sketch_name = |id: u32| {
            self.session
                .doc
                .sketch(id)
                .map_or_else(|_| format!("Sketch {id}"), |f| f.name.clone())
        };
        let Some(names) = self.solid.sweep.as_ref().map(|t| {
            (
                t.profiles
                    .iter()
                    .map(|(s, _)| sketch_name(*s))
                    .collect::<Vec<_>>(),
                t.path_sketch.map(sketch_name),
            )
        }) else {
            return;
        };
        let Some(tool) = self.solid.sweep.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let (icon, title) = if tool.loft {
            (Icon::Loft, label("solid.loft"))
        } else {
            (Icon::Sweep, label("solid.sweep"))
        };
        let ready = tool.ready();
        egui::Window::new(title)
            .id(egui::Id::unique("sweep dialog"))
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
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sweep grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            let muted = look.c(|t| t.text_muted);
                            let text = look.c(|t| t.text);
                            if tool.loft {
                                widgets::field_label(ui, look, "Profiles");
                                let list = if names.0.is_empty() {
                                    "Click 2 or more, in order".to_owned()
                                } else {
                                    names
                                        .0
                                        .iter()
                                        .enumerate()
                                        .map(|(i, n)| format!("{}. {n}", i + 1))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                };
                                let colour = if names.0.is_empty() { muted } else { text };
                                ui.label(egui::RichText::new(list).color(colour))
                                    .on_hover_text("Click a picked profile again to unpick it");
                                ui.end_row();
                            } else {
                                widgets::field_label(ui, look, "Profile");
                                let profile = names.0.first().cloned();
                                if ui
                                    .selectable_label(
                                        tool.picking == SweepPick::Profile,
                                        egui::RichText::new(
                                            profile.unwrap_or_else(|| "Click a profile".into()),
                                        ),
                                    )
                                    .on_hover_text("Click here, then a closed profile")
                                    .clicked()
                                {
                                    tool.picking = SweepPick::Profile;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Path");
                                let path = match (&names.1, tool.path.len()) {
                                    (Some(s), n) if n > 0 => format!(
                                        "{n} curve{} of {s}",
                                        if n == 1 { "" } else { "s" }
                                    ),
                                    _ => "Click a line or arc".into(),
                                };
                                if ui
                                    .selectable_label(tool.picking == SweepPick::Path, path)
                                    .on_hover_text(
                                        "Click here, then a line or arc: the chain it is part of is picked",
                                    )
                                    .clicked()
                                {
                                    tool.picking = SweepPick::Path;
                                }
                                ui.end_row();
                            }
                            widgets::field_label(ui, look, "Operation");
                            egui::ComboBox::from_id_salt("sweep operation")
                                .selected_text(operation_name(tool.operation))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for o in [
                                        Operation::NewBody,
                                        Operation::Join,
                                        Operation::Cut,
                                        Operation::Intersect,
                                    ] {
                                        ui.selectable_value(
                                            &mut tool.operation,
                                            o,
                                            operation_name(o),
                                        );
                                    }
                                });
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
        if ok {
            self.sweep_ok();
        }
        if cancel {
            self.solid.sweep = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Point3;

    /// Where `p` is on an 800 × 600 viewport after `view.home`.
    fn on_screen(app: &ChamferaApp, p: Point3) -> (f64, f64) {
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
    fn picking_a_profile_and_a_path_and_ok_sweeps_it() {
        let mut app = ChamferaApp::headless();
        // A square 4 × 4 on the YZ plane, swept along 30 mm of the x axis drawn on XY.
        for (id, p) in [
            ("sketch.create", json!({ "plane": "yz" })),
            (
                "sketch.rectangle",
                json!({ "x1": -2, "y1": -2, "x2": 2, "y2": 2 }),
            ),
            ("sketch.finish", json!({})),
            ("sketch.create", json!({ "plane": "xy" })),
            (
                "sketch.line",
                json!({ "x1": 0, "y1": 0, "x2": 30, "y2": 0 }),
            ),
            ("sketch.finish", json!({})),
            ("view.home", json!({ "width": 800, "height": 600 })),
        ] {
            assert!(app.run(id, p).is_some(), "{id}");
        }
        app.open_sweep(false);
        // Inside the square, off the path line: (0, 1, 1) on the YZ plane.
        let (x, y) = on_screen(&app, Point3::new(0.0, 1.0, 1.0));
        app.sweep_click(x, y, 800.0, 600.0);
        let tool = app.solid.sweep.as_ref().unwrap();
        assert_eq!(tool.profiles.len(), 1);
        assert_eq!(tool.picking, SweepPick::Path);
        let (x, y) = on_screen(&app, Point3::new(20.0, 0.0, 0.0));
        app.sweep_click(x, y, 800.0, 600.0);
        let tool = app.solid.sweep.as_ref().unwrap();
        assert_eq!((tool.path_sketch, tool.path.len()), (Some(2), 1));
        assert!(app.sweep_overlay().is_some_and(|o| !o.lines.is_empty()));
        assert!(app.sweep_ok());
        // 4 · 4 · 30.
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - 480.0).abs() < 1e-9, "{v}");
        assert!(app.solid.sweep.is_none());
    }

    #[test]
    fn a_loft_needs_two_profiles_and_builds_through_them_in_order() {
        let mut tool = SweepTool::new(true);
        assert!(!tool.ready());
        tool.profiles = vec![(1, Point2::ORIGIN), (2, Point2::ORIGIN)];
        assert!(tool.ready());
        let (id, p) = tool.command();
        assert_eq!(id, "solid.loft");
        assert_eq!(p["sections"][1]["sketch"], 2);
        let mut app = ChamferaApp::headless();
        for (offset, half) in [(0.0, 10.0), (10.0, 5.0)] {
            app.run("sketch.create", json!({ "plane": "xy", "offset": offset }));
            app.run(
                "sketch.rectangle",
                json!({ "x1": -half, "y1": -half, "x2": half, "y2": half }),
            );
            app.run("sketch.finish", json!({}));
        }
        app.solid.sweep = Some(tool);
        assert!(app.sweep_ok());
        // The frustum 10/3 (400 + 100 + 200).
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - 7000.0 / 3.0).abs() < 1e-9, "{v}");
    }
}

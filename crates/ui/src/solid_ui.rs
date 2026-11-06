//! Solid modelling in the UI: the SOLID ribbon and the Extrude and Revolve tools (pick
//! profiles, set a distance or an axis). Acts only through `solid.*` and `sketch.*`
//! commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{ExtrudeDirection, Operation};
use chamfera_geom::Point2;
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::json;

/// The Extrude or Revolve tool while it is open: both pick profiles the same way.
pub struct ExtrudeTool {
    /// Revolve instead of extrude.
    pub revolve: bool,
    sketch: Option<u32>,
    /// A point inside each picked profile.
    seeds: Vec<Point2>,
    pub distance: String,
    direction: ExtrudeDirection,
    /// Revolve: the sketch line to turn around, and the angle expression.
    axis: Option<u32>,
    angle: String,
    /// New body, join, cut or intersect.
    operation: Operation,
}

impl Default for ExtrudeTool {
    fn default() -> Self {
        ExtrudeTool {
            revolve: false,
            sketch: None,
            seeds: Vec::new(),
            distance: "10 mm".into(),
            direction: ExtrudeDirection::Normal,
            operation: Operation::NewBody,
            axis: None,
            angle: "360 deg".into(),
        }
    }
}

#[derive(Default)]
pub struct SolidUi {
    pub extrude: Option<ExtrudeTool>,
    /// The Fillet or Chamfer tool.
    pub blend: Option<crate::blend_ui::BlendTool>,
    /// Waiting for a click on a flat face to sketch on.
    pub pick_face: bool,
    /// `--extrude`: pick the profile under the viewport's centre on the first frame.
    pub pick_centre: bool,
    /// The Measure or Mass Properties tool (`inspect_ui.rs`).
    pub inspect: Option<crate::inspect_ui::InspectTool>,
}

impl ChamferaApp {
    /// The SOLID ribbon: CREATE (sketch, extrude, revolve), MODIFY and INSERT groups.
    pub(crate) fn solid_toolbar(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        let has_profiles = self
            .session
            .doc
            .sketches
            .iter()
            .any(|s| !s.sketch.profiles().is_empty());
        let has_solids = !self.session.solids().is_empty();
        let tool_free = self.solid.extrude.is_none()
            && self.solid.blend.is_none()
            && self.solid.inspect.is_none();
        let extruding = self.solid.extrude.as_ref().is_some_and(|t| !t.revolve);
        let revolving = self.solid.extrude.as_ref().is_some_and(|t| t.revolve);
        let mut plane: Option<(&str, &str)> = None;
        let mut on_face = false;
        let mut open_tool: Option<bool> = None;
        let mut sketch_button: Option<egui::Response> = None;
        let (_, menu) = widgets::ribbon_group(ui, look, "CREATE", true, |ui| {
            let r = widgets::tool_button(
                ui,
                look,
                Icon::SketchCreate,
                label("sketch.create"),
                true,
                false,
                true,
            )
            .on_hover_text("Create Sketch\nStart a 2D sketch on an origin plane or a flat face");
            sketch_button = Some(r);
            if widgets::tool_button(
                ui,
                look,
                Icon::Extrude,
                label("solid.extrude"),
                has_profiles && (tool_free || extruding),
                extruding,
                false,
            )
            .on_hover_text(if has_profiles {
                "Extrude\nPull closed sketch profiles straight out into a solid"
            } else {
                "Extrude\nDraw a closed profile in a sketch first"
            })
            .clicked()
                && tool_free
            {
                open_tool = Some(false);
            }
            if widgets::tool_button(
                ui,
                look,
                Icon::Revolve,
                label("solid.revolve"),
                has_profiles && (tool_free || revolving),
                revolving,
                false,
            )
            .on_hover_text(if has_profiles {
                "Revolve\nTurn closed sketch profiles around a line of the same sketch"
            } else {
                "Revolve\nDraw a closed profile and a line to turn it around first"
            })
            .clicked()
                && tool_free
            {
                open_tool = Some(true);
            }
        });
        // Create Sketch opens the list of planes under its button.
        if let Some(r) = &sketch_button {
            egui::Popup::menu(r).show(|ui| {
                widgets::caption(ui, look, "SKETCH ON");
                for (p, text, view) in crate::sketch_ui::PLANES {
                    if widgets::menu_item(ui, look, Icon::Plane, text, true).clicked() {
                        ui.close();
                        plane = Some((p, view));
                    }
                }
                ui.separator();
                if widgets::menu_item(ui, look, Icon::Body, "A flat face…", has_solids).clicked()
                {
                    ui.close();
                    on_face = true;
                }
            });
        }
        egui::Popup::menu(&menu).show(|ui| {
            for (p, text, view) in crate::sketch_ui::PLANES {
                let item = format!("{} on {text}", label("sketch.create"));
                if widgets::menu_item(ui, look, Icon::SketchCreate, &item, true).clicked() {
                    ui.close();
                    plane = Some((p, view));
                }
            }
            if widgets::menu_item(
                ui,
                look,
                Icon::SketchCreate,
                label("sketch.create_on_face"),
                has_solids,
            )
            .clicked()
            {
                ui.close();
                on_face = true;
            }
            ui.separator();
            if widgets::menu_item(
                ui,
                look,
                Icon::Extrude,
                label("solid.extrude"),
                has_profiles && tool_free,
            )
            .clicked()
            {
                ui.close();
                open_tool = Some(false);
            }
            if widgets::menu_item(
                ui,
                look,
                Icon::Revolve,
                label("solid.revolve"),
                has_profiles && tool_free,
            )
            .clicked()
            {
                ui.close();
                open_tool = Some(true);
            }
        });
        if let Some((p, view)) = plane {
            self.create_sketch(p, view);
        }
        if on_face {
            self.solid.pick_face = true;
            self.message = Some(("Click a flat face of a solid to sketch on".into(), false));
        }
        match open_tool {
            Some(false) => {
                self.solid.extrude = Some(ExtrudeTool::default());
                self.message = Some(("Click inside the profiles to extrude".into(), false));
            }
            Some(true) => {
                self.solid.extrude = Some(ExtrudeTool {
                    revolve: true,
                    ..ExtrudeTool::default()
                });
                self.message = Some((
                    "Click inside the profiles to revolve, then choose the axis line".into(),
                    false,
                ));
            }
            None => {}
        }
        let look = self.look();
        widgets::ribbon_divider(ui, look);
        let mut parameters = false;
        let mut open_blend: Option<bool> = None;
        let filleting = self.solid.blend.as_ref().is_some_and(|t| !t.chamfer);
        let chamfering = self.solid.blend.as_ref().is_some_and(|t| t.chamfer);
        widgets::ribbon_group(ui, look, "MODIFY", false, |ui| {
            for (chamfer, on, text, tip) in [
                (
                    false,
                    filleting,
                    label("solid.fillet"),
                    "Fillet\nRound edges of a solid with a constant radius",
                ),
                (
                    true,
                    chamfering,
                    label("solid.chamfer"),
                    "Chamfer\nBevel edges of a solid by an equal distance",
                ),
            ] {
                if widgets::tool_button(
                    ui,
                    look,
                    if chamfer { Icon::Chamfer } else { Icon::Fillet },
                    text,
                    has_solids && (tool_free || on),
                    on,
                    false,
                )
                .on_hover_text(if has_solids {
                    tip
                } else {
                    "Build a solid first"
                })
                .clicked()
                    && tool_free
                {
                    open_blend = Some(chamfer);
                }
            }
            if widgets::tool_button(
                ui,
                look,
                Icon::Parameters,
                "Parameters",
                true,
                self.params.open,
                false,
            )
            .on_hover_text(
                "Parameters\nNamed values and formulas that dimensions and features follow",
            )
            .clicked()
            {
                parameters = true;
            }
        });
        if parameters {
            self.params.open = !self.params.open;
        }
        if let Some(chamfer) = open_blend {
            self.solid.blend = Some(crate::blend_ui::BlendTool::new(chamfer));
            self.message = Some(("Click the edges to round or bevel".into(), false));
        }
        let look = self.look();
        widgets::ribbon_divider(ui, look);
        let modelling_free = self.solid.extrude.is_none() && self.solid.blend.is_none();
        self.inspect_group(ui, has_solids, modelling_free);
        let look = self.look();
        widgets::ribbon_divider(ui, look);
        let mut import = false;
        let mut sample = false;
        widgets::ribbon_group(ui, look, "INSERT", false, |ui| {
            if widgets::tool_button(
                ui,
                look,
                Icon::ImportMesh,
                label("body.import_stl"),
                true,
                false,
                false,
            )
            .on_hover_text(format!(
                "{}\nAdd a triangle mesh from an STL file, for reference",
                label("body.import_stl")
            ))
            .clicked()
            {
                import = true;
            }
            if widgets::tool_button(
                ui,
                look,
                Icon::Body,
                label("body.sample"),
                true,
                false,
                false,
            )
            .on_hover_text(format!(
                "{}\nA small bracket mesh to try the viewer on",
                label("body.sample")
            ))
            .clicked()
            {
                sample = true;
            }
        });
        if import {
            self.import_dialog();
        }
        if sample {
            self.run("body.sample", json!({}));
            self.fit_pending = true;
        }
    }

    /// A click while waiting for a face: creates a sketch on it and looks straight at it.
    pub(crate) fn face_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.solid.pick_face = false;
        let Some(r) = self.run_sketch(
            "sketch.create_on_face",
            json!({ "x": x, "y": y, "width": w, "height": h }),
        ) else {
            return;
        };
        self.run("view.normal_to", r["normal"].clone());
        self.sketch.tool = Some(crate::sketch_ui::Tool::Line);
        self.message = Some((
            format!(
                "Sketching on the {} face",
                r["face"].as_str().unwrap_or("picked")
            ),
            false,
        ));
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
        if tool.sketch.is_none() && tool.seeds.is_empty() {
            // A sketch on a solid's face usually adds to (or cuts) that solid.
            let on_face = self
                .session
                .doc
                .sketch(sketch)
                .is_ok_and(|f| f.attached.is_some());
            tool.operation = if on_face {
                Operation::Join
            } else {
                Operation::NewBody
            };
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

    /// Where a tool dialog opens: the viewport's right side, under the view cube, so it
    /// does not cover the model in the middle.
    pub(crate) fn tool_dialog_pos(&self, width: f32) -> egui::Pos2 {
        let r = self.viewport_rect;
        egui::pos2(
            (r.max.x - width - widgets::S5).max(r.min.x + widgets::S5),
            r.min.y + crate::viewcube::CUBE_AREA + widgets::S3,
        )
    }

    /// The Extrude (or Revolve) dialog.
    pub(crate) fn extrude_dialog(&mut self, ctx: &egui::Context) {
        // The lines of the picked sketch, for the revolve axis.
        let lines: Vec<(u32, String)> = self
            .solid
            .extrude
            .as_ref()
            .and_then(|t| t.sketch)
            .and_then(|s| self.session.doc.sketch(s).ok())
            .map(|f| {
                f.sketch
                    .curves()
                    .filter(|(_, c)| c.is_line())
                    .map(|(id, c)| {
                        let kind = if c.is_construction() {
                            "Construction line"
                        } else {
                            "Line"
                        };
                        (id.0, format!("{kind} {}", id.0))
                    })
                    .collect()
            })
            .unwrap_or_default();
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.extrude.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let (title, icon) = if tool.revolve {
            (label("solid.revolve"), Icon::Revolve)
        } else {
            (label("solid.extrude"), Icon::Extrude)
        };
        let ready = !tool.seeds.is_empty() && (!tool.revolve || tool.axis.is_some());
        egui::Window::new(title)
            .id(egui::Id::unique("extrude dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(200.0)
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
                    egui::Grid::new("extrude grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Profiles");
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::Vec2::splat(16.0),
                                    egui::Sense::hover(),
                                );
                                let picked = !tool.seeds.is_empty();
                                let (b, a) = look.icon_colours(picked);
                                look.icons.paint(ui.painter(), Icon::Sketch, r, b, a);
                                let text = match tool.seeds.len() {
                                    0 => "Click inside a profile".to_owned(),
                                    1 => "1 selected".to_owned(),
                                    n => format!("{n} selected"),
                                };
                                let colour = if picked {
                                    look.c(|t| t.text)
                                } else {
                                    look.c(|t| t.text_muted)
                                };
                                ui.label(egui::RichText::new(text).color(colour));
                            })
                            .response
                            .on_hover_text("Click a picked profile again to unpick it");
                            ui.end_row();
                            widgets::field_label(ui, look, "Operation");
                            egui::ComboBox::from_id_salt("feature operation")
                                .selected_text(operation_name(tool.operation))
                                .width(180.0)
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
                            if tool.revolve {
                                widgets::field_label(ui, look, "Axis");
                                let current = lines
                                    .iter()
                                    .find(|(id, _)| Some(*id) == tool.axis)
                                    .map_or("Choose a line", |(_, n)| n.as_str());
                                egui::ComboBox::from_id_salt("revolve axis")
                                    .selected_text(current)
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        for (id, name) in &lines {
                                            ui.selectable_value(&mut tool.axis, Some(*id), name);
                                        }
                                    });
                                ui.end_row();
                                widgets::field_label(ui, look, "Angle");
                                ui.add_sized(
                                    [180.0, 26.0],
                                    egui::TextEdit::singleline(&mut tool.angle)
                                        .hint_text("360 deg")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                ui.end_row();
                                return;
                            }
                            widgets::field_label(ui, look, "Distance");
                            let edit = ui.add_sized(
                                [180.0, 26.0],
                                egui::TextEdit::singleline(&mut tool.distance)
                                    .hint_text("10 mm or thickness * 2")
                                    .align(egui::Align2::LEFT_CENTER),
                            );
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                ok = true;
                            }
                            ui.end_row();
                            widgets::field_label(ui, look, "Direction");
                            egui::ComboBox::from_id_salt("extrude direction")
                                .selected_text(direction_name(tool.direction))
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for d in [
                                        ExtrudeDirection::Normal,
                                        ExtrudeDirection::Reversed,
                                        ExtrudeDirection::Symmetric,
                                    ] {
                                        ui.selectable_value(
                                            &mut tool.direction,
                                            d,
                                            direction_name(d),
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
        if ok && tool.revolve && !tool.seeds.is_empty() && tool.axis.is_some() {
            let params = json!({
                "sketch": tool.sketch,
                "profiles": tool.seeds.iter().map(|p| json!({ "x": p.x, "y": p.y })).collect::<Vec<_>>(),
                "axis": tool.axis,
                "angle": tool.angle.trim(),
                "operation": tool.operation.key(),
            });
            if let Some(r) = self.run("solid.revolve", params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {v:.2} mm³",
                        r["name"].as_str().unwrap_or("Revolve")
                    ),
                    false,
                ));
                self.solid.extrude = None;
            }
        } else if ok && !tool.revolve && !tool.seeds.is_empty() {
            let params = json!({
                "sketch": tool.sketch,
                "profiles": tool.seeds.iter().map(|p| json!({ "x": p.x, "y": p.y })).collect::<Vec<_>>(),
                "distance": tool.distance.trim(),
                "direction": direction_key(tool.direction),
                "operation": tool.operation.key(),
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
}

pub(crate) fn operation_name(o: Operation) -> &'static str {
    match o {
        Operation::NewBody => "New body",
        Operation::Join => "Join",
        Operation::Cut => "Cut",
        Operation::Intersect => "Intersect",
    }
}

pub(crate) fn direction_name(d: ExtrudeDirection) -> &'static str {
    match d {
        ExtrudeDirection::Normal => "One side",
        ExtrudeDirection::Reversed => "Flipped",
        ExtrudeDirection::Symmetric => "Symmetric",
    }
}

pub(crate) fn direction_key(d: ExtrudeDirection) -> &'static str {
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

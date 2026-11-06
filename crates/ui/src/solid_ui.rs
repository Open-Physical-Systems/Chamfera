//! Solid modelling in the UI: the SOLID ribbon and the Extrude and Revolve tools (pick
//! profiles, set a distance or an axis). Acts only through `solid.*` and `sketch.*`
//! commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{Extrude, ExtrudeDirection, ExtrudeExtent, ExtrudeTo, FaceRef, Operation};
use chamfera_geom::Point2;
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::{Value, json};

/// The extrude options as the Extrude tool and the Edit Feature dialog edit them.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtrudeForm {
    pub extent: ExtrudeExtent,
    pub direction: ExtrudeDirection,
    pub distance: String,
    /// The second side's distance (two sides).
    pub distance2: String,
    pub start_offset: String,
    pub taper: String,
    /// What a to-object extrude goes up to.
    pub to: Option<ExtrudeTo>,
    /// Waiting for a click on the face to extrude up to.
    pub picking_to: bool,
}

impl Default for ExtrudeForm {
    fn default() -> Self {
        ExtrudeForm {
            extent: ExtrudeExtent::Distance,
            direction: ExtrudeDirection::Normal,
            distance: "10 mm".into(),
            distance2: "5 mm".into(),
            start_offset: "0 mm".into(),
            taper: "0 deg".into(),
            to: None,
            picking_to: false,
        }
    }
}

use widgets::FIELD;

impl ExtrudeForm {
    /// The form for an extrude on the timeline.
    pub fn from_extrude(e: &Extrude) -> ExtrudeForm {
        let d = ExtrudeForm::default();
        ExtrudeForm {
            extent: e.extent,
            direction: e.direction,
            distance: e.distance.clone(),
            distance2: e.distance2.clone().unwrap_or(d.distance2),
            start_offset: e.start_offset.clone().unwrap_or(d.start_offset),
            taper: e.taper.clone().unwrap_or(d.taper),
            to: e.to.clone(),
            picking_to: false,
        }
    }

    /// The options as `solid.extrude` and `feature.edit` take them.
    pub fn params(&self) -> Value {
        let or_zero = |s: &str, zero: &str| {
            let t = s.trim();
            if t.is_empty() {
                zero.to_owned()
            } else {
                t.to_owned()
            }
        };
        let mut p = json!({
            "extent": self.extent.key(),
            "direction": self.direction.key(),
            "start_offset": or_zero(&self.start_offset, "0"),
            "taper": or_zero(&self.taper, "0"),
        });
        if self.extent == ExtrudeExtent::Distance {
            p["distance"] = json!(self.distance.trim());
            if self.direction == ExtrudeDirection::TwoSides {
                p["distance2"] = json!(self.distance2.trim());
            }
        }
        if self.extent == ExtrudeExtent::ToObject {
            match &self.to {
                Some(ExtrudeTo::Face(r)) => {
                    p["to"] = json!({ "feature": r.feature, "face": r.face });
                }
                Some(ExtrudeTo::Plane(f)) => {
                    let (o, n) = (f.origin(), f.z());
                    p["to"] = json!({ "plane": {
                        "origin": { "x": o.x, "y": o.y, "z": o.z },
                        "normal": { "x": n.x, "y": n.y, "z": n.z },
                    }});
                }
                None => {}
            }
        }
        p
    }

    fn to_label(&self) -> String {
        match &self.to {
            _ if self.picking_to => "Click a flat face…".to_owned(),
            Some(ExtrudeTo::Face(r)) => format!("Face {} of feature {}", r.face, r.feature),
            Some(ExtrudeTo::Plane(_)) => "A plane".to_owned(),
            None => "Pick a face".to_owned(),
        }
    }

    /// The option rows of a two-column dialog grid. True when Enter was pressed in a field.
    pub fn rows(&mut self, ui: &mut egui::Ui, look: Look) -> bool {
        let mut enter = false;
        let mut field = |ui: &mut egui::Ui, text: &mut String, hint: &str, tip: &str| {
            let r = ui
                .add_sized(
                    FIELD,
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .align(egui::Align2::LEFT_CENTER),
                )
                .on_hover_text(tip);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                enter = true;
            }
        };
        widgets::field_label(ui, look, "Extent");
        egui::ComboBox::from_id_salt("extrude extent")
            .selected_text(extent_name(self.extent))
            .width(FIELD[0])
            .show_ui(ui, |ui| {
                for x in [
                    ExtrudeExtent::Distance,
                    ExtrudeExtent::ThroughAll,
                    ExtrudeExtent::ToObject,
                ] {
                    ui.selectable_value(&mut self.extent, x, extent_name(x));
                }
            });
        ui.end_row();
        if self.extent != ExtrudeExtent::ToObject {
            widgets::field_label(ui, look, "Direction");
            if self.extent == ExtrudeExtent::ThroughAll
                && self.direction == ExtrudeDirection::TwoSides
            {
                self.direction = ExtrudeDirection::Symmetric;
            }
            let mut choices = vec![
                ExtrudeDirection::Normal,
                ExtrudeDirection::Reversed,
                ExtrudeDirection::Symmetric,
            ];
            if self.extent == ExtrudeExtent::Distance {
                choices.push(ExtrudeDirection::TwoSides);
            }
            egui::ComboBox::from_id_salt("extrude direction")
                .selected_text(direction_name(self.direction))
                .width(FIELD[0])
                .show_ui(ui, |ui| {
                    for d in choices {
                        ui.selectable_value(&mut self.direction, d, direction_name(d));
                    }
                });
            ui.end_row();
        }
        match self.extent {
            ExtrudeExtent::Distance => {
                let two = self.direction == ExtrudeDirection::TwoSides;
                widgets::field_label(ui, look, if two { "Side 1" } else { "Distance" });
                field(
                    ui,
                    &mut self.distance,
                    "10 mm or thickness * 2",
                    "How far to extrude",
                );
                ui.end_row();
                if two {
                    widgets::field_label(ui, look, "Side 2");
                    field(
                        ui,
                        &mut self.distance2,
                        "5 mm",
                        "How far the other side goes, against the normal",
                    );
                    ui.end_row();
                }
            }
            ExtrudeExtent::ThroughAll => {}
            ExtrudeExtent::ToObject => {
                widgets::field_label(ui, look, "Up to");
                let picked = self.to.is_some() && !self.picking_to;
                let text = egui::RichText::new(self.to_label()).color(if picked {
                    look.c(|t| t.text)
                } else {
                    look.c(|t| t.text_muted)
                });
                if ui
                    .add_sized(FIELD, egui::Button::new(text))
                    .on_hover_text(
                        "Click, then click a flat face parallel to the profile; the extrude follows it",
                    )
                    .clicked()
                {
                    self.picking_to = !self.picking_to;
                }
                ui.end_row();
            }
        }
        widgets::field_label(ui, look, "Start offset");
        field(
            ui,
            &mut self.start_offset,
            "0 mm",
            "Start this far from the sketch plane, along its normal",
        );
        ui.end_row();
        widgets::field_label(ui, look, "Taper");
        field(
            ui,
            &mut self.taper,
            "0 deg",
            "Draft angle of the sides: positive narrows away from the start",
        );
        ui.end_row();
        enter
    }
}

/// A short description of an extrude for the timeline.
pub(crate) fn extrude_summary(e: &Extrude) -> String {
    let mut parts = vec![match e.extent {
        ExtrudeExtent::Distance => match (&e.direction, &e.distance2) {
            (ExtrudeDirection::TwoSides, Some(d2)) => format!("{} and {d2}", e.distance),
            _ => format!(
                "{} · {}",
                e.distance,
                direction_name(e.direction).to_lowercase()
            ),
        },
        ExtrudeExtent::ThroughAll => format!(
            "through all · {}",
            direction_name(e.direction).to_lowercase()
        ),
        ExtrudeExtent::ToObject => match &e.to {
            Some(ExtrudeTo::Face(r)) => format!("up to face {} of feature {}", r.face, r.feature),
            _ => "up to a plane".to_owned(),
        },
    }];
    if let Some(o) = &e.start_offset {
        parts.push(format!("offset {o}"));
    }
    if let Some(t) = &e.taper {
        parts.push(format!("taper {t}"));
    }
    parts.join(" · ")
}

/// The Extrude or Revolve tool while it is open: both pick profiles the same way.
pub struct ExtrudeTool {
    /// Revolve instead of extrude.
    pub revolve: bool,
    sketch: Option<u32>,
    /// A point inside each picked profile.
    seeds: Vec<Point2>,
    /// The extrude's options.
    pub form: ExtrudeForm,
    /// Revolve: the sketch line to turn around, and the angle expression.
    axis: Option<u32>,
    angle: String,
    /// New body, join, cut or intersect.
    operation: Operation,
    /// The parameters the preview shows, and what it said: the volume or why not.
    preview_for: Option<Value>,
    preview: Result<f64, String>,
}

impl Default for ExtrudeTool {
    fn default() -> Self {
        ExtrudeTool {
            revolve: false,
            sketch: None,
            seeds: Vec::new(),
            form: ExtrudeForm::default(),
            operation: Operation::NewBody,
            axis: None,
            angle: "360 deg".into(),
            preview_for: None,
            preview: Err(String::new()),
        }
    }
}

impl ExtrudeTool {
    /// Sets the options given as `solid.extrude` parameters (for `--extrude-with`).
    pub fn apply(&mut self, p: &Value) {
        let text = |k: &str, unit: &str| match p.get(k) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(format!("{n} {unit}")),
            _ => None,
        };
        let f = &mut self.form;
        if let Some(x) = p
            .get("extent")
            .and_then(Value::as_str)
            .and_then(ExtrudeExtent::parse)
        {
            f.extent = x;
        }
        if let Some(d) = p
            .get("direction")
            .and_then(Value::as_str)
            .and_then(ExtrudeDirection::parse)
        {
            f.direction = d;
        }
        if let Some(o) = p
            .get("operation")
            .and_then(Value::as_str)
            .and_then(Operation::parse)
        {
            self.operation = o;
        }
        for (k, unit, slot) in [
            ("distance", "mm", &mut f.distance),
            ("distance2", "mm", &mut f.distance2),
            ("start_offset", "mm", &mut f.start_offset),
            ("taper", "deg", &mut f.taper),
        ] {
            if let Some(v) = text(k, unit) {
                *slot = v;
            }
        }
        if let (Some(feature), Some(face)) = (
            p["to"]["feature"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok()),
            p["to"]["face"].as_str(),
        ) {
            f.to = Some(ExtrudeTo::Face(FaceRef {
                feature,
                face: face.to_owned(),
            }));
        }
    }

    /// The `solid.extrude` parameters for what the dialog shows, once a profile is picked.
    fn params(&self) -> Option<Value> {
        if self.revolve || self.seeds.is_empty() {
            return None;
        }
        let mut p = self.form.params();
        p["sketch"] = json!(self.sketch);
        p["profiles"] = json!(
            self.seeds
                .iter()
                .map(|p| json!({ "x": p.x, "y": p.y }))
                .collect::<Vec<_>>()
        );
        p["operation"] = json!(self.operation.key());
        Some(p)
    }
}

#[derive(Default)]
pub struct SolidUi {
    pub extrude: Option<ExtrudeTool>,
    /// The Fillet or Chamfer tool.
    pub blend: Option<crate::blend_ui::BlendTool>,
    /// The Hole tool.
    pub hole: Option<crate::hole_ui::HoleTool>,
    /// The Mirror, Move or pattern tool.
    pub transform: Option<crate::transform_ui::TransformTool>,
    /// The Shell tool.
    pub shell: Option<crate::shell_ui::ShellTool>,
    /// The Press Pull, Offset Face or Delete Face tool (`press_pull_ui.rs`).
    pub press_pull: Option<crate::press_pull_ui::PressPullTool>,
    /// Combine, Split Body, Split Face or Thicken (`body_tools_ui.rs`).
    pub body_tool: Option<crate::body_tools_ui::BodyTool>,
    /// A surface tool (`surface_ui.rs`).
    pub surface: Option<crate::surface_ui::SurfaceTool>,
    /// The Sweep or Loft tool.
    pub sweep: Option<crate::sweep_ui::SweepTool>,
    /// Waiting for a click on a flat face to sketch on.
    pub pick_face: bool,
    /// `--extrude`: pick the profile under the viewport's centre on the first frame.
    pub pick_centre: bool,
    /// The Measure or Mass Properties tool (`inspect_ui.rs`).
    pub inspect: Option<crate::inspect_ui::InspectTool>,
}

impl ChamferaApp {
    /// Waits for a click on a flat face to sketch on (Create Sketch > A flat face).
    pub(crate) fn start_sketch_on_face(&mut self) {
        self.solid.pick_face = true;
        self.message = Some(("Click a flat face of a solid to sketch on".into(), false));
    }

    /// Opens the Extrude tool, or the Revolve tool, as their ribbon buttons do.
    pub(crate) fn open_extrude(&mut self, revolve: bool) {
        if revolve {
            self.solid.extrude = Some(ExtrudeTool {
                revolve: true,
                ..ExtrudeTool::default()
            });
            self.message = Some((
                "Click inside the profiles to revolve, then choose the axis line".into(),
                false,
            ));
        } else {
            self.solid.extrude = Some(ExtrudeTool::default());
            self.message = Some(("Click inside the profiles to extrude".into(), false));
        }
    }

    /// Opens the Fillet tool, or the Chamfer tool, as their ribbon buttons do.
    pub(crate) fn open_blend(&mut self, chamfer: bool) {
        self.solid.blend = Some(crate::blend_ui::BlendTool::new(chamfer));
        self.message = Some(("Click the edges to round or bevel".into(), false));
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
        let units = self.session.doc.units;
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
                            if tool.revolve {
                                widgets::field_label(ui, look, "Axis");
                                let current = lines
                                    .iter()
                                    .find(|(id, _)| Some(*id) == tool.axis)
                                    .map_or("Choose a line", |(_, n)| n.as_str());
                                egui::ComboBox::from_id_salt("revolve axis")
                                    .selected_text(current)
                                    .width(widgets::FIELD_WIDTH)
                                    .show_ui(ui, |ui| {
                                        for (id, name) in &lines {
                                            ui.selectable_value(&mut tool.axis, Some(*id), name);
                                        }
                                    });
                                ui.end_row();
                                widgets::field_label(ui, look, "Angle");
                                ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut tool.angle)
                                        .hint_text("360 deg")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                ui.end_row();
                                return;
                            }
                            if tool.form.rows(ui, look) {
                                ok = true;
                            }
                        });
                    if !tool.revolve && !tool.seeds.is_empty() {
                        // What the translucent preview shows, or why there is none.
                        let (text, colour) = match &tool.preview {
                            Ok(v) => (
                                format!("Preview · volume {}", units.volume(*v)),
                                look.c(|t| t.text_muted),
                            ),
                            Err(e) if e.is_empty() => (String::new(), look.c(|t| t.text_muted)),
                            Err(e) => (e.clone(), look.c(|t| t.error)),
                        };
                        if !text.is_empty() {
                            ui.add_space(widgets::S2);
                            ui.add(
                                egui::Label::new(egui::RichText::new(text).color(colour)).wrap(),
                            );
                        }
                    }
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
                        "{} · volume {}",
                        r["name"].as_str().unwrap_or("Revolve"),
                        self.units().volume(v),
                    ),
                    false,
                ));
                self.solid.extrude = None;
            }
        } else if ok
            && let Some(params) = tool.params()
            && let Some(r) = self.run("solid.extrude", params)
        {
            let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
            self.message = Some((
                format!(
                    "{} · volume {}",
                    r["name"].as_str().unwrap_or("Extrude"),
                    self.units().volume(v),
                ),
                false,
            ));
            self.solid.extrude = None;
        }
        if cancel {
            self.solid.extrude = None;
        }
        if self.update_extrude_preview() {
            ctx.request_repaint();
        }
    }

    /// Keeps the translucent preview in step with the Extrude dialog: built again through
    /// `solid.extrude_preview` when its values change, removed when the tool closes. True
    /// when the preview changed.
    pub(crate) fn update_extrude_preview(&mut self) -> bool {
        let wanted = self.solid.extrude.as_ref().and_then(ExtrudeTool::params);
        let Some(params) = wanted else {
            let had = !self.session.scene.previews.is_empty();
            if had {
                let _ = self
                    .session
                    .execute("solid.extrude_preview", &json!({ "clear": true }));
                self.preview_version += 1;
            }
            if let Some(tool) = self.solid.extrude.as_mut() {
                tool.preview_for = None;
                tool.preview = Err(String::new());
            }
            return had;
        };
        let shown = self.solid.extrude.as_ref().is_some_and(|t| {
            t.preview_for.as_ref() == Some(&params)
                && (t.preview.is_err() || !self.session.scene.previews.is_empty())
        });
        if shown {
            return false;
        }
        self.preview_version += 1;
        let result = self.session.execute("solid.extrude_preview", &params);
        if let Some(tool) = self.solid.extrude.as_mut() {
            tool.preview = match result {
                Ok(r) => Ok(r["volume"].as_f64().unwrap_or(0.0)),
                Err(e) => Err(e.to_string()),
            };
            tool.preview_for = Some(params);
        }
        true
    }

    /// True while an extrude dialog waits for a click on the face to extrude up to.
    pub(crate) fn picking_extrude_target(&self) -> bool {
        self.solid
            .extrude
            .as_ref()
            .is_some_and(|t| t.form.picking_to)
            || self
                .timeline
                .editing
                .as_ref()
                .and_then(|e| e.extrude.as_ref())
                .is_some_and(|f| f.picking_to)
    }

    /// A click while an extrude dialog waits for its target face: the flat face there.
    pub(crate) fn extrude_target_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let face = self.session.face_at(x, y, w, h);
        let form = match self.solid.extrude.as_mut() {
            Some(t) if t.form.picking_to => Some(&mut t.form),
            _ => self
                .timeline
                .editing
                .as_mut()
                .and_then(|e| e.extrude.as_mut()),
        };
        let Some(form) = form else {
            return;
        };
        match face {
            Some((feature, face, _)) => {
                form.to = Some(ExtrudeTo::Face(FaceRef { feature, face }));
                form.picking_to = false;
            }
            None => {
                self.message = Some(("Click a flat face of a solid".into(), true));
            }
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
        ExtrudeDirection::TwoSides => "Two sides",
    }
}

pub(crate) fn extent_name(x: ExtrudeExtent) -> &'static str {
    match x {
        ExtrudeExtent::Distance => "Distance",
        ExtrudeExtent::ThroughAll => "Through all",
        ExtrudeExtent::ToObject => "To object",
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

    /// The dialog's options drive the live preview, OK extrudes what it showed, and the
    /// up-to face is picked by clicking it.
    #[test]
    fn the_extrude_options_preview_live_and_pick_their_face() {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 20 }),
        );
        app.finish_sketch();
        app.session.camera.target = chamfera_geom::Point3::new(20.0, 10.0, 0.0);
        let mut tool = ExtrudeTool::default();
        tool.apply(&json!({"distance": 10, "direction": "two_sides", "distance2": 4}));
        app.solid.extrude = Some(tool);
        app.extrude_click(400.0, 300.0, 800.0, 600.0);
        // 40 x 20 x (10 + 4) = 11200, shown translucent before OK.
        assert!(app.update_extrude_preview());
        let shown = app.solid.extrude.as_ref().unwrap().preview.clone();
        assert!(
            shown.as_ref().is_ok_and(|v| (*v - 11_200.0).abs() < 1e-6),
            "{shown:?}"
        );
        assert_eq!(app.session.scene.previews.len(), 1);
        assert!(app.session.solids().is_empty());
        // Unchanged values do not build it again; a refused taper says why and shows none.
        assert!(!app.update_extrude_preview());
        app.solid.extrude.as_mut().unwrap().form.taper = "60 deg".into();
        app.update_extrude_preview();
        assert!(app.solid.extrude.as_ref().unwrap().preview.is_err());
        assert!(app.session.scene.previews.is_empty());
        app.solid.extrude.as_mut().unwrap().form.taper = "0 deg".into();
        let params = app.solid.extrude.as_ref().unwrap().params().unwrap();
        let r = app.run("solid.extrude", params).unwrap();
        assert!((r["solids"][0]["volume"].as_f64().unwrap() - 11_200.0).abs() < 1e-6);
        // Closing the tool removes the preview.
        app.solid.extrude = None;
        app.update_extrude_preview();
        assert!(app.session.scene.previews.is_empty());
        // Up to the top face: clicking the solid in the top view picks it.
        let mut tool = ExtrudeTool::default();
        tool.form.extent = ExtrudeExtent::ToObject;
        tool.form.picking_to = true;
        app.solid.extrude = Some(tool);
        assert!(app.picking_extrude_target());
        app.extrude_target_click(400.0, 300.0, 800.0, 600.0);
        let form = &app.solid.extrude.as_ref().unwrap().form;
        assert!(!form.picking_to);
        assert_eq!(
            form.to,
            Some(ExtrudeTo::Face(FaceRef {
                feature: 2,
                face: "top".into()
            }))
        );
    }
}

//! Assembly tools in the UI: Drive Joint (drag a jointed part, or set the value), the
//! Motion Study panel (keys, play, pause, scrub), Interference (find overlaps) and
//! Explode (move components apart for show). Everything runs through `joint.*`,
//! `motion.*`, `inspect.interference` and `view.explode*` commands; playing and dragging
//! are display state, so they add no undo steps until a drag ends in `joint.set_value`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{JointKind, Unit};
use egui::{PointerButton, Rect, Response};
use serde_json::{Value, json};

/// A drag of a jointed part in the viewport.
pub struct DriveDrag {
    pub joint: u32,
    /// Where the drag began, in viewport pixels.
    pub from: (f64, f64),
    /// The joint's value then, in degrees or mm.
    pub start: f64,
    /// The value the drag shows now, in degrees or mm.
    pub value: f64,
}

/// The Drive Joint panel.
#[derive(Default)]
pub struct DriveTool {
    /// The joint the panel drives.
    pub joint: Option<u32>,
    /// The value as typed.
    pub value: String,
    pub drag: Option<DriveDrag>,
}

/// The Motion Study panel.
pub struct StudyPanel {
    /// The time shown, in seconds.
    pub time: f64,
    pub playing: bool,
    /// Playback runs back to the start at the end.
    pub looping: bool,
    /// The new key's joint, time and value as typed.
    pub key_joint: Option<u32>,
    pub key_time: String,
    pub key_value: String,
    /// The last check at the keyframes: (time, pairs).
    pub checked: Option<Vec<(f64, usize)>>,
}

impl Default for StudyPanel {
    fn default() -> Self {
        StudyPanel {
            time: 0.0,
            playing: false,
            looping: true,
            key_joint: None,
            key_time: "0".into(),
            key_value: String::new(),
            checked: None,
        }
    }
}

/// The Interference panel.
#[derive(Default)]
pub struct InterferencePanel {
    pub at_keys: bool,
    /// The last reply of `inspect.interference`.
    pub result: Option<Value>,
}

/// The Explode panel.
pub struct ExplodePanel {
    pub factor: f64,
    /// The manual offset for the chosen component, mm.
    pub offset: [f64; 3],
}

impl Default for ExplodePanel {
    fn default() -> Self {
        ExplodePanel {
            factor: 2.0,
            offset: [0.0; 3],
        }
    }
}

#[derive(Default)]
pub struct MotionUi {
    pub drive: Option<DriveTool>,
    pub study: Option<StudyPanel>,
    pub interference: Option<InterferencePanel>,
    pub explode: Option<ExplodePanel>,
}

/// A joint value as typed into the panels, from the stored value.
fn shown(kind: JointKind, v: f64) -> f64 {
    match kind.value_unit() {
        Some(Unit::Angle) => v.to_degrees(),
        _ => v,
    }
}

fn unit_text(kind: JointKind) -> &'static str {
    match kind.value_unit() {
        Some(Unit::Angle) => "°",
        _ => "mm",
    }
}

/// A typed number or expression as a parameter.
fn typed(v: &str) -> Value {
    let v = v.trim();
    match v.parse::<f64>() {
        Ok(n) => json!(n),
        Err(_) => json!(v),
    }
}

impl ChamferaApp {
    /// The joints that move something: (id, name, kind).
    fn movable_joints(&self) -> Vec<(u32, String, JointKind)> {
        self.session
            .doc
            .assembly
            .joints
            .iter()
            .filter(|j| j.kind != JointKind::Rigid)
            .map(|j| (j.id, j.name.clone(), j.kind))
            .collect()
    }

    fn joint_shown(&self, joint: u32) -> Option<(JointKind, f64)> {
        let kind = self.session.doc.assembly.joint(joint).ok()?.kind;
        Some((kind, shown(kind, self.session.shown_joint_value(joint)?)))
    }

    /// Drive Joint: opens or closes the panel.
    pub(crate) fn toggle_drive(&mut self) {
        if self.motion.drive.take().is_some() {
            return;
        }
        let joint = self.movable_joints().first().map(|j| j.0);
        let value = joint
            .and_then(|j| self.joint_shown(j))
            .map(|(_, v)| format!("{v:.2}"))
            .unwrap_or_default();
        self.motion.drive = Some(DriveTool {
            joint,
            value,
            drag: None,
        });
        self.message = Some((
            "Drive Joint: drag a jointed part to turn or slide it, or type a value".into(),
            false,
        ));
    }

    /// Motion Study: opens the panel, or closes it and shows the joints' own values.
    pub(crate) fn toggle_study(&mut self) {
        if self.motion.study.take().is_some() {
            self.run("motion.set_time", json!({ "t": null }));
            return;
        }
        let joint = self.movable_joints().first().map(|j| j.0);
        self.motion.study = Some(StudyPanel {
            key_joint: joint,
            ..StudyPanel::default()
        });
        self.run("motion.set_time", json!({ "t": 0.0 }));
    }

    /// Interference: checks now and opens the results, or closes them and the highlight.
    pub(crate) fn toggle_interference(&mut self) {
        if self.motion.interference.take().is_some() {
            self.run("inspect.interference", json!({ "clear": true }));
            return;
        }
        let result = self.run("inspect.interference", json!({}));
        self.motion.interference = Some(InterferencePanel {
            at_keys: false,
            result,
        });
    }

    /// Explode: shows the assembly exploded by the panel's factor, or collapses it.
    pub(crate) fn toggle_explode(&mut self) {
        if self.motion.explode.take().is_some() {
            self.run("view.explode_clear", json!({}));
            return;
        }
        let panel = ExplodePanel::default();
        self.run("view.explode", json!({ "factor": panel.factor }));
        self.motion.explode = Some(panel);
    }

    /// The component a drag on the pointer's body would move, and the joint that moves it.
    fn jointed_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<u32> {
        let bounds = self.session.scene.bounds();
        let ray = self
            .session
            .camera
            .ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.session.scene.pick(&ray)?;
        let component = self.session.component_of_body(hit.body)?;
        let driver = self.session.doc.assembly.driver_of(component)?;
        let kind = self.session.doc.assembly.joint(driver.joint).ok()?.kind;
        (kind != JointKind::Rigid).then_some(driver.joint)
    }

    /// Viewport input while Drive Joint is open: a left drag on a jointed part moves it
    /// within its joint (`joint.drag` while dragging, one `joint.set_value` at the end).
    /// True when the pointer was used.
    pub(crate) fn drive_input(&mut self, response: &Response, rect: Rect) -> bool {
        if self.motion.drive.is_none() {
            return false;
        }
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let local = |p: egui::Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        if response.drag_started_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            let Some(joint) = self.jointed_at(x, y, w, h) else {
                return false;
            };
            let Some((_, start)) = self.joint_shown(joint) else {
                return false;
            };
            if let Some(tool) = self.motion.drive.as_mut() {
                tool.joint = Some(joint);
                tool.drag = Some(DriveDrag {
                    joint,
                    from: (x, y),
                    start,
                    value: start,
                });
            }
            return true;
        }
        let Some((joint, from, start)) = self
            .motion
            .drive
            .as_ref()
            .and_then(|t| t.drag.as_ref())
            .map(|d| (d.joint, d.from, d.start))
        else {
            return false;
        };
        if response.dragged_by(PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            let reply = self.run(
                "joint.drag",
                json!({ "joint": joint, "from": {"x": from.0, "y": from.1},
                        "to": {"x": x, "y": y}, "width": w, "height": h, "start": start }),
            );
            if let (Some(v), Some(tool)) = (
                reply.and_then(|r| r["value"].as_f64()),
                self.motion.drive.as_mut(),
            ) {
                if let Some(d) = tool.drag.as_mut() {
                    d.value = v;
                }
                tool.value = format!("{v:.2}");
            }
            return true;
        }
        if response.drag_stopped() {
            let value = self
                .motion
                .drive
                .as_mut()
                .and_then(|t| t.drag.take())
                .map(|d| d.value);
            if let Some(v) = value
                && self
                    .run("joint.set_value", json!({ "joint": joint, "value": v }))
                    .is_none()
            {
                self.run("joint.drag", json!({ "joint": joint, "cancel": true }));
            }
            return true;
        }
        false
    }

    /// Plays the motion study: moves its time on by the frame's time.
    fn motion_tick(&mut self, ctx: &egui::Context) {
        let duration = self.session.doc.motion.duration();
        let Some(panel) = self.motion.study.as_mut() else {
            return;
        };
        if !panel.playing {
            return;
        }
        if duration <= 0.0 {
            panel.playing = false;
            return;
        }
        let dt = f64::from(ctx.input(|i| i.stable_dt)).clamp(0.0, 0.1);
        let mut t = panel.time + dt;
        if t > duration {
            if panel.looping {
                t = 0.0;
            } else {
                t = duration;
                panel.playing = false;
            }
        }
        panel.time = t;
        self.run("motion.set_time", json!({ "t": t }));
        ctx.request_repaint();
    }

    /// Every assembly-tool panel that is open, and the motion study's playback.
    pub(crate) fn motion_panels(&mut self, ctx: &egui::Context) {
        self.motion_tick(ctx);
        self.drive_panel(ctx);
        self.study_panel(ctx);
        self.interference_panel(ctx);
        self.explode_panel(ctx);
    }

    fn panel_window(&self, title: &str, salt: &'static str, width: f32) -> egui::Window<'static> {
        let pos = self.tool_dialog_pos(width);
        egui::Window::new(title.to_owned())
            .id(egui::Id::unique(salt))
            .title_bar(false)
            .frame(widgets::dialog_frame(self.look()))
            .default_pos(pos)
            .fade_in(false)
            .default_width(width)
            .min_width(width)
            .resizable(false)
    }

    fn joint_combo(
        ui: &mut egui::Ui,
        salt: &str,
        joints: &[(u32, String, JointKind)],
        chosen: &mut Option<u32>,
    ) -> bool {
        let text = chosen
            .and_then(|c| joints.iter().find(|j| j.0 == c))
            .map_or("Pick a joint", |j| j.1.as_str())
            .to_owned();
        let before = *chosen;
        egui::ComboBox::from_id_salt(salt)
            .selected_text(text)
            .width(widgets::FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for (id, name, _) in joints {
                    ui.selectable_value(chosen, Some(*id), name);
                }
            });
        before != *chosen
    }

    fn drive_panel(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        if self.motion.drive.is_none() {
            return;
        }
        let joints = self.movable_joints();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let window = self.panel_window(label("joint.set_value"), "drive", WIDTH);
        let mut close = false;
        let mut set: Option<(u32, Value)> = None;
        let mut picked: Option<u32> = None;
        let current = self
            .motion
            .drive
            .as_ref()
            .and_then(|t| t.joint)
            .and_then(|j| self.joint_shown(j));
        let limits = self
            .motion
            .drive
            .as_ref()
            .and_then(|t| t.joint)
            .and_then(|j| self.session.doc.assembly.joint(j).ok())
            .and_then(|j| {
                j.limits
                    .map(|[lo, hi]| (shown(j.kind, lo), shown(j.kind, hi)))
            });
        let Some(tool) = self.motion.drive.as_mut() else {
            return;
        };
        window.show(ctx, |ui| {
            ui.set_width(WIDTH);
            if widgets::dialog_title(ui, look, Icon::DriveJoint, "Drive Joint") {
                close = true;
            }
            widgets::dialog_body(ui, |ui| {
                egui::Grid::new("drive grid")
                    .num_columns(2)
                    .spacing([widgets::S3, widgets::S3])
                    .show(ui, |ui| {
                        widgets::field_label(ui, look, "Joint");
                        if Self::joint_combo(ui, "drive joint", &joints, &mut tool.joint) {
                            picked = tool.joint;
                        }
                        ui.end_row();
                        if let Some((kind, _)) = current {
                            widgets::field_label(
                                ui,
                                look,
                                if kind.value_unit() == Some(Unit::Angle) {
                                    "Angle"
                                } else {
                                    "Distance"
                                },
                            );
                            let edit = ui.add_sized(
                                widgets::FIELD,
                                egui::TextEdit::singleline(&mut tool.value)
                                    .hint_text(unit_text(kind)),
                            );
                            if edit.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                && let Some(j) = tool.joint
                            {
                                set = Some((j, typed(&tool.value)));
                            }
                            ui.end_row();
                            if let Some((lo, hi)) = limits {
                                widgets::field_label(ui, look, "Limits");
                                ui.label(format!("{lo:.2} to {hi:.2} {}", unit_text(kind)));
                                ui.end_row();
                            }
                        }
                    });
                ui.label(
                    egui::RichText::new("Drag a jointed part in the viewport to move it")
                        .size(widgets::T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                );
            });
            widgets::dialog_footer(ui, look, |ui| {
                if widgets::primary_button(ui, look, "Set", tool.joint.is_some()).clicked()
                    && let Some(j) = tool.joint
                {
                    set = Some((j, typed(&tool.value)));
                }
                if widgets::secondary_button(ui, look, "Close").clicked() {
                    close = true;
                }
            });
        });
        if let Some(j) = picked
            && let Some((_, v)) = self.joint_shown(j)
            && let Some(tool) = self.motion.drive.as_mut()
        {
            tool.value = format!("{v:.2}");
        }
        if let Some((joint, value)) = set {
            self.run("joint.set_value", json!({ "joint": joint, "value": value }));
        }
        if close {
            self.motion.drive = None;
        }
    }

    fn study_panel(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 360.0;
        if self.motion.study.is_none() {
            return;
        }
        let joints = self.movable_joints();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let window = self.panel_window(label("motion.set_time"), "study", WIDTH);
        let duration = self.session.doc.motion.duration();
        let keys: Vec<(u32, String, f64, String)> = self
            .session
            .doc
            .motion
            .keys
            .iter()
            .map(|k| {
                let (name, value) = match self.session.doc.assembly.joint(k.joint) {
                    Ok(j) => (
                        j.name.clone(),
                        format!("{:.2} {}", shown(j.kind, k.value), unit_text(j.kind)),
                    ),
                    Err(_) => (format!("joint {}", k.joint), String::new()),
                };
                (k.joint, name, k.time, value)
            })
            .collect();
        let mut close = false;
        let mut scrub: Option<f64> = None;
        let mut add = false;
        let mut remove: Option<(u32, f64)> = None;
        let mut check = false;
        let Some(panel) = self.motion.study.as_mut() else {
            return;
        };
        window.show(ctx, |ui| {
            ui.set_width(WIDTH);
            if widgets::dialog_title(ui, look, Icon::MotionStudy, "Motion Study") {
                close = true;
            }
            widgets::dialog_body(ui, |ui| {
                ui.horizontal(|ui| {
                    let play = if panel.playing { "Pause" } else { "Play" };
                    if widgets::secondary_button(ui, look, play).clicked() {
                        panel.playing = !panel.playing && duration > 0.0;
                    }
                    ui.checkbox(&mut panel.looping, "Loop");
                    let mut t = panel.time;
                    let slider = ui.add(
                        egui::Slider::new(&mut t, 0.0..=duration.max(1e-6))
                            .suffix(" s")
                            .max_decimals(2),
                    );
                    if slider.changed() {
                        panel.playing = false;
                        panel.time = t;
                        scrub = Some(t);
                    }
                });
                ui.add_space(widgets::S3);
                if keys.is_empty() {
                    ui.label(
                        egui::RichText::new("No keys yet: add a joint's value at a time")
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                }
                egui::Grid::new("motion keys")
                    .num_columns(4)
                    .spacing([widgets::S3, widgets::S2])
                    .show(ui, |ui| {
                        for (joint, name, time, value) in &keys {
                            ui.label(name);
                            ui.label(format!("{time:.2} s"));
                            ui.label(value);
                            if ui.small_button("Remove").clicked() {
                                remove = Some((*joint, *time));
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(widgets::S3);
                egui::Grid::new("motion new key")
                    .num_columns(2)
                    .spacing([widgets::S3, widgets::S3])
                    .show(ui, |ui| {
                        widgets::field_label(ui, look, "Joint");
                        Self::joint_combo(ui, "key joint", &joints, &mut panel.key_joint);
                        ui.end_row();
                        widgets::field_label(ui, look, "Time");
                        ui.add_sized(
                            widgets::FIELD,
                            egui::TextEdit::singleline(&mut panel.key_time).hint_text("s"),
                        );
                        ui.end_row();
                        widgets::field_label(ui, look, "Value");
                        ui.add_sized(
                            widgets::FIELD,
                            egui::TextEdit::singleline(&mut panel.key_value)
                                .hint_text("as shown now"),
                        );
                        ui.end_row();
                    });
                if let Some(c) = &panel.checked {
                    ui.add_space(widgets::S2);
                    for (t, n) in c {
                        let text = if *n == 0 {
                            format!("{t:.2} s: no interference")
                        } else {
                            format!("{t:.2} s: {n} overlapping pair(s)")
                        };
                        let colour = if *n == 0 {
                            look.c(|t| t.success)
                        } else {
                            look.c(|t| t.error)
                        };
                        ui.label(egui::RichText::new(text).color(colour));
                    }
                }
            });
            widgets::dialog_footer(ui, look, |ui| {
                if widgets::primary_button(ui, look, "Add Key", panel.key_joint.is_some()).clicked()
                {
                    add = true;
                }
                if widgets::secondary_button(ui, look, "Check at Keys").clicked() {
                    check = true;
                }
                if widgets::secondary_button(ui, look, "Close").clicked() {
                    close = true;
                }
            });
        });
        if let Some(t) = scrub {
            self.run("motion.set_time", json!({ "t": t }));
        }
        if add {
            let p = self.motion.study.as_ref().map(|panel| {
                let mut p = json!({
                    "joint": panel.key_joint,
                    "time": typed(&panel.key_time),
                });
                if !panel.key_value.trim().is_empty() {
                    p["value"] = typed(&panel.key_value);
                }
                p
            });
            if let Some(p) = p {
                self.run("motion.add_key", p);
            }
        }
        if let Some((joint, time)) = remove {
            self.run("motion.remove_key", json!({ "joint": joint, "time": time }));
        }
        if check && let Some(r) = self.run("inspect.interference", json!({ "at_keys": true })) {
            let found: Vec<(f64, usize)> = r["keys"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|k| {
                            (
                                k["time"].as_f64().unwrap_or(0.0),
                                k["count"].as_u64().unwrap_or(0) as usize,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(panel) = self.motion.study.as_mut() {
                panel.checked = Some(found);
            }
        }
        if close {
            self.motion.study = None;
            self.run("motion.set_time", json!({ "t": null }));
        }
    }

    fn interference_panel(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 340.0;
        if self.motion.interference.is_none() {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let units = self.units();
        let window = self.panel_window(label("inspect.interference"), "interference", WIDTH);
        let mut close = false;
        let mut again = false;
        let Some(panel) = self.motion.interference.as_mut() else {
            return;
        };
        window.show(ctx, |ui| {
            ui.set_width(WIDTH);
            if widgets::dialog_title(ui, look, Icon::Interference, "Interference") {
                close = true;
            }
            widgets::dialog_body(ui, |ui| {
                let pairs = panel
                    .result
                    .as_ref()
                    .and_then(|r| r["pairs"].as_array().cloned())
                    .unwrap_or_default();
                if panel.result.is_none() {
                    ui.label("The check could not run");
                } else if pairs.is_empty() {
                    ui.label(egui::RichText::new("No bodies overlap").color(look.c(|t| t.success)));
                } else {
                    egui::Grid::new("interference pairs")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S2])
                        .show(ui, |ui| {
                            for p in &pairs {
                                ui.label(format!(
                                    "{} and {}",
                                    p["a"].as_str().unwrap_or("?"),
                                    p["b"].as_str().unwrap_or("?")
                                ));
                                ui.label(
                                    egui::RichText::new(
                                        units.volume(p["volume"].as_f64().unwrap_or(0.0)),
                                    )
                                    .color(look.c(|t| t.error)),
                                );
                                ui.end_row();
                            }
                        });
                }
                if let Some(keys) = panel.result.as_ref().and_then(|r| r["keys"].as_array()) {
                    ui.add_space(widgets::S2);
                    for k in keys {
                        ui.label(format!(
                            "At {:.2} s: {} pair(s)",
                            k["time"].as_f64().unwrap_or(0.0),
                            k["count"]
                        ));
                    }
                }
                ui.checkbox(&mut panel.at_keys, "Also check at each motion keyframe");
            });
            widgets::dialog_footer(ui, look, |ui| {
                if widgets::primary_button(ui, look, "Check", true).clicked() {
                    again = true;
                }
                if widgets::secondary_button(ui, look, "Close").clicked() {
                    close = true;
                }
            });
        });
        if again {
            let at_keys = self.motion.interference.as_ref().is_some_and(|p| p.at_keys);
            let r = self.run("inspect.interference", json!({ "at_keys": at_keys }));
            if let Some(p) = self.motion.interference.as_mut() {
                p.result = r;
            }
        }
        if close {
            self.motion.interference = None;
            self.run("inspect.interference", json!({ "clear": true }));
        }
    }

    fn explode_panel(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        if self.motion.explode.is_none() {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let chosen = self.chosen_component();
        let chosen_name = self.session.doc.assembly.name_of(chosen);
        let window = self.panel_window(label("view.explode"), "explode", WIDTH);
        let mut close = false;
        let mut factor: Option<f64> = None;
        let mut manual: Option<[f64; 3]> = None;
        let Some(panel) = self.motion.explode.as_mut() else {
            return;
        };
        window.show(ctx, |ui| {
            ui.set_width(WIDTH);
            if widgets::dialog_title(ui, look, Icon::Explode, "Explode") {
                close = true;
            }
            widgets::dialog_body(ui, |ui| {
                egui::Grid::new("explode grid")
                    .num_columns(2)
                    .spacing([widgets::S3, widgets::S3])
                    .show(ui, |ui| {
                        widgets::field_label(ui, look, "Factor");
                        if ui
                            .add(egui::Slider::new(&mut panel.factor, 1.0..=5.0).max_decimals(2))
                            .changed()
                        {
                            factor = Some(panel.factor);
                        }
                        ui.end_row();
                        widgets::field_label(ui, look, "Move");
                        ui.label(chosen_name.as_str());
                        ui.end_row();
                        widgets::field_label(ui, look, "Offset");
                        ui.horizontal(|ui| {
                            let mut changed = false;
                            for v in &mut panel.offset {
                                changed |= ui
                                    .add(egui::DragValue::new(v).speed(0.5).suffix(" mm"))
                                    .changed();
                            }
                            if changed {
                                manual = Some(panel.offset);
                            }
                        });
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(
                        "A view of the assembly: the model, measures and exports do not change",
                    )
                    .size(widgets::T_SMALL)
                    .color(look.c(|t| t.text_muted)),
                );
            });
            widgets::dialog_footer(ui, look, |ui| {
                if widgets::secondary_button(ui, look, "Collapse").clicked() {
                    close = true;
                }
            });
        });
        if let Some(f) = factor {
            self.run("view.explode", json!({ "factor": f }));
        }
        if let Some([x, y, z]) = manual {
            self.run(
                "view.explode",
                json!({ "component": chosen, "offset": {"x": x, "y": y, "z": z} }),
            );
        }
        if close {
            self.motion.explode = None;
            self.run("view.explode_clear", json!({}));
        }
    }
}

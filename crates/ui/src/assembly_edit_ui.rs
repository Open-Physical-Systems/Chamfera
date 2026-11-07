//! Editing an assembly from the Browser: the Joints section (each joint with Edit Joint
//! and Delete Joint), and the Edit Joint, Move Component and Rename Component dialogs.
//! Each dialog builds the parameters of one command (`joint.edit`, `component.move`,
//! `component.rename`) and runs it on OK; nothing changes the design directly.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::expr::{self, Dim};
use chamfera_doc::{JointKind, Unit};
use serde_json::{Value, json};

/// The axes Move Component turns about: what the user reads, and the axis.
pub(crate) const TURN_AXES: [(&str, [f64; 3]); 3] = [
    ("X axis", [1.0, 0.0, 0.0]),
    ("Y axis", [0.0, 1.0, 0.0]),
    ("Z axis", [0.0, 0.0, 1.0]),
];

/// An assembly dialog opened from the Browser, with its fields as typed.
#[derive(Debug, Clone, PartialEq)]
pub enum AssemblyDialog {
    /// Edit Joint: its name and limits (degrees or the design's length unit).
    EditJoint {
        joint: u32,
        name: String,
        limited: bool,
        min: String,
        max: String,
    },
    /// Move Component: a move along X, Y and Z (the design's length unit), then a turn
    /// about an axis through the origin; relative to where it is unless `absolute`.
    MoveComponent {
        component: u32,
        dx: String,
        dy: String,
        dz: String,
        /// Index into [`TURN_AXES`].
        axis: usize,
        angle: String,
        absolute: bool,
    },
    /// Rename Component.
    RenameComponent { component: u32, name: String },
}

impl AssemblyDialog {
    fn title(&self) -> &'static str {
        label(match self {
            AssemblyDialog::EditJoint { .. } => "joint.edit",
            AssemblyDialog::MoveComponent { .. } => "component.move",
            AssemblyDialog::RenameComponent { .. } => "component.rename",
        })
    }
}

/// A joint value as the dialog shows it: degrees for turning joints, mm for slides.
fn shown(kind: JointKind, v: f64) -> String {
    let (v, unit) = match kind.value_unit() {
        Some(Unit::Angle) => (v.to_degrees(), "deg"),
        _ => (v, "mm"),
    };
    let text = format!("{v:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} {unit}")
}

impl ChamferaApp {
    /// Edit Joint: opens the dialog on a joint, filled in from it.
    pub(crate) fn open_joint_edit(&mut self, joint: u32) {
        let Ok(j) = self.session.doc.assembly.joint(joint) else {
            return;
        };
        let (min, max) = match j.limits {
            Some([lo, hi]) => (shown(j.kind, lo), shown(j.kind, hi)),
            None => {
                let v = shown(j.kind, j.value);
                (v.clone(), v)
            }
        };
        self.assembly.edit = Some(AssemblyDialog::EditJoint {
            joint,
            name: j.name.clone(),
            limited: j.limits.is_some(),
            min,
            max,
        });
        self.assembly.edit_error = None;
    }

    /// Move Component: opens the dialog on a component, with no move typed yet.
    pub(crate) fn open_component_move(&mut self, component: u32) {
        if self.session.doc.assembly.component(component).is_err() {
            return;
        }
        self.assembly.edit = Some(AssemblyDialog::MoveComponent {
            component,
            dx: "0".into(),
            dy: "0".into(),
            dz: "0".into(),
            axis: 2,
            angle: "0".into(),
            absolute: false,
        });
        self.assembly.edit_error = None;
    }

    /// Rename Component: opens the dialog with the component's name.
    pub(crate) fn open_component_rename(&mut self, component: u32) {
        let Ok(c) = self.session.doc.assembly.component(component) else {
            return;
        };
        self.assembly.edit = Some(AssemblyDialog::RenameComponent {
            component,
            name: c.name.clone(),
        });
        self.assembly.edit_error = None;
    }

    /// The command and parameters the open assembly dialog stands for, or why its
    /// fields cannot be read.
    pub(crate) fn assembly_dialog_params(&self) -> Result<(&'static str, Value), String> {
        let Some(d) = &self.assembly.edit else {
            return Err("no dialog is open".into());
        };
        Ok(match d {
            AssemblyDialog::EditJoint {
                joint,
                name,
                limited,
                min,
                max,
            } => {
                let mut p = json!({ "joint": joint, "name": name.trim() });
                if *limited {
                    if min.trim().is_empty() || max.trim().is_empty() {
                        return Err("type both limits, or turn the limits off".into());
                    }
                    p["min"] = json!(min.trim());
                    p["max"] = json!(max.trim());
                } else {
                    p["limits"] = json!(false);
                }
                ("joint.edit", p)
            }
            AssemblyDialog::MoveComponent {
                component,
                dx,
                dy,
                dz,
                axis,
                angle,
                absolute,
            } => {
                let unit = self.session.doc.units.length.mm();
                let length = |what: &str, v: &str| -> Result<f64, String> {
                    let v = if v.trim().is_empty() { "0" } else { v.trim() };
                    expr::evaluate_as_in(v, Dim::LENGTH, unit, &expr::NoNames)
                        .map_err(|e| format!("{what} {v:?}: {e}"))
                };
                let (x, y, z) = (length("X", dx)?, length("Y", dy)?, length("Z", dz)?);
                let mut transform = json!({
                    "translate": { "x": x, "y": y, "z": z },
                    "absolute": absolute,
                });
                let angle = angle.trim();
                let turns = !angle.is_empty() && angle.parse::<f64>().map_or(true, |a| a != 0.0);
                if turns {
                    let [ax, ay, az] = TURN_AXES.get(*axis).map_or([0.0, 0.0, 1.0], |a| a.1);
                    transform["rotate"] = json!({
                        "axis": { "x": ax, "y": ay, "z": az },
                        "angle": match angle.parse::<f64>() {
                            Ok(n) => json!(n),
                            Err(_) => json!(angle),
                        },
                    });
                }
                (
                    "component.move",
                    json!({ "component": component, "transform": transform }),
                )
            }
            AssemblyDialog::RenameComponent { component, name } => (
                "component.rename",
                json!({ "component": component, "name": name.trim() }),
            ),
        })
    }

    /// OK in an assembly dialog: runs its command and closes it when that worked.
    pub(crate) fn assembly_dialog_ok(&mut self) -> Option<Value> {
        let (id, params) = match self.assembly_dialog_params() {
            Ok(x) => x,
            Err(e) => {
                self.assembly.edit_error = Some(e);
                return None;
            }
        };
        match self.session.execute(id, &params) {
            Ok(r) => {
                self.tutorial_saw(id);
                let name = r["name"].as_str().unwrap_or(label(id));
                self.message = Some((format!("{}: {name}", label(id)), false));
                self.assembly.edit = None;
                self.assembly.edit_error = None;
                Some(r)
            }
            Err(e) => {
                self.assembly.edit_error = Some(e.to_string());
                None
            }
        }
    }

    /// The open assembly dialog.
    pub(crate) fn assembly_edit_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        if self.assembly.edit.is_none() {
            return;
        }
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let units = self.session.doc.units.length.symbol();
        let subject = match &self.assembly.edit {
            Some(AssemblyDialog::EditJoint { joint, .. }) => self
                .session
                .doc
                .assembly
                .joint(*joint)
                .map(|j| (j.kind.value_unit() == Some(Unit::Angle), j.kind))
                .ok(),
            _ => None,
        };
        let component_name = match &self.assembly.edit {
            Some(
                AssemblyDialog::MoveComponent { component, .. }
                | AssemblyDialog::RenameComponent { component, .. },
            ) => self.session.doc.assembly.name_of(*component),
            _ => String::new(),
        };
        let error = self.assembly.edit_error.clone();
        let Some(d) = self.assembly.edit.as_mut() else {
            return;
        };
        let title = d.title();
        let icon = match d {
            AssemblyDialog::EditJoint { .. } => Icon::Joint,
            AssemblyDialog::MoveComponent { .. } => Icon::Move,
            AssemblyDialog::RenameComponent { .. } => Icon::Edit,
        };
        let mut ok = false;
        let mut cancel = false;
        egui::Window::new(title)
            .id(egui::Id::unique("assembly edit dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
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
                    egui::Grid::new("assembly edit grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            let mut field =
                                |ui: &mut egui::Ui, caption: &str, v: &mut String, hint: &str| {
                                    widgets::field_label(ui, look, caption);
                                    let r = ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(v)
                                            .hint_text(hint)
                                            .align(egui::Align2::LEFT_CENTER),
                                    );
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        ok = true;
                                    }
                                    ui.end_row();
                                };
                            match d {
                                AssemblyDialog::EditJoint {
                                    name,
                                    limited,
                                    min,
                                    max,
                                    ..
                                } => {
                                    field(ui, "Name", name, "Joint name");
                                    if let Some((angle, kind)) = subject {
                                        widgets::field_label(ui, look, "Type");
                                        ui.label(crate::assembly_ui::kind_text(kind));
                                        ui.end_row();
                                        if kind != JointKind::Rigid {
                                            widgets::field_label(ui, look, "Limits");
                                            ui.checkbox(limited, "Keep the value between");
                                            ui.end_row();
                                            if *limited {
                                                let hint = if angle { "0 deg" } else { "0 mm" };
                                                field(ui, "Minimum", min, hint);
                                                field(ui, "Maximum", max, hint);
                                            }
                                        }
                                    }
                                }
                                AssemblyDialog::MoveComponent {
                                    dx,
                                    dy,
                                    dz,
                                    axis,
                                    angle,
                                    absolute,
                                    ..
                                } => {
                                    widgets::field_label(ui, look, "Component");
                                    ui.label(component_name.as_str());
                                    ui.end_row();
                                    field(ui, "Move X", dx, units);
                                    field(ui, "Move Y", dy, units);
                                    field(ui, "Move Z", dz, units);
                                    widgets::field_label(ui, look, "Turn about");
                                    egui::ComboBox::from_id_salt("move component axis")
                                        .selected_text(TURN_AXES.get(*axis).map_or("", |a| a.0))
                                        .width(widgets::FIELD_WIDTH)
                                        .show_ui(ui, |ui| {
                                            for (i, (n, _)) in TURN_AXES.iter().enumerate() {
                                                ui.selectable_value(axis, i, *n);
                                            }
                                        });
                                    ui.end_row();
                                    field(ui, "Angle", angle, "deg");
                                    widgets::field_label(ui, look, "From");
                                    ui.checkbox(absolute, "The origin, not where it is");
                                    ui.end_row();
                                }
                                AssemblyDialog::RenameComponent { name, .. } => {
                                    widgets::field_label(ui, look, "Component");
                                    ui.label(component_name.as_str());
                                    ui.end_row();
                                    field(ui, "Name", name, "New name");
                                }
                            }
                        });
                    if let Some(e) = &error {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(e)
                                    .size(widgets::T_SMALL)
                                    .color(look.c(|t| t.error)),
                            )
                            .wrap(),
                        );
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "OK", true).clicked() {
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
            self.assembly_dialog_ok();
        }
        if cancel {
            self.assembly.edit = None;
            self.assembly.edit_error = None;
        }
    }

    /// The Joints part of the Browser's Components section: one row per joint, with
    /// Edit Joint and Delete Joint in its menu (a double-click edits it).
    pub(crate) fn browser_joints(&mut self, ui: &mut egui::Ui, depth: usize) {
        let look = self.look();
        let joints: Vec<(u32, String, String, String)> = self
            .session
            .doc
            .assembly
            .joints
            .iter()
            .map(|j| {
                let value = match j.kind {
                    JointKind::Rigid => String::new(),
                    k => format!(" · {}", shown(k, j.value)),
                };
                let limits = match j.limits {
                    Some([lo, hi]) => {
                        format!("Limits {} to {}\n", shown(j.kind, lo), shown(j.kind, hi))
                    }
                    None => String::new(),
                };
                let note = format!("{}{value}", crate::assembly_ui::kind_text(j.kind));
                (j.id, j.name.clone(), note, limits)
            })
            .collect();
        if joints.is_empty() {
            return;
        }
        let mut run: Option<(u32, bool)> = None;
        for (id, name, note, limits) in joints {
            let r = crate::assembly_ui::row(
                ui,
                look,
                ui.scope_id().with(("joint", id)),
                (depth, Icon::Joint),
                &name,
                Some(note.as_str()),
                false,
            )
            .on_hover_text(format!(
                "{limits}Double-click to edit the joint's name and limits"
            ));
            if r.double_clicked() {
                run = Some((id, false));
            }
            r.context_menu(|ui| {
                if widgets::menu_item(ui, look, Icon::Edit, label("joint.edit"), true).clicked() {
                    run = Some((id, false));
                }
                if widgets::menu_item(ui, look, Icon::Delete, label("joint.delete"), true).clicked()
                {
                    run = Some((id, true));
                }
                if run.is_some() {
                    ui.close();
                }
            });
        }
        match run {
            Some((id, true)) => {
                if let Some(r) = self.run("joint.delete", json!({ "joint": id })) {
                    self.message = Some((
                        format!(
                            "{} deleted: its part stays where it was",
                            r["name"].as_str().unwrap_or("Joint")
                        ),
                        false,
                    ));
                }
            }
            Some((id, false)) => self.open_joint_edit(id),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two components: A (grounded), a 40 x 30 x 4 plate, and B, a 40 x 10 x 4 plate
    /// sliding along X from the origin at 15 mm. Returns (B, the joint).
    fn slider(app: &mut ChamferaApp) -> (u32, u32) {
        let mut ids = Vec::new();
        for y2 in [30, 10] {
            let r = app.run("sketch.create", json!({ "plane": "xy" })).unwrap();
            app.run(
                "sketch.rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 40, "y2": y2 }),
            );
            app.run("sketch.finish", json!({}));
            let f = app
                .run(
                    "solid.extrude",
                    json!({ "sketch": r["sketch"], "distance": 4,
                            "profiles": [{ "x": 20, "y": 5 }] }),
                )
                .unwrap();
            let c = app
                .run("component.new", json!({ "from_bodies": [f["feature"]] }))
                .unwrap();
            ids.push(u32::try_from(c["component"].as_u64().unwrap()).unwrap());
        }
        app.run("component.ground", json!({ "component": ids[0] }));
        let at = |c: u32| {
            json!({ "component": c, "point": { "x": 0, "y": 0, "z": 0 },
                    "axis": { "x": 1, "y": 0, "z": 0 } })
        };
        let j = app
            .run(
                "joint.create",
                json!({ "type": "slider", "a": at(ids[0]), "b": at(ids[1]), "value": 15 }),
            )
            .unwrap();
        (ids[1], u32::try_from(j["joint"].as_u64().unwrap()).unwrap())
    }

    fn min_x(app: &ChamferaApp, component: u32) -> f64 {
        app.session
            .solids()
            .iter()
            .filter(|s| s.component == component)
            .filter_map(|s| s.body.bounding_box().map(|b| b.min.x))
            .fold(f64::INFINITY, f64::min)
    }

    /// Edit Joint (QA user guide pass: there was no UI for joint limits): the dialog
    /// opens with the value as both limits, typed limits 0 and 10 mm move the slider
    /// from 15 to 10 (B's plate from x = 15 to x = 10), and turning limits off sends
    /// `limits: false`.
    #[test]
    fn edit_joint_sets_limits_and_keeps_the_value_inside() {
        let mut app = ChamferaApp::headless();
        let (b, j) = slider(&mut app);
        assert!((min_x(&app, b) - 15.0).abs() < 1e-9);
        app.open_joint_edit(j);
        let Some(AssemblyDialog::EditJoint {
            limited, min, max, ..
        }) = app.assembly.edit.as_mut()
        else {
            panic!("the Edit Joint dialog is open");
        };
        assert!(!*limited);
        assert_eq!(min, "15 mm");
        *limited = true;
        *min = "0".into();
        *max = "10 mm".into();
        let r = app.assembly_dialog_ok().unwrap();
        assert!(app.assembly.edit.is_none());
        assert_eq!(r["limits"], json!([0.0, 10.0]));
        assert!((min_x(&app, b) - 10.0).abs() < 1e-9);
        app.open_joint_edit(j);
        if let Some(AssemblyDialog::EditJoint { limited, .. }) = app.assembly.edit.as_mut() {
            assert!(*limited);
            *limited = false;
        }
        let (id, p) = app.assembly_dialog_params().unwrap();
        assert_eq!(id, "joint.edit");
        assert_eq!(p["limits"], false);
        app.assembly_dialog_ok().unwrap();
        assert!(app.session.doc.assembly.joint(j).unwrap().limits.is_none());
    }

    /// A refused edit keeps the dialog open with the error under the fields.
    #[test]
    fn a_refused_joint_edit_keeps_the_dialog_with_its_error() {
        let mut app = ChamferaApp::headless();
        let (_, j) = slider(&mut app);
        app.open_joint_edit(j);
        if let Some(AssemblyDialog::EditJoint {
            limited, min, max, ..
        }) = app.assembly.edit.as_mut()
        {
            *limited = true;
            *min = "20".into();
            *max = "5".into();
        }
        assert!(app.assembly_dialog_ok().is_none());
        assert!(app.assembly.edit.is_some());
        let e = app.assembly.edit_error.clone().unwrap_or_default();
        assert!(e.contains("min"), "{e}");
        assert!(app.session.doc.assembly.joint(j).unwrap().limits.is_none());
    }

    /// Move Component with typed values (QA user guide pass: a component could only be
    /// moved by script). A 40 x 30 x 4 plate moved 10 along X and 1 in along Y spans
    /// x 10..50, y 25.4..55.4; then turned 90 degrees about Z through the origin,
    /// (x, y) -> (-y, x), it spans x -55.4..-25.4, y 10..50.
    #[test]
    fn move_component_moves_by_the_typed_values() {
        let mut app = ChamferaApp::headless();
        let r = app.run("sketch.create", json!({ "plane": "xy" })).unwrap();
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        let f = app
            .run(
                "solid.extrude",
                json!({ "sketch": r["sketch"], "distance": 4,
                        "profiles": [{ "x": 20, "y": 15 }] }),
            )
            .unwrap();
        let c = app
            .run("component.new", json!({ "from_bodies": [f["feature"]] }))
            .unwrap();
        let c = u32::try_from(c["component"].as_u64().unwrap()).unwrap();
        let bounds = |app: &ChamferaApp| {
            let b = app.session.solids()[0].body.bounding_box().unwrap();
            [b.min.x, b.min.y, b.max.x, b.max.y]
        };
        let near = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        app.open_component_move(c);
        if let Some(AssemblyDialog::MoveComponent { dx, dy, .. }) = app.assembly.edit.as_mut() {
            *dx = "10".into();
            *dy = "1 in".into();
        }
        app.assembly_dialog_ok().unwrap();
        assert!(
            near(bounds(&app), [10.0, 25.4, 50.0, 55.4]),
            "{:?}",
            bounds(&app)
        );
        app.open_component_move(c);
        if let Some(AssemblyDialog::MoveComponent { angle, axis, .. }) = app.assembly.edit.as_mut()
        {
            *axis = 2;
            *angle = "90".into();
        }
        app.assembly_dialog_ok().unwrap();
        assert!(
            near(bounds(&app), [-55.4, 10.0, -25.4, 50.0]),
            "{:?}",
            bounds(&app)
        );
        // An unreadable value is refused before anything runs.
        app.open_component_move(c);
        if let Some(AssemblyDialog::MoveComponent { dz, .. }) = app.assembly.edit.as_mut() {
            *dz = "abc".into();
        }
        assert!(app.assembly_dialog_ok().is_none());
        assert!(
            app.assembly
                .edit_error
                .as_deref()
                .is_some_and(|e| e.starts_with("Z"))
        );
        // Rename Component runs component.rename.
        app.open_component_rename(c);
        if let Some(AssemblyDialog::RenameComponent { name, .. }) = app.assembly.edit.as_mut() {
            *name = "Bracket".into();
        }
        app.assembly_dialog_ok().unwrap();
        assert_eq!(app.session.doc.assembly.name_of(c), "Bracket");
    }
}

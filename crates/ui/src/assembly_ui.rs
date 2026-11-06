//! Assemblies in the UI: the ASSEMBLE ribbon group (New Component, Joint, Ground), the
//! Joint dialog (pick two joint origins, choose a type, set a value, OK) and the
//! Components section of the Browser. Acts only through `component.*` and `joint.*`
//! commands.

use crate::icons::Icon;
use crate::widgets::{self, Look, ROW_HEIGHT, T_BODY};
use crate::{ChamferaApp, label};
use chamfera_doc::{JointKind, JointOrigin, OriginAt, ROOT, Unit};
use chamfera_geom::{Frame, Point3};
use chamfera_render::{Overlay, OverlayStyle};
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Ui, vec2};
use serde_json::json;

/// The Joint tool while it is open.
pub struct JointTool {
    /// The first pick: the origin on the component that moves.
    pub moving: Option<(JointOrigin, Frame)>,
    /// The second pick: the origin it moves to.
    pub target: Option<(JointOrigin, Frame)>,
    pub kind: JointKind,
    /// The angle (degrees) or distance (mm), as typed.
    pub value: String,
    pub flip: bool,
    /// The origin under the pointer, shown as a preview.
    pub hover: Option<Frame>,
}

impl Default for JointTool {
    fn default() -> Self {
        JointTool {
            moving: None,
            target: None,
            kind: JointKind::Revolute,
            value: "0".into(),
            flip: false,
            hover: None,
        }
    }
}

/// The assembly UI's state.
#[derive(Default)]
pub struct AssemblyUi {
    pub joint: Option<JointTool>,
    /// The Components section of the Browser is folded.
    folded: bool,
}

fn kind_text(k: JointKind) -> &'static str {
    match k {
        JointKind::Rigid => "Rigid",
        JointKind::Revolute => "Revolute",
        JointKind::Slider => "Slider",
        JointKind::Cylindrical => "Cylindrical",
        JointKind::Planar => "Planar",
    }
}

/// A short description of where an origin is, for the dialog.
fn origin_text(app: &ChamferaApp, o: &JointOrigin) -> String {
    let what = match &o.at {
        OriginAt::Face { face } => format!("face {face}"),
        OriginAt::Edge { edge } => format!("edge {edge}"),
        OriginAt::Vertex { edge, .. } => format!("vertex of {edge}"),
        OriginAt::Point { .. } => "point".to_owned(),
    };
    let comp = if o.component == ROOT {
        app.design_name()
    } else {
        app.session.doc.assembly.name_of(o.component)
    };
    format!("{comp} · {what}")
}

/// A Browser row for a component or a body, at a depth in the tree with an icon.
fn row(
    ui: &mut Ui,
    look: Look,
    id: egui::Id,
    (depth, icon): (usize, Icon),
    text: &str,
    note: Option<&str>,
    active: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(widgets::R_SMALL);
    if active {
        painter.rect_filled(rect, radius, look.c(|t| t.selected));
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 5.0), vec2(2.5, rect.height() - 10.0));
        painter.rect_filled(bar, CornerRadius::same(1), look.c(|t| t.accent));
    } else if response.hovered() {
        painter.rect_filled(rect, radius, look.c(|t| t.hover));
    }
    widgets::focus_ring(ui, look, &response, widgets::R_SMALL);
    let x = rect.min.x + 4.0 + depth as f32 * 14.0 + 16.0;
    let cy = rect.center().y;
    let (b, a) = look.icon_colours(true);
    look.icons.paint(
        painter,
        icon,
        Rect::from_center_size(egui::pos2(x + 8.0, cy), egui::Vec2::splat(16.0)),
        b,
        a,
    );
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        FontId::proportional(T_BODY),
        look.c(|t| t.text),
    );
    let end = x + 22.0 + galley.size().x;
    let clip = Rect::from_min_max(egui::pos2(x, rect.min.y), rect.max);
    painter.with_clip_rect(clip).galley(
        egui::pos2(x + 22.0, cy - galley.size().y * 0.5),
        galley,
        look.c(|t| t.text),
    );
    if let Some(note) = note {
        painter.with_clip_rect(clip).text(
            egui::pos2(end + 6.0, cy),
            Align2::LEFT_CENTER,
            note,
            FontId::proportional(widgets::T_SMALL),
            look.c(|t| t.text_muted),
        );
    }
    response
}

impl ChamferaApp {
    /// The component the user means: the selected body's, else the active one.
    pub(crate) fn chosen_component(&self) -> u32 {
        self.session
            .scene
            .selection
            .and_then(|h| self.session.component_of_body(h.body))
            .unwrap_or(self.session.doc.assembly.active)
    }

    /// The chosen component and whether it is grounded, for the Ground tool.
    pub(crate) fn ground_state(&self) -> (u32, bool) {
        let chosen = self.chosen_component();
        let grounded = chosen != ROOT
            && self
                .session
                .doc
                .assembly
                .component(chosen)
                .is_ok_and(|c| c.grounded);
        (chosen, grounded)
    }

    /// Joint: opens the tool, or closes it when it is open.
    pub(crate) fn toggle_joint(&mut self) {
        if self.assembly.joint.is_some() {
            self.assembly.joint = None;
        } else {
            self.assembly.joint = Some(JointTool::default());
            self.message = Some((
                "Joint: click a face or edge on the part that moves, then where it goes".into(),
                false,
            ));
        }
    }

    /// Ground: fixes the chosen component in place, or lets it move again.
    pub(crate) fn toggle_ground(&mut self) {
        let (chosen, grounded) = self.ground_state();
        if chosen != ROOT {
            self.run(
                "component.ground",
                json!({ "component": chosen, "grounded": !grounded }),
            );
        }
    }

    /// New Component: from the selected body, or an empty active one.
    pub(crate) fn new_component(&mut self) {
        let feature = self.session.scene.selection.and_then(|h| {
            self.session
                .solids()
                .iter()
                .find(|s| s.scene_id == Some(h.body))
                .map(|s| s.feature)
        });
        let params = match feature {
            Some(f) => json!({ "from_bodies": [f] }),
            None => json!({}),
        };
        if let Some(r) = self.run("component.new", params) {
            let name = r["name"].as_str().unwrap_or("Component");
            let text = if feature.is_some() {
                format!("{name}: the selected body is in it")
            } else {
                format!("{name} is active: new features go into it")
            };
            self.message = Some((text, false));
        }
    }

    /// A click in the viewport while the Joint tool is open: the first pick is the part
    /// that moves, the second where it goes; a third starts again.
    pub(crate) fn joint_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(picked) = self.session.joint_origin_at(x, y, w, h) else {
            return;
        };
        let Some(tool) = self.assembly.joint.as_mut() else {
            return;
        };
        if tool.moving.is_none() || tool.target.is_some() {
            tool.moving = Some(picked);
            tool.target = None;
        } else {
            tool.target = Some(picked);
        }
    }

    /// The pointer moved over the viewport while the Joint tool is open.
    pub(crate) fn joint_hover(&mut self, at: Option<(f64, f64)>, w: f64, h: f64) {
        let hover = at.and_then(|(x, y)| self.session.joint_origin_at(x, y, w, h).map(|o| o.1));
        if let Some(tool) = self.assembly.joint.as_mut() {
            tool.hover = hover;
        }
    }

    /// The picked origins and the one under the pointer: a point and a short line along
    /// its axis. Drawn as previews: the viewport clears those every frame.
    pub(crate) fn joint_overlays(&self) -> Vec<Overlay> {
        let Some(tool) = self.assembly.joint.as_ref() else {
            return Vec::new();
        };
        let size = self
            .session
            .scene
            .bounds()
            .map_or(10.0, |b| (b.max - b.min).length() * 0.12);
        let mark = |f: &Frame| -> ([Point3; 2], Point3) {
            ([f.origin(), f.origin() + f.z() * size], f.origin())
        };
        let mut picked = Overlay {
            style: OverlayStyle::Preview,
            lines: Vec::new(),
            points: Vec::new(),
            on_top: true,
        };
        for (_, f) in tool.moving.iter().chain(tool.target.iter()) {
            let (l, p) = mark(f);
            picked.lines.push(l);
            picked.points.push(p);
        }
        let mut out = vec![picked];
        if let Some(f) = &tool.hover {
            let (l, p) = mark(f);
            out.push(Overlay {
                style: OverlayStyle::Preview,
                lines: vec![l],
                points: vec![p],
                on_top: true,
            });
        }
        out
    }

    /// The Joint dialog.
    pub(crate) fn joint_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        if self.assembly.joint.is_none() {
            return;
        }
        let pos = self.tool_dialog_pos(WIDTH);
        let texts = self.assembly.joint.as_ref().map(|t| {
            (
                t.moving.as_ref().map(|o| origin_text(self, &o.0)),
                t.target.as_ref().map(|o| origin_text(self, &o.0)),
            )
        });
        let (moving_text, target_text) = texts.unwrap_or_default();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.assembly.joint.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = label("joint.create");
        let ready = tool.moving.is_some() && tool.target.is_some();
        egui::Window::new(title)
            .id(egui::Id::unique("joint dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Joint, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("joint grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            for (caption, text, hint) in [
                                ("Moves", &moving_text, "Click the part that moves"),
                                ("To", &target_text, "Click where it goes"),
                            ] {
                                widgets::field_label(ui, look, caption);
                                let (t, c) = match text {
                                    Some(t) => (t.as_str(), look.c(|t| t.text)),
                                    None => (hint, look.c(|t| t.text_muted)),
                                };
                                ui.label(egui::RichText::new(t).color(c));
                                ui.end_row();
                            }
                            widgets::field_label(ui, look, "Type");
                            egui::ComboBox::from_id_salt("joint type")
                                .selected_text(kind_text(tool.kind))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for k in JointKind::ALL {
                                        ui.selectable_value(&mut tool.kind, k, kind_text(k));
                                    }
                                });
                            ui.end_row();
                            if let Some(unit) = tool.kind.value_unit() {
                                let angle = unit == Unit::Angle;
                                widgets::field_label(
                                    ui,
                                    look,
                                    if angle { "Angle" } else { "Distance" },
                                );
                                let edit = ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(&mut tool.value)
                                        .hint_text(if angle { "90 deg" } else { "10 mm" })
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if edit.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    ok = true;
                                }
                                ui.end_row();
                            }
                            widgets::field_label(ui, look, "Flip");
                            ui.checkbox(&mut tool.flip, "Turn the moving part over");
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
            let params = self.joint_params();
            if let Some(r) = self.run("joint.create", params) {
                self.message = Some((
                    format!("{} created", r["name"].as_str().unwrap_or("Joint")),
                    false,
                ));
                self.assembly.joint = None;
            }
        }
        if cancel {
            self.assembly.joint = None;
        }
    }

    /// The `joint.create` parameters the open dialog stands for.
    pub(crate) fn joint_params(&self) -> serde_json::Value {
        let Some(tool) = self.assembly.joint.as_ref() else {
            return json!({});
        };
        let mut p = json!({
            "type": tool.kind.key(),
            "a": tool.target.as_ref().map(|o| &o.0),
            "b": tool.moving.as_ref().map(|o| &o.0),
            "flip": tool.flip,
        });
        if tool.kind.value_unit().is_some() {
            let v = tool.value.trim();
            p["value"] = match v.parse::<f64>() {
                Ok(n) => json!(n),
                Err(_) => json!(v),
            };
        }
        p
    }

    /// The Components section of the Browser: the component tree, each with its bodies.
    pub(crate) fn browser_components(&mut self, ui: &mut Ui) {
        let look = self.look();
        let count = self.session.doc.assembly.components.len();
        let open = !self.assembly.folded;
        let note = format!("{count}");
        let header = row(
            ui,
            look,
            ui.scope_id().with("components"),
            (1, Icon::Component),
            "Components",
            (count > 0).then_some(note.as_str()),
            false,
        );
        if header.clicked() {
            self.assembly.folded = open;
        }
        if !open {
            return;
        }
        if count == 0 {
            ui.horizontal(|ui| {
                ui.add_space(4.0 + 2.0 * 14.0 + 16.0);
                ui.label(
                    egui::RichText::new("New Component groups bodies into parts")
                        .size(widgets::T_SMALL)
                        .color(self.look().c(|t| t.text_muted)),
                );
            });
            return;
        }
        // Depth-first from the root, bounded by the component count.
        let mut stack: Vec<(u32, usize)> = vec![(ROOT, 2)];
        let mut steps = 0;
        while let Some((id, depth)) = stack.pop() {
            steps += 1;
            if steps > count + 1 {
                break;
            }
            self.component_row(ui, id, depth);
            let children: Vec<u32> = self
                .session
                .doc
                .assembly
                .components
                .iter()
                .filter(|c| c.parent == id && c.id != id)
                .map(|c| c.id)
                .rev()
                .collect();
            stack.extend(children.into_iter().map(|c| (c, depth + 1)));
        }
    }

    /// One component's row, its context menu, and its bodies under it.
    fn component_row(&mut self, ui: &mut Ui, id: u32, depth: usize) {
        let look = self.look();
        let asm = &self.session.doc.assembly;
        let c = asm.component(id).ok().cloned();
        let name = match &c {
            Some(c) => c.name.clone(),
            None => self.design_name(),
        };
        let active = asm.active == id;
        let mut notes: Vec<&str> = Vec::new();
        if active {
            notes.push("active");
        }
        if c.as_ref().is_some_and(|c| c.grounded) {
            notes.push("grounded");
        }
        if c.as_ref().is_some_and(|c| c.occurrence_of.is_some()) {
            notes.push("copy");
        }
        if asm.driver_of(id).is_some() {
            notes.push("jointed");
        }
        let note = notes.join(" · ");
        let r = row(
            ui,
            look,
            ui.scope_id().with(("component", id)),
            (
                depth,
                if id == ROOT {
                    Icon::Body
                } else {
                    Icon::Component
                },
            ),
            &name,
            (!note.is_empty()).then_some(note.as_str()),
            active,
        )
        .on_hover_text("Double-click to activate: new features go into the active component");
        let copy = c.as_ref().is_some_and(|c| c.occurrence_of.is_some());
        let mut run: Option<(&str, serde_json::Value)> = None;
        if r.double_clicked() && !copy {
            run = Some(("component.activate", json!({ "component": id })));
        }
        if let Some(comp) = &c {
            let (grounded, tinted) = (comp.grounded, comp.tint);
            r.context_menu(|ui| {
                if widgets::menu_item(ui, look, Icon::Edit, label("component.activate"), !copy)
                    .clicked()
                {
                    run = Some(("component.activate", json!({ "component": id })));
                }
                let ground_text = if grounded { "Unground" } else { "Ground" };
                if widgets::menu_item(ui, look, Icon::Ground, ground_text, true).clicked() {
                    run = Some((
                        "component.ground",
                        json!({ "component": id, "grounded": !grounded }),
                    ));
                }
                let tint_text = if tinted { "Plain Colour" } else { "Tint" };
                if widgets::menu_item(ui, look, Icon::Eye, tint_text, true).clicked() {
                    run = Some((
                        "component.tint",
                        json!({ "component": id, "tint": !tinted }),
                    ));
                }
                if widgets::menu_item(ui, look, Icon::Component, label("component.copy"), true)
                    .clicked()
                {
                    run = Some(("component.copy", json!({ "component": id })));
                }
                if widgets::menu_item(ui, look, Icon::Delete, label("component.delete"), true)
                    .clicked()
                {
                    run = Some(("component.delete", json!({ "component": id })));
                }
                if run.is_some() {
                    ui.close();
                }
            });
        }
        let bodies: Vec<(u32, String)> = self
            .session
            .solids()
            .iter()
            .filter(|s| s.component == id)
            .filter_map(|s| s.scene_id.map(|b| (b, s.name.clone())))
            .collect();
        let selected = self.session.scene.selection.map(|h| h.body);
        for (b, name) in bodies {
            row(
                ui,
                look,
                ui.scope_id().with(("component body", id, b)),
                (depth + 1, Icon::Body),
                &name,
                None,
                selected == Some(b),
            );
        }
        if let Some((cmd, p)) = run {
            self.run(cmd, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_picks_and_ok_make_a_revolute_joint() {
        let mut app = ChamferaApp::headless();
        // Two 40 x 20 x 4 plates side by side, each its own component.
        for (x1, x2) in [(0.0, 40.0), (-40.0, 0.0)] {
            let r = app.run("sketch.create", json!({ "plane": "xy" })).unwrap();
            app.run(
                "sketch.rectangle",
                json!({ "x1": x1, "y1": 0, "x2": x2, "y2": 20 }),
            );
            app.run("sketch.finish", json!({}));
            let f = app
                .run(
                    "solid.extrude",
                    json!({ "sketch": r["sketch"], "distance": 4,
                            "profiles": [{ "x": (x1 + x2) / 2.0, "y": 10 }] }),
                )
                .unwrap();
            app.run("component.new", json!({ "from_bodies": [f["feature"]] }));
        }
        app.run("component.ground", json!({ "component": 1 }));
        // The two picks, as the viewport would make them: the second plate's top face
        // moves to the first plate's top face.
        let face = |c: u32, f: u32| JointOrigin {
            component: c,
            at: OriginAt::Face {
                face: format!("{f}:top"),
            },
        };
        let frame = Frame::XY;
        app.assembly.joint = Some(JointTool {
            moving: Some((face(2, 4), frame)),
            target: Some((face(1, 2), frame)),
            kind: JointKind::Revolute,
            value: "90".into(),
            ..JointTool::default()
        });
        assert_eq!(app.joint_overlays()[0].points.len(), 2);
        let r = app.run("joint.create", app.joint_params()).unwrap();
        assert_eq!(r["type"], json!("revolute"));
        // Top centres (-20, 10, 4) of B onto (20, 10, 4) of A, then a quarter turn about
        // +z there: B's 40 x 20 footprint becomes 20 x 40 centred on (20, 10).
        let b = &r["components"][1]["bodies"][0]["bounds"];
        let min = (
            b["min"]["x"].as_f64().unwrap(),
            b["min"]["y"].as_f64().unwrap(),
        );
        let max = (
            b["max"]["x"].as_f64().unwrap(),
            b["max"]["y"].as_f64().unwrap(),
        );
        assert!(
            (min.0 - 10.0).abs() < 1e-9 && (max.0 - 30.0).abs() < 1e-9,
            "{min:?}"
        );
        assert!(
            (min.1 + 10.0).abs() < 1e-9 && (max.1 - 30.0).abs() < 1e-9,
            "{max:?}"
        );
    }
}

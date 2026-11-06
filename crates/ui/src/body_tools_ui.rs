//! The body tools of the SOLID ribbon's MODIFY group: Combine, Split Body, Split Face and
//! Thicken, one small dialog each. Combine and Split Body act on bodies (the selected ones
//! when the tool opens; clicking a body picks or unpicks it, the first being Combine's
//! target), Split Face and Thicken on faces (clicked). Acts only through `select.pick`,
//! `solid.combine`, `solid.split_body`, `solid.split_face` and `solid.thicken`.

use crate::icons::Icon;
use crate::solid_ui::operation_name;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{FeatureKind, Operation, PlaneRef, SplitFaceTool, ThickenDirection};
use serde_json::{Value, json};

/// Which body tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyToolKind {
    Combine,
    SplitBody,
    SplitFace,
    Thicken,
}

impl BodyToolKind {
    pub const ALL: [BodyToolKind; 4] = [
        BodyToolKind::Combine,
        BodyToolKind::SplitBody,
        BodyToolKind::SplitFace,
        BodyToolKind::Thicken,
    ];

    pub fn command(self) -> &'static str {
        match self {
            BodyToolKind::Combine => "solid.combine",
            BodyToolKind::SplitBody => "solid.split_body",
            BodyToolKind::SplitFace => "solid.split_face",
            BodyToolKind::Thicken => "solid.thicken",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            BodyToolKind::Combine => Icon::Combine,
            BodyToolKind::SplitBody => Icon::SplitBody,
            BodyToolKind::SplitFace => Icon::SplitFace,
            BodyToolKind::Thicken => Icon::Thicken,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            BodyToolKind::Combine => {
                "Combine\nJoin, cut or intersect bodies: click the target, then the tools"
            }
            BodyToolKind::SplitBody => "Split Body\nCut bodies in two with a plane",
            BodyToolKind::SplitFace => {
                "Split Face\nCut flat faces along a plane, keeping the shape (for draft and CAM)"
            }
            BodyToolKind::Thicken => "Thicken\nGive flat or cylindrical faces a thickness",
        }
    }

    fn on_bodies(self) -> bool {
        matches!(self, BodyToolKind::Combine | BodyToolKind::SplitBody)
    }
}

/// The operations Combine offers.
const COMBINE_OPS: [Operation; 3] = [Operation::Join, Operation::Cut, Operation::Intersect];
/// The operations Thicken offers.
const THICKEN_OPS: [Operation; 4] = [
    Operation::NewBody,
    Operation::Join,
    Operation::Cut,
    Operation::Intersect,
];
/// The sides Thicken offers.
const SIDES: [(ThickenDirection, &str); 3] = [
    (ThickenDirection::OneSide, "One side"),
    (ThickenDirection::Symmetric, "Symmetric"),
    (ThickenDirection::BothSides, "Both sides"),
];
/// The origin planes a split offers: (command value, what the user reads).
const ORIGIN_PLANES: [(&str, &str); 3] = [
    ("yz", "YZ plane (x = 0)"),
    ("xz", "XZ plane (y = 0)"),
    ("xy", "XY plane (z = 0)"),
];

/// A body tool while it is open.
#[derive(Debug, Clone)]
pub struct BodyTool {
    pub kind: BodyToolKind,
    /// Combine: the target first, then the tools. Split Body: the bodies.
    pub bodies: Vec<String>,
    /// Split Face and Thicken: the faces, by name (`2:top`).
    pub faces: Vec<String>,
    /// Index into the operations this tool offers.
    pub operation: usize,
    pub keep_tools: bool,
    /// Index into the origin planes, then the construction planes.
    pub plane: usize,
    pub thickness: String,
    /// Index into [`SIDES`].
    pub side: usize,
}

impl BodyTool {
    pub fn new(kind: BodyToolKind) -> BodyTool {
        BodyTool {
            kind,
            bodies: Vec::new(),
            faces: Vec::new(),
            operation: 0,
            keep_tools: false,
            plane: 0,
            thickness: "2 mm".into(),
            side: 0,
        }
    }

    /// The plane parameter for choice `plane`, given the construction planes.
    fn plane_value(&self, constructs: &[(u32, String)]) -> Value {
        match ORIGIN_PLANES.get(self.plane) {
            Some((key, _)) => json!(key),
            None => constructs
                .get(self.plane - ORIGIN_PLANES.len())
                .map_or(json!("yz"), |(id, _)| json!(id)),
        }
    }

    /// The command's parameters.
    pub fn params(&self, constructs: &[(u32, String)]) -> Value {
        match self.kind {
            BodyToolKind::Combine => json!({
                "target": self.bodies.first(),
                "tools": self.bodies.iter().skip(1).collect::<Vec<_>>(),
                "operation": COMBINE_OPS.get(self.operation).unwrap_or(&Operation::Join).key(),
                "keep_tools": self.keep_tools,
            }),
            BodyToolKind::SplitBody => json!({
                "bodies": self.bodies,
                "plane": self.plane_value(constructs),
            }),
            BodyToolKind::SplitFace => json!({
                "faces": self.faces,
                "plane": self.plane_value(constructs),
            }),
            BodyToolKind::Thicken => json!({
                "faces": self.faces,
                "thickness": self.thickness.trim(),
                "sides": SIDES.get(self.side).map_or("one_side", |s| s.0.key()),
                "operation": THICKEN_OPS.get(self.operation).unwrap_or(&Operation::NewBody).key(),
            }),
        }
    }

    /// True when OK can run.
    fn ready(&self) -> bool {
        match self.kind {
            BodyToolKind::Combine => self.bodies.len() >= 2,
            BodyToolKind::SplitBody => !self.bodies.is_empty(),
            BodyToolKind::SplitFace => !self.faces.is_empty(),
            BodyToolKind::Thicken => !self.faces.is_empty() && !self.thickness.trim().is_empty(),
        }
    }
}

/// The body tool buttons of the MODIFY group; the tool clicked to open, if any.
pub(crate) fn ribbon_buttons(
    ui: &mut egui::Ui,
    look: Look,
    has_solids: bool,
    free: bool,
    open: Option<BodyToolKind>,
) -> Option<BodyToolKind> {
    let mut start = None;
    for kind in BodyToolKind::ALL {
        let on = open == Some(kind);
        if widgets::tool_button(
            ui,
            look,
            kind.icon(),
            label(kind.command()),
            has_solids && (free || on),
            on,
            false,
        )
        .on_hover_text(if has_solids {
            kind.tip()
        } else {
            "Build a solid first"
        })
        .clicked()
            && free
        {
            start = Some(kind);
        }
    }
    start
}

/// The timeline's icon and one-line summary of a body tool.
pub(crate) fn details(kind: &FeatureKind) -> (Icon, String) {
    let plane = |p: &PlaneRef| match p {
        PlaneRef::Origin { plane } => match plane {
            chamfera_doc::OriginPlane::Xy => "XY plane".to_owned(),
            chamfera_doc::OriginPlane::Xz => "XZ plane".to_owned(),
            chamfera_doc::OriginPlane::Yz => "YZ plane".to_owned(),
        },
        PlaneRef::Construct { feature } => format!("plane {feature}"),
        PlaneRef::Face(f) => format!("face {}:{}", f.feature, f.face),
    };
    let count = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    match kind {
        FeatureKind::Combine(c) => (
            Icon::Combine,
            format!(
                "{} {} with {}{}",
                operation_name(c.operation),
                c.target,
                c.tools.join(", "),
                if c.keep_tools { " · tools kept" } else { "" }
            ),
        ),
        FeatureKind::SplitBody(s) => (
            Icon::SplitBody,
            format!("Split {} by {}", s.bodies.join(", "), plane(&s.plane)),
        ),
        FeatureKind::SplitFace(s) => (
            Icon::SplitFace,
            format!(
                "Split {} by {}",
                count(s.faces.len(), "face"),
                match &s.tool {
                    SplitFaceTool::Plane { plane: p } => plane(p),
                    SplitFaceTool::SketchLine { curve, .. } => format!("sketch line {curve}"),
                }
            ),
        ),
        FeatureKind::Thicken(t) => (
            Icon::Thicken,
            format!(
                "Thicken {} · {} · {}",
                t.thickness,
                count(t.faces.len(), "face"),
                operation_name(t.operation)
            ),
        ),
        FeatureKind::Remove(r) => (Icon::Delete, format!("Remove {}", r.bodies.join(", "))),
        _ => (Icon::Body, String::new()),
    }
}

/// A labelled choice between named options in a dialog grid.
fn choice(ui: &mut egui::Ui, look: Look, name: &str, value: &mut usize, options: &[String]) {
    widgets::field_label(ui, look, name);
    let current = options.get(*value).cloned().unwrap_or_default();
    egui::ComboBox::from_id_salt(("body tool", name))
        .selected_text(current)
        .width(180.0)
        .show_ui(ui, |ui| {
            for (i, o) in options.iter().enumerate() {
                ui.selectable_value(value, i, o);
            }
        });
    ui.end_row();
}

fn picked_text(n: usize, none: &str) -> String {
    match n {
        0 => none.to_owned(),
        1 => "1 selected".to_owned(),
        n => format!("{n} selected"),
    }
}

impl ChamferaApp {
    /// Opens a body tool, starting from the selected bodies (or face).
    pub(crate) fn open_body_tool(&mut self, kind: BodyToolKind) {
        let mut tool = BodyTool::new(kind);
        if kind.on_bodies() {
            tool.bodies = self.session.selected_solid_names();
        } else {
            tool.faces = self.session.selected_face().into_iter().collect();
        }
        self.solid.body_tool = Some(tool);
        self.message = Some((
            match kind {
                BodyToolKind::Combine => "Click the target body, then the tool bodies",
                BodyToolKind::SplitBody => "Click the bodies to split and pick the plane",
                BodyToolKind::SplitFace => "Click the flat faces to split and pick the plane",
                BodyToolKind::Thicken => "Click the faces to thicken",
            }
            .into(),
            false,
        ));
    }

    /// A click in the viewport while a body tool is open: picks or unpicks the body or
    /// face there.
    pub(crate) fn body_tool_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let on_bodies = self
            .solid
            .body_tool
            .as_ref()
            .is_some_and(|t| t.kind.on_bodies());
        let picked: Vec<String> = if on_bodies {
            self.run(
                "select.pick",
                json!({ "x": x, "y": y, "width": w, "height": h }),
            );
            self.session.selected_body_names()
        } else {
            self.session.face_ref_at(x, y, w, h).into_iter().collect()
        };
        let Some(tool) = self.solid.body_tool.as_mut() else {
            return;
        };
        let list = if on_bodies {
            &mut tool.bodies
        } else {
            &mut tool.faces
        };
        for name in picked {
            if let Some(i) = list.iter().position(|b| *b == name) {
                list.remove(i);
            } else {
                list.push(name);
            }
        }
    }

    /// Runs the open tool's command; closes the tool when it worked.
    pub(crate) fn body_tool_ok(&mut self) -> Option<Value> {
        let tool = self.solid.body_tool.clone()?;
        let planes = self.construct_planes();
        let r = self.run(tool.kind.command(), tool.params(&planes))?;
        let shown: Vec<String> = r["solids"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|s| {
                        Some(format!(
                            "{} {:.2} mm³",
                            s["name"].as_str()?,
                            s["volume"].as_f64()?
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.message = Some((
            format!(
                "{} · {}",
                r["name"].as_str().unwrap_or(label(tool.kind.command())),
                shown.join(", ")
            ),
            false,
        ));
        self.solid.body_tool = None;
        Some(r)
    }

    /// The open body tool's dialog.
    pub(crate) fn body_tool_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let planes = self.construct_planes();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.body_tool.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = label(tool.kind.command());
        let ready = tool.ready();
        let plane_names: Vec<String> = ORIGIN_PLANES
            .iter()
            .map(|(_, n)| (*n).to_owned())
            .chain(planes.iter().map(|(_, n)| n.clone()))
            .collect();
        egui::Window::new(title)
            .id(egui::Id::unique("body tool dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, tool.kind.icon(), title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("body tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| match tool.kind {
                            BodyToolKind::Combine => {
                                widgets::field_label(ui, look, "Target");
                                ui.label(
                                    tool.bodies.first().map_or("Click a body", |s| s.as_str()),
                                );
                                ui.end_row();
                                widgets::field_label(ui, look, "Tools");
                                ui.label(picked_text(
                                    tool.bodies.len().saturating_sub(1),
                                    "Click the tool bodies",
                                ));
                                ui.end_row();
                                let ops: Vec<String> = COMBINE_OPS
                                    .iter()
                                    .map(|o| operation_name(*o).to_owned())
                                    .collect();
                                choice(ui, look, "Operation", &mut tool.operation, &ops);
                                widgets::field_label(ui, look, "Keep tools");
                                ui.checkbox(&mut tool.keep_tools, "Keep the tool bodies");
                                ui.end_row();
                            }
                            BodyToolKind::SplitBody => {
                                widgets::field_label(ui, look, "Bodies");
                                ui.label(picked_text(tool.bodies.len(), "Click the bodies"));
                                ui.end_row();
                                choice(ui, look, "Splitting plane", &mut tool.plane, &plane_names);
                            }
                            BodyToolKind::SplitFace => {
                                widgets::field_label(ui, look, "Faces");
                                ui.label(picked_text(tool.faces.len(), "Click flat faces"));
                                ui.end_row();
                                choice(ui, look, "Splitting plane", &mut tool.plane, &plane_names);
                            }
                            BodyToolKind::Thicken => {
                                widgets::field_label(ui, look, "Faces");
                                ui.label(picked_text(tool.faces.len(), "Click the faces"));
                                ui.end_row();
                                widgets::field_label(ui, look, "Thickness");
                                let edit = ui.add_sized(
                                    [180.0, 26.0],
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
                                let sides: Vec<String> =
                                    SIDES.iter().map(|s| s.1.to_owned()).collect();
                                choice(ui, look, "Direction", &mut tool.side, &sides);
                                let ops: Vec<String> = THICKEN_OPS
                                    .iter()
                                    .map(|o| operation_name(*o).to_owned())
                                    .collect();
                                choice(ui, look, "Operation", &mut tool.operation, &ops);
                            }
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
            self.body_tool_ok();
        }
        if cancel {
            self.solid.body_tool = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(app: &mut ChamferaApp) {
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
    }

    #[test]
    fn the_split_body_tool_splits_the_selected_box_by_the_plane_picked() {
        let mut app = ChamferaApp::headless();
        block(&mut app);
        app.run(
            "construct.plane_offset",
            json!({ "base": "yz", "distance": 20 }),
        );
        let id = app.session.solids()[0].scene_id.unwrap();
        app.session.scene.selected = vec![id];
        app.open_body_tool(BodyToolKind::SplitBody);
        let tool = app.solid.body_tool.as_mut().unwrap();
        assert_eq!(tool.bodies, vec!["Extrude1"]);
        // The construction plane follows the three origin planes.
        tool.plane = 3;
        assert!(app.body_tool_ok().is_some());
        assert!(app.solid.body_tool.is_none());
        let v: Vec<f64> = app
            .session
            .solids()
            .iter()
            .map(|s| s.body.volume().unwrap())
            .collect();
        // 20 * 30 * 10 and 30 * 30 * 10.
        assert_eq!(v.len(), 2);
        assert!((v[0] - 6000.0).abs() < 1e-6 && (v[1] - 9000.0).abs() < 1e-6);
    }

    #[test]
    fn the_thicken_tool_runs_with_its_side_and_operation() {
        let mut app = ChamferaApp::headless();
        block(&mut app);
        app.open_body_tool(BodyToolKind::Thicken);
        let tool = app.solid.body_tool.as_mut().unwrap();
        assert!(!tool.ready());
        tool.faces = vec!["2:top".into()];
        tool.thickness = "3 mm".into();
        tool.side = 1;
        tool.operation = 0;
        let p = tool.params(&[]);
        assert_eq!(p["sides"], "symmetric");
        assert_eq!(p["operation"], "new_body");
        app.body_tool_ok().unwrap();
        // 50 * 30 * 3 = 4500, from z 8.5 to 11.5.
        let t = app
            .session
            .solids()
            .iter()
            .find(|s| s.name == "Thicken1")
            .unwrap();
        assert!((t.body.volume().unwrap() - 4500.0).abs() < 1e-6);
        let (icon, text) = details(&app.session.doc.features.last().unwrap().kind);
        assert_eq!(icon, Icon::Thicken);
        assert!(text.starts_with("Thicken 3 mm"), "{text}");
    }

    #[test]
    fn combine_params_put_the_target_first() {
        let mut tool = BodyTool::new(BodyToolKind::Combine);
        tool.bodies = vec!["A".into(), "B".into(), "C".into()];
        tool.operation = 1;
        let p = tool.params(&[]);
        assert_eq!(p["target"], "A");
        assert_eq!(p["tools"], json!(["B", "C"]));
        assert_eq!(p["operation"], "cut");
    }
}

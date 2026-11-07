//! The Mirror, Move, Rectangular Pattern and Circular Pattern tools: a TRANSFORM group in
//! the SOLID ribbon and one small dialog each. Mirror and Move act on bodies (the selected
//! one when the tool opens; clicking a body picks or unpicks it), the patterns on a
//! feature. Acts only through `select.pick`, `solid.mirror`, `solid.move`,
//! `solid.pattern_linear` and `solid.pattern_circular`.

use crate::icons::Icon;
use crate::solid_ui::operation_name;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{FeatureKind, Mirror, MirrorPlane, Move, MoveTransform, Operation};
use serde_json::{Value, json};

/// Which tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Mirror,
    Move,
    PatternLinear,
    PatternCircular,
}

impl TransformKind {
    pub(crate) fn command(self) -> &'static str {
        match self {
            TransformKind::Mirror => "solid.mirror",
            TransformKind::Move => "solid.move",
            TransformKind::PatternLinear => "solid.pattern_linear",
            TransformKind::PatternCircular => "solid.pattern_circular",
        }
    }

    fn icon(self) -> Icon {
        match self {
            TransformKind::Mirror => Icon::MirrorBody,
            TransformKind::Move => Icon::Move,
            TransformKind::PatternLinear => Icon::PatternRectangular,
            TransformKind::PatternCircular => Icon::PatternCircular,
        }
    }

    fn on_bodies(self) -> bool {
        matches!(self, TransformKind::Mirror | TransformKind::Move)
    }
}

/// The origin planes a mirror offers: (command key, what the user reads).
const PLANES: [(&str, &str); 3] = [
    ("yz", "YZ plane (x = 0)"),
    ("xz", "XZ plane (y = 0)"),
    ("xy", "XY plane (z = 0)"),
];

/// The world axes a rotation or pattern can follow.
const AXES: [(&str, [f64; 3]); 3] = [
    ("X axis", [1.0, 0.0, 0.0]),
    ("Y axis", [0.0, 1.0, 0.0]),
    ("Z axis", [0.0, 0.0, 1.0]),
];

/// A Mirror, Move or pattern tool while it is open.
pub struct TransformTool {
    pub kind: TransformKind,
    /// Mirror and Move: the bodies, by name.
    pub bodies: Vec<String>,
    /// Mirror: an index into [`PLANES`], and join the mirror to what it touches.
    pub plane: usize,
    pub join: bool,
    /// Move: rotate instead of translate; the distances, the axis (index into [`AXES`],
    /// through the origin) and the angle; keep the originals.
    pub rotate: bool,
    pub dx: String,
    pub dy: String,
    pub dz: String,
    pub axis: usize,
    /// Circular Pattern: a construction axis to turn around instead of `axis`.
    pub axis_feature: Option<u32>,
    pub angle: String,
    pub copy: bool,
    /// Patterns: the feature to repeat, how many, and the spacing (linear) along `axis`.
    pub feature: Option<u32>,
    pub count: String,
    pub spacing: String,
}

impl TransformTool {
    pub fn new(kind: TransformKind, bodies: Vec<String>, feature: Option<u32>) -> TransformTool {
        TransformTool {
            kind,
            bodies,
            plane: 0,
            join: false,
            rotate: false,
            dx: "0 mm".into(),
            dy: "0 mm".into(),
            dz: "0 mm".into(),
            axis: if kind == TransformKind::PatternLinear {
                0
            } else {
                2
            },
            axis_feature: None,
            angle: if kind == TransformKind::PatternCircular {
                "360 deg"
            } else {
                "90 deg"
            }
            .into(),
            copy: false,
            feature,
            count: if kind == TransformKind::PatternCircular {
                "6"
            } else {
                "3"
            }
            .into(),
            spacing: "20 mm".into(),
        }
    }

    /// The row of `axes` (from `pattern_axes`) the tool's axis is.
    fn axis_choice(&self, axes: &[(String, Option<u32>)]) -> usize {
        match self.axis_feature {
            Some(id) => axes
                .iter()
                .position(|a| a.1 == Some(id))
                .unwrap_or(self.axis),
            None => self.axis,
        }
    }

    /// Picks row `k` of `axes`: X, Y or Z through the origin, or a construction axis.
    fn set_axis_choice(&mut self, k: usize, axes: &[(String, Option<u32>)]) {
        match axes.get(k) {
            Some((_, Some(id))) => self.axis_feature = Some(*id),
            Some((_, None)) => {
                self.axis = k;
                self.axis_feature = None;
            }
            None => {}
        }
    }

    fn axis_vector(&self) -> Value {
        let [x, y, z] = AXES.get(self.axis).map_or([0.0, 0.0, 1.0], |a| a.1);
        json!({ "x": x, "y": y, "z": z })
    }

    /// The command's parameters, or `None` while something is missing.
    pub fn params(&self) -> Option<Value> {
        let filled = |s: &String| !s.trim().is_empty();
        match self.kind {
            TransformKind::Mirror => {
                let plane = PLANES.get(self.plane)?.0;
                (!self.bodies.is_empty()).then(|| {
                    json!({
                        "bodies": self.bodies,
                        "plane": plane,
                        "operation": if self.join { "join" } else { "new_body" },
                    })
                })
            }
            TransformKind::Move if self.bodies.is_empty() => None,
            TransformKind::Move if self.rotate => filled(&self.angle).then(|| {
                json!({
                    "bodies": self.bodies,
                    "angle": self.angle.trim(),
                    "axis": self.axis_vector(),
                    "copy": self.copy,
                })
            }),
            TransformKind::Move => {
                [&self.dx, &self.dy, &self.dz]
                    .into_iter()
                    .all(filled)
                    .then(|| {
                        json!({
                            "bodies": self.bodies,
                            "dx": self.dx.trim(),
                            "dy": self.dy.trim(),
                            "dz": self.dz.trim(),
                            "copy": self.copy,
                        })
                    })
            }
            TransformKind::PatternLinear => {
                let feature = self.feature?;
                (filled(&self.count) && filled(&self.spacing)).then(|| {
                    json!({
                        "feature": feature,
                        "count": self.count.trim(),
                        "direction": self.axis_vector(),
                        "spacing": self.spacing.trim(),
                    })
                })
            }
            TransformKind::PatternCircular => {
                let feature = self.feature?;
                (filled(&self.count) && filled(&self.angle)).then(|| match self.axis_feature {
                    Some(id) => json!({
                        "feature": feature,
                        "count": self.count.trim(),
                        "axis_feature": id,
                        "angle": self.angle.trim(),
                    }),
                    None => json!({
                        "feature": feature,
                        "count": self.count.trim(),
                        "origin": { "x": 0.0, "y": 0.0, "z": 0.0 },
                        "axis": self.axis_vector(),
                        "angle": self.angle.trim(),
                    }),
                })
            }
        }
    }
}

/// A short description of a mirror for the timeline.
pub(crate) fn mirror_details(m: &Mirror) -> String {
    let n = m.bodies.len() + m.features.len();
    let plane = match &m.plane {
        MirrorPlane::Xy => "the XY plane".to_owned(),
        MirrorPlane::Xz => "the XZ plane".to_owned(),
        MirrorPlane::Yz => "the YZ plane".to_owned(),
        MirrorPlane::Face { feature, face } => format!("face {face} of feature {feature}"),
        MirrorPlane::Plane { .. } => "a plane".to_owned(),
    };
    let what = if m.features.is_empty() { "bod" } else { "item" };
    let plural = match (n == 1, m.features.is_empty()) {
        (true, true) => "y",
        (false, true) => "ies",
        (true, false) => "",
        (false, false) => "s",
    };
    format!(
        "Mirror {n} {what}{plural} in {plane} · {}",
        operation_name(m.operation).to_lowercase()
    )
}

/// A short description of a move for the timeline.
pub(crate) fn move_details(m: &Move) -> String {
    let how = match &m.transform {
        MoveTransform::Translate { dx, dy, dz } => format!("by ({dx}, {dy}, {dz})"),
        MoveTransform::Rotate { angle, .. } => format!("turned {angle}"),
        MoveTransform::PointToPoint { .. } => "point to point".to_owned(),
    };
    let verb = if m.copy { "Copy" } else { "Move" };
    format!("{verb} {} · {how}", m.bodies.join(", "))
}

/// Features a pattern can repeat: (id, name). Patterns and moves cannot.
/// What Circular Pattern can turn around: X, Y and Z through the origin (`None`), then
/// the built construction axes (`Some(id)`, followed on rebuild).
fn pattern_axes(app: &ChamferaApp) -> Vec<(String, Option<u32>)> {
    AXES.iter()
        .map(|a| (a.0.to_owned(), None))
        .chain(
            app.construction_axes()
                .into_iter()
                .map(|(id, name)| (name, Some(id))),
        )
        .collect()
}

fn pattern_sources(app: &ChamferaApp) -> Vec<(u32, String)> {
    app.session
        .doc
        .features
        .iter()
        .filter(|f| !matches!(f.kind, FeatureKind::Pattern(_) | FeatureKind::Move(_)))
        .map(|f| (f.id, f.name.clone()))
        .collect()
}

/// One labelled text field in a dialog grid; true when Enter was pressed in it.
fn field(ui: &mut egui::Ui, look: Look, name: &str, value: &mut String, hint: &str) -> bool {
    widgets::field_label(ui, look, name);
    let r = ui.add_sized(
        widgets::FIELD,
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.end_row();
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A labelled choice between named options in a dialog grid.
fn choice(ui: &mut egui::Ui, look: Look, name: &str, value: &mut usize, options: &[&str]) {
    widgets::field_label(ui, look, name);
    let current = options.get(*value).copied().unwrap_or("");
    egui::ComboBox::from_id_salt(("transform", name))
        .selected_text(current)
        .width(widgets::FIELD_WIDTH)
        .show_ui(ui, |ui| {
            for (i, o) in options.iter().enumerate() {
                ui.selectable_value(value, i, *o);
            }
        });
    ui.end_row();
}

impl ChamferaApp {
    /// Opens a tool, with the selected body or the last feature already picked.
    pub(crate) fn open_transform(&mut self, kind: TransformKind) {
        let bodies = if kind.on_bodies() {
            self.session.selected_body_names()
        } else {
            Vec::new()
        };
        let feature = pattern_sources(self).last().map(|f| f.0);
        self.solid.transform = Some(TransformTool::new(kind, bodies, feature));
        let hint = if kind.on_bodies() {
            "Click the bodies to use (click again to unpick), then set the values"
        } else {
            "Choose the feature to repeat and how"
        };
        self.message = Some((hint.into(), false));
    }

    /// A click in the viewport while a tool is open: selects the body there and, for
    /// Mirror and Move, picks it (or unpicks it when it was picked).
    pub(crate) fn transform_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.run(
            "select.pick",
            json!({ "x": x, "y": y, "width": w, "height": h }),
        );
        let picked = self.session.selected_body_names();
        let Some(tool) = self.solid.transform.as_mut() else {
            return;
        };
        if !tool.kind.on_bodies() {
            return;
        }
        for name in picked {
            if let Some(i) = tool.bodies.iter().position(|b| *b == name) {
                tool.bodies.remove(i);
            } else {
                tool.bodies.push(name);
            }
        }
    }

    /// Runs the open tool's command; closes the tool when it worked.
    pub(crate) fn transform_ok(&mut self) -> Option<Value> {
        let tool = self.solid.transform.as_ref()?;
        let command = tool.kind.command();
        let params = tool.params()?;
        let r = self.run(command, params)?;
        let v: f64 = r["solids"]
            .as_array()
            .map_or(0.0, |s| s.iter().filter_map(|x| x["volume"].as_f64()).sum());
        self.message = Some((
            format!(
                "{} · volume {}",
                r["name"].as_str().unwrap_or(label(command)),
                self.units().volume(v),
            ),
            false,
        ));
        self.solid.transform = None;
        Some(r)
    }

    /// The open tool's dialog.
    pub(crate) fn transform_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        if self.solid.transform.is_none() {
            return;
        }
        let pos = self.tool_dialog_pos(WIDTH);
        let sources = pattern_sources(self);
        let axes = pattern_axes(self);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.transform.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = label(tool.kind.command());
        let icon = tool.kind.icon();
        egui::Window::new(title)
            .id(egui::Id::unique("transform dialog"))
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
                    egui::Grid::new("transform grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            ok |= transform_fields(ui, look, tool, &sources, &axes);
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    let ready = tool.params().is_some();
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
            self.transform_ok();
        }
        if cancel {
            self.solid.transform = None;
        }
    }
}

/// The rows of the open tool's dialog; true when Enter was pressed in a field.
fn transform_fields(
    ui: &mut egui::Ui,
    look: Look,
    tool: &mut TransformTool,
    sources: &[(u32, String)],
    pattern_axes: &[(String, Option<u32>)],
) -> bool {
    let mut enter = false;
    if tool.kind.on_bodies() {
        widgets::field_label(ui, look, "Bodies");
        let (text, colour) = match tool.bodies.len() {
            0 => ("Click a body".to_owned(), look.c(|t| t.text_muted)),
            1 => (tool.bodies.join(""), look.c(|t| t.text)),
            n => (format!("{n} bodies"), look.c(|t| t.text)),
        };
        ui.label(egui::RichText::new(text).color(colour))
            .on_hover_text("Click a picked body again to unpick it");
        ui.end_row();
    } else {
        widgets::field_label(ui, look, "Feature");
        let current = sources
            .iter()
            .find(|s| Some(s.0) == tool.feature)
            .map_or("Choose a feature", |s| s.1.as_str());
        egui::ComboBox::from_id_salt("pattern feature")
            .selected_text(current)
            .width(widgets::FIELD_WIDTH)
            .show_ui(ui, |ui| {
                for (id, name) in sources {
                    ui.selectable_value(&mut tool.feature, Some(*id), name);
                }
            });
        ui.end_row();
    }
    let axes: Vec<&str> = AXES.iter().map(|a| a.0).collect();
    match tool.kind {
        TransformKind::Mirror => {
            let planes: Vec<&str> = PLANES.iter().map(|p| p.1).collect();
            choice(ui, look, "Plane", &mut tool.plane, &planes);
            let mut op = usize::from(tool.join);
            let ops = [
                operation_name(Operation::NewBody),
                operation_name(Operation::Join),
            ];
            choice(ui, look, "Operation", &mut op, &ops);
            tool.join = op == 1;
        }
        TransformKind::Move => {
            let mut mode = usize::from(tool.rotate);
            choice(ui, look, "Type", &mut mode, &["Translate", "Rotate"]);
            tool.rotate = mode == 1;
            if tool.rotate {
                choice(ui, look, "Axis", &mut tool.axis, &axes);
                enter |= field(ui, look, "Angle", &mut tool.angle, "90 deg");
            } else {
                enter |= field(ui, look, "X distance", &mut tool.dx, "0 mm");
                enter |= field(ui, look, "Y distance", &mut tool.dy, "0 mm");
                enter |= field(ui, look, "Z distance", &mut tool.dz, "0 mm");
            }
            widgets::field_label(ui, look, "Copy");
            ui.checkbox(&mut tool.copy, "Keep the original");
            ui.end_row();
        }
        TransformKind::PatternLinear => {
            choice(ui, look, "Direction", &mut tool.axis, &axes);
            enter |= field(ui, look, "Count", &mut tool.count, "3");
            enter |= field(ui, look, "Spacing", &mut tool.spacing, "20 mm");
        }
        TransformKind::PatternCircular => {
            let names: Vec<&str> = pattern_axes.iter().map(|a| a.0.as_str()).collect();
            let mut k = tool.axis_choice(pattern_axes);
            choice(ui, look, "Axis", &mut k, &names);
            tool.set_axis_choice(k, pattern_axes);
            enter |= field(ui, look, "Count", &mut tool.count, "6");
            enter |= field(ui, look, "Angle", &mut tool.angle, "360 deg");
        }
    }
    enter
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A headless app with a box x 0..50, y 0..30, z 0..10 and the home view.
    fn app_with_box() -> ChamferaApp {
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
        app.run("view.home", json!({ "width": 800.0, "height": 600.0 }));
        app
    }

    /// Where a world point is on an 800 x 600 viewport.
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
    fn clicking_the_box_and_ok_mirrors_it_joined_into_one_body() {
        let mut app = app_with_box();
        app.open_transform(TransformKind::Mirror);
        assert!(app.solid.transform.as_ref().unwrap().params().is_none());
        // The middle of the top face, (25, 15, 10).
        let (x, y) = pixel(&app, chamfera_geom::Point3::new(25.0, 15.0, 10.0));
        app.transform_click(x, y, 800.0, 600.0);
        assert_eq!(app.solid.transform.as_ref().unwrap().bodies, ["Extrude1"]);
        app.solid.transform.as_mut().unwrap().join = true;
        let r = app.transform_ok().unwrap();
        // Joined with its mirror in x = 0: one 100 x 30 x 10 body, 30000.
        assert_eq!(app.session.solids().len(), 1);
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - 30_000.0).abs() < 1e-6, "{v}");
        assert!(app.solid.transform.is_none());
    }

    #[test]
    fn a_move_dialog_with_the_box_selected_copies_it() {
        let mut app = app_with_box();
        let (x, y) = pixel(&app, chamfera_geom::Point3::new(25.0, 15.0, 10.0));
        app.run(
            "select.pick",
            json!({ "x": x, "y": y, "width": 800.0, "height": 600.0 }),
        );
        app.open_transform(TransformKind::Move);
        let tool = app.solid.transform.as_mut().unwrap();
        assert_eq!(tool.bodies, ["Extrude1"], "the selection is picked");
        tool.dx = "60 mm".into();
        tool.copy = true;
        app.transform_ok().unwrap();
        // The copy sits at x 60..110; the original stays.
        assert_eq!(app.session.solids().len(), 2);
        let b = app.session.solids()[1].body.bounding_box().unwrap();
        assert!((b.min.x - 60.0).abs() < 1e-9 && (b.max.x - 110.0).abs() < 1e-9);
    }

    #[test]
    fn the_pattern_dialogs_repeat_the_last_feature() {
        let mut app = app_with_box();
        app.open_transform(TransformKind::PatternLinear);
        let tool = app.solid.transform.as_ref().unwrap();
        assert_eq!(tool.feature, Some(2));
        app.transform_ok().unwrap();
        // 3 boxes 20 apart along x would overlap; as new bodies they are 3 bodies of 15000.
        assert_eq!(app.session.solids().len(), 3);
        app.open_transform(TransformKind::PatternCircular);
        let tool = app.solid.transform.as_mut().unwrap();
        tool.feature = Some(2);
        tool.count = "4".into();
        app.transform_ok().unwrap();
        // 3 more copies of the box at 90, 180 and 270 degrees about z.
        assert_eq!(app.session.solids().len(), 6);
    }

    /// Circular Pattern offers the built construction axes after X, Y and Z and passes
    /// `axis_feature` for one. The box x 0..50, y 0..30 turned 180 degrees about a
    /// vertical axis through (60, 15) (the planes x = 60 and y = 15 meet there) lands at
    /// x 70..120, y 0..30: centre (95, 15) mirrored through (60, 15) from (25, 15).
    #[test]
    fn the_circular_pattern_dialog_turns_around_a_construction_axis() {
        let mut app = app_with_box();
        let px = app
            .run(
                "construct.plane_offset",
                json!({ "base": "yz", "distance": 60 }),
            )
            .unwrap()["feature"]
            .clone();
        let py = app
            // The XZ plane's normal is -Y, so y = 15 is an offset of -15.
            .run(
                "construct.plane_offset",
                json!({ "base": "xz", "distance": -15 }),
            )
            .unwrap()["feature"]
            .clone();
        let axis = app
            .run("construct.axis_two_planes", json!({ "planes": [px, py] }))
            .unwrap()["feature"]
            .as_u64()
            .unwrap() as u32;
        app.open_transform(TransformKind::PatternCircular);
        let axes = pattern_axes(&app);
        let k = axes
            .iter()
            .position(|(_, id)| *id == Some(axis))
            .expect("the construction axis is not offered");
        let tool = app.solid.transform.as_mut().unwrap();
        tool.feature = Some(2);
        tool.count = "2".into();
        tool.set_axis_choice(k, &axes);
        let params = tool.params().unwrap();
        assert_eq!(params["axis_feature"], json!(axis));
        app.transform_ok().unwrap();
        assert_eq!(app.session.solids().len(), 2);
        let b = app.session.solids()[1].body.bounding_box().unwrap();
        assert!(
            (b.min.x - 70.0).abs() < 1e-6 && (b.max.x - 120.0).abs() < 1e-6,
            "{b:?}"
        );
        assert!(
            b.min.y.abs() < 1e-6 && (b.max.y - 30.0).abs() < 1e-6,
            "{b:?}"
        );
    }
}

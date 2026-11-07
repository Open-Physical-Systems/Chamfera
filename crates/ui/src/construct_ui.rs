//! Construction geometry in the UI: the CONSTRUCT ribbon group (planes, axes, points), its
//! dialogs, the Browser's Construction folder, the timeline's details and the edit dialog.
//! Acts only through `construct.*`, `sketch.create` and `feature.*` commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{ConstructKind, Construction, FeatureKind};
use serde_json::{Value, json};

/// The construction tools, one per `construct.*` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PlaneOffset,
    PlaneAngle,
    PlaneMidplane,
    PlaneThreePoints,
    PlaneTangent,
    AxisCylinder,
    AxisEdge,
    AxisTwoPoints,
    AxisTwoPlanes,
    PointVertex,
    PointCentre,
    PointCoordinates,
}

/// What one input of a tool takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// An origin plane, a construction plane or a flat face.
    Plane,
    /// An origin axis, a construction axis or a straight edge.
    Axis,
    /// The origin, a construction point or a vertex.
    Point,
    /// A cylindrical (or conical) face.
    CurvedFace,
    /// A straight edge.
    Edge,
    /// A circle or arc edge.
    CircleEdge,
    /// A vertex (the faces meeting there).
    Vertex,
}

impl Slot {
    /// What to click for this input.
    fn prompt(self) -> &'static str {
        match self {
            Slot::Plane => "Click a flat face",
            Slot::Axis | Slot::Edge => "Click near a straight edge",
            Slot::Point | Slot::Vertex => "Click near a corner",
            Slot::CurvedFace => "Click a cylindrical face",
            Slot::CircleEdge => "Click near a circular edge",
        }
    }
}

impl Kind {
    pub const PLANES: [Kind; 5] = [
        Kind::PlaneOffset,
        Kind::PlaneAngle,
        Kind::PlaneMidplane,
        Kind::PlaneThreePoints,
        Kind::PlaneTangent,
    ];
    pub const AXES: [Kind; 4] = [
        Kind::AxisCylinder,
        Kind::AxisEdge,
        Kind::AxisTwoPoints,
        Kind::AxisTwoPlanes,
    ];
    pub const POINTS: [Kind; 3] = [Kind::PointVertex, Kind::PointCentre, Kind::PointCoordinates];

    pub fn command(self) -> &'static str {
        match self {
            Kind::PlaneOffset => "construct.plane_offset",
            Kind::PlaneAngle => "construct.plane_angle",
            Kind::PlaneMidplane => "construct.plane_midplane",
            Kind::PlaneThreePoints => "construct.plane_three_points",
            Kind::PlaneTangent => "construct.plane_tangent",
            Kind::AxisCylinder => "construct.axis_cylinder",
            Kind::AxisEdge => "construct.axis_edge",
            Kind::AxisTwoPoints => "construct.axis_two_points",
            Kind::AxisTwoPlanes => "construct.axis_two_planes",
            Kind::PointVertex => "construct.point_vertex",
            Kind::PointCentre => "construct.point_centre",
            Kind::PointCoordinates => "construct.point_coordinates",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Kind::PlaneOffset => Icon::ConstructPlaneOffset,
            Kind::PlaneAngle | Kind::PlaneTangent => Icon::ConstructPlaneAngle,
            Kind::PlaneMidplane => Icon::ConstructMidplane,
            Kind::PlaneThreePoints => Icon::ConstructPlanePoints,
            Kind::AxisCylinder | Kind::AxisEdge | Kind::AxisTwoPoints | Kind::AxisTwoPlanes => {
                Icon::ConstructAxis
            }
            Kind::PointVertex | Kind::PointCentre | Kind::PointCoordinates => Icon::ConstructPoint,
        }
    }

    /// The tool's inputs and their labels.
    fn slots(self) -> &'static [(Slot, &'static str)] {
        match self {
            Kind::PlaneOffset => &[(Slot::Plane, "Plane")],
            Kind::PlaneAngle => &[(Slot::Axis, "Axis")],
            Kind::PlaneMidplane => &[(Slot::Plane, "First"), (Slot::Plane, "Second")],
            Kind::PlaneThreePoints => &[
                (Slot::Point, "Point 1"),
                (Slot::Point, "Point 2"),
                (Slot::Point, "Point 3"),
            ],
            Kind::PlaneTangent => &[(Slot::CurvedFace, "Face"), (Slot::Point, "Near")],
            Kind::AxisCylinder => &[(Slot::CurvedFace, "Face")],
            Kind::AxisEdge => &[(Slot::Edge, "Edge")],
            Kind::AxisTwoPoints => &[(Slot::Point, "From"), (Slot::Point, "To")],
            Kind::AxisTwoPlanes => &[(Slot::Plane, "First"), (Slot::Plane, "Second")],
            Kind::PointVertex => &[(Slot::Vertex, "Vertex")],
            Kind::PointCentre => &[(Slot::CircleEdge, "Edge")],
            Kind::PointCoordinates => &[],
        }
    }

    /// True when the tool needs a solid to pick from.
    pub(crate) fn needs_solid(self) -> bool {
        matches!(
            self,
            Kind::PlaneTangent
                | Kind::AxisCylinder
                | Kind::AxisEdge
                | Kind::PointVertex
                | Kind::PointCentre
        )
    }

    pub(crate) fn tip(self) -> &'static str {
        match self {
            Kind::PlaneOffset => "A plane parallel to a plane or flat face, a distance away",
            Kind::PlaneAngle => "A plane through a line, turned by an angle",
            Kind::PlaneMidplane => "A plane halfway between two parallel planes or faces",
            Kind::PlaneThreePoints => "A plane through three points",
            Kind::PlaneTangent => "A plane touching a cylindrical face",
            Kind::AxisCylinder => "The axis of a cylindrical or conical face",
            Kind::AxisEdge => "An axis along a straight edge",
            Kind::AxisTwoPoints => "An axis through two points",
            Kind::AxisTwoPlanes => "An axis where two planes cross",
            Kind::PointVertex => "A point at a corner of a solid",
            Kind::PointCentre => "A point at the centre of a circular edge",
            Kind::PointCoordinates => "A point at X, Y, Z coordinates",
        }
    }
}

/// A filled input: the reference in command form, and what the dialog shows.
#[derive(Debug, Clone)]
struct Pick {
    value: Value,
    text: String,
}

/// A construction tool while its dialog is open.
pub struct ConstructTool {
    pub kind: Kind,
    picks: Vec<Option<Pick>>,
    /// The input that viewport clicks fill.
    active: usize,
    pub distance: String,
    pub angle: String,
    pub xyz: [String; 3],
}

impl ConstructTool {
    pub fn new(kind: Kind) -> ConstructTool {
        let mut picks: Vec<Option<Pick>> = kind.slots().iter().map(|_| None).collect();
        // An offset plane starts from the top plane and a plane at an angle from the X axis,
        // so OK works at once.
        if let Some(first) = picks.first_mut() {
            *first = match kind {
                Kind::PlaneOffset => Some(Pick {
                    value: json!("xy"),
                    text: "Top plane (XY)".into(),
                }),
                Kind::PlaneAngle => Some(Pick {
                    value: json!("x"),
                    text: "X axis".into(),
                }),
                _ => None,
            };
        }
        let active = picks.iter().position(Option::is_none).unwrap_or(0);
        ConstructTool {
            kind,
            picks,
            active,
            distance: "10 mm".into(),
            angle: "45 deg".into(),
            xyz: ["0".into(), "0".into(), "0".into()],
        }
    }

    fn pick(&self, i: usize) -> Option<Value> {
        self.picks.get(i)?.as_ref().map(|p| p.value.clone())
    }

    /// The command's parameters, once every input is filled.
    /// `unit`: the design's length unit, written after plain numbers.
    pub fn params(&self, unit: &str) -> Option<Value> {
        let v = |i| self.pick(i);
        Some(match self.kind {
            Kind::PlaneOffset => {
                json!({ "base": v(0)?, "distance": with_unit(&self.distance, unit) })
            }
            Kind::PlaneAngle => json!({ "axis": v(0)?, "angle": with_unit(&self.angle, "deg") }),
            Kind::PlaneMidplane | Kind::AxisTwoPlanes => json!({ "planes": [v(0)?, v(1)?] }),
            Kind::PlaneThreePoints => json!({ "points": [v(0)?, v(1)?, v(2)?] }),
            Kind::PlaneTangent => json!({ "face": v(0)?, "point": v(1)? }),
            Kind::AxisCylinder => json!({ "face": v(0)? }),
            Kind::AxisEdge | Kind::PointCentre => json!({ "edge": v(0)? }),
            Kind::AxisTwoPoints => json!({ "points": [v(0)?, v(1)?] }),
            Kind::PointVertex => json!({ "faces": v(0)? }),
            Kind::PointCoordinates => json!({
                "x": with_unit(&self.xyz[0], unit),
                "y": with_unit(&self.xyz[1], unit),
                "z": with_unit(&self.xyz[2], unit),
            }),
        })
    }
}

/// A plain number gets `unit`; an expression is kept as typed.
fn with_unit(s: &str, unit: &str) -> String {
    let t = s.trim();
    if t.parse::<f64>().is_ok() {
        format!("{t} {unit}")
    } else {
        t.to_owned()
    }
}

/// A construction being edited from the timeline or the Browser.
pub struct ConstructEdit {
    feature: u32,
    /// The distance (offset plane) or angle (plane at an angle), if it has one.
    value: Option<(&'static str, String)>,
    suppressed: bool,
}

#[derive(Default)]
pub struct ConstructUi {
    pub tool: Option<ConstructTool>,
    pub editing: Option<ConstructEdit>,
}

/// The icon of a construction feature.
pub(crate) fn feature_icon(c: &Construction) -> Icon {
    match c {
        Construction::PlaneOffset { .. } => Icon::ConstructPlaneOffset,
        Construction::PlaneAngle { .. } | Construction::PlaneTangent { .. } => {
            Icon::ConstructPlaneAngle
        }
        Construction::PlaneMidplane { .. } => Icon::ConstructMidplane,
        Construction::PlaneThreePoints { .. } => Icon::ConstructPlanePoints,
        _ => match c.kind() {
            ConstructKind::Axis => Icon::ConstructAxis,
            _ => Icon::ConstructPoint,
        },
    }
}

/// The construction planes as Create Sketch menu items; the one clicked, if any.
pub(crate) fn plane_menu(ui: &mut egui::Ui, look: Look, planes: &[(u32, String)]) -> Option<u32> {
    let mut picked = None;
    for (id, name) in planes {
        if widgets::menu_item(ui, look, Icon::ConstructPlaneOffset, name, true).clicked() {
            ui.close();
            picked = Some(*id);
        }
    }
    picked
}

impl ChamferaApp {
    /// The construction planes that built, for menus: (id, name).
    pub(crate) fn construct_planes(&self) -> Vec<(u32, String)> {
        self.session
            .doc
            .constructions(ConstructKind::Plane)
            .into_iter()
            .filter(|(id, _)| self.session.constructs().contains_key(id))
            .collect()
    }

    pub(crate) fn open_construct(&mut self, kind: Kind) {
        let tool = ConstructTool::new(kind);
        let prompt = kind
            .slots()
            .get(tool.active)
            .filter(|_| tool.picks.get(tool.active).is_some_and(Option::is_none))
            .map(|(s, _)| s.prompt());
        self.message = Some((
            prompt.map_or_else(|| kind.tip().to_owned(), str::to_owned),
            false,
        ));
        self.construct.tool = Some(tool);
    }

    /// A click in the viewport while a construction tool is open: fills its active input
    /// from what is under the pointer.
    pub(crate) fn construct_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Ok(r) = self.session.execute(
            "construct.pick",
            &json!({ "x": x, "y": y, "width": w, "height": h }),
        ) else {
            return;
        };
        let doc = &self.session.doc;
        let Some(tool) = self.construct.tool.as_mut() else {
            return;
        };
        let Some((slot, _)) = tool.kind.slots().get(tool.active).copied() else {
            return;
        };
        let feature_name = |v: &Value| {
            v["feature"]
                .as_u64()
                .and_then(|f| u32::try_from(f).ok())
                .and_then(|f| doc.feature(f).ok())
                .map_or_else(|| "?".to_owned(), |f| f.name.clone())
        };
        let face_text = |v: &Value| {
            format!(
                "{} of {}",
                v["face"].as_str().unwrap_or("face"),
                feature_name(v)
            )
        };
        let surface = r["surface"].as_str().unwrap_or("");
        let curve = r["edge"]["curve"].as_str().unwrap_or("");
        let edge_text = || {
            format!(
                "Edge {} / {}",
                r["edge"]["faces"][0]["face"].as_str().unwrap_or("?"),
                r["edge"]["faces"][1]["face"].as_str().unwrap_or("?")
            )
        };
        let vertex_text = || {
            let p = &r["vertex"]["point"];
            format!(
                "Vertex ({:.1}, {:.1}, {:.1})",
                p["x"].as_f64().unwrap_or(0.0),
                p["y"].as_f64().unwrap_or(0.0),
                p["z"].as_f64().unwrap_or(0.0)
            )
        };
        let curved_ok =
            surface == "cylinder" || (surface == "cone" && tool.kind == Kind::AxisCylinder);
        let pick = match slot {
            _ if r["face"].is_null() => None,
            Slot::Plane if surface == "plane" => Some(Pick {
                value: r["face"].clone(),
                text: face_text(&r["face"]),
            }),
            Slot::CurvedFace if curved_ok => Some(Pick {
                value: r["face"].clone(),
                text: face_text(&r["face"]),
            }),
            Slot::Axis | Slot::Edge if curve == "line" => Some(Pick {
                value: json!({ "faces": r["edge"]["faces"] }),
                text: edge_text(),
            }),
            Slot::CircleEdge if curve == "circle" || curve == "arc" => Some(Pick {
                value: json!({ "faces": r["edge"]["faces"] }),
                text: edge_text(),
            }),
            Slot::Point if !r["vertex"].is_null() => Some(Pick {
                value: json!({ "vertex": r["vertex"]["faces"] }),
                text: vertex_text(),
            }),
            Slot::Vertex if !r["vertex"].is_null() => Some(Pick {
                value: r["vertex"]["faces"].clone(),
                text: vertex_text(),
            }),
            _ => None,
        };
        let Some(pick) = pick else {
            self.message = Some((format!("{} for this input", slot.prompt()), true));
            return;
        };
        if let Some(p) = tool.picks.get_mut(tool.active) {
            *p = Some(pick);
        }
        // Move on to the next empty input.
        let next = tool.picks.iter().position(Option::is_none);
        let prompt = next
            .and_then(|i| tool.kind.slots().get(i))
            .map(|(s, _)| s.prompt());
        if let Some(i) = next {
            tool.active = i;
        }
        self.message = Some((
            prompt.map_or_else(|| "Press OK to create it".to_owned(), str::to_owned),
            false,
        ));
    }

    /// Runs the open tool's command; closes the tool when it worked.
    pub(crate) fn construct_ok(&mut self) -> Option<Value> {
        let tool = self.construct.tool.as_ref()?;
        let id = tool.kind.command();
        let Some(params) = tool.params(self.session.doc.units.length.symbol()) else {
            self.message = Some(("Fill in every input first".into(), true));
            return None;
        };
        let r = self.run(id, params)?;
        self.message = Some((
            format!("{} created", r["name"].as_str().unwrap_or("Construction")),
            false,
        ));
        self.construct.tool = None;
        Some(r)
    }

    /// Starts a sketch on a construction plane and looks straight at it.
    pub(crate) fn sketch_on_construct(&mut self, id: u32) {
        let Some(r) = self.run_sketch("sketch.create", json!({ "plane": id })) else {
            return;
        };
        self.run("view.normal_to", r["normal"].clone());
        self.sketch.tool = Some(crate::sketch_ui::Tool::Line);
        let name = self
            .session
            .doc
            .feature(id)
            .map_or_else(|_| "the plane".to_owned(), |f| f.name.clone());
        self.message = Some((format!("Sketching on {name}"), false));
    }

    /// The construction tool's dialog and the edit dialog.
    pub(crate) fn construct_dialog(&mut self, ctx: &egui::Context) {
        self.construct_edit_dialog(ctx);
        let Some(kind) = self.construct.tool.as_ref().map(|t| t.kind) else {
            return;
        };
        // The choices each kind of input offers besides clicking in the viewport.
        let doc = &self.session.doc;
        let built = self.session.constructs();
        let listed = |k: ConstructKind| -> Vec<(Value, String)> {
            doc.constructions(k)
                .into_iter()
                .filter(|(id, _)| built.contains_key(id))
                .map(|(id, n)| (json!(id), n))
                .collect()
        };
        let mut planes: Vec<(Value, String)> = crate::sketch_ui::PLANES
            .iter()
            .map(|(p, text, _)| (json!(p), (*text).to_owned()))
            .collect();
        planes.extend(listed(ConstructKind::Plane));
        let mut axes: Vec<(Value, String)> = [("x", "X axis"), ("y", "Y axis"), ("z", "Z axis")]
            .iter()
            .map(|(a, t)| (json!(a), (*t).to_owned()))
            .collect();
        axes.extend(listed(ConstructKind::Axis));
        let mut points = vec![(json!("origin"), "Origin".to_owned())];
        points.extend(listed(ConstructKind::Point));
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.construct.tool.as_mut() else {
            return;
        };
        let title = label(kind.command());
        let mut ok = false;
        let mut cancel = false;
        let mut prompt: Option<&'static str> = None;
        egui::Window::new(title)
            .id(egui::Id::unique("construct dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, kind.icon(), title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("construct grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            for (i, (slot, name)) in kind.slots().iter().enumerate() {
                                widgets::field_label(ui, look, name);
                                let choices: &[(Value, String)] = match slot {
                                    Slot::Plane => &planes,
                                    Slot::Axis => &axes,
                                    Slot::Point => &points,
                                    _ => &[],
                                };
                                let picking = tool.active == i;
                                let current = tool.picks.get(i).and_then(Option::as_ref);
                                let current_value = current.map(|p| p.value.clone());
                                let shown = match (current, picking) {
                                    (Some(p), _) => p.text.clone(),
                                    (None, true) => slot.prompt().to_owned(),
                                    (None, false) => "Choose…".to_owned(),
                                };
                                egui::ComboBox::from_id_salt(("construct input", i))
                                    .selected_text(shown)
                                    .width(widgets::FIELD_WIDTH)
                                    .show_ui(ui, |ui| {
                                        for (value, text) in choices {
                                            let chosen = current_value.as_ref() == Some(value);
                                            if ui.selectable_label(chosen, text).clicked()
                                                && let Some(p) = tool.picks.get_mut(i)
                                            {
                                                *p = Some(Pick {
                                                    value: value.clone(),
                                                    text: text.clone(),
                                                });
                                            }
                                        }
                                        let pick_text =
                                            format!("{} in the viewport", slot.prompt());
                                        if ui.selectable_label(picking, pick_text).clicked() {
                                            tool.active = i;
                                            prompt = Some(slot.prompt());
                                        }
                                    });
                                ui.end_row();
                            }
                            let mut field =
                                |ui: &mut egui::Ui, name: &str, text: &mut String, hint: &str| {
                                    widgets::field_label(ui, look, name);
                                    let r = ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(text)
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
                            match kind {
                                Kind::PlaneOffset => {
                                    field(ui, "Distance", &mut tool.distance, "10 mm or gap * 2");
                                }
                                Kind::PlaneAngle => {
                                    field(ui, "Angle", &mut tool.angle, "45 deg");
                                }
                                Kind::PointCoordinates => {
                                    let [x, y, z] = &mut tool.xyz;
                                    field(ui, "X", x, "mm");
                                    field(ui, "Y", y, "mm");
                                    field(ui, "Z", z, "mm");
                                }
                                _ => {}
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(
                        ui,
                        look,
                        "OK",
                        tool.params(self.session.doc.units.length.symbol())
                            .is_some(),
                    )
                    .clicked()
                    {
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
        if let Some(p) = prompt {
            self.message = Some((p.to_owned(), false));
        }
        if ok {
            self.construct_ok();
        } else if cancel {
            self.construct.tool = None;
            self.message = None;
        }
    }

    /// Opens the edit dialog of a construction feature.
    pub(crate) fn edit_construct(&mut self, feature: u32) {
        let Ok(f) = self.session.doc.feature(feature) else {
            return;
        };
        let value = match &f.kind {
            FeatureKind::Construct(Construction::PlaneOffset { distance, .. }) => {
                Some(("Distance", distance.clone()))
            }
            FeatureKind::Construct(Construction::PlaneAngle { angle, .. }) => {
                Some(("Angle", angle.clone()))
            }
            _ => None,
        };
        self.construct.editing = Some(ConstructEdit {
            feature,
            value,
            suppressed: self.session.doc.is_suppressed(f),
        });
    }

    fn construct_edit_dialog(&mut self, ctx: &egui::Context) {
        let Some(feature) = self.construct.editing.as_ref().map(|e| e.feature) else {
            return;
        };
        let Ok(f) = self.session.doc.feature(feature) else {
            self.construct.editing = None;
            return;
        };
        let name = f.name.clone();
        let (icon, about) = match &f.kind {
            FeatureKind::Construct(c) => (feature_icon(c), c.describe()),
            _ => (Icon::Construction, String::new()),
        };
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(edit) = self.construct.editing.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        let title = format!("{}: {name}", label("feature.edit"));
        egui::Window::new("Edit Construction")
            .id(egui::Id::unique("construct edit dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, icon, &title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("construct edit grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Made");
                            ui.label(&about);
                            ui.end_row();
                            if let Some((what, value)) = edit.value.as_mut() {
                                widgets::field_label(ui, look, what);
                                let r = ui.add_sized(
                                    widgets::FIELD,
                                    egui::TextEdit::singleline(value)
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    ok = true;
                                }
                                ui.end_row();
                            }
                            widgets::field_label(ui, look, "Build");
                            let mut on = !edit.suppressed;
                            if ui.checkbox(&mut on, "Build this feature").changed() {
                                edit.suppressed = !on;
                            }
                            ui.end_row();
                        });
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
            let mut params = json!({ "feature": edit.feature, "suppressed": edit.suppressed });
            match &edit.value {
                Some(("Distance", v)) => {
                    params["distance"] = json!(with_unit(v, self.session.doc.units.length.symbol()))
                }
                Some((_, v)) => params["angle"] = json!(with_unit(v, "deg")),
                None => {}
            }
            if self.run("feature.edit", params).is_some() {
                self.construct.editing = None;
                self.message = Some((format!("{name} updated"), false));
            }
        } else if cancel {
            self.construct.editing = None;
        }
    }

    /// The Browser's Construction folder: planes, axes and points; double-click a plane to
    /// sketch on it, right-click for more.
    pub(crate) fn browser_construction(&mut self, ui: &mut egui::Ui) {
        let sketching = self.session.active_sketch.is_some();
        let items: Vec<(u32, String, Icon, bool, bool, bool)> = self
            .session
            .doc
            .features
            .iter()
            .filter_map(|f| match &f.kind {
                FeatureKind::Construct(c) => Some((
                    f.id,
                    f.name.clone(),
                    feature_icon(c),
                    c.kind() == ConstructKind::Plane,
                    self.session.doc.is_suppressed(f),
                    self.session.feature_errors().contains_key(&f.id),
                )),
                _ => None,
            })
            .collect();
        if items.is_empty() {
            return;
        }
        if !self.section(
            ui,
            "construction",
            Icon::Construction,
            "Construction",
            items.len(),
        ) {
            return;
        }
        for (id, name, icon, plane, suppressed, error) in items {
            let visible = self.session.construct_visible(id);
            let r = crate::browser::tree_row(
                ui,
                self.look(),
                ui.scope_id().with(("construct", id)),
                &crate::browser::Row {
                    depth: 2,
                    icon: Some(icon),
                    text: &name,
                    note: if error {
                        Some("cannot build")
                    } else if suppressed {
                        Some("off")
                    } else {
                        None
                    },
                    dimmed: suppressed || !visible,
                    error,
                    eye: Some(visible),
                    ..crate::browser::Row::default()
                },
            );
            if let Some(eye) = r.eye {
                let tip = if visible { "Hide" } else { "Show" };
                if eye
                    .on_hover_text(format!("{tip}\n{}", label("construct.visibility")))
                    .clicked()
                {
                    self.run(
                        "construct.visibility",
                        json!({ "feature": id, "visible": !visible }),
                    );
                }
            }
            let can_sketch = plane && !sketching && !suppressed && !error;
            let row = r.row.on_hover_text(if can_sketch {
                "Double-click to sketch on this plane"
            } else {
                "Right-click to edit or delete"
            });
            if can_sketch && row.double_clicked() {
                self.sketch_on_construct(id);
            } else if !plane && row.double_clicked() {
                self.edit_construct(id);
            }
            let mut action = None;
            row.context_menu(|ui| {
                let look = self.look();
                if plane
                    && widgets::menu_item(
                        ui,
                        look,
                        Icon::SketchCreate,
                        label("sketch.create"),
                        can_sketch,
                    )
                    .clicked()
                {
                    ui.close();
                    action = Some("sketch");
                }
                if widgets::menu_item(ui, look, Icon::Edit, label("feature.edit"), true).clicked() {
                    ui.close();
                    action = Some("edit");
                }
                if widgets::menu_item(ui, look, Icon::Delete, label("feature.delete"), true)
                    .clicked()
                {
                    ui.close();
                    action = Some("delete");
                }
            });
            match action {
                Some("sketch") => self.sketch_on_construct(id),
                Some("edit") => self.edit_construct(id),
                Some("delete") => {
                    self.run("feature.delete", json!({ "feature": id }));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_plane_tool_starts_on_xy_and_ok_makes_the_plane() {
        let mut app = ChamferaApp::headless();
        app.open_construct(Kind::PlaneOffset);
        app.construct.tool.as_mut().unwrap().distance = "15".into();
        let r = app.construct_ok().unwrap();
        // An offset of 15 from XY is at z = 15.
        assert_eq!(r["geometry"]["origin"]["z"].as_f64(), Some(15.0));
        assert!(app.construct.tool.is_none());
        // It has its own icon and description, and a sketch can go on it.
        let id = r["feature"].as_u64().unwrap() as u32;
        let FeatureKind::Construct(c) = &app.session.doc.feature(id).unwrap().kind else {
            panic!("a construction");
        };
        assert_eq!(feature_icon(c), Icon::ConstructPlaneOffset);
        assert_eq!(c.describe(), "Offset 15 mm from XY");
        assert_eq!(app.construct_planes(), vec![(id, "Plane1".to_owned())]);
        app.sketch_on_construct(id);
        let sk = app.session.active_sketch.unwrap();
        assert_eq!(app.session.doc.sketch(sk).unwrap().on_plane, Some(id));
    }

    #[test]
    fn clicking_faces_fills_the_midplane_inputs() {
        let mut app = ChamferaApp::headless();
        for (id, p) in [
            ("sketch.create", json!({ "plane": "xy" })),
            (
                "sketch.rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 20 }),
            ),
            ("sketch.finish", json!({})),
            (
                "solid.extrude",
                json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": 10 }),
            ),
            ("view.standard", json!({ "view": "top" })),
            ("view.fit", json!({ "width": 800, "height": 600 })),
        ] {
            assert!(app.run(id, p).is_some(), "{id}");
        }
        app.open_construct(Kind::PlaneMidplane);
        // The top face from above, then XY from the list (as the combo box does).
        app.construct_click(400.0, 300.0, 800.0, 600.0);
        let tool = app.construct.tool.as_mut().unwrap();
        assert_eq!(tool.pick(0).unwrap()["face"], "top");
        assert_eq!(tool.active, 1);
        tool.picks[1] = Some(Pick {
            value: json!("xy"),
            text: "Top plane (XY)".into(),
        });
        // Faces z = 10 and z = 0: the midplane is at z = 5.
        let r = app.construct_ok().unwrap();
        assert_eq!(r["geometry"]["origin"]["z"].as_f64(), Some(5.0));
    }

    #[test]
    fn plain_numbers_get_their_unit_and_expressions_stay() {
        assert_eq!(with_unit(" 30 ", "deg"), "30 deg");
        assert_eq!(with_unit("tilt * 2", "deg"), "tilt * 2");
        assert_eq!(with_unit("5 mm", "mm"), "5 mm");
    }
}

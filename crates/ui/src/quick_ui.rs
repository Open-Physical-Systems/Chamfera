//! The modelling quick wins' dialogs: Box, Cylinder, Sphere, Torus and Cone (SOLID >
//! CREATE), Scale, Align and Physical Material (MODIFY), and Check Bodies (INSPECT).
//! One small dialog each, acting only through `solid.box` ... `solid.cone`,
//! `solid.scale`, `solid.align`, `body.material`, `inspect.check`, `select.pick` and
//! `feature.edit` (a primitive or scale opened again from the timeline).

use crate::icons::Icon;
use crate::solid_ui::operation_name;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{FeatureKind, Operation, PrimitiveAxis, PrimitiveShape};
use serde_json::{Value, json};

/// Which quick tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickKind {
    Box,
    Cylinder,
    Sphere,
    Torus,
    Cone,
    Scale,
    Align,
    Material,
    Check,
}

impl QuickKind {
    pub const ALL: [QuickKind; 9] = [
        QuickKind::Box,
        QuickKind::Cylinder,
        QuickKind::Sphere,
        QuickKind::Torus,
        QuickKind::Cone,
        QuickKind::Scale,
        QuickKind::Align,
        QuickKind::Material,
        QuickKind::Check,
    ];

    pub fn command(self) -> &'static str {
        match self {
            QuickKind::Box => "solid.box",
            QuickKind::Cylinder => "solid.cylinder",
            QuickKind::Sphere => "solid.sphere",
            QuickKind::Torus => "solid.torus",
            QuickKind::Cone => "solid.cone",
            QuickKind::Scale => "solid.scale",
            QuickKind::Align => "solid.align",
            QuickKind::Material => "body.material",
            QuickKind::Check => "inspect.check",
        }
    }

    /// The quick tool a ribbon tool id opens.
    pub fn from_command(id: &str) -> Option<QuickKind> {
        QuickKind::ALL.into_iter().find(|k| k.command() == id)
    }

    pub fn icon(self) -> Icon {
        match self {
            QuickKind::Box => Icon::Body,
            QuickKind::Cylinder => Icon::Extrude,
            QuickKind::Sphere | QuickKind::Torus => Icon::Revolve,
            QuickKind::Cone => Icon::Loft,
            QuickKind::Scale => Icon::SketchTransform,
            QuickKind::Align => Icon::Move,
            QuickKind::Material => Icon::Mass,
            QuickKind::Check => Icon::Warning,
        }
    }

    /// A primitive makes a body from nothing; the others need one.
    pub fn needs_solid(self) -> bool {
        !self.is_primitive()
    }

    fn is_primitive(self) -> bool {
        matches!(
            self,
            QuickKind::Box
                | QuickKind::Cylinder
                | QuickKind::Sphere
                | QuickKind::Torus
                | QuickKind::Cone
        )
    }

    /// The size fields: (parameter, what the user reads, default).
    fn sizes(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            QuickKind::Box => &[
                ("length", "Length", "20 mm"),
                ("width", "Width", "20 mm"),
                ("height", "Height", "20 mm"),
            ],
            QuickKind::Cylinder => &[("radius", "Radius", "10 mm"), ("height", "Height", "20 mm")],
            QuickKind::Sphere => &[("radius", "Radius", "10 mm")],
            QuickKind::Torus => &[
                ("major_radius", "Major radius", "20 mm"),
                ("minor_radius", "Minor radius", "5 mm"),
            ],
            QuickKind::Cone => &[
                ("radius", "Base radius", "10 mm"),
                ("top_radius", "Top radius", "0 mm"),
                ("height", "Height", "20 mm"),
            ],
            _ => &[],
        }
    }
}

const OPERATIONS: [Operation; 4] = [
    Operation::NewBody,
    Operation::Join,
    Operation::Cut,
    Operation::Intersect,
];
const AXES: [(PrimitiveAxis, &str); 3] = [
    (PrimitiveAxis::Z, "Z (up)"),
    (PrimitiveAxis::X, "X"),
    (PrimitiveAxis::Y, "Y"),
];

/// A quick tool while it is open.
#[derive(Debug, Clone)]
pub struct QuickTool {
    pub kind: QuickKind,
    /// The primitive or scale feature being edited (opened from the timeline).
    pub editing: Option<u32>,
    /// Primitive sizes, in the order of `QuickKind::sizes`.
    pub sizes: Vec<String>,
    /// Primitive position or scale centre: x, y, z.
    pub origin: [String; 3],
    /// Index into [`AXES`].
    pub axis: usize,
    /// Index into [`OPERATIONS`].
    pub operation: usize,
    /// Scale: the bodies. Material: the body.
    pub bodies: Vec<String>,
    /// Scale: one factor for all axes.
    pub uniform: bool,
    pub factors: [String; 3],
    /// Align: the entity to move, then the one to meet (as `inspect.measure` names them).
    pub from: Option<Value>,
    pub to: Option<Value>,
    pub copy: bool,
    /// Material: index into the library, or past its end for a typed density.
    pub preset: usize,
    pub density: String,
    /// The last reply worth showing (Check's report, Material's mass).
    pub report: Option<Value>,
}

impl QuickTool {
    pub fn new(kind: QuickKind) -> QuickTool {
        QuickTool {
            kind,
            editing: None,
            sizes: kind.sizes().iter().map(|s| s.2.to_owned()).collect(),
            origin: ["0".into(), "0".into(), "0".into()],
            axis: 0,
            operation: 0,
            bodies: Vec::new(),
            uniform: true,
            factors: ["2".into(), "1".into(), "1".into()],
            from: None,
            to: None,
            copy: false,
            preset: 0,
            density: "1 g/cm3".into(),
            report: None,
        }
    }

    /// A typed origin as {x, y, z} in mm, or `None` if a field is not a number.
    fn origin_value(&self) -> Option<Value> {
        let mut v = [0.0; 3];
        for (slot, t) in v.iter_mut().zip(&self.origin) {
            *slot = t.trim().trim_end_matches("mm").trim().parse::<f64>().ok()?;
        }
        Some(json!({ "x": v[0], "y": v[1], "z": v[2] }))
    }

    /// The command's parameters.
    pub fn params(&self) -> Value {
        let mut p = json!({});
        match self.kind {
            k if k.is_primitive() => {
                for ((key, _, _), v) in k.sizes().iter().zip(&self.sizes) {
                    p[*key] = json!(v.trim());
                }
                p["origin"] = self.origin_value().unwrap_or(Value::Null);
                p["axis"] = json!(AXES.get(self.axis).map_or("z", |a| a.0.key()));
                p["operation"] = json!(
                    OPERATIONS
                        .get(self.operation)
                        .unwrap_or(&Operation::NewBody)
                        .key()
                );
            }
            QuickKind::Scale => {
                p["bodies"] = json!(self.bodies);
                if self.uniform {
                    p["factor"] = json!(self.factors[0].trim());
                } else {
                    p["x"] = json!(self.factors[0].trim());
                    p["y"] = json!(self.factors[1].trim());
                    p["z"] = json!(self.factors[2].trim());
                }
                p["origin"] = self.origin_value().unwrap_or(Value::Null);
            }
            QuickKind::Align => {
                p["from"] = self.from.clone().unwrap_or(Value::Null);
                p["to"] = self.to.clone().unwrap_or(Value::Null);
                p["copy"] = json!(self.copy);
            }
            QuickKind::Material => {
                p["body"] = json!(self.bodies.first());
                match chamfera_doc::physical::LIBRARY.get(self.preset) {
                    Some((key, _, _)) => p["preset"] = json!(key),
                    None => p["density"] = json!(self.density.trim()),
                }
            }
            _ => {}
        }
        p
    }

    /// True when OK can run.
    pub fn ready(&self) -> bool {
        match self.kind {
            k if k.is_primitive() => {
                self.sizes.iter().all(|s| !s.trim().is_empty()) && self.origin_value().is_some()
            }
            QuickKind::Scale => !self.bodies.is_empty() && self.origin_value().is_some(),
            QuickKind::Align => self.from.is_some() && self.to.is_some(),
            QuickKind::Material => !self.bodies.is_empty(),
            _ => true,
        }
    }

    /// The tool filled from a primitive or scale feature, to edit it.
    fn from_feature(id: u32, kind: &FeatureKind) -> Option<QuickTool> {
        let num = |v: f64| format!("{v}");
        match kind {
            FeatureKind::Primitive(p) => {
                let k = match &p.shape {
                    PrimitiveShape::Box { .. } => QuickKind::Box,
                    PrimitiveShape::Cylinder { .. } => QuickKind::Cylinder,
                    PrimitiveShape::Sphere { .. } => QuickKind::Sphere,
                    PrimitiveShape::Torus { .. } => QuickKind::Torus,
                    PrimitiveShape::Cone { .. } => QuickKind::Cone,
                };
                let mut t = QuickTool::new(k);
                let mut shape = p.shape.clone();
                t.sizes = shape.sizes_mut().into_iter().map(|x| x.1.clone()).collect();
                t.origin = [num(p.origin.x), num(p.origin.y), num(p.origin.z)];
                t.axis = AXES.iter().position(|a| a.0 == p.axis).unwrap_or(0);
                t.operation = OPERATIONS
                    .iter()
                    .position(|o| *o == p.operation)
                    .unwrap_or(0);
                t.editing = Some(id);
                Some(t)
            }
            FeatureKind::Scale(sc) => {
                let mut t = QuickTool::new(QuickKind::Scale);
                t.bodies = sc.bodies.clone();
                t.uniform = sc.x == sc.y && sc.y == sc.z;
                t.factors = [sc.x.clone(), sc.y.clone(), sc.z.clone()];
                t.origin = [num(sc.origin.x), num(sc.origin.y), num(sc.origin.z)];
                t.editing = Some(id);
                Some(t)
            }
            _ => None,
        }
    }
}

/// The timeline's icon and one-line summary of a primitive or scale.
pub(crate) fn details(kind: &FeatureKind) -> (Icon, String) {
    match kind {
        FeatureKind::Primitive(p) => {
            let mut shape = p.shape.clone();
            let sizes: Vec<String> = shape
                .sizes_mut()
                .into_iter()
                .map(|(k, v)| format!("{} {v}", k.replace('_', " ")))
                .collect();
            let icon = match p.shape {
                PrimitiveShape::Box { .. } => QuickKind::Box.icon(),
                PrimitiveShape::Cylinder { .. } => QuickKind::Cylinder.icon(),
                PrimitiveShape::Sphere { .. } => QuickKind::Sphere.icon(),
                PrimitiveShape::Torus { .. } => QuickKind::Torus.icon(),
                PrimitiveShape::Cone { .. } => QuickKind::Cone.icon(),
            };
            (
                icon,
                format!(
                    "{} {} · {}",
                    p.shape.label(),
                    sizes.join(" · "),
                    operation_name(p.operation)
                ),
            )
        }
        FeatureKind::Scale(sc) => (
            QuickKind::Scale.icon(),
            if sc.x == sc.y && sc.y == sc.z {
                format!("Scale {} × {}", sc.bodies.join(", "), sc.x)
            } else {
                format!(
                    "Scale {} × {}, {}, {}",
                    sc.bodies.join(", "),
                    sc.x,
                    sc.y,
                    sc.z
                )
            },
        ),
        _ => (Icon::Body, String::new()),
    }
}

/// A labelled text field in a dialog grid; true when Enter was pressed in it.
fn text_field(ui: &mut egui::Ui, look: Look, name: &str, value: &mut String) -> bool {
    widgets::field_label(ui, look, name);
    let edit = ui.add_sized(
        widgets::FIELD,
        egui::TextEdit::singleline(value).align(egui::Align2::LEFT_CENTER),
    );
    ui.end_row();
    edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A labelled choice between named options.
fn choice(ui: &mut egui::Ui, look: Look, name: &str, value: &mut usize, options: &[String]) {
    widgets::field_label(ui, look, name);
    let current = options.get(*value).cloned().unwrap_or_default();
    egui::ComboBox::from_id_salt(("quick tool", name))
        .selected_text(current)
        .width(widgets::FIELD_WIDTH)
        .show_ui(ui, |ui| {
            for (i, o) in options.iter().enumerate() {
                ui.selectable_value(value, i, o);
            }
        });
    ui.end_row();
}

/// A short name for a picked entity reference.
fn entity_text(v: &Option<Value>, none: &str) -> String {
    let Some(v) = v else {
        return none.to_owned();
    };
    let solid = v["solid"].as_str().unwrap_or("");
    for key in ["face", "edge", "vertex"] {
        if let Some(n) = v[key].as_str() {
            return format!("{key} {n} of {solid}");
        }
        if v[key].is_array() {
            return format!("{key} of {solid}");
        }
    }
    if v["point"].is_array() {
        return "a point".into();
    }
    solid.to_owned()
}

impl ChamferaApp {
    /// Opens a quick tool, starting from the selected bodies; Check runs at once.
    pub(crate) fn open_quick_tool(&mut self, kind: QuickKind) {
        let mut tool = QuickTool::new(kind);
        if matches!(kind, QuickKind::Scale | QuickKind::Material) {
            tool.bodies = self.session.selected_solid_names();
        }
        // With nothing selected and one body in the design, that body.
        if tool.bodies.is_empty()
            && matches!(kind, QuickKind::Scale | QuickKind::Material)
            && let [only] = self.session.solids()
        {
            tool.bodies.push(only.name.clone());
        }
        if kind == QuickKind::Material {
            tool.bodies.truncate(1);
            self.material_current(&mut tool);
        }
        self.message = Some((
            match kind {
                QuickKind::Scale => "Click the bodies to scale",
                QuickKind::Align => "Click the face, edge or vertex to move, then the one to meet",
                QuickKind::Material => "Click the body and pick its material",
                QuickKind::Check => "Every body checked by the kernel",
                _ => "Type the sizes and the position",
            }
            .into(),
            false,
        ));
        if kind == QuickKind::Check {
            tool.report = self.run("inspect.check", json!({}));
        }
        self.solid.quick = Some(tool);
    }

    /// Opens the tool on primitive or scale `feature` to change it (timeline).
    pub(crate) fn edit_quick_feature(&mut self, feature: u32) {
        let Ok(f) = self.session.doc.feature(feature) else {
            return;
        };
        if let Some(t) = QuickTool::from_feature(feature, &f.kind) {
            self.solid.quick = Some(t);
        }
    }

    /// The material tool shows the body's current material and mass.
    fn material_current(&mut self, tool: &mut QuickTool) {
        tool.report = None;
        let Some(body) = tool.bodies.first() else {
            return;
        };
        let feature = self
            .session
            .solids()
            .iter()
            .find(|x| x.name == *body)
            .map(|x| x.feature);
        if let Some(m) = feature.and_then(|f| self.session.doc.materials.get(&f)) {
            tool.preset = chamfera_doc::physical::LIBRARY
                .iter()
                .position(|l| l.0 == m.name)
                .unwrap_or(chamfera_doc::physical::LIBRARY.len());
            tool.density = format!("{} g/cm3", m.density);
            tool.report = self
                .session
                .execute("inspect.mass", &json!({ "body": body }))
                .ok();
        }
    }

    /// A click in the viewport while a quick tool is open.
    pub(crate) fn quick_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(kind) = self.solid.quick.as_ref().map(|t| t.kind) else {
            return;
        };
        match kind {
            QuickKind::Scale | QuickKind::Material => {
                self.run(
                    "select.pick",
                    json!({ "x": x, "y": y, "width": w, "height": h }),
                );
                let picked = self.session.selected_body_names();
                let Some(mut tool) = self.solid.quick.take() else {
                    return;
                };
                if kind == QuickKind::Material {
                    tool.bodies = picked.into_iter().take(1).collect();
                    self.material_current(&mut tool);
                } else {
                    for name in picked {
                        if let Some(i) = tool.bodies.iter().position(|b| *b == name) {
                            tool.bodies.remove(i);
                        } else {
                            tool.bodies.push(name);
                        }
                    }
                }
                self.solid.quick = Some(tool);
            }
            QuickKind::Align => {
                let hit = self.session.entity_at(x, y, w, h);
                if let (Some(tool), Some(hit)) = (self.solid.quick.as_mut(), hit) {
                    if tool.from.is_none() || tool.to.is_some() {
                        tool.from = Some(hit);
                        tool.to = None;
                    } else {
                        tool.to = Some(hit);
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs the open tool's command; closes the tool when it worked (Check and Material
    /// stay open with their result).
    pub(crate) fn quick_ok(&mut self) -> Option<Value> {
        let tool = self.solid.quick.clone()?;
        let r = match (tool.kind, tool.editing) {
            (QuickKind::Check, _) => {
                let r = self.run("inspect.check", json!({}))?;
                if let Some(t) = self.solid.quick.as_mut() {
                    t.report = Some(r.clone());
                }
                return Some(r);
            }
            (_, Some(id)) => {
                let mut p = tool.params();
                p["feature"] = json!(id);
                if tool.kind == QuickKind::Scale {
                    // An edit names factors one way: all three.
                    if let Some(o) = p.as_object_mut() {
                        o.remove("factor");
                    }
                    for (k, v) in ["x", "y", "z"].iter().zip(&tool.factors) {
                        let f = if tool.uniform { &tool.factors[0] } else { v };
                        p[*k] = json!(f.trim());
                    }
                }
                self.run("feature.edit", p)?
            }
            _ => self.run(tool.kind.command(), tool.params())?,
        };
        let solid_text = |r: &Value| -> String {
            r["solids"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|s| {
                            Some(format!(
                                "{} {}",
                                s["name"].as_str()?,
                                self.units().volume(s["volume"].as_f64()?)
                            ))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        let text = match tool.kind {
            QuickKind::Material => {
                let mass = r["mass"]
                    .as_f64()
                    .map_or_else(String::new, |m| format!("{m:.3} g"));
                format!(
                    "{} is {} ({} g/cm³) · {mass}",
                    r["body"].as_str().unwrap_or(""),
                    r["material"].as_str().unwrap_or(""),
                    r["density"].as_f64().unwrap_or(0.0)
                )
            }
            QuickKind::Align => format!(
                "{} · moved {}",
                r["name"].as_str().unwrap_or(label(tool.kind.command())),
                self.units().length(r["distance"].as_f64().unwrap_or(0.0))
            ),
            _ => format!(
                "{} · {}",
                r["name"].as_str().unwrap_or(label(tool.kind.command())),
                solid_text(&r)
            ),
        };
        self.message = Some((text, false));
        if tool.kind == QuickKind::Material {
            if let Some(mut t) = self.solid.quick.take() {
                self.material_current(&mut t);
                self.solid.quick = Some(t);
            }
        } else {
            self.solid.quick = None;
        }
        Some(r)
    }

    /// The open quick tool's dialog.
    pub(crate) fn quick_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let units = self.units();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.quick.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = match tool.editing {
            Some(_) => format!("Edit {}", label(tool.kind.command())),
            None => label(tool.kind.command()).to_owned(),
        };
        let ready = tool.ready();
        let ok_text = match tool.kind {
            QuickKind::Check => "Check Again",
            QuickKind::Material => "Apply",
            _ => "OK",
        };
        egui::Window::new(title.clone())
            .id(egui::Id::unique("quick tool dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, tool.kind.icon(), &title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("quick tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| match tool.kind {
                            k if k.is_primitive() => {
                                for ((_, name, _), v) in k.sizes().iter().zip(tool.sizes.iter_mut())
                                {
                                    ok |= text_field(ui, look, name, v);
                                }
                                for (axis, v) in ["X", "Y", "Z"].iter().zip(tool.origin.iter_mut())
                                {
                                    ok |= text_field(ui, look, &format!("Position {axis}"), v);
                                }
                                let axes: Vec<String> =
                                    AXES.iter().map(|a| a.1.to_owned()).collect();
                                choice(ui, look, "Axis", &mut tool.axis, &axes);
                                let ops: Vec<String> = OPERATIONS
                                    .iter()
                                    .map(|o| operation_name(*o).to_owned())
                                    .collect();
                                choice(ui, look, "Operation", &mut tool.operation, &ops);
                            }
                            QuickKind::Scale => {
                                widgets::field_label(ui, look, "Bodies");
                                ui.label(match tool.bodies.len() {
                                    0 => "Click the bodies".to_owned(),
                                    1 => tool.bodies[0].clone(),
                                    n => format!("{n} selected"),
                                });
                                ui.end_row();
                                widgets::field_label(ui, look, "Type");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut tool.uniform, true, "Uniform");
                                    ui.radio_value(&mut tool.uniform, false, "Per axis");
                                });
                                ui.end_row();
                                if tool.uniform {
                                    ok |= text_field(ui, look, "Factor", &mut tool.factors[0]);
                                } else {
                                    for (axis, v) in
                                        ["X", "Y", "Z"].iter().zip(tool.factors.iter_mut())
                                    {
                                        ok |= text_field(ui, look, &format!("Factor {axis}"), v);
                                    }
                                }
                                for (axis, v) in ["X", "Y", "Z"].iter().zip(tool.origin.iter_mut())
                                {
                                    ok |= text_field(ui, look, &format!("Centre {axis}"), v);
                                }
                            }
                            QuickKind::Align => {
                                widgets::field_label(ui, look, "Move");
                                ui.label(entity_text(&tool.from, "Click a face, edge or vertex"));
                                ui.end_row();
                                widgets::field_label(ui, look, "Onto");
                                ui.label(entity_text(&tool.to, "Click where it should go"));
                                ui.end_row();
                                widgets::field_label(ui, look, "Copy");
                                ui.checkbox(&mut tool.copy, "Keep the original");
                                ui.end_row();
                            }
                            QuickKind::Material => {
                                widgets::field_label(ui, look, "Body");
                                ui.label(
                                    tool.bodies.first().map_or("Click a body", |s| s.as_str()),
                                );
                                ui.end_row();
                                let mut names: Vec<String> = chamfera_doc::physical::LIBRARY
                                    .iter()
                                    .map(|(_, n, d)| format!("{n} · {d} g/cm³"))
                                    .collect();
                                names.push("Custom density".into());
                                choice(ui, look, "Material", &mut tool.preset, &names);
                                if tool.preset >= chamfera_doc::physical::LIBRARY.len() {
                                    ok |= text_field(ui, look, "Density", &mut tool.density);
                                }
                                if let Some(r) = &tool.report {
                                    widgets::field_label(ui, look, "Volume");
                                    ui.label(units.volume(r["volume"].as_f64().unwrap_or(0.0)));
                                    ui.end_row();
                                    widgets::field_label(ui, look, "Mass");
                                    ui.label(format!("{:.3} g", r["mass"].as_f64().unwrap_or(0.0)));
                                    ui.end_row();
                                }
                            }
                            QuickKind::Check => {
                                let rows = tool
                                    .report
                                    .as_ref()
                                    .and_then(|r| r["bodies"].as_array().cloned())
                                    .unwrap_or_default();
                                if rows.is_empty() {
                                    widgets::field_label(ui, look, "Bodies");
                                    ui.label("No bodies to check");
                                    ui.end_row();
                                }
                                for b in rows {
                                    widgets::field_label(
                                        ui,
                                        look,
                                        b["name"].as_str().unwrap_or(""),
                                    );
                                    let valid = b["valid"] == json!(true);
                                    let text = if valid {
                                        format!(
                                            "Valid {} · {} faces · genus {}",
                                            b["kind"].as_str().unwrap_or(""),
                                            b["faces"],
                                            b["genus"]
                                        )
                                    } else {
                                        format!("Invalid: {}", b["error"].as_str().unwrap_or(""))
                                    };
                                    let colour = if valid {
                                        look.c(|t| t.success)
                                    } else {
                                        look.c(|t| t.error)
                                    };
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(text).color(colour))
                                            .wrap(),
                                    );
                                    ui.end_row();
                                }
                            }
                            _ => {}
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, ok_text, ready).clicked() {
                        ok = true;
                    }
                    let close = if matches!(tool.kind, QuickKind::Check | QuickKind::Material) {
                        "Close"
                    } else {
                        "Cancel"
                    };
                    if widgets::secondary_button(ui, look, close).clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if ok && ready {
            self.quick_ok();
        }
        if cancel {
            self.solid.quick = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_tool_makes_a_box_of_its_typed_sizes() {
        let mut app = ChamferaApp::headless();
        app.run_tool("solid.box");
        let tool = app.solid.quick.as_mut().unwrap();
        assert_eq!(tool.kind, QuickKind::Box);
        tool.sizes = vec!["10".into(), "20 mm".into(), "30".into()];
        tool.origin = ["5".into(), "0".into(), "0".into()];
        tool.operation = 0;
        app.quick_ok().unwrap();
        assert!(app.solid.quick.is_none());
        // 10 x 20 x 30 = 6000.
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - 6000.0).abs() < 1e-6);
        let (icon, text) = details(&app.session.doc.features[0].kind);
        assert_eq!(icon, Icon::Body);
        assert_eq!(text, "Box length 10 · width 20 mm · height 30 · New body");
        // Opened again from the timeline it edits: height 15 gives 3000.
        app.edit_quick_feature(app.session.doc.features[0].id);
        let tool = app.solid.quick.as_mut().unwrap();
        assert_eq!(tool.sizes[1], "20 mm");
        tool.sizes[2] = "15 mm".into();
        app.quick_ok().unwrap();
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - 3000.0).abs() < 1e-6);
        assert_eq!(app.session.doc.features.len(), 1);
    }

    #[test]
    fn the_scale_and_material_tools_run_on_the_selected_body() {
        let mut app = ChamferaApp::headless();
        app.run(
            "solid.box",
            json!({"length": 10, "width": 10, "height": 10}),
        );
        let id = app.session.solids()[0].scene_id.unwrap();
        app.session.scene.selected = vec![id];
        app.open_quick_tool(QuickKind::Material);
        let tool = app.solid.quick.as_mut().unwrap();
        assert_eq!(tool.bodies, vec!["Box1"]);
        tool.preset = 0; // steel, 7.85 g/cm³
        app.quick_ok().unwrap();
        // The tool stays open with the mass: 1 cm³ of steel is 7.85 g.
        let r = app.solid.quick.as_ref().unwrap().report.clone().unwrap();
        assert!((r["mass"].as_f64().unwrap() - 7.85).abs() < 1e-9);
        app.solid.quick = None;
        app.open_quick_tool(QuickKind::Scale);
        let tool = app.solid.quick.as_mut().unwrap();
        tool.factors[0] = "2".into();
        assert_eq!(tool.params()["factor"], "2");
        app.quick_ok().unwrap();
        // 8 times 1000.
        let v = app.session.solids()[0].body.volume().unwrap();
        assert!((v - 8000.0).abs() < 1e-6);
    }

    #[test]
    fn the_check_tool_reports_every_body() {
        let mut app = ChamferaApp::headless();
        app.run(
            "solid.torus",
            json!({"major_radius": 10, "minor_radius": 2}),
        );
        app.open_quick_tool(QuickKind::Check);
        let r = app.solid.quick.as_ref().unwrap().report.clone().unwrap();
        assert_eq!(r["checked"], 1);
        assert_eq!(r["bodies"][0]["valid"], true);
        assert_eq!(r["bodies"][0]["genus"], 1);
    }

    #[test]
    fn align_params_carry_both_picks() {
        let mut tool = QuickTool::new(QuickKind::Align);
        assert!(!tool.ready());
        tool.from = Some(json!({"solid": "A", "face": "1:top"}));
        tool.to = Some(json!({"point": [0, 0, 0]}));
        assert!(tool.ready());
        let p = tool.params();
        assert_eq!(p["from"]["face"], "1:top");
        assert_eq!(entity_text(&tool.from, ""), "face 1:top of A");
    }
}

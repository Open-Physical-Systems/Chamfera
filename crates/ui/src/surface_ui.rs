//! The surface tools of the SOLID tab's SURFACE group (rows in `ribbon.rs`): Surface
//! Extrude, Surface Revolve, Surface Sweep, Patch, Stitch and Trim, one small dialog each. Clicks in the viewport pick what
//! the tool works on: sketch profiles (`solid.profile_at`) and open curves
//! (`solid.path_at`), a revolve's axis line and a sweep's path, model edges
//! (`solid.edge_at`) and bodies (`select.pick`). Acts only through `surface.*` commands,
//! and `feature.edit` when Edit Feature reopens a sweep, patch, stitch or trim here.
//! Surface bodies are drawn from both sides in the theme's `surface_body` colour.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{PlaneRef, SurfaceFeature, TrimKeep, TrimTool};
use chamfera_geom::Point2;
use serde_json::{Value, json};

/// Which surface tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    Extrude,
    Revolve,
    Sweep,
    Patch,
    Stitch,
    Trim,
}

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 6] = [
        SurfaceKind::Extrude,
        SurfaceKind::Revolve,
        SurfaceKind::Sweep,
        SurfaceKind::Patch,
        SurfaceKind::Stitch,
        SurfaceKind::Trim,
    ];

    pub fn command(self) -> &'static str {
        match self {
            SurfaceKind::Extrude => "surface.extrude",
            SurfaceKind::Revolve => "surface.revolve",
            SurfaceKind::Sweep => "surface.sweep",
            SurfaceKind::Patch => "surface.patch",
            SurfaceKind::Stitch => "surface.stitch",
            SurfaceKind::Trim => "surface.trim",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            SurfaceKind::Extrude => Icon::SurfaceExtrude,
            SurfaceKind::Revolve => Icon::SurfaceRevolve,
            SurfaceKind::Sweep => Icon::SurfaceSweep,
            SurfaceKind::Patch => Icon::Patch,
            SurfaceKind::Stitch => Icon::Stitch,
            SurfaceKind::Trim => Icon::SurfaceTrim,
        }
    }

    pub(crate) fn needs_sketch(self) -> bool {
        matches!(
            self,
            SurfaceKind::Extrude | SurfaceKind::Revolve | SurfaceKind::Sweep
        )
    }

    fn hint(self) -> &'static str {
        match self {
            SurfaceKind::Extrude => "Click profiles or open curves to extrude",
            SurfaceKind::Revolve => "Click profiles or open curves, then pick the axis",
            SurfaceKind::Sweep => "Click profiles or open curves, then pick the path",
            SurfaceKind::Patch => "Click a closed profile, or the edges of a closed loop",
            SurfaceKind::Stitch => "Click the surfaces to stitch",
            SurfaceKind::Trim => "Click the surface to trim and pick the plane",
        }
    }
}

/// What the next viewport click picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePick {
    /// Profiles and open curves (or, for a patch, profiles and edges; bodies for stitch
    /// and trim).
    Shapes,
    /// A revolve's axis line.
    Axis,
    /// A sweep's path.
    Path,
}

/// The origin planes a trim offers: (command value, what the user reads).
const ORIGIN_PLANES: [(&str, &str); 3] = [
    ("xy", "XY plane (z = 0)"),
    ("xz", "XZ plane (y = 0)"),
    ("yz", "YZ plane (x = 0)"),
];
/// The sides a trim keeps.
const KEEP: [(TrimKeep, &str); 2] = [
    (TrimKeep::Below, "Behind the plane"),
    (TrimKeep::Above, "In front of the plane"),
];
/// The same sides when a body trims: outside a solid (or behind a flat surface), inside.
const KEEP_BODY: [&str; 2] = ["Outside (behind)", "Inside (in front)"];

/// A surface tool while it is open.
#[derive(Debug, Clone)]
pub struct SurfaceTool {
    pub kind: SurfaceKind,
    pub picking: SurfacePick,
    pub sketch: Option<u32>,
    pub profiles: Vec<Point2>,
    pub curves: Vec<u32>,
    pub axis: Option<u32>,
    pub path_sketch: Option<u32>,
    pub path: Vec<u32>,
    pub edges: Vec<String>,
    pub bodies: Vec<String>,
    pub distance: String,
    pub angle: String,
    pub symmetric: bool,
    /// Index into the origin planes, then the construction planes.
    pub plane: usize,
    /// Index into [`KEEP`].
    pub keep: usize,
    /// Trim: the body that trims instead of a plane.
    pub trim_body: Option<String>,
    /// Trim: the flat face the edited trim is cut by (`{feature, face}`), kept until
    /// another plane or body is chosen.
    pub trim_face: Option<Value>,
    /// Edit Feature reopened this tool on that feature: OK runs `feature.edit`.
    pub editing: Option<u32>,
}

impl SurfaceTool {
    pub fn new(kind: SurfaceKind) -> SurfaceTool {
        SurfaceTool {
            kind,
            picking: SurfacePick::Shapes,
            sketch: None,
            profiles: Vec::new(),
            curves: Vec::new(),
            axis: None,
            path_sketch: None,
            path: Vec::new(),
            edges: Vec::new(),
            bodies: Vec::new(),
            distance: "10 mm".into(),
            angle: "360 deg".into(),
            symmetric: false,
            plane: 0,
            keep: 0,
            trim_body: None,
            trim_face: None,
            editing: None,
        }
    }

    fn plane_value(&self, constructs: &[(u32, String)]) -> Value {
        match ORIGIN_PLANES.get(self.plane) {
            Some((key, _)) => json!(key),
            None => constructs
                .get(self.plane - ORIGIN_PLANES.len())
                .map_or(json!("xy"), |(id, _)| json!(id)),
        }
    }

    /// The command's parameters.
    pub fn params(&self, constructs: &[(u32, String)]) -> Value {
        let seeds: Vec<Value> = self
            .profiles
            .iter()
            .map(|p| json!({ "x": p.x, "y": p.y }))
            .collect();
        match self.kind {
            SurfaceKind::Extrude => json!({
                "sketch": self.sketch, "profiles": seeds, "curves": self.curves,
                "distance": self.distance.trim(), "symmetric": self.symmetric,
            }),
            SurfaceKind::Revolve => json!({
                "sketch": self.sketch, "profiles": seeds, "curves": self.curves,
                "axis": self.axis, "angle": self.angle.trim(),
            }),
            SurfaceKind::Sweep => json!({
                "sketch": self.sketch, "profiles": seeds, "curves": self.curves,
                "path_sketch": self.path_sketch, "path_curves": self.path,
            }),
            SurfaceKind::Patch => {
                if self.edges.is_empty() {
                    json!({ "sketch": self.sketch, "profiles": seeds })
                } else {
                    json!({ "edges": self.edges })
                }
            }
            SurfaceKind::Stitch => json!({ "bodies": self.bodies }),
            SurfaceKind::Trim => {
                let mut p = json!({
                    "body": self.bodies.first(),
                    "keep": KEEP.get(self.keep).map_or("below", |k| k.0.key()),
                });
                match (&self.trim_body, &self.trim_face) {
                    (Some(b), _) => p["tool"] = json!(b),
                    (None, Some(face)) => p["plane"] = face.clone(),
                    (None, None) => p["plane"] = self.plane_value(constructs),
                }
                p
            }
        }
    }

    /// `feature.edit`'s parameters for the feature this tool edits: what the tool's
    /// command takes, less what an edit keeps (the sketches the picks are in).
    pub fn edit_params(&self, feature: u32, constructs: &[(u32, String)]) -> Value {
        let mut p = self.params(constructs);
        if let Some(o) = p.as_object_mut() {
            o.remove("sketch");
            o.remove("path_sketch");
            o.insert("feature".into(), json!(feature));
        }
        p
    }

    /// True when OK can run.
    pub fn ready(&self) -> bool {
        let shapes = self.sketch.is_some() && !(self.profiles.is_empty() && self.curves.is_empty());
        match self.kind {
            SurfaceKind::Extrude => shapes && !self.distance.trim().is_empty(),
            SurfaceKind::Revolve => shapes && self.axis.is_some(),
            SurfaceKind::Sweep => shapes && !self.path.is_empty(),
            SurfaceKind::Patch => {
                !self.edges.is_empty() || (self.sketch.is_some() && !self.profiles.is_empty())
            }
            SurfaceKind::Stitch => !self.bodies.is_empty(),
            SurfaceKind::Trim => {
                self.bodies.len() == 1
                    && self
                        .trim_body
                        .as_ref()
                        .is_none_or(|b| Some(b) != self.bodies.first())
            }
        }
    }

    /// The tool, filled in from a sweep, patch, stitch or trim feature for Edit Feature;
    /// `None` for an extrude or revolve (Edit Feature edits their value itself).
    pub fn editing(
        id: u32,
        sf: &SurfaceFeature,
        constructs: &[(u32, String)],
    ) -> Option<SurfaceTool> {
        let seeds = |v: &[Point2]| v.to_vec();
        let mut t = match sf {
            SurfaceFeature::Extrude(_) | SurfaceFeature::Revolve(_) => return None,
            SurfaceFeature::Sweep(w) => {
                let mut t = SurfaceTool::new(SurfaceKind::Sweep);
                t.sketch = Some(w.shapes.sketch);
                t.profiles = seeds(&w.shapes.profiles);
                t.curves = w.shapes.curves.clone();
                t.path_sketch = Some(w.path_sketch);
                t.path = w.path.clone();
                t
            }
            SurfaceFeature::Patch(pa) => {
                let mut t = SurfaceTool::new(SurfaceKind::Patch);
                t.sketch = pa.sketch;
                t.profiles = seeds(&pa.profiles);
                t.edges = pa.edges.clone();
                t
            }
            SurfaceFeature::Stitch(st) => {
                let mut t = SurfaceTool::new(SurfaceKind::Stitch);
                t.bodies = st.bodies.clone();
                t
            }
            SurfaceFeature::Trim(tr) => {
                let mut t = SurfaceTool::new(SurfaceKind::Trim);
                t.bodies = vec![tr.body.clone()];
                t.keep = KEEP.iter().position(|k| k.0 == tr.keep).unwrap_or(0);
                match &tr.tool {
                    TrimTool::Body { body } => t.trim_body = Some(body.clone()),
                    TrimTool::Plane { plane } => match plane {
                        PlaneRef::Origin { plane } => {
                            t.plane = match plane {
                                chamfera_doc::OriginPlane::Xy => 0,
                                chamfera_doc::OriginPlane::Xz => 1,
                                chamfera_doc::OriginPlane::Yz => 2,
                            };
                        }
                        PlaneRef::Construct { feature } => {
                            t.plane = constructs
                                .iter()
                                .position(|(c, _)| c == feature)
                                .map_or(0, |i| i + ORIGIN_PLANES.len());
                        }
                        PlaneRef::Face(f) => {
                            t.trim_face = Some(json!({ "feature": f.feature, "face": f.face }));
                        }
                    },
                }
                t
            }
        };
        t.editing = Some(id);
        Some(t)
    }
}

/// The timeline's icon and one-line summary of a surface feature.
pub(crate) fn details(sf: &SurfaceFeature) -> (Icon, String) {
    let count = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    let shapes = |p: usize, c: usize| match (p, c) {
        (0, c) => count(c, "curve"),
        (p, 0) => count(p, "profile"),
        (p, c) => format!("{} and {}", count(p, "profile"), count(c, "curve")),
    };
    match sf {
        SurfaceFeature::Extrude(e) => (
            Icon::SurfaceExtrude,
            format!(
                "Surface extrude {} · {}{}",
                e.distance,
                shapes(e.shapes.profiles.len(), e.shapes.curves.len()),
                if e.symmetric { " · symmetric" } else { "" }
            ),
        ),
        SurfaceFeature::Revolve(r) => (
            Icon::SurfaceRevolve,
            format!(
                "Surface revolve {} · {}",
                r.angle,
                shapes(r.shapes.profiles.len(), r.shapes.curves.len())
            ),
        ),
        SurfaceFeature::Sweep(w) => (
            Icon::SurfaceSweep,
            format!(
                "Surface sweep · {} along {}",
                shapes(w.shapes.profiles.len(), w.shapes.curves.len()),
                count(w.path.len(), "path curve")
            ),
        ),
        SurfaceFeature::Patch(p) => (
            Icon::Patch,
            if p.edges.is_empty() {
                format!("Patch · {}", count(p.profiles.len(), "profile"))
            } else {
                format!("Patch · {}", count(p.edges.len(), "edge"))
            },
        ),
        SurfaceFeature::Stitch(s) => (Icon::Stitch, format!("Stitch {}", s.bodies.join(", "))),
        SurfaceFeature::Trim(t) => (
            Icon::SurfaceTrim,
            format!(
                "Trim {} by {}",
                t.body,
                match &t.tool {
                    TrimTool::Plane { plane } => match plane {
                        PlaneRef::Origin { plane } => match plane {
                            chamfera_doc::OriginPlane::Xy => "XY plane".to_owned(),
                            chamfera_doc::OriginPlane::Xz => "XZ plane".to_owned(),
                            chamfera_doc::OriginPlane::Yz => "YZ plane".to_owned(),
                        },
                        PlaneRef::Construct { feature } => format!("plane {feature}"),
                        PlaneRef::Face(f) => format!("face {}:{}", f.feature, f.face),
                    },
                    TrimTool::Body { body } => body.clone(),
                }
            ),
        ),
    }
}

/// The surface feature of a timeline item, if it is one.
#[cfg(test)]
pub(crate) fn surface_of(kind: &chamfera_doc::FeatureKind) -> Option<&SurfaceFeature> {
    match kind {
        chamfera_doc::FeatureKind::Surface(sf) => Some(sf),
        _ => None,
    }
}

fn picked_text(n: usize, none: &str) -> String {
    match n {
        0 => none.to_owned(),
        1 => "1 picked".to_owned(),
        n => format!("{n} picked"),
    }
}

fn toggle<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if let Some(i) = list.iter().position(|x| *x == item) {
        list.remove(i);
    } else {
        list.push(item);
    }
}

fn id_of(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl ChamferaApp {
    /// True when some sketch has curves (to sweep or patch).
    pub(crate) fn has_sketch_curves(&self) -> bool {
        self.session
            .doc
            .sketches
            .iter()
            .any(|f| f.sketch.curves().next().is_some())
    }

    /// Opens a surface tool, starting from the selected bodies for stitch and trim.
    pub(crate) fn open_surface_tool(&mut self, kind: SurfaceKind) {
        let mut tool = SurfaceTool::new(kind);
        if matches!(kind, SurfaceKind::Stitch | SurfaceKind::Trim) {
            tool.bodies = self.session.selected_solid_names();
            if kind == SurfaceKind::Trim {
                tool.bodies.truncate(1);
            }
        }
        self.solid.surface = Some(tool);
        self.message = Some((kind.hint().into(), false));
    }

    /// A click in the viewport while a surface tool is open.
    pub(crate) fn surface_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let at = json!({ "x": x, "y": y, "width": w, "height": h });
        let Some((kind, picking)) = self.solid.surface.as_ref().map(|t| (t.kind, t.picking)) else {
            return;
        };
        if matches!(kind, SurfaceKind::Stitch | SurfaceKind::Trim) {
            self.run("select.pick", at);
            let picked = self.session.selected_body_names();
            if let Some(tool) = self.solid.surface.as_mut() {
                for name in picked {
                    if kind == SurfaceKind::Trim {
                        tool.bodies = vec![name];
                    } else {
                        toggle(&mut tool.bodies, name);
                    }
                }
            }
            return;
        }
        let path = self.session.execute("solid.path_at", &at).ok();
        let path_sketch = path.as_ref().and_then(|r| id_of(&r["sketch"]));
        let path_curve = path.as_ref().and_then(|r| id_of(&r["curve"]));
        let chain: Vec<u32> = path
            .as_ref()
            .and_then(|r| r["chain"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(id_of)
            .collect();
        if picking == SurfacePick::Axis || picking == SurfacePick::Path {
            if let Some(tool) = self.solid.surface.as_mut() {
                let other_path = tool.editing.is_some()
                    && path_sketch.is_some()
                    && path_sketch != tool.path_sketch;
                match (picking, path_sketch, path_curve) {
                    (SurfacePick::Axis, Some(_), Some(c)) => tool.axis = Some(c),
                    (SurfacePick::Path, Some(s), Some(_)) if !other_path => {
                        tool.path_sketch = Some(s);
                        tool.path = chain;
                    }
                    _ => {}
                }
            }
            return;
        }
        // Shapes: an open curve under the pointer first, else the profile there, else
        // (for a patch) a model edge.
        if let (Some(s), Some(c)) = (path_sketch, path_curve)
            && kind != SurfaceKind::Patch
        {
            if let Some(tool) = self.solid.surface.as_mut()
                && tool.sketch.is_none_or(|t| t == s)
            {
                tool.sketch = Some(s);
                toggle(&mut tool.curves, c);
            }
            return;
        }
        // An edited patch keeps what it was made from: profiles of its sketch, or edges.
        let edited_edges = self
            .solid
            .surface
            .as_ref()
            .is_some_and(|t| t.editing.is_some() && t.sketch.is_none());
        let edited_profiles = self
            .solid
            .surface
            .as_ref()
            .is_some_and(|t| t.editing.is_some() && t.sketch.is_some());
        if !edited_edges
            && let Ok(r) = self.session.execute("solid.profile_at", &at)
            && let (Some(s), Some(px), Some(py)) =
                (id_of(&r["sketch"]), r["x"].as_f64(), r["y"].as_f64())
        {
            if let Some(tool) = self.solid.surface.as_mut()
                && tool.sketch.is_none_or(|t| t == s)
            {
                tool.sketch = Some(s);
                tool.edges.clear();
                toggle(&mut tool.profiles, Point2::new(px, py));
            }
            return;
        }
        if kind == SurfaceKind::Patch
            && !edited_profiles
            && let Ok(r) = self.session.execute("solid.edge_at", &at)
            && let Some(e) = r["edge"].as_str()
            && let Some(tool) = self.solid.surface.as_mut()
        {
            tool.profiles.clear();
            tool.sketch = None;
            toggle(&mut tool.edges, e.to_owned());
        }
    }

    /// Runs the open tool's command; closes the tool when it worked.
    pub(crate) fn surface_ok(&mut self) -> Option<Value> {
        let tool = self.solid.surface.clone()?;
        let planes = self.construct_planes();
        let r = match tool.editing {
            Some(id) => {
                let r = self.run("feature.edit", tool.edit_params(id, &planes))?;
                self.message = Some((
                    format!("{} updated", r["name"].as_str().unwrap_or("Feature")),
                    false,
                ));
                self.solid.surface = None;
                return Some(r);
            }
            None => self.run(tool.kind.command(), tool.params(&planes))?,
        };
        let units = self.units();
        let shown: Vec<String> = r["solids"]
            .as_array()
            .map(|l| {
                l.iter()
                    .filter_map(|s| {
                        let name = s["name"].as_str()?;
                        Some(match s["volume"].as_f64() {
                            Some(v) => format!("{name} {}", units.volume(v)),
                            None => format!("{name} surface {}", units.area(s["area"].as_f64()?)),
                        })
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
        self.solid.surface = None;
        Some(r)
    }

    /// Edit Feature on a surface sweep, patch, stitch or trim: reopens its tool with the
    /// feature's picks, to change them. False for other features.
    pub(crate) fn edit_surface_feature(&mut self, id: u32) -> bool {
        let planes = self.construct_planes();
        let Some(tool) = self
            .session
            .doc
            .feature(id)
            .ok()
            .and_then(|f| match &f.kind {
                chamfera_doc::FeatureKind::Surface(sf) => SurfaceTool::editing(id, sf, &planes),
                _ => None,
            })
        else {
            return false;
        };
        self.message = Some((
            format!("{}: change the picks, then OK", tool.kind.hint()),
            false,
        ));
        self.solid.surface = Some(tool);
        true
    }

    /// The bodies a trim can be cut by: every body but the one trimmed.
    fn trim_tools(&self) -> Vec<String> {
        let trimmed = self.solid.surface.as_ref().and_then(|t| t.bodies.first());
        self.session
            .solids()
            .iter()
            .map(|s| s.name.clone())
            .filter(|n| Some(n) != trimmed)
            .collect()
    }

    /// The open surface tool's dialog.
    pub(crate) fn surface_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let planes = self.construct_planes();
        let trim_tools = self.trim_tools();
        let edited = self
            .solid
            .surface
            .as_ref()
            .and_then(|t| t.editing)
            .and_then(|id| self.session.doc.feature(id).ok())
            .map(|f| f.name.clone());
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.surface.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title_text = match &edited {
            Some(name) => format!("{}: {name}", label("feature.edit")),
            None => label(tool.kind.command()).to_owned(),
        };
        let title = title_text.as_str();
        let ready = tool.ready();
        let plane_names: Vec<String> = ORIGIN_PLANES
            .iter()
            .map(|(_, n)| (*n).to_owned())
            .chain(planes.iter().map(|(_, n)| n.clone()))
            .collect();
        egui::Window::new(title)
            .id(egui::Id::unique("surface tool dialog"))
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
                    egui::Grid::new("surface tool grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            let text_field = |ui: &mut egui::Ui, value: &mut String, hint: &str| {
                                ui.add_sized(
                                    [180.0, 26.0],
                                    egui::TextEdit::singleline(value)
                                        .hint_text(hint)
                                        .align(egui::Align2::LEFT_CENTER),
                                )
                            };
                            match tool.kind {
                                SurfaceKind::Extrude
                                | SurfaceKind::Revolve
                                | SurfaceKind::Sweep => {
                                    widgets::field_label(ui, look, "Profiles");
                                    if ui
                                        .selectable_label(
                                            tool.picking == SurfacePick::Shapes,
                                            picked_text(
                                                tool.profiles.len() + tool.curves.len(),
                                                "Click profiles or curves",
                                            ),
                                        )
                                        .clicked()
                                    {
                                        tool.picking = SurfacePick::Shapes;
                                    }
                                    ui.end_row();
                                }
                                SurfaceKind::Patch => {
                                    widgets::field_label(ui, look, "Boundary");
                                    ui.label(if tool.edges.is_empty() {
                                        picked_text(tool.profiles.len(), "Click a profile or edges")
                                    } else {
                                        format!("{} edges", tool.edges.len())
                                    });
                                    ui.end_row();
                                }
                                SurfaceKind::Stitch => {
                                    widgets::field_label(ui, look, "Surfaces");
                                    ui.label(picked_text(tool.bodies.len(), "Click the surfaces"));
                                    ui.end_row();
                                }
                                SurfaceKind::Trim => {
                                    widgets::field_label(ui, look, "Surface");
                                    ui.label(
                                        tool.bodies
                                            .first()
                                            .map_or("Click a surface", |s| s.as_str()),
                                    );
                                    ui.end_row();
                                }
                            }
                            match tool.kind {
                                SurfaceKind::Extrude => {
                                    widgets::field_label(ui, look, "Distance");
                                    if text_field(ui, &mut tool.distance, "10 mm").lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        ok = true;
                                    }
                                    ui.end_row();
                                    widgets::field_label(ui, look, "Symmetric");
                                    ui.checkbox(&mut tool.symmetric, "Half each way");
                                    ui.end_row();
                                }
                                SurfaceKind::Revolve => {
                                    widgets::field_label(ui, look, "Axis");
                                    if ui
                                        .selectable_label(
                                            tool.picking == SurfacePick::Axis,
                                            tool.axis
                                                .map_or("Pick a sketch line".to_owned(), |a| {
                                                    format!("Line {a}")
                                                }),
                                        )
                                        .clicked()
                                    {
                                        tool.picking = SurfacePick::Axis;
                                    }
                                    ui.end_row();
                                    widgets::field_label(ui, look, "Angle");
                                    text_field(ui, &mut tool.angle, "360 deg");
                                    ui.end_row();
                                }
                                SurfaceKind::Sweep => {
                                    widgets::field_label(ui, look, "Path");
                                    if ui
                                        .selectable_label(
                                            tool.picking == SurfacePick::Path,
                                            picked_text(tool.path.len(), "Pick the path"),
                                        )
                                        .clicked()
                                    {
                                        tool.picking = SurfacePick::Path;
                                    }
                                    ui.end_row();
                                }
                                SurfaceKind::Trim => {
                                    widgets::field_label(ui, look, "Trim with");
                                    let current = match (&tool.trim_body, &tool.trim_face) {
                                        (Some(b), _) => format!("Body {b}"),
                                        (None, Some(f)) => format!(
                                            "Face {}:{}",
                                            f["feature"],
                                            f["face"].as_str().unwrap_or("")
                                        ),
                                        (None, None) => {
                                            plane_names.get(tool.plane).cloned().unwrap_or_default()
                                        }
                                    };
                                    egui::ComboBox::from_id_salt("surface trim plane")
                                        .selected_text(current)
                                        .width(180.0)
                                        .show_ui(ui, |ui| {
                                            for (i, o) in plane_names.iter().enumerate() {
                                                let on = tool.trim_body.is_none()
                                                    && tool.trim_face.is_none()
                                                    && tool.plane == i;
                                                if ui.selectable_label(on, o).clicked() {
                                                    tool.plane = i;
                                                    tool.trim_body = None;
                                                    tool.trim_face = None;
                                                }
                                            }
                                            for b in &trim_tools {
                                                let on = tool.trim_body.as_ref() == Some(b);
                                                if ui
                                                    .selectable_label(on, format!("Body {b}"))
                                                    .clicked()
                                                {
                                                    tool.trim_body = Some(b.clone());
                                                    tool.trim_face = None;
                                                }
                                            }
                                        });
                                    ui.end_row();
                                    widgets::field_label(ui, look, "Keep");
                                    let names: [&str; 2] = if tool.trim_body.is_some() {
                                        KEEP_BODY
                                    } else {
                                        [KEEP[0].1, KEEP[1].1]
                                    };
                                    let current = names.get(tool.keep).copied().unwrap_or("");
                                    egui::ComboBox::from_id_salt("surface trim keep")
                                        .selected_text(current)
                                        .width(180.0)
                                        .show_ui(ui, |ui| {
                                            for (i, n) in names.iter().enumerate() {
                                                ui.selectable_value(&mut tool.keep, i, *n);
                                            }
                                        });
                                    ui.end_row();
                                }
                                SurfaceKind::Patch | SurfaceKind::Stitch => {}
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
            self.surface_ok();
        }
        if cancel {
            self.solid.surface = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_surface_extrude_tool_builds_open_sides_of_area_1600() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.open_surface_tool(SurfaceKind::Extrude);
        let tool = app.solid.surface.as_mut().unwrap();
        assert!(!tool.ready());
        tool.sketch = Some(1);
        tool.profiles = vec![Point2::new(25.0, 15.0)];
        assert!(tool.ready());
        let r = app.surface_ok().unwrap();
        assert!(app.solid.surface.is_none());
        // 2 (50 + 30) x 10 = 1600, no volume.
        let s = &r["solids"][0];
        assert!((s["area"].as_f64().unwrap() - 1600.0).abs() < 1e-6);
        assert!(s["volume"].is_null());
        assert_eq!(s["surface"], true);
        let (icon, text) =
            details(surface_of(&app.session.doc.features.last().unwrap().kind).unwrap());
        assert_eq!(icon, Icon::SurfaceExtrude);
        assert!(text.starts_with("Surface extrude 10 mm"), "{text}");
    }

    #[test]
    fn trim_params_name_the_plane_and_side() {
        let mut tool = SurfaceTool::new(SurfaceKind::Trim);
        assert!(!tool.ready());
        tool.bodies = vec!["SurfaceRevolve1".into()];
        tool.plane = 1;
        tool.keep = 1;
        let p = tool.params(&[]);
        assert_eq!(p["plane"], "xz");
        assert_eq!(p["keep"], "above");
        assert!(tool.ready());
    }

    fn rectangle(app: &mut ChamferaApp, x1: f64, y1: f64, x2: f64, y2: f64) {
        app.run(
            "sketch.rectangle",
            json!({ "x1": x1, "y1": y1, "x2": x2, "y2": y2 }),
        );
    }

    fn area_of(app: &ChamferaApp, name: &str) -> f64 {
        app.session
            .solids()
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.body.area().ok())
            .unwrap_or(f64::NAN)
    }

    /// Trim Surface by a body from the dialog (QA user guide pass: it trimmed by planes
    /// only). The 50 x 30 rectangle's sides extruded 10 (area 1600) cut by a block over
    /// x -10..10: outside it stay the long walls from x = 10 (2 x 40 x 10 = 800) and the
    /// end wall x = 50 (30 x 10 = 300), 1100 in all.
    #[test]
    fn the_trim_dialog_trims_a_surface_by_a_body() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        rectangle(&mut app, 0.0, 0.0, 50.0, 30.0);
        app.run("sketch.finish", json!({}));
        app.run(
            "surface.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        );
        let sk = app.run("sketch.create", json!({ "plane": "xy" })).unwrap();
        rectangle(&mut app, -10.0, -10.0, 10.0, 40.0);
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": sk["sketch"], "profiles": [{ "x": 0, "y": 15 }],
                    "distance": 40, "direction": "symmetric" }),
        );
        let block = app
            .session
            .solids()
            .iter()
            .map(|s| s.name.clone())
            .find(|n| n != "SurfaceExtrude1")
            .unwrap();
        app.open_surface_tool(SurfaceKind::Trim);
        let tool = app.solid.surface.as_mut().unwrap();
        tool.bodies = vec!["SurfaceExtrude1".into()];
        tool.trim_body = Some(block.clone());
        let p = tool.params(&[]);
        assert_eq!(p["tool"], block);
        assert!(p.get("plane").is_none());
        // The body that is trimmed cannot trim itself.
        assert!(app.trim_tools().iter().all(|n| n != "SurfaceExtrude1"));
        app.surface_ok().unwrap();
        let a = area_of(&app, "SurfaceExtrude1");
        assert!((a - 1100.0).abs() < 1e-6, "{a}");
    }

    /// Edit Feature on a patch reopens the Patch tool with its profile (QA user guide
    /// pass: sweeps and patches only showed their definition). Patched from the 50 x 30
    /// rectangle (area 1500), then changed to the 10 x 10 one beside it: area 100.
    #[test]
    fn edit_feature_reopens_a_patch_to_change_its_profile() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        rectangle(&mut app, 0.0, 0.0, 50.0, 30.0);
        rectangle(&mut app, 60.0, 0.0, 70.0, 10.0);
        app.run("sketch.finish", json!({}));
        let r = app
            .run(
                "surface.patch",
                json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }] }),
            )
            .unwrap();
        let id = u32::try_from(r["feature"].as_u64().unwrap()).unwrap();
        assert!((area_of(&app, "Patch1") - 1500.0).abs() < 1e-6);
        let before = app.session.doc.features.len();
        app.edit_feature(id);
        assert!(app.timeline.editing.is_none(), "no view-only dialog");
        let tool = app.solid.surface.as_mut().unwrap();
        assert_eq!(tool.kind, SurfaceKind::Patch);
        assert_eq!(tool.editing, Some(id));
        assert_eq!(tool.profiles, vec![Point2::new(25.0, 15.0)]);
        tool.profiles = vec![Point2::new(65.0, 5.0)];
        let r = app.surface_ok().unwrap();
        assert_eq!(r["name"], "Patch1");
        assert!(app.solid.surface.is_none());
        assert_eq!(app.session.doc.features.len(), before, "edited, not added");
        let a = area_of(&app, "Patch1");
        assert!((a - 100.0).abs() < 1e-6, "{a}");
    }
}

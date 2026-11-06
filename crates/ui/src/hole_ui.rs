//! The Hole tool: click points on a flat face (click a point again to remove it), choose
//! simple, counterbore or countersink, set the sizes, OK. The circles of each hole are
//! drawn on the face as a preview. Acts only through `solid.hole` and `feature.edit`.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{Hole, HoleKind};
use chamfera_geom::{Frame, Point2, Point3};
use chamfera_render::{Overlay, OverlayStyle};
use serde_json::{Value, json};

/// The Hole tool while it is open.
pub struct HoleTool {
    /// The hole feature being edited (from the timeline), or `None` for a new one.
    pub editing: Option<u32>,
    /// The face the holes go on, by name (`2:top`).
    pub face: Option<String>,
    /// Hole centres in the face's plane coordinates.
    pub points: Vec<Point2>,
    pub kind: HoleKind,
    pub diameter: String,
    pub through: bool,
    pub depth: String,
    pub cb_diameter: String,
    pub cb_depth: String,
    pub cs_diameter: String,
    pub cs_angle: String,
    /// A drill point at the bottom of a blind hole.
    pub tip: bool,
    pub tip_angle: String,
    /// Where the pointer is on a face, for the preview.
    pub hover: Option<(String, Point2)>,
}

impl Default for HoleTool {
    fn default() -> Self {
        HoleTool {
            editing: None,
            face: None,
            points: Vec::new(),
            kind: HoleKind::Simple,
            diameter: "6 mm".into(),
            through: true,
            depth: "10 mm".into(),
            cb_diameter: "10 mm".into(),
            cb_depth: "3 mm".into(),
            cs_diameter: "12 mm".into(),
            cs_angle: "90 deg".into(),
            tip: true,
            tip_angle: "118 deg".into(),
            hover: None,
        }
    }
}

impl HoleTool {
    /// The tool for editing hole feature `id`.
    pub fn editing(id: u32, h: &Hole) -> HoleTool {
        let d = HoleTool::default();
        HoleTool {
            editing: Some(id),
            face: Some(format!("{}:{}", h.face.feature, h.face.face)),
            points: h.points.clone(),
            kind: h.hole,
            diameter: h.diameter.clone(),
            through: h.depth.is_none(),
            depth: h.depth.clone().unwrap_or(d.depth),
            cb_diameter: h.cb_diameter.clone().unwrap_or(d.cb_diameter),
            cb_depth: h.cb_depth.clone().unwrap_or(d.cb_depth),
            cs_diameter: h.cs_diameter.clone().unwrap_or(d.cs_diameter),
            cs_angle: h.cs_angle.clone().unwrap_or(d.cs_angle),
            tip: h.tip_angle.is_some(),
            tip_angle: h.tip_angle.clone().unwrap_or(d.tip_angle),
            hover: None,
        }
    }

    /// The `solid.hole` (or `feature.edit`) parameters for the values in the dialog.
    pub fn params(&self) -> Value {
        let t = |s: &String| json!(s.trim());
        let mut p = json!({
            "points": self.points.iter().map(|q| json!({ "x": q.x, "y": q.y })).collect::<Vec<_>>(),
            "kind": self.kind.key(),
            "diameter": t(&self.diameter),
            "tip_angle": if self.tip { t(&self.tip_angle) } else { json!("flat") },
        });
        if self.through {
            p["through_all"] = json!(true);
        } else {
            p["depth"] = t(&self.depth);
        }
        match self.kind {
            HoleKind::Simple => {}
            HoleKind::Counterbore => {
                p["cb_diameter"] = t(&self.cb_diameter);
                p["cb_depth"] = t(&self.cb_depth);
            }
            HoleKind::Countersink => {
                p["cs_diameter"] = t(&self.cs_diameter);
                p["cs_angle"] = t(&self.cs_angle);
            }
        }
        match (self.editing, &self.face) {
            (Some(id), _) => p["feature"] = json!(id),
            (None, Some(face)) => p["face"] = json!(face),
            (None, None) => {}
        }
        p
    }
}

fn kind_name(k: HoleKind) -> &'static str {
    match k {
        HoleKind::Simple => "Simple",
        HoleKind::Counterbore => "Counterbore",
        HoleKind::Countersink => "Countersink",
    }
}

/// The Hole button of the SOLID ribbon's CREATE group; true when it was clicked to open
/// the tool.
pub(crate) fn hole_button(
    ui: &mut egui::Ui,
    look: Look,
    has_solids: bool,
    tool_free: bool,
    on: bool,
) -> bool {
    widgets::tool_button(
        ui,
        look,
        Icon::Hole,
        label("solid.hole"),
        has_solids && (tool_free || on),
        on,
        false,
    )
    .on_hover_text(if has_solids {
        "Hole\nDrill simple, counterbored or countersunk holes at points on a flat face"
    } else {
        "Hole\nBuild a solid first"
    })
    .clicked()
        && tool_free
}

/// The timeline's one-line summary of a hole feature.
pub(crate) fn hole_details(h: &Hole) -> String {
    let n = h.points.len();
    let extent = match &h.depth {
        Some(d) => format!("{d} deep"),
        None => "through all".into(),
    };
    let points = if h.sketch.is_some() {
        "sketch points".to_owned()
    } else {
        format!("{n} point{}", if n == 1 { "" } else { "s" })
    };
    format!(
        "{} hole Ø{} · {extent} · {points} on {}",
        kind_name(h.hole),
        h.diameter,
        h.face.face
    )
}

/// A circle of radius `r` about `c` in `plane`, as line segments.
fn circle(plane: &Frame, c: Point2, r: f64) -> Vec<[Point3; 2]> {
    const N: usize = 48;
    let at = |k: usize| {
        let a = std::f64::consts::TAU * k as f64 / N as f64;
        plane.from_plane(Point2::new(c.x + r * a.cos(), c.y + r * a.sin()))
    };
    (0..N).map(|k| [at(k), at(k + 1)]).collect()
}

impl ChamferaApp {
    /// Opens the Hole tool.
    pub(crate) fn open_hole_tool(&mut self) {
        self.solid.hole = Some(HoleTool::default());
        self.message = Some((
            "Click points on a flat face to drill holes there".into(),
            false,
        ));
    }

    /// `--hole`: opens the Hole tool on the design's last hole feature, or a new one.
    pub(crate) fn open_hole_on_last(&mut self) {
        let last = self
            .session
            .doc
            .features
            .iter()
            .rev()
            .find_map(|f| match &f.kind {
                chamfera_doc::FeatureKind::Hole(h) => Some(HoleTool::editing(f.id, h)),
                _ => None,
            });
        match last {
            Some(tool) => self.solid.hole = Some(tool),
            None => self.open_hole_tool(),
        }
    }

    /// A click in the viewport while the tool is open: adds a hole point on the face
    /// there, or removes the point under the click.
    pub(crate) fn hole_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some((feature, role, p)) = self.session.face_point_at(x, y, w, h) else {
            return;
        };
        let key = format!("{feature}:{role}");
        let radius = self.hole_radius();
        let Some(tool) = self.solid.hole.as_mut() else {
            return;
        };
        if tool.face.as_ref().is_some_and(|f| *f != key) {
            self.message = Some((
                "One hole feature drills one face: click points on the same face".into(),
                true,
            ));
            return;
        }
        tool.face = Some(key);
        let near = tool
            .points
            .iter()
            .position(|q| (q.x - p.x).hypot(q.y - p.y) <= radius.max(0.5));
        match near {
            Some(i) => {
                tool.points.remove(i);
                if tool.points.is_empty() && tool.editing.is_none() {
                    tool.face = None;
                }
            }
            None => tool.points.push(p),
        }
    }

    /// The pointer moved over the viewport while the tool is open.
    pub(crate) fn hole_hover(&mut self, at: Option<(f64, f64)>, w: f64, h: f64) {
        let hover = at
            .and_then(|(x, y)| self.session.face_point_at(x, y, w, h))
            .map(|(f, role, p)| (format!("{f}:{role}"), p));
        if let Some(tool) = self.solid.hole.as_mut() {
            tool.hover = hover;
        }
    }

    /// The hole's radius in the dialog, if it evaluates (for the preview and clicks).
    fn hole_radius(&self) -> f64 {
        self.solid
            .hole
            .as_ref()
            .and_then(|t| {
                self.session
                    .doc
                    .evaluate_feature_value(&t.diameter, chamfera_doc::expr::Dim::LENGTH)
                    .ok()
            })
            .filter(|d| d.is_finite() && *d > 0.0 && *d < 1e6)
            .map_or(1.0, |d| d * 0.5)
    }

    /// The radii drawn for one hole: the hole and its counterbore or countersink.
    fn hole_radii(&self) -> Vec<f64> {
        let mut out = vec![self.hole_radius()];
        let Some(tool) = self.solid.hole.as_ref() else {
            return out;
        };
        let outer = match tool.kind {
            HoleKind::Simple => None,
            HoleKind::Counterbore => Some(&tool.cb_diameter),
            HoleKind::Countersink => Some(&tool.cs_diameter),
        };
        if let Some(d) = outer.and_then(|d| {
            self.session
                .doc
                .evaluate_feature_value(d, chamfera_doc::expr::Dim::LENGTH)
                .ok()
        }) && d.is_finite()
            && d > 0.0
            && d < 1e6
        {
            out.push(d * 0.5);
        }
        out
    }

    /// The circles of the placed holes (highlighted) and of one under the pointer.
    pub(crate) fn hole_overlays(&self) -> Vec<Overlay> {
        let Some(tool) = self.solid.hole.as_ref() else {
            return Vec::new();
        };
        let radii = self.hole_radii();
        let mut out = Vec::new();
        if let Some(plane) = tool
            .face
            .as_ref()
            .and_then(|f| self.session.named_face_plane(f))
        {
            let mut lines = Vec::new();
            for p in &tool.points {
                for r in &radii {
                    lines.extend(circle(&plane, *p, *r));
                }
            }
            out.push(Overlay {
                style: OverlayStyle::Highlight,
                lines,
                points: tool.points.iter().map(|p| plane.from_plane(*p)).collect(),
                on_top: true,
            });
        }
        if let Some((face, p)) = &tool.hover
            && tool.face.as_ref().is_none_or(|f| f == face)
            && let Some(plane) = self.session.named_face_plane(face)
        {
            out.push(Overlay {
                style: OverlayStyle::Preview,
                lines: radii.iter().flat_map(|r| circle(&plane, *p, *r)).collect(),
                points: vec![plane.from_plane(*p)],
                on_top: true,
            });
        }
        out
    }

    /// Runs the hole command for the dialog's values; closes the tool when it works.
    pub(crate) fn hole_ok(&mut self) -> Option<Value> {
        let tool = self.solid.hole.as_ref()?;
        let command = if tool.editing.is_some() {
            "feature.edit"
        } else {
            "solid.hole"
        };
        let r = self.run(command, tool.params())?;
        let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
        self.message = Some((
            format!(
                "{} · volume {}",
                r["name"].as_str().unwrap_or(label("solid.hole")),
                self.units().volume(v),
            ),
            false,
        ));
        self.solid.hole = None;
        Some(r)
    }

    /// The Hole dialog.
    pub(crate) fn hole_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 320.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.hole.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let title = match tool.editing {
            Some(_) => label("feature.edit"),
            None => label("solid.hole"),
        };
        let ready = !tool.points.is_empty() && !tool.diameter.trim().is_empty();
        egui::Window::new(title)
            .id(egui::Id::unique("hole dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Hole, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("hole grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            let mut enter = false;
                            let mut field =
                                |ui: &mut egui::Ui, name: &str, v: &mut String, hint: &str| {
                                    widgets::field_label(ui, look, name);
                                    let r = ui.add_sized(
                                        [180.0, 26.0],
                                        egui::TextEdit::singleline(v)
                                            .hint_text(hint)
                                            .align(egui::Align2::LEFT_CENTER),
                                    );
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        enter = true;
                                    }
                                    ui.end_row();
                                };
                            widgets::field_label(ui, look, "Points");
                            let text = match tool.points.len() {
                                0 => "Click a flat face".to_owned(),
                                1 => "1 placed".to_owned(),
                                n => format!("{n} placed"),
                            };
                            let colour = if tool.points.is_empty() {
                                look.c(|t| t.text_muted)
                            } else {
                                look.c(|t| t.text)
                            };
                            ui.label(egui::RichText::new(text).color(colour))
                                .on_hover_text("Click a placed point again to remove it");
                            ui.end_row();
                            widgets::field_label(ui, look, "Type");
                            egui::ComboBox::from_id_salt("hole kind")
                                .selected_text(kind_name(tool.kind))
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for k in [
                                        HoleKind::Simple,
                                        HoleKind::Counterbore,
                                        HoleKind::Countersink,
                                    ] {
                                        ui.selectable_value(&mut tool.kind, k, kind_name(k));
                                    }
                                });
                            ui.end_row();
                            field(ui, "Diameter", &mut tool.diameter, "6 mm");
                            widgets::field_label(ui, look, "Extent");
                            egui::ComboBox::from_id_salt("hole extent")
                                .selected_text(if tool.through {
                                    "Through all"
                                } else {
                                    "Distance"
                                })
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut tool.through, false, "Distance");
                                    ui.selectable_value(&mut tool.through, true, "Through all");
                                });
                            ui.end_row();
                            if !tool.through {
                                field(ui, "Depth", &mut tool.depth, "10 mm");
                            }
                            match tool.kind {
                                HoleKind::Simple => {}
                                HoleKind::Counterbore => {
                                    field(ui, "Counterbore Ø", &mut tool.cb_diameter, "10 mm");
                                    field(ui, "Counterbore depth", &mut tool.cb_depth, "3 mm");
                                }
                                HoleKind::Countersink => {
                                    field(ui, "Countersink Ø", &mut tool.cs_diameter, "12 mm");
                                    field(ui, "Countersink angle", &mut tool.cs_angle, "90 deg");
                                }
                            }
                            if !tool.through {
                                widgets::field_label(ui, look, "Drill point");
                                ui.checkbox(&mut tool.tip, "");
                                ui.end_row();
                                if tool.tip {
                                    field(ui, "Point angle", &mut tool.tip_angle, "118 deg");
                                }
                            }
                            if enter {
                                ok = true;
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
            self.hole_ok();
        }
        if cancel {
            self.solid.hole = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A 50 x 30 x 10 plate (feature 2), seen from straight above and fitted.
    fn plate_from_above(w: f64, h: f64) -> ChamferaApp {
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
        app.run("view.normal_to", json!({ "x": 0, "y": 0, "z": 1 }));
        app.run("view.fit", json!({ "width": w, "height": h }));
        app
    }

    #[test]
    fn clicking_the_face_places_points_and_ok_drills_them() {
        let (w, h) = (800.0, 600.0);
        let mut app = plate_from_above(w, h);
        app.solid.hole = Some(HoleTool::default());
        // The middle of the view is the middle of the top, (25, 15, 10).
        app.hole_click(w / 2.0, h / 2.0, w, h);
        assert_eq!(app.solid.hole.as_ref().unwrap().points.len(), 1);
        assert_eq!(
            app.solid.hole.as_ref().unwrap().face.as_deref(),
            Some("2:top")
        );
        // The preview draws the hole's circle on the face.
        assert!(!app.hole_overlays()[0].lines.is_empty());
        // Clicking it again removes it; a third click puts it back.
        app.hole_click(w / 2.0, h / 2.0, w, h);
        assert!(app.solid.hole.as_ref().unwrap().points.is_empty());
        app.hole_click(w / 2.0, h / 2.0, w, h);
        if let Some(t) = app.solid.hole.as_mut() {
            t.kind = HoleKind::Counterbore;
        }
        let r = app.hole_ok().unwrap();
        assert!(app.solid.hole.is_none());
        // Counterbore d 10, 3 deep, over a d 6 through hole: 15000 - π(25·3 + 9·7).
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        let want = 15_000.0 - PI * (75.0 + 63.0);
        assert!((v - want).abs() < 1e-6, "{v} vs {want}");
        // Editing it from the timeline opens the tool on the same hole.
        let id = r["feature"].as_u64().unwrap() as u32;
        let Ok(chamfera_doc::Feature {
            kind: chamfera_doc::FeatureKind::Hole(hole),
            ..
        }) = app.session.doc.feature(id).cloned()
        else {
            panic!("not a hole")
        };
        let mut tool = HoleTool::editing(id, &hole);
        tool.kind = HoleKind::Simple;
        app.solid.hole = Some(tool);
        let r = app.hole_ok().unwrap();
        let v = r["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - (15_000.0 - 90.0 * PI)).abs() < 1e-6, "{v}");
    }
}

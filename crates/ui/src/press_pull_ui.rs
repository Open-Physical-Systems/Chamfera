//! The Press Pull tool (Q), and Offset Face and Delete Face from the SOLID MODIFY group.
//!
//! Press Pull: click a sketch profile, a face or an edge; the tool picks the operation as
//! `solid.press_pull` does (a profile is extruded, a face offset, a fillet's face changes
//! that fillet's radius, an edge is filleted). Drag the arrow to change the distance live
//! (a translucent preview follows through `solid.press_pull` with `preview`), or type it,
//! then OK. Offset Face is the same tool limited to faces; Delete Face collects faces and
//! runs `solid.delete_face`. Everything acts through commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, color, label};
use chamfera_engine::face_edit::{PressPull, PressPullTarget};
use chamfera_geom::Point3;
use chamfera_render::{Overlay, OverlayStyle, mat_mul, transform_point};
use egui::{Pos2, Rect, Vec2};
use serde_json::{Value, json};

/// What the tool was opened as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// Press Pull: profiles, faces and edges.
    Auto,
    /// Offset Face: faces only, always an offset.
    Offset,
    /// Delete Face: pick faces to remove.
    Delete,
}

/// A drag of the arrow in progress: where it started and the value then.
#[derive(Debug, Clone, Copy)]
struct Drag {
    start: Pos2,
    value: f64,
}

/// The Press Pull (or Offset Face, Delete Face) tool while it is open.
pub struct PressPullTool {
    pub mode: PullMode,
    /// What the distance applies to (Press Pull and Offset Face).
    pub target: Option<PressPullTarget>,
    /// The faces to delete (Delete Face).
    pub faces: Vec<String>,
    /// The distance (or radius) expression.
    pub value: String,
    drag: Option<Drag>,
    /// The parameters the preview shows, and what it said: the volume or why not.
    preview_for: Option<Value>,
    preview: Result<f64, String>,
    /// `--press-pull X,Y,D`: pick at viewport (X, Y) and drag to D on the first frame.
    pub demo: Option<[f32; 3]>,
}

impl PressPullTool {
    pub fn new(mode: PullMode) -> PressPullTool {
        PressPullTool {
            mode,
            target: None,
            faces: Vec::new(),
            value: "5 mm".into(),
            drag: None,
            preview_for: None,
            preview: Err(String::new()),
            demo: None,
        }
    }

    fn command(&self) -> &'static str {
        match self.mode {
            PullMode::Auto => "solid.press_pull",
            PullMode::Offset => "solid.offset_face",
            PullMode::Delete => "solid.delete_face",
        }
    }

    /// The command parameters for the current target and value, if complete.
    fn params(&self) -> Option<Value> {
        if self.mode == PullMode::Delete {
            return (!self.faces.is_empty()).then(|| json!({ "faces": self.faces }));
        }
        let t = self.target.as_ref()?;
        let value = self.value.trim();
        if value.is_empty() {
            return None;
        }
        let mut p = t.params.clone();
        p["distance"] = json!(value);
        Some(p)
    }

    /// The dialog's field name for the value.
    fn value_label(&self) -> &'static str {
        match self.target.as_ref().map(|t| &t.operation) {
            Some(PressPull::Fillet { .. } | PressPull::FilletRadius { .. }) => "Radius",
            _ => "Distance",
        }
    }

    /// The value as a number of mm, when it is a plain length.
    fn number(&self) -> Option<f64> {
        let v = self.value.trim();
        let v = v.strip_suffix("mm").unwrap_or(v).trim();
        v.parse::<f64>().ok().filter(|x| x.is_finite())
    }

    /// The arrow's base and direction: a face or profile's arrow rides on the moved face,
    /// an edge's sits on the edge and points out of the solid (dragging it in grows the
    /// fillet).
    fn arrow(&self) -> Option<(Point3, chamfera_geom::Vec3, bool)> {
        let t = self.target.as_ref()?;
        let rides = matches!(
            t.operation,
            PressPull::Extrude | PressPull::OffsetFace { .. }
        );
        let base = if rides {
            t.anchor + t.direction * self.number().unwrap_or(0.0)
        } else {
            t.anchor
        };
        Some((base, t.direction, rides))
    }
}

/// The Press Pull, Offset Face and Delete Face buttons of the SOLID MODIFY group; the
/// mode of the one clicked.
pub(crate) fn ribbon_buttons(
    ui: &mut egui::Ui,
    look: Look,
    has_any: bool,
    has_solids: bool,
    free: bool,
    open: Option<PullMode>,
) -> Option<PullMode> {
    let mut out = None;
    for (mode, icon, id, tip, enabled) in [
        (
            PullMode::Auto,
            Icon::PressPull,
            "solid.press_pull",
            "Press Pull\nDrag a profile, face or edge: extrudes, offsets the face or fillets",
            has_any,
        ),
        (
            PullMode::Offset,
            Icon::OffsetFace,
            "solid.offset_face",
            "Offset Face\nMove faces along their normals; the faces around them follow",
            has_solids,
        ),
        (
            PullMode::Delete,
            Icon::DeleteFace,
            "solid.delete_face",
            "Delete Face\nRemove fillets, chamfers or holes and heal the solid",
            has_solids,
        ),
    ] {
        let on = open == Some(mode);
        let tip = if mode == PullMode::Auto {
            crate::shortcuts::tip(tip, crate::shortcuts::Action::PressPull)
        } else {
            tip.to_owned()
        };
        if widgets::tool_button(
            ui,
            look,
            icon,
            label(id),
            enabled && (free || on),
            on,
            false,
        )
        .on_hover_text(if enabled {
            tip
        } else {
            "Build a solid or draw a profile first".to_owned()
        })
        .clicked()
            && free
            && enabled
        {
            out = Some(mode);
        }
    }
    out
}

/// The timeline's icon and line for a face edit.
pub(crate) fn timeline_entry(kind: &chamfera_doc::FeatureKind) -> (Icon, String) {
    use chamfera_doc::FeatureKind as K;
    let faces = |n: usize| format!("{n} face{}", if n == 1 { "" } else { "s" });
    match kind {
        K::OffsetFace(o) => (
            Icon::OffsetFace,
            format!("Offset Face {} · {}", o.distance, faces(o.faces.len())),
        ),
        K::DeleteFace(d) => (
            Icon::DeleteFace,
            format!("Delete Face · {}", faces(d.faces.len())),
        ),
        K::MoveFace(m) => (
            Icon::Move,
            format!(
                "Move Face {}{} · {}",
                m.distance,
                m.turn
                    .as_ref()
                    .map(|t| format!(", turn {}", t.angle))
                    .unwrap_or_default(),
                faces(m.faces.len())
            ),
        ),
        _ => (Icon::Edit, String::new()),
    }
}

/// How far, in pixels, from the arrow a press still grabs it.
const GRAB_PIXELS: f32 = 14.0;

impl ChamferaApp {
    /// Opens the tool.
    pub(crate) fn open_press_pull(&mut self, mode: PullMode) {
        self.solid.press_pull = Some(PressPullTool::new(mode));
        let hint = match mode {
            PullMode::Auto => "Click a profile, face or edge, then drag the arrow",
            PullMode::Offset => "Click a face, then drag the arrow",
            PullMode::Delete => "Click the faces to delete (a fillet, a chamfer, a hole's faces)",
        };
        self.message = Some((hint.into(), false));
    }

    /// A click in the viewport: picks what the tool works on.
    pub(crate) fn press_pull_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(mode) = self.solid.press_pull.as_ref().map(|t| t.mode) else {
            return;
        };
        if mode == PullMode::Delete {
            let Some(name) = self.session.face_ref_at(x, y, w, h) else {
                return;
            };
            if let Some(tool) = self.solid.press_pull.as_mut() {
                match tool.faces.iter().position(|f| *f == name) {
                    Some(i) => {
                        tool.faces.remove(i);
                    }
                    None => tool.faces.push(name),
                }
            }
            return;
        }
        let target = self.session.press_pull_target(x, y, w, h);
        let target = match (mode, target) {
            (PullMode::Offset, Some(t)) => {
                let face = t
                    .params
                    .get("face")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                face.map(|face| PressPullTarget {
                    operation: PressPull::OffsetFace { face },
                    ..t
                })
            }
            (_, t) => t,
        };
        let Some(target) = target else {
            return;
        };
        let value = match &target.operation {
            PressPull::FilletRadius { radius, .. } => radius.clone(),
            PressPull::Fillet { .. } => "2 mm".into(),
            _ => "5 mm".into(),
        };
        self.message = Some((format!("Press Pull: {}", target.operation.label()), false));
        if let Some(tool) = self.solid.press_pull.as_mut() {
            tool.target = Some(target);
            tool.value = value;
        }
    }

    /// Projects a world point into the viewport rect.
    fn pull_project(&self, rect: Rect, p: Point3) -> Option<Pos2> {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let bounds = self.session.scene.bounds();
        let vp = mat_mul(
            &self
                .session
                .camera
                .projection_matrix(w / h, bounds.as_ref()),
            &self.session.camera.view_matrix(),
        );
        let n = transform_point(&vp, p)?;
        if !(0.0..=1.0).contains(&n[2]) {
            return None;
        }
        Some(Pos2::new(
            rect.min.x + ((n[0] + 1.0) * 0.5 * w) as f32,
            rect.min.y + ((1.0 - n[1]) * 0.5 * h) as f32,
        ))
    }

    /// The arrow's length in mm: a share of the scene's size.
    fn arrow_length(&self) -> f64 {
        self.session
            .scene
            .bounds()
            .map_or(10.0, |b| (b.max - b.min).length() * 0.15)
            .max(1.0)
    }

    /// The arrow on screen: base and tip, in viewport points.
    fn arrow_on_screen(&self, rect: Rect) -> Option<(Pos2, Pos2)> {
        let (base, dir, _) = self.solid.press_pull.as_ref()?.arrow()?;
        let tip = base + dir * self.arrow_length();
        Some((
            self.pull_project(rect, base)?,
            self.pull_project(rect, tip)?,
        ))
    }

    /// Viewport input while the tool is open: a drag on the arrow changes the value, a
    /// click picks. True when the input was used.
    pub(crate) fn press_pull_input(&mut self, response: &egui::Response, rect: Rect) -> bool {
        if self.solid.press_pull.is_none() {
            return false;
        }
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        if let Some([x, y, d]) = self.solid.press_pull.as_mut().and_then(|t| t.demo.take()) {
            self.press_pull_click(f64::from(x), f64::from(y), w, h);
            if let Some(t) = self.solid.press_pull.as_mut() {
                t.value = format!("{d} mm");
            }
            return true;
        }
        let arrow = self.arrow_on_screen(rect);
        if response.drag_started_by(egui::PointerButton::Primary)
            && let (Some(p), Some((a, b))) = (response.interact_pointer_pos(), arrow)
            && distance_to_segment(p, a, b) <= GRAB_PIXELS
            && let Some(tool) = self.solid.press_pull.as_mut()
        {
            let value = tool.number().unwrap_or(0.0);
            tool.drag = Some(Drag { start: p, value });
            return true;
        }
        let dragging = self.solid.press_pull.as_ref().and_then(|t| t.drag);
        if let Some(drag) = dragging {
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos()
            {
                self.press_pull_drag(rect, drag, p);
            }
            if response.drag_stopped()
                && let Some(t) = self.solid.press_pull.as_mut()
            {
                t.drag = None;
            }
            return true;
        }
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.press_pull_click(x, y, w, h);
            return true;
        }
        false
    }

    /// The pointer at `p` during a drag that started at `drag`: the value follows the
    /// pointer's travel along the arrow on screen, in mm, rounded to 0.1 mm.
    fn press_pull_drag(&mut self, rect: Rect, drag: Drag, p: Pos2) {
        let Some(t) = self
            .solid
            .press_pull
            .as_ref()
            .and_then(|t| t.target.clone())
        else {
            return;
        };
        let (Some(a), Some(b)) = (
            self.pull_project(rect, t.anchor),
            self.pull_project(rect, t.anchor + t.direction),
        ) else {
            return;
        };
        let per_mm = b - a;
        let len2 = per_mm.length_sq();
        if len2 < 1e-6 {
            return;
        }
        let moved = f64::from((p - drag.start).dot(per_mm) / len2);
        // An edge's arrow points out of the solid: pulling it in grows the radius.
        let sign = match t.operation {
            PressPull::Extrude | PressPull::OffsetFace { .. } => 1.0,
            PressPull::Fillet { .. } | PressPull::FilletRadius { .. } => -1.0,
        };
        let value = ((drag.value + sign * moved) * 10.0).round() / 10.0;
        if let Some(tool) = self.solid.press_pull.as_mut() {
            tool.value = format!("{value} mm");
        }
    }

    /// The arrow (and the faces picked for deletion) as overlays.
    pub(crate) fn press_pull_overlays(&self) -> Vec<Overlay> {
        let Some(tool) = self.solid.press_pull.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some((base, dir, _)) = tool.arrow() {
            let len = self.arrow_length();
            let tip = base + dir * len;
            // Two barbs in a plane through the arrow.
            let side = dir
                .cross(chamfera_geom::Vec3::new(0.0, 0.0, 1.0))
                .normalized()
                .or_else(|| {
                    dir.cross(chamfera_geom::Vec3::new(1.0, 0.0, 0.0))
                        .normalized()
                });
            let mut lines = vec![[base, tip]];
            if let Some(s) = side {
                let back = tip + dir * (-len * 0.22);
                lines.push([tip, back + s * (len * 0.12)]);
                lines.push([tip, back + s * (-len * 0.12)]);
            }
            out.push(Overlay {
                style: OverlayStyle::Highlight,
                lines,
                points: vec![base, tip],
                on_top: true,
            });
        }
        out
    }

    /// The value label next to the arrow's tip.
    pub(crate) fn press_pull_labels(&self, ui: &egui::Ui, rect: Rect) {
        let Some(tool) = self.solid.press_pull.as_ref() else {
            return;
        };
        let Some((_, tip)) = self.arrow_on_screen(rect) else {
            return;
        };
        let text = format!("{} {}", tool.value_label(), tool.value.trim());
        let font = egui::FontId::proportional(12.5);
        let galley = ui
            .painter()
            .layout_no_wrap(text, font, color(self.theme.text));
        // To the left of the tip, clear of the dialog on the right.
        let at = tip + Vec2::new(-(galley.size().x * 0.5 + 14.0), -12.0);
        let r = Rect::from_center_size(at, galley.size() + Vec2::new(12.0, 6.0));
        let radius = egui::CornerRadius::same(widgets::R_SMALL);
        ui.painter()
            .rect_filled(r, radius, color(self.theme.panel).gamma_multiply(0.96));
        ui.painter().rect_stroke(
            r,
            radius,
            egui::Stroke::new(1.5, color(self.theme.highlight)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            r.center() - galley.size() * 0.5,
            galley,
            color(self.theme.text),
        );
    }

    /// Keeps the translucent preview in step with the tool; true when it changed.
    pub(crate) fn update_press_pull_preview(&mut self) -> bool {
        let wanted = self
            .solid
            .press_pull
            .as_ref()
            .filter(|t| t.mode != PullMode::Delete)
            .and_then(PressPullTool::params);
        let Some(params) = wanted else {
            let had = !self.session.scene.previews.is_empty()
                && self
                    .solid
                    .press_pull
                    .as_ref()
                    .is_none_or(|t| t.preview_for.is_some());
            if had {
                let _ = self
                    .session
                    .execute("solid.press_pull", &json!({ "clear": true }));
                self.preview_version += 1;
            }
            if let Some(t) = self.solid.press_pull.as_mut() {
                t.preview_for = None;
            }
            return had;
        };
        if self
            .solid
            .press_pull
            .as_ref()
            .is_some_and(|t| t.preview_for.as_ref() == Some(&params))
        {
            return false;
        }
        let mut p = params.clone();
        p["preview"] = json!(true);
        if self.solid.press_pull.as_ref().map(|t| t.mode) == Some(PullMode::Offset)
            && let Some(face) = params.get("face").cloned()
        {
            // Offset Face previews as the Press Pull of the face would.
            p = json!({ "face": face, "distance": params["distance"], "preview": true });
        }
        let result = self.session.execute("solid.press_pull", &p);
        self.preview_version += 1;
        if let Some(t) = self.solid.press_pull.as_mut() {
            t.preview = match result {
                Ok(r) => Ok(r["volume"].as_f64().unwrap_or(0.0)),
                Err(e) => Err(e.to_string()),
            };
            t.preview_for = Some(params);
        }
        true
    }

    /// The tool's dialog.
    pub(crate) fn press_pull_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(tool) = self.solid.press_pull.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let command = tool.command();
        let title = label(command);
        let icon = match tool.mode {
            PullMode::Auto => Icon::PressPull,
            PullMode::Offset => Icon::OffsetFace,
            PullMode::Delete => Icon::DeleteFace,
        };
        let ready = tool.params().is_some();
        egui::Window::new(title)
            .id(egui::Id::unique("press pull dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(160.0)
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
                    egui::Grid::new("press pull grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            if tool.mode == PullMode::Delete {
                                widgets::field_label(ui, look, "Faces");
                                let text = match tool.faces.len() {
                                    0 => "Click a face".to_owned(),
                                    1 => "1 selected".to_owned(),
                                    n => format!("{n} selected"),
                                };
                                ui.label(text)
                                    .on_hover_text("Click a picked face again to keep it");
                                ui.end_row();
                                return;
                            }
                            widgets::field_label(ui, look, "Operation");
                            let (text, colour) = match &tool.target {
                                Some(t) => (t.operation.label(), look.c(|t| t.text)),
                                None => ("Click something", look.c(|t| t.text_muted)),
                            };
                            ui.label(egui::RichText::new(text).color(colour));
                            ui.end_row();
                            widgets::field_label(ui, look, tool.value_label());
                            let edit = ui.add_sized(
                                [180.0, 26.0],
                                egui::TextEdit::singleline(&mut tool.value)
                                    .hint_text("5 mm")
                                    .align(egui::Align2::LEFT_CENTER),
                            );
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                ok = true;
                            }
                            ui.end_row();
                            if tool.target.is_some() {
                                widgets::field_label(ui, look, "Preview");
                                let (text, colour) = match &tool.preview {
                                    Ok(v) => (format!("volume {v:.2} mm³"), look.c(|t| t.text)),
                                    Err(e) if e.is_empty() => {
                                        (String::new(), look.c(|t| t.text_muted))
                                    }
                                    Err(e) => (e.clone(), look.c(|t| t.error)),
                                };
                                ui.add(
                                    egui::Label::new(egui::RichText::new(text).color(colour))
                                        .wrap(),
                                );
                                ui.end_row();
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
        if ok
            && let Some(params) = self
                .solid
                .press_pull
                .as_ref()
                .and_then(PressPullTool::params)
        {
            let params = match (command, params.get("face")) {
                ("solid.offset_face", Some(face)) => {
                    json!({ "faces": [face], "distance": params["distance"] })
                }
                _ => params,
            };
            if let Some(r) = self.run(command, params) {
                let v = r["solids"][0]["volume"].as_f64().unwrap_or(0.0);
                self.message = Some((
                    format!(
                        "{} · volume {v:.2} mm³",
                        r["name"].as_str().unwrap_or(title)
                    ),
                    false,
                ));
                self.solid.press_pull = None;
            }
        }
        if cancel {
            self.solid.press_pull = None;
        }
        if self.update_press_pull_preview() {
            ctx.request_repaint();
        }
    }
}

/// The distance from `p` to the segment `a b`, in points.
fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 1e-6 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> ChamferaApp {
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
        app
    }

    fn screen(app: &ChamferaApp, w: f64, h: f64, p: Point3) -> (f64, f64) {
        let vp = mat_mul(
            &app.session
                .camera
                .projection_matrix(w / h, app.session.scene.bounds().as_ref()),
            &app.session.camera.view_matrix(),
        );
        let n = transform_point(&vp, p).unwrap();
        ((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h)
    }

    #[test]
    fn press_pull_on_the_top_face_previews_and_offsets_it() {
        let mut app = plate();
        let (w, h) = (800.0, 600.0);
        app.run("view.home", json!({ "width": w, "height": h }));
        let (x, y) = screen(&app, w, h, Point3::new(25.0, 15.0, 10.0));
        app.open_press_pull(PullMode::Auto);
        app.press_pull_click(x, y, w, h);
        let tool = app.solid.press_pull.as_ref().unwrap();
        assert_eq!(
            tool.target.as_ref().unwrap().operation,
            PressPull::OffsetFace {
                face: "2:top".into()
            }
        );
        // The arrow points up (the top's outward normal) and the preview is the plate
        // raised by the default 5 mm: 50 x 30 x 15.
        assert!((tool.target.as_ref().unwrap().direction.z - 1.0).abs() < 1e-9);
        assert!(app.update_press_pull_preview());
        let tool = app.solid.press_pull.as_ref().unwrap();
        assert!((tool.preview.clone().unwrap() - 22_500.0).abs() < 1e-6);
        assert_eq!(app.session.scene.previews.len(), 1);
        assert_eq!(app.press_pull_overlays()[0].lines.len(), 3);
        // A drag along the arrow: the value follows (the arrow's screen length per mm
        // maps pixels back to mm), here 3 mm further up.
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32));
        let a = app
            .pull_project(rect, Point3::new(25.0, 15.0, 10.0))
            .unwrap();
        let b = app
            .pull_project(rect, Point3::new(25.0, 15.0, 11.0))
            .unwrap();
        let start = a;
        app.press_pull_drag(rect, Drag { start, value: 5.0 }, start + (b - a) * 3.0);
        assert_eq!(app.solid.press_pull.as_ref().unwrap().value, "8 mm");
        // OK through the command: 50 x 30 x 18.
        let params = app.solid.press_pull.as_ref().unwrap().params().unwrap();
        let r = app.run("solid.press_pull", params).unwrap();
        assert!((r["solids"][0]["volume"].as_f64().unwrap() - 27_000.0).abs() < 1e-6);
        assert!(app.session.scene.previews.is_empty());
    }

    #[test]
    fn delete_face_collects_faces_and_offset_face_takes_only_faces() {
        let mut app = plate();
        let (w, h) = (800.0, 600.0);
        app.run("view.home", json!({ "width": w, "height": h }));
        let (x, y) = screen(&app, w, h, Point3::new(25.0, 15.0, 10.0));
        app.open_press_pull(PullMode::Delete);
        app.press_pull_click(x, y, w, h);
        assert_eq!(app.solid.press_pull.as_ref().unwrap().faces, vec!["2:top"]);
        app.press_pull_click(x, y, w, h);
        assert!(app.solid.press_pull.as_ref().unwrap().faces.is_empty());
        app.open_press_pull(PullMode::Offset);
        app.press_pull_click(x, y, w, h);
        let t = app.solid.press_pull.as_ref().unwrap();
        assert!(matches!(
            t.target.as_ref().unwrap().operation,
            PressPull::OffsetFace { .. }
        ));
    }
}

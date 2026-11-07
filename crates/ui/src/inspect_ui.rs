//! The INSPECT tools: Measure (click two things: distance, closest points, angle, and each
//! one's length, area or radius) and Mass Properties (a table for one solid). Acts only
//! through `inspect.measure_at`, `inspect.measure`, `inspect.mass` and `select.pick`; the
//! numbers are the kernel's exact values, the viewport only draws them.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, color, label};
use chamfera_engine::naming::{Kind, Want};
use chamfera_geom::Point3;
use chamfera_render::{Overlay, OverlayStyle, mat_mul, transform_point};
use egui::{Pos2, Rect, Vec2};
use serde_json::{Value, json};

/// Which INSPECT tool is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectMode {
    Measure,
    Mass,
}

/// The Measure or Mass Properties tool while it is open.
pub struct InspectTool {
    pub mode: InspectMode,
    /// Measure: the picked entities (references from `inspect.measure_at`), one or two.
    pub picks: Vec<Value>,
    /// The last reply of `inspect.measure_at`, `inspect.measure` or `inspect.mass`.
    pub result: Option<Value>,
    /// Mass Properties: the solid (name), and the density as typed.
    pub body: Option<String>,
    pub density: String,
    /// Why the density cannot be used, shown under it (Mass Properties).
    pub error: Option<String>,
}

impl InspectTool {
    pub fn new(mode: InspectMode) -> InspectTool {
        InspectTool {
            mode,
            picks: Vec::new(),
            result: None,
            body: None,
            density: "1".into(),
            error: None,
        }
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

use chamfera_doc::DisplayUnits;

/// A length in the design's unit and precision.
fn mm(u: DisplayUnits, v: f64) -> String {
    u.length(v)
}

fn point(v: &Value) -> Option<Point3> {
    Some(Point3::new(num(&v[0])?, num(&v[1])?, num(&v[2])?))
}

fn xyz(u: DisplayUnits, v: &Value) -> String {
    match point(v) {
        Some(p) => format!(
            "{}, {}, {}",
            u.length_number(p.x),
            u.length_number(p.y),
            u.length_number(p.z)
        ),
        None => "-".into(),
    }
}

/// The rows (label, value) describing one entity from an `inspect.*` reply.
fn entity_rows(u: DisplayUnits, e: &Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let kind = e["kind"].as_str().unwrap_or("?");
    let what = match kind {
        "face" => format!("Face ({})", e["surface"].as_str().unwrap_or("?")),
        "edge" => format!("Edge ({})", e["curve"].as_str().unwrap_or("?")),
        "vertex" => "Vertex".into(),
        "point" => "Point".into(),
        "body" => "Body".into(),
        _ => kind.to_owned(),
    };
    let name = e["ref"]["face"]
        .as_str()
        .or(e["ref"]["edge"].as_str())
        .or(e["ref"]["vertex"].as_str())
        .or(e["solid"].as_str())
        .unwrap_or("");
    rows.push((what, name.to_owned()));
    if let Some(l) = num(&e["length"]) {
        rows.push(("Length".into(), mm(u, l)));
    }
    if let Some(a) = num(&e["area"]) {
        rows.push(("Area".into(), u.area(a)));
    }
    if let Some(r) = num(&e["radius"]) {
        rows.push(("Radius".into(), mm(u, r)));
        rows.push(("Diameter".into(), mm(u, 2.0 * r)));
    }
    if let Some(v) = num(&e["volume"]) {
        rows.push(("Volume".into(), u.volume(v)));
    }
    if e["point"].is_array() {
        rows.push(("Position".into(), xyz(u, &e["point"])));
    }
    rows
}

/// The rows of a Measure result: each entity, then distance and angle between them.
pub(crate) fn measure_rows(u: DisplayUnits, r: &Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let two = r.get("b").is_some_and(|b| !b.is_null());
    for (key, tag) in [("a", "1"), ("b", "2")] {
        if r[key].is_object() {
            // With two entities the first row of each (what it is) says which one.
            for (i, (k, v)) in entity_rows(u, &r[key]).into_iter().enumerate() {
                let k = if two && i == 0 {
                    format!("{tag}. {k}")
                } else {
                    k
                };
                rows.push((k, v));
            }
        }
    }
    if let Some(d) = num(&r["distance"]) {
        rows.push(("Distance".into(), mm(u, d)));
        if let (Some(dx), Some(dy), Some(dz)) = (
            num(&r["delta"][0]),
            num(&r["delta"][1]),
            num(&r["delta"][2]),
        ) {
            rows.push((
                "ΔX ΔY ΔZ".into(),
                format!(
                    "{}, {}, {}",
                    u.length_number(dx),
                    u.length_number(dy),
                    u.length_number(dz)
                ),
            ));
        }
    }
    if let Some(a) = num(&r["angle_degrees"]) {
        rows.push(("Angle".into(), u.angle(a.to_radians())));
    }
    rows
}

/// The rows of a Mass Properties result.
/// `v` with six significant digits, without trailing zeros or an exponent unless
/// it is very large or small: `9.8125`, `1234567`, `0.015209`, `1.2346e-9`.
fn significant(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v.is_finite() {
            "0".into()
        } else {
            "?".into()
        };
    }
    let mag = v.abs().log10().floor();
    if !(-4.0..9.0).contains(&mag) {
        return format!("{v:.4e}");
    }
    let decimals = (5.0 - mag).max(0.0) as usize;
    let text = format!("{v:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

pub(crate) fn mass_rows(u: DisplayUnits, r: &Value) -> Vec<(String, String)> {
    // Moments in mass x (display length)²; round-off below a billionth of the largest
    // reads 0.
    let mm2 = u.length.mm() * u.length.mm();
    let all: Vec<f64> = (0..3)
        .flat_map(|a| (0..3).map(move |b| (a, b)))
        .filter_map(|(a, b)| num(&r["inertia"][a][b]))
        .collect();
    let largest = all.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
    let unit = format!("mass·{}²", u.length.symbol());
    let f = |v: &Value| {
        num(v).map_or_else(
            || "-".to_owned(),
            |x| {
                let x = if x.abs() <= largest * 1e-9 { 0.0 } else { x };
                format!("{} {unit}", significant(x / mm2))
            },
        )
    };
    let mut rows = vec![
        (
            "Solid".to_owned(),
            r["solid"].as_str().unwrap_or("").to_owned(),
        ),
        (
            "Volume".into(),
            num(&r["volume"]).map_or("-".into(), |v| u.volume(v)),
        ),
        (
            "Area".into(),
            num(&r["area"]).map_or("-".into(), |v| u.area(v)),
        ),
        (
            "Mass".into(),
            num(&r["mass"]).map_or("-".into(), |v| format!("{v:.4}")),
        ),
        ("Centre of mass".into(), xyz(u, &r["centre_of_mass"])),
    ];
    let i = &r["inertia"];
    for (name, a, b) in [
        ("Ixx", 0, 0),
        ("Iyy", 1, 1),
        ("Izz", 2, 2),
        ("Ixy", 0, 1),
        ("Iyz", 1, 2),
        ("Ixz", 0, 2),
    ] {
        rows.push((name.to_owned(), f(&i[a][b])));
    }
    for k in 0..3 {
        rows.push((
            format!("Principal I{}", k + 1),
            f(&r["principal_moments"][k]),
        ));
    }
    rows
}

impl ChamferaApp {
    /// Opens (or closes, when it is open) an INSPECT tool.
    pub(crate) fn open_inspect(&mut self, mode: InspectMode) {
        if self.solid.inspect.as_ref().is_some_and(|t| t.mode == mode) {
            self.solid.inspect = None;
            return;
        }
        let mut tool = InspectTool::new(mode);
        match mode {
            InspectMode::Measure => {
                self.message = Some(("Click a vertex, an edge or a face to measure".into(), false));
            }
            InspectMode::Mass => {
                // The selected solid, else the first one.
                let selected = self.session.scene.selection.and_then(|h| {
                    self.session
                        .solids()
                        .iter()
                        .find(|s| s.scene_id == Some(h.body))
                });
                tool.body = selected
                    .or(self.session.solids().first())
                    .map(|s| s.name.clone());
                self.message = Some(("Click a solid to see its mass properties".into(), false));
            }
        }
        self.solid.inspect = Some(tool);
        self.refresh_mass();
    }

    /// Runs `inspect.mass` for the tool's solid and density.
    fn refresh_mass(&mut self) {
        let Some(tool) = self.solid.inspect.as_ref() else {
            return;
        };
        if tool.mode != InspectMode::Mass {
            return;
        }
        let Some(body) = tool.body.clone() else {
            return;
        };
        let typed = tool.density.trim().to_owned();
        let density = typed
            .parse::<f64>()
            .ok()
            .filter(|d| d.is_finite() && *d > 0.0);
        let Some(density) = density else {
            // Steel is 7.85 g/cm³ = 0.00785 g/mm³.
            let e = "Density must be a number above 0:\nmass per mm³ (steel: 0.00785 g)".to_owned();
            self.message = Some((e.clone(), true));
            if let Some(tool) = self.solid.inspect.as_mut() {
                tool.result = None;
                tool.error = Some(e);
            }
            return;
        };
        let r = self.run("inspect.mass", json!({ "body": body, "density": density }));
        if let Some(tool) = self.solid.inspect.as_mut() {
            tool.result = r;
            tool.error = None;
        }
    }

    /// A click in the viewport while an INSPECT tool is open.
    pub(crate) fn inspect_click(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(tool) = self.solid.inspect.as_ref() else {
            return;
        };
        let pixel = json!({ "x": x, "y": y, "width": w, "height": h });
        match tool.mode {
            InspectMode::Measure => {
                let mut params = pixel;
                // A second pick measures from the first; a third starts again.
                let from = (tool.picks.len() == 1).then(|| tool.picks[0].clone());
                if let Some(f) = &from {
                    params["from"] = f.clone();
                }
                let Some(r) = self.run("inspect.measure_at", params) else {
                    return;
                };
                if r["entity"].is_null() {
                    return;
                }
                let Some(tool) = self.solid.inspect.as_mut() else {
                    return;
                };
                if from.is_none() {
                    tool.picks.clear();
                }
                tool.picks.push(r["entity"].clone());
                tool.result = Some(r);
                self.message = Some((
                    if tool.picks.len() == 1 {
                        "Click a second entity to measure between them".into()
                    } else {
                        "Click again to start a new measure".into()
                    },
                    false,
                ));
            }
            InspectMode::Mass => {
                let hit = self.run("select.pick", pixel);
                let name = hit.and_then(|h| {
                    let id = h["body"].as_u64()?;
                    self.session
                        .solids()
                        .iter()
                        .find(|s| s.scene_id.map(u64::from) == Some(id))
                        .map(|s| s.name.clone())
                });
                if let (Some(name), Some(tool)) = (name, self.solid.inspect.as_mut()) {
                    tool.body = Some(name);
                }
                self.refresh_mass();
            }
        }
    }

    /// The Measure overlays: the dimension line between the closest points, its end
    /// points and the picked edges, in the theme's `measure` colour. Replaces the last
    /// frame's.
    pub(crate) fn inspect_overlays(&mut self) {
        self.session
            .scene
            .overlays
            .retain(|o| o.style != OverlayStyle::Measure);
        let Some(tool) = self.solid.inspect.as_ref() else {
            return;
        };
        if tool.mode != InspectMode::Measure {
            return;
        }
        let mut lines = Vec::new();
        let mut points = Vec::new();
        if let Some(r) = &tool.result
            && let (Some(a), Some(b)) = (point(&r["on_a"]), point(&r["on_b"]))
        {
            lines.push([a, b]);
            points.extend([a, b]);
        }
        for pick in &tool.picks {
            if let Some(name) = pick["edge"].as_str() {
                // The picked name through the engine's resolver (`2:top 0#2` is one piece).
                let found: Vec<(usize, _)> = self
                    .session
                    .resolve_reference(None, Kind::Edge, name, Want::All)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|f| Some((f.body, f.edge()?)))
                    .collect();
                for (si, s) in self.session.solids().iter().enumerate() {
                    if pick["solid"].as_str() != Some(s.name.as_str()) {
                        continue;
                    }
                    for (e, line) in &s.edges {
                        if found.contains(&(si, *e)) {
                            lines.extend(line.windows(2).filter_map(|p| match p {
                                [a, b] => Some([*a, *b]),
                                _ => None,
                            }));
                        }
                    }
                }
            }
        }
        if let Some(r) = &tool.result {
            for key in ["a", "b"] {
                if let Some(p) = point(&r[key]["point"]) {
                    points.push(p);
                }
            }
        }
        if lines.is_empty() && points.is_empty() {
            return;
        }
        self.session.scene.overlays.push(Overlay {
            style: OverlayStyle::Measure,
            lines,
            points,
            on_top: true,
        });
    }

    /// Where a model point is in the viewport `rect`.
    fn inspect_project(&self, rect: Rect, p: Point3) -> Option<Pos2> {
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

    /// The distance (and angle) label at the middle of the dimension line.
    pub(crate) fn inspect_labels(&self, ui: &egui::Ui, rect: Rect) {
        let Some(r) = self
            .solid
            .inspect
            .as_ref()
            .filter(|t| t.mode == InspectMode::Measure)
            .and_then(|t| t.result.as_ref())
        else {
            return;
        };
        let (Some(a), Some(b), Some(d)) =
            (point(&r["on_a"]), point(&r["on_b"]), num(&r["distance"]))
        else {
            return;
        };
        let mid = Point3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5);
        let Some(pos) = self.inspect_project(rect, mid) else {
            return;
        };
        let u = self.session.doc.units;
        let mut text = mm(u, d);
        if let Some(t) = num(&r["angle_degrees"]) {
            text = format!("{text}   {}", u.angle(t.to_radians()));
        }
        let font = egui::FontId::proportional(12.5);
        let galley = ui
            .painter()
            .layout_no_wrap(text, font, color(self.theme.text));
        let at = pos + Vec2::new(0.0, -16.0);
        let r = Rect::from_center_size(at, galley.size() + Vec2::new(12.0, 6.0));
        let radius = egui::CornerRadius::same(widgets::R_SMALL);
        ui.painter()
            .rect_filled(r, radius, color(self.theme.panel).gamma_multiply(0.96));
        ui.painter().rect_stroke(
            r,
            radius,
            egui::Stroke::new(1.5, color(self.theme.measure)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            r.center() - galley.size() * 0.5,
            galley,
            color(self.theme.text),
        );
    }

    /// The results panel of the open INSPECT tool.
    pub(crate) fn inspect_dialog(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 330.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let units = self.session.doc.units;
        let Some(tool) = self.solid.inspect.as_mut() else {
            return;
        };
        let mut close = false;
        let mut recompute = false;
        let (icon, title) = match tool.mode {
            InspectMode::Measure => (Icon::Measure, label("inspect.measure")),
            InspectMode::Mass => (Icon::Mass, label("inspect.mass")),
        };
        let rows = match (&tool.result, tool.mode) {
            (Some(r), InspectMode::Measure) => measure_rows(units, r),
            (Some(r), InspectMode::Mass) => mass_rows(units, r),
            (None, _) => Vec::new(),
        };
        egui::Window::new(title)
            .id(egui::Id::unique("inspect dialog"))
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
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    if rows.is_empty() {
                        let hint = match tool.mode {
                            InspectMode::Measure => "Click a vertex, an edge or a face",
                            InspectMode::Mass => "Click a solid",
                        };
                        ui.label(egui::RichText::new(hint).color(look.c(|t| t.text_muted)));
                    }
                    egui::Grid::new("inspect grid")
                        .num_columns(2)
                        .spacing([widgets::S3, 0.0])
                        .show(ui, |ui| {
                            if tool.mode == InspectMode::Mass {
                                widgets::field_label(ui, look, "Density");
                                let edit = ui
                                    .add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(&mut tool.density)
                                            .hint_text("1"),
                                    )
                                    .on_hover_text("Mass per mm³ (1 gives mass = volume)");
                                if edit.lost_focus() || edit.changed() {
                                    recompute = true;
                                }
                                ui.end_row();
                                if let Some(e) = &tool.error {
                                    ui.label("");
                                    ui.label(egui::RichText::new(e).color(look.c(|t| t.error)));
                                    ui.end_row();
                                }
                            }
                            for (k, v) in &rows {
                                widgets::fixed_text(
                                    ui,
                                    112.0,
                                    egui::RichText::new(k).color(look.c(|t| t.text_muted)),
                                    egui::Align::Min,
                                );
                                widgets::fixed_text(
                                    ui,
                                    186.0,
                                    egui::RichText::new(v).color(look.c(|t| t.text)),
                                    egui::Align::Min,
                                );
                                ui.end_row();
                            }
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            close = true;
        }
        if close {
            self.solid.inspect = None;
            self.message = None;
        } else if recompute {
            self.refresh_mass();
        }
    }

    /// A start-up script that ends in `inspect.measure` or `inspect.mass` opens that tool
    /// with the script's result, so a screenshot shows it.
    pub(crate) fn inspect_from_script(&mut self, steps: &[Value], replies: &[Value]) {
        for (step, reply) in steps.iter().zip(replies).rev() {
            match step["command"].as_str() {
                Some("inspect.measure") => {
                    let mut tool = InspectTool::new(InspectMode::Measure);
                    for k in ["a", "b"] {
                        if reply[k]["ref"].is_object() {
                            tool.picks.push(reply[k]["ref"].clone());
                        }
                    }
                    tool.result = Some(reply.clone());
                    self.solid.inspect = Some(tool);
                    return;
                }
                Some("inspect.mass") => {
                    let mut tool = InspectTool::new(InspectMode::Mass);
                    tool.body = reply["solid"].as_str().map(str::to_owned);
                    tool.density = num(&reply["density"]).map_or("1".into(), |d| d.to_string());
                    tool.result = Some(reply.clone());
                    self.solid.inspect = Some(tool);
                    return;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(app: &mut ChamferaApp, x0: f64, x1: f64) {
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": x0, "y1": 0, "x2": x1, "y2": 30 }),
        );
        let sketch = app.session.active_sketch.unwrap_or(1);
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": sketch, "profiles": [{ "x": (x0 + x1) / 2.0, "y": 15 }], "distance": 10 }),
        );
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
    fn two_clicks_on_box_faces_measure_the_gap_and_draw_the_dimension_line() {
        // Boxes [0,50] and [60,110] in x, 30 deep and 10 high: their tops are 10 apart
        // (between the edges x = 50 and x = 60) and parallel (angle 0).
        let mut app = ChamferaApp::headless();
        boxed(&mut app, 0.0, 50.0);
        boxed(&mut app, 60.0, 110.0);
        let (w, h) = (800.0, 600.0);
        app.run("view.home", json!({ "width": w, "height": h }));
        app.open_inspect(InspectMode::Measure);
        let (x, y) = screen(&app, w, h, Point3::new(25.0, 15.0, 10.0));
        app.inspect_click(x, y, w, h);
        let tool = app.solid.inspect.as_ref().unwrap();
        assert_eq!(tool.picks.len(), 1);
        let rows = measure_rows(app.session.doc.units, tool.result.as_ref().unwrap());
        assert!(
            rows.iter().any(|(k, v)| k == "Area" && v == "1500.00 mm²"),
            "{rows:?}"
        );
        let (x, y) = screen(&app, w, h, Point3::new(85.0, 15.0, 10.0));
        app.inspect_click(x, y, w, h);
        let r = app.solid.inspect.as_ref().unwrap().result.clone().unwrap();
        assert!((r["distance"].as_f64().unwrap() - 10.0).abs() < 1e-9, "{r}");
        assert!(r["angle_degrees"].as_f64().unwrap().abs() < 1e-9);
        let rows = measure_rows(app.session.doc.units, &r);
        assert!(rows.iter().any(|(k, v)| k == "Distance" && v == "10.00 mm"));
        app.inspect_overlays();
        let o = app
            .session
            .scene
            .overlays
            .iter()
            .find(|o| o.style == OverlayStyle::Measure)
            .unwrap();
        let [a, b] = o.lines[0];
        assert!((a.distance(b) - 10.0).abs() < 1e-9);
        // Drawn once, not once per frame.
        app.inspect_overlays();
        let n = app
            .session
            .scene
            .overlays
            .iter()
            .filter(|o| o.style == OverlayStyle::Measure)
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn mass_properties_open_on_the_solid_and_follow_the_density() {
        let mut app = ChamferaApp::headless();
        boxed(&mut app, 0.0, 50.0);
        app.open_inspect(InspectMode::Mass);
        let r = app.solid.inspect.as_ref().unwrap().result.clone().unwrap();
        // 50 x 30 x 10: mass 15000 at density 1, centre (25, 15, 5).
        assert!((r["mass"].as_f64().unwrap() - 15_000.0).abs() < 1e-6);
        let rows = mass_rows(app.session.doc.units, &r);
        assert!(
            rows.iter()
                .any(|(k, v)| k == "Centre of mass" && v == "25.00, 15.00, 5.00")
        );
        app.solid.inspect.as_mut().unwrap().density = "2.5".into();
        app.refresh_mass();
        let r = app.solid.inspect.as_ref().unwrap().result.clone().unwrap();
        assert!((r["mass"].as_f64().unwrap() - 37_500.0).abs() < 1e-6);
        // Clicking the button again closes it.
        app.open_inspect(InspectMode::Mass);
        assert!(app.solid.inspect.is_none());
    }

    /// Inertia reads as a plain number with its unit, mass x length squared in the
    /// design's length unit, and round-off far below the largest moment reads 0 (QA user
    /// guide pass: "9.8125e0" beside "2.57e-16", with no units). In inches the same
    /// 9.8125 mass·mm² is 9.8125 / 645.16 = 0.01520940... mass·in², six digits 0.0152094.
    #[test]
    fn inertia_rows_are_plain_numbers_with_units_and_no_noise() {
        let r = json!({
            "inertia": [[9.8125, 2.57e-16, 0.0], [2.57e-16, 1234567.0, 0.0], [0.0, 0.0, 0.5]],
            "principal_moments": [0.5, 9.8125, 1234567.0],
        });
        let rows = mass_rows(DisplayUnits::default(), &r);
        let get = |rows: &[(String, String)], k: &str| {
            rows.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get(&rows, "Ixx"), "9.8125 mass·mm²");
        assert_eq!(get(&rows, "Ixy"), "0 mass·mm²");
        assert_eq!(get(&rows, "Iyy"), "1234567 mass·mm²");
        let inch = DisplayUnits {
            length: chamfera_doc::LengthUnit::In,
            ..DisplayUnits::default()
        };
        assert_eq!(get(&mass_rows(inch, &r), "Ixx"), "0.0152094 mass·in²");
    }

    /// A density that is not a number ("7.85 g/cm3", "abc", "-2", empty) is refused
    /// with an error saying what is allowed; it is never taken as 1 (mass 15000 for the
    /// 50 x 30 x 10 box). A good density afterwards clears the error.
    #[test]
    fn mass_properties_refuse_an_unreadable_density() {
        let mut app = ChamferaApp::headless();
        boxed(&mut app, 0.0, 50.0);
        app.open_inspect(InspectMode::Mass);
        for bad in ["7.85 g/cm3", "abc", "-2", "0", ""] {
            app.solid.inspect.as_mut().unwrap().density = bad.into();
            app.refresh_mass();
            let tool = app.solid.inspect.as_ref().unwrap();
            assert!(tool.result.is_none(), "{bad}: {:?}", tool.result);
            let e = tool.error.clone().unwrap_or_default();
            assert!(e.contains("Density"), "{bad}: {e}");
        }
        app.solid.inspect.as_mut().unwrap().density = "2".into();
        app.refresh_mass();
        let tool = app.solid.inspect.as_ref().unwrap();
        assert!(tool.error.is_none());
        let mass = tool.result.as_ref().and_then(|r| r["mass"].as_f64());
        assert!(
            mass.is_some_and(|m| (m - 30_000.0).abs() < 1e-6),
            "{mass:?}"
        );
    }
}

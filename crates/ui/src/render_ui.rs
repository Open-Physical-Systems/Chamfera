//! The RENDER workspace: the material library (left panel; click or drag a material onto
//! a body), the environment and camera windows, the realistic view toggle and the Render
//! window, where a path-traced picture of the current view refines and can be saved as a
//! PNG. Every change runs a command (`body.appearance`, `render.environment`,
//! `view.style`, `view.standard`, `view.projection`, `render.start`, `render.save`).

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::widgets::{self, Look, S2, S3, T_SMALL};
use chamfera_engine::pick_filter::{PickFilter, Picked};
use chamfera_raytrace::Render;
use chamfera_render::material::{Category, LIBRARY};
use chamfera_render::{EnvironmentPreset, VisualStyle};
use egui::{Color32, Rect, vec2};
use serde_json::json;
use std::sync::Arc;

/// Default render size and sample count. The browser renders on the UI thread, so it
/// starts smaller.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT: (u32, u32, u32) = (1280, 800, 256);
#[cfg(target_arch = "wasm32")]
const DEFAULT: (u32, u32, u32) = (480, 300, 32);
/// Widest the Render window shows the picture (points); larger renders are scaled down.
const PREVIEW_WIDTH: f32 = 560.0;

/// The material dragged from the library, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DraggedMaterial(&'static str);

pub(crate) struct RenderWindow {
    pub width: String,
    pub height: String,
    pub samples: String,
    pub job: Option<Arc<Render>>,
    texture: Option<egui::TextureHandle>,
    /// Samples the texture shows.
    shown: u32,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<std::thread::JoinHandle<()>>,
}

impl RenderWindow {
    fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(w) = self.worker.take() {
            // The worker stops after the pass under way.
            let _ = w.join();
        }
    }
}

impl Drop for RenderWindow {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
pub(crate) struct RenderUi {
    /// The RENDER workspace is showing.
    pub active: bool,
    /// The left panel shows the material library (the Materials tool).
    pub library: bool,
    pub environment_open: bool,
    pub camera_open: bool,
    /// The style to go back to when the realistic view is turned off.
    style_before: Option<VisualStyle>,
    pub window: Option<RenderWindow>,
}

impl RenderUi {
    /// A render is running, or its picture is not yet on screen.
    pub(crate) fn busy(&self) -> bool {
        self.window.as_ref().is_some_and(|w| {
            w.job
                .as_ref()
                .is_some_and(|j| !j.done() || w.shown < j.samples())
        })
    }
}

/// The library material's swatch colour: material data, not a theme colour.
fn swatch(id: &str) -> Color32 {
    chamfera_render::material::by_id(id).map_or(Color32::GRAY, |m| {
        Color32::from_rgb(m.colour.0, m.colour.1, m.colour.2)
    })
}

fn whole(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok()
}

impl ChamferaApp {
    /// Enters or leaves the RENDER workspace: the viewport turns realistic while in it.
    pub(crate) fn set_render_workspace(&mut self, on: bool) {
        self.render.active = on;
        if on {
            self.render.library = true;
            if self.session.scene.display.style != VisualStyle::Realistic {
                self.toggle_realistic();
            }
        } else {
            self.render.environment_open = false;
            self.render.camera_open = false;
            self.render.window = None;
            if self.session.scene.display.style == VisualStyle::Realistic {
                self.toggle_realistic();
            }
        }
    }

    /// Realistic View: turns the realistic style on, or back to the style before it.
    pub(crate) fn toggle_realistic(&mut self) {
        let now = self.session.scene.display.style;
        let next = if now == VisualStyle::Realistic {
            self.render
                .style_before
                .take()
                .unwrap_or(VisualStyle::ShadedEdges)
        } else {
            self.render.style_before = Some(now);
            VisualStyle::Realistic
        };
        self.run("view.style", json!({ "style": next.id() }));
    }

    /// Gives a library material to bodies (by name: each change rebuilds the model and
    /// gives its bodies new ids).
    fn assign_material(&mut self, id: &str, bodies: &[u32]) {
        let names: Vec<String> = bodies
            .iter()
            .filter_map(|b| self.session.scene.body(*b).map(|b| b.name.clone()))
            .collect();
        if names.is_empty() {
            self.message = Some((
                "Select a body first, or drag the material onto one".into(),
                true,
            ));
            return;
        }
        for name in names {
            self.run("body.appearance", json!({ "body": name, "preset": id }));
        }
    }

    /// The left panel in RENDER: the material library by category.
    pub(crate) fn render_library(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            widgets::caption(ui, look, "MATERIALS");
        });
        ui.label(
            egui::RichText::new(
                "Click to give a material to the selected bodies, or drag it onto a body.",
            )
            .size(T_SMALL)
            .color(look.c(|t| t.text_muted)),
        );
        ui.add_space(S2);
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for c in Category::ALL {
                widgets::caption(ui, look, &c.label().to_uppercase());
                for m in LIBRARY.iter().filter(|m| m.category == c) {
                    let r = ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!("      {}", m.label)).size(T_SMALL),
                        )
                        .fill(look.c(|t| t.panel_alt))
                        .min_size(vec2(ui.available_width(), 22.0))
                        .sense(egui::Sense::click_and_drag()),
                    );
                    let dot = Rect::from_center_size(
                        r.rect.left_center() + vec2(12.0, 0.0),
                        vec2(12.0, 12.0),
                    );
                    ui.painter().circle_filled(dot.center(), 6.0, swatch(m.id));
                    let tip = format!(
                        "{}\nMetallic {:.0}%, roughness {:.2}{}",
                        m.label,
                        m.metallic * 100.0,
                        m.roughness,
                        if m.transmission > 0.5 {
                            ", transparent"
                        } else {
                            ""
                        }
                    );
                    if r.drag_started() {
                        egui::DragAndDrop::set_payload(ui.ctx(), DraggedMaterial(m.id));
                    }
                    if r.on_hover_text(tip).clicked() {
                        clicked = Some(m.id);
                    }
                }
                ui.add_space(S2);
            }
        });
        if let Some(id) = clicked {
            let selected = self.session.scene.selected_bodies();
            self.assign_material(id, &selected);
        }
    }

    /// A material dropped on the viewport goes to the body under the pointer.
    pub(crate) fn render_drop(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        let Some(payload) = egui::DragAndDrop::take_payload::<DraggedMaterial>(ctx) else {
            return;
        };
        let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let r = self.viewport_rect;
        if !r.contains(at) {
            return;
        }
        let p = at - r.min;
        let filter = PickFilter {
            bodies: true,
            faces: true,
            ..PickFilter::NONE
        };
        let picked = self.session.pick_filtered(
            f64::from(p.x),
            f64::from(p.y),
            f64::from(r.width()),
            f64::from(r.height()),
            &filter,
        );
        match picked {
            Picked::Body(hit) | Picked::Face { hit, .. } => {
                self.assign_material(payload.0, &[hit.body]);
            }
            _ => self.message = Some(("Drop the material onto a body".into(), true)),
        }
    }

    /// Opens the Render window with sizes from the viewport's shape.
    pub(crate) fn open_render_window(&mut self) {
        if self.render.window.is_some() {
            return;
        }
        let (w, _, samples) = DEFAULT;
        let aspect = (self.viewport.y / self.viewport.x.max(1.0)).clamp(0.2, 5.0);
        let h = ((w as f32 * aspect).round() as u32).clamp(1, chamfera_raytrace::MAX_SIZE);
        self.render.window = Some(RenderWindow {
            width: w.to_string(),
            height: h.to_string(),
            samples: samples.to_string(),
            job: None,
            texture: None,
            shown: 0,
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
        });
    }

    pub(crate) fn start_render(&mut self) {
        let Some(win) = &self.render.window else {
            return;
        };
        let (Some(w), Some(h), Some(s)) =
            (whole(&win.width), whole(&win.height), whole(&win.samples))
        else {
            self.message = Some((
                "Width, height and samples must be whole numbers".into(),
                true,
            ));
            return;
        };
        if let Some(win) = &mut self.render.window {
            win.stop();
        }
        if self
            .run(
                "render.start",
                json!({ "width": w, "height": h, "samples": s }),
            )
            .is_none()
        {
            return;
        }
        let Some(job) = self.session.render_job.clone() else {
            return;
        };
        if let Some(win) = &mut self.render.window {
            win.shown = 0;
            win.texture = None;
            #[cfg(not(target_arch = "wasm32"))]
            {
                let worker_job = job.clone();
                win.worker = std::thread::Builder::new()
                    .name("render".into())
                    .spawn(move || {
                        while !worker_job.done() {
                            worker_job.step();
                        }
                    })
                    .ok();
            }
            win.job = Some(job);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_render(&mut self) {
        self.export("PNG image", &["png"], "render.save");
    }

    #[cfg(target_arch = "wasm32")]
    fn save_render(&mut self) {
        let Some(v) = self.run("render.save_bytes", json!({})) else {
            return;
        };
        let name = format!("{}.png", self.design_name());
        match crate::web::download(&name, "image/png", v["data"].as_str().unwrap_or("")) {
            Ok(()) => self.message = Some((format!("Downloaded {name}"), false)),
            Err(e) => self.message = Some((format!("Save Render: {e}"), true)),
        }
    }

    /// The Environment, Camera and Render windows, and materials dropped on the view.
    pub(crate) fn render_windows(&mut self, ctx: &egui::Context) {
        self.render_drop(ctx);
        self.environment_window(ctx);
        self.camera_window(ctx);
        self.render_window(ctx);
    }

    fn environment_window(&mut self, ctx: &egui::Context) {
        if !self.render.environment_open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let current = self.session.scene.display.environment;
        let mut pick = None;
        let mut close = false;
        egui::Window::new("Environment")
            .id(egui::Id::unique("render environment"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(self.tool_dialog_pos(260.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(260.0);
                if widgets::dialog_title(ui, look, Icon::RenderEnvironment, "Environment") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    for e in EnvironmentPreset::ALL {
                        if ui
                            .radio(e == current, e.label())
                            .on_hover_text(e.tip())
                            .clicked()
                        {
                            pick = Some(e);
                        }
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(e) = pick {
            self.run("render.environment", json!({ "preset": e.id() }));
        }
        if close {
            self.render.environment_open = false;
        }
    }

    fn camera_window(&mut self, ctx: &egui::Context) {
        if !self.render.camera_open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let ortho = self.session.camera.projection == chamfera_render::Projection::Orthographic;
        let mut view = None;
        let mut projection = None;
        let mut close = false;
        egui::Window::new("Camera")
            .id(egui::Id::unique("render camera"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(self.tool_dialog_pos(260.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(260.0);
                if widgets::dialog_title(ui, look, Icon::RenderCamera, "Camera") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Renders use the current view. Orbit, pan and zoom as usual, or pick a view:")
                            .size(T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for (id, name) in [("iso", "Iso"), ("front", "Front"), ("right", "Right"), ("top", "Top")] {
                            if widgets::secondary_button(ui, look, name).clicked() {
                                view = Some(id);
                            }
                        }
                    });
                    ui.add_space(S2);
                    if ui.radio(!ortho, "Perspective").clicked() {
                        projection = Some("perspective");
                    }
                    if ui.radio(ortho, "Orthographic").clicked() {
                        projection = Some("orthographic");
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(v) = view {
            self.run("view.standard", json!({ "view": v }));
        }
        if let Some(m) = projection {
            self.run("view.projection", json!({ "mode": m }));
        }
        if close {
            self.render.camera_open = false;
        }
    }

    fn render_window(&mut self, ctx: &egui::Context) {
        if self.render.window.is_none() {
            return;
        }
        // In the browser there is no worker: one pass per frame.
        #[cfg(target_arch = "wasm32")]
        if let Some(job) = self.render.window.as_ref().and_then(|w| w.job.clone())
            && !job.done()
        {
            job.step();
        }
        // Bring the picture up to date.
        if let Some(win) = &mut self.render.window
            && let Some(job) = &win.job
        {
            let n = job.samples();
            if n != win.shown && n > 0 {
                let st = job.settings();
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [st.width as usize, st.height as usize],
                    &job.rgba(),
                );
                match &mut win.texture {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        win.texture =
                            Some(ctx.load_texture("render", image, egui::TextureOptions::LINEAR));
                    }
                }
                win.shown = n;
            }
            if !job.done() {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let mut close = false;
        let mut start = false;
        let mut stop = false;
        let mut save = false;
        let Some(win) = &mut self.render.window else {
            return;
        };
        let running = win.job.as_ref().is_some_and(|j| !j.done());
        let progress = win
            .job
            .as_ref()
            .map(|j| (j.samples(), j.settings().samples, j.is_cancelled()));
        egui::Window::new("Render")
            .id(egui::Id::unique("render window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(self.viewport_rect.min + vec2(S3 * 4.0, S3 * 4.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(PREVIEW_WIDTH);
                if widgets::dialog_title(ui, look, Icon::RenderImage, "Render") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        ui.horizontal(|ui| {
                            widgets::field_label(ui, look, "Size");
                            ui.add(egui::TextEdit::singleline(&mut win.width).desired_width(56.0));
                            ui.label("×");
                            ui.add(egui::TextEdit::singleline(&mut win.height).desired_width(56.0));
                            ui.label("px");
                            ui.add_space(S3);
                            widgets::field_label(ui, look, "Samples");
                            ui.add(egui::TextEdit::singleline(&mut win.samples).desired_width(56.0));
                        });
                    });
                    ui.add_space(S2);
                    match (&win.texture, &win.job) {
                        (Some(t), Some(job)) => {
                            let st = job.settings();
                            let k = (PREVIEW_WIDTH / st.width as f32).min(1.0);
                            let size = vec2(st.width as f32 * k, st.height as f32 * k);
                            ui.image((t.id(), size));
                        }
                        _ => {
                            ui.label(
                                egui::RichText::new("Render path-traces the current view with the bodies' materials and the environment. The picture refines as samples add up.")
                                    .size(T_SMALL)
                                    .color(look.c(|t| t.text_muted)),
                            );
                        }
                    }
                    if let Some((done, of, cancelled)) = progress {
                        let state = if done >= of {
                            "done"
                        } else if cancelled {
                            "stopped"
                        } else {
                            "rendering"
                        };
                        ui.label(
                            egui::RichText::new(format!("{done} of {of} samples, {state}"))
                                .size(T_SMALL)
                                .color(look.c(|t| t.text_muted)),
                        );
                        if of > 0 {
                            ui.add(egui::ProgressBar::new(done as f32 / of as f32).desired_height(6.0));
                        }
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if running {
                        if widgets::primary_button(ui, look, "Stop", true).clicked() {
                            stop = true;
                        }
                    } else if widgets::primary_button(ui, look, "Render", true).clicked() {
                        start = true;
                    }
                    let has_picture = progress.is_some_and(|(done, _, _)| done > 0);
                    if ui
                        .add_enabled_ui(has_picture, |ui| widgets::secondary_button(ui, look, "Save PNG…").clicked())
                        .inner
                    {
                        save = true;
                    }
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        close = true;
                    }
                });
            });
        if stop && let Some(win) = &mut self.render.window {
            win.stop();
        }
        if start {
            self.start_render();
        }
        if save {
            self.save_render();
        }
        if close {
            self.render.window = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ribbon::Workspace;

    #[test]
    fn the_render_workspace_turns_the_view_realistic_and_back() {
        let mut app = ChamferaApp::headless();
        app.run("body.sample", json!({}));
        assert_eq!(app.session.scene.display.style, VisualStyle::ShadedEdges);
        app.set_workspace(Workspace::Render);
        assert_eq!(app.workspace(), Workspace::Render);
        assert_eq!(app.session.scene.display.style, VisualStyle::Realistic);
        assert!(app.render.library);
        app.set_workspace(Workspace::Design);
        assert_eq!(app.session.scene.display.style, VisualStyle::ShadedEdges);
        assert!(!app.render.active);
    }

    #[test]
    fn a_library_material_goes_to_the_selected_body_and_the_render_window_renders() {
        let mut app = ChamferaApp::headless();
        app.run("body.sample", json!({}));
        let body = app.session.scene.bodies[0].id;
        app.session.scene.selected = vec![body];
        app.assign_material("brass", &[body]);
        let a = app.session.scene.bodies[0].appearance;
        assert_eq!(a.metallic, 1.0);
        app.open_render_window();
        if let Some(w) = &mut app.render.window {
            w.width = "16".into();
            w.height = "10".into();
            w.samples = "2".into();
        }
        app.start_render();
        let job = app
            .render
            .window
            .as_ref()
            .and_then(|w| w.job.clone())
            .unwrap();
        // The worker finishes the two samples.
        for _ in 0..1000 {
            if job.done() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(job.samples(), 2);
        // Bad numbers are refused without starting anything.
        if let Some(w) = &mut app.render.window {
            w.samples = "lots".into();
        }
        app.start_render();
        assert!(app.message.as_ref().is_some_and(|m| m.1));
    }
}

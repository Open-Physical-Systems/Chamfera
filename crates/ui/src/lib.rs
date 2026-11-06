//! Layer 6: the user interface (egui + wgpu), for the desktop and the browser.
//!
//! The UI is thin: it reads the session and acts only through commands (`run`), so what a
//! user does, a script or an agent can do the same way. Colours come from the theme tokens
//! in `chamfera_render::Theme`. All camera input goes through `navigate`.

use chamfera_engine::{Session, registry};
use chamfera_render::{Camera, Hit, Theme, ThemeMode};
use egui::{Color32, PointerButton, Rect, Sense, UiBuilder, Vec2};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

mod sketch_ui;

/// How the app starts.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Insert the sample part.
    pub sample: bool,
    /// STL files to insert.
    pub files: Vec<std::path::PathBuf>,
    /// Save a screenshot of the window here once it has drawn, then quit.
    pub screenshot: Option<std::path::PathBuf>,
    /// Force a theme instead of following the system.
    pub theme: Option<ThemeMode>,
    /// Commands to run at start: `(command id, params)`, as in a `chamfera-cli run` script.
    pub script: Vec<(String, Value)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

/// The GPU side of the viewport, created once the window's device exists.
struct ViewGpu {
    renderer: chamfera_gpu::ViewRenderer,
    texture: Option<egui::TextureId>,
}

/// What was drawn last, so an unchanged frame is not drawn again.
#[derive(PartialEq)]
struct Drawn {
    camera: Camera,
    bodies: Vec<(u32, bool, usize)>,
    selection: Option<Hit>,
    overlays: Vec<chamfera_render::Overlay>,
    theme: ThemeMode,
    size: (u32, u32),
}

enum Shot {
    Waiting(u32),
    Requested,
    Done,
}

pub struct ChamferaApp {
    session: Session,
    theme_choice: ThemeChoice,
    theme: Theme,
    styled_for: Option<ThemeMode>,
    gpu: Option<ViewGpu>,
    drawn: Option<Drawn>,
    /// Viewport size in points; commands work in these units.
    viewport: Vec2,
    fit_pending: bool,
    message: Option<(String, bool)>,
    screenshot: Option<(std::path::PathBuf, Shot)>,
    shot_image: Arc<Mutex<Option<Arc<egui::ColorImage>>>>,
    sketch: sketch_ui::SketchUi,
}

/// A command's label, so menus show the same words as the command list.
fn label(id: &str) -> &'static str {
    registry()
        .into_iter()
        .find(|c| c.id == id)
        .map(|c| c.label)
        .unwrap_or("?")
}

fn color(c: chamfera_render::Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}

impl ChamferaApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> ChamferaApp {
        let gpu = cc.wgpu_render_state.as_ref().map(|rs| ViewGpu {
            renderer: chamfera_gpu::ViewRenderer::new(&rs.device),
            texture: None,
        });
        let mut app = ChamferaApp::with_gpu(gpu, &options);
        if app.gpu.is_none() {
            app.message = Some(("No GPU device: the viewport cannot draw.".into(), true));
        }
        app
    }

    /// An app without a window or GPU, for tests.
    #[cfg(test)]
    pub(crate) fn headless() -> ChamferaApp {
        ChamferaApp::with_gpu(None, &Options::default())
    }

    fn with_gpu(gpu: Option<ViewGpu>, options: &Options) -> ChamferaApp {
        let mut app = ChamferaApp {
            session: Session::new(),
            theme_choice: match options.theme {
                None => ThemeChoice::System,
                Some(ThemeMode::Light) => ThemeChoice::Light,
                Some(ThemeMode::Dark) => ThemeChoice::Dark,
            },
            theme: Theme::light(),
            styled_for: None,
            gpu,
            drawn: None,
            viewport: Vec2::new(1280.0, 800.0),
            // A start-up script sets its own view.
            fit_pending: options.script.is_empty(),
            message: None,
            screenshot: options.screenshot.clone().map(|p| (p, Shot::Waiting(4))),
            shot_image: Arc::new(Mutex::new(None)),
            sketch: sketch_ui::SketchUi::default(),
        };
        if options.sample {
            app.run("body.sample", json!({}));
        }
        for f in &options.files {
            app.run("body.import_stl", json!({ "path": f.to_string_lossy() }));
        }
        for (id, params) in &options.script {
            if app.run(id, params.clone()).is_none() {
                break;
            }
        }
        if app.session.active_sketch.is_some() {
            app.sketch.tool = Some(sketch_ui::Tool::Select);
            app.refresh_sketch_status();
        }
        app
    }

    /// Runs a command and shows its error, if any, in the status bar.
    fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.session.execute(id, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.message = Some((format!("{}: {e}", label(id)), true));
                None
            }
        }
    }

    fn viewport_params(&self) -> Value {
        json!({ "width": self.viewport.x.max(1.0), "height": self.viewport.y.max(1.0) })
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let mode = match self.theme_choice {
            ThemeChoice::Light => ThemeMode::Light,
            ThemeChoice::Dark => ThemeMode::Dark,
            ThemeChoice::System => match ctx.system_theme() {
                Some(egui::Theme::Dark) => ThemeMode::Dark,
                _ => ThemeMode::Light,
            },
        };
        if self.styled_for == Some(mode) {
            return;
        }
        self.theme = Theme::for_mode(mode);
        let t = &self.theme;
        let mut v = match mode {
            ThemeMode::Light => egui::Visuals::light(),
            ThemeMode::Dark => egui::Visuals::dark(),
        };
        v.panel_fill = color(t.panel);
        v.window_fill = color(t.panel);
        v.faint_bg_color = color(t.panel_alt);
        v.extreme_bg_color = color(t.panel_alt);
        v.override_text_color = Some(color(t.text));
        v.selection.bg_fill = color(t.accent);
        v.hyperlink_color = color(t.accent);
        v.error_fg_color = color(t.error);
        ctx.set_visuals(v);
        self.styled_for = Some(mode);
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if ui
                    .button(format!("{}…", label("body.import_stl")))
                    .clicked()
                {
                    ui.close();
                    self.import_dialog();
                }
                if ui.button(label("body.sample")).clicked() {
                    ui.close();
                    self.run("body.sample", json!({}));
                    self.fit_pending = true;
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button("Sketch", |ui| {
                ui.label(label("sketch.create"));
                for (plane, text, view) in sketch_ui::PLANES {
                    if ui
                        .add_enabled(
                            self.session.active_sketch.is_none(),
                            egui::Button::new(text),
                        )
                        .clicked()
                    {
                        ui.close();
                        self.create_sketch(plane, view);
                    }
                }
                if self.session.active_sketch.is_some() {
                    ui.separator();
                    if ui.button(label("sketch.finish")).clicked() {
                        ui.close();
                        self.finish_sketch();
                    }
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button(format!("{}    H", label("view.home"))).clicked() {
                    ui.close();
                    self.run("view.home", self.viewport_params());
                }
                if ui.button(format!("{}    F", label("view.fit"))).clicked() {
                    ui.close();
                    self.run("view.fit", self.viewport_params());
                }
                ui.separator();
                for (name, text) in [
                    ("front", "Front"),
                    ("back", "Back"),
                    ("left", "Left"),
                    ("right", "Right"),
                    ("top", "Top"),
                    ("bottom", "Bottom"),
                    ("iso", "Isometric"),
                ] {
                    if ui.button(text).clicked() {
                        ui.close();
                        self.run("view.standard", json!({ "view": name }));
                        self.run("view.fit", self.viewport_params());
                    }
                }
                ui.separator();
                let ortho =
                    self.session.camera.projection == chamfera_render::Projection::Orthographic;
                let mut o = ortho;
                if ui.checkbox(&mut o, "Orthographic").changed() {
                    let mode = if o { "orthographic" } else { "perspective" };
                    self.run("view.projection", json!({ "mode": mode }));
                }
                ui.separator();
                ui.label("Theme");
                for (c, text) in [
                    (ThemeChoice::System, "Follow system"),
                    (ThemeChoice::Light, "Light"),
                    (ThemeChoice::Dark, "Dark"),
                ] {
                    if ui.radio(self.theme_choice == c, text).clicked() {
                        self.theme_choice = c;
                        self.styled_for = None;
                    }
                }
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn import_dialog(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .set_title(label("body.import_stl"))
            .add_filter("STL mesh", &["stl", "STL"])
            .pick_files()
        else {
            return;
        };
        for f in files {
            if let Some(v) = self.run("body.import_stl", json!({ "path": f.to_string_lossy() })) {
                self.message = Some((
                    format!(
                        "Inserted {} ({} triangles)",
                        v["name"].as_str().unwrap_or("mesh"),
                        v["triangles"]
                    ),
                    false,
                ));
            }
        }
        self.fit_pending = true;
    }

    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.strong("Browser");
        ui.separator();
        let selected = self.session.scene.selection.map(|h| h.body);
        let rows: Vec<(u32, String, bool)> = self
            .session
            .scene
            .bodies
            .iter()
            .map(|b| (b.id, b.name.clone(), b.visible))
            .collect();
        let sketches: Vec<(u32, String)> = self
            .session
            .doc
            .sketches
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        if rows.is_empty() && sketches.is_empty() {
            ui.weak("Empty design. Sketch → Create Sketch, File → Insert Mesh (STL), or Insert Sample Part.");
        }
        for (id, name) in sketches {
            ui.horizontal(|ui| {
                let active = self.session.active_sketch == Some(id);
                let text = egui::RichText::new(name.as_str());
                let text = if active {
                    text.color(color(self.theme.accent)).strong()
                } else {
                    text
                };
                ui.label(text);
                if !active
                    && self.session.active_sketch.is_none()
                    && ui.small_button(label("sketch.edit")).clicked()
                {
                    self.run_sketch("sketch.edit", json!({ "sketch": id }));
                    self.sketch.tool = Some(sketch_ui::Tool::Select);
                }
            });
        }
        for (id, name, visible) in rows {
            ui.horizontal(|ui| {
                let mut v = visible;
                if ui
                    .checkbox(&mut v, "")
                    .on_hover_text(label("body.visibility"))
                    .changed()
                {
                    self.run("body.visibility", json!({ "body": id, "visible": v }));
                }
                let text = egui::RichText::new(&name);
                let text = if selected == Some(id) {
                    text.color(color(self.theme.accent)).strong()
                } else {
                    text
                };
                ui.label(text);
            });
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.session.active_sketch.is_some() {
                ui.colored_label(sketch_ui::hint_color(self), sketch_ui::tool_hint(self));
                if let Some(s) = &self.sketch.status {
                    ui.separator();
                    ui.label(s);
                }
                ui.separator();
            }
            match &self.message {
                Some((m, true)) => {
                    ui.colored_label(color(self.theme.error), m);
                }
                Some((m, false)) => {
                    ui.label(m);
                }
                None => {
                    ui.weak("Left-click: select · Middle-drag: pan · Shift+middle or right-drag: orbit · Wheel: zoom");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tris: usize = self.session.scene.bodies.iter().map(|b| b.mesh.triangles.len()).sum();
                let n = self.session.scene.bodies.len();
                let bodies = if n == 1 { "body" } else { "bodies" };
                ui.weak(format!("{n} {bodies} · {tris} triangles"));
                if let Some(h) = self.session.scene.selection {
                    let name = self.session.scene.body(h.body).map(|b| b.name.as_str()).unwrap_or("");
                    let n = self
                        .session
                        .scene
                        .body(h.body)
                        .and_then(|b| b.mesh.triangle_normal(h.triangle))
                        .unwrap_or_default();
                    ui.separator();
                    ui.label(format!(
                        "{name} · point ({:.2}, {:.2}, {:.2}) mm · normal ({:.2}, {:.2}, {:.2})",
                        h.point.x, h.point.y, h.point.z, n.x, n.y, n.z
                    ));
                }
            });
        });
    }

    /// Mouse, touchpad and keys → view and select commands. The only camera input path.
    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let local = |p: egui::Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        let (shift, scroll, zoom, keys) = ui.input(|i| {
            (
                i.modifiers.shift,
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                (
                    i.key_pressed(egui::Key::F),
                    i.key_pressed(egui::Key::H),
                    i.key_pressed(egui::Key::Escape),
                ),
            )
        });
        let delta = response.drag_delta();
        if delta != Vec2::ZERO {
            let (dx, dy) = (f64::from(delta.x), f64::from(delta.y));
            let orbit = response.dragged_by(PointerButton::Secondary)
                || (response.dragged_by(PointerButton::Middle) && shift);
            if orbit {
                self.run(
                    "view.orbit",
                    json!({ "yaw": -dx * 0.008, "pitch": dy * 0.008 }),
                );
            } else if response.dragged_by(PointerButton::Middle) {
                self.run("view.pan", json!({ "dx": dx, "dy": dy, "height": h }));
            }
        }
        if response.hovered() {
            let factor = if (zoom - 1.0).abs() > f32::EPSILON {
                1.0 / f64::from(zoom)
            } else if scroll.abs() > 0.0 {
                (-f64::from(scroll) * 0.0025).exp()
            } else {
                1.0
            };
            if (factor - 1.0).abs() > 1e-9 {
                let mut p = json!({ "factor": factor.clamp(0.01, 100.0) });
                if let Some(pos) = response.hover_pos() {
                    let (x, y) = local(pos);
                    p = json!({ "factor": factor.clamp(0.01, 100.0), "x": x, "y": y, "width": w, "height": h });
                }
                self.run("view.zoom", p);
            }
        }
        let used_by_sketch =
            self.session.active_sketch.is_some() && self.sketch_input(ui, response, rect);
        if !used_by_sketch
            && self.session.active_sketch.is_none()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.run(
                "select.pick",
                json!({ "x": x, "y": y, "width": w, "height": h }),
            );
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            if keys.0 {
                self.run("view.fit", self.viewport_params());
            }
            if keys.1 {
                self.run("view.home", self.viewport_params());
            }
            if keys.2 && self.session.active_sketch.is_none() {
                self.run("select.clear", json!({}));
            }
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.viewport = rect.size();
        if self.fit_pending && rect.width() > 1.0 && rect.height() > 1.0 {
            self.fit_pending = false;
            self.run("view.home", self.viewport_params());
        }
        self.navigate(ui, &response, rect);

        // The drawing tool's preview rides along with the sketch overlays.
        self.session
            .scene
            .overlays
            .retain(|o| o.style != chamfera_render::OverlayStyle::Preview);
        if let Some(preview) = self.sketch_preview() {
            self.session.scene.overlays.push(preview);
        }
        let ppp = ui.ctx().pixels_per_point();
        let size = (
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        );
        let now = Drawn {
            camera: self.session.camera,
            bodies: self
                .session
                .scene
                .bodies
                .iter()
                .map(|b| (b.id, b.visible, b.mesh.triangles.len()))
                .collect(),
            selection: self.session.scene.selection,
            overlays: self.session.scene.overlays.clone(),
            theme: self.theme.mode,
            size,
        };
        if let (Some(gpu), Some(rs)) = (self.gpu.as_mut(), frame.wgpu_render_state())
            && self.drawn.as_ref() != Some(&now)
        {
            let commands = gpu.renderer.render(
                &rs.device,
                &rs.queue,
                &chamfera_gpu::Frame {
                    scene: &self.session.scene,
                    camera: &self.session.camera,
                    theme: &self.theme,
                    width: size.0,
                    height: size.1,
                    scale: ppp,
                },
            );
            rs.queue.submit([commands]);
            if let Some(view) = gpu.renderer.display_view(rs.target_format.is_srgb()) {
                let mut r = rs.renderer.write();
                match gpu.texture {
                    Some(id) => r.update_egui_texture_from_wgpu_texture(
                        &rs.device,
                        view,
                        egui_wgpu::wgpu::FilterMode::Linear,
                        id,
                    ),
                    None => {
                        gpu.texture = Some(r.register_native_texture(
                            &rs.device,
                            view,
                            egui_wgpu::wgpu::FilterMode::Linear,
                        ));
                    }
                }
            }
            self.drawn = Some(now);
        }
        if let Some(Some(id)) = self.gpu.as_ref().map(|g| g.texture) {
            ui.painter().image(
                id,
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.session.active_sketch.is_some() {
            self.dimension_labels(ui, rect);
        }
        self.view_bar(ui, rect);
    }

    /// Standard-view buttons in the viewport's top-right corner.
    fn view_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let bar = Rect::from_min_size(
            egui::pos2(rect.max.x - 330.0, rect.min.y + 8.0),
            Vec2::new(322.0, 28.0),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        egui::Frame::popup(child.style()).show(&mut child, |ui| {
            ui.horizontal(|ui| {
                for (name, text) in [
                    ("iso", "Iso"),
                    ("right", "Right"),
                    ("top", "Top"),
                    ("front", "Front"),
                ] {
                    if ui.small_button(text).clicked() {
                        self.run("view.standard", json!({ "view": name }));
                        self.run("view.fit", self.viewport_params());
                    }
                }
                if ui.small_button(label("view.fit")).clicked() {
                    self.run("view.fit", self.viewport_params());
                }
                if ui.small_button(label("view.home")).clicked() {
                    self.run("view.home", self.viewport_params());
                }
            });
        });
    }

    /// `--screenshot`: wait a few frames for the view to settle, grab the window, save, quit.
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some((path, state)) = &mut self.screenshot else {
            return;
        };
        match state {
            Shot::Waiting(0) => {
                let slot = self.shot_image.clone();
                let repaint = ctx.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |image| {
                        if let Ok(mut s) = slot.lock() {
                            *s = Some(image);
                        }
                        repaint.request_repaint();
                    }),
                ));
                *state = Shot::Requested;
            }
            Shot::Waiting(n) => {
                *n -= 1;
                ctx.request_repaint();
            }
            Shot::Requested => {
                let image = self.shot_image.lock().ok().and_then(|mut s| s.take());
                if let Some(image) = image {
                    let result = save_png(path, &image);
                    match result {
                        Ok(()) => println!("screenshot saved: {}", path.display()),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                    *state = Shot::Done;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    ctx.request_repaint();
                }
            }
            Shot::Done => {}
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = image.size;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(image.as_raw())
        .map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn save_png(_: &std::path::Path, _: &egui::ColorImage) -> Result<(), String> {
    Err("screenshots are saved by the desktop app only".into())
}

impl eframe::App for ChamferaApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        if self.session.active_sketch.is_some() {
            egui::Panel::top("sketch toolbar").show(ui, |ui| self.sketch_toolbar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("browser")
            .resizable(true)
            .default_size(220.0)
            .min_size(140.0)
            .show(ui, |ui| self.browser(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, frame));
        self.screenshot_step(&ctx);
    }
}

/// Opens the desktop window.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native(options: Options) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Chamfera")
            .with_app_id("chamfera")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Chamfera",
        native,
        Box::new(move |cc| Ok(Box::new(ChamferaApp::new(cc, options)))),
    )
    .map_err(|e| e.to_string())
}

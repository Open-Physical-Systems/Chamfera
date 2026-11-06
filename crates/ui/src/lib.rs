//! Layer 6: the user interface (egui + wgpu), for the desktop and the browser.
//!
//! The UI is thin: it reads the session and acts only through commands (`run`), so what a
//! user does, a script or an agent can do the same way. Colours come from the theme tokens
//! in `chamfera_render::Theme`. All camera input goes through `navigate`.
//!
//! Layout (`docs/design/ui.md`): the app bar and the ribbon on top, the Browser on the
//! left, the viewport with its view cube in the middle, the timeline and the status bar at
//! the bottom.

use chamfera_engine::{Session, registry};
use chamfera_render::{Camera, Hit, Overlay, Theme, ThemeMode};
use egui::{Color32, PointerButton, Rect, Sense, Vec2};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

mod blend_ui;
mod browser;
mod chrome;
mod file_ui;

/// `--screenshot` gives up after this long without a drawn frame.
const SHOT_TIMEOUT_SECS: f64 = 30.0;
/// Set when `--screenshot` gave up, so the app exits with an error.
static SHOT_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
mod icons;
mod params_ui;
mod search;
mod shell_ui;
mod shortcuts;
mod sketch_select;
mod sketch_ui;
mod solid_ui;
mod timeline;
mod viewcube;
#[cfg(target_arch = "wasm32")]
mod web;
mod widgets;

use widgets::Look;

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
    pub script: Vec<Value>,
    /// Open the Parameters window at start.
    pub parameters: bool,
    /// Open the Extrude tool at start and pick the profile under the viewport's centre,
    /// as a click there would.
    pub extrude: bool,
    /// The window's inner size in points, instead of the default 1400 x 900.
    pub window_size: Option<[f32; 2]>,
    /// Interface zoom (pixels per point multiplier), for checking crispness at 2x.
    pub zoom: Option<f32>,
    /// Show a selection box with these viewport corners (x1, y1, x2, y2) as if it were
    /// being dragged, for screenshots of the window and crossing styles.
    pub select_box: Option<[f32; 4]>,
    /// Open the search palette at start with this query typed.
    pub search: Option<String>,
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
    selected: Vec<u32>,
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
    icons: icons::Icons,
    gpu: Option<ViewGpu>,
    drawn: Option<Drawn>,
    /// Viewport size in points; commands work in these units.
    viewport: Vec2,
    /// Where the viewport is on screen, for placing dialogs beside the model.
    viewport_rect: Rect,
    fit_pending: bool,
    message: Option<(String, bool)>,
    screenshot: Option<(std::path::PathBuf, Shot)>,
    /// When `--screenshot` started (egui seconds), to give up instead of hanging: a
    /// covered window on Wayland may never be asked to draw.
    shot_started: Option<f64>,
    shot_image: Arc<Mutex<Option<Arc<egui::ColorImage>>>>,
    sketch: sketch_ui::SketchUi,
    params: params_ui::ParamsUi,
    solid: solid_ui::SolidUi,
    file: file_ui::FileUi,
    browser: browser::BrowserUi,
    timeline: timeline::TimelineUi,
    search: search::SearchUi,
    /// The browser's file picker, downloads and local storage.
    #[cfg(target_arch = "wasm32")]
    web: web::WebFiles,
    /// Close the window on the next frame (Quit was confirmed).
    ctx_close: bool,
    /// `--zoom`: applied on the first frame.
    zoom: Option<f32>,
    /// Frames drawn so far (up to a few): dialogs wait until the layout has settled, so
    /// their first position is beside the real viewport.
    frames: u8,
    /// A box selection being dragged (`sketch_select`).
    select_box: Option<sketch_select::BoxDrag>,
    /// `--box`: viewport corners of a box to show as if dragged, until the next drag.
    box_preview: Option<[f32; 4]>,
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
        #[cfg(target_arch = "wasm32")]
        app.restore_from_browser();
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
            icons: icons::Icons::default(),
            gpu,
            drawn: None,
            viewport: Vec2::new(1280.0, 800.0),
            viewport_rect: Rect::from_min_size(egui::pos2(260.0, 120.0), Vec2::new(1280.0, 800.0)),
            // A start-up script sets its own view.
            fit_pending: options.script.is_empty(),
            message: None,
            screenshot: options.screenshot.clone().map(|p| (p, Shot::Waiting(6))),
            shot_image: Arc::new(Mutex::new(None)),
            shot_started: None,
            sketch: sketch_ui::SketchUi::default(),
            params: params_ui::ParamsUi::default(),
            solid: solid_ui::SolidUi::default(),
            file: file_ui::FileUi::default(),
            browser: browser::BrowserUi::default(),
            timeline: timeline::TimelineUi::default(),
            search: search::SearchUi::default(),
            #[cfg(target_arch = "wasm32")]
            web: web::WebFiles::default(),
            ctx_close: false,
            zoom: None,
            frames: 0,
            select_box: None,
            box_preview: None,
        };
        if options.sample {
            app.run("body.sample", json!({}));
        }
        for f in &options.files {
            app.run("body.import_stl", json!({ "path": f.to_string_lossy() }));
        }
        if !options.script.is_empty()
            && let Err(e) = chamfera_engine::script::run_steps(&mut app.session, &options.script)
        {
            app.message = Some((e.to_string(), true));
        }
        if app.session.active_sketch.is_some() {
            app.sketch.tool = Some(sketch_ui::Tool::Select);
            app.refresh_sketch_status();
        }
        app.params.open = options.parameters;
        if options.extrude {
            app.solid.extrude = Some(solid_ui::ExtrudeTool::default());
            app.solid.pick_centre = true;
        }
        app.zoom = options.zoom;
        app.box_preview = options.select_box;
        if let Some(q) = &options.search {
            app.run_action(shortcuts::Action::Search);
            app.search.query = q.chars().take(120).collect();
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

    /// Looks along a standard view and fits the model in it.
    fn standard_view(&mut self, view: &str) {
        self.run("view.standard", json!({ "view": view }));
        self.run("view.fit", self.viewport_params());
    }

    fn look(&self) -> Look<'_> {
        Look {
            theme: &self.theme,
            icons: &self.icons,
        }
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
        // Both of egui's slots get our style, so its own theme switch never shows another.
        let style = Arc::new(widgets::style_for(&self.theme, &ctx.global_style()));
        ctx.set_style_of(egui::Theme::Light, style.clone());
        ctx.set_style_of(egui::Theme::Dark, style);
        self.styled_for = Some(mode);
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
        if self.solid.pick_face
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.face_click(x, y, w, h);
            return;
        }
        if self.solid.blend.is_some() {
            let at = response.hover_pos().map(local);
            self.blend_hover(at, w, h);
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let (x, y) = local(pos);
                self.blend_click(x, y, w, h);
                return;
            }
        }
        if self.solid.shell.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.shell_click(x, y, w, h);
            return;
        }
        if self.solid.extrude.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.extrude_click(x, y, w, h);
            return;
        }
        let used_by_sketch =
            self.session.active_sketch.is_some() && self.sketch_input(ui, response, rect);
        if self.session.active_sketch.is_none() {
            // A left drag from anywhere draws a selection box (`sketch_select`).
            if response.drag_started_by(PointerButton::Primary) && self.select_box.is_none() {
                self.box_begin(ui, rect, false);
            }
            if self.box_input(ui, response, rect) {
                return;
            }
        }
        if !used_by_sketch
            && self.session.active_sketch.is_none()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            let mode = sketch_select::pick_mode(ui.input(|i| i.modifiers));
            self.run(
                "select.pick",
                json!({ "x": x, "y": y, "width": w, "height": h, "mode": mode }),
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
                if self.solid.pick_face {
                    self.solid.pick_face = false;
                    self.message = None;
                } else {
                    self.run("select.clear", json!({}));
                }
            }
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.viewport = rect.size();
        self.viewport_rect = rect;
        if self.fit_pending && rect.width() > 1.0 && rect.height() > 1.0 {
            self.fit_pending = false;
            self.run("view.home", self.viewport_params());
        }
        if self.solid.pick_centre && rect.width() > 1.0 && rect.height() > 1.0 {
            self.solid.pick_centre = false;
            let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
            self.extrude_click(w * 0.5, h * 0.5, w, h);
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
        let blends = self.blend_overlays();
        self.session.scene.overlays.extend(blends);
        if let Some(picked) = self.extrude_overlay() {
            self.session.scene.overlays.push(Overlay {
                style: chamfera_render::OverlayStyle::Highlight,
                ..picked
            });
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
            selected: self.session.scene.selected.clone(),
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
        self.paint_box(ui, rect);
        self.view_cube(ui, rect);
        self.viewport_hint(ui, rect);
    }

    /// `--screenshot`: wait a few frames for the view to settle, grab the window, save, quit.
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some((path, state)) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let started = *self.shot_started.get_or_insert(now);
        if !matches!(state, Shot::Done) && now - started >= SHOT_TIMEOUT_SECS {
            eprintln!(
                "screenshot failed: no frame was drawn within {SHOT_TIMEOUT_SECS} s \
                 (is the window hidden or covered?)"
            );
            *state = Shot::Done;
            self.file.allow_close = true;
            SHOT_FAILED.store(true, std::sync::atomic::Ordering::SeqCst);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
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
                    self.file.allow_close = true;
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
    /// Runs every frame, even when the window is covered or minimised (then eframe skips
    /// `ui`). `--screenshot` lives here and asks for painting while hidden, so a covered
    /// window still draws, captures and quits instead of waiting forever.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.screenshot.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint();
        }
        self.screenshot_step(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(z) = self.zoom.take()
            && z.is_finite()
        {
            ctx.set_zoom_factor(z.clamp(0.5, 4.0));
        }
        self.apply_theme(&ctx);
        // Keys first, so the palette and the shortcuts take theirs before anything else.
        self.search_keys(&ctx);
        self.shortcut_keys(&ctx);
        let chrome = egui::Frame::new()
            .fill(color(self.theme.chrome))
            .inner_margin(egui::Margin::symmetric(widgets::S3 as i8, 0));
        egui::Panel::top("app bar")
            .frame(chrome)
            .exact_size(widgets::APP_BAR_HEIGHT)
            .show_separator_line(true)
            .show(ui, |ui| self.app_bar(ui));
        egui::Panel::top("ribbon")
            .frame(chrome)
            .exact_size(widgets::RIBBON_HEIGHT)
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("status")
            .frame(chrome)
            .exact_size(widgets::STATUS_HEIGHT)
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("timeline")
            .frame(chrome)
            .exact_size(widgets::TIMELINE_HEIGHT)
            .show(ui, |ui| self.timeline_bar(ui));
        egui::Panel::left("browser")
            .frame(
                egui::Frame::new()
                    .fill(color(self.theme.panel))
                    .inner_margin(egui::Margin::symmetric(
                        widgets::S2 as i8 + 2,
                        widgets::S3 as i8,
                    )),
            )
            .resizable(true)
            .default_size(248.0)
            .size_range(180.0..=420.0)
            .show(ui, |ui| self.browser(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, frame));
        if self.frames < 2 {
            self.frames += 1;
            ctx.request_repaint();
        } else {
            self.parameters_window(&ctx);
            self.extrude_dialog(&ctx);
            self.blend_dialog(&ctx);
            self.shell_dialog(&ctx);
            self.feature_dialog(&ctx);
            self.timeline_error_window(&ctx);
        }
        self.rename_box(&ctx);
        self.unsaved_dialog(&ctx);
        self.shortcuts_window(&ctx);
        self.search_window(&ctx);
        self.file_shortcuts(&ctx);
        #[cfg(target_arch = "wasm32")]
        self.browser_files(&ctx);
        self.update_title(&ctx);
        self.handle_close_request(&ctx);
    }
}

/// Opens the desktop window.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native(options: Options) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Chamfera")
            .with_app_id("chamfera")
            .with_inner_size(options.window_size.unwrap_or([1400.0, 900.0]))
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Chamfera",
        native,
        Box::new(move |cc| Ok(Box::new(ChamferaApp::new(cc, options)))),
    )
    .map_err(|e| e.to_string())?;
    if SHOT_FAILED.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("the screenshot could not be taken".into());
    }
    Ok(())
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use eframe::App;

    /// Runs one frame of the real UI with these input events.
    fn frame(app: &mut ChamferaApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
        frame_with(app, ctx, t, events, egui::Modifiers::NONE);
    }

    /// One frame with these keys held (Shift, Ctrl).
    fn frame_with(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let mut all = vec![egui::Event::ModifiersChanged(modifiers)];
        all.extend(events);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1400.0, 900.0),
            )),
            time: Some(t),
            events: all,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
    }

    #[test]
    fn a_left_click_in_the_viewport_selects_the_body_under_it() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.run("body.sample", json!({}));
        app.fit_pending = true;
        let mut t = 0.0;
        for _ in 0..4 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        let at = app.viewport_rect.center();
        eprintln!("viewport {:?}", app.viewport_rect);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, &ctx, t, vec![egui::Event::PointerMoved(at)]);
        t += 0.05;
        frame(&mut app, &ctx, t, vec![button(true)]);
        t += 0.08;
        frame(&mut app, &ctx, t, vec![button(false)]);
        t += 0.05;
        frame(&mut app, &ctx, t, vec![]);
        assert!(app.session.scene.selection.is_some(), "nothing selected");
    }

    fn click(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, *t, vec![egui::Event::PointerMoved(at)]);
        *t += 0.05;
        frame(app, ctx, *t, vec![button(true)]);
        *t += 0.08;
        frame(app, ctx, *t, vec![button(false)]);
        *t += 0.05;
        frame(app, ctx, *t, vec![]);
        *t += 0.05;
    }

    #[test]
    fn two_left_clicks_in_a_new_sketch_draw_a_line() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        let mut t = 0.0;
        for _ in 0..3 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        app.create_sketch("xy", "top");
        frame(&mut app, &ctx, t, vec![]);
        t += 0.05;
        let c = app.viewport_rect.center();
        click(&mut app, &ctx, &mut t, c);
        click(&mut app, &ctx, &mut t, c + Vec2::new(120.0, 7.0));
        let sketch = app.session.active_sketch.unwrap();
        let n = app
            .session
            .doc
            .sketch(sketch)
            .unwrap()
            .sketch
            .curves()
            .count();
        assert!(n >= 1, "no line drawn");
    }

    /// A left drag from `a` to `b` in small steps, with these keys held.
    fn drag(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        (a, b): (egui::Pos2, egui::Pos2),
        button: egui::PointerButton,
        modifiers: egui::Modifiers,
    ) {
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        };
        frame_with(app, ctx, *t, vec![egui::Event::PointerMoved(a)], modifiers);
        *t += 0.05;
        frame_with(app, ctx, *t, vec![press(a, true)], modifiers);
        for k in 1..=8 {
            *t += 0.03;
            let p = a + (b - a) * (k as f32 / 8.0);
            frame_with(app, ctx, *t, vec![egui::Event::PointerMoved(p)], modifiers);
        }
        *t += 0.03;
        frame_with(app, ctx, *t, vec![press(b, false)], modifiers);
        *t += 0.05;
        frame_with(app, ctx, *t, vec![], modifiers);
        *t += 0.05;
    }

    /// A left click at `at` with these keys held.
    fn click_with(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        at: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame_with(app, ctx, *t, vec![egui::Event::PointerMoved(at)], modifiers);
        *t += 0.05;
        frame_with(app, ctx, *t, vec![press(true)], modifiers);
        *t += 0.08;
        frame_with(app, ctx, *t, vec![press(false)], modifiers);
        *t += 0.05;
        frame_with(app, ctx, *t, vec![], modifiers);
        *t += 0.05;
    }

    /// Orthographic top view centred on model (cx, cy) with a 30 mm half height, so the
    /// viewport (height H points) shows H / 60 points per mm. Returns model (x, y) to
    /// screen position: the viewport centre plus (x - cx, -(y - cy)) times that scale.
    fn top_view(
        app: &mut ChamferaApp,
        cx: f64,
        cy: f64,
    ) -> impl Fn(f64, f64) -> egui::Pos2 + use<> {
        app.run("view.standard", json!({ "view": "top" }));
        app.session.camera.projection = chamfera_render::Projection::Orthographic;
        app.session.camera.target = chamfera_geom::Point3::new(cx, cy, 0.0);
        app.session.camera.distance = 30.0 / (app.session.camera.fov_y * 0.5).tan();
        let r = app.viewport_rect;
        let per_mm = f64::from(r.height()) / 60.0;
        let c = r.center();
        move |x, y| {
            egui::pos2(
                c.x + ((x - cx) * per_mm) as f32,
                c.y - ((y - cy) * per_mm) as f32,
            )
        }
    }

    /// Two 10 mm cubes: A at x 0..10, B at x 30..40, both y 0..10. The view is centred
    /// on (20, 5).
    fn two_cubes(
        ctx: &egui::Context,
        t: &mut f64,
    ) -> (
        ChamferaApp,
        u32,
        u32,
        impl Fn(f64, f64) -> egui::Pos2 + use<>,
    ) {
        use chamfera_geom::{Point3, TriMesh};
        let mut app = ChamferaApp::headless();
        let a = app
            .session
            .scene
            .add(
                "A",
                TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0)),
            )
            .unwrap();
        let b = app
            .session
            .scene
            .add(
                "B",
                TriMesh::cuboid(Point3::new(30.0, 0.0, 0.0), Point3::new(40.0, 10.0, 10.0)),
            )
            .unwrap();
        for _ in 0..4 {
            frame(&mut app, ctx, *t, vec![]);
            *t += 0.05;
        }
        let px = top_view(&mut app, 20.0, 5.0);
        (app, a, b, px)
    }

    fn selected(app: &ChamferaApp) -> Vec<u32> {
        let mut ids = app.session.scene.selected_bodies();
        ids.sort();
        ids
    }

    #[test]
    fn a_left_drag_to_the_right_is_a_window_and_to_the_left_a_crossing() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let (mut app, a, b, px) = two_cubes(&ctx, &mut t);
        let none = egui::Modifiers::NONE;
        let left = egui::PointerButton::Primary;
        // Window from (-2, 12) to (35, -2): all of A, half of B. Only A is fully inside.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(-2.0, 12.0), px(35.0, -2.0)),
            left,
            none,
        );
        assert!(app.select_box.is_none(), "the box ends with the drag");
        assert_eq!(selected(&app), vec![a]);
        // The same box dragged the other way is a crossing: it touches both.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(35.0, -2.0), px(-2.0, 12.0)),
            left,
            none,
        );
        assert_eq!(selected(&app), vec![a, b]);
        // A crossing over the gap only (x 15..25) selects nothing, replacing the set.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(25.0, 12.0), px(15.0, -2.0)),
            left,
            none,
        );
        assert!(selected(&app).is_empty());
    }

    #[test]
    fn shift_and_ctrl_clicks_toggle_ctrl_shift_removes_and_empty_space_or_esc_clears() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let (mut app, a, b, px) = two_cubes(&ctx, &mut t);
        let (none, shift, ctrl) = (
            egui::Modifiers::NONE,
            egui::Modifiers::SHIFT,
            egui::Modifiers::CTRL,
        );
        // Cube centres: A (5, 5), B (35, 5); the gap at (20, 5).
        click_with(&mut app, &ctx, &mut t, px(5.0, 5.0), none);
        assert_eq!(selected(&app), vec![a]);
        click_with(&mut app, &ctx, &mut t, px(35.0, 5.0), shift);
        assert_eq!(selected(&app), vec![a, b]);
        click_with(&mut app, &ctx, &mut t, px(5.0, 5.0), ctrl);
        assert_eq!(selected(&app), vec![b], "Ctrl+click toggles A off");
        click_with(&mut app, &ctx, &mut t, px(5.0, 5.0), ctrl);
        click_with(&mut app, &ctx, &mut t, px(35.0, 5.0), ctrl | shift);
        assert_eq!(selected(&app), vec![a], "Ctrl+Shift+click removes B");
        // A Shift box adds B back.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(28.0, 12.0), px(42.0, -2.0)),
            egui::PointerButton::Primary,
            shift,
        );
        assert_eq!(selected(&app), vec![a, b]);
        // A plain click on empty space clears.
        click_with(&mut app, &ctx, &mut t, px(20.0, 5.0), none);
        assert!(selected(&app).is_empty());
        // Esc clears too.
        click_with(&mut app, &ctx, &mut t, px(5.0, 5.0), none);
        let esc = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: none,
        };
        frame(&mut app, &ctx, t, vec![esc(true)]);
        t += 0.05;
        frame(&mut app, &ctx, t, vec![esc(false)]);
        assert!(selected(&app).is_empty());
    }

    #[test]
    fn a_right_drag_still_orbits_and_draws_no_box() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let (mut app, _, _, px) = two_cubes(&ctx, &mut t);
        let yaw = app.session.camera.yaw;
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(-2.0, 12.0), px(35.0, -2.0)),
            egui::PointerButton::Secondary,
            egui::Modifiers::NONE,
        );
        assert!((app.session.camera.yaw - yaw).abs() > 1e-3, "no orbit");
        assert!(selected(&app).is_empty());
    }

    #[test]
    fn in_a_sketch_a_drag_on_empty_space_boxes_and_a_drag_on_a_point_moves_it() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = ChamferaApp::headless();
        for _ in 0..3 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        app.create_sketch("xy", "top");
        let line = app
            .run(
                "sketch.line",
                json!({ "x1": 10, "y1": 10, "x2": 20, "y2": 10 }),
            )
            .unwrap();
        app.sketch.tool = Some(sketch_ui::Tool::Select);
        frame(&mut app, &ctx, t, vec![]);
        t += 0.05;
        let px = top_view(&mut app, 15.0, 10.0);
        let left = egui::PointerButton::Primary;
        // A window from (8, 13) to (22, 7) holds the whole line: both ends and the line.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(8.0, 13.0), px(22.0, 7.0)),
            left,
            egui::Modifiers::NONE,
        );
        let items = serde_json::to_value(&app.session.sketch_selection).unwrap();
        let expected =
            json!([{"point": line["start"]}, {"point": line["end"]}, {"curve": line["curve"]}]);
        assert_eq!(items, expected);
        // A crossing over the middle (x 14..16) picks just the line.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(16.0, 13.0), px(14.0, 7.0)),
            left,
            egui::Modifiers::NONE,
        );
        let items = serde_json::to_value(&app.session.sketch_selection).unwrap();
        assert_eq!(items, json!([{"curve": line["curve"]}]));
        // A drag that starts on the end point (20, 10) moves it to about (20, 15).
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(20.0, 10.0), px(20.0, 15.0)),
            left,
            egui::Modifiers::NONE,
        );
        let sketch = app.session.active_sketch().unwrap();
        let end = line["end"].as_u64().unwrap() as u32;
        let p = sketch.sketch.point(chamfera_sketch::PointId(end)).unwrap();
        assert!((p.y - 15.0).abs() < 0.5, "end point at {p:?}");
    }
}

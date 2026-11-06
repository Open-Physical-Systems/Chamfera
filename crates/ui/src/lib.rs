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

mod assembly_ui;
mod assistant_ui;
mod blend_ui;
mod browser;
mod cam_ui;
#[cfg(not(target_arch = "wasm32"))]
mod cam_worker;
mod chrome;
mod config_ui;
mod construct_ui;
mod context_menu;
mod display_ui;
mod drawing_tools_ui;
mod drawing_ui;
mod file_ui;
mod history_ui;
mod hole_ui;
mod inspect_ui;

/// `--screenshot` gives up after this long without a drawn frame.
const SHOT_TIMEOUT_SECS: f64 = 30.0;
/// Set when `--screenshot` gave up, so the app exits with an error.
static SHOT_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
mod body_tools_ui;
mod icons;
mod motion_ui;
mod navigation;
mod params_ui;
mod prefs;
mod prefs_ui;
mod press_pull_ui;
mod recovery_ui;
mod render_ui;
mod ribbon;
mod search;
mod sheet_ui;
mod shell_ui;
mod shortcuts;
mod sketch_select;
mod sketch_shapes;
mod sketch_ui;
mod solid_ui;
mod spline_ui;
mod surface_ui;
mod sweep_ui;
mod timeline;
mod transform_ui;
mod tutorial_ui;
mod viewcube;
#[cfg(target_arch = "wasm32")]
mod web;
mod welcome_ui;
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
    /// Open the Configurations window at start.
    pub configurations: bool,
    /// Open the MANUFACTURE workspace's Tool Library panel at start.
    pub tool_library: bool,
    /// Open the DRAWING workspace at start.
    pub drawing: bool,
    /// Open the Extrude tool at start and pick the profile under the viewport's centre,
    /// as a click there would.
    pub extrude: bool,
    /// Extrude options for the tool `extrude` opens, as `solid.extrude` parameters (such as
    /// `{"taper": 5, "direction": "two_sides", "distance2": 4}`).
    pub extrude_with: Option<Value>,
    /// Open the Hole tool at start, on the design's last hole feature if it has one.
    pub hole: bool,
    /// The window's inner size in points, instead of the default 1400 x 900.
    pub window_size: Option<[f32; 2]>,
    /// Interface zoom (pixels per point multiplier), for checking crispness at 2x.
    pub zoom: Option<f32>,
    /// Drawing-tool steps to run in the active sketch at start, for screenshots of snaps
    /// and typed values: `tool line; click 0,0; hover 30,12; type 25` (see
    /// `sketch_ui::parse_demo`).
    pub sketch_input: Option<String>,
    /// Show a selection box with these viewport corners (x1, y1, x2, y2) as if it were
    /// being dragged, for screenshots of the window and crossing styles.
    pub select_box: Option<[f32; 4]>,
    /// In the DRAWING workspace, select what is at sheet point (x1, y1) and show it
    /// being dragged to (x2, y2), for screenshots of moving drawing items.
    pub drawing_drag: Option<[f32; 4]>,
    /// Open the search palette at start with this query typed.
    pub search: Option<String>,
    /// Open the viewport's right-click menu at this viewport point once it has drawn.
    pub context_menu: Option<[f32; 2]>,
    /// Open Press Pull, pick at this viewport point and set the distance, as a drag
    /// would: `[x, y, distance]`.
    pub press_pull: Option<[f32; 3]>,
    /// Open the Preferences window at start on this page (general, navigation, units,
    /// grid).
    pub preferences: Option<String>,
    /// Open the dropdown of this ribbon group (by caption, such as `MODIFY`) at start.
    pub ribbon_menu: Option<String>,
    /// Open the Assistant and play a recorded conversation that builds a plate, without
    /// an API key or a network call (for screenshots).
    pub assistant_demo: bool,
    /// Open the History panel at start; a script's last `version.compare` shows in it.
    pub history: bool,
    /// Keep the design the script built as if a crash had left it unsaved, and start
    /// with the question to recover it (for screenshots; a scratch recovery folder).
    pub recovery_demo: bool,
    /// Open the RENDER workspace at start.
    pub render: bool,
    /// Open the Render window and start a render of this width, height and sample count
    /// (the screenshot waits for it to finish).
    pub render_window: Option<[u32; 3]>,
    /// Start this guided tour (`tutorial_ui`, such as `first-part`).
    pub tutorial: Option<String>,
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
    fills: Vec<chamfera_render::Fill>,
    /// Visual style and section plane, and each body's appearance.
    display: chamfera_render::Display,
    appearances: Vec<chamfera_render::Appearance>,
    theme: ThemeMode,
    size: (u32, u32),
    grid: Option<u64>,
    /// How many times the tool preview was rebuilt, and how many shapes it shows.
    previews: (u64, usize),
    /// The compared versions' meshes: side and triangle count.
    compare: Vec<(chamfera_render::CompareSide, usize)>,
}

enum Shot {
    Waiting(u32),
    Requested,
    Done,
}

pub struct ChamferaApp {
    session: Session,
    theme_choice: ThemeChoice,
    /// A theme forced at start (`--theme`) and the preferences' theme then: it holds until
    /// the preferences pick another theme.
    forced_theme: Option<(ThemeChoice, ThemeChoice)>,
    theme: Theme,
    styled_for: Option<ThemeMode>,
    icons: icons::Icons,
    gpu: Option<ViewGpu>,
    drawn: Option<Drawn>,
    /// Counts rebuilds of the extrude preview, so a changed preview is drawn again.
    preview_version: u64,
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
    configs: config_ui::ConfigUi,
    solid: solid_ui::SolidUi,
    /// The SHEET METAL workspace (`sheet_ui`).
    sheet_metal: sheet_ui::SheetUi,
    assembly: assembly_ui::AssemblyUi,
    /// Drive Joint, Motion Study, Interference and Explode (`motion_ui`).
    motion: motion_ui::MotionUi,
    drawing: drawing_ui::DrawingUi,
    construct: construct_ui::ConstructUi,
    file: file_ui::FileUi,
    browser: browser::BrowserUi,
    timeline: timeline::TimelineUi,
    search: search::SearchUi,
    /// The ribbon's menus and last layout.
    ribbon: ribbon::RibbonUi,
    /// Right-click menus, the selection filter and hover pre-highlight.
    ctx_menu: context_menu::ContextMenuUi,
    /// The Manufacture workspace.
    cam: cam_ui::CamUi,
    /// Visual style, section and the Appearance panel.
    display: display_ui::DisplayUi,
    /// The RENDER workspace (`render_ui`).
    render: render_ui::RenderUi,
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
    /// The user's preferences (`prefs`), and whether changes are written to their file
    /// (not in tests).
    prefs: prefs::Preferences,
    prefs_store: bool,
    prefs_ui: prefs_ui::PrefsUi,
    /// The drag in progress and what the navigation preset made of it.
    nav_drag: Option<navigation::NavDrag>,
    /// The design file last added to the recent list.
    recent_seen: Option<std::path::PathBuf>,
    /// The Assistant panel (`assistant_ui`).
    assistant: assistant_ui::AssistantUi,
    /// The History panel and Save Version (`history_ui`).
    history: history_ui::HistoryUi,
    /// Autosave and the recovery question (`recovery_ui`).
    recovery: recovery_ui::RecoveryUi,
    /// The welcome panel and its sample gallery (`welcome_ui`).
    welcome: welcome_ui::WelcomeUi,
    /// The guided tour in progress (`tutorial_ui`).
    tutorial: tutorial_ui::TutorialUi,
    /// The thread toolpaths are made on after a rebuild (`cam_worker`), desktop only.
    #[cfg(not(target_arch = "wasm32"))]
    cam_worker: Option<cam_worker::CamWorker>,
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
        let gpu = cc.wgpu_render_state.as_ref().map(|rs| {
            chamfera_gpu::report_errors_instead_of_panicking(&rs.device);
            ViewGpu {
                renderer: chamfera_gpu::ViewRenderer::new(&rs.device),
                texture: None,
            }
        });
        // Screenshots neither read nor write the user's preferences or Assistant key, so
        // they show no recent files or key and look the same on every machine.
        let shot = options.screenshot.is_some();
        let prefs = if shot {
            prefs::Preferences::default()
        } else {
            prefs::load()
        };
        let mut app = ChamferaApp::with_gpu(gpu, &options, prefs);
        app.prefs_store = !shot;
        app.welcome.open = welcome_ui::shows_at_start(&options, app.prefs.show_welcome);
        // Autosave and the recovery question, except in screenshots, which must neither
        // write recovery files nor show another run's work.
        #[cfg(not(target_arch = "wasm32"))]
        if options.recovery_demo {
            app.recovery_demo();
        } else if options.screenshot.is_none() {
            app.recovery.dir = recovery_ui::default_dir();
            app.recovery_scan();
        }
        // The window makes toolpaths on a worker after each rebuild, so an edit to a
        // machined part does not wait for them (scripts and tests stay synchronous).
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.cam_worker = cam_worker::CamWorker::spawn();
            if app.cam_worker.is_some() {
                app.session.set_cam_background(true);
            }
        }
        if app.assistant.transport.is_none() && !shot {
            app.assistant.load_key();
        }
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
        ChamferaApp::with_gpu(None, &Options::default(), prefs::Preferences::default())
    }

    fn with_gpu(gpu: Option<ViewGpu>, options: &Options, prefs: prefs::Preferences) -> ChamferaApp {
        let mut app = ChamferaApp {
            session: Session::new(),
            theme_choice: match options.theme {
                None => prefs.theme,
                Some(ThemeMode::Light) => ThemeChoice::Light,
                Some(ThemeMode::Dark) => ThemeChoice::Dark,
            },
            forced_theme: match options.theme {
                None => None,
                Some(ThemeMode::Light) => Some((ThemeChoice::Light, prefs.theme)),
                Some(ThemeMode::Dark) => Some((ThemeChoice::Dark, prefs.theme)),
            },
            theme: Theme::light(),
            styled_for: None,
            icons: icons::Icons::default(),
            gpu,
            drawn: None,
            preview_version: 0,
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
            configs: config_ui::ConfigUi::default(),
            solid: solid_ui::SolidUi::default(),
            sheet_metal: sheet_ui::SheetUi::default(),
            assembly: assembly_ui::AssemblyUi::default(),
            motion: motion_ui::MotionUi::default(),
            drawing: drawing_ui::DrawingUi::default(),
            construct: construct_ui::ConstructUi::default(),
            file: file_ui::FileUi::default(),
            browser: browser::BrowserUi::default(),
            timeline: timeline::TimelineUi::default(),
            search: search::SearchUi::default(),
            ribbon: ribbon::RibbonUi::default(),
            ctx_menu: context_menu::ContextMenuUi::default(),
            cam: cam_ui::CamUi::default(),
            display: display_ui::DisplayUi::default(),
            render: render_ui::RenderUi::default(),
            #[cfg(target_arch = "wasm32")]
            web: web::WebFiles::default(),
            ctx_close: false,
            zoom: None,
            frames: 0,
            select_box: None,
            box_preview: None,
            prefs,
            prefs_store: false,
            prefs_ui: prefs_ui::PrefsUi::default(),
            nav_drag: None,
            recent_seen: None,
            assistant: assistant_ui::AssistantUi::default(),
            history: history_ui::HistoryUi::default(),
            recovery: recovery_ui::RecoveryUi::default(),
            welcome: welcome_ui::WelcomeUi::default(),
            tutorial: tutorial_ui::TutorialUi::default(),
            #[cfg(not(target_arch = "wasm32"))]
            cam_worker: None,
        };
        app.session.history.limits = app.prefs.history_limits();
        if app.prefs.default_length != chamfera_doc::LengthUnit::Mm {
            let length = app.prefs.default_length.symbol();
            app.run("design.new", json!({ "length": length }));
        }
        app.sync_grid();
        if options.sample {
            app.run("body.sample", json!({}));
        }
        for f in &options.files {
            let path = f.to_string_lossy();
            app.run(file_ui::mesh_import_command(&path), json!({ "path": path }));
        }
        if !options.script.is_empty() {
            match chamfera_engine::script::run_steps(&mut app.session, &options.script) {
                Ok(replies) => {
                    app.inspect_from_script(&options.script, &replies);
                    app.sheet_from_script(&options.script, &replies);
                    app.history_from_script(&options.script, &replies);
                }
                Err(e) => app.message = Some((e.to_string(), true)),
            }
        }
        // A script that generated toolpaths opens in the Manufacture workspace.
        app.cam.active = app.session.toolpaths_visible() && app.session.active_sketch.is_none();
        // One that simulated the stock opens with the Simulate panel at that time.
        if let Some(sim) = app.session.simulation() {
            app.cam.active = true;
            let r = sim.report;
            app.cam.sim_panel = Some(cam_ui::SimPanel {
                time: sim.time,
                resolution: u32::try_from(sim.resolution).unwrap_or(256),
                result: Some(json!({
                    "removed_volume": r.removed_volume,
                    "rest_volume": r.rest_volume,
                    "gouges": {"cells": r.gouge_cells, "volume": r.gouge_volume,
                               "max_depth": r.max_gouge},
                    "holder": {"checked": sim.holder.checked, "moves": sim.holder.moves,
                               "cells": sim.holder.cells, "max_depth": sim.holder.max_depth,
                               "incomplete": sim.holder.incomplete},
                })),
            });
        }
        if app.session.active_sketch.is_some() {
            app.sketch.tool = Some(sketch_ui::Tool::Select);
            app.refresh_sketch_status();
        }
        app.params.open = options.parameters;
        app.configs.open = options.configurations;
        if options.tool_library {
            app.cam.active = true;
            app.cam.library_open = true;
        }
        app.drawing.active = options.drawing;
        if options.extrude {
            let mut tool = solid_ui::ExtrudeTool::default();
            if let Some(p) = &options.extrude_with {
                tool.apply(p);
            }
            app.solid.extrude = Some(tool);
            app.solid.pick_centre = true;
        }
        if options.hole {
            app.open_hole_on_last();
        }
        app.zoom = options
            .zoom
            .or(Some(app.prefs.ui_scale).filter(|z| (z - 1.0).abs() > f32::EPSILON));
        if let Some(page) = &options.preferences {
            app.prefs_ui.open = true;
            app.prefs_ui.page = prefs_ui::Page::parse(page).unwrap_or_default();
        }
        if let Some(text) = &options.sketch_input {
            match sketch_ui::parse_demo(text) {
                Ok(steps) => app.sketch.demo = steps,
                Err(e) => app.message = Some((e, true)),
            }
        }
        app.ctx_menu.open_at = options.context_menu;
        if let Some(demo) = options.press_pull {
            app.open_press_pull(press_pull_ui::PullMode::Auto);
            if let Some(t) = app.solid.press_pull.as_mut() {
                t.demo = Some(demo);
            }
        }
        app.box_preview = options.select_box;
        if let Some([x1, y1, x2, y2]) = options.drawing_drag {
            app.drawing_drag_preview(
                chamfera_geom::Point2::new(f64::from(x1), f64::from(y1)),
                chamfera_geom::Point2::new(f64::from(x2), f64::from(y2)),
            );
        }
        if let Some(q) = &options.search {
            app.run_action(shortcuts::Action::Search);
            app.search.query = q.chars().take(120).collect();
        }
        if options.assistant_demo {
            app.assistant_demo();
        }
        if options.history {
            app.open_history();
        }
        if options.render || options.render_window.is_some() {
            app.set_workspace(ribbon::Workspace::Render);
        }
        if let Some([w, h, samples]) = options.render_window {
            app.open_render_window();
            if let Some(win) = &mut app.render.window {
                win.width = w.to_string();
                win.height = h.to_string();
                win.samples = samples.to_string();
            }
            app.start_render();
        }
        if let Some(caption) = &options.ribbon_menu {
            let tab = app.ribbon_tab();
            app.ribbon.request = ribbon::groups(tab)
                .find(|g| g.caption.eq_ignore_ascii_case(caption))
                .map(|g| ribbon::MenuRequest::Group(g.caption));
        }
        if let Some(id) = &options.tutorial {
            app.start_tutorial(id);
        }
        app
    }

    /// Runs a command and shows its error, if any, in the status bar.
    fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.session.execute(id, &params) {
            Ok(v) => {
                self.tutorial_saw(id);
                Some(v)
            }
            Err(e) => {
                self.message = Some((format!("{}: {e}", label(id)), true));
                None
            }
        }
    }

    /// The design's display units (a copy).
    fn units(&self) -> chamfera_doc::DisplayUnits {
        self.session.doc.units
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
            .set_title("Insert Mesh")
            .add_filter("Mesh", &["stl", "STL", "obj", "OBJ", "3mf", "3MF"])
            .pick_files()
        else {
            return;
        };
        for f in files {
            let path = f.to_string_lossy();
            let command = file_ui::mesh_import_command(&path);
            if let Some(v) = self.run(command, json!({ "path": path })) {
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
        let keys = ui.input(|i| {
            (
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::H),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let pressed = ui.input(|i| i.pointer.any_down());
        let hover = response
            .hover_pos()
            .filter(|_| !pressed)
            .map(|p| (local(p).0, local(p).1, w, h));
        self.hover_update(hover);
        let drag = self.navigate_drag(ui, response, rect);
        self.navigate_zoom(ui, response, rect);
        if self.cam_picking()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.cam_click(x, y, w, h);
            return;
        }
        if self.press_pull_input(response, rect) {
            return;
        }
        if self.picking_extrude_target()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.extrude_target_click(x, y, w, h);
            return;
        }
        if self.construct.tool.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.construct_click(x, y, w, h);
            return;
        }
        if self.solid.pick_face
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.face_click(x, y, w, h);
            return;
        }
        if self.assembly.joint.is_some() {
            let at = response.hover_pos().map(local);
            self.joint_hover(at, w, h);
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let (x, y) = local(pos);
                self.joint_click(x, y, w, h);
                return;
            }
        }
        if self.drive_input(response, rect) {
            return;
        }
        if self.solid.hole.is_some() {
            let at = response.hover_pos().map(local);
            self.hole_hover(at, w, h);
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let (x, y) = local(pos);
                self.hole_click(x, y, w, h);
                return;
            }
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
        if self.solid.transform.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.transform_click(x, y, w, h);
            return;
        }
        if self.solid.inspect.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.inspect_click(x, y, w, h);
            return;
        }
        if self.solid.sweep.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.sweep_click(x, y, w, h);
            return;
        }
        if self.solid.shell.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.shell_click(x, y, w, h);
            return;
        }
        if self.sheet_tool_open() {
            let clicked = response
                .clicked()
                .then(|| response.interact_pointer_pos().map(local))
                .flatten();
            let at = clicked.or_else(|| response.hover_pos().map(local));
            if self.sheet_input(at, clicked.is_some(), w, h) {
                return;
            }
        }
        if self.solid.surface.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.surface_click(x, y, w, h);
            return;
        }
        if self.solid.body_tool.is_some()
            && response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (x, y) = local(pos);
            self.body_tool_click(x, y, w, h);
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
        if !used_by_sketch {
            self.context_click(response, rect);
        }
        if self.session.active_sketch.is_none() {
            // A drag the preset makes a selection (a left drag by default) draws a box.
            if response.drag_started_by(PointerButton::Primary)
                && drag == Some(navigation::Drag::Select)
                && self.select_box.is_none()
            {
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
                json!({ "x": x, "y": y, "width": w, "height": h, "mode": mode,
                        "filter": self.filter_param() }),
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
        // Tool and hover overlays are added for this frame's picture only.
        let base_overlays = self.session.scene.overlays.len();
        if let Some(preview) = self.sketch_preview() {
            self.session.scene.overlays.push(preview);
        }
        let blends = self.blend_overlays();
        self.session.scene.overlays.extend(blends);
        let sheet = self.sheet_overlays();
        self.session.scene.overlays.extend(sheet);
        let joints = self.joint_overlays();
        self.session.scene.overlays.extend(joints);
        let holes = self.hole_overlays();
        self.session.scene.overlays.extend(holes);
        let pulls = self.press_pull_overlays();
        self.session.scene.overlays.extend(pulls);
        self.inspect_overlays();
        if let Some(picked) = self.extrude_overlay() {
            self.session.scene.overlays.push(Overlay {
                style: chamfera_render::OverlayStyle::Highlight,
                ..picked
            });
        }
        if let Some(picked) = self.sweep_overlay() {
            self.session.scene.overlays.push(picked);
        }
        let picks = self.pick_overlays();
        self.session.scene.overlays.extend(picks);
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
            fills: self.session.scene.fills.clone(),
            display: self.session.scene.display,
            appearances: self
                .session
                .scene
                .bodies
                .iter()
                .map(|b| b.appearance)
                .collect(),
            theme: self.theme.mode,
            size,
            grid: self.session.scene.grid_spacing.map(f64::to_bits),
            previews: (self.preview_version, self.session.scene.previews.len()),
            compare: self
                .session
                .scene
                .compare
                .iter()
                .map(|c| (c.side, c.mesh.triangles.len()))
                .collect(),
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
        self.session.scene.overlays.truncate(base_overlays);
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
        self.inspect_labels(ui, rect);
        self.press_pull_labels(ui, rect);
        self.paint_box(ui, rect);
        self.view_cube(ui, rect);
        if !self.welcome.open {
            self.viewport_hint(ui, rect);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.cam_badge(ui, rect);
        self.selection_filter_bar(ui.ctx(), rect);
        self.display_bar(ui.ctx(), rect);
        if self.frames >= 2
            && let Some([x, y]) = self.ctx_menu.open_at.take()
        {
            self.open_context_menu(rect.min + Vec2::new(x, y), rect);
        }
        self.context_menu_window(ui.ctx());
    }

    /// `--screenshot`: wait a few frames for the view to settle, grab the window, save, quit.
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let rendering = self.render.busy();
        if rendering {
            // A render takes as long as it takes: the time-out counts from its end.
            self.shot_started = None;
        }
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
        // A request of the Assistant, a render, a version thumbnail being drawn, or
        // toolpaths still being made on the worker are part of the picture to wait for.
        if (self.assistant.running()
            || rendering
            || self.session.cam_regenerating()
            || self.history.thumbnails_pending(self.gpu.is_some())
            || self.welcome.thumbnails_pending(self.gpu.is_some()))
            && matches!(state, Shot::Waiting(_))
        {
            *state = Shot::Waiting(6);
            ctx.request_repaint();
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
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.sketch_raw_input(ctx, raw_input);
    }

    /// Runs every frame, even when the window is covered or minimised (then eframe skips
    /// `ui`). `--screenshot` lives here and asks for painting while hidden, so a covered
    /// window still draws, captures and quits instead of waiting forever.
    /// A clean quit: this run's recovery files go (a crash never gets here).
    fn on_exit(&mut self) {
        self.recovery_remove_own();
    }

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
        self.track_last_used();
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
            .show(ui, |ui| {
                if self.cam.active {
                    self.cam_tree(ui);
                } else if self.render.active && self.render.library {
                    self.render_library(ui);
                } else {
                    self.browser(ui);
                }
            });
        self.assistant_poll(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.cam_poll(&ctx);
        if self.assistant.open {
            egui::Panel::right("assistant")
                .frame(
                    egui::Frame::new()
                        .fill(color(self.theme.panel))
                        .inner_margin(egui::Margin::symmetric(
                            widgets::S3 as i8 + 2,
                            widgets::S3 as i8,
                        )),
                )
                .resizable(true)
                .default_size(360.0)
                .size_range(280.0..=640.0)
                .show(ui, |ui| self.assistant_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.drawing.active && self.session.active_sketch.is_none() {
                self.drawing_canvas(ui);
            } else {
                self.viewport(ui, frame);
            }
        });
        if self.frames < 2 {
            self.frames += 1;
            ctx.request_repaint();
        } else {
            self.parameters_window(&ctx);
            self.configurations_window(&ctx);
            self.extrude_dialog(&ctx);
            self.blend_dialog(&ctx);
            self.joint_dialog(&ctx);
            self.motion_panels(&ctx);
            self.hole_dialog(&ctx);
            self.transform_dialog(&ctx);
            self.inspect_dialog(&ctx);
            self.shell_dialog(&ctx);
            self.sheet_dialogs(&ctx);
            self.press_pull_dialog(&ctx);
            self.body_tool_dialog(&ctx);
            self.surface_dialog(&ctx);
            self.sweep_dialog(&ctx);
            self.feature_dialog(&ctx);
            self.timeline_error_window(&ctx);
            self.construct_dialog(&ctx);
            self.rename_dialog(&ctx);
            self.properties_window(&ctx);
            self.cam_dialogs(&ctx);
            self.appearance_window(&ctx);
            self.history_windows(&ctx);
            self.render_windows(&ctx);
            self.welcome_panel(&ctx);
            self.tutorial_overlay(&ctx);
        }
        if self.frames >= 2 {
            self.preferences_window(&ctx);
        }
        self.sync_grid();
        self.track_recent();
        self.rename_box(&ctx);
        self.unsaved_dialog(&ctx);
        if self.frames >= 2 {
            self.recovery_prompt(&ctx);
        }
        self.autosave_frame(&ctx);
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
    fn under_the_left_drag_orbit_preset_left_orbits_and_shift_left_draws_a_box() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let (mut app, a, _, px) = two_cubes(&ctx, &mut t);
        app.prefs.navigation.preset = navigation::Preset::LeftOrbit;
        let left = egui::PointerButton::Primary;
        let camera = app.session.camera;
        // Shift+left: a window from (-2, 12) to (35, -2) holds only cube A; Shift adds.
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(-2.0, 12.0), px(35.0, -2.0)),
            left,
            egui::Modifiers::SHIFT,
        );
        assert_eq!(selected(&app), vec![a]);
        assert_eq!(app.session.camera, camera, "a box does not move the view");
        // A plain left drag over the same pixels orbits and selects nothing new.
        app.run("select.clear", json!({}));
        drag(
            &mut app,
            &ctx,
            &mut t,
            (px(-2.0, 12.0), px(35.0, -2.0)),
            left,
            egui::Modifiers::NONE,
        );
        assert!(
            (app.session.camera.yaw - camera.yaw).abs() > 1e-3,
            "no orbit"
        );
        assert!(selected(&app).is_empty());
        assert!(app.select_box.is_none());
    }

    #[test]
    fn middle_and_right_drags_follow_the_preset() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let (mut app, _, _, px) = two_cubes(&ctx, &mut t);
        let path = (px(-2.0, 12.0), px(35.0, -2.0));
        let none = egui::Modifiers::NONE;
        // Default: middle pans (the target moves, the direction stays).
        let c = app.session.camera;
        drag(
            &mut app,
            &ctx,
            &mut t,
            path,
            egui::PointerButton::Middle,
            none,
        );
        assert!(
            (app.session.camera.target - c.target).length() > 1e-6,
            "no pan"
        );
        assert!((app.session.camera.yaw - c.yaw).abs() < 1e-12);
        // SolidWorks-style: middle orbits, Ctrl+middle pans.
        app.prefs.navigation.preset = navigation::Preset::SolidWorks;
        let c = app.session.camera;
        drag(
            &mut app,
            &ctx,
            &mut t,
            path,
            egui::PointerButton::Middle,
            none,
        );
        assert!((app.session.camera.yaw - c.yaw).abs() > 1e-3, "no orbit");
        let c = app.session.camera;
        drag(
            &mut app,
            &ctx,
            &mut t,
            path,
            egui::PointerButton::Middle,
            egui::Modifiers::CTRL,
        );
        assert!((app.session.camera.yaw - c.yaw).abs() < 1e-12);
        assert!(
            (app.session.camera.target - c.target).length() > 1e-6,
            "no pan"
        );
        // Fusion-style: a right drag leaves the view alone.
        app.prefs.navigation.preset = navigation::Preset::Fusion;
        let c = app.session.camera;
        drag(
            &mut app,
            &ctx,
            &mut t,
            path,
            egui::PointerButton::Secondary,
            none,
        );
        assert_eq!(app.session.camera, c);
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

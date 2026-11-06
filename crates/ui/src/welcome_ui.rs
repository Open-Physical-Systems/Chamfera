//! The welcome panel, shown over the viewport when the app starts with no design and no
//! script while Preferences > General > "Show the welcome screen at start" is on: New
//! Design, Open, the recent designs, a gallery of sample designs, the guided tours
//! (`tutorial_ui`), the user guide and the shortcut list. Help > Welcome Screen shows it
//! again.
//!
//! Samples are scripts from `examples/` built into the binary; opening one runs it in a new
//! design. Their thumbnails are drawn on first view by the snapshot renderer on another
//! thread, one at a time, and cached in the user's cache folder (desktop with a GPU only;
//! elsewhere a card shows an icon).

use crate::icons::Icon;
use crate::shortcuts::Action;
use crate::widgets::{self, S2, S3, S4, T_BODY, T_SMALL};
use crate::{ChamferaApp, Options};
use egui::{Rect, Sense, Vec2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// A sample design: a script from `examples/`.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// The example's file name without `.json`.
    pub key: &'static str,
    pub name: &'static str,
    /// What it shows, for the tooltip.
    pub what: &'static str,
    pub script: &'static str,
}

macro_rules! sample {
    ($key:literal, $name:literal, $what:literal) => {
        Sample {
            key: $key,
            name: $name,
            what: $what,
            script: include_str!(concat!("../../../examples/", $key, ".json")),
        }
    };
}

/// The gallery, in the order it shows them.
pub const SAMPLES: [Sample; 12] = [
    sample!(
        "hole-plate",
        "Hole plate",
        "A plate with simple, counterbored and countersunk holes"
    ),
    sample!(
        "configurations",
        "Configurations",
        "A plate with a hole in three sizes: Small, Medium and Large"
    ),
    sample!(
        "filleted-block",
        "Filleted block",
        "Fillets and chamfers on a block's edges"
    ),
    sample!(
        "shelled-enclosure",
        "Enclosure",
        "A box hollowed by Shell with its top open"
    ),
    sample!(
        "patterned-flange",
        "Flange",
        "A flange with a circular pattern of bolt holes"
    ),
    sample!(
        "revolved-parts",
        "Revolved parts",
        "Solids made by turning profiles about an axis"
    ),
    sample!(
        "swept-pipe-and-loft",
        "Pipe and loft",
        "A sweep along a bent path and a loft between profiles"
    ),
    sample!(
        "mirrored-bracket",
        "Mirrored bracket",
        "A bracket finished with a mirror of its features"
    ),
    sample!(
        "sheet-bracket",
        "Sheet metal bracket",
        "A base flange, flanges and the flat pattern"
    ),
    sample!(
        "hinge-assembly",
        "Hinge",
        "Two components joined by a revolute joint"
    ),
    sample!(
        "render-studio",
        "Render studio",
        "Brass, glass and rubber parts in the realistic style"
    ),
    sample!(
        "cam-plate",
        "CAM plate",
        "A pocketed plate with facing, contour, pocket and drilling toolpaths"
    ),
];

/// The user guide online.
pub const GUIDE_URL: &str =
    "https://github.com/Open-Physical-Systems/Chamfera/blob/dev/docs/user-guide/README.md";

/// Thumbnail size in pixels.
const THUMB_W: u32 = 192;
const THUMB_H: u32 = 120;
/// Recent designs listed.
const RECENT_SHOWN: usize = 5;

/// A thumbnail drawn on the worker thread: the sample and its PNG (`None`: it failed).
type Drawn = (&'static str, Option<Vec<u8>>);

#[derive(Default)]
// The thumbnail worker's fields are desktop-only.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct WelcomeUi {
    pub open: bool,
    textures: BTreeMap<&'static str, egui::TextureHandle>,
    /// Samples whose thumbnail was asked for, or failed, this run.
    tried: BTreeSet<&'static str>,
    failed: BTreeSet<&'static str>,
    /// Thumbnails drawn on the worker thread and not yet made textures.
    /// (`None`: it could not be drawn.)
    done: Arc<Mutex<Vec<Drawn>>>,
    busy: Arc<AtomicBool>,
}

impl WelcomeUi {
    /// The panel is open and a thumbnail is still to come (`--screenshot` waits).
    pub fn thumbnails_pending(&self, gpu: bool) -> bool {
        self.open
            && gpu
            && !cfg!(target_arch = "wasm32")
            && SAMPLES
                .iter()
                .any(|s| !self.textures.contains_key(s.key) && !self.failed.contains(s.key))
    }
}

/// Whether the app starts with the welcome panel: nothing given to open or run, and no
/// window asked for at start.
pub fn shows_at_start(options: &Options, show: bool) -> bool {
    show && options.script.is_empty()
        && options.files.is_empty()
        && !options.sample
        && !options.recovery_demo
        && !options.assistant_demo
        && !options.parameters
        && !options.configurations
        && !options.history
        && !options.drawing
        && !options.render
        && options.preferences.is_none()
        && options.search.is_none()
        && options.tutorial.is_none()
}

/// FNV-1a, for cache file names that change when a sample or the app changes.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The cache folder for thumbnails.
#[cfg(not(target_arch = "wasm32"))]
fn cache_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let var = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base = if cfg!(target_os = "windows") {
        var("LOCALAPPDATA")?.join("Chamfera").join("Cache")
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Caches/Chamfera")
    } else {
        var("XDG_CACHE_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".cache")))?
            .join("chamfera")
    };
    Some(base.join("samples"))
}

/// The thumbnail file of a sample in a theme.
#[cfg(not(target_arch = "wasm32"))]
fn thumb_file(s: &Sample, dark: bool) -> Option<std::path::PathBuf> {
    let key = format!("{}{}{dark}", s.script, env!("CARGO_PKG_VERSION"));
    Some(cache_dir()?.join(format!("{}-{:016x}.png", s.key, fnv(&key))))
}

/// Builds a sample in a fresh session and draws it from the iso view.
#[cfg(not(target_arch = "wasm32"))]
fn render_thumbnail(s: &Sample, theme: &chamfera_render::Theme) -> Option<Vec<u8>> {
    let steps: Vec<Value> = serde_json::from_str(s.script).ok()?;
    let mut session = chamfera_engine::Session::new();
    chamfera_engine::script::run_steps(&mut session, &steps).ok()?;
    let bounds = session.scene.bounds()?;
    let mut camera = chamfera_render::Camera::default();
    camera.set_view(chamfera_render::StandardView::Iso);
    camera.fit(&bounds, f64::from(THUMB_W) / f64::from(THUMB_H));
    let frame = chamfera_gpu::Frame {
        scene: &session.scene,
        camera: &camera,
        theme,
        width: THUMB_W,
        height: THUMB_H,
        scale: 1.0,
    };
    chamfera_gpu::snapshot_png(&frame).ok()
}

/// Opens a link in the system browser (or a new tab in the browser build).
fn open_link(ctx: &egui::Context, url: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        ctx.open_url(egui::OpenUrl::new_tab(url));
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ctx;
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else {
            std::process::Command::new("xdg-open")
        };
        cmd.arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open {url}: {e}"))
    }
}

/// The guide beside the app, when it runs from a source checkout: the folder to open.
#[cfg(not(target_arch = "wasm32"))]
fn local_guide() -> Option<std::path::PathBuf> {
    let mut starts = vec![std::env::current_dir().ok()?];
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    starts
        .iter()
        .flat_map(|p| p.ancestors().take(6))
        .map(|p| p.join("docs").join("user-guide"))
        .find(|p| p.join("README.md").is_file())
}

#[cfg(target_arch = "wasm32")]
fn local_guide() -> Option<std::path::PathBuf> {
    None
}

/// What the user picked in the panel, done once it is drawn.
enum Pick {
    New,
    Open,
    Recent(String),
    Sample(usize),
    Tutorial(String),
    Guide,
    Shortcuts,
    Close,
}

impl ChamferaApp {
    /// Opens a sample in a new design (after asking nothing: when the design has unsaved
    /// changes the user is told to save or start a new design first).
    pub(crate) fn open_sample(&mut self, i: usize) -> bool {
        let Some(s) = SAMPLES.get(i) else {
            return false;
        };
        if self.session.has_unsaved_changes() {
            self.message = Some((
                "Save the design or choose File > New first: a sample opens as a new design".into(),
                true,
            ));
            return false;
        }
        let steps: Vec<Value> = match serde_json::from_str(s.script) {
            Ok(v) => v,
            Err(e) => {
                self.message = Some((format!("{}: {e}", s.name), true));
                return false;
            }
        };
        let length = self.prefs.default_length.symbol();
        if self
            .run("design.new", json!({ "length": length }))
            .is_none()
        {
            return false;
        }
        self.sketch = crate::sketch_ui::SketchUi::default();
        self.solid = crate::solid_ui::SolidUi::default();
        if let Err(e) = chamfera_engine::script::run_steps(&mut self.session, &steps) {
            self.message = Some((format!("{}: {e}", s.name), true));
            return false;
        }
        if self.session.active_sketch.is_some() {
            self.sketch.tool = Some(crate::sketch_ui::Tool::Select);
            self.refresh_sketch_status();
        }
        self.run("view.standard", json!({ "view": "iso" }));
        self.fit_pending = true;
        self.welcome.open = false;
        self.message = Some((format!("Opened the sample {}", s.name), false));
        true
    }

    /// Help menu entries: the welcome panel and the tours.
    pub(crate) fn help_menu_welcome(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.separator();
        let mut pick = None;
        if widgets::menu_item(ui, look, Icon::New, "Welcome Screen", true).clicked() {
            pick = Some(None);
        }
        for t in crate::tutorial_ui::all() {
            let text = format!("Tutorial: {}", t.title);
            if widgets::menu_item(ui, look, Icon::Select, &text, true)
                .on_hover_text(&t.summary)
                .clicked()
            {
                pick = Some(Some(t.id));
            }
        }
        if widgets::menu_item(ui, look, Icon::Open, "User Guide", true).clicked() {
            ui.close();
            self.open_guide(ui.ctx());
        }
        match pick {
            Some(None) => {
                ui.close();
                self.welcome.open = true;
            }
            Some(Some(id)) => {
                ui.close();
                self.start_tutorial(&id);
            }
            None => {}
        }
    }

    fn open_guide(&mut self, ctx: &egui::Context) {
        let target = local_guide()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| GUIDE_URL.to_owned());
        match open_link(ctx, &target) {
            Ok(()) => self.message = Some((format!("Opening the user guide: {target}"), false)),
            Err(e) => self.message = Some((e, true)),
        }
    }

    /// Thumbnails: textures for cached ones, and one missing one drawn at a time.
    #[cfg(not(target_arch = "wasm32"))]
    fn welcome_thumbnails(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        let dark = self.theme.mode == chamfera_render::ThemeMode::Dark;
        let drawn: Vec<Drawn> = self
            .welcome
            .done
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default();
        for (key, png) in drawn {
            match png.as_deref().and_then(crate::history_ui::decode_png) {
                Some(img) => {
                    let t = ctx.load_texture(format!("sample {key}"), img, Default::default());
                    self.welcome.textures.insert(key, t);
                }
                None => {
                    self.welcome.failed.insert(key);
                }
            }
        }
        for s in &SAMPLES {
            if self.welcome.textures.contains_key(s.key) || self.welcome.failed.contains(s.key) {
                continue;
            }
            let file = thumb_file(s, dark);
            if let Some(img) = file
                .as_ref()
                .and_then(|f| std::fs::read(f).ok())
                .and_then(|b| crate::history_ui::decode_png(&b))
            {
                let t = ctx.load_texture(format!("sample {}", s.key), img, Default::default());
                self.welcome.textures.insert(s.key, t);
                continue;
            }
            if self.gpu.is_none()
                || self.welcome.tried.contains(s.key)
                || self.welcome.busy.load(Ordering::SeqCst)
            {
                continue;
            }
            self.welcome.tried.insert(s.key);
            let (sample, theme) = (*s, self.theme);
            let done = self.welcome.done.clone();
            let busy = self.welcome.busy.clone();
            busy.store(true, Ordering::SeqCst);
            let repaint = ctx.clone();
            let job = move || {
                let png = render_thumbnail(&sample, &theme);
                if let Some(png) = &png
                    && let Some(f) = &file
                    && let Some(dir) = f.parent()
                    && std::fs::create_dir_all(dir).is_ok()
                {
                    // A cache: a failed write only means drawing it again next time.
                    let _ = std::fs::write(f, png);
                }
                if let Ok(mut d) = done.lock() {
                    d.push((sample.key, png));
                }
                busy.store(false, Ordering::SeqCst);
                repaint.request_repaint();
            };
            let busy = self.welcome.busy.clone();
            if std::thread::Builder::new()
                .name("chamfera sample thumbnail".into())
                .spawn(job)
                .is_err()
            {
                busy.store(false, Ordering::SeqCst);
            }
            break;
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn welcome_thumbnails(&mut self, _ctx: &egui::Context) {}

    /// The welcome panel, centred over the viewport. It closes by itself once the design
    /// has a sketch or a body.
    pub(crate) fn welcome_panel(&mut self, ctx: &egui::Context) {
        if !self.welcome.open {
            return;
        }
        let empty = self.session.scene.bodies.is_empty()
            && self.session.doc.sketches.is_empty()
            && self.session.active_sketch.is_none();
        if !empty {
            self.welcome.open = false;
            return;
        }
        self.welcome_thumbnails(ctx);
        let look = self.look();
        let vr = self.viewport_rect;
        let recent: Vec<String> = self
            .prefs
            .recent_shown()
            .iter()
            .take(RECENT_SHOWN)
            .cloned()
            .collect();
        let tours = crate::tutorial_ui::all();
        let columns = if vr.width() > 900.0 { 4 } else { 3 };
        let card = Vec2::new(THUMB_W as f32 * 0.8, THUMB_H as f32 * 0.8);
        let mut show = self.prefs.show_welcome;
        let mut pick = None;
        egui::Area::new(egui::Id::unique("welcome panel"))
            .order(egui::Order::Middle)
            .fade_in(false)
            .fixed_pos(vr.center())
            .pivot(egui::Align2::CENTER_CENTER)
            .show(ctx, |ui| {
                widgets::dialog_frame(look).show(ui, |ui| {
                    if widgets::dialog_title(ui, look, Icon::New, "Welcome to Chamfera") {
                        pick = Some(Pick::Close);
                    }
                    widgets::dialog_body(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height((vr.height() - 150.0).max(200.0))
                            .show(ui, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(S4, S3);
                                    ui.vertical(|ui| {
                                        ui.set_width(210.0);
                                        self.welcome_actions(ui, &recent, &tours, &mut pick);
                                    });
                                    ui.vertical(|ui| {
                                        widgets::caption(ui, look, "SAMPLE DESIGNS");
                                        ui.add_space(S2);
                                        self.welcome_gallery(ui, columns, card, &mut pick);
                                    });
                                });
                            });
                    });
                    widgets::dialog_footer(ui, look, |ui| {
                        if widgets::primary_button(ui, look, "Close", true).clicked() {
                            pick = Some(Pick::Close);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.checkbox(&mut show, "Show at start")
                                .on_hover_text("Also in Preferences > General");
                        });
                    });
                });
            });
        if show != self.prefs.show_welcome {
            self.prefs.show_welcome = show;
            self.preferences_changed();
        }
        match pick {
            None => {}
            Some(Pick::Close) => self.welcome.open = false,
            Some(Pick::New) => {
                self.welcome.open = false;
                self.ask_or(crate::file_ui::Pending::New);
            }
            Some(Pick::Open) => self.ask_or(crate::file_ui::Pending::Open),
            Some(Pick::Recent(path)) => {
                self.ask_or(crate::file_ui::Pending::OpenPath(path.into()));
            }
            Some(Pick::Sample(i)) => {
                self.open_sample(i);
            }
            Some(Pick::Tutorial(id)) => {
                self.start_tutorial(&id);
            }
            Some(Pick::Guide) => self.open_guide(ctx),
            Some(Pick::Shortcuts) => self.run_action(Action::ShowShortcuts),
        }
    }

    /// The left column: start, recent designs and learning.
    fn welcome_actions(
        &self,
        ui: &mut egui::Ui,
        recent: &[String],
        tours: &[crate::tutorial_ui::Tutorial],
        pick: &mut Option<Pick>,
    ) {
        let look = self.look();
        widgets::caption(ui, look, "START");
        if widgets::menu_item(ui, look, Icon::New, "New Design", true).clicked() {
            *pick = Some(Pick::New);
        }
        if widgets::menu_item(ui, look, Icon::Open, "Open…", true).clicked() {
            *pick = Some(Pick::Open);
        }
        if !recent.is_empty() {
            ui.add_space(S2);
            widgets::caption(ui, look, "OPEN RECENT");
            for r in recent {
                let name = std::path::Path::new(r)
                    .file_name()
                    .map_or_else(|| r.clone(), |n| n.to_string_lossy().into_owned());
                if widgets::menu_item(ui, look, Icon::Body, &name, true)
                    .on_hover_text(r)
                    .clicked()
                {
                    *pick = Some(Pick::Recent(r.clone()));
                }
            }
        }
        ui.add_space(S2);
        widgets::caption(ui, look, "LEARN");
        for t in tours {
            if widgets::menu_item(ui, look, Icon::Select, &t.title, true)
                .on_hover_text(format!("Guided tour: {}", t.summary))
                .clicked()
            {
                *pick = Some(Pick::Tutorial(t.id.clone()));
            }
        }
        if widgets::menu_item(ui, look, Icon::Open, "User Guide", true)
            .on_hover_text("The guide beside the app, or online")
            .clicked()
        {
            *pick = Some(Pick::Guide);
        }
        if widgets::menu_item(ui, look, Icon::Select, "Keyboard Shortcuts", true).clicked() {
            *pick = Some(Pick::Shortcuts);
        }
    }

    /// The sample cards: thumbnail (or icon) and name.
    fn welcome_gallery(
        &self,
        ui: &mut egui::Ui,
        columns: usize,
        card: Vec2,
        pick: &mut Option<Pick>,
    ) {
        let look = self.look();
        egui::Grid::new("welcome samples")
            .spacing(egui::vec2(S3, S3))
            .show(ui, |ui| {
                for (i, s) in SAMPLES.iter().enumerate() {
                    let size = card + egui::vec2(0.0, 22.0);
                    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
                    let image = Rect::from_min_size(rect.min, card);
                    let fill = if r.hovered() {
                        look.c(|t| t.hover)
                    } else {
                        look.c(|t| t.panel_alt)
                    };
                    let painter = ui.painter();
                    painter.rect(
                        rect,
                        egui::CornerRadius::same(widgets::R_SMALL),
                        fill,
                        egui::Stroke::new(1.0, look.c(|t| t.border)),
                        egui::StrokeKind::Inside,
                    );
                    match self.welcome.textures.get(s.key) {
                        Some(t) => {
                            painter.image(
                                t.id(),
                                image.shrink(1.0),
                                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                        None => look.icon(
                            ui,
                            Icon::Body,
                            Rect::from_center_size(image.center(), Vec2::splat(32.0)),
                            false,
                        ),
                    }
                    painter.text(
                        egui::pos2(rect.min.x + S3, rect.max.y - 11.0),
                        egui::Align2::LEFT_CENTER,
                        s.name,
                        egui::FontId::proportional(T_SMALL),
                        look.c(|t| t.text),
                    );
                    if r.on_hover_text(format!("{}\n{}", s.name, s.what)).clicked() {
                        *pick = Some(Pick::Sample(i));
                    }
                    if (i + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(S2);
        ui.label(
            egui::RichText::new("Click a sample to open it as a new design.")
                .size(T_BODY - 1.0)
                .color(look.c(|t| t.text_muted)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App as _;

    /// Every text drawn in one frame of the real UI.
    fn texts(app: &mut ChamferaApp, ctx: &egui::Context, t: f64) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1400.0, 900.0),
            )),
            time: Some(t),
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_welcome_screen_shows_on_a_clean_start_and_not_with_a_script() {
        let clean = Options::default();
        assert!(shows_at_start(&clean, true));
        assert!(
            !shows_at_start(&clean, false),
            "the preference turns it off"
        );
        let scripted = Options {
            script: vec![json!({"command": "body.sample"})],
            ..Options::default()
        };
        assert!(!shows_at_start(&scripted, true));
        let sample = Options {
            sample: true,
            ..Options::default()
        };
        assert!(!shows_at_start(&sample, true));
        // A clean start draws the panel; the same start with a script does not.
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.welcome.open = shows_at_start(&clean, app.prefs.show_welcome);
        let mut seen = Vec::new();
        for k in 0..4 {
            seen = texts(&mut app, &ctx, f64::from(k) * 0.05);
        }
        for want in [
            "Welcome to Chamfera",
            "New Design",
            "Open…",
            "SAMPLE DESIGNS",
            "Hinge",
        ] {
            assert!(seen.iter().any(|t| t == want), "{want} not drawn");
        }
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.welcome.open = shows_at_start(&scripted, app.prefs.show_welcome);
        for k in 0..4 {
            seen = texts(&mut app, &ctx, f64::from(k) * 0.05);
        }
        assert!(!seen.iter().any(|t| t == "Welcome to Chamfera"));
    }

    #[test]
    fn every_sample_in_the_gallery_loads_and_builds() {
        for (i, s) in SAMPLES.iter().enumerate() {
            let mut app = ChamferaApp::headless();
            app.welcome.open = true;
            assert!(app.open_sample(i), "{}: {:?}", s.key, app.message);
            assert!(!app.welcome.open, "opening a sample closes the panel");
            let failed = app.session.feature_errors();
            assert!(failed.is_empty(), "{}: failed features {failed:?}", s.key);
            let sketches = app.session.sketch_errors();
            assert!(
                sketches.is_empty(),
                "{}: failed sketches {sketches:?}",
                s.key
            );
            assert!(
                !app.session.scene.bodies.is_empty(),
                "{}: the sample shows no body",
                s.key
            );
        }
    }

    #[test]
    fn a_sample_does_not_replace_unsaved_work() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        assert!(app.session.has_unsaved_changes());
        assert!(!app.open_sample(0));
        assert!(app.session.active_sketch.is_some(), "the work is kept");
        assert!(!app.open_sample(SAMPLES.len()), "no such sample");
    }

    #[test]
    fn the_welcome_panel_closes_once_the_design_has_a_body() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.welcome.open = true;
        texts(&mut app, &ctx, 0.0);
        app.run("body.sample", json!({}));
        for k in 1..4 {
            texts(&mut app, &ctx, f64::from(k) * 0.05);
        }
        assert!(!app.welcome.open);
    }

    #[test]
    fn sample_cache_names_change_with_the_script() {
        assert_ne!(fnv("a"), fnv("b"));
        assert_eq!(fnv(""), 0xcbf2_9ce4_8422_2325);
    }
}

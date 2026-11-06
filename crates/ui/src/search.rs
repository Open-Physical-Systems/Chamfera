//! The search palette: Ctrl+Space, then F, opens a floating box over the viewport that
//! finds any tool or command by name as you type. Up and Down choose, Enter runs, Esc
//! closes; recently used items come first.
//!
//! Running an item does what its button does (`run_action`): Extrude opens the Extrude
//! tool, Line picks the line tool in a sketch or says why it cannot. Commands that take
//! no parameters run directly; others open their tool or say where they are used.
//!
//! Also here: Help > Keyboard Shortcuts, which lists `shortcuts::SHORTCUTS`.

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::shortcuts::{self, Action, SHORTCUTS};
use crate::sketch_ui::{CREATE_ALL, MODIFY_TOOLS, PLANES, Tool};
use crate::widgets::{self, Look, S3, S4, T_BODY, T_SMALL};
use chamfera_engine::registry;
use egui::{Align2, CornerRadius, FontId, Key, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::json;

/// After Ctrl+Space is let go, F still opens the palette for this long (seconds).
const CHORD_SECS: f64 = 1.0;
/// Rows shown at once.
const ROWS: usize = 10;
/// Recently used items remembered.
const RECENT: usize = 8;
/// The longest query kept.
const QUERY_CHARS: usize = 120;

#[derive(Default)]
pub struct SearchUi {
    pub open: bool,
    pub query: String,
    /// The highlighted row, an index into the results.
    selected: usize,
    /// Most recent first.
    recent: Vec<Action>,
    /// When Ctrl+Space was last pressed (egui seconds), waiting for F.
    armed: Option<f64>,
    /// Give the query box the keyboard on the next draw.
    focus: bool,
    /// Help > Keyboard Shortcuts is open.
    pub shortcuts_open: bool,
}

/// One row of the palette.
#[derive(Debug, Clone)]
pub struct Entry {
    pub action: Action,
    pub label: String,
    pub icon: Icon,
    /// The workspace or menu it belongs to: Sketch, Solid, View, File, Edit, Command.
    pub context: &'static str,
    pub keys: Option<String>,
    /// Why it cannot run now, if it cannot.
    pub unavailable: Option<String>,
}

/// The command a UI action runs, so the palette does not list it twice.
pub(crate) fn command_of_action(action: Action) -> Option<&'static str> {
    Some(match action {
        Action::SketchTool(t) => match t {
            Tool::Select => return None,
            Tool::Line => "sketch.line",
            Tool::Rectangle => "sketch.rectangle",
            Tool::CentreRectangle => "sketch.rectangle.centre",
            Tool::Circle => "sketch.circle",
            Tool::Arc => "sketch.arc",
            Tool::ThreePointArc => "sketch.arc.three_point",
            Tool::TangentArc => "sketch.arc.tangent",
            Tool::Spline => "sketch.spline",
            Tool::Polygon => "sketch.polygon",
            Tool::Slot => "sketch.slot",
            Tool::Ellipse => "sketch.ellipse",
            Tool::Project => "sketch.project",
            Tool::Offset => "sketch.offset",
            Tool::Mirror => "sketch.mirror",
            Tool::RectangularPattern => "sketch.pattern.rectangular",
            Tool::CircularPattern => "sketch.pattern.circular",
            Tool::Fillet => "sketch.fillet",
            Tool::Chamfer => "sketch.chamfer",
            Tool::Trim => "sketch.trim",
            Tool::Extend => "sketch.extend",
            Tool::Split => "sketch.split",
        },
        Action::CreateSketch(_) => "sketch.create",
        Action::SketchOnFace => "sketch.create_on_face",
        Action::FinishSketch => "sketch.finish",
        Action::Extrude => "solid.extrude",
        Action::Revolve => "solid.revolve",
        Action::PressPull => "solid.press_pull",
        Action::Sweep => "solid.sweep",
        Action::Loft => "solid.loft",
        Action::Fillet => "solid.fillet",
        Action::Chamfer => "solid.chamfer",
        Action::Parameters => "param.set",
        Action::Dimension => "sketch.dimension_selection",
        Action::Construction => "sketch.construction",
        Action::Delete => "sketch.remove",
        Action::Undo => "edit.undo",
        Action::Redo => "edit.redo",
        Action::Fit => "view.fit",
        Action::Home => "view.home",
        Action::StandardView(_) => "view.standard",
        Action::ToggleProjection => "view.projection",
        Action::New => "design.new",
        Action::Open => "design.open",
        Action::Save => "design.save",
        Action::ExportStep => "design.export_step",
        Action::ExportStl => "design.export_stl",
        Action::ImportStl => "body.import_stl",
        Action::Sample => "body.sample",
        Action::Command(id) | Action::Tool(id) => id,
        Action::SaveAs
        | Action::Cancel
        | Action::ShowShortcuts
        | Action::Search
        | Action::Preferences
        | Action::Assistant => return None,
    })
}

/// The UI actions the palette lists, in its default order.
fn ui_actions() -> Vec<Action> {
    let mut v = vec![
        Action::Extrude,
        Action::Revolve,
        Action::PressPull,
        Action::Sweep,
        Action::Loft,
        Action::Fillet,
        Action::Chamfer,
    ];
    v.extend((0..PLANES.len()).map(Action::CreateSketch));
    v.push(Action::SketchOnFace);
    v.push(Action::Parameters);
    v.push(Action::SketchTool(Tool::Select));
    v.extend(CREATE_ALL.iter().map(|t| Action::SketchTool(*t)));
    v.extend(MODIFY_TOOLS.iter().map(|t| Action::SketchTool(*t)));
    v.extend([
        Action::Dimension,
        Action::Construction,
        Action::Delete,
        Action::FinishSketch,
        Action::Undo,
        Action::Redo,
        Action::Fit,
        Action::Home,
    ]);
    v.extend(
        ["front", "back", "left", "right", "top", "bottom", "iso"]
            .into_iter()
            .map(Action::StandardView),
    );
    v.extend([
        Action::ToggleProjection,
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::ExportStep,
        Action::ExportStl,
        Action::ImportStl,
        Action::Sample,
        Action::Preferences,
        Action::Assistant,
        Action::ShowShortcuts,
    ]);
    // Every ribbon tool, so the palette and the ribbon list the same tools.
    for t in crate::ribbon::TOOLS {
        if !v.contains(&t.action) {
            v.push(t.action);
        }
    }
    v
}

/// Every action the palette lists: the UI tools, then each registry command that has no
/// tool of its own.
fn all_actions() -> Vec<Action> {
    let mut v = ui_actions();
    let covered: Vec<&str> = v.iter().filter_map(|a| command_of_action(*a)).collect();
    let rest: Vec<Action> = registry()
        .into_iter()
        .filter(|c| !covered.contains(&c.id))
        .map(|c| Action::Command(c.id))
        .collect();
    v.extend(rest);
    v
}

fn context_of(action: Action) -> &'static str {
    match action {
        Action::SketchTool(_)
        | Action::Dimension
        | Action::Construction
        | Action::Delete
        | Action::FinishSketch => "Sketch",
        Action::CreateSketch(_)
        | Action::SketchOnFace
        | Action::Extrude
        | Action::Revolve
        | Action::PressPull
        | Action::Sweep
        | Action::Loft
        | Action::Fillet
        | Action::Chamfer
        | Action::Parameters => "Solid",
        Action::Fit
        | Action::Home
        | Action::StandardView(_)
        | Action::ToggleProjection
        | Action::Assistant => "View",
        Action::New
        | Action::Open
        | Action::Save
        | Action::SaveAs
        | Action::ExportStep
        | Action::ExportStl
        | Action::ImportStl
        | Action::Sample
        | Action::Preferences => "File",
        Action::Undo | Action::Redo | Action::Cancel => "Edit",
        Action::ShowShortcuts | Action::Search => "Help",
        Action::Tool(id) => crate::ribbon::by_id(id).map_or("Command", |t| t.tab.context()),
        Action::Command(id) => match id.split('.').next() {
            Some("sketch") => "Sketch",
            Some("solid" | "feature" | "param" | "surface") => "Solid",
            Some("view" | "select") => "View",
            Some("design" | "body" | "version") => "File",
            Some("edit") => "Edit",
            _ => "Command",
        },
    }
}

pub(crate) fn icon_of(action: Action) -> Icon {
    match action {
        Action::SketchTool(t) => t.icon(),
        Action::CreateSketch(_) | Action::SketchOnFace => Icon::SketchCreate,
        Action::FinishSketch => Icon::SketchFinish,
        Action::Extrude => Icon::Extrude,
        Action::Revolve => Icon::Revolve,
        Action::PressPull => Icon::PressPull,
        Action::Sweep => Icon::Sweep,
        Action::Loft => Icon::Loft,
        Action::Fillet => Icon::Fillet,
        Action::Chamfer => Icon::Chamfer,
        Action::Parameters => Icon::Parameters,
        Action::Dimension => Icon::Dimension,
        Action::Construction => Icon::Construction,
        Action::Delete => Icon::Delete,
        Action::Cancel => Icon::Close,
        Action::Undo => Icon::Undo,
        Action::Redo => Icon::Redo,
        Action::Fit => Icon::Fit,
        Action::Home => Icon::Home,
        Action::StandardView("top" | "bottom") => Icon::ViewTop,
        Action::StandardView("left" | "right") => Icon::ViewRight,
        Action::StandardView("iso") => Icon::ViewIso,
        Action::StandardView(_) => Icon::ViewFront,
        Action::ToggleProjection => Icon::ViewIso,
        Action::New => Icon::New,
        Action::Open => Icon::Open,
        Action::Save | Action::SaveAs => Icon::Save,
        Action::ExportStep | Action::ExportStl => Icon::Export,
        Action::ImportStl => Icon::ImportMesh,
        Action::Sample => Icon::Body,
        Action::Preferences => Icon::Parameters,
        Action::Assistant => Icon::Assistant,
        Action::ShowShortcuts | Action::Search => Icon::Select,
        Action::Tool(id) => crate::ribbon::by_id(id).map_or(Icon::Origin, |t| t.icon),
        Action::Command(id) => match id.split('.').next() {
            Some("surface") => crate::surface_ui::SurfaceKind::ALL
                .into_iter()
                .find(|k| k.command() == id)
                .map_or(Icon::SurfaceExtrude, |k| k.icon()),
            Some("sketch") => Icon::Sketch,
            Some("solid" | "feature" | "body") => Icon::Body,
            Some("param") => Icon::Parameters,
            Some("view") => Icon::ViewIso,
            Some("select") => Icon::Select,
            Some("design") => Icon::Save,
            Some("edit") => Icon::Undo,
            _ => Icon::Origin,
        },
    }
}

/// How well `query` matches `text`, higher is better; `None` when it does not match.
/// Case is ignored. In order: the whole text, a prefix, the start of a word, anywhere,
/// then the query's letters in order with gaps (a fuzzy match). A query of several words
/// also matches when every word matches on its own.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = text.to_lowercase();
    let whole = score_one(&q, &t);
    let words: Vec<&str> = q.split_whitespace().collect();
    let each = if words.len() > 1 {
        words
            .iter()
            .map(|w| score_one(w, &t))
            .try_fold(i32::MAX, |m, s| s.map(|s| m.min(s)))
            .map(|m| m - 50)
    } else {
        None
    };
    whole.max(each)
}

fn is_word_break(c: char) -> bool {
    !c.is_alphanumeric()
}

fn score_one(q: &str, t: &str) -> Option<i32> {
    let extra = i32::try_from(t.chars().count().saturating_sub(q.chars().count()))
        .unwrap_or(i32::MAX)
        .min(50);
    if t == q {
        return Some(1200);
    }
    if t.starts_with(q) {
        return Some(1000 - extra);
    }
    // The start of a later word.
    let mut prev_break = false;
    for (i, c) in t.char_indices() {
        if prev_break && t.get(i..).is_some_and(|rest| rest.starts_with(q)) {
            let at = i32::try_from(i).unwrap_or(i32::MAX).min(100);
            return Some(800 - at - extra);
        }
        prev_break = is_word_break(c);
    }
    if let Some(at) = t.find(q) {
        return Some(600 - i32::try_from(at).unwrap_or(i32::MAX).min(100) - extra);
    }
    // The letters in order: reward runs and word starts, charge for gaps.
    let tc: Vec<char> = t.chars().collect();
    let mut s = 300;
    let mut ti = 0usize;
    let mut last: Option<usize> = None;
    for qc in q.chars() {
        let found = tc.get(ti..)?.iter().position(|c| *c == qc)? + ti;
        let word_start = found == 0 || tc.get(found - 1).is_some_and(|c| is_word_break(*c));
        if word_start {
            s += 20;
        }
        match last {
            Some(l) if found == l + 1 => s += 15,
            Some(l) => s -= i32::try_from(found - l - 1).unwrap_or(100).min(30),
            None => s -= i32::try_from(found).unwrap_or(100).min(30),
        }
        last = Some(found);
        ti = found + 1;
    }
    Some(s.max(1))
}

impl ChamferaApp {
    fn has_profiles(&self) -> bool {
        self.session
            .doc
            .sketches
            .iter()
            .any(|s| !s.sketch.profiles().is_empty())
    }

    /// Why `action` cannot run now, in words, or `None` when it can.
    pub(crate) fn unavailable(&self, action: Action) -> Option<String> {
        if let Action::Tool(id) = action {
            return self.tool_unavailable(id);
        }
        let sketching = self.session.active_sketch.is_some();
        let tool_free = !self.tool_open();
        let has_solids = !self.session.solids().is_empty();
        let name = shortcuts::action_label(action);
        let desktop = cfg!(not(target_arch = "wasm32"));
        // The extrude preview needs what the Extrude tool needs (a profile, no open tool).
        if matches!(action, Action::Command("solid.extrude_preview")) {
            return self.unavailable(Action::Extrude);
        }
        let why = match action {
            Action::SketchTool(_) | Action::FinishSketch if !sketching => {
                format!("{name} works in a sketch: start one with Create Sketch first")
            }
            Action::Dimension | Action::Construction | Action::Delete if !sketching => {
                format!("{name} works in a sketch: start one with Create Sketch first")
            }
            Action::Dimension | Action::Construction | Action::Delete
                if self.session.sketch_selection.is_empty() =>
            {
                format!("{name}: select sketch geometry first")
            }
            Action::CreateSketch(_)
            | Action::SketchOnFace
            | Action::Extrude
            | Action::Revolve
            | Action::PressPull
            | Action::Sweep
            | Action::Loft
            | Action::Fillet
            | Action::Chamfer
                if sketching =>
            {
                format!("{name}: finish the sketch first")
            }
            Action::SketchOnFace | Action::Fillet | Action::Chamfer if !has_solids => {
                format!("{name}: build a solid first")
            }
            Action::Extrude | Action::Revolve | Action::Sweep | Action::Loft
                if !self.has_profiles() =>
            {
                format!("{name}: draw a closed profile in a sketch first")
            }
            Action::PressPull if !has_solids && !self.has_profiles() => {
                format!("{name}: draw a profile or build a solid first")
            }
            Action::Extrude
            | Action::Revolve
            | Action::PressPull
            | Action::Sweep
            | Action::Loft
            | Action::Fillet
            | Action::Chamfer
                if !tool_free =>
            {
                format!("{name}: close the open tool first")
            }
            Action::Undo if !self.session.can_undo() => "Nothing to undo".into(),
            Action::Redo if !self.session.can_redo() => "Nothing to redo".into(),
            Action::Open
            | Action::SaveAs
            | Action::ExportStep
            | Action::ExportStl
            | Action::ImportStl
                if !desktop =>
            {
                format!("{name} is in the desktop app")
            }
            _ => return None,
        };
        Some(why)
    }

    fn entry(&self, action: Action) -> Entry {
        Entry {
            action,
            label: shortcuts::action_label(action),
            icon: icon_of(action),
            context: context_of(action),
            keys: shortcuts::keys_for(action),
            unavailable: self.unavailable(action),
        }
    }

    /// The palette's rows for `query`, best first. Ranking adds to the match score: items
    /// that can run now, items of the workspace in use (Sketch while sketching, else
    /// Solid) and recently used items; commands with no tool of their own come later.
    pub(crate) fn search_results(&self, query: &str) -> Vec<Entry> {
        let workspace = self.ribbon_tab().context();
        let mut scored: Vec<(i32, Entry)> = Vec::new();
        for action in all_actions() {
            let e = self.entry(action);
            let by_id = command_of_action(action)
                .and_then(|id| score(query, id))
                .map(|s| s - 100);
            let Some(s) = score(query, &e.label).max(by_id) else {
                continue;
            };
            let mut rank = s;
            if e.unavailable.is_none() {
                rank += 100;
            }
            if e.context == workspace {
                rank += 60;
            }
            // Commands without a tool (mostly for scripts) come after the tools.
            if matches!(action, Action::Command(_)) {
                rank -= 80;
            }
            if let Some(i) = self.search.recent.iter().position(|r| *r == action) {
                let i = i32::try_from(i).unwrap_or(0);
                rank += if query.trim().is_empty() {
                    10_000 - i
                } else {
                    150 - 10 * i
                };
            }
            scored.push((rank, e));
        }
        // Stable: equal ranks keep the default order.
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().map(|(_, e)| e).collect()
    }

    fn open_search(&mut self) {
        self.search.open = true;
        self.search.query.clear();
        self.search.selected = 0;
        self.search.focus = true;
        self.search.armed = None;
    }

    fn close_search(&mut self) {
        self.search.open = false;
        self.search.query.clear();
        self.search.selected = 0;
    }

    /// Runs a palette row: remembers it, closes the palette and does it.
    fn run_from_search(&mut self, action: Action) {
        self.search.recent.retain(|a| *a != action);
        self.search.recent.insert(0, action);
        self.search.recent.truncate(RECENT);
        self.close_search();
        self.run_action(action);
    }

    /// Does what the action's button does, or says in the status bar why it cannot.
    pub(crate) fn run_action(&mut self, action: Action) {
        if let Some(why) = self.unavailable(action) {
            self.message = Some((why, false));
            return;
        }
        match action {
            Action::SketchTool(tool) => {
                self.sketch.tool = Some(tool);
                self.sketch.reset_tool();
            }
            Action::CreateSketch(i) => {
                if let Some((plane, _, view)) = PLANES.get(i) {
                    self.create_sketch(plane, view);
                }
            }
            Action::SketchOnFace => self.start_sketch_on_face(),
            Action::FinishSketch => self.finish_sketch(),
            Action::Extrude => self.open_extrude(false),
            Action::Revolve => self.open_extrude(true),
            Action::PressPull => self.open_press_pull(crate::press_pull_ui::PullMode::Auto),
            Action::Sweep => self.open_sweep(false),
            Action::Loft => self.open_sweep(true),
            Action::Fillet => self.open_blend(false),
            Action::Chamfer => self.open_blend(true),
            Action::Parameters => self.params.open = true,
            Action::Dimension => self.dimension_selected(),
            Action::Construction => self.toggle_construction(),
            Action::Delete => {
                self.run_sketch("sketch.remove", json!({}));
            }
            Action::Cancel => {}
            Action::Undo | Action::Redo => {
                let id = if action == Action::Undo {
                    "edit.undo"
                } else {
                    "edit.redo"
                };
                self.run(id, json!({}));
                self.refresh_sketch_status();
            }
            Action::Fit => {
                self.run("view.fit", self.viewport_params());
            }
            Action::Home => {
                self.run("view.home", self.viewport_params());
            }
            Action::StandardView(v) => self.standard_view(v),
            Action::ToggleProjection => {
                let ortho =
                    self.session.camera.projection == chamfera_render::Projection::Orthographic;
                let mode = if ortho { "perspective" } else { "orthographic" };
                self.run("view.projection", json!({ "mode": mode }));
            }
            Action::New => self.ask_or(crate::file_ui::Pending::New),
            Action::Open => self.ask_or(crate::file_ui::Pending::Open),
            Action::Save => self.save(false),
            Action::SaveAs => self.save(true),
            #[cfg(not(target_arch = "wasm32"))]
            Action::ExportStep => self.export("STEP", &["step", "stp"], "design.export_step"),
            #[cfg(not(target_arch = "wasm32"))]
            Action::ExportStl => self.export("STL mesh", &["stl"], "design.export_stl"),
            #[cfg(target_arch = "wasm32")]
            Action::ExportStep | Action::ExportStl => {}
            Action::ImportStl => self.import_dialog(),
            Action::Sample => {
                self.run("body.sample", json!({}));
                self.fit_pending = true;
            }
            Action::ShowShortcuts => self.search.shortcuts_open = true,
            Action::Preferences => self.prefs_ui.open = true,
            Action::Assistant => self.assistant.open = !self.assistant.open,
            Action::Search => self.open_search(),
            Action::Command(id) => self.run_command_item(id),
            Action::Tool(id) => self.run_tool(id),
        }
    }

    /// A registry command with no tool of its own: open the tool it belongs to, run it
    /// when it takes no parameters, or say where it is used.
    fn run_command_item(&mut self, id: &'static str) {
        if id.starts_with("param.") {
            self.params.open = true;
            return;
        }
        if id.starts_with("version.") {
            self.history_command(id);
            return;
        }
        let Some(spec) = registry().into_iter().find(|c| c.id == id) else {
            return;
        };
        let where_used = match id {
            "sketch.dimension" => Some("click a dimension's value in the sketch to change it"),
            "sketch.constrain" | "sketch.constrain_selection" => {
                Some("select sketch geometry, then use the CONSTRAINTS buttons")
            }
            "sketch.edit" | "sketch.delete" | "feature.edit" | "feature.delete" => {
                Some("use the Browser or the timeline")
            }
            "view.orbit" => Some("right-drag in the viewport, or Shift and middle-drag"),
            "view.pan" => Some("middle-drag in the viewport"),
            "view.zoom" => Some("turn the mouse wheel over the viewport"),
            "select.pick" | "sketch.select" | "sketch.pick" => Some("click in the viewport"),
            _ => None,
        };
        if let Some(how) = where_used {
            self.message = Some((format!("{}: {how}", spec.label), false));
            return;
        }
        if spec.params.starts_with("(none)") {
            if let Some(v) = self.run(id, json!({})) {
                let mut text = v.to_string();
                if text.chars().count() > 100 {
                    text = text.chars().take(100).collect::<String>() + "…";
                }
                self.message = Some((format!("{}: {text}", spec.label), false));
            }
            if id.starts_with("sketch.") || id.starts_with("edit.") {
                self.refresh_sketch_status();
            }
            return;
        }
        self.message = Some((
            format!(
                "{} takes parameters: run it from a script or `chamfera-cli exec {id}`",
                spec.label
            ),
            false,
        ));
    }

    /// Keys for the palette, read before anything else this frame and taken so nothing
    /// else sees them: Ctrl+Space then F opens it; while it is open, Up, Down, Enter, Esc.
    pub(crate) fn search_keys(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.search.open {
            let (mut up, mut down, mut enter, mut esc) = (0usize, 0usize, false, false);
            ctx.input_mut(|i| {
                i.events.retain(|e| match e {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => match key {
                        Key::ArrowUp => {
                            up += 1;
                            false
                        }
                        Key::ArrowDown | Key::Tab => {
                            down += 1;
                            false
                        }
                        Key::Enter => {
                            enter = true;
                            false
                        }
                        Key::Escape => {
                            esc = true;
                            false
                        }
                        _ => true,
                    },
                    egui::Event::Key {
                        key: Key::ArrowUp | Key::ArrowDown | Key::Tab | Key::Enter | Key::Escape,
                        ..
                    } => false,
                    _ => true,
                });
            });
            if esc {
                self.close_search();
                return;
            }
            let results = self.search_results(&self.search.query.clone());
            let n = results.len();
            if n > 0 {
                // Down and Up move one row each, wrapping at the ends.
                let mut s = self.search.selected.min(n - 1);
                for _ in 0..down {
                    s = (s + 1) % n;
                }
                for _ in 0..up {
                    s = (s + n - 1) % n;
                }
                self.search.selected = s;
            }
            if enter && let Some(e) = results.get(self.search.selected) {
                let action = e.action;
                self.run_from_search(action);
            }
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            self.search.armed = None;
            return;
        }
        let mut space = false;
        let mut f = false;
        let mut other = false;
        let ctrl_now = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
        ctx.input_mut(|i| {
            let held = |m: &egui::Modifiers| m.ctrl || m.command;
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    if (*key == Key::Space || *physical_key == Some(Key::Space)) && held(modifiers)
                    {
                        space = true;
                    } else if *key == Key::F {
                        f = true;
                    } else if *key != Key::Space {
                        other = true;
                    }
                }
            }
            // Some platforms report Ctrl+Space only as a held key, not as a press.
            if ctrl_now && i.key_down(Key::Space) {
                space = true;
            }
        });
        if space {
            self.search.armed = Some(now);
            // Ctrl+Space belongs to the palette: no focused button takes it.
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: Key::Space, modifiers, .. }
                        if modifiers.ctrl || modifiers.command)
                });
            });
        }
        let Some(at) = self.search.armed else {
            return;
        };
        if f && (ctrl_now || now - at <= CHORD_SECS) {
            // Take the F so it does not also fit the view, and its letter so it does not
            // land in the query box.
            ctx.input_mut(|i| {
                i.events.retain(|e| match e {
                    egui::Event::Key { key: Key::F, .. } => false,
                    egui::Event::Text(t) => !t.eq_ignore_ascii_case("f"),
                    _ => true,
                });
            });
            self.open_search();
        } else if other || (!ctrl_now && now - at > CHORD_SECS) {
            self.search.armed = None;
        }
    }

    /// The palette itself: a query box and the best rows, centred near the top of the
    /// viewport.
    pub(crate) fn search_window(&mut self, ctx: &egui::Context) {
        if !self.search.open {
            return;
        }
        let results = self.search_results(&self.search.query.clone());
        let n = results.len();
        self.search.selected = self.search.selected.min(n.saturating_sub(1));
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let vr = self.viewport_rect;
        let width = (vr.width() - 32.0).clamp(280.0, 520.0);
        let pos = egui::pos2(vr.center().x - width * 0.5, vr.min.y + 56.0);
        let search = &mut self.search;
        let first = search.selected.saturating_sub(ROWS - 1);
        let mut run: Option<Action> = None;
        let area = egui::Area::new(egui::Id::unique("search palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                widgets::dialog_frame(look).show(ui, |ui| {
                    ui.set_width(width);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(S4 as i8, S3 as i8 + 2))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = S3;
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                                look.icon(ui, Icon::Select, r, true);
                                let edit = ui.add(
                                    egui::TextEdit::singleline(&mut search.query)
                                        .id(egui::Id::unique("search query"))
                                        .hint_text("Search tools and commands")
                                        .char_limit(QUERY_CHARS)
                                        .frame(egui::Frame::NONE)
                                        .font(FontId::proportional(widgets::T_TITLE))
                                        .desired_width(f32::INFINITY),
                                );
                                if search.focus {
                                    edit.request_focus();
                                    search.focus = false;
                                }
                                if edit.changed() {
                                    search.selected = 0;
                                }
                            });
                        });
                    let top = ui.min_rect().max.y;
                    ui.painter().hline(
                        ui.min_rect().x_range(),
                        top.round() + 0.5,
                        Stroke::new(1.0, look.c(|t| t.border)),
                    );
                    ui.add_space(4.0);
                    if results.is_empty() {
                        ui.add_space(S3);
                        ui.horizontal(|ui| {
                            ui.add_space(S4);
                            ui.label(
                                egui::RichText::new("Nothing matches")
                                    .size(T_BODY)
                                    .color(look.c(|t| t.text_muted)),
                            );
                        });
                        ui.add_space(S3);
                    }
                    for (k, e) in results.iter().enumerate().skip(first).take(ROWS) {
                        if palette_row(ui, look, width, e, k == search.selected).clicked() {
                            run = Some(e.action);
                        }
                    }
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(look.c(|t| t.chrome))
                        .corner_radius(CornerRadius {
                            nw: 0,
                            ne: 0,
                            sw: widgets::R_LARGE,
                            se: widgets::R_LARGE,
                        })
                        .inner_margin(egui::Margin::symmetric(S4 as i8, 6))
                        .show(ui, |ui| {
                            ui.set_width(width - 2.0 * S4);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{n} found · Up and Down choose · Enter runs · Esc closes"
                                ))
                                .size(T_SMALL)
                                .color(look.c(|t| t.text_muted)),
                            );
                        });
                });
            });
        let outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !area.response.rect.contains(p))
        });
        if let Some(action) = run {
            self.run_from_search(action);
        } else if outside {
            self.close_search();
        }
    }

    /// The Help menu.
    pub(crate) fn help_menu(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.set_min_width(260.0);
        let mut pick = None;
        for action in [Action::Search, Action::ShowShortcuts] {
            let r = widgets::menu_item(
                ui,
                look,
                icon_of(action),
                &shortcuts::action_label(action),
                true,
            );
            if let Some(keys) = shortcuts::keys_for(action) {
                ui.painter().text(
                    egui::pos2(r.rect.max.x - 8.0, r.rect.center().y),
                    Align2::RIGHT_CENTER,
                    keys,
                    FontId::proportional(T_SMALL),
                    look.c(|t| t.text_muted),
                );
            }
            if r.clicked() {
                pick = Some(action);
            }
        }
        ui.separator();
        let about = widgets::menu_item(ui, look, Icon::Body, "About Chamfera", true);
        #[cfg(target_arch = "wasm32")]
        self.web.target("About Chamfera", about.rect);
        if about.clicked() {
            ui.close();
            self.about.open = true;
        }
        if let Some(a) = pick {
            ui.close();
            self.run_action(a);
        }
        self.help_menu_welcome(ui);
    }

    /// Help > Keyboard Shortcuts: every shortcut, its action and where it works.
    pub(crate) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.search.shortcuts_open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let vr = self.viewport_rect;
        let mut close = false;
        egui::Window::new("Keyboard Shortcuts")
            .id(egui::Id::unique("shortcuts window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(egui::pos2(vr.center().x - 210.0, vr.min.y + 40.0))
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                if widgets::dialog_title(ui, look, Icon::Select, "Keyboard Shortcuts") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("shortcuts grid")
                        .num_columns(3)
                        .spacing(vec2(S4, 6.0))
                        .show(ui, |ui| {
                            let row = |ui: &mut egui::Ui, keys: &str, what: &str, ctx: &str| {
                                ui.label(
                                    egui::RichText::new(keys)
                                        .size(T_BODY)
                                        .strong()
                                        .color(look.c(|t| t.text)),
                                );
                                ui.label(
                                    egui::RichText::new(what)
                                        .size(T_BODY)
                                        .color(look.c(|t| t.text)),
                                );
                                ui.label(
                                    egui::RichText::new(ctx)
                                        .size(T_SMALL)
                                        .color(look.c(|t| t.text_muted)),
                                );
                                ui.end_row();
                            };
                            for s in SHORTCUTS {
                                row(
                                    ui,
                                    &s.keys.text(),
                                    &shortcuts::action_label(s.action),
                                    s.context,
                                );
                            }
                            row(ui, "Up / Down", "Choose a search result", "Search");
                            row(ui, "Enter", "Run the chosen result", "Search");
                            for (key, what) in shortcuts::RESERVED {
                                row(ui, key.name(), what, "Kept free");
                            }
                        });
                    ui.add_space(S3);
                    ui.label(
                        egui::RichText::new("Shortcuts do nothing while you type in a text box.")
                            .size(T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                });
            });
        let esc = ctx.input(|i| i.key_pressed(Key::Escape)) && !ctx.egui_wants_keyboard_input();
        if close || esc {
            self.search.shortcuts_open = false;
        }
    }
}

/// One palette row: icon, label (muted when it cannot run now), then its workspace and
/// shortcut on the right.
fn palette_row(
    ui: &mut egui::Ui,
    look: Look,
    width: f32,
    e: &Entry,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let row = rect.shrink2(vec2(6.0, 1.0));
    if selected {
        ui.painter().rect_filled(
            row,
            CornerRadius::same(widgets::R_SMALL),
            look.c(|t| t.selected),
        );
    } else if response.hovered() {
        ui.painter().rect_filled(
            row,
            CornerRadius::same(widgets::R_SMALL),
            look.c(|t| t.hover),
        );
    }
    let can = e.unavailable.is_none();
    let icon = Rect::from_center_size(
        egui::pos2(row.min.x + 18.0, row.center().y),
        Vec2::splat(18.0),
    );
    look.icon(ui, e.icon, icon, can);
    let text = if can {
        look.c(|t| t.text)
    } else {
        look.c(|t| t.text_muted)
    };
    ui.painter().text(
        egui::pos2(row.min.x + 38.0, row.center().y),
        Align2::LEFT_CENTER,
        &e.label,
        FontId::proportional(T_BODY),
        text,
    );
    let mut right = row.max.x - 10.0;
    if let Some(keys) = &e.keys {
        let g = ui.painter().layout_no_wrap(
            keys.clone(),
            FontId::proportional(T_SMALL),
            look.c(|t| t.text),
        );
        let chip = Rect::from_min_size(
            egui::pos2(right - g.size().x - 10.0, row.center().y - 9.0),
            vec2(g.size().x + 10.0, 18.0),
        );
        ui.painter().rect(
            chip,
            CornerRadius::same(widgets::R_SMALL),
            look.c(|t| t.panel_alt),
            Stroke::new(1.0, look.c(|t| t.border)),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .galley(chip.center() - g.size() * 0.5, g, look.c(|t| t.text));
        right = chip.min.x - 10.0;
    }
    ui.painter().text(
        egui::pos2(right, row.center().y),
        Align2::RIGHT_CENTER,
        e.context,
        FontId::proportional(T_SMALL),
        look.c(|t| t.text_muted),
    );
    match &e.unavailable {
        Some(why) => response.on_hover_text(why.as_str()),
        None => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    fn top(app: &ChamferaApp, query: &str) -> String {
        app.search_results(query)
            .first()
            .map(|e| e.label.clone())
            .unwrap_or_default()
    }

    #[test]
    fn ext_ranks_extrude_first_in_the_solid_workspace() {
        // "ext" is a prefix of both Extrude and Extend (and Extend is shorter); Extrude
        // wins because it belongs to the workspace in use (Solid, no sketch open).
        let app = ChamferaApp::headless();
        assert_eq!(top(&app, "ext"), "Extrude");
        assert_eq!(top(&app, "EXT"), "Extrude");
    }

    #[test]
    fn ext_ranks_extend_first_while_sketching() {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        assert_eq!(top(&app, "ext"), "Extend");
    }

    #[test]
    fn fil_finds_fillet() {
        let app = ChamferaApp::headless();
        let r = app.search_results("fil");
        assert_eq!(r.first().map(|e| e.label.as_str()), Some("Fillet"));
        assert!(r.iter().any(|e| e.label == "Sketch Fillet"));
    }

    #[test]
    fn hand_picked_queries_find_their_tool_first() {
        let app = ChamferaApp::headless();
        for (q, want) in [
            ("param", "Parameters"),
            ("export step", "Export STEP…"),
            ("step exp", "Export STEP…"),
            ("trim", "Trim"),
            ("undo", "Undo"),
            ("shortcuts", "Keyboard Shortcuts"),
        ] {
            assert_eq!(top(&app, q), want, "query {q:?}");
        }
        // A fuzzy match: the letters of "rvl" in order are only in Revolve.
        assert_eq!(top(&app, "rvl"), "Revolve");
        assert!(app.search_results("zzzq").is_empty());
    }

    #[test]
    fn score_orders_exact_prefix_word_inside_and_fuzzy() {
        let s = |q, t| score(q, t).unwrap_or(-1);
        assert!(s("line", "Line") > s("line", "Lines"));
        assert!(s("lin", "Line") > s("lin", "Sketch Line"));
        assert!(s("lin", "Sketch Line") > s("lin", "Spline"));
        assert!(s("lin", "Spline") > s("lne", "Line"));
        assert!(s("lne", "Line") > 0);
        assert_eq!(score("enl", "Line"), None);
        // Hostile input does not panic.
        let long = "x".repeat(10_000);
        let _ = score(&long, "Extrude");
        let _ = score("é∂ƒ\u{0}", "Extrude ünïcode");
    }

    #[test]
    fn every_registry_command_is_in_the_palette() {
        let listed: Vec<&str> = all_actions()
            .into_iter()
            .filter_map(command_of_action)
            .collect();
        for c in registry() {
            assert!(listed.contains(&c.id), "{} is missing", c.id);
        }
    }

    #[test]
    fn running_every_item_never_panics() {
        let mut app = ChamferaApp::headless();
        for a in all_actions() {
            if matches!(
                a,
                Action::Open
                    | Action::SaveAs
                    | Action::ExportStep
                    | Action::ExportStl
                    | Action::ImportStl
                    | Action::Save
            ) {
                continue; // these open file dialogs
            }
            app.run_action(a);
        }
    }

    #[test]
    fn line_outside_a_sketch_says_why() {
        let mut app = ChamferaApp::headless();
        app.run_action(Action::SketchTool(Tool::Line));
        let (m, _) = app.message.clone().unwrap_or_default();
        assert!(m.contains("sketch"), "{m}");
        assert!(app.session.active_sketch.is_none());
    }

    // --- Real key events through the whole UI -------------------------------------

    fn ctrl() -> egui::Modifiers {
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        }
    }

    fn key(key: Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    /// Runs one frame of the real UI with these modifiers held and these events.
    fn frame(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        modifiers: egui::Modifiers,
        events: Vec<egui::Event>,
    ) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1400.0, 900.0),
            )),
            time: Some(*t),
            events: std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect(),
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
        *t += 0.05;
    }

    fn settled() -> (ChamferaApp, egui::Context, f64) {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        let mut t = 0.0;
        for _ in 0..3 {
            frame(&mut app, &ctx, &mut t, egui::Modifiers::NONE, vec![]);
        }
        (app, ctx, t)
    }

    /// A 40 x 20 rectangle on XY, finished: a closed profile to extrude.
    fn with_profile(app: &mut ChamferaApp) {
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0.0, "y1": 0.0, "x2": 40.0, "y2": 20.0 }),
        );
        app.run("sketch.finish", json!({}));
    }

    fn type_text(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64, text: &str) {
        frame(
            app,
            ctx,
            t,
            egui::Modifiers::NONE,
            vec![egui::Event::Text(text.into())],
        );
    }

    #[test]
    fn ctrl_space_f_with_ctrl_held_opens_the_palette_and_does_not_fit() {
        let (mut app, ctx, mut t) = settled();
        app.run("body.sample", json!({}));
        app.run("view.zoom", json!({ "factor": 3.0 }));
        let before = app.session.camera;
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::Space, true, ctrl())],
        );
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::F, true, ctrl())],
        );
        assert!(app.search.open, "the palette did not open");
        assert_eq!(app.session.camera, before, "F also fitted the view");
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::F, false, ctrl()), key(Key::Space, false, ctrl())],
        );
        assert!(app.search.open);
        assert!(app.search.query.is_empty(), "the F landed in the query");
    }

    #[test]
    fn ctrl_space_and_f_in_the_same_frame_open_the_palette() {
        // A fast typist's Space and F can arrive together; F still counts.
        let (mut app, ctx, mut t) = settled();
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::Space, true, ctrl()), key(Key::F, true, ctrl())],
        );
        assert!(app.search.open);
    }

    #[test]
    fn f_within_a_second_after_ctrl_space_opens_it_and_later_f_does_not() {
        let (mut app, ctx, mut t) = settled();
        let none = egui::Modifiers::NONE;
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::Space, true, ctrl())],
        );
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::Space, false, none)],
        );
        t += 0.4;
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::F, true, none), egui::Event::Text("f".into())],
        );
        assert!(app.search.open, "F 0.45 s after Ctrl+Space did not open it");
        assert!(app.search.query.is_empty());

        let (mut app, ctx, mut t) = settled();
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::Space, true, ctrl())],
        );
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::Space, false, none)],
        );
        t += 1.5;
        frame(&mut app, &ctx, &mut t, none, vec![key(Key::F, true, none)]);
        assert!(!app.search.open, "F 1.5 s later still opened it");
    }

    #[test]
    fn typing_ext_and_enter_opens_the_extrude_tool() {
        let (mut app, ctx, mut t) = settled();
        with_profile(&mut app);
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::Space, true, ctrl()), key(Key::F, true, ctrl())],
        );
        frame(&mut app, &ctx, &mut t, egui::Modifiers::NONE, vec![]);
        type_text(&mut app, &ctx, &mut t, "ext");
        assert_eq!(app.search.query, "ext");
        frame(
            &mut app,
            &ctx,
            &mut t,
            egui::Modifiers::NONE,
            vec![key(Key::Enter, true, egui::Modifiers::NONE)],
        );
        assert!(!app.search.open, "Enter did not close the palette");
        assert!(
            app.solid.extrude.as_ref().is_some_and(|x| !x.revolve),
            "the Extrude tool is not open"
        );
        // It is remembered: an empty palette lists it first.
        assert_eq!(top(&app, ""), "Extrude");
    }

    #[test]
    fn down_then_enter_runs_the_second_hit_and_esc_closes() {
        let (mut app, ctx, mut t) = settled();
        app.create_sketch("xy", "top");
        let none = egui::Modifiers::NONE;
        app.run_action(Action::Search);
        frame(&mut app, &ctx, &mut t, none, vec![]);
        type_text(&mut app, &ctx, &mut t, "rectangle");
        let second = app.search_results("rectangle").get(1).map(|e| e.action);
        assert_eq!(second, Some(Action::SketchTool(Tool::CentreRectangle)));
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::ArrowDown, true, none)],
        );
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::Enter, true, none)],
        );
        assert_eq!(app.sketch.tool, Some(Tool::CentreRectangle));

        app.run_action(Action::Search);
        frame(&mut app, &ctx, &mut t, none, vec![]);
        assert!(app.search.open);
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::Escape, true, none)],
        );
        assert!(!app.search.open);
        assert!(
            app.session.active_sketch.is_some(),
            "Esc reached the sketch too"
        );
    }

    #[test]
    fn single_key_shortcuts_pick_tools_but_not_while_typing() {
        let (mut app, ctx, mut t) = settled();
        app.create_sketch("xy", "top");
        let none = egui::Modifiers::NONE;
        frame(&mut app, &ctx, &mut t, none, vec![key(Key::C, true, none)]);
        assert_eq!(app.sketch.tool, Some(Tool::Circle));
        // Ctrl+C is not C.
        frame(
            &mut app,
            &ctx,
            &mut t,
            ctrl(),
            vec![key(Key::R, true, ctrl())],
        );
        assert_eq!(app.sketch.tool, Some(Tool::Circle));
        // With the palette's text box focused, R is a letter, not the Rectangle tool.
        app.run_action(Action::Search);
        frame(&mut app, &ctx, &mut t, none, vec![]);
        frame(
            &mut app,
            &ctx,
            &mut t,
            none,
            vec![key(Key::R, true, none), egui::Event::Text("r".into())],
        );
        assert_eq!(app.sketch.tool, Some(Tool::Circle));
        assert_eq!(app.search.query, "r");
    }

    #[test]
    fn e_opens_extrude_and_l_outside_a_sketch_explains() {
        let (mut app, ctx, mut t) = settled();
        with_profile(&mut app);
        let none = egui::Modifiers::NONE;
        frame(&mut app, &ctx, &mut t, none, vec![key(Key::L, true, none)]);
        assert!(app.session.active_sketch.is_none());
        assert!(
            app.message
                .as_ref()
                .is_some_and(|(m, _)| m.contains("sketch"))
        );
        frame(&mut app, &ctx, &mut t, none, vec![key(Key::E, true, none)]);
        assert!(app.solid.extrude.is_some());
    }
}

//! Autosave and crash recovery.
//!
//! While the design has unsaved changes, every `autosave_secs` (a preference, 60 s by
//! default) it is written to a recovery file in the user's data folder
//! (`chamfera_io::recovery`, atomically). A save or a clean quit removes the file; a
//! crash leaves it. At start the files other runs left behind are offered: "Recover
//! unsaved work from <time>?" with Recover, Show Later and Discard. Work saved after its
//! recovery file was written, and files of a run still going, are not offered.
//!
//! The status bar shows the autosave state. Screenshots (`--screenshot`) and tests never
//! read or write recovery files unless they set `dir` (`--recovery-demo`).

use crate::icons::Icon;
use crate::widgets;
use crate::{ChamferaApp, color};
use chamfera_io::recovery::{self, Entry};
use egui::Rect;
use serde_json::json;
use std::path::PathBuf;

/// The user's recovery folder: `$XDG_DATA_HOME/chamfera/recovery` or
/// `~/.local/share/chamfera/recovery` on Linux, `~/Library/Application
/// Support/Chamfera/Recovery` on macOS, `%LOCALAPPDATA%\Chamfera\Recovery` on Windows.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_dir() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if cfg!(target_os = "windows") {
        Some(var("LOCALAPPDATA")?.join("Chamfera").join("Recovery"))
    } else if cfg!(target_os = "macos") {
        Some(var("HOME")?.join("Library/Application Support/Chamfera/Recovery"))
    } else {
        Some(
            var("XDG_DATA_HOME")
                .or_else(|| var("HOME").map(|h| h.join(".local/share")))?
                .join("chamfera")
                .join("recovery"),
        )
    }
}

/// The answer to the recovery question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Recover,
    Later,
    Discard,
}

pub struct RecoveryUi {
    /// The recovery folder, or `None` for no autosave (tests, screenshots, the browser).
    pub dir: Option<PathBuf>,
    /// This run's recovery files are `<key>.chamfera` and `<key>.json`.
    pub key: String,
    /// Recovery files left by earlier runs, newest first; the first is being asked about.
    pub offers: Vec<Entry>,
    /// The question is showing.
    pub prompt: bool,
    /// When this run's recovery file was last written (ms since 1970), while it exists.
    pub written: Option<u64>,
    /// The design as last written to the recovery file.
    last: Option<chamfera_doc::Document>,
    /// When (egui seconds) the unsaved changes began, or the last autosave was.
    since: Option<f64>,
    /// The last autosave error.
    pub error: Option<String>,
    /// Where the prompt's buttons were drawn, for tests.
    pub buttons: Vec<(Answer, Rect)>,
}

impl Default for RecoveryUi {
    fn default() -> Self {
        Self {
            dir: None,
            key: recovery::new_key(),
            offers: Vec::new(),
            prompt: false,
            written: None,
            last: None,
            since: None,
            error: None,
            buttons: Vec::new(),
        }
    }
}

/// How long ago `ms` was, in words.
fn ago(ms: u64) -> String {
    let secs = recovery::now_ms().saturating_sub(ms) / 1000;
    match secs {
        0..=59 => "less than a minute ago".to_owned(),
        60..=119 => "a minute ago".to_owned(),
        120..=3599 => format!("{} minutes ago", secs / 60),
        3600..=7199 => "an hour ago".to_owned(),
        7200..=172_799 => format!("{} hours ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

impl ChamferaApp {
    /// Looks for recovery files other runs left (at start). Files of work saved since,
    /// and of runs still going, are not offered; the stale ones are removed.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn recovery_scan(&mut self) {
        let Some(dir) = self.recovery.dir.clone() else {
            return;
        };
        let found = match recovery::scan(&dir) {
            Ok((found, _skipped)) => found,
            Err(e) => {
                self.message = Some((format!("Recovery files: {e}"), true));
                return;
            }
        };
        self.recovery.offers.clear();
        for e in found {
            if e.key == self.recovery.key || recovery::writer_running(&e) {
                continue;
            }
            if recovery::saved_since(&e) {
                let _ = recovery::remove(&dir, &e.key);
                continue;
            }
            self.recovery.offers.push(e);
        }
        self.recovery.prompt = !self.recovery.offers.is_empty();
    }

    /// Acts on the answer to the question about the first offered file.
    pub(crate) fn recovery_answer(&mut self, answer: Answer) {
        let Some(dir) = self.recovery.dir.clone() else {
            self.recovery.prompt = false;
            return;
        };
        match answer {
            Answer::Later => self.recovery.prompt = false,
            Answer::Discard => {
                if !self.recovery.offers.is_empty() {
                    let e = self.recovery.offers.remove(0);
                    if let Err(err) = recovery::remove(&dir, &e.key) {
                        self.message = Some((format!("Discard: {err}"), true));
                    }
                }
                self.recovery.prompt = !self.recovery.offers.is_empty();
            }
            Answer::Recover => {
                if self.recovery.offers.is_empty() {
                    self.recovery.prompt = false;
                    return;
                }
                let e = self.recovery.offers.remove(0);
                let mut params = json!({ "path": e.file.to_string_lossy() });
                if let Some(d) = &e.design_path {
                    params["design_path"] = json!(d.to_string_lossy());
                }
                if self.run("design.recover", params).is_some() {
                    // This run carries on with that file: autosaves replace it, and a
                    // save or a clean quit removes it.
                    self.recovery_remove_own();
                    self.recovery.key = e.key.clone();
                    self.recovery.written = Some(e.time_ms);
                    self.recovery.last = Some(self.session.doc.clone());
                    self.recovery.since = None;
                    self.sketch = crate::sketch_ui::SketchUi::default();
                    self.solid = crate::solid_ui::SolidUi::default();
                    self.fit_pending = true;
                    self.history.reload = true;
                    self.message = Some((
                        format!(
                            "Recovered {} from {} ({})",
                            e.name(),
                            chamfera_io::history::format_utc(e.time()),
                            ago(e.time_ms)
                        ),
                        false,
                    ));
                }
                self.recovery.prompt = false;
            }
        }
    }

    /// Removes this run's recovery files (after a save, a clean quit, or once there is
    /// nothing unsaved).
    pub(crate) fn recovery_remove_own(&mut self) {
        if let (Some(dir), Some(_)) = (&self.recovery.dir, self.recovery.written)
            && let Err(e) = recovery::remove(dir, &self.recovery.key)
        {
            self.recovery.error = Some(e.to_string());
            return;
        }
        self.recovery.written = None;
        self.recovery.last = None;
    }

    /// Every frame: writes the recovery file once the design has had unsaved changes for
    /// the autosave interval (and changed since the last write); removes it once nothing
    /// is unsaved. `now` is in seconds (egui time). Returns how long until the next
    /// autosave is due, so the caller can ask for a frame then.
    pub(crate) fn autosave_step(&mut self, now: f64) -> Option<f64> {
        let interval = f64::from(self.prefs.autosave_secs);
        let dir = self.recovery.dir.clone()?;
        if !self.session.has_unsaved_changes() {
            self.recovery.since = None;
            if self.recovery.written.is_some() {
                self.recovery_remove_own();
            }
            return None;
        }
        if interval <= 0.0 {
            return None;
        }
        let since = *self.recovery.since.get_or_insert(now);
        let due = since + interval - now;
        if due > 0.0 {
            return Some(due);
        }
        self.recovery.since = Some(now);
        if self.recovery.last.as_ref() == Some(&self.session.doc) {
            return Some(interval);
        }
        match recovery::write(
            &dir,
            &self.recovery.key,
            &self.session.doc,
            self.session.path.as_deref(),
        ) {
            Ok(e) => {
                self.recovery.written = Some(e.time_ms);
                self.recovery.last = Some(self.session.doc.clone());
                self.recovery.error = None;
            }
            Err(e) => self.recovery.error = Some(e.to_string()),
        }
        Some(interval)
    }

    /// The autosave every frame, with a frame asked for when the next one is due.
    pub(crate) fn autosave_frame(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if let Some(due) = self.autosave_step(now)
            && due.is_finite()
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(due.clamp(0.05, 3600.0)));
        }
    }

    /// The autosave state for the status bar: the text, whether it is an error, and the
    /// tooltip. `None` when autosave is off.
    pub(crate) fn autosave_state(&self) -> Option<(String, bool, String)> {
        self.recovery.dir.as_ref()?;
        if let Some(e) = &self.recovery.error {
            return Some((
                "Autosave failed".into(),
                true,
                format!("The recovery file could not be written: {e}"),
            ));
        }
        if self.prefs.autosave_secs == 0 {
            return Some((
                "Autosave off".into(),
                false,
                "Turn it on in Preferences > General".into(),
            ));
        }
        let unsaved = self.session.has_unsaved_changes();
        Some(match (unsaved, self.recovery.written) {
            (true, Some(t)) if self.recovery.last.as_ref() == Some(&self.session.doc) => (
                "Autosaved".into(),
                false,
                format!(
                    "Unsaved work is in a recovery file, written {}",
                    chamfera_io::history::format_utc(t / 1000)
                ),
            ),
            (true, _) => (
                "Autosave pending".into(),
                false,
                format!(
                    "Unsaved work goes to a recovery file every {} s",
                    self.prefs.autosave_secs
                ),
            ),
            (false, _) => (
                "Autosave on".into(),
                false,
                format!(
                    "Unsaved work goes to a recovery file every {} s",
                    self.prefs.autosave_secs
                ),
            ),
        })
    }

    /// The status bar's autosave indicator: a dot and a word.
    pub(crate) fn autosave_indicator(&self, ui: &mut egui::Ui) {
        let Some((text, error, tip)) = self.autosave_state() else {
            return;
        };
        let look = self.look();
        let dot = if error {
            look.c(|t| t.error)
        } else if text == "Autosaved" || text == "Autosave on" {
            look.c(|t| t.success)
        } else {
            look.c(|t| t.warning)
        };
        let r = widgets::dot(ui, dot, 7.0);
        let label = ui.label(
            egui::RichText::new(&text)
                .size(widgets::T_SMALL)
                .color(if error {
                    look.c(|t| t.error)
                } else {
                    look.c(|t| t.text_muted)
                }),
        );
        r.union(label).on_hover_text(tip);
        widgets::bar_divider(ui, look, 14.0);
    }

    /// The question at start: recover the unsaved work a crash left?
    pub(crate) fn recovery_prompt(&mut self, ctx: &egui::Context) {
        if !self.recovery.prompt {
            return;
        }
        let Some(e) = self.recovery.offers.first().cloned() else {
            self.recovery.prompt = false;
            return;
        };
        let look = self.look();
        let count = self.recovery.offers.len();
        let mut answer: Option<Answer> = None;
        let mut buttons = Vec::new();
        let modal = egui::Modal::new(egui::Id::unique("recover unsaved work"))
            .frame(widgets::dialog_frame(look))
            .backdrop_color(color(self.theme.shadow))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                if widgets::dialog_title(ui, look, Icon::Warning, "Recover Unsaved Work") {
                    answer = Some(Answer::Later);
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "Recover unsaved work from {}?",
                            chamfera_io::history::format_utc(e.time())
                        ))
                        .size(widgets::T_BODY)
                        .color(look.c(|t| t.text)),
                    );
                    ui.add_space(widgets::S2);
                    let file = e
                        .design_path
                        .as_ref()
                        .map_or_else(|| "never saved".to_owned(), |p| p.display().to_string());
                    let muted = |t: String| {
                        egui::RichText::new(t)
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted))
                    };
                    ui.label(muted(format!(
                        "{} ({file}) was not saved when Chamfera last closed, {}.",
                        e.name(),
                        ago(e.time_ms)
                    )));
                    if count > 1 {
                        ui.label(muted(format!("1 of {count} designs with unsaved work.")));
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    let r = widgets::primary_button(ui, look, "Recover", true);
                    buttons.push((Answer::Recover, r.rect));
                    if r.clicked() {
                        answer = Some(Answer::Recover);
                    }
                    let r = widgets::secondary_button(ui, look, "Show Later");
                    buttons.push((Answer::Later, r.rect));
                    if r.clicked() {
                        answer = Some(Answer::Later);
                    }
                    let r = widgets::secondary_button(ui, look, "Discard");
                    buttons.push((Answer::Discard, r.rect));
                    if r.clicked() {
                        answer = Some(Answer::Discard);
                    }
                });
            });
        self.recovery.buttons = buttons;
        if modal.should_close() && answer.is_none() {
            answer = Some(Answer::Later);
        }
        if let Some(a) = answer {
            self.recovery_answer(a);
        }
    }

    /// `--recovery-demo`: keeps the design the start-up script built as if a crash had
    /// left it, in a scratch recovery folder, and starts on an empty design with the
    /// question showing (for screenshots).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recovery_demo(&mut self) {
        let dir =
            std::env::temp_dir().join(format!("chamfera-recovery-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let design = std::env::var_os("HOME")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("Designs")
            .join("bracket.chamfera");
        // Written five minutes ago by a run that is gone (process 0 is never running).
        let mut entry = match recovery::write(&dir, "0-1", &self.session.doc, Some(&design)) {
            Ok(e) => e,
            Err(e) => {
                self.message = Some((format!("Recovery demo: {e}"), true));
                return;
            }
        };
        entry.time_ms = entry.time_ms.saturating_sub(5 * 60 * 1000);
        self.run("design.new", json!({}));
        self.recovery.dir = Some(dir);
        self.recovery.offers = vec![entry];
        self.recovery.prompt = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use egui::Vec2;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "chamfera-ui-recovery-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn frame(app: &mut ChamferaApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1400.0, 900.0),
            )),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
    }

    fn click(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            frame(app, ctx, *t, events);
            *t += 0.05;
        }
    }

    /// An app that autosaved its unsaved work and then "crashed" (was dropped without a
    /// save or a clean quit).
    fn crash_with_work(dir: &std::path::Path, design: Option<&std::path::Path>) {
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.to_path_buf());
        if let Some(d) = design {
            app.run("design.save", json!({ "path": d.to_string_lossy() }));
        }
        app.run("param.set", json!({ "name": "w", "expression": "42 mm" }));
        // Unsaved since t = 0: nothing is written before the interval (60 s) is up.
        assert_eq!(app.autosave_step(0.0), Some(60.0));
        assert_eq!(app.autosave_step(59.0), Some(1.0));
        assert!(app.recovery.written.is_none());
        app.autosave_step(60.0);
        assert!(app.recovery.written.is_some());
        drop(app);
    }

    #[test]
    fn after_a_crash_the_prompt_offers_the_work_and_recover_brings_it_back() {
        let dir = scratch("recover");
        crash_with_work(&dir, None);
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.clone());
        app.recovery_scan();
        assert!(app.recovery.prompt);
        assert_eq!(app.recovery.offers.len(), 1);
        let ctx = egui::Context::default();
        let mut t = 0.0;
        for _ in 0..4 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        let recover = app
            .recovery
            .buttons
            .iter()
            .find(|(a, _)| *a == Answer::Recover)
            .map(|(_, r)| r.center())
            .expect("the Recover button is drawn");
        click(&mut app, &ctx, &mut t, recover);
        assert!(!app.recovery.prompt);
        assert_eq!(app.session.doc.parameters["w"].value, 42.0);
        assert!(app.session.has_unsaved_changes());
        // Saving removes the recovery file: a later start offers nothing.
        let path = dir.join("saved.chamfera");
        app.run("design.save", json!({ "path": path.to_string_lossy() }));
        app.autosave_step(t);
        assert!(app.recovery.written.is_none());
        assert!(recovery::scan(&dir).unwrap().0.is_empty());
    }

    #[test]
    fn discard_removes_the_file_and_show_later_keeps_it() {
        let dir = scratch("discard");
        crash_with_work(&dir, None);
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.clone());
        app.recovery_scan();
        let ctx = egui::Context::default();
        let mut t = 0.0;
        for _ in 0..4 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        let at = |app: &ChamferaApp, which| {
            app.recovery
                .buttons
                .iter()
                .find(|(a, _)| *a == which)
                .map(|(_, r)| r.center())
                .unwrap()
        };
        let later = at(&app, Answer::Later);
        click(&mut app, &ctx, &mut t, later);
        assert!(!app.recovery.prompt);
        assert!(app.session.doc.parameters.is_empty());
        assert_eq!(
            recovery::scan(&dir).unwrap().0.len(),
            1,
            "Show Later keeps it"
        );
        // The next start asks again; Discard removes it.
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.clone());
        app.recovery_scan();
        assert!(app.recovery.prompt);
        for _ in 0..4 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        let discard = at(&app, Answer::Discard);
        click(&mut app, &ctx, &mut t, discard);
        assert!(!app.recovery.prompt);
        assert!(app.session.doc.parameters.is_empty());
        assert!(recovery::scan(&dir).unwrap().0.is_empty());
    }

    #[test]
    fn work_saved_after_the_autosave_is_not_offered() {
        let dir = scratch("saved");
        let design = dir.join("d.chamfera");
        crash_with_work(&dir, Some(&design));
        // The design file is written after the recovery file: that work is not lost.
        std::thread::sleep(std::time::Duration::from_millis(5));
        chamfera_io::save_design(&chamfera_doc::Document::default(), &design).unwrap();
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.clone());
        app.recovery_scan();
        assert!(!app.recovery.prompt);
        assert!(
            recovery::scan(&dir).unwrap().0.is_empty(),
            "the stale file is removed"
        );
    }

    #[test]
    fn autosave_writes_only_changed_work_and_is_off_at_zero() {
        let dir = scratch("off");
        let mut app = ChamferaApp::headless();
        app.recovery.dir = Some(dir.clone());
        app.prefs.autosave_secs = 0;
        app.run("param.set", json!({ "name": "w", "value": 1 }));
        assert_eq!(app.autosave_step(0.0), None);
        assert_eq!(app.autosave_step(1000.0), None);
        assert!(recovery::scan(&dir).unwrap().0.is_empty());
        assert_eq!(app.autosave_state().unwrap().0, "Autosave off");
        app.prefs.autosave_secs = 10;
        app.autosave_step(1000.0);
        app.autosave_step(1010.0);
        let first = app.recovery.written.unwrap();
        assert_eq!(app.autosave_state().unwrap().0, "Autosaved");
        // Nothing changed since: the next interval writes nothing new.
        std::thread::sleep(std::time::Duration::from_millis(3));
        app.autosave_step(1020.0);
        assert_eq!(app.recovery.written, Some(first));
        // Undoing back to the saved design removes the file.
        app.run("edit.undo", json!({}));
        app.autosave_step(1021.0);
        assert!(recovery::scan(&dir).unwrap().0.is_empty());
    }
}

//! The History panel (File > Version History): the open design's kept versions with
//! their notes, times, summaries and thumbnails; Restore (asked first; one undo step),
//! Compare (the difference list, and both versions' bodies over each other in the
//! viewport in the `compare_a` and `compare_b` tints) and Delete. "Save Version with
//! note" (Ctrl+Alt+S) asks for a note and runs `version.save`.
//!
//! Everything goes through the `version.*` commands. Thumbnails are drawn after a save
//! with the snapshot renderer, off the UI thread, at a small size.

use crate::icons::Icon;
use crate::widgets::{self, Look, S2, S3, T_BODY, T_SMALL};
use crate::{ChamferaApp, color};
use egui::{Rect, Sense, Vec2, vec2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// Thumbnail size in pixels.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const THUMB_W: u32 = 128;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const THUMB_H: u32 = 96;

/// Something the panel asks about before it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Restore(u32),
    Delete(u32),
}

#[derive(Default)]
pub struct HistoryUi {
    pub open: bool,
    /// The last `version.list` reply.
    pub list: Option<Value>,
    /// Read the list again before drawing it.
    pub reload: bool,
    /// Versions picked for Compare (at most two, in the order picked).
    pub picked: Vec<u32>,
    /// The last comparison's reply.
    pub compare: Option<Value>,
    pub confirm: Option<Confirm>,
    /// "Save Version" is asking for a note.
    pub note: Option<String>,
    thumbs: BTreeMap<u32, egui::TextureHandle>,
    thumb_failed: BTreeSet<u32>,
    /// Thumbnails finished off the UI thread, by version id.
    thumbs_done: Arc<Mutex<Vec<u32>>>,
    /// A thumbnail is being drawn.
    thumb_busy: Arc<std::sync::atomic::AtomicBool>,
    /// Versions whose thumbnail was asked for this run.
    thumb_tried: BTreeSet<u32>,
}

impl HistoryUi {
    /// The panel is open and a thumbnail is being drawn or still to be drawn (a
    /// screenshot waits for them).
    pub fn thumbnails_pending(&self, gpu: bool) -> bool {
        if !self.open || !gpu || cfg!(target_arch = "wasm32") {
            return false;
        }
        if self.thumb_busy.load(std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        let Some(list) = &self.list else {
            return self.reload;
        };
        list["versions"].as_array().into_iter().flatten().any(|v| {
            v["thumbnail"] != true
                && v["id"]
                    .as_u64()
                    .and_then(|i| u32::try_from(i).ok())
                    .is_some_and(|id| !self.thumb_tried.contains(&id))
        })
    }
}

/// What a row or button asked for this frame.
enum Act {
    Pick(u32),
    Confirm(Confirm),
    Compare,
    ClearCompare,
    SaveVersion,
    Close,
}

fn summary_line(v: &Value, units: chamfera_doc::DisplayUnits) -> String {
    let s = &v["summary"];
    let features = s["features"].as_u64().unwrap_or(0);
    let plural = if features == 1 { "" } else { "s" };
    match s["volume"].as_f64() {
        Some(vol) => format!("{} · {features} feature{plural}", units.volume(vol)),
        None => format!("{features} feature{plural}"),
    }
}

impl ChamferaApp {
    /// Opens the History panel.
    pub(crate) fn open_history(&mut self) {
        self.history.open = true;
        self.history.reload = true;
    }

    /// Asks for a note, then saves the design and keeps a version with it.
    pub(crate) fn ask_version_note(&mut self) {
        self.history.note = Some(String::new());
    }

    /// A start-up script whose last `version.*` step compared versions opens the History
    /// panel with that comparison and its versions picked.
    pub(crate) fn history_from_script(&mut self, steps: &[Value], replies: &[Value]) {
        let Some(reply) = steps
            .iter()
            .zip(replies)
            .rev()
            .find(|(s, _)| s["command"] == "version.compare")
            .map(|(_, r)| r.clone())
        else {
            return;
        };
        if reply["cleared"] == true {
            return;
        }
        self.history.picked = ["a", "b"]
            .iter()
            .filter_map(|k| reply[*k].as_u64().and_then(|i| u32::try_from(i).ok()))
            .collect();
        self.history.compare = Some(reply);
        self.open_history();
    }

    /// The `version.*` items of the search palette and the menus.
    pub(crate) fn history_command(&mut self, id: &str) {
        if id == "version.save" {
            self.ask_version_note();
        } else {
            self.open_history();
        }
    }

    /// After a save that kept a version: read the list again (its thumbnail follows).
    pub(crate) fn after_version_saved(&mut self, _reply: &Value) {
        self.history.reload = true;
    }

    /// Draws the thumbnail of one listed version that has none yet, with the snapshot
    /// renderer on another thread (one at a time), and stores it in the history.
    /// Desktop with a GPU only; a version that fails is not tried again.
    #[cfg(not(target_arch = "wasm32"))]
    fn queue_thumbnails(&mut self, versions: &[Value]) {
        use std::sync::atomic::Ordering;
        if self.gpu.is_none() || self.history.thumb_busy.load(Ordering::SeqCst) {
            return;
        }
        let Some(id) = versions
            .iter()
            .rev()
            .filter(|v| v["thumbnail"] != true)
            .filter_map(|v| v["id"].as_u64().and_then(|i| u32::try_from(i).ok()))
            .find(|id| !self.history.thumb_tried.contains(id))
        else {
            return;
        };
        let Some(dir) = self
            .session
            .path
            .as_deref()
            .and_then(|p| chamfera_io::history::folder_for(p).ok())
        else {
            return;
        };
        self.history.thumb_tried.insert(id);
        let theme = self.theme;
        let done = self.history.thumbs_done.clone();
        let busy = self.history.thumb_busy.clone();
        busy.store(true, Ordering::SeqCst);
        let job = move || {
            let Ok(doc) = chamfera_io::history::read(&dir, id) else {
                return;
            };
            let scene = chamfera_engine::versions::preview_scene(doc);
            let Some(bounds) = scene.bounds() else {
                return;
            };
            let mut camera = chamfera_render::Camera::default();
            camera.set_view(chamfera_render::StandardView::Iso);
            camera.fit(&bounds, f64::from(THUMB_W) / f64::from(THUMB_H));
            let frame = chamfera_gpu::Frame {
                scene: &scene,
                camera: &camera,
                theme: &theme,
                width: THUMB_W,
                height: THUMB_H,
                scale: 1.0,
            };
            if let Ok(png) = chamfera_gpu::snapshot_png(&frame)
                && chamfera_io::history::write_thumbnail(&dir, id, &png).is_ok()
                && let Ok(mut d) = done.lock()
            {
                d.push(id);
            }
        };
        let busy_done = busy.clone();
        let spawned = std::thread::Builder::new()
            .name("chamfera thumbnail".into())
            .spawn(move || {
                job();
                busy_done.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            busy.store(false, Ordering::SeqCst);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn queue_thumbnails(&mut self, _versions: &[Value]) {}

    /// A version's thumbnail texture, read and decoded once.
    #[cfg(not(target_arch = "wasm32"))]
    fn thumbnail(&mut self, ctx: &egui::Context, id: u32) -> Option<egui::TextureId> {
        if let Some(t) = self.history.thumbs.get(&id) {
            return Some(t.id());
        }
        if self.history.thumb_failed.contains(&id) {
            return None;
        }
        let dir = self
            .session
            .path
            .as_deref()
            .and_then(|p| chamfera_io::history::folder_for(p).ok())?;
        let image = chamfera_io::history::read_thumbnail(&dir, id)
            .ok()
            .and_then(|bytes| decode_png(&bytes));
        match image {
            Some(img) => {
                let t =
                    ctx.load_texture(format!("version {id}"), img, egui::TextureOptions::LINEAR);
                let tid = t.id();
                self.history.thumbs.insert(id, t);
                Some(tid)
            }
            None => {
                self.history.thumb_failed.insert(id);
                None
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn thumbnail(&mut self, _ctx: &egui::Context, _id: u32) -> Option<egui::TextureId> {
        None
    }

    fn reload_history(&mut self) {
        self.history.reload = false;
        self.history.list = self.run("version.list", json!({}));
        let ids: BTreeSet<u32> = self
            .history
            .list
            .as_ref()
            .and_then(|l| l["versions"].as_array())
            .map(|vs| {
                vs.iter()
                    .filter_map(|v| v["id"].as_u64().and_then(|i| u32::try_from(i).ok()))
                    .collect()
            })
            .unwrap_or_default();
        self.history.picked.retain(|id| ids.contains(id));
        self.history.thumbs.retain(|id, _| ids.contains(id));
        self.history.thumb_failed.clear();
    }

    /// Runs the comparison of the picked versions: two picked, older against newer; one,
    /// against the open design.
    pub(crate) fn compare_picked(&mut self) {
        let mut picked = self.history.picked.clone();
        picked.sort_unstable();
        let params = match picked.as_slice() {
            [a] => json!({ "a": a, "b": "current" }),
            [a, b, ..] => json!({ "a": a, "b": b }),
            [] => return,
        };
        self.history.compare = self.run("version.compare", params);
    }

    /// The History panel, the questions it asks and the note dialog, every frame.
    pub(crate) fn history_windows(&mut self, ctx: &egui::Context) {
        if let Ok(mut done) = self.history.thumbs_done.lock()
            && !done.is_empty()
        {
            for id in done.drain(..) {
                self.history.thumb_failed.remove(&id);
            }
            self.history.reload = true;
        }
        self.version_note_dialog(ctx);
        self.history_confirm(ctx);
        if !self.history.open {
            return;
        }
        if self.history.reload {
            self.reload_history();
        }
        let versions: Vec<Value> = self
            .history
            .list
            .as_ref()
            .and_then(|l| l["versions"].as_array().cloned())
            .unwrap_or_default();
        let saved = self
            .history
            .list
            .as_ref()
            .is_some_and(|l| l["saved"] == true);
        self.queue_thumbnails(&versions);
        let mut thumbs: BTreeMap<u32, egui::TextureId> = BTreeMap::new();
        for v in &versions {
            if v["thumbnail"] == true
                && let Some(id) = v["id"].as_u64().and_then(|i| u32::try_from(i).ok())
                && let Some(t) = self.thumbnail(ctx, id)
            {
                thumbs.insert(id, t);
            }
        }
        let look = self.look();
        let units = self.units();
        let picked = self.history.picked.clone();
        let compare = self.history.compare.clone();
        let vr = self.viewport_rect;
        let mut acts: Vec<Act> = Vec::new();
        egui::Window::new("History")
            .id(egui::Id::unique("history window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(egui::pos2(vr.max.x - 400.0, vr.min.y + widgets::S5))
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(372.0);
                if widgets::dialog_title(ui, look, Icon::Undo, "Version History") {
                    acts.push(Act::Close);
                }
                widgets::dialog_body(ui, |ui| {
                    if !saved {
                        ui.label(
                            egui::RichText::new(
                                "Save the design to a file first: each save keeps a version \
                                 beside it.",
                            )
                            .size(T_BODY)
                            .color(look.c(|t| t.text_muted)),
                        );
                        return;
                    }
                    if versions.is_empty() {
                        ui.label(
                            egui::RichText::new("No versions yet: the next save keeps one.")
                                .size(T_BODY)
                                .color(look.c(|t| t.text_muted)),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for v in versions.iter().rev() {
                                version_row(ui, look, v, units, &picked, &thumbs, &mut acts);
                            }
                        });
                    if let Some(c) = &compare {
                        ui.add_space(S3);
                        compare_view(ui, look, c, units, &mut acts);
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    let text = match picked.len() {
                        0 | 1 => "Compare with Current",
                        _ => "Compare",
                    };
                    let can = !picked.is_empty();
                    if widgets::primary_button(ui, look, text, can).clicked() && can {
                        acts.push(Act::Compare);
                    }
                    if widgets::secondary_button(ui, look, "Save Version…").clicked() {
                        acts.push(Act::SaveVersion);
                    }
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        acts.push(Act::Close);
                    }
                });
            });
        for a in acts {
            match a {
                Act::Pick(id) => {
                    let p = &mut self.history.picked;
                    if let Some(i) = p.iter().position(|x| *x == id) {
                        p.remove(i);
                    } else {
                        p.push(id);
                        if p.len() > 2 {
                            p.remove(0);
                        }
                    }
                }
                Act::Confirm(c) => self.history.confirm = Some(c),
                Act::Compare => self.compare_picked(),
                Act::ClearCompare => {
                    self.run("version.compare", json!({ "clear": true }));
                    self.history.compare = None;
                }
                Act::SaveVersion => self.ask_version_note(),
                Act::Close => {
                    self.history.open = false;
                    if self.history.compare.take().is_some() {
                        self.run("version.compare", json!({ "clear": true }));
                    }
                }
            }
        }
    }

    /// "Restore version N?" and "Delete version N?".
    fn history_confirm(&mut self, ctx: &egui::Context) {
        let Some(c) = self.history.confirm else {
            return;
        };
        let look = self.look();
        let (title, text, ok) = match c {
            Confirm::Restore(id) => (
                "Restore Version",
                format!(
                    "Replace the open design with version {id}? Undo brings back the design \
                     as it is now; the file changes only when you save."
                ),
                "Restore",
            ),
            Confirm::Delete(id) => (
                "Delete Version",
                format!("Delete version {id} from the history? This cannot be undone."),
                "Delete",
            ),
        };
        let mut answer: Option<bool> = None;
        let modal = egui::Modal::new(egui::Id::unique("history confirm"))
            .frame(widgets::dialog_frame(look))
            .backdrop_color(color(self.theme.shadow))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                if widgets::dialog_title(ui, look, Icon::Warning, title) {
                    answer = Some(false);
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(text);
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, ok, true).clicked() {
                        answer = Some(true);
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.history.confirm = None;
                self.confirmed(c);
            }
            Some(false) => self.history.confirm = None,
            None => {}
        }
    }

    /// Does what was confirmed.
    pub(crate) fn confirmed(&mut self, c: Confirm) {
        match c {
            Confirm::Restore(id) => {
                if self.run("version.restore", json!({ "id": id })).is_some() {
                    self.sketch = crate::sketch_ui::SketchUi::default();
                    self.history.compare = None;
                    self.message =
                        Some((format!("Restored version {id} (Ctrl+Z undoes it)"), false));
                }
            }
            Confirm::Delete(id) => {
                if self.run("version.delete", json!({ "id": id })).is_some() {
                    self.history.picked.retain(|x| *x != id);
                    self.message = Some((format!("Deleted version {id}"), false));
                }
            }
        }
        self.history.reload = true;
    }

    /// Saves the design and keeps a version with this note.
    pub(crate) fn save_version(&mut self, note: &str) {
        if self.session.path.is_none() {
            // The first save asks where (and keeps a version of its own).
            self.save(false);
            if self.session.path.is_none() {
                return;
            }
        }
        if let Some(r) = self.run("version.save", json!({ "note": note })) {
            self.after_version_saved(&r);
            self.recovery_remove_own();
            let id = r["version"]["id"].as_u64().unwrap_or(0);
            self.message = Some((format!("Saved version {id}"), false));
        }
    }

    /// "Save Version": a note, then Save.
    fn version_note_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut note) = self.history.note.take() else {
            return;
        };
        let look = self.look();
        let mut answer: Option<bool> = None;
        let modal = egui::Modal::new(egui::Id::unique("version note"))
            .frame(widgets::dialog_frame(look))
            .backdrop_color(color(self.theme.shadow))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                if widgets::dialog_title(ui, look, Icon::Save, "Save Version") {
                    answer = Some(false);
                }
                widgets::dialog_body(ui, |ui| {
                    widgets::field_label(ui, look, "What changed in this version?");
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut note)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .char_limit(chamfera_io::history::MAX_NOTE_CHARS),
                    );
                    r.request_focus();
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Save", true).clicked() {
                        answer = Some(true);
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => self.save_version(&note),
            Some(false) => {}
            None => self.history.note = Some(note),
        }
    }
}

/// One version: thumbnail, title, time and summary, and its buttons.
fn version_row(
    ui: &mut egui::Ui,
    look: Look,
    v: &Value,
    units: chamfera_doc::DisplayUnits,
    picked: &[u32],
    thumbs: &BTreeMap<u32, egui::TextureId>,
    acts: &mut Vec<Act>,
) {
    let Some(id) = v["id"].as_u64().and_then(|i| u32::try_from(i).ok()) else {
        return;
    };
    let is_picked = picked.contains(&id);
    let fill = if is_picked {
        look.c(|t| t.selected)
    } else {
        look.c(|t| t.chrome)
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(widgets::R_SMALL))
        .inner_margin(egui::Margin::same(S2 as i8 + 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(64.0, 48.0), Sense::click());
                match thumbs.get(&id) {
                    Some(t) => {
                        ui.painter().image(
                            *t,
                            r,
                            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        ui.painter().rect_filled(
                            r,
                            egui::CornerRadius::same(widgets::R_SMALL),
                            look.c(|t| t.panel_alt),
                        );
                        look.icons.paint(
                            ui.painter(),
                            Icon::Body,
                            Rect::from_center_size(r.center(), Vec2::splat(20.0)),
                            look.c(|t| t.text_muted),
                            look.c(|t| t.text_muted),
                        );
                    }
                }
                if resp.clicked() {
                    acts.push(Act::Pick(id));
                }
                ui.vertical(|ui| {
                    let note = v["note"].as_str().unwrap_or("");
                    let title = if note.is_empty() {
                        format!("Version {id}")
                    } else {
                        format!("Version {id} · {note}")
                    };
                    let t = ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .size(T_BODY)
                                .color(look.c(|t| t.text)),
                        )
                        .truncate()
                        .sense(Sense::click()),
                    );
                    if t.clicked() {
                        acts.push(Act::Pick(id));
                    }
                    let kind = if v["kind"] == "note" {
                        "Version"
                    } else {
                        "Save"
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "{kind} · {}",
                            v["when"].as_str().unwrap_or("")
                        ))
                        .size(T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                    );
                    ui.label(
                        egui::RichText::new(summary_line(v, units))
                            .size(T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                    );
                    ui.horizontal(|ui| {
                        let mut check = is_picked;
                        if ui.checkbox(&mut check, "Compare").changed() {
                            acts.push(Act::Pick(id));
                        }
                        if ui.small_button("Restore").clicked() {
                            acts.push(Act::Confirm(Confirm::Restore(id)));
                        }
                        if ui.small_button("Delete").clicked() {
                            acts.push(Act::Confirm(Confirm::Delete(id)));
                        }
                    });
                });
            });
        });
    ui.add_space(S2);
}

/// The difference list of a comparison, with the tint legend.
fn compare_view(
    ui: &mut egui::Ui,
    look: Look,
    c: &Value,
    units: chamfera_doc::DisplayUnits,
    acts: &mut Vec<Act>,
) {
    let side = |v: &Value| match v.as_u64() {
        Some(id) => format!("version {id}"),
        None => "the open design".to_owned(),
    };
    widgets::caption(ui, look, "COMPARISON");
    ui.horizontal(|ui| {
        widgets::dot(ui, look.c(|t| t.compare_a), 10.0);
        ui.label(egui::RichText::new(format!("A: {}", side(&c["a"]))).size(T_SMALL));
        ui.add_space(S3);
        widgets::dot(ui, look.c(|t| t.compare_b), 10.0);
        ui.label(egui::RichText::new(format!("B: {}", side(&c["b"]))).size(T_SMALL));
    });
    if c["shown"] == true {
        ui.label(
            egui::RichText::new(
                "In the viewport B is see-through over A: where both have material the tints mix.",
            )
            .size(T_SMALL)
            .color(look.c(|t| t.text_muted)),
        );
    }
    let text = |ui: &mut egui::Ui, s: String| {
        ui.label(
            egui::RichText::new(s)
                .size(T_SMALL)
                .color(look.c(|t| t.text)),
        );
    };
    if c["same"] == true {
        text(ui, "No differences.".into());
    }
    if let Some(d) = c["volume_delta"].as_f64() {
        let sign = if d > 0.0 { "+" } else { "" };
        text(
            ui,
            format!(
                "Volume {sign}{} (A {}, B {})",
                units.volume(d),
                c["volume_a"]
                    .as_f64()
                    .map_or("?".into(), |v| units.volume(v)),
                c["volume_b"]
                    .as_f64()
                    .map_or("?".into(), |v| units.volume(v)),
            ),
        );
    }
    for p in c["parameters"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or("");
        let line = match p["change"].as_str() {
            Some("added") => format!("Parameter {name} added: {}", p["to"].as_str().unwrap_or("")),
            Some("removed") => format!("Parameter {name} removed"),
            _ => format!(
                "Parameter {name}: {} to {}",
                p["from"].as_str().unwrap_or(""),
                p["to"].as_str().unwrap_or("")
            ),
        };
        text(ui, line);
    }
    for (key, word) in [
        ("added", "Added"),
        ("removed", "Removed"),
        ("changed", "Changed"),
    ] {
        for f in c["features"][key].as_array().into_iter().flatten() {
            let mut line = format!("{word} {}", f["name"].as_str().unwrap_or("?"));
            if let Some(fields) = f["fields"].as_array()
                && !fields.is_empty()
            {
                let names: Vec<&str> = fields.iter().filter_map(Value::as_str).collect();
                line.push_str(&format!(" ({})", names.join(", ")));
            }
            text(ui, line);
        }
    }
    if c["shown"] == true && ui.small_button("Clear Comparison").clicked() {
        acts.push(Act::ClearCompare);
    }
}

/// A PNG thumbnail as an egui image (small images only).
#[cfg(not(target_arch = "wasm32"))]
fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (w, h) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let data = buf.get(..frame.buffer_size())?;
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        _ => return None,
    };
    let size = [w as usize, h as usize];
    (rgba.len() == size[0] * size[1] * 4)
        .then(|| egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("chamfera-ui-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_panel_restores_after_asking_and_compares_the_picked_versions() {
        let dir = scratch("panel");
        let mut app = ChamferaApp::headless();
        let path = dir.join("p.chamfera");
        app.run("param.set", json!({ "name": "w", "value": 1 }));
        app.run("design.save", json!({ "path": path.to_string_lossy() }));
        app.run("param.set", json!({ "name": "w", "value": 2 }));
        app.save_version("w is 2");
        app.open_history();
        let ctx = egui::Context::default();
        let raw = egui::RawInput::default();
        let mut out = ctx.run_ui(raw, |ui| app.history_windows(ui.ctx()));
        out.textures_delta.clear();
        let list = app.history.list.clone().unwrap();
        assert_eq!(list["versions"].as_array().unwrap().len(), 2);
        assert_eq!(list["versions"][1]["note"], "w is 2");
        // Restore asks first; only the answer restores.
        app.history.confirm = Some(Confirm::Restore(1));
        assert_eq!(app.session.doc.parameters["w"].value, 2.0);
        app.history.confirm = None;
        app.confirmed(Confirm::Restore(1));
        assert_eq!(app.session.doc.parameters["w"].value, 1.0);
        app.run("edit.undo", json!({}));
        assert_eq!(app.session.doc.parameters["w"].value, 2.0);
        // Two picked: older against newer, and both shown in the viewport.
        app.history.picked = vec![2, 1];
        app.compare_picked();
        let c = app.history.compare.clone().unwrap();
        assert_eq!((c["a"].clone(), c["b"].clone()), (json!(1), json!(2)));
        assert_eq!(c["parameters"][0]["name"], "w");
        // One picked: against the open design (w = 2, the same as version 2).
        app.history.picked = vec![2];
        app.compare_picked();
        assert_eq!(app.history.compare.clone().unwrap()["same"], true);
    }

    #[test]
    fn a_broken_thumbnail_is_not_shown_and_never_panics() {
        assert!(decode_png(b"").is_none());
        assert!(decode_png(b"\x89PNG\r\n\x1a\nnot really").is_none());
    }
}

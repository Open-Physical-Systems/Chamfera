//! The Assistant panel (View > Assistant, Ctrl+Shift+A): the user types a request and
//! Claude builds it live by running Chamfera's commands on the open design.
//!
//! The Messages API calls run on a worker thread (`chamfera_assistant::agent::run`); each
//! tool call comes back here over a channel and runs on the UI thread through
//! `Session::execute`, so the viewport, timeline and browser update after every step and
//! the UI never waits on the network. Calls that replace or delete much of the design, and
//! file commands (when allowed in Preferences), wait for Allow or Deny in the panel. Stop
//! cancels between stream lines and tool calls. The steps of one request are merged into
//! one undo step (`Session::group_undo_since`), which "Undo assistant step" takes back;
//! when Claude itself undoes or redoes during a request, its steps stay separate.
//!
//! The browser build shows a note instead: calling the API from a web page would expose
//! the key to the page.

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::widgets::{self, Look, S2, S3, S4, T_BODY, T_SMALL};
use chamfera_assistant::tools::{self, Gate, Kind, ToolSet};
use chamfera_assistant::{ApiKey, Conversation, Event, RunEnd, ToolCall, ToolOutcome, Usage};
use egui::{Color32, Key, Modifiers, vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

/// Most transcript entries kept (the oldest go first).
const MAX_ENTRIES: usize = 2000;
/// Most messages from the worker handled in one frame.
const MAX_MESSAGES_PER_FRAME: usize = 400;

/// A line of the transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User(String),
    Assistant(String),
    Tool {
        id: String,
        label: String,
        input: String,
        state: ToolState,
    },
    Notice {
        text: String,
        error: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolState {
    /// Streaming in or waiting to run.
    Pending,
    /// Waiting for the user's Allow or Deny.
    Asking,
    Ok(String),
    Failed(String),
}

/// What the worker sends to the UI thread.
enum Msg {
    Event(Event),
    Tool(ToolCall, Sender<ToolOutcome>),
    Done(Box<Conversation>, RunEnd),
}

/// Makes the transport for a request (tests and the demo play a recording).
pub type TransportMaker = Box<dyn FnMut() -> Box<dyn chamfera_assistant::Transport>>;

/// A request in progress.
struct Job {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    tools: ToolSet,
    mark: chamfera_engine::UndoMark,
    bodies_before: usize,
}

/// A call waiting for the user.
struct Asking {
    call: ToolCall,
    reply: Sender<ToolOutcome>,
    what: String,
}

#[derive(Default)]
pub struct AssistantUi {
    pub open: bool,
    pub input: String,
    pub entries: Vec<Entry>,
    /// The conversation between requests (it lives on the worker during one).
    conversation: Option<Conversation>,
    job: Option<Job>,
    asking: Option<Asking>,
    /// Token counts of the conversation so far.
    pub usage: Usage,
    thinking: bool,
    /// The undo place right after the last request's steps were merged: while the
    /// history is still there, "Undo assistant step" undoes them.
    undo_after: Option<chamfera_engine::UndoMark>,
    /// The key, and where it came from.
    pub key: Option<(ApiKey, chamfera_assistant::key::KeySource)>,
    /// Replaces the HTTPS transport (tests, `--assistant-demo`).
    pub transport: Option<TransportMaker>,
    /// The key being typed on the Preferences page.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub key_text: String,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub key_message: Option<(String, bool)>,
}

impl AssistantUi {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    fn push(&mut self, e: Entry) {
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(e);
    }

    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.push(Entry::Notice {
            text: text.into(),
            error,
        });
    }

    fn tool_entry(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .rev()
            .find(|e| matches!(e, Entry::Tool { id: i, .. } if i == id))
    }

    fn set_tool(&mut self, id: &str, f: impl FnOnce(&mut String, &mut String, &mut ToolState)) {
        if let Some(Entry::Tool {
            label,
            input,
            state,
            ..
        }) = self.tool_entry(id)
        {
            f(label, input, state);
        }
    }

    /// Loads the key: the key file first, then `ANTHROPIC_API_KEY`.
    pub fn load_key(&mut self) {
        use chamfera_assistant::key::KeySource;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(k) = key_path().and_then(|p| chamfera_assistant::key::load(&p)) {
            self.key = Some((k, KeySource::File));
            return;
        }
        self.key = ApiKey::from_env().map(|k| (k, KeySource::Environment));
    }
}

/// The key file, beside the preferences file.
#[cfg(not(target_arch = "wasm32"))]
pub fn key_path() -> Option<std::path::PathBuf> {
    crate::prefs::default_path().map(|p| p.with_file_name("assistant-key"))
}

/// The worker's side of tool calls: send each to the UI thread and wait for its outcome.
struct ChannelHost {
    tx: Sender<Msg>,
    cancel: Arc<AtomicBool>,
}

impl chamfera_assistant::ToolHost for ChannelHost {
    fn call(&mut self, call: &ToolCall) -> ToolOutcome {
        let (otx, orx) = std::sync::mpsc::channel();
        if self.tx.send(Msg::Tool(call.clone(), otx)).is_err() {
            return ToolOutcome::error("not run: the panel was closed");
        }
        loop {
            match orx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(o) => return o,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if self.cancel.load(Ordering::SeqCst) {
                        return ToolOutcome::error("not run: the user pressed Stop");
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return ToolOutcome::error("not run: the panel was closed");
                }
            }
        }
    }
}

/// Renders the viewport as it is now, for the `screenshot` tool.
#[cfg(not(target_arch = "wasm32"))]
fn viewport_png(
    session: &mut chamfera_engine::Session,
    theme: &chamfera_render::Theme,
) -> Result<Vec<u8>, String> {
    let (width, height) = tools::SCREENSHOT_SIZE;
    let frame = chamfera_gpu::Frame {
        scene: &session.scene,
        camera: &session.camera,
        theme,
        width,
        height,
        scale: 1.0,
    };
    chamfera_gpu::snapshot_png(&frame).map_err(|e| format!("screenshot unavailable: {e}"))
}

fn short_json(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.chars().count() <= 600 {
        s
    } else {
        let mut t: String = s.chars().take(600).collect();
        t.push('…');
        t
    }
}

impl ChamferaApp {
    fn assistant_settings(&self) -> chamfera_assistant::Settings {
        let p = self.prefs.assistant;
        chamfera_assistant::Settings {
            model: p.model,
            effort: p.effort,
            allow_file_commands: p.allow_file_commands,
            screenshots: p.screenshots && cfg!(not(target_arch = "wasm32")),
            ..Default::default()
        }
    }

    /// `--assistant-demo`: open the panel and play the recorded plate conversation.
    pub(crate) fn assistant_demo(&mut self) {
        self.assistant.open = true;
        self.assistant.transport = Some(Box::new(|| {
            let mut r = chamfera_assistant::script::Replay::new(
                &chamfera_assistant::script::demo_replies(),
            );
            r.line_delay = std::time::Duration::from_millis(4);
            Box::new(r)
        }));
        self.assistant.input = chamfera_assistant::script::DEMO_REQUEST.to_owned();
        self.assistant_send();
    }

    /// Sends the typed request.
    pub(crate) fn assistant_send(&mut self) {
        let text = self.assistant.input.trim().to_owned();
        if text.is_empty() || self.assistant.running() {
            return;
        }
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let transport: Box<dyn chamfera_assistant::Transport> =
            if let Some(make) = self.assistant.transport.as_mut() {
                make()
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let Some((key, _)) = self.assistant.key.clone() else {
                        self.assistant
                            .notice("Add an API key first (Preferences > Assistant).", true);
                        return;
                    };
                    Box::new(chamfera_assistant::http::HttpTransport::new(key))
                }
                #[cfg(target_arch = "wasm32")]
                return;
            };
        let settings = self.assistant_settings();
        // Settings changed since the conversation started apply after Clear: the tool
        // list and the model stay fixed within one conversation, for the prompt cache.
        let mut conv = self
            .assistant
            .conversation
            .take()
            .unwrap_or_else(|| Conversation::new(settings));
        if conv.settings != settings {
            self.assistant.notice(
                "Settings changed: they apply to the next conversation (Clear).",
                false,
            );
        }
        self.assistant.input.clear();
        self.assistant.push(Entry::User(text.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let tools = conv.tools.clone();
        let worker_cancel = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("assistant".into())
            .spawn(move || {
                let mut transport = transport;
                let mut host = ChannelHost {
                    tx: tx.clone(),
                    cancel: worker_cancel.clone(),
                };
                let events_tx = tx.clone();
                let end = chamfera_assistant::agent::run(
                    &mut conv,
                    &text,
                    transport.as_mut(),
                    &mut host,
                    &mut |e| {
                        let _ = events_tx.send(Msg::Event(e));
                    },
                    &worker_cancel,
                );
                let _ = tx.send(Msg::Done(Box::new(conv), end));
            });
        if let Err(e) = spawned {
            self.assistant
                .notice(format!("Could not start the assistant: {e}"), true);
            return;
        }
        self.assistant.undo_after = None;
        self.assistant.job = Some(Job {
            rx,
            cancel,
            tools,
            mark: self.session.undo_mark(),
            bodies_before: self.session.scene.bodies.len(),
        });
    }

    /// Stop: cancel the request between steps; a call waiting for the user is denied.
    pub(crate) fn assistant_stop(&mut self) {
        if let Some(job) = &self.assistant.job {
            job.cancel.store(true, Ordering::SeqCst);
        }
        if let Some(a) = self.assistant.asking.take() {
            let _ = a
                .reply
                .send(ToolOutcome::error("not run: the user pressed Stop"));
            self.assistant.set_tool(&a.call.id, |_, _, s| {
                *s = ToolState::Failed("stopped".into());
            });
        }
    }

    /// Clear: a new conversation (the design keeps what was built).
    pub(crate) fn assistant_clear(&mut self) {
        self.assistant_stop();
        self.assistant.job = None;
        self.assistant.conversation = None;
        self.assistant.entries.clear();
        self.assistant.usage = Usage::default();
        self.assistant.thinking = false;
    }

    /// Runs one tool call on the session, as the panel's own command.
    fn assistant_execute(&mut self, tools: &ToolSet, call: &ToolCall) -> ToolOutcome {
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = {
            let theme = self.theme;
            let mut shot = |s: &mut chamfera_engine::Session| viewport_png(s, &theme);
            tools::execute(
                &mut self.session,
                tools,
                &call.name,
                &call.input,
                Some(&mut shot),
            )
        };
        #[cfg(target_arch = "wasm32")]
        let outcome = tools::execute(&mut self.session, tools, &call.name, &call.input, None);
        // The sketch tools follow the active sketch, as after a start-up script.
        if self.session.active_sketch.is_some() {
            if self.sketch.tool.is_none() {
                self.sketch.tool = Some(crate::sketch_ui::Tool::Select);
            }
            self.refresh_sketch_status();
        } else if self.sketch.tool.is_some() {
            self.sketch = crate::sketch_ui::SketchUi::default();
        }
        outcome
    }

    /// Handles what the worker sent since the last frame. Runs every frame.
    pub(crate) fn assistant_poll(&mut self, ctx: &egui::Context) {
        if self.assistant.asking.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        for _ in 0..MAX_MESSAGES_PER_FRAME {
            let Some(job) = &self.assistant.job else {
                return;
            };
            let msg = match job.rx.try_recv() {
                Ok(m) => m,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.assistant.job = None;
                    self.assistant
                        .notice("The assistant stopped unexpectedly.", true);
                    return;
                }
            };
            match msg {
                Msg::Event(e) => self.assistant_event(e),
                Msg::Tool(call, reply) => {
                    let tools = job.tools.clone();
                    let kind = tools.kind(&call.name);
                    let label = kind
                        .map_or_else(|| call.name.clone(), |k| tools::row_label(k, &call.input));
                    let input = short_json(&call.input);
                    self.assistant.set_tool(&call.id, |l, i, _| {
                        *l = label.clone();
                        *i = input.clone();
                    });
                    let confirm = self.prefs.assistant.confirm_destructive;
                    match kind.map(|k| tools::gate(k, &call.input, confirm)) {
                        Some(Gate::Confirm(what)) => {
                            self.assistant
                                .set_tool(&call.id, |_, _, s| *s = ToolState::Asking);
                            self.assistant.asking = Some(Asking { call, reply, what });
                            ctx.request_repaint();
                            return;
                        }
                        _ => {
                            let outcome = self.assistant_execute(&tools, &call);
                            let _ = reply.send(outcome);
                        }
                    }
                }
                Msg::Done(conv, end) => {
                    self.assistant_done(*conv, end);
                    return;
                }
            }
        }
        if self.assistant.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    fn assistant_event(&mut self, e: Event) {
        let a = &mut self.assistant;
        match e {
            Event::Text(t) => {
                a.thinking = false;
                if let Some(Entry::Assistant(s)) = a.entries.last_mut() {
                    s.push_str(&t);
                } else {
                    a.push(Entry::Assistant(t));
                }
            }
            Event::Thinking => a.thinking = true,
            Event::ToolPending { id, name } => {
                a.thinking = false;
                let kind = a.job.as_ref().and_then(|j| j.tools.kind(&name));
                let label = match kind {
                    Some(_) => kind_label(kind).to_owned(),
                    None => name,
                };
                a.push(Entry::Tool {
                    id,
                    label,
                    input: String::new(),
                    state: ToolState::Pending,
                });
            }
            Event::ToolDone { id, ok, summary } => a.set_tool(&id, |_, _, s| {
                *s = if ok {
                    ToolState::Ok(summary)
                } else {
                    ToolState::Failed(summary)
                };
            }),
            Event::Usage(u) => a.usage.add(u),
            Event::Notice { text, error } => a.notice(text, error),
        }
    }

    fn assistant_done(&mut self, conv: Conversation, end: RunEnd) {
        let job = self.assistant.job.take();
        self.assistant.conversation = Some(conv);
        self.assistant.thinking = false;
        if let Some(job) = job {
            if self.session.group_undo_since(job.mark) > 0 {
                self.assistant.undo_after = Some(self.session.undo_mark());
            }
            // The first body of a request: bring it into view.
            if job.bodies_before == 0 && !self.session.scene.bodies.is_empty() {
                self.run("view.fit", self.viewport_params());
            }
        }
        // Calls still shown as pending did not run.
        for e in &mut self.assistant.entries {
            if let Entry::Tool { state, .. } = e
                && matches!(state, ToolState::Pending | ToolState::Asking)
            {
                *state = ToolState::Failed("not run".into());
            }
        }
        match end {
            RunEnd::Done => {}
            RunEnd::Stopped => self.assistant.notice("Stopped.", false),
            RunEnd::Refused => self.assistant.notice("Claude declined this request.", true),
            RunEnd::MaxTokens => self.assistant.notice(
                "The reply reached the output limit and was cut off; its tool calls did not run.",
                true,
            ),
            RunEnd::Failed(why) => self.assistant.notice(why, true),
        }
    }

    fn assistant_answer(&mut self, allow: bool) {
        let Some(a) = self.assistant.asking.take() else {
            return;
        };
        let outcome = match (allow, self.assistant.job.as_ref()) {
            (true, Some(job)) => {
                let tools = job.tools.clone();
                self.assistant_execute(&tools, &a.call)
            }
            _ => ToolOutcome::error("not run: the user declined"),
        };
        let _ = a.reply.send(outcome);
    }

    /// "Undo assistant step" is offered while the last request's merged step is still
    /// the newest one in the history.
    fn assistant_can_undo(&self) -> bool {
        !self.assistant.running()
            && self
                .assistant
                .undo_after
                .is_some_and(|m| m == self.session.undo_mark())
    }

    /// The panel.
    pub(crate) fn assistant_panel(&mut self, ui: &mut egui::Ui) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let mut close = false;
        let mut send = false;
        let mut stop = false;
        let mut clear = false;
        let mut undo = false;
        let mut answer = None;
        let mut open_prefs = false;
        let running = self.assistant.running();
        let can_undo = self.assistant_can_undo();
        let settings = self.prefs.assistant;
        let has_key = self.assistant.key.is_some() || self.assistant.transport.is_some();
        // Header: title, model and effort, usage.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S3;
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::hover());
            look.icon(ui, Icon::Assistant, r, true);
            ui.label(
                egui::RichText::new("Assistant")
                    .size(widgets::T_TITLE)
                    .color(look.c(|t| t.text)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, look, Icon::Close, 24.0, 16.0, true, false)
                    .on_hover_text(crate::shortcuts::tip(
                        "Close the Assistant",
                        crate::shortcuts::Action::Assistant,
                    ))
                    .clicked()
                {
                    close = true;
                }
            });
        });
        let model = self
            .assistant
            .conversation
            .as_ref()
            .map_or(settings.model, |c| c.settings.model);
        let effort = self
            .assistant
            .conversation
            .as_ref()
            .map_or(settings.effort, |c| c.settings.effort);
        let u = self.assistant.usage;
        let tokens = |n: u64| {
            if n >= 10_000 {
                format!("{}k", n / 1000)
            } else {
                n.to_string()
            }
        };
        ui.label(
            egui::RichText::new(format!(
                "{} · effort {} · {} in, {} cached, {} out · ≈ ${:.3}",
                model.name(),
                effort.key(),
                tokens(u.input + u.cache_write),
                tokens(u.cache_read),
                tokens(u.output),
                u.cost(model)
            ))
            .size(T_SMALL)
            .color(look.c(|t| t.text_muted)),
        );
        ui.add_space(S2);
        let r = ui.min_rect();
        ui.painter().hline(
            r.x_range(),
            r.max.y.round() + 0.5,
            egui::Stroke::new(1.0, look.c(|t| t.border)),
        );
        ui.add_space(S2);

        // The input and buttons sit at the bottom; the transcript fills what is left.
        let footer = 132.0;
        let transcript_height = (ui.available_height() - footer).max(80.0);
        egui::ScrollArea::vertical()
            .id_salt("assistant transcript")
            .max_height(transcript_height)
            .min_scrolled_height(transcript_height)
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = S2;
                if cfg!(target_arch = "wasm32") {
                    hint(
                        ui,
                        look,
                        "The Assistant works in the desktop app for now. Calling the Claude \
                         API from a web page would expose your API key to the page.",
                    );
                } else if !has_key {
                    hint(
                        ui,
                        look,
                        "To use the Assistant, add your Anthropic API key in Preferences > \
                         Assistant, or set ANTHROPIC_API_KEY before starting Chamfera. The \
                         key stays on this computer, never in a design.",
                    );
                    if widgets::secondary_button(ui, look, "Open Preferences").clicked() {
                        open_prefs = true;
                    }
                } else if self.assistant.entries.is_empty() {
                    hint(
                        ui,
                        look,
                        "Describe a part, for example \"a 60×40×8 mm plate with four 5.5 mm \
                         holes 6 mm from the corners\". Claude builds it step by step in the \
                         open design; each request is one undo step.",
                    );
                }
                for (i, e) in self.assistant.entries.iter().enumerate() {
                    entry_ui(ui, look, i, e);
                }
                if let Some(a) = &self.assistant.asking {
                    egui::Frame::new()
                        .fill(look.c(|t| t.panel_alt))
                        .stroke(egui::Stroke::new(1.0, look.c(|t| t.warning)))
                        .corner_radius(egui::CornerRadius::same(widgets::R_MEDIUM))
                        .inner_margin(egui::Margin::same(S3 as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new(format!("Claude wants to run: {}", a.what))
                                    .size(T_BODY)
                                    .color(look.c(|t| t.text)),
                            );
                            ui.horizontal(|ui| {
                                if widgets::primary_button(ui, look, "Allow", true).clicked() {
                                    answer = Some(true);
                                }
                                if widgets::secondary_button(ui, look, "Deny").clicked() {
                                    answer = Some(false);
                                }
                            });
                        });
                }
                if running && self.assistant.asking.is_none() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        let what = if self.assistant.thinking {
                            "Thinking…"
                        } else {
                            "Working…"
                        };
                        hint(ui, look, what);
                    });
                }
            });

        ui.add_space(S2);
        let input_id = egui::Id::unique("assistant input");
        // Enter sends, Shift+Enter makes a new line: take the plain Enter before the text
        // box sees it.
        let focused = ui.ctx().memory(|m| m.has_focus(input_id));
        if focused && has_key && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            send = true;
        }
        let enabled = has_key && cfg!(not(target_arch = "wasm32"));
        ui.add_enabled(
            enabled,
            egui::TextEdit::multiline(&mut self.assistant.input)
                .id(input_id)
                .hint_text("Ask Claude to build or change the part…")
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .char_limit(chamfera_assistant::agent::MAX_USER_TEXT),
        );
        ui.horizontal(|ui| {
            if running {
                if widgets::primary_button(ui, look, "Stop", true).clicked() {
                    stop = true;
                }
            } else if widgets::primary_button(
                ui,
                look,
                "Send",
                enabled && !self.assistant.input.trim().is_empty(),
            )
            .on_hover_text("Send (Enter); Shift+Enter starts a new line")
            .clicked()
            {
                send = true;
            }
            if widgets::secondary_button(ui, look, "Clear")
                .on_hover_text("Start a new conversation; the design keeps what was built")
                .clicked()
            {
                clear = true;
            }
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo assistant step"))
                .on_hover_text("Undo everything the last request changed, as one step")
                .clicked()
            {
                undo = true;
            }
        });
        if close {
            self.assistant.open = false;
        }
        if open_prefs {
            self.prefs_ui.open = true;
            self.prefs_ui.page = crate::prefs_ui::Page::Assistant;
        }
        if let Some(allow) = answer {
            self.assistant_answer(allow);
        }
        if stop {
            self.assistant_stop();
        }
        if clear {
            self.assistant_clear();
        }
        if send {
            self.assistant_send();
        }
        if undo && self.assistant_can_undo() {
            self.run("edit.undo", serde_json::json!({}));
            self.assistant.undo_after = None;
            self.assistant
                .notice("Undid the last request's changes.", false);
        }
    }
}

fn hint(ui: &mut egui::Ui, look: Look, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(T_SMALL)
            .color(look.c(|t| t.text_muted)),
    );
}

/// A small status mark painted in front of a tool row: a ring while pending, a tick when
/// it ran, a cross when it failed.
fn status_mark(ui: &mut egui::Ui, look: Look, state: &ToolState) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
    let c = r.center();
    let p = ui.painter();
    match state {
        ToolState::Pending | ToolState::Asking => {
            let col = if matches!(state, ToolState::Asking) {
                look.c(|t| t.warning)
            } else {
                look.c(|t| t.text_muted)
            };
            p.circle_stroke(c, 4.5, egui::Stroke::new(1.5, col));
        }
        ToolState::Ok(_) => {
            let s = egui::Stroke::new(2.0, look.c(|t| t.success));
            p.line_segment([c + vec2(-4.5, 0.5), c + vec2(-1.5, 3.5)], s);
            p.line_segment([c + vec2(-1.5, 3.5), c + vec2(4.5, -3.5)], s);
        }
        ToolState::Failed(_) => {
            let s = egui::Stroke::new(2.0, look.c(|t| t.error));
            p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], s);
            p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], s);
        }
    }
}

fn entry_ui(ui: &mut egui::Ui, look: Look, i: usize, e: &Entry) {
    match e {
        Entry::User(text) => {
            egui::Frame::new()
                .fill(look.c(|t| t.selected))
                .corner_radius(egui::CornerRadius::same(widgets::R_MEDIUM))
                .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8 + 2))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(text)
                            .size(T_BODY)
                            .color(look.c(|t| t.text)),
                    );
                });
        }
        Entry::Assistant(text) => {
            ui.label(
                egui::RichText::new(text.trim())
                    .size(T_BODY)
                    .color(look.c(|t| t.text)),
            );
        }
        Entry::Tool {
            label,
            input,
            state,
            ..
        } => {
            let (detail, colour): (&str, Color32) = match state {
                ToolState::Pending => ("", look.c(|t| t.text_muted)),
                ToolState::Asking => ("waiting for you", look.c(|t| t.warning)),
                ToolState::Ok(s) => (s.as_str(), look.c(|t| t.text_muted)),
                ToolState::Failed(s) => (s.as_str(), look.c(|t| t.error)),
            };
            let id = ui.make_persistent_id(("assistant tool", i));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    status_mark(ui, look, state);
                    let failed = matches!(state, ToolState::Failed(_));
                    ui.label(egui::RichText::new(label).size(T_SMALL).color(if failed {
                        look.c(|t| t.error)
                    } else {
                        look.c(|t| t.text)
                    }));
                    if !detail.is_empty() && !failed {
                        ui.label(egui::RichText::new(detail).size(T_SMALL).color(colour));
                    }
                })
                .body(|ui| {
                    if !input.is_empty() {
                        ui.label(
                            egui::RichText::new(input)
                                .size(T_SMALL)
                                .monospace()
                                .color(look.c(|t| t.text_muted)),
                        );
                    }
                    if !detail.is_empty() {
                        ui.label(egui::RichText::new(detail).size(T_SMALL).color(colour));
                    }
                });
            // A failure says why right under the row, without opening it.
            if let ToolState::Failed(why) = state {
                ui.add_space(-S2);
                ui.horizontal(|ui| {
                    ui.add_space(S4 + 14.0);
                    ui.label(egui::RichText::new(why).size(T_SMALL).color(colour));
                });
            }
        }
        Entry::Notice { text, error } => {
            let c = if *error {
                look.c(|t| t.error)
            } else {
                look.c(|t| t.text_muted)
            };
            ui.label(egui::RichText::new(text).size(T_SMALL).color(c));
        }
    }
}

fn kind_label(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::Command(c)) => c.label,
        Some(Kind::Summary) => "Design summary",
        Some(Kind::Screenshot) => "Screenshot",
        None => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_assistant::script::{Replay, ScriptedReply};
    use eframe::App;
    use serde_json::json;

    fn frame(app: &mut ChamferaApp, ctx: &egui::Context, t: f64) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1400.0, 900.0),
            )),
            time: Some(t),
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
    }

    /// Frames until the request ends (at most about 10 s).
    fn until_done(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64) {
        for _ in 0..1000 {
            frame(app, ctx, *t);
            *t += 0.016;
            if !app.assistant.running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.assistant.running(), "the request never ended");
        frame(app, ctx, *t);
    }

    fn with_replies(app: &mut ChamferaApp, replies: Vec<ScriptedReply>) {
        app.assistant.transport = Some(Box::new(move || Box::new(Replay::new(&replies))));
    }

    fn box_replies() -> Vec<ScriptedReply> {
        vec![
            ScriptedReply::new("tool_use")
                .text("Sketching a 50 x 30 rectangle.")
                .tool("t1", "sketch_create", json!({ "plane": "xy" }))
                .tool(
                    "t2",
                    "sketch_rectangle",
                    json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
                ),
            ScriptedReply::new("tool_use")
                .tool(
                    "t3",
                    "solid_extrude",
                    json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
                )
                .tool(
                    "t4",
                    "solid_fillet",
                    json!({ "edges": ["no such edge"], "radius": 2 }),
                ),
            ScriptedReply::new("end_turn").text("Done: a 50 x 30 x 10 box, 15000 mm³."),
        ]
    }

    #[test]
    fn a_request_streams_the_reply_shows_tool_rows_and_builds_the_box() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.assistant.open = true;
        with_replies(&mut app, box_replies());
        let mut t = 0.0;
        frame(&mut app, &ctx, t);
        app.assistant.input = "make a 50x30x10 box".into();
        app.assistant_send();
        assert!(app.assistant.running());
        until_done(&mut app, &ctx, &mut t);
        let e = &app.assistant.entries;
        assert_eq!(e[0], Entry::User("make a 50x30x10 box".into()));
        assert_eq!(
            e[1],
            Entry::Assistant("Sketching a 50 x 30 rectangle.".into())
        );
        let rows: Vec<(&str, &ToolState)> = e
            .iter()
            .filter_map(|x| match x {
                Entry::Tool { label, state, .. } => Some((label.as_str(), state)),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[2].0, "Extrude 10 mm");
        // 50 x 30 x 10 = 15000 mm³, reported on the extrude's row.
        assert!(
            matches!(rows[2].1, ToolState::Ok(s) if s.contains("15000")),
            "{rows:?}"
        );
        assert!(
            matches!(rows[3].1, ToolState::Failed(_)),
            "the bad fillet is an error row"
        );
        assert!(matches!(
            e.last(),
            Some(Entry::Assistant(s)) if s.contains("15000")
        ));
        let v = chamfera_engine::agent::summary(&mut app.session)["bodies"][0]["volume"]
            .as_f64()
            .unwrap();
        assert!((v - 15000.0).abs() < 1e-6, "{v}");
        // The request's steps are one undo step.
        assert!(app.assistant_can_undo());
        app.run("edit.undo", json!({}));
        assert!(app.session.doc.sketches.is_empty() && app.session.doc.features.is_empty());
        assert!(!app.session.can_undo());
    }

    #[test]
    fn a_destructive_call_waits_for_allow_or_deny() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.run("param.set", json!({ "name": "w", "value": 5 }));
        with_replies(
            &mut app,
            vec![
                ScriptedReply::new("tool_use").tool("t1", "design_new", json!({})),
                ScriptedReply::new("end_turn").text("Left it."),
            ],
        );
        let mut t = 0.0;
        app.assistant.input = "start over".into();
        app.assistant_send();
        for _ in 0..500 {
            frame(&mut app, &ctx, t);
            t += 0.016;
            if app.assistant.asking.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.assistant.asking.is_some(), "no confirmation asked");
        assert_eq!(app.session.doc.parameters.len(), 1);
        app.assistant_answer(false);
        until_done(&mut app, &ctx, &mut t);
        assert_eq!(
            app.session.doc.parameters.len(),
            1,
            "denied, so nothing replaced"
        );
        assert!(app.assistant.entries.iter().any(|e| matches!(
            e,
            Entry::Tool { state: ToolState::Failed(s), .. } if s.contains("declined")
        )));
    }

    #[test]
    fn without_a_key_the_panel_explains_how_to_add_one_and_sends_nothing() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        app.assistant.open = true;
        frame(&mut app, &ctx, 0.0);
        app.assistant.input = "hello".into();
        app.assistant_send();
        assert!(!app.assistant.running());
        assert!(matches!(
            app.assistant.entries.last(),
            Some(Entry::Notice { error: true, .. })
        ));
    }
}

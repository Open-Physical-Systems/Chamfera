//! The tool-use loop.
//!
//! One user request runs [`run`]: send the conversation, stream the reply (showing text as
//! it comes), append the reply to the history exactly as it came (thinking blocks too), run
//! the tools it asks for through the [`ToolHost`] and send all their results back in one
//! user message; repeat until Claude ends its turn. The history only ever grows, so the
//! prompt cache and Claude's own earlier reasoning stay valid.
//!
//! Tools run only after a reply that stopped for `tool_use`. A refusal or a reply cut off
//! at `max_tokens` runs none of its tool calls; they are answered as not run, so the
//! history stays well formed. The cancel flag is checked between stream lines and between
//! tool calls.

use crate::api::{self, Settings, Usage};
use crate::prompt;
use crate::sse::{Delta, MessageBuilder, SseParser};
use crate::tools::{Kind, Outcome, ToolSet};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

pub use crate::tools::Outcome as ToolOutcome;

/// Most requests one user message may lead to (each tool round trip is one).
pub const MAX_REQUESTS_PER_TURN: usize = 60;
/// Most messages a conversation keeps before it asks to be cleared.
pub const MAX_MESSAGES: usize = 2000;
/// Longest user message (bytes).
pub const MAX_USER_TEXT: usize = 32 * 1024;
/// How often a request that failed for a passing reason (429, 500, 529) is tried again.
const RETRIES: usize = 2;

/// Why a transport call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The API answered with an error status; `body` is what it said.
    Http { status: u16, body: String },
    /// No answer: network, TLS or a broken stream.
    Network(String),
    /// The line callback asked to stop.
    Cancelled,
}

/// Sends one request and hands each line of the streamed answer to `line`, until the
/// stream ends or `line` returns false.
pub trait Transport: Send {
    fn send(
        &mut self,
        body: &Value,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<(), TransportError>;
}

/// One tool call Claude asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Runs tool calls, wherever the session lives (the UI thread, or a test's own).
pub trait ToolHost {
    fn call(&mut self, call: &ToolCall) -> Outcome;
}

/// What the panel shows as a request runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// More reply text.
    Text(String),
    /// Claude is thinking.
    Thinking,
    /// A tool call started streaming in.
    ToolPending { id: String, name: String },
    /// A tool call ran.
    ToolDone {
        id: String,
        ok: bool,
        summary: String,
    },
    /// The token counts of one reply.
    Usage(Usage),
    /// Something the user should know (a refusal, a cut-off reply, a retry).
    Notice { text: String, error: bool },
}

/// How a request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEnd {
    /// Claude finished its turn.
    Done,
    /// The user pressed Stop.
    Stopped,
    /// Claude declined the request.
    Refused,
    /// The reply hit the output limit.
    MaxTokens,
    /// Something went wrong; the text says what.
    Failed(String),
}

/// The conversation: the messages so far and the tool list it started with (fixed, so
/// the prompt cache holds; changed settings apply after Clear).
#[derive(Debug, Clone)]
pub struct Conversation {
    pub settings: Settings,
    pub tools: ToolSet,
    pub messages: Vec<Value>,
    pub usage: Usage,
}

impl Conversation {
    pub fn new(settings: Settings) -> Conversation {
        Conversation {
            settings,
            tools: ToolSet::new(settings.allow_file_commands, settings.screenshots),
            messages: Vec::new(),
            usage: Usage::default(),
        }
    }

    /// Adds user content, joining it to a user message already last (a request that was
    /// stopped, or tool results), so roles keep alternating.
    fn push_user(&mut self, mut blocks: Vec<Value>) {
        if let Some(last) = self.messages.last_mut()
            && last.get("role").and_then(Value::as_str) == Some("user")
            && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
        {
            content.append(&mut blocks);
            return;
        }
        self.messages
            .push(json!({ "role": "user", "content": blocks }));
    }

    /// The request body for the conversation as it stands.
    pub fn request(&self) -> Value {
        api::request_body(
            &self.settings,
            prompt::SYSTEM,
            self.tools.definitions(),
            &self.messages,
        )
    }
}

fn cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

/// Waits about `secs` seconds, returning early (true) if cancelled.
fn wait(secs: u64, cancel: &AtomicBool) -> bool {
    for _ in 0..secs * 10 {
        if cancelled(cancel) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    cancelled(cancel)
}

/// Streams one reply. Returns the message, or why there is none.
fn stream_reply(
    conv: &Conversation,
    transport: &mut dyn Transport,
    events: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Result<crate::sse::Message, TransportError> {
    let body = conv.request();
    let mut parser = SseParser::default();
    let mut builder = MessageBuilder::default();
    let mut failure: Option<String> = None;
    let mut on_line = |line: &str| -> bool {
        if cancelled(cancel) {
            return false;
        }
        let ev = match parser.line(line) {
            Ok(Some(ev)) => ev,
            Ok(None) => return true,
            Err(e) => {
                failure = Some(e);
                return false;
            }
        };
        match builder.apply(&ev) {
            Ok(Some(Delta::Text(t))) => events(Event::Text(t)),
            Ok(Some(Delta::Thinking)) => events(Event::Thinking),
            Ok(Some(Delta::ToolStart { id, name })) => events(Event::ToolPending { id, name }),
            Ok(None) => {}
            Err(e) => {
                failure = Some(e);
                return false;
            }
        }
        !builder.is_complete()
    };
    let sent = transport.send(&body, &mut on_line);
    if cancelled(cancel) {
        return Err(TransportError::Cancelled);
    }
    if let Some(e) = failure {
        return Err(TransportError::Network(e));
    }
    sent?;
    if !builder.is_complete() {
        return Err(TransportError::Network(
            "the stream ended before the message was complete".into(),
        ));
    }
    builder.finish().map_err(TransportError::Network)
}

/// The tool calls of a reply, in order.
fn tool_calls(content: &[Value]) -> Vec<ToolCall> {
    content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|b| ToolCall {
            id: b["id"].as_str().unwrap_or_default().to_owned(),
            name: b["name"].as_str().unwrap_or_default().to_owned(),
            input: b.get("input").cloned().unwrap_or(Value::Null),
        })
        .collect()
}

fn result_block(id: &str, outcome: &Outcome) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": outcome.content,
        "is_error": outcome.is_error,
    })
}

/// Answers every call of a reply as not run, so the history stays well formed.
fn answer_not_run(
    conv: &mut Conversation,
    calls: &[ToolCall],
    why: &str,
    events: &mut dyn FnMut(Event),
) {
    if calls.is_empty() {
        return;
    }
    let blocks = calls
        .iter()
        .map(|c| {
            events(Event::ToolDone {
                id: c.id.clone(),
                ok: false,
                summary: why.to_owned(),
            });
            result_block(&c.id, &Outcome::error(why))
        })
        .collect();
    conv.push_user(blocks);
}

/// Runs one user request to its end.
pub fn run(
    conv: &mut Conversation,
    user_text: &str,
    transport: &mut dyn Transport,
    host: &mut dyn ToolHost,
    events: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> RunEnd {
    let text = user_text.trim();
    if text.is_empty() {
        return RunEnd::Failed("type a request first".into());
    }
    if text.len() > MAX_USER_TEXT {
        return RunEnd::Failed(format!("a request is at most {} KB", MAX_USER_TEXT / 1024));
    }
    if conv.messages.len() >= MAX_MESSAGES {
        return RunEnd::Failed(
            "the conversation is too long: press Clear to start a new one".into(),
        );
    }
    conv.push_user(vec![json!({ "type": "text", "text": text })]);
    let mut retries = 0;
    for _ in 0..MAX_REQUESTS_PER_TURN {
        if cancelled(cancel) {
            return RunEnd::Stopped;
        }
        let msg = match stream_reply(conv, transport, events, cancel) {
            Ok(m) => m,
            Err(TransportError::Cancelled) => return RunEnd::Stopped,
            Err(TransportError::Http { status, body }) => {
                let message = api::error_message(status, &body);
                if matches!(status, 429 | 500 | 502 | 503 | 529) && retries < RETRIES {
                    retries += 1;
                    events(Event::Notice {
                        text: format!("{message} Trying again."),
                        error: false,
                    });
                    if wait(3 * retries as u64, cancel) {
                        return RunEnd::Stopped;
                    }
                    continue;
                }
                return RunEnd::Failed(message);
            }
            Err(TransportError::Network(e)) => return RunEnd::Failed(e),
        };
        retries = 0;
        conv.usage.add(msg.usage);
        events(Event::Usage(msg.usage));
        let calls = tool_calls(&msg.content);
        if !msg.content.is_empty() {
            conv.messages
                .push(json!({ "role": "assistant", "content": msg.content }));
        }
        match msg.stop_reason.as_deref() {
            Some("tool_use") => {}
            Some("end_turn") | Some("stop_sequence") => {
                answer_not_run(conv, &calls, "not run: the turn ended", events);
                return RunEnd::Done;
            }
            // A server-side pause: send the conversation again and Claude carries on.
            Some("pause_turn") => continue,
            Some("refusal") => {
                answer_not_run(conv, &calls, "not run: the request was declined", events);
                return RunEnd::Refused;
            }
            Some("max_tokens") => {
                answer_not_run(
                    conv,
                    &calls,
                    "not run: the reply was cut off at the output limit",
                    events,
                );
                return RunEnd::MaxTokens;
            }
            Some("model_context_window_exceeded") => {
                answer_not_run(conv, &calls, "not run: the context is full", events);
                return RunEnd::Failed(
                    "the conversation no longer fits the model's context: press Clear".into(),
                );
            }
            other => {
                answer_not_run(conv, &calls, "not run", events);
                return RunEnd::Failed(format!(
                    "the reply stopped for an unknown reason ({})",
                    other.unwrap_or("none")
                ));
            }
        }
        if calls.is_empty() {
            return RunEnd::Failed("the reply asked for tools but held no tool call".into());
        }
        let mut results = Vec::with_capacity(calls.len());
        let mut stopped = false;
        for call in &calls {
            let outcome = if stopped || cancelled(cancel) {
                stopped = true;
                Outcome::error("not run: the user pressed Stop")
            } else if let Some((_, raw)) = msg.bad_inputs.iter().find(|(id, _)| *id == call.id) {
                let raw: String = raw.chars().take(2000).collect();
                Outcome::error(json!({ "INVALID_JSON": raw }).to_string())
            } else {
                host.call(call)
            };
            events(Event::ToolDone {
                id: call.id.clone(),
                ok: !outcome.is_error,
                summary: outcome.summary.clone(),
            });
            results.push(result_block(&call.id, &outcome));
        }
        conv.push_user(results);
        if stopped {
            return RunEnd::Stopped;
        }
    }
    RunEnd::Failed(format!(
        "stopped after {MAX_REQUESTS_PER_TURN} requests for one message"
    ))
}

/// Runs tool calls straight on a session: for tests and the command line, where the
/// session lives on the same thread as the loop. Nothing asks for confirmation here, so
/// gated calls are refused.
pub struct DirectHost<'a> {
    pub session: &'a mut chamfera_engine::Session,
    pub tools: ToolSet,
    pub confirm_destructive: bool,
}

impl ToolHost for DirectHost<'_> {
    fn call(&mut self, call: &ToolCall) -> Outcome {
        if let Some(kind) = self.tools.kind(&call.name)
            && let crate::tools::Gate::Confirm(what) =
                crate::tools::gate(kind, &call.input, self.confirm_destructive)
        {
            return Outcome::error(format!("not run: {what} needs the user's confirmation"));
        }
        if matches!(self.tools.kind(&call.name), Some(Kind::Screenshot)) {
            return Outcome::error("screenshots are not available here");
        }
        crate::tools::execute(self.session, &self.tools, &call.name, &call.input, None)
    }
}

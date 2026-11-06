//! Server-sent events (the HTML Living Standard's `text/event-stream`) and the Messages
//! API stream built back into one message.
//!
//! A stream is `message_start`, then per content block `content_block_start`, any number
//! of `content_block_delta` (`text_delta`, `input_json_delta`, `thinking_delta`,
//! `signature_delta`) and `content_block_stop`, then `message_delta` (stop reason and
//! output tokens) and `message_stop`. `ping` events may come anywhere; an `error` event
//! ends the stream. Every byte of it is hostile: sizes are capped and nothing panics.

use crate::api::Usage;
use serde_json::{Map, Value, json};

/// Longest single event accepted (bytes of data).
pub const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;
/// Most content blocks in one message.
pub const MAX_BLOCKS: usize = 512;
/// Longest text, thinking or tool input of one block (bytes).
pub const MAX_BLOCK_BYTES: usize = 4 * 1024 * 1024;

/// One event: its `event:` name, if any, and its `data:` lines joined by newlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Reads an event stream a line at a time.
#[derive(Debug, Default)]
pub struct SseParser {
    event: Option<String>,
    data: String,
    has_data: bool,
    too_big: bool,
}

impl SseParser {
    /// Feeds one line (with or without its line ending). Returns the event a blank line
    /// completes, or an error for an event over [`MAX_EVENT_BYTES`].
    pub fn line(&mut self, raw: &str) -> Result<Option<SseEvent>, String> {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            let too_big = std::mem::take(&mut self.too_big);
            let event = self.event.take();
            let data = std::mem::take(&mut self.data);
            let had = std::mem::take(&mut self.has_data);
            if too_big {
                return Err(format!(
                    "the stream sent an event over {MAX_EVENT_BYTES} bytes"
                ));
            }
            return Ok(had.then_some(SseEvent { event, data }));
        }
        if line.starts_with(':') {
            return Ok(None); // a comment
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.chars().take(64).collect()),
            "data" => {
                if self.data.len() + value.len() + 1 > MAX_EVENT_BYTES {
                    self.too_big = true;
                } else {
                    if self.has_data {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                }
                self.has_data = true;
            }
            _ => {} // id, retry and unknown fields are not used
        }
        Ok(None)
    }
}

/// Something worth showing while the reply streams in.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// More reply text.
    Text(String),
    /// Claude started thinking (the text of thinking is not shown).
    Thinking,
    /// A tool call started streaming: its id and tool name.
    ToolStart { id: String, name: String },
}

/// A content block being put together.
#[derive(Debug, Clone)]
enum Block {
    Text(String),
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    /// Any other block (redacted thinking, ...), kept exactly as it came.
    Other(Value),
}

/// A finished message.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// The content blocks, as the history must hold them.
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
    /// Tool calls whose streamed input was not a JSON object: id and the raw text.
    pub bad_inputs: Vec<(String, String)>,
}

/// Builds a message from the stream's events.
#[derive(Debug, Default)]
pub struct MessageBuilder {
    blocks: Vec<Option<Block>>,
    stop_reason: Option<String>,
    usage: Usage,
    started: bool,
    stopped: bool,
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn push_capped(s: &mut String, more: &str) -> Result<(), String> {
    if s.len() + more.len() > MAX_BLOCK_BYTES {
        return Err(format!("a content block grew past {MAX_BLOCK_BYTES} bytes"));
    }
    s.push_str(more);
    Ok(())
}

impl MessageBuilder {
    /// True once `message_stop` arrived.
    pub fn is_complete(&self) -> bool {
        self.stopped
    }

    /// Applies one event. Returns what to show, if anything; an `error` event or a
    /// malformed one is an error.
    pub fn apply(&mut self, ev: &SseEvent) -> Result<Option<Delta>, String> {
        let v: Value = serde_json::from_str(&ev.data)
            .map_err(|e| format!("the stream sent an event that is not JSON: {e}"))?;
        let kind = str_field(&v, "type")
            .or(ev.event.as_deref())
            .unwrap_or_default();
        match kind {
            "message_start" => {
                self.started = true;
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.usage.merge(u);
                }
                Ok(None)
            }
            "content_block_start" => self.block_start(&v),
            "content_block_delta" => self.block_delta(&v),
            "content_block_stop" => Ok(None),
            "message_delta" => {
                if let Some(r) = v.get("delta").and_then(|d| str_field(d, "stop_reason")) {
                    self.stop_reason = Some(r.chars().take(64).collect());
                }
                if let Some(u) = v.get("usage") {
                    self.usage.merge(u);
                }
                Ok(None)
            }
            "message_stop" => {
                self.stopped = true;
                Ok(None)
            }
            "error" => {
                let e = v.get("error");
                let t = e.and_then(|e| str_field(e, "type")).unwrap_or("error");
                let m = e.and_then(|e| str_field(e, "message")).unwrap_or("");
                let m: String = m.chars().take(500).collect();
                Err(format!("the API reported {t}: {m}"))
            }
            // ping, and event types added later
            _ => Ok(None),
        }
    }

    fn index(v: &Value) -> Result<usize, String> {
        let i = v
            .get("index")
            .and_then(Value::as_u64)
            .ok_or("a content block event has no index")?;
        let i = usize::try_from(i).map_err(|_| "a content block index is too large")?;
        if i >= MAX_BLOCKS {
            return Err(format!("a message may hold at most {MAX_BLOCKS} blocks"));
        }
        Ok(i)
    }

    fn block_start(&mut self, v: &Value) -> Result<Option<Delta>, String> {
        let i = Self::index(v)?;
        let block = v
            .get("content_block")
            .ok_or("content_block_start has no content_block")?;
        let (b, delta) = match str_field(block, "type").unwrap_or_default() {
            "text" => {
                let t = str_field(block, "text").unwrap_or_default().to_owned();
                let d = (!t.is_empty()).then(|| Delta::Text(t.clone()));
                (Block::Text(t), d)
            }
            "thinking" => (
                Block::Thinking {
                    thinking: str_field(block, "thinking").unwrap_or_default().to_owned(),
                    signature: str_field(block, "signature").unwrap_or_default().to_owned(),
                },
                Some(Delta::Thinking),
            ),
            "tool_use" => {
                let id = str_field(block, "id").unwrap_or_default().to_owned();
                let name = str_field(block, "name").unwrap_or_default().to_owned();
                if id.is_empty() || id.len() > 256 || name.len() > 256 {
                    return Err("a tool call has a missing or overlong id or name".into());
                }
                let d = Delta::ToolStart {
                    id: id.clone(),
                    name: name.clone(),
                };
                (
                    Block::ToolUse {
                        id,
                        name,
                        json: String::new(),
                    },
                    Some(d),
                )
            }
            _ => (Block::Other(block.clone()), None),
        };
        if self.blocks.len() <= i {
            self.blocks.resize(i + 1, None);
        }
        if let Some(slot) = self.blocks.get_mut(i) {
            *slot = Some(b);
        }
        Ok(delta)
    }

    fn block_delta(&mut self, v: &Value) -> Result<Option<Delta>, String> {
        let i = Self::index(v)?;
        let delta = v.get("delta").ok_or("content_block_delta has no delta")?;
        let Some(Some(block)) = self.blocks.get_mut(i) else {
            return Err(format!("a delta for block {i}, which never started"));
        };
        let kind = str_field(delta, "type").unwrap_or_default();
        match (block, kind) {
            (Block::Text(t), "text_delta") => {
                let more = str_field(delta, "text").unwrap_or_default();
                push_capped(t, more)?;
                Ok((!more.is_empty()).then(|| Delta::Text(more.to_owned())))
            }
            (Block::ToolUse { json, .. }, "input_json_delta") => {
                push_capped(json, str_field(delta, "partial_json").unwrap_or_default())?;
                Ok(None)
            }
            (Block::Thinking { thinking, .. }, "thinking_delta") => {
                push_capped(thinking, str_field(delta, "thinking").unwrap_or_default())?;
                Ok(None)
            }
            (Block::Thinking { signature, .. }, "signature_delta") => {
                push_capped(signature, str_field(delta, "signature").unwrap_or_default())?;
                Ok(None)
            }
            // Deltas of block types we keep as they came, or new delta types: ignored.
            _ => Ok(None),
        }
    }

    /// The finished message. A stream that never started is an error.
    pub fn finish(self) -> Result<Message, String> {
        if !self.started {
            return Err("the stream ended before the message started".into());
        }
        let mut content = Vec::new();
        let mut bad_inputs = Vec::new();
        for block in self.blocks.into_iter().flatten() {
            content.push(match block {
                Block::Text(text) => json!({ "type": "text", "text": text }),
                Block::Thinking {
                    thinking,
                    signature,
                } => json!({ "type": "thinking", "thinking": thinking, "signature": signature }),
                Block::ToolUse { id, name, json } => {
                    let input = if json.trim().is_empty() {
                        Value::Object(Map::new())
                    } else {
                        match serde_json::from_str::<Value>(&json) {
                            Ok(v @ Value::Object(_)) => v,
                            _ => {
                                bad_inputs.push((id.clone(), json));
                                Value::Object(Map::new())
                            }
                        }
                    };
                    json!({ "type": "tool_use", "id": id, "name": name, "input": input })
                }
                Block::Other(v) => v,
            });
        }
        Ok(Message {
            content,
            stop_reason: self.stop_reason,
            usage: self.usage,
            bad_inputs,
        })
    }
}

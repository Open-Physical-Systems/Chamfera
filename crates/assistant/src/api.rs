//! The Messages API request: models, effort, the request body and what a reply costs.
//!
//! The request keeps everything ahead of the conversation stable (system prompt, the
//! sorted tool list, thinking and effort settings), so prompt caching reads it back on
//! every turn: one explicit breakpoint after the system prompt (tools render before it, so
//! it covers them too) and the top-level automatic breakpoint on the last message.

use serde_json::{Value, json};

/// The endpoint.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version header value.
pub const API_VERSION: &str = "2026-10-21";
/// Output limit per reply. Thinking counts toward it.
pub const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// The models offered in Preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Model {
    #[default]
    Opus,
    Sonnet,
    Haiku,
}

impl Model {
    pub const ALL: [Model; 3] = [Model::Opus, Model::Sonnet, Model::Haiku];

    /// The exact model id.
    pub fn id(self) -> &'static str {
        match self {
            Model::Opus => "claude-opus-5-5",
            Model::Sonnet => "claude-sonnet-5-5",
            Model::Haiku => "claude-haiku-5-5",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Model::Opus => "Claude Opus 5.5",
            Model::Sonnet => "Claude Sonnet 5.5",
            Model::Haiku => "Claude Haiku 5.5",
        }
    }

    pub fn parse(id: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.id() == id)
    }

    /// US dollars per million tokens: input, output, cache read, cache write (5 minute
    /// cache, 1.25 times input). Haiku's is its rate for prompts up to 100K tokens.
    pub fn prices(self) -> [f64; 4] {
        match self {
            Model::Opus => [4.0, 20.0, 0.20, 5.0],
            Model::Sonnet => [2.0, 10.0, 0.20, 2.5],
            Model::Haiku => [0.10, 0.50, 0.01, 0.125],
        }
    }
}

/// How much Claude thinks before it acts: more is slower and costs more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 5] = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::XHigh,
        Effort::Max,
    ];

    /// The `output_config.effort` value.
    pub fn key(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Effort> {
        Effort::ALL.into_iter().find(|e| e.key() == s)
    }
}

/// What one conversation runs with. The key is not here: it goes only to the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub model: Model,
    pub effort: Effort,
    pub max_tokens: u32,
    /// Offer the commands that read or write files (each call still asks the user).
    pub allow_file_commands: bool,
    /// Offer the `screenshot` tool.
    pub screenshots: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: Model::default(),
            effort: Effort::default(),
            max_tokens: DEFAULT_MAX_TOKENS,
            allow_file_commands: false,
            screenshots: true,
        }
    }
}

/// Token counts of one reply, or a running total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    /// Reads the counts a `usage` object gives, keeping the old value for any it leaves out.
    pub fn merge(&mut self, usage: &Value) {
        let get = |k: &str| usage.get(k).and_then(Value::as_u64);
        if let Some(v) = get("input_tokens") {
            self.input = v;
        }
        if let Some(v) = get("output_tokens") {
            self.output = v;
        }
        if let Some(v) = get("cache_read_input_tokens") {
            self.cache_read = v;
        }
        if let Some(v) = get("cache_creation_input_tokens") {
            self.cache_write = v;
        }
    }

    pub fn add(&mut self, other: Usage) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }

    /// Estimated cost in US dollars at the model's list prices.
    pub fn cost(&self, model: Model) -> f64 {
        let [input, output, read, write] = model.prices();
        let per = |n: u64, price: f64| n as f64 * price / 1_000_000.0;
        per(self.input, input)
            + per(self.output, output)
            + per(self.cache_read, read)
            + per(self.cache_write, write)
    }
}

/// The JSON body of one streamed request.
pub fn request_body(
    settings: &Settings,
    system: &str,
    tools: &[Value],
    messages: &[Value],
) -> Value {
    json!({
        "model": settings.model.id(),
        "max_tokens": settings.max_tokens,
        "stream": true,
        "system": [{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }],
        "tools": tools,
        "tool_choice": { "type": "auto" },
        "thinking": { "type": "adaptive" },
        "output_config": { "effort": settings.effort.key() },
        "cache_control": { "type": "ephemeral" },
        "messages": messages,
    })
}

/// The message of an API error body (`{"type": "error", "error": {"type", "message"}}`),
/// or the start of the body when it is not one.
pub fn error_message(status: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let err = parsed.as_ref().and_then(|v| v.get("error"));
    let kind = err.and_then(|e| e.get("type")).and_then(Value::as_str);
    let message = err.and_then(|e| e.get("message")).and_then(Value::as_str);
    let hint = match status {
        401 => " Check the API key in Preferences > Assistant.",
        403 => " The key may not have access to this model.",
        429 => " Too many requests: wait a moment and try again.",
        529 => " The API is overloaded: try again shortly.",
        _ => "",
    };
    match (kind, message) {
        (Some(k), Some(m)) => format!("HTTP {status} {k}: {m}.{hint}"),
        _ => {
            let start: String = body.chars().take(200).collect();
            format!("HTTP {status}: {start}{hint}")
        }
    }
}

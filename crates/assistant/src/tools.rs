//! Chamfera's commands as Claude tools, and running one on the session.
//!
//! Every command of `chamfera_engine::registry()` becomes a tool named after its id with
//! dots as underscores (`solid.extrude` is `solid_extrude`); its input schema comes from
//! the command's parameter docs (one line per parameter: `name: type (required|optional) -
//! meaning`). Two tools of our own sit beside them: `design_summary` and, when enabled,
//! `screenshot`. The list is sorted by name and fixed for a conversation, so it stays in
//! the prompt cache.
//!
//! Commands that move whole files in or out (a `path`, or `*_bytes`) are left out unless
//! the user allows them in Preferences; each such call then still asks the user. So do
//! commands that replace or delete much of the design ([`DESTRUCTIVE`]).

use chamfera_engine::{CommandSpec, Session};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Longest reply text returned to Claude (bytes); longer replies are cut with a note.
pub const MAX_RESULT_BYTES: usize = 24 * 1024;
/// The size of a screenshot (pixels).
pub const SCREENSHOT_SIZE: (u32, u32) = (960, 640);

/// Commands that replace the design or delete parts of it: the panel asks first.
pub const DESTRUCTIVE: &[&str] = &[
    "body.remove",
    "component.delete",
    "design.new",
    "design.open",
    "drawing.delete",
    "feature.delete",
    "sketch.delete",
    "solid.remove",
    "timeline.roll_to",
];

/// What a tool does.
#[derive(Clone, Copy)]
pub enum Kind {
    Command(&'static CommandSpec),
    Summary,
    Screenshot,
}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Command(c) => write!(f, "Command({})", c.id),
            Kind::Summary => f.write_str("Summary"),
            Kind::Screenshot => f.write_str("Screenshot"),
        }
    }
}

/// Whether a call may run at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Run,
    /// Ask the user first; the text says what the call would do.
    Confirm(String),
}

/// The tools of one conversation.
#[derive(Debug, Clone)]
pub struct ToolSet {
    definitions: Vec<Value>,
    kinds: BTreeMap<String, Kind>,
}

/// A command id as a tool name: `sketch.rectangle.centre` -> `sketch_rectangle_centre`.
pub fn tool_name(id: &str) -> String {
    id.replace('.', "_")
}

/// One parameter from a command's docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub schema: Value,
    pub required: bool,
}

fn type_schema(ty: &str) -> Map<String, Value> {
    let t = ty.trim().to_ascii_lowercase();
    let mut m = Map::new();
    let set = |m: &mut Map<String, Value>, v: Value| {
        m.insert("type".into(), v);
    };
    if t.starts_with("number or null") {
        set(&mut m, json!(["number", "null"]));
    } else if t.starts_with("string or number")
        || t.starts_with("number or string")
        || t.starts_with("string or integer")
    {
        set(&mut m, json!(["string", "number"]));
    } else if t.starts_with("integer") || t == "id" {
        set(&mut m, json!("integer"));
    } else if t.starts_with("number") {
        set(&mut m, json!("number"));
    } else if t.starts_with("string") {
        set(&mut m, json!("string"));
    } else if t.starts_with("boolean") {
        set(&mut m, json!("boolean"));
    } else if t.starts_with("list") || t.starts_with("array") || t.starts_with('[') {
        set(&mut m, json!("array"));
    } else if t.starts_with("object") || t.starts_with('{') {
        set(&mut m, json!("object"));
    }
    // Anything else (a plane, an axis, ...) is left open; its description says what.
    m
}

/// Reads a command's parameter docs. Lines that do not look like `names: type (...)`
/// are added to the previous parameter's description.
pub fn parse_params(doc: &str) -> Vec<Param> {
    let mut out: Vec<Param> = Vec::new();
    for line in doc.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let parsed = line.split_once(':').and_then(|(names, rest)| {
            let names: Vec<&str> = names.split(',').map(str::trim).collect();
            let ok = names.iter().all(|n| {
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
            ok.then_some((names, rest.trim()))
        });
        let Some((names, rest)) = parsed else {
            if let Some(last) = out.last_mut()
                && let Some(Value::String(d)) = last.schema.get_mut("description")
            {
                d.push(' ');
                d.push_str(line);
            }
            continue;
        };
        let (ty, after) = match rest.find('(') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        // Only a plain "(required)" makes it required: "(required unless ...)" and
        // "(required for ...)" depend on other parameters.
        let required = after.starts_with("(required)");
        for name in names {
            let mut schema = type_schema(ty);
            schema.insert("description".into(), Value::String(rest.to_owned()));
            out.push(Param {
                name: name.to_owned(),
                schema: Value::Object(schema),
                required,
            });
        }
    }
    out
}

/// The input schema of a command.
pub fn input_schema(doc: &str) -> Value {
    let params = parse_params(doc);
    let mut props = Map::new();
    let mut required = Vec::new();
    for p in params {
        if p.required && !required.contains(&p.name) {
            required.push(p.name.clone());
        }
        props.insert(p.name, p.schema);
    }
    let mut schema = json!({ "type": "object", "properties": props });
    if !required.is_empty()
        && let Some(obj) = schema.as_object_mut()
    {
        obj.insert("required".into(), json!(required));
    }
    schema
}

fn command_definition(c: &CommandSpec) -> Value {
    let effect = if c.mutates {
        "Changes the design; one undo step."
    } else {
        "Reads only; changes nothing in the design."
    };
    let params = if c.params.trim().is_empty() {
        "No parameters.".to_owned()
    } else {
        format!("Parameters:\n{}", c.params)
    };
    json!({
        "name": tool_name(c.id),
        "description": format!("{} (Chamfera command `{}`). {effect}\n{params}", c.label, c.id),
        "input_schema": input_schema(c.params),
        "eager_input_streaming": true,
    })
}

fn summary_definition() -> Value {
    json!({
        "name": "design_summary",
        "description": "A compact overview of the open design: parameters; sketches with \
            point, curve and constraint counts, degrees of freedom, status and profile areas \
            (mm², net of holes); bodies with volumes (mm³) and bounds; the timeline's \
            features. Call it to check a result against the values you worked out.",
        "input_schema": { "type": "object", "properties": {}, "additionalProperties": false },
        "strict": true,
        "eager_input_streaming": true,
    })
}

fn screenshot_definition() -> Value {
    json!({
        "name": "screenshot",
        "description": "Renders the viewport as the user sees it now (same camera) and \
            returns the picture, so you can look at what you built. Move the view first with \
            view_standard and view_fit if the part is not in view.",
        "input_schema": { "type": "object", "properties": {}, "additionalProperties": false },
        "strict": true,
        "eager_input_streaming": true,
    })
}

impl ToolSet {
    /// The tools for these settings, sorted by name.
    pub fn new(allow_file_commands: bool, screenshots: bool) -> ToolSet {
        let mut kinds = BTreeMap::new();
        let mut defs = BTreeMap::new();
        for c in chamfera_engine::registry() {
            if chamfera_engine::agent::is_file_command(c.id) && !allow_file_commands {
                continue;
            }
            // The `*_bytes` commands carry whole files as text: never useful here.
            if c.id.ends_with("_bytes") {
                continue;
            }
            let name = tool_name(c.id);
            if kinds.contains_key(&name) {
                continue; // two ids with the same name: keep the first (a test guards it)
            }
            defs.insert(name.clone(), command_definition(c));
            kinds.insert(name, Kind::Command(c));
        }
        defs.insert("design_summary".into(), summary_definition());
        kinds.insert("design_summary".into(), Kind::Summary);
        if screenshots {
            defs.insert("screenshot".into(), screenshot_definition());
            kinds.insert("screenshot".into(), Kind::Screenshot);
        }
        ToolSet {
            definitions: defs.into_values().collect(),
            kinds,
        }
    }

    /// The tool definitions for the request.
    pub fn definitions(&self) -> &[Value] {
        &self.definitions
    }

    pub fn kind(&self, name: &str) -> Option<Kind> {
        self.kinds.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

/// Whether a call must ask the user first.
pub fn gate(kind: Kind, input: &Value, confirm_destructive: bool) -> Gate {
    let Kind::Command(c) = kind else {
        return Gate::Run;
    };
    if chamfera_engine::agent::is_file_command(c.id) {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .map(|p| format!(" `{}`", p.chars().take(300).collect::<String>()))
            .unwrap_or_default();
        return Gate::Confirm(format!("{}{path}", c.label));
    }
    if confirm_destructive && DESTRUCTIVE.contains(&c.id) {
        return Gate::Confirm(row_label(kind, input));
    }
    Gate::Run
}

/// A short line for the transcript: the command's label and its main value, such as
/// "Extrude 8 mm".
pub fn row_label(kind: Kind, input: &Value) -> String {
    let c = match kind {
        Kind::Command(c) => c,
        Kind::Summary => return "Design summary".into(),
        Kind::Screenshot => return "Screenshot".into(),
    };
    let lengths = [
        "distance",
        "radius",
        "thickness",
        "height",
        "depth",
        "diameter",
    ];
    let num = |v: &Value| v.as_f64().filter(|f| f.is_finite()).map(fmt_number);
    for k in lengths {
        match input.get(k) {
            Some(v @ Value::Number(_)) => {
                if let Some(n) = num(v) {
                    return format!("{} {n} mm", c.label);
                }
            }
            Some(Value::String(s)) if !s.is_empty() => {
                return format!("{} {}", c.label, short(s, 32));
            }
            _ => {}
        }
    }
    if let Some(name) = input.get("name").and_then(Value::as_str) {
        let value = input.get("value").and_then(num).or_else(|| {
            input
                .get("expression")
                .and_then(Value::as_str)
                .map(|s| short(s, 24))
        });
        return match value {
            Some(v) => format!("{} {} = {v}", c.label, short(name, 24)),
            None => format!("{} {}", c.label, short(name, 24)),
        };
    }
    if let Some(kind) = input
        .get("constraint")
        .and_then(|k| k.get("kind"))
        .and_then(Value::as_str)
    {
        return format!("{} {}", c.label, short(kind, 24));
    }
    if let Some(plane) = input.get("plane").and_then(Value::as_str) {
        return format!("{} {}", c.label, short(plane, 16));
    }
    c.label.to_owned()
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

fn fmt_number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e12 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// What a tool call gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The `content` of the `tool_result` block (text, or an image and text).
    pub content: Vec<Value>,
    pub is_error: bool,
    /// One line for the transcript: what came back, or the error.
    pub summary: String,
}

impl Outcome {
    pub fn error(msg: impl Into<String>) -> Outcome {
        let msg = msg.into();
        Outcome {
            content: vec![json!({ "type": "text", "text": msg })],
            is_error: true,
            summary: short(&msg, 160),
        }
    }

    fn text(text: String, summary: String) -> Outcome {
        Outcome {
            content: vec![json!({ "type": "text", "text": text })],
            is_error: false,
            summary,
        }
    }
}

/// A reply as text, cut to [`MAX_RESULT_BYTES`].
fn capped_json(v: &Value) -> String {
    let s = v.to_string();
    if s.len() <= MAX_RESULT_BYTES {
        return s;
    }
    let mut end = MAX_RESULT_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… [cut: the reply has {} bytes; ask for less, such as one sketch or body]",
        s.get(..end).unwrap_or_default(),
        s.len()
    )
}

/// A one-line summary of a command reply: the volume of a body it made, else nothing.
fn reply_summary(reply: &Value) -> String {
    let volume = reply
        .get("solids")
        .and_then(Value::as_array)
        .and_then(|s| s.last())
        .and_then(|s| s.get("volume"))
        .and_then(Value::as_f64)
        .or_else(|| reply.get("volume").and_then(Value::as_f64));
    match volume {
        Some(v) if v.is_finite() => format!("volume {} mm³", fmt_number(v)),
        _ => String::new(),
    }
}

/// Renders the viewport to PNG bytes; supplied by the front end, which owns the GPU.
pub type Screenshotter<'a> = &'a mut dyn FnMut(&mut Session) -> Result<Vec<u8>, String>;

/// Runs one tool call on the session. Bad input is a tool error Claude can act on.
pub fn execute(
    session: &mut Session,
    tools: &ToolSet,
    name: &str,
    input: &Value,
    screenshot: Option<Screenshotter<'_>>,
) -> Outcome {
    let Some(kind) = tools.kind(name) else {
        return Outcome::error(format!("there is no tool named `{}`", short(name, 64)));
    };
    if !input.is_object() {
        return Outcome::error("the input must be a JSON object");
    }
    match kind {
        Kind::Command(c) => match session.execute(c.id, input) {
            Ok(reply) => Outcome::text(capped_json(&reply), reply_summary(&reply)),
            Err(e) => Outcome::error(format!("{}: {e}", c.id)),
        },
        Kind::Summary => {
            if input.as_object().is_some_and(|o| !o.is_empty()) {
                return Outcome::error("design_summary takes no input");
            }
            let v = chamfera_engine::agent::summary(session);
            let bodies = v["bodies"].as_array().map_or(0, Vec::len);
            Outcome::text(capped_json(&v), format!("{bodies} bodies"))
        }
        Kind::Screenshot => {
            let Some(shoot) = screenshot else {
                return Outcome::error("screenshots are not available here");
            };
            match shoot(session) {
                Ok(png) => Outcome {
                    content: vec![
                        json!({ "type": "image", "source": {
                            "type": "base64", "media_type": "image/png",
                            "data": chamfera_engine::agent::base64(&png) } }),
                        json!({ "type": "text", "text": format!(
                            "The viewport, {}x{} pixels.", SCREENSHOT_SIZE.0, SCREENSHOT_SIZE.1) }),
                    ],
                    is_error: false,
                    summary: format!("{} KB", png.len().div_ceil(1024)),
                },
                Err(e) => Outcome::error(e),
            }
        }
    }
}

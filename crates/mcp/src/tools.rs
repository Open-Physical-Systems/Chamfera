//! The MCP tools. Each returns a `CallToolResult`; bad arguments and failing commands are
//! tool errors (`isError: true`) with a message the agent can act on.

use crate::paths::PathGuard;
use chamfera_engine::Session;
use serde_json::{Map, Value, json};

/// Most steps in one `batch` call.
pub const MAX_BATCH_STEPS: usize = 1000;
/// Snapshot sizes (pixels per side). Small by default, so replies stay small.
pub const SNAPSHOT_MIN: u64 = 16;
pub const SNAPSHOT_MAX: u64 = 2048;
const SNAPSHOT_DEFAULT: (u64, u64) = (800, 600);

/// The bundled modelling guide.
pub const SKILL: &str = include_str!("../../../skills/chamfera-modeling/SKILL.md");

const VIEWS: &[&str] = &["iso", "front", "back", "left", "right", "top", "bottom"];

fn annotations(title: &str, read_only: bool, destructive: bool, idempotent: bool) -> Value {
    json!({
        "title": title,
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        // Only the local design and the allowed folders: nothing on the network.
        "openWorldHint": false,
    })
}

fn no_args_schema() -> Value {
    json!({ "type": "object", "additionalProperties": false })
}

/// The tool list, in a fixed order.
pub fn definitions() -> Value {
    let step = json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": "a command id, such as sketch.rectangle" },
            "params": { "type": "object", "description": "the command's parameters" }
        },
        "required": ["command"],
        "additionalProperties": false
    });
    json!([
        {
            "name": "commands",
            "title": "List Chamfera commands",
            "description": "Lists Chamfera command ids with their labels, parameter docs (one line \
                per parameter: `name: type (required|optional) - meaning`) and whether they change \
                the design. Filter with a prefix such as \"sketch.\".",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prefix": { "type": "string", "description": "only ids starting with this, e.g. \"sketch.\"" }
                },
                "additionalProperties": false
            },
            "annotations": annotations("List Chamfera commands", true, false, true),
        },
        {
            "name": "run",
            "title": "Run one Chamfera command",
            "description": "Runs one command on the open design and returns its JSON reply, or the \
                error. A failing command changes nothing. Commands can delete geometry, replace the \
                design (design.new, design.open) or overwrite files (design.save); edit.undo undoes \
                one step. File paths must lie in the allowed folders (see the server instructions).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "a command id from `commands`" },
                    "params": { "type": "object", "description": "the command's parameters; default {}" }
                },
                "required": ["command"],
                "additionalProperties": false
            },
            "annotations": annotations("Run one Chamfera command", false, true, false),
        },
        {
            "name": "batch",
            "title": "Run Chamfera commands in order",
            "description": "Runs steps in order and returns every reply, stopping at the first \
                error (earlier steps stay applied; each is one undo step). The way to build a part \
                in few calls. Ids in later steps must be worked out in advance: points, curves and \
                constraints share one counter per sketch.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": { "type": "array", "items": step, "maxItems": MAX_BATCH_STEPS }
                },
                "required": ["steps"],
                "additionalProperties": false
            },
            "annotations": annotations("Run Chamfera commands in order", false, true, false),
        },
        {
            "name": "snapshot",
            "title": "Snapshot the viewport",
            "description": "Renders the open design (bodies and sketches) to a PNG and returns it as \
                an image, so you can look at what you built. The view is fitted to the content; \
                the session's own camera is left as it was. Needs a graphics adapter (a software \
                Vulkan driver such as lavapipe works).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": { "type": "string", "enum": VIEWS, "description": "default iso" },
                    "projection": { "type": "string", "enum": ["perspective", "orthographic"] },
                    "theme": { "type": "string", "enum": ["light", "dark"], "description": "default light" },
                    "width": { "type": "integer", "minimum": SNAPSHOT_MIN, "maximum": SNAPSHOT_MAX, "description": "pixels, default 800" },
                    "height": { "type": "integer", "minimum": SNAPSHOT_MIN, "maximum": SNAPSHOT_MAX, "description": "pixels, default 600" }
                },
                "additionalProperties": false
            },
            "annotations": annotations("Snapshot the viewport", true, false, true),
        },
        {
            "name": "design_summary",
            "title": "Summarize the design",
            "description": "A compact overview: parameters; sketches with point, curve and \
                constraint counts, degrees of freedom, status and profile areas (mm², net of \
                holes); bodies with volumes and bounds; the allowed folders.",
            "inputSchema": no_args_schema(),
            "annotations": annotations("Summarize the design", true, false, true),
        },
        {
            "name": "skill_reference",
            "title": "Chamfera modelling guide",
            "description": "Returns the guide to building a sketch-based part with these tools: \
                plan, sketch, constrain, dimension, check, snapshot. Read it before modelling.",
            "inputSchema": no_args_schema(),
            "annotations": annotations("Chamfera modelling guide", true, false, true),
        },
    ])
}

/// Runs a tool. `None` means there is no tool by that name.
pub fn call(session: &mut Session, paths: &PathGuard, name: &str, args: &Value) -> Option<Value> {
    let result = match name {
        "commands" => commands(args),
        "run" => run(session, paths, args),
        "batch" => batch(session, paths, args),
        "snapshot" => snapshot(session, args),
        "design_summary" => no_args(args).map(|()| text(&summary(session, paths))),
        "skill_reference" => no_args(args).map(|()| text_raw(SKILL.to_owned())),
        _ => return None,
    };
    Some(result.unwrap_or_else(
        |msg| json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
    ))
}

type ToolResult = Result<Value, String>;

fn text_raw(t: String) -> Value {
    json!({ "content": [{ "type": "text", "text": t }], "isError": false })
}

/// A JSON value as compact text: the smallest reply an agent can still parse.
fn text(v: &Value) -> Value {
    text_raw(v.to_string())
}

fn object<'a>(args: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let obj = args
        .as_object()
        .ok_or("the arguments must be a JSON object")?;
    if let Some(k) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(if allowed.is_empty() {
            format!("unknown argument `{k}`: this tool takes none")
        } else {
            format!("unknown argument `{k}`: use {}", allowed.join(", "))
        });
    }
    Ok(obj)
}

fn no_args(args: &Value) -> Result<(), String> {
    object(args, &[]).map(|_| ())
}

fn opt_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn commands(args: &Value) -> ToolResult {
    let obj = object(args, &["prefix"])?;
    let prefix = opt_str(obj, "prefix")?.unwrap_or("");
    let list: Vec<Value> = chamfera_engine::registry()
        .into_iter()
        .filter(|c| c.id.starts_with(prefix))
        .map(|c| json!({ "id": c.id, "label": c.label, "params": c.params, "mutates": c.mutates }))
        .collect();
    if list.is_empty() {
        return Err(format!(
            "no command id starts with {prefix:?}; try \"sketch.\", \"param.\", \"body.\" or no prefix"
        ));
    }
    Ok(text(&json!({ "commands": list })))
}

/// One step's command id and parameters, checked for shape.
fn step_parts<'a>(step: &'a Value, empty: &'a Value) -> Result<(&'a str, &'a Value), String> {
    let obj = object(step, &["command", "params"])?;
    let id = obj
        .get("command")
        .and_then(Value::as_str)
        .ok_or("`command` must be a string, such as \"sketch.create\"")?;
    let params = match obj.get("params") {
        None | Some(Value::Null) => empty,
        Some(p @ Value::Object(_)) => p,
        Some(_) => return Err("`params` must be an object".into()),
    };
    Ok((id, params))
}

/// Runs one command through the path rules.
fn execute(session: &mut Session, paths: &PathGuard, id: &str, params: &Value) -> ToolResult {
    let params = paths.check_command(id, params)?;
    session
        .execute(id, &params)
        .map_err(|e| format!("{id}: {e}"))
}

fn run(session: &mut Session, paths: &PathGuard, args: &Value) -> ToolResult {
    let empty = json!({});
    let (id, params) = step_parts(args, &empty)?;
    execute(session, paths, id, params).map(|reply| text(&reply))
}

fn batch(session: &mut Session, paths: &PathGuard, args: &Value) -> ToolResult {
    let obj = object(args, &["steps"])?;
    let steps = obj
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("`steps` must be a list of {\"command\": ..., \"params\": {...}}")?;
    if steps.is_empty() || steps.len() > MAX_BATCH_STEPS {
        return Err(format!("`steps` must hold 1 to {MAX_BATCH_STEPS} steps"));
    }
    // Check every step's shape first, so a typo in step 9 does not leave steps 1-8 applied.
    let empty = json!({});
    let mut parsed = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        parsed.push(step_parts(step, &empty).map_err(|e| format!("step {}: {e}", i + 1))?);
    }
    let mut replies = Vec::with_capacity(parsed.len());
    for (i, (id, params)) in parsed.into_iter().enumerate() {
        match execute(session, paths, id, params) {
            Ok(reply) => replies.push(json!({ "step": i + 1, "command": id, "reply": reply })),
            Err(e) => {
                let out = json!({
                    "completed": i,
                    "replies": replies,
                    "error": { "step": i + 1, "command": id, "message": e },
                });
                return Ok(json!({
                    "content": [{ "type": "text", "text": out.to_string() }],
                    "isError": true,
                }));
            }
        }
    }
    Ok(text(
        &json!({ "completed": replies.len(), "replies": replies }),
    ))
}

/// Snapshot options, checked.
pub struct SnapshotOptions {
    pub view: String,
    pub projection: Option<String>,
    pub dark: bool,
    pub width: u32,
    pub height: u32,
}

fn size(obj: &Map<String, Value>, key: &str, default: u64) -> Result<u32, String> {
    let v = match obj.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| format!("`{key}` must be a whole number of pixels"))?,
    };
    if !(SNAPSHOT_MIN..=SNAPSHOT_MAX).contains(&v) {
        return Err(format!(
            "`{key}` must be {SNAPSHOT_MIN} to {SNAPSHOT_MAX} pixels"
        ));
    }
    u32::try_from(v).map_err(|_| format!("`{key}` is too large"))
}

pub fn snapshot_options(args: &Value) -> Result<SnapshotOptions, String> {
    let obj = object(args, &["view", "projection", "theme", "width", "height"])?;
    let view = opt_str(obj, "view")?.unwrap_or("iso");
    if !VIEWS.contains(&view) {
        return Err(format!("unknown view {view:?}: use {}", VIEWS.join(", ")));
    }
    let projection = opt_str(obj, "projection")?;
    if let Some(p) = projection
        && p != "perspective"
        && p != "orthographic"
    {
        return Err(format!(
            "unknown projection {p:?}: use perspective or orthographic"
        ));
    }
    let dark = match opt_str(obj, "theme")? {
        None | Some("light") => false,
        Some("dark") => true,
        Some(other) => return Err(format!("unknown theme {other:?}: use light or dark")),
    };
    Ok(SnapshotOptions {
        view: view.to_owned(),
        projection: projection.map(str::to_owned),
        dark,
        width: size(obj, "width", SNAPSHOT_DEFAULT.0)?,
        height: size(obj, "height", SNAPSHOT_DEFAULT.1)?,
    })
}

/// Renders the session to PNG bytes with the view the options ask for, then puts the
/// session's camera back, so a snapshot changes nothing.
pub fn render_png(session: &mut Session, opts: &SnapshotOptions) -> Result<Vec<u8>, String> {
    let saved = session.camera;
    let result = render_with_view(session, opts);
    session.camera = saved;
    result
}

fn render_with_view(session: &mut Session, opts: &SnapshotOptions) -> Result<Vec<u8>, String> {
    let viewport = json!({ "width": opts.width, "height": opts.height });
    let mut run = |id: &str, p: Value| {
        session
            .execute(id, &p)
            .map(|_| ())
            .map_err(|e| format!("{id}: {e}"))
    };
    if let Some(p) = &opts.projection {
        run("view.projection", json!({ "mode": p }))?;
    }
    if opts.view == "iso" {
        run("view.home", viewport)?;
    } else {
        run("view.standard", json!({ "view": opts.view }))?;
        run("view.fit", viewport)?;
    }
    let theme = if opts.dark {
        chamfera_render::Theme::dark()
    } else {
        chamfera_render::Theme::light()
    };
    let frame = chamfera_gpu::Frame {
        scene: &session.scene,
        camera: &session.camera,
        theme: &theme,
        width: opts.width,
        height: opts.height,
        scale: 1.0,
    };
    chamfera_gpu::snapshot_png(&frame).map_err(|e| match e {
        chamfera_gpu::SnapshotError::NoAdapter(_) | chamfera_gpu::SnapshotError::Device(_) => {
            format!(
                "snapshot unavailable: {e}. Install a Vulkan driver (on Linux the software \
                 one, lavapipe, is in mesa-vulkan-drivers); `design_summary` still works"
            )
        }
        other => format!("snapshot failed: {other}"),
    })
}

fn snapshot(session: &mut Session, args: &Value) -> ToolResult {
    let opts = snapshot_options(args)?;
    let png = render_png(session, &opts)?;
    Ok(json!({
        "content": [
            { "type": "image", "data": crate::base64::encode(&png), "mimeType": "image/png" },
            { "type": "text", "text": json!({
                "view": opts.view, "width": opts.width, "height": opts.height, "bytes": png.len()
            }).to_string() },
        ],
        "isError": false,
    }))
}

/// The compact overview behind `design_summary`.
pub fn summary(session: &mut Session, paths: &PathGuard) -> Value {
    let ids: Vec<u32> = session.doc.sketches.iter().map(|f| f.id).collect();
    let sketches: Vec<Value> = ids
        .into_iter()
        .map(
            |id| match session.execute("sketch.get", &json!({ "sketch": id })) {
                Ok(r) => {
                    let count = |k: &str| r.get(k).and_then(Value::as_array).map_or(0, Vec::len);
                    json!({
                        "sketch": id,
                        "name": r.get("name"),
                        "points": count("points"),
                        "curves": count("curves"),
                        "constraints": count("constraints"),
                        "dof": r.get("dof"),
                        "status": r.get("status"),
                        "profiles": r.get("profiles"),
                    })
                }
                Err(e) => json!({ "sketch": id, "error": e.to_string() }),
            },
        )
        .collect();
    let bodies: Vec<Value> = session
        .execute("body.list", &json!({}))
        .ok()
        .and_then(|r| r.get("bodies").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|b| {
            json!({
                "body": b.get("body"),
                "name": b.get("name"),
                "visible": b.get("visible"),
                "volume": b.get("volume"),
                "bounds": b.get("bounds"),
            })
        })
        .collect();
    json!({
        "parameters": session.doc.parameters,
        "active_sketch": session.active_sketch,
        "sketches": sketches,
        "bodies": bodies,
        "can_undo": session.can_undo(),
        "can_redo": session.can_redo(),
        "folders": {
            "scratch": paths.scratch().display().to_string(),
            "allowed": paths.allowed().iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        },
    })
}

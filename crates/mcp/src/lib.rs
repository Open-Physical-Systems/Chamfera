//! Layer 6: the MCP server. AI agents drive Chamfera's commands over stdio.
//!
//! Our own small JSON-RPC 2.0 implementation of the Model Context Protocol (spec revision
//! 2026-07-28, <https://modelcontextprotocol.io/specification/2026-07-28>). The server is
//! dual-era: a request carrying the per-request `_meta` fields of 2026-07-28 is served
//! statelessly; a client that opens with `initialize` (2025-11-25 and earlier) gets the
//! handshake and is then served under those legacy rules.
//!
//! The process owns one design session, like an open app window: every tool works on it.
//! That is application state, not protocol state: nothing about the protocol (version,
//! capabilities, client identity) is inferred from earlier requests on the connection.
//!
//! Every message is hostile input. Nothing here panics; a bad message gets a JSON-RPC
//! error, a bad tool argument gets a tool error (`isError: true`) the agent can act on.
//! See `docs/mcp.md`.

mod base64;
mod paths;
mod tools;

use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

pub use paths::{Access, PATH_COMMANDS, PathGuard};

/// The protocol revision whose requests carry their version in `_meta`.
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
/// Revisions that open with an `initialize` handshake, newest first.
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Longest message accepted (bytes, one line). A longer line gets an error and is skipped.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

// JSON-RPC 2.0 and MCP error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// What the server may touch on disk.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Folders the file commands may read and write (`--allow-path`, repeatable).
    pub allow: Vec<PathBuf>,
    /// The scratch folder; relative paths resolve here. `None` makes a private one in the
    /// system temp folder.
    pub scratch: Option<PathBuf>,
}

/// One MCP server: the design session plus the path rules.
pub struct Server {
    session: chamfera_engine::Session,
    paths: PathGuard,
    /// The legacy revision agreed by `initialize`, if a legacy client opened with it.
    legacy: Option<&'static str>,
}

/// A JSON-RPC error: code, message and optional data.
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// Which protocol rules a request is served under.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy,
}

impl Server {
    /// Makes a server. Fails when an allowed folder does not exist or the scratch folder
    /// cannot be made.
    pub fn new(config: Config) -> Result<Self, String> {
        let scratch = match config.scratch {
            Some(dir) => dir,
            None => paths::make_scratch()?,
        };
        Ok(Self {
            session: chamfera_engine::Session::new(),
            paths: PathGuard::new(&config.allow, &scratch)?,
            legacy: None,
        })
    }

    /// The path rules in force.
    pub fn paths(&self) -> &PathGuard {
        &self.paths
    }

    /// Handles one line of input. Returns the reply line, or `None` for notifications,
    /// responses and blank lines.
    pub fn handle_line(&mut self, line: &[u8]) -> Option<String> {
        let reply = self.handle_bytes(line)?;
        // Compact JSON escapes newlines inside strings, so a reply is always one line.
        Some(serde_json::to_string(&reply).unwrap_or_else(|_| {
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"could not encode the reply"}}"#
                .to_owned()
        }))
    }

    fn handle_bytes(&mut self, line: &[u8]) -> Option<Value> {
        let text = match std::str::from_utf8(line) {
            Ok(t) => t.trim(),
            Err(_) => {
                return Some(error_reply(
                    Value::Null,
                    RpcError::new(PARSE_ERROR, "a message must be UTF-8 text"),
                ));
            }
        };
        if text.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(text) {
            Ok(msg) => self.handle_message(&msg),
            Err(e) => Some(error_reply(
                Value::Null,
                RpcError::new(PARSE_ERROR, format!("not valid JSON: {e}")),
            )),
        }
    }

    /// Handles one parsed JSON-RPC message.
    pub fn handle_message(&mut self, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            let what = if msg.is_array() {
                "batches are not part of MCP: send one message per line"
            } else {
                "a message must be a JSON object"
            };
            return Some(error_reply(
                Value::Null,
                RpcError::new(INVALID_REQUEST, what),
            ));
        };
        // A usable id is a string or an integer; anything else cannot be echoed back.
        let id = match obj.get("id") {
            None => None,
            Some(v @ Value::String(_)) => Some(v.clone()),
            Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Some(Value::Number(n.clone())),
            Some(_) => {
                return Some(error_reply(
                    Value::Null,
                    RpcError::new(INVALID_REQUEST, "`id` must be a string or an integer"),
                ));
            }
        };
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|id| {
                error_reply(
                    id,
                    RpcError::new(INVALID_REQUEST, "`jsonrpc` must be \"2.0\""),
                )
            });
        }
        let Some(method) = obj.get("method") else {
            // A response from the client (we never send requests): nothing to answer.
            if obj.contains_key("result") || obj.contains_key("error") {
                return None;
            }
            return Some(error_reply(
                id.unwrap_or(Value::Null),
                RpcError::new(INVALID_REQUEST, "a request needs a `method`"),
            ));
        };
        let Some(method) = method.as_str() else {
            return Some(error_reply(
                id.unwrap_or(Value::Null),
                RpcError::new(INVALID_REQUEST, "`method` must be a string"),
            ));
        };
        // Notifications (no id) never get a reply: initialized, cancelled, and the rest.
        let id = id?;
        let empty = Map::new();
        let params = match obj.get("params") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(p)) => p,
            Some(_) => {
                return Some(error_reply(
                    id,
                    RpcError::new(INVALID_PARAMS, "`params` must be an object"),
                ));
            }
        };
        Some(match self.dispatch(method, params) {
            Ok((result, era)) => result_reply(id, result, era),
            Err(e) => error_reply(id, e),
        })
    }

    fn dispatch(
        &mut self,
        method: &str,
        params: &Map<String, Value>,
    ) -> Result<(Value, Era), RpcError> {
        if method == "initialize" {
            return Ok((self.initialize(params), Era::Legacy));
        }
        let era = self.era(params)?;
        let result = match method {
            "server/discover" if era == Era::Modern => json!({
                "supportedVersions": MODERN_VERSIONS,
                "capabilities": capabilities(),
                "instructions": self.instructions(),
                // Fixed for the life of the process; private because it names local folders.
                "ttlMs": 3_600_000,
                "cacheScope": "private",
            }),
            "ping" => json!({}),
            "tools/list" => {
                if params.get("cursor").is_some_and(|c| !c.is_null()) {
                    return Err(RpcError::new(
                        INVALID_PARAMS,
                        "unknown cursor: tools/list returns every tool on one page",
                    ));
                }
                let mut result = json!({ "tools": tools::definitions() });
                if era == Era::Modern
                    && let Some(r) = result.as_object_mut()
                {
                    // The tool list is fixed for the life of the process.
                    r.insert("ttlMs".into(), json!(3_600_000));
                    r.insert("cacheScope".into(), json!("public"));
                }
                result
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::new(INVALID_PARAMS, "`name` must be a string"))?;
                let empty = Value::Object(Map::new());
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => &empty,
                    Some(a @ Value::Object(_)) => a,
                    Some(_) => {
                        return Err(RpcError::new(
                            INVALID_PARAMS,
                            "`arguments` must be an object",
                        ));
                    }
                };
                tools::call(&mut self.session, &self.paths, name, args)
                    .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("Unknown tool: {name}")))?
            }
            other => {
                return Err(RpcError::new(
                    METHOD_NOT_FOUND,
                    format!("unknown method {other:?}"),
                ));
            }
        };
        Ok((result, era))
    }

    /// The legacy handshake: agree a version (the client's if we know it, else our newest
    /// legacy one) and describe the server.
    fn initialize(&mut self, params: &Map<String, Value>) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = LEGACY_VERSIONS
            .iter()
            .copied()
            .find(|v| Some(*v) == asked)
            .unwrap_or(LEGACY_VERSIONS[0]);
        self.legacy = Some(version);
        json!({
            "protocolVersion": version,
            "capabilities": capabilities(),
            "serverInfo": server_info(),
            "instructions": self.instructions(),
        })
    }

    /// Modern requests name their version in `_meta`; others need an earlier `initialize`.
    fn era(&self, params: &Map<String, Value>) -> Result<Era, RpcError> {
        let meta = params.get("_meta").and_then(Value::as_object);
        match meta.and_then(|m| m.get(META_VERSION)) {
            Some(v) => {
                let v = v.as_str().unwrap_or_default();
                if !MODERN_VERSIONS.contains(&v) {
                    return Err(RpcError {
                        code: UNSUPPORTED_PROTOCOL_VERSION,
                        message: "Unsupported protocol version".into(),
                        data: Some(json!({ "supported": MODERN_VERSIONS, "requested": v })),
                    });
                }
                if !meta
                    .and_then(|m| m.get(META_CAPABILITIES))
                    .is_some_and(Value::is_object)
                {
                    return Err(RpcError::new(
                        INVALID_PARAMS,
                        format!("`_meta` must carry `{META_CAPABILITIES}` (an object)"),
                    ));
                }
                Ok(Era::Modern)
            }
            None if self.legacy.is_some() => Ok(Era::Legacy),
            None => Err(RpcError::new(
                INVALID_PARAMS,
                format!(
                    "`_meta` must carry `{META_VERSION}` ({}); clients of {} and earlier \
                     send `initialize` first",
                    MODERN_VERSIONS.join(", "),
                    LEGACY_VERSIONS[0]
                ),
            )),
        }
    }

    fn instructions(&self) -> String {
        format!(
            "Chamfera is a parametric 3D CAD application; these tools run its commands on one \
             open design. Call `skill_reference` first for the modelling guide. `commands` \
             lists command ids and their parameters; `batch` runs many steps in one call; \
             `design_summary` checks the result; `snapshot` shows it. Units are mm and \
             radians, Z is up. File paths must be inside {}; relative paths go to the \
             scratch folder {}.",
            self.paths.describe(),
            self.paths.scratch().display()
        )
    }
}

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

fn server_info() -> Value {
    json!({ "name": "chamfera-mcp", "title": "Chamfera", "version": env!("CARGO_PKG_VERSION") })
}

fn result_reply(id: Value, mut result: Value, era: Era) -> Value {
    if era == Era::Modern
        && let Some(r) = result.as_object_mut()
    {
        r.insert("resultType".into(), json!("complete"));
        r.insert("_meta".into(), json!({ META_SERVER_INFO: server_info() }));
    }
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: Value, e: RpcError) -> Value {
    let mut error = json!({ "code": e.code, "message": e.message });
    if let (Some(data), Some(obj)) = (e.data, error.as_object_mut()) {
        obj.insert("data".into(), data);
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// Reads one line of at most `limit` bytes into `buf` (without the newline). Returns
/// `Ok(None)` at end of input and `Ok(Some(false))` for a line that was too long (its
/// rest is skipped).
fn read_line_limited(
    input: &mut impl BufRead,
    buf: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<Option<bool>> {
    buf.clear();
    let mut fits = true;
    let mut read_any = false;
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if read_any { Some(fits) } else { None });
        }
        read_any = true;
        let (part, done) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (&chunk[..i], Some(i + 1)),
            None => (chunk, None),
        };
        if fits && buf.len() + part.len() <= limit {
            buf.extend_from_slice(part);
        } else {
            fits = false;
            buf.clear();
        }
        let used = done.unwrap_or(chunk.len());
        input.consume(used);
        if done.is_some() {
            return Ok(Some(fits));
        }
    }
}

/// Serves newline-delimited JSON-RPC from `input` to `output` until the input ends.
pub fn serve(
    server: &mut Server,
    mut input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    while let Some(fits) = read_line_limited(&mut input, &mut buf, MAX_MESSAGE_BYTES)? {
        let reply = if fits {
            server.handle_line(&buf)
        } else {
            serde_json::to_string(&error_reply(
                Value::Null,
                RpcError::new(
                    INVALID_REQUEST,
                    format!("a message is at most {MAX_MESSAGE_BYTES} bytes"),
                ),
            ))
            .ok()
        };
        if let Some(line) = reply {
            output.write_all(line.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// `chamfera-cli mcp`: serves stdin to stdout. Logs go to stderr.
pub fn serve_stdio(config: Config) -> Result<(), String> {
    let mut server = Server::new(config)?;
    eprintln!(
        "chamfera-mcp {}: allowed folders {}; scratch {}",
        env!("CARGO_PKG_VERSION"),
        server.paths.describe(),
        server.paths.scratch().display()
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(&mut server, stdin.lock(), stdout.lock()).map_err(|e| format!("stdio: {e}"))
}

#[cfg(test)]
mod tests;

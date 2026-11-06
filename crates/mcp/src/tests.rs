use super::*;
use crate::tools::{SKILL, render_png, snapshot_options};
use serde_json::json;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

/// A fresh folder under the system temp folder, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("chamfera-mcp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn sub(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server whose scratch is `<tmp>/scratch` and which may also use `<tmp>/allowed`;
/// `<tmp>/outside` exists but is not allowed.
fn server(tmp: &TempDir) -> Server {
    let scratch = tmp.sub("scratch");
    let allowed = tmp.sub("allowed");
    tmp.sub("outside");
    Server::new(Config {
        allow: vec![allowed],
        scratch: Some(scratch),
    })
    .unwrap()
}

fn send(s: &mut Server, msg: Value) -> Option<Value> {
    s.handle_line(msg.to_string().as_bytes())
        .map(|line| serde_json::from_str(&line).unwrap())
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// A server after the legacy handshake.
fn initialized(tmp: &TempDir) -> Server {
    let mut s = server(tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } }),
    )
    .unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    assert!(
        send(
            &mut s,
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
        )
        .is_none()
    );
    s
}

/// Calls a tool and returns (isError, first text, whole result).
fn call(s: &mut Server, name: &str, args: Value) -> (bool, String, Value) {
    let r = send(
        s,
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": name, "arguments": args } }),
    )
    .unwrap();
    let result = r.get("result").cloned().unwrap_or_else(|| panic!("{r}"));
    let text = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "text")
        .and_then(|c| c["text"].as_str())
        .unwrap_or("")
        .to_owned();
    (result["isError"] == true, text, result)
}

fn ok_json(s: &mut Server, name: &str, args: Value) -> Value {
    let (err, text, _) = call(s, name, args);
    assert!(!err, "{name} failed: {text}");
    serde_json::from_str(&text).unwrap()
}

fn plate_steps() -> Value {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sketch-plate.json"),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

// ---- JSON-RPC framing and the protocol --------------------------------------------------

#[test]
fn the_legacy_handshake_agrees_a_version_and_names_the_server() {
    let tmp = TempDir::new("handshake");
    let mut s = server(&tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "t", "version": "1" } } }),
    )
    .unwrap();
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "chamfera-mcp");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    assert!(
        r["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("skill_reference")
    );
    // A version we do not know gets our newest legacy one.
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": "x", "method": "initialize",
                "params": { "protocolVersion": "1999-01-01" } }),
    )
    .unwrap();
    assert_eq!(r["id"], "x");
    assert_eq!(r["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
}

#[test]
fn tools_list_names_six_tools_with_honest_annotations() {
    let tmp = TempDir::new("list");
    let mut s = initialized(&tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "commands",
            "run",
            "batch",
            "snapshot",
            "design_summary",
            "skill_reference"
        ]
    );
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        let a = &t["annotations"];
        let changes = matches!(t["name"].as_str(), Some("run" | "batch"));
        assert_eq!(a["readOnlyHint"], !changes, "{}", t["name"]);
        assert_eq!(a["destructiveHint"], changes, "{}", t["name"]);
        assert_eq!(a["openWorldHint"], false);
    }
    // Legacy replies carry no modern fields.
    assert!(r["result"].get("resultType").is_none());
}

#[test]
fn tools_call_runs_a_command_and_reports_failures_as_tool_errors() {
    let tmp = TempDir::new("call");
    let mut s = initialized(&tmp);
    let reply = ok_json(
        &mut s,
        "run",
        json!({ "command": "param.set", "params": { "name": "width", "value": 80 } }),
    );
    assert_eq!(reply["value"], 80.0);
    let (err, text, _) = call(
        &mut s,
        "run",
        json!({ "command": "param.set", "params": { "name": "1bad", "value": 1 } }),
    );
    assert!(err);
    assert!(text.starts_with("param.set:"), "{text}");
    // An unknown tool is a protocol error.
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "nope" } }),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn modern_requests_carry_their_version_and_need_no_handshake() {
    let tmp = TempDir::new("modern");
    let mut s = server(&tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover",
                "params": { "_meta": modern_meta() } }),
    )
    .unwrap();
    let res = &r["result"];
    assert_eq!(res["resultType"], "complete");
    assert_eq!(res["supportedVersions"], json!(["2026-07-28"]));
    assert!(res["capabilities"]["tools"].is_object());
    assert_eq!(
        res["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "chamfera-mcp"
    );
    assert!(res["ttlMs"].is_number() && res["cacheScope"].is_string());

    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": { "_meta": modern_meta() } }),
    )
    .unwrap();
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 6);
    assert_eq!(r["result"]["cacheScope"], "public");

    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "_meta": modern_meta(), "name": "commands", "arguments": { "prefix": "app." } } }),
    )
    .unwrap();
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["isError"], false);
}

#[test]
fn an_unsupported_version_lists_the_supported_ones() {
    let tmp = TempDir::new("version");
    let mut s = server(&tmp);
    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "_meta": meta } }),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(r["error"]["data"]["supported"], json!(["2026-07-28"]));
}

#[test]
fn a_request_without_version_or_handshake_is_invalid_params() {
    let tmp = TempDir::new("nometa");
    let mut s = server(&tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize")
    );
    // Modern metadata without client capabilities is malformed too.
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": { "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn an_unknown_method_is_method_not_found() {
    let tmp = TempDir::new("unknown");
    let mut s = initialized(&tmp);
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" }),
    )
    .unwrap();
    assert_eq!(r["id"], 9);
    assert_eq!(r["error"]["code"], -32601);
}

#[test]
fn malformed_json_gets_a_parse_error_with_a_null_id() {
    let tmp = TempDir::new("parse");
    let mut s = server(&tmp);
    for bad in [
        &b"{"[..],
        b"{\"jsonrpc\": \"2.0\", \"id\": 1,",
        b"nul",
        b"\xff\xfe",
    ] {
        let r: Value = serde_json::from_str(&s.handle_line(bad).unwrap()).unwrap();
        assert_eq!(r["error"]["code"], -32700, "{bad:?}");
        assert_eq!(r["id"], Value::Null);
    }
}

#[test]
fn invalid_requests_get_invalid_request() {
    let tmp = TempDir::new("invalid");
    let mut s = initialized(&tmp);
    for bad in [
        json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]),
        json!(null),
        json!(5),
        json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1.5, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": {}, "method": "ping" }),
        json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": 5 }),
        json!({ "jsonrpc": "2.0", "id": 1 }),
    ] {
        let r = send(&mut s, bad.clone()).unwrap();
        assert_eq!(r["error"]["code"], -32600, "{bad}");
    }
    let r = send(
        &mut s,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": [1] }),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn notifications_and_client_responses_get_no_reply() {
    let tmp = TempDir::new("notify");
    let mut s = initialized(&tmp);
    for msg in [
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 3 } }),
        json!({ "jsonrpc": "2.0", "method": "tools/call", "params": { "name": "run",
                "arguments": { "command": "param.set", "params": { "name": "a", "value": 1 } } } }),
        json!({ "jsonrpc": "2.0", "method": "no/such/thing" }),
        json!({ "jsonrpc": "2.0", "id": 4, "result": {} }),
    ] {
        assert!(send(&mut s, msg.clone()).is_none(), "{msg}");
    }
    // A notification is not run either: the parameter was not set.
    let summary = ok_json(&mut s, "design_summary", json!({}));
    assert_eq!(summary["parameters"], json!({}));
    assert!(s.handle_line(b"   ").is_none());
}

#[test]
fn serve_answers_each_line_on_its_own_line() {
    let tmp = TempDir::new("serve");
    let mut s = server(&tmp);
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
        "\r\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"skill_reference"}}"#,
        "\n",
        "{oops\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#, // no final newline
    );
    let mut out = Vec::new();
    serve(&mut s, input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["id"], 1);
    // The skill text has many newlines; the reply is still one line.
    assert_eq!(lines[1]["result"]["content"][0]["text"], SKILL);
    assert_eq!(lines[2]["error"]["code"], -32700);
    assert_eq!(lines[3]["id"], 3);
}

#[test]
fn an_overlong_line_is_refused_and_the_next_one_still_served() {
    let mut buf = Vec::new();
    let mut input = &b"0123456789abc\nshort\n"[..];
    assert_eq!(
        read_line_limited(&mut input, &mut buf, 8).unwrap(),
        Some(false)
    );
    assert_eq!(
        read_line_limited(&mut input, &mut buf, 8).unwrap(),
        Some(true)
    );
    assert_eq!(buf, b"short");
    assert_eq!(read_line_limited(&mut input, &mut buf, 8).unwrap(), None);

    let tmp = TempDir::new("overlong");
    let mut s = initialized(&tmp);
    let mut input = vec![b' '; MAX_MESSAGE_BYTES + 10];
    input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n");
    let mut out = Vec::new();
    serve(&mut s, &input[..], &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["error"]["code"], -32600);
    assert_eq!(lines[1]["id"], 5);
}

// ---- Tools ------------------------------------------------------------------------------

/// examples/sketch-plate.json: an 80 x 50 rectangle with two r = 8 circles inside and a
/// construction line. Profiles, by hand: the plate 80 * 50 - 2 * (pi * 8^2) = 4000 - 128 pi
/// with two holes, and each circle pi * 8^2 = 64 pi.
#[test]
fn batch_builds_the_plate_sketch_and_the_summary_reports_its_profiles() {
    let tmp = TempDir::new("plate");
    let mut s = initialized(&tmp);
    let out = ok_json(&mut s, "batch", json!({ "steps": plate_steps() }));
    assert_eq!(out["completed"], 10);
    assert_eq!(out["replies"][1]["reply"]["points"], json!([1, 2, 3, 4]));

    let summary = ok_json(&mut s, "design_summary", json!({}));
    let sketches = summary["sketches"].as_array().unwrap();
    assert_eq!(sketches.len(), 1);
    let profiles = sketches[0]["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 3);
    let mut areas: Vec<(f64, u64)> = profiles
        .iter()
        .map(|p| (p["area"].as_f64().unwrap(), p["holes"].as_u64().unwrap()))
        .collect();
    areas.sort_by(|a, b| b.0.total_cmp(&a.0));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(near(areas[0].0, 4000.0 - 2.0 * 64.0 * PI), "{areas:?}");
    assert_eq!(areas[0].1, 2);
    assert!(near(areas[1].0, 64.0 * PI) && near(areas[2].0, 64.0 * PI));
    // Rectangle 8 DOF - 4 (h/v) - 2 (lengths) - 2 (fix) = 0; two circles 3 each; the
    // construction line 4: 10 in all.
    assert_eq!(sketches[0]["dof"], 10);
    assert_eq!(sketches[0]["status"], "UnderConstrained");
    assert_eq!(summary["active_sketch"], 1);
}

#[test]
fn a_batch_stops_at_the_first_error_and_keeps_the_earlier_steps() {
    let tmp = TempDir::new("batchstop");
    let mut s = initialized(&tmp);
    let (err, text, _) = call(
        &mut s,
        "batch",
        json!({ "steps": [
            { "command": "param.set", "params": { "name": "a", "value": 1 } },
            { "command": "sketch.line", "params": { "x1": 0, "y1": 0, "x2": 1, "y2": 1 } },
            { "command": "param.set", "params": { "name": "b", "value": 2 } },
        ] }),
    );
    assert!(err);
    let out: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(out["completed"], 1);
    assert_eq!(out["error"]["step"], 2);
    let params = ok_json(&mut s, "design_summary", json!({}))["parameters"].clone();
    assert!(params.get("a").is_some() && params.get("b").is_none());
    // A malformed step anywhere runs nothing.
    let (err, text, _) = call(
        &mut s,
        "batch",
        json!({ "steps": [
            { "command": "param.set", "params": { "name": "c", "value": 1 } },
            { "comand": "param.list" },
        ] }),
    );
    assert!(err && text.starts_with("step 2:"), "{text}");
    let params = ok_json(&mut s, "design_summary", json!({}))["parameters"].clone();
    assert!(params.get("c").is_none());
}

/// The worked example in the skill is real: it runs and ends fully constrained with the
/// areas the skill derives (4000 - 128 pi, and 64 pi twice).
#[test]
fn the_skill_worked_example_runs_and_is_fully_constrained() {
    let start = SKILL.find("```json\n").unwrap() + "```json\n".len();
    let len = SKILL[start..].find("```").unwrap();
    let steps: Value = serde_json::from_str(&SKILL[start..start + len]).unwrap();
    let tmp = TempDir::new("skillexample");
    let mut s = initialized(&tmp);
    let out = ok_json(&mut s, "batch", json!({ "steps": steps }));
    let last = out["replies"].as_array().unwrap().last().unwrap()["reply"].clone();
    assert_eq!(last["dof"], 0);
    assert_eq!(last["status"], "FullyConstrained");
    let mut areas: Vec<f64> = last["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["area"].as_f64().unwrap())
        .collect();
    areas.sort_by(|a, b| b.total_cmp(a));
    assert_eq!(areas.len(), 3);
    assert!((areas[0] - (4000.0 - 128.0 * PI)).abs() < 1e-6);
    assert!((areas[1] - 64.0 * PI).abs() < 1e-6 && (areas[2] - 64.0 * PI).abs() < 1e-6);
}

#[test]
fn commands_filters_by_prefix() {
    let tmp = TempDir::new("commands");
    let mut s = initialized(&tmp);
    let out = ok_json(&mut s, "commands", json!({ "prefix": "sketch." }));
    let list = out["commands"].as_array().unwrap();
    assert!(!list.is_empty());
    assert!(
        list.iter()
            .all(|c| c["id"].as_str().unwrap().starts_with("sketch."))
    );
    let all = ok_json(&mut s, "commands", json!({}));
    assert_eq!(
        all["commands"].as_array().unwrap().len(),
        chamfera_engine::registry().len()
    );
    let (err, _, _) = call(&mut s, "commands", json!({ "prefix": "zzz." }));
    assert!(err);
}

#[test]
fn skill_reference_returns_the_bundled_skill() {
    let tmp = TempDir::new("skillref");
    let mut s = initialized(&tmp);
    let (err, text, _) = call(&mut s, "skill_reference", json!({}));
    assert!(!err);
    assert!(text.starts_with("---\nname: chamfera-modeling\n"));
}

/// Every command id the skill mentions exists, so the guide cannot rot.
#[test]
fn every_command_id_in_the_modeling_skill_exists() {
    let ids: Vec<&str> = chamfera_engine::registry().iter().map(|c| c.id).collect();
    let areas: Vec<&str> = ids.iter().filter_map(|i| i.split('.').next()).collect();
    let mut checked = 0;
    for word in SKILL
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'))
    {
        let word = word.trim_matches('.');
        let Some((area, _)) = word.split_once('.') else {
            continue;
        };
        if !areas.contains(&area) {
            continue;
        }
        assert!(
            ids.contains(&word),
            "skills/chamfera-modeling/SKILL.md names `{word}`, which is not a command"
        );
        checked += 1;
    }
    assert!(
        checked > 10,
        "only {checked} command ids found in the skill"
    );
}

#[test]
fn hostile_tool_arguments_never_panic() {
    let tmp = TempDir::new("hostile");
    let mut s = initialized(&tmp);
    let deep = (0..200).fold(json!(1), |v, _| json!([v]));
    let long = "x".repeat(100_000);
    let hostile = [
        json!(null),
        json!(1),
        json!("x"),
        json!([]),
        json!({ "": 1 }),
        json!({ "command": 5 }),
        json!({ "command": "" }),
        json!({ "command": long.clone() }),
        json!({ "command": "param.set", "params": [] }),
        json!({ "command": "param.set", "params": "x" }),
        json!({ "command": "design.save", "params": { "path": 5 } }),
        json!({ "command": "design.open", "params": { "path": "\0" } }),
        json!({ "command": "body.import_stl", "params": { "path": long.clone() } }),
        json!({ "steps": "x" }),
        json!({ "steps": [] }),
        json!({ "steps": [1, null, "x"] }),
        json!({ "steps": [{ "command": "nope" }] }),
        json!({ "steps": vec![json!({ "command": "app.about" }); MAX_BATCH_STEPS_PLUS] }),
        json!({ "steps": [deep.clone()] }),
        json!({ "prefix": 7 }),
        json!({ "prefix": long }),
        json!({ "view": "sideways" }),
        json!({ "width": -1 }),
        json!({ "width": 0 }),
        json!({ "width": 1e308 }),
        json!({ "height": 100_000 }),
        json!({ "width": 64.5 }),
        json!({ "theme": "neon" }),
        json!({ "projection": 1 }),
        json!({ "extra": true }),
        deep,
    ];
    for name in [
        "commands",
        "run",
        "batch",
        "snapshot",
        "design_summary",
        "skill_reference",
        "nope",
    ] {
        for args in &hostile {
            // `null` arguments mean "no arguments", which for snapshot is a valid render;
            // every other case above is invalid for it.
            if name == "snapshot" && args.is_null() {
                continue;
            }
            let r = send(
                &mut s,
                json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": { "name": name, "arguments": args } }),
            )
            .unwrap();
            assert!(r.get("result").is_some() || r.get("error").is_some());
        }
    }
    // Every command, with hostile parameters, through `run`.
    for c in chamfera_engine::registry() {
        for params in [
            json!({}),
            json!({ "path": "../../../../etc/passwd" }),
            json!({ "path": "/etc/passwd", "x": 1e308, "sketch": -1 }),
            json!({ "constraint": { "kind": "length", "line": 999, "value": -1 } }),
        ] {
            let _ = call(&mut s, "run", json!({ "command": c.id, "params": params }));
        }
    }
}

const MAX_BATCH_STEPS_PLUS: usize = crate::tools::MAX_BATCH_STEPS + 1;

// ---- Path rules -------------------------------------------------------------------------

fn run_path(s: &mut Server, command: &str, path: &str) -> (bool, String) {
    let (err, text, _) = call(
        s,
        "run",
        json!({ "command": command, "params": { "path": path } }),
    );
    (err, text)
}

#[test]
fn files_inside_the_scratch_and_allowed_folders_work() {
    let tmp = TempDir::new("pathsok");
    let mut s = initialized(&tmp);
    ok_json(
        &mut s,
        "run",
        json!({ "command": "param.set", "params": { "name": "w", "value": 3 } }),
    );
    // Relative paths go to the scratch folder.
    let (err, text) = run_path(&mut s, "design.save", "part.chamfera");
    assert!(!err, "{text}");
    assert!(tmp.path().join("scratch/part.chamfera").is_file());
    let (err, text) = run_path(&mut s, "design.open", "part.chamfera");
    assert!(!err, "{text}");
    let allowed = tmp.path().join("allowed/b.chamfera");
    let (err, text) = run_path(&mut s, "design.save", allowed.to_str().unwrap());
    assert!(!err, "{text}");
    assert!(allowed.is_file());
    // A `.` component is harmless.
    let (err, text) = run_path(&mut s, "design.open", "./part.chamfera");
    assert!(!err, "{text}");
}

#[test]
fn dot_dot_escapes_are_refused() {
    let tmp = TempDir::new("dotdot");
    let mut s = initialized(&tmp);
    let sneaky = format!("{}/allowed/../outside/x.chamfera", tmp.path().display());
    for p in [
        "../outside/x.chamfera",
        "../../x.chamfera",
        "a/../../x",
        sneaky.as_str(),
    ] {
        let (err, text) = run_path(&mut s, "design.save", p);
        assert!(err && text.contains("refused"), "{p}: {text}");
    }
    assert!(!tmp.path().join("outside/x.chamfera").exists());
}

#[test]
fn absolute_paths_outside_the_allowed_folders_are_refused() {
    let tmp = TempDir::new("absolute");
    let mut s = initialized(&tmp);
    let outside = tmp.path().join("outside/x.chamfera");
    let (err, text) = run_path(&mut s, "design.save", outside.to_str().unwrap());
    assert!(err && text.contains("refused"), "{text}");
    assert!(!outside.exists());
    // A file that exists outside cannot be read either, by any file command.
    std::fs::write(&outside, "{}").unwrap();
    for c in ["design.open", "body.import_stl"] {
        let (err, text) = run_path(&mut s, c, outside.to_str().unwrap());
        assert!(err && text.contains("refused"), "{c}: {text}");
    }
    // A sibling whose name starts like an allowed folder is not inside it.
    let sibling = tmp.sub("allowed2").join("x.chamfera");
    let (err, text) = run_path(&mut s, "design.save", sibling.to_str().unwrap());
    assert!(err && text.contains("refused"), "{text}");
    // The root and system folders are refused.
    let (err, _) = run_path(&mut s, "design.open", "/etc/passwd");
    assert!(err);
    let (err, _) = run_path(&mut s, "design.save", "/");
    assert!(err);
}

#[cfg(unix)]
#[test]
fn symlinks_leaving_the_allowed_folders_are_refused() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new("symlink");
    let mut s = initialized(&tmp);
    let target = tmp.path().join("outside/secret.chamfera");
    std::fs::write(&target, "{}").unwrap();
    let scratch = tmp.path().join("scratch");
    // A link to a file outside: neither read nor written through.
    symlink(&target, scratch.join("link.chamfera")).unwrap();
    for c in ["design.open", "design.save", "body.import_stl"] {
        let (err, text) = run_path(&mut s, c, "link.chamfera");
        assert!(err && text.contains("refused"), "{c}: {text}");
    }
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
    // A dangling link pointing outside cannot create the file there.
    let missing = tmp.path().join("outside/new.chamfera");
    symlink(&missing, scratch.join("dangling.chamfera")).unwrap();
    let (err, text) = run_path(&mut s, "design.save", "dangling.chamfera");
    assert!(err && text.contains("refused"), "{text}");
    assert!(!missing.exists());
    // A linked folder that leads outside.
    symlink(tmp.path().join("outside"), scratch.join("out")).unwrap();
    for c in ["design.save", "design.open"] {
        let (err, text) = run_path(&mut s, c, "out/secret.chamfera");
        assert!(err && text.contains("refused"), "{c}: {text}");
    }
    // A link that stays inside is fine.
    let (err, text) = run_path(&mut s, "design.save", "real.chamfera");
    assert!(!err, "{text}");
    symlink(
        scratch.join("real.chamfera"),
        scratch.join("inside.chamfera"),
    )
    .unwrap();
    let (err, text) = run_path(&mut s, "design.open", "inside.chamfera");
    assert!(!err, "{text}");
}

#[test]
fn every_command_that_takes_a_path_is_guarded() {
    for c in chamfera_engine::registry() {
        let guarded = PATH_COMMANDS.iter().any(|(id, _)| *id == c.id);
        assert_eq!(
            crate::paths::takes_path(c.params),
            guarded,
            "{}: params {:?}",
            c.id,
            c.params
        );
    }
    for (id, _) in PATH_COMMANDS {
        assert!(chamfera_engine::cmd::find(id).is_some(), "{id}");
    }
}

#[test]
fn a_missing_allowed_folder_is_an_error_at_start() {
    let tmp = TempDir::new("missing");
    let r = Server::new(Config {
        allow: vec![tmp.path().join("nope")],
        scratch: Some(tmp.sub("scratch")),
    });
    assert!(r.is_err());
}

// ---- Snapshot ---------------------------------------------------------------------------

/// GPU tests skip on machines without any adapter, unless `CHAMFERA_REQUIRE_GPU=1` (set in
/// CI, which installs a software adapter), where a missing GPU is a failure. Same rule as
/// `skip_without_gpu` in crates/gpu/src/snapshot.rs.
fn skip_without_gpu(reason: &str) {
    if std::env::var("CHAMFERA_REQUIRE_GPU").is_ok_and(|v| v == "1") {
        panic!("CHAMFERA_REQUIRE_GPU=1 but no GPU: {reason}");
    }
    eprintln!("skipped (no GPU): {reason}");
}

/// The PNG's size from its IHDR chunk: bytes 16-19 width, 20-23 height, big-endian.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[1..4], b"PNG");
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    (be(16), be(20))
}

#[test]
fn snapshot_returns_a_png_image_of_the_requested_size() {
    let tmp = TempDir::new("snapshot");
    let mut s = initialized(&tmp);
    ok_json(&mut s, "batch", json!({ "steps": plate_steps() }));
    let camera = s.session.camera;
    let opts = snapshot_options(&json!({ "view": "top", "width": 96, "height": 64 })).unwrap();
    match render_png(&mut s.session, &opts) {
        Ok(png) => assert_eq!(png_size(&png), (96, 64)),
        Err(e) if e.starts_with("snapshot unavailable") => {
            skip_without_gpu(&e);
            return;
        }
        Err(e) => panic!("{e}"),
    }
    // The snapshot leaves the session's own camera as it was.
    assert_eq!(s.session.camera, camera);
    let (err, _, result) = call(
        &mut s,
        "snapshot",
        json!({ "view": "front", "width": 64, "height": 48, "theme": "dark" }),
    );
    assert!(!err);
    let image = &result["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    let data = image["data"].as_str().unwrap();
    // 4 base64 characters carry 3 bytes; "iVBORw0KGgo" is the PNG signature encoded.
    assert!(data.starts_with("iVBORw0KGgo"), "{}", &data[..16]);
    assert_eq!(s.session.camera, camera);
}

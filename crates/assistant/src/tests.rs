//! Tests with a scripted server: no test talks to the real API.

use crate::agent::{self, DirectHost, Event, RunEnd, TransportError};
use crate::api::{Effort, Model, Settings, Usage};
use crate::key::ApiKey;
use crate::sse::{Delta, MessageBuilder, SseParser};
use crate::tools::{self, Gate, Kind, ToolSet};
use crate::{Conversation, Transport};
use chamfera_engine::Session;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

// ---- A scripted server -----------------------------------------------------------------

/// SSE text for one event.
fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// One reply as a stream: `blocks` are (block start, deltas) pairs; text is split into
/// small deltas and tool input into JSON fragments, as the API sends them.
struct Reply {
    blocks: Vec<(Value, Vec<Value>)>,
    stop: &'static str,
}

impl Reply {
    fn new(stop: &'static str) -> Reply {
        Reply {
            blocks: Vec::new(),
            stop,
        }
    }

    fn thinking(mut self) -> Reply {
        self.blocks.push((
            json!({ "type": "thinking", "thinking": "" }),
            vec![json!({ "type": "signature_delta", "signature": "c2lnbmF0dXJl" })],
        ));
        self
    }

    fn text(mut self, text: &str) -> Reply {
        let deltas = text
            .split_inclusive(' ')
            .map(|t| json!({ "type": "text_delta", "text": t }))
            .collect();
        self.blocks
            .push((json!({ "type": "text", "text": "" }), deltas));
        self
    }

    fn tool(self, id: &str, name: &str, input: Value) -> Reply {
        let s = input.to_string();
        self.raw_tool(id, name, &s)
    }

    /// A tool call whose input is streamed as this text, valid JSON or not.
    fn raw_tool(mut self, id: &str, name: &str, input: &str) -> Reply {
        let chars: Vec<char> = input.chars().collect();
        let deltas = chars
            .chunks(7)
            .map(|c| json!({ "type": "input_json_delta", "partial_json": c.iter().collect::<String>() }))
            .collect();
        self.blocks.push((
            json!({ "type": "tool_use", "id": id, "name": name, "input": {} }),
            deltas,
        ));
        self
    }

    fn stream(&self) -> String {
        let mut s = sse(
            "message_start",
            json!({ "type": "message_start", "message": {
                "id": "msg_test", "type": "message", "role": "assistant", "content": [],
                "model": "claude-opus-5-5", "stop_reason": null,
                "usage": { "input_tokens": 1200, "cache_read_input_tokens": 9000,
                           "cache_creation_input_tokens": 300, "output_tokens": 1 } } }),
        );
        s.push_str(&sse("ping", json!({ "type": "ping" })));
        for (i, (start, deltas)) in self.blocks.iter().enumerate() {
            s.push_str(&sse(
                "content_block_start",
                json!({ "type": "content_block_start", "index": i, "content_block": start }),
            ));
            for d in deltas {
                s.push_str(&sse(
                    "content_block_delta",
                    json!({ "type": "content_block_delta", "index": i, "delta": d }),
                ));
            }
            s.push_str(&sse(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": i }),
            ));
        }
        s.push_str(&sse(
            "message_delta",
            json!({ "type": "message_delta", "delta": { "stop_reason": self.stop, "stop_sequence": null },
                    "usage": { "output_tokens": 250 } }),
        ));
        s.push_str(&sse("message_stop", json!({ "type": "message_stop" })));
        s
    }
}

/// Answers each request with the next scripted stream and keeps every request body.
struct Scripted {
    replies: Vec<Result<String, TransportError>>,
    requests: Vec<Value>,
}

impl Scripted {
    fn new(replies: Vec<Reply>) -> Scripted {
        Scripted {
            replies: replies.iter().map(|r| Ok(r.stream())).collect(),
            requests: Vec::new(),
        }
    }
}

impl Transport for Scripted {
    fn send(
        &mut self,
        body: &Value,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<(), TransportError> {
        self.requests.push(body.clone());
        if self.replies.is_empty() {
            return Err(TransportError::Http {
                status: 500,
                body: "no more scripted replies".into(),
            });
        }
        let reply = self.replies.remove(0)?;
        for l in reply.split_inclusive('\n') {
            if !line(l) {
                break;
            }
        }
        Ok(())
    }
}

struct Run {
    end: RunEnd,
    events: Vec<Event>,
    server: Scripted,
    conv: Conversation,
}

fn run_script(session: &mut Session, replies: Vec<Reply>, text: &str) -> Run {
    let mut conv = Conversation::new(Settings::default());
    let mut server = Scripted::new(replies);
    let mut host = DirectHost {
        session,
        tools: conv.tools.clone(),
        confirm_destructive: true,
    };
    let mut events = Vec::new();
    let cancel = AtomicBool::new(false);
    let end = agent::run(
        &mut conv,
        text,
        &mut server,
        &mut host,
        &mut |e| events.push(e),
        &cancel,
    );
    Run {
        end,
        events,
        server,
        conv,
    }
}

fn body_volume(session: &mut Session) -> f64 {
    let v = chamfera_engine::agent::summary(session);
    v["bodies"][0]["volume"].as_f64().unwrap_or(f64::NAN)
}

/// The scripted build: a sketch on XY with a 50 x 30 rectangle, then a 10 mm extrude.
fn box_script() -> Vec<Reply> {
    vec![
        Reply::new("tool_use")
            .thinking()
            .text("I will sketch a 50 x 30 rectangle and extrude it 10 mm.")
            .tool("toolu_1", "sketch_create", json!({ "plane": "xy" }))
            .tool(
                "toolu_2",
                "sketch_rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
            ),
        Reply::new("tool_use").tool(
            "toolu_3",
            "solid_extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        ),
        Reply::new("end_turn").text("Built a 50 x 30 x 10 mm box: volume 15000 mm³."),
    ]
}

// ---- SSE ---------------------------------------------------------------------------------

#[test]
fn a_recorded_stream_gives_text_a_tool_call_usage_and_the_stop_reason() {
    // Recorded shape: CRLF line ends, a comment, a ping, two text deltas, a tool call
    // whose input arrives in three fragments, then usage and the stop reason.
    let recorded = "event: message_start\r\n\
        data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\r\n\r\n\
        : a comment\r\n\
        event: ping\r\ndata: {\"type\": \"ping\"}\r\n\r\n\
        event: content_block_start\r\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\r\n\r\n\
        event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Making \"}}\r\n\r\n\
        event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"a plate.\"}}\r\n\r\n\
        event: content_block_stop\r\ndata: {\"type\":\"content_block_stop\",\"index\":0}\r\n\r\n\
        event: content_block_start\r\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"solid_extrude\",\"input\":{}}}\r\n\r\n\
        event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\"}}\r\n\r\n\
        event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"sketch\\\": 1, \\\"dist\"}}\r\n\r\n\
        event: content_block_delta\r\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ance\\\": 8}\"}}\r\n\r\n\
        event: content_block_stop\r\ndata: {\"type\":\"content_block_stop\",\"index\":1}\r\n\r\n\
        event: message_delta\r\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":89}}\r\n\r\n\
        event: message_stop\r\ndata: {\"type\":\"message_stop\"}\r\n\r\n";
    let mut parser = SseParser::default();
    let mut builder = MessageBuilder::default();
    let mut deltas = Vec::new();
    for line in recorded.split_inclusive('\n') {
        if let Some(ev) = parser.line(line).unwrap()
            && let Some(d) = builder.apply(&ev).unwrap()
        {
            deltas.push(d);
        }
    }
    assert!(builder.is_complete());
    assert_eq!(
        deltas,
        vec![
            Delta::Text("Making ".into()),
            Delta::Text("a plate.".into()),
            Delta::ToolStart {
                id: "toolu_9".into(),
                name: "solid_extrude".into()
            },
        ]
    );
    let msg = builder.finish().unwrap();
    assert_eq!(msg.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(msg.usage.input, 25);
    assert_eq!(msg.usage.output, 89);
    assert_eq!(
        msg.content,
        vec![
            json!({ "type": "text", "text": "Making a plate." }),
            json!({ "type": "tool_use", "id": "toolu_9", "name": "solid_extrude",
                    "input": { "sketch": 1, "distance": 8 } }),
        ]
    );
    assert!(msg.bad_inputs.is_empty());
}

#[test]
fn multi_line_data_joins_with_newlines_and_an_error_event_is_an_error() {
    let mut p = SseParser::default();
    assert_eq!(p.line("data: a\n").unwrap(), None);
    assert_eq!(p.line("data:b\n").unwrap(), None);
    let ev = p.line("\n").unwrap().unwrap();
    assert_eq!(ev.data, "a\nb");
    assert_eq!(ev.event, None);
    let mut b = MessageBuilder::default();
    let err = b
        .apply(&crate::sse::SseEvent {
            event: Some("error".into()),
            data: json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } })
                .to_string(),
        })
        .unwrap_err();
    assert!(err.contains("overloaded_error"), "{err}");
}

#[test]
fn hostile_streams_never_panic() {
    let long = "x".repeat(100_000);
    let cases = [
        "data: {not json}\n\n".to_owned(),
        "data: null\n\n".into(),
        "data: []\n\n".into(),
        "data: {\"type\":\"content_block_delta\",\"index\":5,\"delta\":{}}\n\n".into(),
        "data: {\"type\":\"content_block_start\",\"index\":99999999999,\"content_block\":{}}\n\n"
            .into(),
        "data: {\"type\":\"content_block_start\",\"index\":-1}\n\n".into(),
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\"}}\n\n"
            .into(),
        format!("event: {long}\ndata: {{\"type\":\"ping\"}}\n\n"),
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":7},\"usage\":{\"output_tokens\":-3}}\n\n"
            .into(),
        "\u{0}\u{1}\n\n".into(),
        "data\n\n".into(),
        ":\n\n".into(),
    ];
    for case in &cases {
        let mut p = SseParser::default();
        let mut b = MessageBuilder::default();
        for line in case.split_inclusive('\n') {
            if let Ok(Some(ev)) = p.line(line) {
                let _ = b.apply(&ev);
            }
        }
        let _ = b.finish();
    }
    // An event over the size limit is refused, not kept.
    let mut p = SseParser::default();
    let chunk = "y".repeat(1024 * 1024);
    for _ in 0..9 {
        let _ = p.line(&format!("data: {chunk}\n"));
    }
    assert!(p.line("\n").is_err());
}

// ---- The tool loop -----------------------------------------------------------------------

#[test]
fn the_scripted_server_builds_a_50_by_30_by_10_box_of_volume_15000() {
    let mut session = Session::new();
    let run = run_script(&mut session, box_script(), "make a 50x30x10 mm box");
    assert_eq!(run.end, RunEnd::Done);
    // 50 x 30 x 10 = 15000 mm³.
    let v = body_volume(&mut session);
    assert!((v - 15000.0).abs() < 1e-6, "volume {v}");
    // Three requests: the question, the results of the two sketch calls, then the
    // extrude's result.
    assert_eq!(run.server.requests.len(), 3);
    let second = &run.server.requests[1]["messages"];
    let msgs = second.as_array().unwrap();
    assert_eq!(msgs.len(), 3, "user, assistant, user");
    // The reply went back exactly as it came, thinking block and signature included.
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(
        msgs[1]["content"][0],
        json!({ "type": "thinking", "thinking": "", "signature": "c2lnbmF0dXJl" })
    );
    // Both results of that turn are in one user message, in order, without errors.
    let results = msgs[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["tool_use_id"], "toolu_1");
    assert_eq!(results[1]["tool_use_id"], "toolu_2");
    assert!(results.iter().all(|r| r["is_error"] == false));
    // The history only grows: each request starts with the one before it.
    let first = run.server.requests[0]["messages"].as_array().unwrap();
    assert_eq!(&msgs[..first.len()], &first[..]);
    let third = run.server.requests[2]["messages"].as_array().unwrap();
    assert_eq!(&third[..msgs.len()], &msgs[..]);
    // The prefix (system, tools, settings) is the same on every request.
    for r in &run.server.requests {
        assert_eq!(r["system"], run.server.requests[0]["system"]);
        assert_eq!(r["tools"], run.server.requests[0]["tools"]);
        assert_eq!(r["model"], "claude-opus-5-5");
        assert_eq!(r["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(r["output_config"], json!({ "effort": "medium" }));
        assert_eq!(r["tool_choice"], json!({ "type": "auto" }));
        assert_eq!(r["stream"], true);
        assert!(r.get("temperature").is_none());
    }
    // The panel saw the streamed text and a finished row for each call.
    let text: String = run
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("volume 15000"), "{text}");
    let done = run
        .events
        .iter()
        .filter(|e| matches!(e, Event::ToolDone { ok: true, .. }))
        .count();
    assert_eq!(done, 3);
    // Usage adds up over the three replies: 3 x 1200 input, 3 x 250 output.
    assert_eq!(run.conv.usage.input, 3600);
    assert_eq!(run.conv.usage.output, 750);
    assert_eq!(run.conv.messages.len(), 6);
}

#[test]
fn a_failing_command_comes_back_as_an_error_result_and_changes_nothing() {
    let mut session = Session::new();
    let replies = vec![
        Reply::new("tool_use").tool(
            "toolu_1",
            "solid_extrude",
            json!({ "sketch": 42, "profiles": [{ "x": 0, "y": 0 }], "distance": 5 }),
        ),
        Reply::new("end_turn").text("There is no sketch 42."),
    ];
    let run = run_script(&mut session, replies, "extrude");
    assert_eq!(run.end, RunEnd::Done);
    let results = &run.server.requests[1]["messages"][2]["content"];
    assert_eq!(results[0]["is_error"], true);
    let text = results[0]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("solid.extrude:"), "{text}");
    assert!(session.doc.features.is_empty());
    assert!(!session.can_undo());
    assert!(
        run.events
            .iter()
            .any(|e| matches!(e, Event::ToolDone { ok: false, .. }))
    );
}

#[test]
fn a_refusal_or_a_cut_off_reply_runs_none_of_its_tools() {
    for (stop, end) in [
        ("refusal", RunEnd::Refused),
        ("max_tokens", RunEnd::MaxTokens),
    ] {
        let mut session = Session::new();
        let replies =
            vec![Reply::new(stop).tool("toolu_1", "sketch_create", json!({ "plane": "xy" }))];
        let run = run_script(&mut session, replies, "draw");
        assert_eq!(run.end, end, "{stop}");
        assert!(session.doc.sketches.is_empty(), "{stop}: a tool ran");
        // The call is answered as not run, so the next request is well formed.
        let last = run.conv.messages.last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(last["content"][0]["is_error"], true);
        assert_eq!(run.server.requests.len(), 1);
    }
}

#[test]
fn a_paused_turn_is_sent_again_and_continues() {
    let mut session = Session::new();
    let replies = vec![
        Reply::new("pause_turn").text("Working "),
        Reply::new("end_turn").text("done."),
    ];
    let run = run_script(&mut session, replies, "hello");
    assert_eq!(run.end, RunEnd::Done);
    assert_eq!(run.server.requests.len(), 2);
    // The second request ends with the paused reply, with no user message added.
    let msgs = run.server.requests[1]["messages"].as_array().unwrap();
    assert_eq!(msgs.last().unwrap()["role"], "assistant");
}

#[test]
fn invalid_tool_input_json_is_not_run_and_goes_back_as_invalid_json() {
    let mut session = Session::new();
    let replies = vec![
        Reply::new("tool_use").raw_tool("toolu_1", "sketch_create", "{\"plane\": \"xy"),
        Reply::new("end_turn").text("Sorry."),
    ];
    let run = run_script(&mut session, replies, "draw");
    assert_eq!(run.end, RunEnd::Done);
    assert!(session.doc.sketches.is_empty());
    let r = &run.server.requests[1]["messages"][2]["content"][0];
    assert_eq!(r["is_error"], true);
    let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["INVALID_JSON"], "{\"plane\": \"xy");
}

#[test]
fn an_api_error_ends_the_request_with_its_message() {
    let mut session = Session::new();
    let mut conv = Conversation::new(Settings::default());
    let mut server = Scripted {
        replies: vec![Err(TransportError::Http {
            status: 400,
            body: json!({ "type": "error", "error": { "type": "invalid_request_error",
                          "message": "max_tokens: too large" } })
            .to_string(),
        })],
        requests: Vec::new(),
    };
    let mut host = DirectHost {
        session: &mut session,
        tools: conv.tools.clone(),
        confirm_destructive: true,
    };
    let end = agent::run(
        &mut conv,
        "hi",
        &mut server,
        &mut host,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    let RunEnd::Failed(msg) = end else {
        panic!("{end:?}")
    };
    assert!(
        msg.contains("invalid_request_error") && msg.contains("too large"),
        "{msg}"
    );
    // The question stays, and the next one joins it, so roles still alternate.
    assert_eq!(conv.messages.len(), 1);
}

#[test]
fn stop_between_tool_calls_answers_the_rest_as_not_run() {
    struct StopAfterFirst<'a> {
        inner: DirectHost<'a>,
        cancel: &'a AtomicBool,
    }
    impl agent::ToolHost for StopAfterFirst<'_> {
        fn call(&mut self, call: &agent::ToolCall) -> tools::Outcome {
            let out = self.inner.call(call);
            self.cancel.store(true, Ordering::SeqCst);
            out
        }
    }
    let mut session = Session::new();
    let cancel = AtomicBool::new(false);
    let mut conv = Conversation::new(Settings::default());
    let mut server = Scripted::new(box_script());
    let mut host = StopAfterFirst {
        inner: DirectHost {
            session: &mut session,
            tools: conv.tools.clone(),
            confirm_destructive: true,
        },
        cancel: &cancel,
    };
    let end = agent::run(
        &mut conv,
        "box",
        &mut server,
        &mut host,
        &mut |_| {},
        &cancel,
    );
    assert_eq!(end, RunEnd::Stopped);
    assert_eq!(session.doc.sketches.len(), 1, "the first call ran");
    let last = conv.messages.last().unwrap();
    assert_eq!(last["content"][1]["tool_use_id"], "toolu_2");
    assert_eq!(last["content"][1]["is_error"], true);
    assert_eq!(server.requests.len(), 1, "nothing is sent after Stop");
}

#[test]
fn destructive_calls_ask_first_and_a_host_without_a_user_refuses_them() {
    let mut session = Session::new();
    session
        .execute("param.set", &json!({ "name": "w", "value": 5 }))
        .unwrap();
    let replies = vec![
        Reply::new("tool_use").tool("toolu_1", "design_new", json!({})),
        Reply::new("end_turn").text("ok"),
    ];
    let run = run_script(&mut session, replies, "start over");
    assert_eq!(session.doc.parameters.len(), 1, "the design was replaced");
    let r = &run.server.requests[1]["messages"][2]["content"][0];
    assert_eq!(r["is_error"], true);
    let set = ToolSet::new(false, true);
    let kind = set.kind("design_new").unwrap();
    assert!(matches!(
        tools::gate(kind, &json!({}), true),
        Gate::Confirm(_)
    ));
    assert_eq!(tools::gate(kind, &json!({}), false), Gate::Run);
}

// ---- Tools -------------------------------------------------------------------------------

#[test]
fn file_commands_are_left_out_unless_allowed_and_then_always_ask() {
    let off = ToolSet::new(false, true);
    for c in chamfera_engine::registry() {
        let name = tools::tool_name(c.id);
        if chamfera_engine::agent::is_file_command(c.id) {
            assert!(off.kind(&name).is_none(), "{} is offered", c.id);
        } else {
            assert!(off.kind(&name).is_some(), "{} is missing", c.id);
        }
    }
    for name in [
        "design_save",
        "design_open",
        "design_export_stl",
        "body_import_stl",
    ] {
        assert!(off.kind(name).is_none(), "{name}");
    }
    let on = ToolSet::new(true, true);
    let save = on.kind("design_save").unwrap();
    assert!(matches!(
        tools::gate(save, &json!({ "path": "/tmp/a.chamfera" }), false),
        Gate::Confirm(t) if t.contains("/tmp/a.chamfera")
    ));
    // The `*_bytes` commands carry whole files: never offered.
    assert!(on.kind("design_save_bytes").is_none());
    assert!(ToolSet::new(false, false).kind("screenshot").is_none());
}

#[test]
fn tool_names_are_unique_valid_and_sorted_with_object_schemas() {
    let set = ToolSet::new(true, true);
    let defs = set.definitions();
    let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "sorted and unique");
    // Every registry id maps to its own name.
    let ids = chamfera_engine::registry();
    let mut mapped: Vec<String> = ids.iter().map(|c| tools::tool_name(c.id)).collect();
    mapped.sort();
    let n = mapped.len();
    mapped.dedup();
    assert_eq!(mapped.len(), n, "two command ids give the same tool name");
    for d in defs {
        let name = d["name"].as_str().unwrap();
        assert!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "{name}"
        );
        assert_eq!(d["input_schema"]["type"], "object", "{name}");
        assert!(d["input_schema"]["properties"].is_object(), "{name}");
        assert!(d["description"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(d["eager_input_streaming"], true);
    }
}

#[test]
fn the_extrude_schema_comes_from_its_parameter_docs() {
    let spec = chamfera_engine::cmd::find("solid.extrude").unwrap();
    let s = tools::input_schema(spec.params);
    assert_eq!(s["properties"]["sketch"]["type"], "integer");
    assert_eq!(s["properties"]["profiles"]["type"], "array");
    assert_eq!(
        s["properties"]["distance"]["type"],
        json!(["string", "number"])
    );
    assert_eq!(s["properties"]["operation"]["type"], "string");
    // "distance" is "required for the distance extent": not always, so not listed.
    assert_eq!(s["required"], json!(["sketch", "profiles"]));
    let rect = chamfera_engine::cmd::find("sketch.rectangle").unwrap();
    let r = tools::input_schema(rect.params);
    for k in ["x1", "y1", "x2", "y2"] {
        assert_eq!(r["properties"][k]["type"], "number", "{k}");
    }
}

#[test]
fn row_labels_name_the_command_and_its_main_value() {
    let set = ToolSet::new(false, true);
    let k = |n: &str| set.kind(n).unwrap();
    assert_eq!(
        tools::row_label(k("solid_extrude"), &json!({ "distance": 8 })),
        "Extrude 8 mm"
    );
    assert_eq!(
        tools::row_label(k("param_set"), &json!({ "name": "width", "value": 60 })),
        format!(
            "{} width = 60",
            chamfera_engine::cmd::find("param.set").unwrap().label
        )
    );
    assert_eq!(
        tools::row_label(k("design_summary"), &json!({})),
        "Design summary"
    );
}

#[test]
fn hostile_tool_inputs_never_panic() {
    let set = ToolSet::new(false, true);
    let long = "x".repeat(70_000);
    let deep = (0..200).fold(json!(1), |v, _| json!([v]));
    let inputs = [
        Value::Null,
        json!([]),
        json!("text"),
        json!(1e308),
        json!({}),
        json!({ "sketch": -1, "x": 1e308, "y": -1e308, "distance": "1e999 mm" }),
        json!({ "plane": long, "name": long, "path": "/etc/passwd" }),
        json!({ "profiles": deep, "points": [null, {}, []] }),
        json!({ "x1": "0", "y1": null, "x2": [], "y2": {}, "radius": -0.0 }),
        json!({ "constraint": { "kind": "length", "line": 999_999, "value": -5 } }),
    ];
    let mut session = Session::new();
    session.execute("body.sample", &json!({})).unwrap();
    let mut shot =
        |_: &mut Session| -> Result<Vec<u8>, String> { Ok(vec![0x89, b'P', b'N', b'G']) };
    for def in set.definitions() {
        let name = def["name"].as_str().unwrap();
        // design.new would only reset the session the next inputs work on.
        if name == "design_new" {
            continue;
        }
        for input in &inputs {
            let out = tools::execute(&mut session, &set, name, input, Some(&mut shot));
            assert!(!out.content.is_empty(), "{name}");
            if !input.is_object() {
                assert!(out.is_error, "{name} took {input}");
            }
            let _ = tools::row_label(set.kind(name).unwrap(), input);
            let _ = tools::gate(set.kind(name).unwrap(), input, true);
        }
    }
    let out = tools::execute(&mut session, &set, "no_such_tool", &json!({}), None);
    assert!(out.is_error);
}

#[test]
fn a_screenshot_is_an_image_block_of_the_png() {
    let set = ToolSet::new(false, true);
    let mut session = Session::new();
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let mut shot = |_: &mut Session| -> Result<Vec<u8>, String> { Ok(png.clone()) };
    let out = tools::execute(
        &mut session,
        &set,
        "screenshot",
        &json!({}),
        Some(&mut shot),
    );
    assert!(!out.is_error);
    assert_eq!(out.content[0]["type"], "image");
    assert_eq!(out.content[0]["source"]["media_type"], "image/png");
    assert_eq!(
        out.content[0]["source"]["data"],
        chamfera_engine::agent::base64(&[0x89, b'P', b'N', b'G', 1, 2, 3])
    );
    assert!(matches!(set.kind("screenshot"), Some(Kind::Screenshot)));
}

// ---- Settings, cost, key -----------------------------------------------------------------

#[test]
fn the_cost_follows_the_list_prices() {
    // Opus 5.5: $4 per million input, $20 per million output, $0.20 per million cache reads.
    let u = Usage {
        input: 1_000_000,
        output: 100_000,
        cache_read: 2_000_000,
        cache_write: 0,
    };
    assert!((u.cost(Model::Opus) - (4.0 + 2.0 + 0.4)).abs() < 1e-9);
    // Sonnet 5.5: $2 / $10, cache reads $0.20.
    assert!((u.cost(Model::Sonnet) - (2.0 + 1.0 + 0.4)).abs() < 1e-9);
    assert_eq!(Model::parse("claude-haiku-5-5"), Some(Model::Haiku));
    assert_eq!(Effort::parse("xhigh"), Some(Effort::XHigh));
    assert_eq!(Model::parse("claude-opus-5-5-20260101"), None);
}

#[test]
fn the_api_key_never_appears_in_debug_output_requests_or_the_design() {
    let secret = "sk-ant-api03-SECRETSECRETSECRET-wxyz";
    let key = ApiKey::new(&format!("  {secret}\n")).unwrap();
    assert_eq!(key.expose(), secret);
    let debug = format!("{key:?}");
    assert!(!debug.contains("SECRET"), "{debug}");
    assert_eq!(key.hint(), "…wxyz");
    assert!(ApiKey::new("short").is_err());
    assert!(ApiKey::new("has a space in it").is_err());
    #[cfg(not(target_arch = "wasm32"))]
    {
        let t = crate::http::HttpTransport::new(key.clone());
        assert!(!format!("{t:?}").contains("SECRET"));
    }
    let mut session = Session::new();
    let run = run_script(&mut session, box_script(), "make a box");
    for r in &run.server.requests {
        assert!(!r.to_string().contains(secret));
    }
    assert!(!format!("{:?}", run.conv).contains(secret));
    let design = session.doc.to_json().unwrap();
    assert!(!design.contains(secret) && !design.contains("sk-ant"));
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
#[test]
fn the_key_file_is_private_and_reads_back() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("chamfera-key-{}", std::process::id()));
    let path = dir.join("sub").join("assistant-key");
    let key = ApiKey::new("sk-ant-test-0123456789").unwrap();
    crate::key::save(&path, &key).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(crate::key::load(&path), Some(key));
    crate::key::remove(&path).unwrap();
    assert_eq!(crate::key::load(&path), None);
    crate::key::remove(&path).unwrap();
    // A folder or a device is no key.
    assert_eq!(crate::key::load(&dir), None);
    assert_eq!(crate::key::load(std::path::Path::new("/dev/zero")), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_recorded_demo_builds_the_plate_with_four_holes() {
    let mut session = Session::new();
    let mut conv = Conversation::new(Settings::default());
    let mut replay = crate::script::Replay::new(&crate::script::demo_replies());
    let mut host = DirectHost {
        session: &mut session,
        tools: conv.tools.clone(),
        confirm_destructive: true,
    };
    let mut errors = Vec::new();
    let mut order = Vec::new();
    let end = agent::run(
        &mut conv,
        crate::script::DEMO_REQUEST,
        &mut replay,
        &mut host,
        &mut |e| match e {
            Event::ToolDone {
                ok: false, summary, ..
            } => errors.push(summary),
            Event::ToolPending { name, .. } => order.push(name),
            _ => {}
        },
        &AtomicBool::new(false),
    );
    assert_eq!(end, RunEnd::Done);
    assert!(errors.is_empty(), "{errors:?}");
    // Finish Sketch comes before the extrude (QA user guide pass: it ran after).
    let at = |n: &str| order.iter().position(|x| x == n);
    assert!(at("sketch_finish") < at("solid_extrude"), "{order:?}");
    assert!(at("sketch_finish").is_some());
    // 60 x 40 x 8 less four 5.5 mm holes 8 deep: 19200 - 4 * pi * 2.75^2 * 8 = 19200 - 242 pi.
    let expected = 19200.0 - 242.0 * std::f64::consts::PI;
    let v = body_volume(&mut session);
    assert!(
        (v - expected).abs() < 1e-6,
        "volume {v}, expected {expected}"
    );
}

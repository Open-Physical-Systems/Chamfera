//! A recorded conversation played back instead of the network: the app's
//! `--assistant-demo` start-up option and the UI tests use it, so the panel can be shown
//! and checked without an API key or a live call.

use crate::agent::{Transport, TransportError};
use serde_json::{Value, json};

fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// One reply, built block by block and streamed the way the API sends it: text in small
/// deltas, tool input in JSON fragments.
#[derive(Debug, Clone)]
pub struct ScriptedReply {
    blocks: Vec<(Value, Vec<Value>)>,
    stop: &'static str,
}

impl ScriptedReply {
    pub fn new(stop: &'static str) -> ScriptedReply {
        ScriptedReply {
            blocks: Vec::new(),
            stop,
        }
    }

    pub fn text(mut self, text: &str) -> ScriptedReply {
        let deltas = text
            .split_inclusive(' ')
            .map(|t| json!({ "type": "text_delta", "text": t }))
            .collect();
        self.blocks
            .push((json!({ "type": "text", "text": "" }), deltas));
        self
    }

    pub fn tool(mut self, id: &str, name: &str, input: Value) -> ScriptedReply {
        let s: Vec<char> = input.to_string().chars().collect();
        let deltas = s
            .chunks(12)
            .map(|c| json!({ "type": "input_json_delta", "partial_json": c.iter().collect::<String>() }))
            .collect();
        self.blocks.push((
            json!({ "type": "tool_use", "id": id, "name": name, "input": {} }),
            deltas,
        ));
        self
    }

    /// The reply as server-sent events.
    pub fn stream(&self) -> String {
        let mut s = sse(
            "message_start",
            json!({ "type": "message_start", "message": {
                "id": "msg_recorded", "type": "message", "role": "assistant", "content": [],
                "usage": { "input_tokens": 850, "cache_read_input_tokens": 21_000,
                           "cache_creation_input_tokens": 0, "output_tokens": 1 } } }),
        );
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
            json!({ "type": "message_delta", "delta": { "stop_reason": self.stop },
                    "usage": { "output_tokens": 180 } }),
        ));
        s.push_str(&sse("message_stop", json!({ "type": "message_stop" })));
        s
    }
}

/// Plays recorded replies back, one per request, with a short pause between lines so
/// the panel shows them streaming in.
#[derive(Debug, Clone)]
pub struct Replay {
    streams: Vec<String>,
    pub line_delay: std::time::Duration,
}

impl Replay {
    pub fn new(replies: &[ScriptedReply]) -> Replay {
        Replay {
            streams: replies.iter().map(ScriptedReply::stream).collect(),
            line_delay: std::time::Duration::ZERO,
        }
    }
}

impl Transport for Replay {
    fn send(
        &mut self,
        _body: &Value,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<(), TransportError> {
        if self.streams.is_empty() {
            return Err(TransportError::Http {
                status: 400,
                body: json!({ "type": "error", "error": { "type": "invalid_request_error",
                    "message": "the recorded conversation has no more replies" } })
                .to_string(),
            });
        }
        let stream = self.streams.remove(0);
        for l in stream.split_inclusive('\n') {
            if !self.line_delay.is_zero() && l.starts_with("data:") {
                std::thread::sleep(self.line_delay);
            }
            if !line(l) {
                break;
            }
        }
        Ok(())
    }
}

/// The request of the recorded demo.
pub const DEMO_REQUEST: &str = "A 60×40×8 mm plate with four 5.5 mm through holes, 6 mm in \
    from each corner.";

/// A recorded conversation that builds the demo plate: a 60 x 40 rectangle with four
/// 5.5 mm circles 6 mm in from the corners, extruded 8 mm. Volume 60 * 40 * 8 - 4 * pi *
/// 2.75^2 * 8 = 19200 - 242 pi = 18439.73 mm³.
pub fn demo_replies() -> Vec<ScriptedReply> {
    let mut sketch = ScriptedReply::new("tool_use")
        .text(
            "Plan: a 60 × 40 rectangle on XY with four Ø5.5 circles centred 6 mm in from the \
             corners, extruded 8 mm. Expected volume 19200 − 242π ≈ 18439.7 mm³.",
        )
        .tool(
            "toolu_d1",
            "param_set",
            json!({ "name": "thickness", "value": 8 }),
        )
        .tool("toolu_d2", "sketch_create", json!({ "plane": "xy" }))
        .tool(
            "toolu_d3",
            "sketch_rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 60, "y2": 40 }),
        );
    for (i, (x, y)) in [(6, 6), (54, 6), (54, 34), (6, 34)].into_iter().enumerate() {
        sketch = sketch.tool(
            &format!("toolu_c{i}"),
            "sketch_circle",
            json!({ "cx": x, "cy": y, "radius": 2.75 }),
        );
    }
    vec![
        sketch,
        // The sketch is finished before it is extruded, as a person would do it.
        ScriptedReply::new("tool_use")
            .tool("toolu_d6", "sketch_finish", json!({}))
            .tool(
                "toolu_d4",
                "solid_extrude",
                json!({ "sketch": 1, "profiles": [{ "x": 30, "y": 20 }], "distance": "thickness" }),
            ),
        ScriptedReply::new("tool_use")
            .text("Checking the result.")
            .tool("toolu_d5", "design_summary", json!({})),
        ScriptedReply::new("end_turn").text(
            "Built the plate: 60 × 40 × 8 mm with four Ø5.5 mm through holes 6 mm from the \
             corners. Measured volume 18439.73 mm³, matching 19200 − 242π.",
        ),
    ]
}

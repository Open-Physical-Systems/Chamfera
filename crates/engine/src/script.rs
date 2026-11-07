//! Scripts: a JSON list of steps `{"command": id, "params": {...}, "as": name}`.
//!
//! A step with `"as": "rect"` keeps its reply under that name, and later steps refer to it
//! with strings like `"$rect.curves.0"` (a field, then a list index). This way scripts never
//! guess ids: points, curves and constraints share one id counter per sketch, and sketches
//! and features share another.
//!
//! ```json
//! [
//!   { "command": "sketch.create", "params": { "plane": "xz" }, "as": "s" },
//!   { "command": "sketch.line", "params": { "x1": 0, "y1": 0, "x2": 0, "y2": 20 }, "as": "axis" },
//!   { "command": "sketch.construction", "params": { "curve": "$axis.curve", "construction": true } }
//! ]
//! ```

use crate::Session;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Most steps one script may hold.
pub const MAX_STEPS: usize = 100_000;

/// Why a script stopped: the step (1-based), its command and the message.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError {
    pub step: usize,
    pub command: String,
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "step {} ({}): {}", self.step, self.command, self.message)
    }
}

/// Replaces `"$name.path"` strings with the value they point to in earlier replies.
fn substitute(v: &Value, vars: &BTreeMap<String, Value>) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) if s.starts_with('$') && s.len() > 1 => {
            let mut parts = s[1..].split('.');
            let name = parts.next().unwrap_or_default();
            let mut cur = vars
                .get(name)
                .ok_or_else(|| format!("{s}: no earlier step is named {name:?}"))?;
            for part in parts {
                cur = match part.parse::<usize>() {
                    Ok(i) => cur.get(i),
                    Err(_) => cur.get(part),
                }
                .ok_or_else(|| format!("{s}: the reply of {name:?} has no {part:?}"))?;
            }
            cur.clone()
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| substitute(x, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| Ok((k.clone(), substitute(x, vars)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

/// Runs the steps in order, stopping at the first failure. Returns each reply.
pub fn run_steps(session: &mut Session, steps: &[Value]) -> Result<Vec<Value>, ScriptError> {
    let mut replies = Vec::with_capacity(steps.len().min(MAX_STEPS));
    run_steps_each(session, steps, |_, reply| replies.push(reply.clone()))?;
    Ok(replies)
}

/// [`run_steps`], handing each step's reply (with its 1-based number) to `each` as soon
/// as the step has run, so the replies before a failing step are not lost.
pub fn run_steps_each(
    session: &mut Session,
    steps: &[Value],
    mut each: impl FnMut(usize, &Value),
) -> Result<(), ScriptError> {
    let fail = |step: usize, command: &str, message: String| ScriptError {
        step,
        command: command.to_owned(),
        message,
    };
    if steps.len() > MAX_STEPS {
        return Err(fail(
            0,
            "",
            format!("a script holds at most {MAX_STEPS} steps"),
        ));
    }
    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    for (i, step) in steps.iter().enumerate() {
        let n = i + 1;
        let id = step
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| fail(n, "?", "`command` must be a string".into()))?;
        let params = substitute(&step.get("params").cloned().unwrap_or(json!({})), &vars)
            .map_err(|m| fail(n, id, m))?;
        let reply = session
            .execute(id, &params)
            .map_err(|e| fail(n, id, e.to_string()))?;
        if let Some(name) = step.get("as") {
            let name = name
                .as_str()
                .filter(|s| crate::cmd::is_valid_name(s))
                .ok_or_else(|| fail(n, id, "`as` must be a name like `rect` or `axis2`".into()))?;
            vars.insert(name.to_owned(), reply.clone());
        }
        each(n, &reply);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The replies of the steps before a failing one still reach the caller (QA user
    /// guide pass: `chamfera-cli run` printed nothing when a step failed).
    #[test]
    fn replies_before_a_failing_step_are_handed_over() {
        let steps: Vec<Value> = serde_json::from_str(
            r#"[
              { "command": "sketch.create", "params": { "plane": "xy" } },
              { "command": "no.such.command" }
            ]"#,
        )
        .unwrap();
        let mut seen = Vec::new();
        let e = run_steps_each(&mut Session::new(), &steps, |n, r| {
            seen.push((n, r.clone()))
        });
        assert_eq!(e.map_err(|e| e.step), Err(2));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 1);
        assert!(seen[0].1["sketch"].is_u64(), "{:?}", seen[0].1);
    }

    #[test]
    fn named_replies_feed_later_steps() {
        let steps: Vec<Value> = serde_json::from_str(
            r#"[
              { "command": "sketch.create", "params": { "plane": "xz" }, "as": "s" },
              { "command": "sketch.rectangle", "params": { "x1": 10, "y1": 0, "x2": 15, "y2": 20 } },
              { "command": "sketch.line", "params": { "x1": 0, "y1": -5, "x2": 0, "y2": 25 }, "as": "axis" },
              { "command": "sketch.construction", "params": { "curve": "$axis.curve", "construction": true } },
              { "command": "sketch.finish" },
              { "command": "solid.revolve", "params": { "sketch": "$s.sketch", "profiles": [{ "x": 12, "y": 10 }], "axis": "$axis.curve" } }
            ]"#,
        )
        .unwrap();
        let mut s = Session::new();
        let replies = run_steps(&mut s, &steps).unwrap();
        let v = replies[5]["solids"][0]["volume"].as_f64().unwrap();
        assert!((v - 2500.0 * std::f64::consts::PI).abs() < 1e-6);
    }

    #[test]
    fn bad_references_name_the_step() {
        let steps: Vec<Value> = serde_json::from_str(
            r#"[
              { "command": "sketch.create", "params": { "plane": "xy" }, "as": "s" },
              { "command": "sketch.point", "params": { "x": "$s.nothing", "y": 0 } }
            ]"#,
        )
        .unwrap();
        let e = run_steps(&mut Session::new(), &steps).unwrap_err();
        assert_eq!(e.step, 2);
        assert!(e.message.contains("nothing"), "{e}");
        let e = run_steps(
            &mut Session::new(),
            &[json!({ "command": "app.about", "as": "1bad" })],
        )
        .unwrap_err();
        assert!(e.message.contains("`as`"));
    }
}

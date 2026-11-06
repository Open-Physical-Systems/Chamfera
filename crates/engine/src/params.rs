//! Reading command parameters. Parameters are hostile: every read validates.

use crate::CommandError;
use serde_json::Value;

fn object(params: &Value) -> Result<&serde_json::Map<String, Value>, CommandError> {
    params
        .as_object()
        .ok_or_else(|| CommandError::BadParams("parameters must be a JSON object".into()))
}

/// A required string.
pub fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    opt_str(params, key)?.ok_or_else(|| CommandError::BadParams(format!("`{key}` is required")))
}

/// An optional string.
pub fn opt_str<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, CommandError> {
    match object(params)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(CommandError::BadParams(format!("`{key}` must be a string"))),
    }
}

/// A required finite number.
pub fn req_f64(params: &Value, key: &str) -> Result<f64, CommandError> {
    match object(params)?.get(key) {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| CommandError::BadParams(format!("`{key}` must be a finite number"))),
        None | Some(Value::Null) => Err(CommandError::BadParams(format!("`{key}` is required"))),
        Some(_) => Err(CommandError::BadParams(format!("`{key}` must be a number"))),
    }
}

/// Accepts `null` or an object for commands that take no parameters.
pub fn none(params: &Value) -> Result<(), CommandError> {
    match params {
        Value::Null => Ok(()),
        Value::Object(_) => Ok(()),
        _ => Err(CommandError::BadParams(
            "parameters must be a JSON object".into(),
        )),
    }
}

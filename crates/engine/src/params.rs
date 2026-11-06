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

/// An optional finite number.
pub fn opt_f64(params: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match object(params)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => req_f64(params, key).map(Some),
    }
}

/// A required boolean.
pub fn req_bool(params: &Value, key: &str) -> Result<bool, CommandError> {
    match object(params)?.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        None | Some(Value::Null) => Err(CommandError::BadParams(format!("`{key}` is required"))),
        Some(_) => Err(CommandError::BadParams(format!(
            "`{key}` must be true or false"
        ))),
    }
}

/// A required id: a whole number from 1 to u32::MAX.
pub fn req_id(params: &Value, key: &str) -> Result<u32, CommandError> {
    let bad = || CommandError::BadParams(format!("`{key}` must be a whole number id"));
    match object(params)?.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(bad),
        None | Some(Value::Null) => Err(CommandError::BadParams(format!("`{key}` is required"))),
        Some(_) => Err(bad()),
    }
}

/// A viewport size in pixels: `width` and `height`, each 1 to 32768.
pub fn viewport(params: &Value) -> Result<(f64, f64), CommandError> {
    let w = req_f64(params, "width")?;
    let h = req_f64(params, "height")?;
    if !(1.0..=32768.0).contains(&w) || !(1.0..=32768.0).contains(&h) {
        return Err(CommandError::BadParams(
            "`width` and `height` must be 1 to 32768 pixels".into(),
        ));
    }
    Ok((w, h))
}

/// A pixel inside a viewport: `x` and `y` (top-left origin) plus `width` and `height`.
pub fn pixel(params: &Value) -> Result<(f64, f64, f64, f64), CommandError> {
    let (w, h) = viewport(params)?;
    let x = req_f64(params, "x")?;
    let y = req_f64(params, "y")?;
    if !(0.0..=w).contains(&x) || !(0.0..=h).contains(&y) {
        return Err(CommandError::BadParams(
            "`x`, `y` must be inside the viewport".into(),
        ));
    }
    Ok((x, y, w, h))
}

//! User parameters as CSV (RFC 4180): a header `name,expression,unit,comment`, then one
//! row per parameter. Fields holding a comma, a quote or a line break are quoted, with
//! quotes doubled. A value without an expression is written as one with its unit
//! (`12.5 mm`, `30 deg`) in the shortest form that reads back to the same number.

use crate::IoError;
use chamfera_doc::{Document, Parameter, Unit};

/// Largest CSV file read (bytes).
pub const MAX_CSV_BYTES: u64 = 4 * 1024 * 1024;
/// Most rows read.
pub const MAX_CSV_ROWS: usize = chamfera_doc::MAX_PARAMETERS;
/// Longest field read (characters).
const MAX_FIELD: usize = 4096;

/// The header row.
pub const HEADER: [&str; 4] = ["name", "expression", "unit", "comment"];

/// One parameter row read from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamRow {
    pub name: String,
    pub expression: String,
    pub unit: Unit,
    pub comment: String,
}

fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn unit_name(u: Unit) -> &'static str {
    match u {
        Unit::Length => "length",
        Unit::Angle => "angle",
        Unit::Unitless => "unitless",
    }
}

/// A plain value as an expression: mm, degrees or a plain number.
fn value_expression(p: &Parameter) -> String {
    match p.unit {
        Unit::Length => format!("{} mm", p.value),
        Unit::Angle => format!("{} rad", p.value),
        Unit::Unitless => format!("{}", p.value),
    }
}

/// The design's user parameters as CSV, in name order, with CRLF line ends.
pub fn write_parameters(doc: &Document) -> String {
    let mut out = HEADER.join(",");
    out.push_str("\r\n");
    for (name, p) in &doc.parameters {
        let e = p.expression.clone().unwrap_or_else(|| value_expression(p));
        let row = [
            field(name),
            field(&e),
            unit_name(p.unit).to_owned(),
            field(&p.comment),
        ];
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out
}

fn bad(line: usize, reason: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("parameter CSV, row {line}: {reason}"))
}

/// Splits CSV text into rows of fields (RFC 4180, LF or CRLF line ends).
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, IoError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut line = 1;
    let mut any = false;
    while let Some(c) = chars.next() {
        if cur.chars().count() > MAX_FIELD {
            return Err(bad(
                line,
                format!("a field is longer than {MAX_FIELD} characters"),
            ));
        }
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '"' if cur.is_empty() => {
                quoted = true;
                any = true;
            }
            ',' => {
                row.push(std::mem::take(&mut cur));
                any = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                if any || !cur.is_empty() {
                    row.push(std::mem::take(&mut cur));
                    rows.push(std::mem::take(&mut row));
                }
                any = false;
                line += 1;
                if rows.len() > MAX_CSV_ROWS + 1 {
                    return Err(bad(line, format!("more than {MAX_CSV_ROWS} rows")));
                }
            }
            _ => {
                cur.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return Err(bad(line, "a quoted field is not closed"));
    }
    if any || !cur.is_empty() {
        row.push(cur);
        rows.push(row);
    }
    Ok(rows)
}

/// Reads parameter rows: the header first (its columns in any order; `comment` may be
/// left out), then one row per parameter.
pub fn read_parameters(text: &str) -> Result<Vec<ParamRow>, IoError> {
    let rows = parse(text)?;
    let mut it = rows.into_iter();
    let header = it.next().ok_or_else(|| bad(1, "the file is empty"))?;
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let (Some(n), Some(e), Some(u)) = (col("name"), col("expression"), col("unit")) else {
        return Err(bad(
            1,
            "the header must name the columns name, expression, unit (and comment)",
        ));
    };
    let c = col("comment");
    let mut out = Vec::new();
    for (i, r) in it.enumerate() {
        let line = i + 2;
        let get = |k: usize| r.get(k).map(|s| s.trim().to_owned()).unwrap_or_default();
        let unit_text = get(u);
        let unit = if unit_text.is_empty() {
            Unit::Length
        } else {
            Unit::parse(&unit_text.to_ascii_lowercase()).ok_or_else(|| {
                bad(
                    line,
                    format!("unit {unit_text:?}: use length, angle or unitless"),
                )
            })?
        };
        let row = ParamRow {
            name: get(n),
            expression: get(e),
            unit,
            comment: c.map(get).unwrap_or_default(),
        };
        if row.name.is_empty() {
            return Err(bad(line, "the name is empty"));
        }
        if row.expression.is_empty() {
            return Err(bad(line, format!("{} has no expression", row.name)));
        }
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_rfc_4180() {
        assert_eq!(field("max(a, b)"), "\"max(a, b)\"");
        assert_eq!(field("2\""), "\"2\"\"\"");
        assert_eq!(field("plain"), "plain");
        let rows = parse("a,\"b, c\",\"d \"\"e\"\"\"\r\n\r\nx,,\n").unwrap();
        assert_eq!(rows, vec![vec!["a", "b, c", "d \"e\""], vec!["x", "", ""]]);
    }

    #[test]
    fn bad_csv_is_an_error_not_a_panic() {
        assert!(parse("\"open").is_err());
        assert!(read_parameters("").is_err());
        assert!(read_parameters("a,b\n1,2").is_err());
        assert!(read_parameters("name,expression,unit\nw,,length").is_err());
        assert!(read_parameters("name,expression,unit\nw,5,furlong").is_err());
        assert!(read_parameters("name,expression,unit\n,5,length").is_err());
        assert!(parse(&"x".repeat(10_000)).is_err());
        let many = "a\n".repeat(MAX_CSV_ROWS + 5);
        assert!(parse(&many).is_err());
        // Short rows and a missing comment column are fine.
        let r = read_parameters("Name,Unit,Expression\nw,,5\n").unwrap();
        assert_eq!(r[0].unit, Unit::Length);
        assert_eq!(r[0].expression, "5");
    }
}

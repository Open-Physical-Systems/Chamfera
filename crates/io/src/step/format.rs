//! Part 21 (ISO 10303-21) tokens: reals, strings, logicals and points.
//!
//! - A real is `[sign] digits "." [digits] ["E" [sign] digits]`: the decimal point is
//!   mandatory. We print the shortest decimal that reads back to the same `f64` (Rust's
//!   `Display`/`LowerExp` are shortest-round-trip), so coordinates survive bit for bit.
//! - A string is enclosed in apostrophes; inside, an apostrophe is doubled and a backslash
//!   is doubled. Characters outside printable ASCII (`' '..='~'`) are written as
//!   `\X2\hhhh…\X0\` (UTF-16 code units of the basic multilingual plane, 4 upper-case hex
//!   digits each) or `\X4\hhhhhhhh…\X0\` (code points beyond it, 8 digits each), so the
//!   file stays plain ASCII.

use crate::IoError;
use chamfera_geom::{Point3, Vec3};
use std::fmt::Write;

/// Plain decimal notation is used for magnitudes in `[1e-5, 1e16)` (and zero); others
/// use an exponent so the text stays short.
const PLAIN_MIN: f64 = 1e-5;
const PLAIN_MAX: f64 = 1e16;

/// A Part 21 real that reads back as exactly `v`. Non-finite values have no Part 21 form.
pub fn real(v: f64) -> Result<String, IoError> {
    if !v.is_finite() {
        return Err(IoError::Format(format!(
            "STEP: a number must be finite, got {v}"
        )));
    }
    let a = v.abs();
    if a == 0.0 || (PLAIN_MIN..PLAIN_MAX).contains(&a) {
        let mut s = format!("{v}");
        if !s.contains('.') {
            s.push('.');
        }
        return Ok(s);
    }
    // `{:e}` gives e.g. "1.5e-7" or "1e300"; Part 21 wants "1.5E-07" / "1.E+300".
    let s = format!("{v:e}");
    let (mantissa, exponent) = s
        .split_once('e')
        .ok_or_else(|| IoError::Format(format!("STEP: cannot format the number {v}")))?;
    let mut out = String::from(mantissa);
    if !out.contains('.') {
        out.push('.');
    }
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(d) => ('-', d),
        None => ('+', exponent),
    };
    let _ = write!(out, "E{sign}{digits:0>2}");
    Ok(out)
}

/// The Part 21 string literal for `s`, apostrophes included.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                // A run of characters of one plane goes into one \X2\ or \X4\ group.
                let wide = u32::from(c) > 0xFFFF;
                out.push_str(if wide { "\\X4\\" } else { "\\X2\\" });
                let mut push = |c: char| {
                    if wide {
                        let _ = write!(out, "{:08X}", u32::from(c));
                    } else {
                        let mut units = [0u16; 2];
                        for u in c.encode_utf16(&mut units) {
                            let _ = write!(out, "{u:04X}");
                        }
                    }
                };
                push(c);
                while let Some(&n) = chars.peek() {
                    let n_plain = matches!(n, ' '..='~');
                    if n_plain || (u32::from(n) > 0xFFFF) != wide {
                        break;
                    }
                    push(n);
                    chars.next();
                }
                out.push_str("\\X0\\");
            }
        }
    }
    out.push('\'');
    out
}

/// `.T.` or `.F.`.
pub fn logical(b: bool) -> &'static str {
    if b { ".T." } else { ".F." }
}

/// `(x,y,z)` for a point's coordinates.
pub fn point(p: Point3) -> Result<String, IoError> {
    Ok(format!("({},{},{})", real(p.x)?, real(p.y)?, real(p.z)?))
}

/// `(x,y,z)` for a vector's components.
pub fn vector(v: Vec3) -> Result<String, IoError> {
    Ok(format!("({},{},{})", real(v.x)?, real(v.y)?, real(v.z)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the Part 21 grammar of a real by hand (no regex dependency).
    fn is_part21_real(s: &str) -> bool {
        let b = s.as_bytes();
        let mut i = 0;
        if matches!(b.first(), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start || b.get(i) != Some(&b'.') {
            return false;
        }
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if b.get(i) == Some(&b'E') {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let e = i;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == e {
                return false;
            }
        }
        i == b.len()
    }

    #[test]
    fn reals_have_a_decimal_point_and_the_expected_text() {
        // Expected forms written out by hand from the Part 21 grammar.
        let cases = [
            (0.0, "0."),
            (-0.0, "-0."),
            (1.0, "1."),
            (50.0, "50."),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e-7, "1.E-07"),
            (-1.5e-7, "-1.5E-07"),
            (1e300, "1.E+300"),
            (2.5e16, "2.5E+16"),
            (1e-5, "0.00001"),
        ];
        for (v, text) in cases {
            assert_eq!(real(v).unwrap(), text, "{v}");
        }
    }

    #[test]
    fn reals_round_trip_bit_for_bit() {
        // Pseudo-random bit patterns (a 64-bit LCG, Knuth's MMIX constants) cover every
        // exponent; plus the extremes.
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut values = vec![
            f64::MIN_POSITIVE,
            5e-324,
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            1.0 / 3.0,
            -2.0 / 3.0,
        ];
        for _ in 0..20_000 {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            values.push(f64::from_bits(x));
        }
        for v in values.into_iter().filter(|v| v.is_finite()) {
            let s = real(v).unwrap();
            assert!(is_part21_real(&s), "{s}");
            let back = super::super::read::parse_real(&s).unwrap();
            assert_eq!(back.to_bits(), v.to_bits(), "{s}");
        }
    }

    #[test]
    fn non_finite_numbers_are_refused() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(real(v).is_err());
            assert!(point(Point3::new(0.0, v, 0.0)).is_err());
        }
    }

    #[test]
    fn strings_escape_apostrophes_backslashes_and_non_ascii() {
        // Expected escapes derived by hand: ü = U+00FC, 日 = U+65E5, 本 = U+672C,
        // 😀 = U+1F600, newline = U+000A.
        assert_eq!(string("plain"), "'plain'");
        assert_eq!(string("O'Brien"), "'O''Brien'");
        assert_eq!(string("a\\b"), "'a\\\\b'");
        assert_eq!(string("Düsseldorf"), "'D\\X2\\00FC\\X0\\sseldorf'");
        assert_eq!(string("日本"), "'\\X2\\65E5672C\\X0\\'");
        assert_eq!(string("x😀"), "'x\\X4\\0001F600\\X0\\'");
        assert_eq!(string("a\nb"), "'a\\X2\\000A\\X0\\b'");
        assert_eq!(string("ü😀"), "'\\X2\\00FC\\X0\\\\X4\\0001F600\\X0\\'");
        assert!(string("日本 ü 😀 '\\").is_ascii());
    }
}

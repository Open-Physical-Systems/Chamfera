//! Text helpers shared by the sheet layout and the writers.

/// Advance widths of a plain sans-serif face, in thousandths of the text height, for
/// placing text before it is drawn. These are rounded approximations of typical
/// Helvetica-like proportions (the widths of the PDF base-14 font are public in its
/// metrics files); exact values are not needed, only a box that fits.
fn advance(c: char) -> f64 {
    match c {
        ' ' | '.' | ',' | ':' | ';' | '!' | '\'' | '|' | 'i' | 'j' | 'l' | 'I' => 278.0,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '/' | '\\' => 333.0,
        '0'..='9'
        | 'a'
        | 'b'
        | 'c'
        | 'd'
        | 'e'
        | 'g'
        | 'h'
        | 'k'
        | 'n'
        | 'o'
        | 'p'
        | 'q'
        | 's'
        | 'u'
        | 'v'
        | 'x'
        | 'y'
        | 'z'
        | '_'
        | '$'
        | '?' => 556.0,
        'm' | 'M' => 833.0,
        'w' | 'W' => 944.0,
        '°' => 400.0,
        'Ø' | 'O' | 'Q' | 'G' | 'C' | 'D' | 'H' | 'N' | 'U' | 'R' => 760.0,
        'A'..='Z' => 667.0,
        _ => 584.0,
    }
}

/// About how wide `text` is at `height` (mm).
pub fn width(text: &str, height: f64) -> f64 {
    text.chars().map(advance).sum::<f64>() * height / 1000.0
}

/// Text for a drawing: control characters dropped, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

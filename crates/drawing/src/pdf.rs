//! A minimal PDF 1.4 writer (ISO 32000-1 / the PDF Reference, 3rd edition): one page,
//! vector paths, the standard Helvetica font with WinAnsiEncoding, and a cross-reference
//! table with exact byte offsets.

use crate::sheet::SheetGeometry;
use crate::svg::num;
use crate::text::width;
use crate::{Align, Layer, Shape};
use std::f64::consts::FRAC_PI_2;
use std::fmt::Write;

/// PostScript points per mm.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// A PDF string literal in WinAnsiEncoding: `( ) \` escaped, Ø and ° as octal codes,
/// other characters outside printable ASCII as `?`.
fn pdf_string(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            'Ø' => out.push_str("\\330"),
            'ø' => out.push_str("\\370"),
            '°' => out.push_str("\\260"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// One cubic Bézier per at most a quarter turn: control distance `4/3 tan(θ/4) r`.
fn arc_path(out: &mut String, cx: f64, cy: f64, r: f64, start: f64, end: f64, move_to: bool) {
    let sweep = end - start;
    let n = ((sweep.abs() / FRAC_PI_2).ceil() as usize).clamp(1, 8);
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
    let p = |a: f64| (cx + r * a.cos(), cy + r * a.sin());
    let (x0, y0) = p(start);
    if move_to {
        let _ = writeln!(out, "{} {} m", num(x0), num(y0));
    }
    for i in 0..n {
        let a0 = start + step * i as f64;
        let a1 = a0 + step;
        let (x0, y0) = p(a0);
        let (x3, y3) = p(a1);
        let (x1, y1) = (x0 - k * a0.sin(), y0 + k * a0.cos());
        let (x2, y2) = (x3 + k * a1.sin(), y3 - k * a1.cos());
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} c",
            num(x1),
            num(y1),
            num(x2),
            num(y2),
            num(x3),
            num(y3)
        );
    }
}

/// The page's content stream, drawn in mm.
fn content(sheet: &SheetGeometry) -> String {
    let mut out = String::new();
    let k = num(PT_PER_MM);
    let _ = writeln!(out, "{k} 0 0 {k} 0 0 cm");
    let _ = writeln!(out, "1 J 1 j 0 G 0 g");
    for layer in Layer::ALL {
        let dashes = layer
            .dashes()
            .iter()
            .map(|d| num(*d))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "{} w [{dashes}] 0 d", num(layer.width()));
        for item in sheet.items.iter().filter(|i| i.layer == layer) {
            match &item.shape {
                Shape::Line { a, b } => {
                    let _ = writeln!(
                        out,
                        "{} {} m {} {} l S",
                        num(a.x),
                        num(a.y),
                        num(b.x),
                        num(b.y)
                    );
                }
                Shape::Circle { centre, radius } => {
                    arc_path(
                        &mut out,
                        centre.x,
                        centre.y,
                        *radius,
                        0.0,
                        std::f64::consts::TAU,
                        true,
                    );
                    let _ = writeln!(out, "h S");
                }
                Shape::Arc {
                    centre,
                    radius,
                    start,
                    end,
                } => {
                    arc_path(&mut out, centre.x, centre.y, *radius, *start, *end, true);
                    let _ = writeln!(out, "S");
                }
                Shape::Fill { points } => {
                    let [a, b, c] = *points;
                    let _ = writeln!(
                        out,
                        "{} {} m {} {} l {} {} l h f",
                        num(a.x),
                        num(a.y),
                        num(b.x),
                        num(b.y),
                        num(c.x),
                        num(c.y)
                    );
                }
                Shape::Text {
                    at,
                    height,
                    angle,
                    align,
                    text,
                } => {
                    let w = width(text, *height);
                    let shift = match align {
                        Align::Left => 0.0,
                        Align::Centre => w * 0.5,
                        Align::Right => w,
                    };
                    let (c, s) = (angle.cos(), angle.sin());
                    let (x, y) = (at.x - c * shift, at.y - s * shift);
                    let _ = writeln!(
                        out,
                        "BT /F1 {} Tf {} {} {} {} {} {} Tm {} Tj ET",
                        num(*height),
                        num(c),
                        num(s),
                        num(-s),
                        num(c),
                        num(x),
                        num(y),
                        pdf_string(text)
                    );
                }
            }
        }
    }
    out
}

/// The sheet as a one-page PDF 1.4 file.
pub fn write(sheet: &SheetGeometry) -> Vec<u8> {
    let stream = content(sheet);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            num(sheet.width * PT_PER_MM),
            num(sheet.height * PT_PER_MM)
        ),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    ];
    let mut out: Vec<u8> = Vec::new();
    // The second line's bytes above 127 mark the file as binary (PDF Reference 3.4.1).
    out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1);
    for o in offsets {
        let _ = write!(table, "{o:010} 00000 n\r\n");
    }
    let _ = write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.extend_from_slice(table.as_bytes());
    out
}

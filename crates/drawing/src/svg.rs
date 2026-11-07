//! SVG 1.1 export (W3C SVG 1.1 Second Edition): one group per layer, sizes in mm,
//! circles and arcs exact. SVG's y axis points down, so y becomes `height - y`.

use crate::sheet::SheetGeometry;
use crate::{Align, Layer, Shape};
use std::f64::consts::PI;
use std::fmt::Write;

/// A number with up to four decimals, no trailing zeros.
pub(crate) fn num(v: f64) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The sheet as an SVG document.
pub fn write(sheet: &SheetGeometry) -> String {
    let (w, h) = (sheet.width, sheet.height);
    let y = |v: f64| num(h - v);
    let mut out = String::new();
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{0}mm" height="{1}mm" viewBox="0 0 {0} {1}">"#,
        num(w),
        num(h)
    );
    let _ = writeln!(
        out,
        r#"<rect x="0" y="0" width="{}" height="{}" fill="white"/>"#,
        num(w),
        num(h)
    );
    for layer in Layer::ALL {
        let dash = layer
            .dashes()
            .iter()
            .map(|d| num(*d))
            .collect::<Vec<_>>()
            .join(" ");
        let dash = if dash.is_empty() {
            String::new()
        } else {
            format!(r#" stroke-dasharray="{dash}""#)
        };
        let _ = writeln!(
            out,
            r#"<g id="{}" fill="none" stroke="black" stroke-width="{}" stroke-linecap="round"{dash}>"#,
            layer.name(),
            num(layer.width())
        );
        for item in sheet.items.iter().filter(|i| i.layer == layer) {
            match &item.shape {
                Shape::Line { a, b } => {
                    let _ = writeln!(
                        out,
                        r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
                        num(a.x),
                        y(a.y),
                        num(b.x),
                        y(b.y)
                    );
                }
                Shape::Circle { centre, radius } => {
                    let _ = writeln!(
                        out,
                        r#"<circle cx="{}" cy="{}" r="{}"/>"#,
                        num(centre.x),
                        y(centre.y),
                        num(*radius)
                    );
                }
                Shape::Arc {
                    centre,
                    radius,
                    start,
                    end,
                } => {
                    // Counter-clockwise on paper is the negative-angle direction on
                    // SVG's y-down canvas: sweep-flag 0. Split in halves so no single
                    // arc command is near a full turn.
                    let mid = 0.5 * (start + end);
                    let p = |a: f64| {
                        (
                            num(centre.x + radius * a.cos()),
                            y(centre.y + radius * a.sin()),
                        )
                    };
                    let (x0, y0) = p(*start);
                    let (x1, y1) = p(mid);
                    let (x2, y2) = p(*end);
                    let r = num(*radius);
                    let large = if (end - start) * 0.5 > PI { 1 } else { 0 };
                    let _ = writeln!(
                        out,
                        r#"<path d="M {x0} {y0} A {r} {r} 0 {large} 0 {x1} {y1} A {r} {r} 0 {large} 0 {x2} {y2}"/>"#
                    );
                }
                Shape::Text {
                    at,
                    height,
                    angle,
                    align,
                    text,
                } => {
                    let anchor = match align {
                        Align::Left => "start",
                        Align::Centre => "middle",
                        Align::Right => "end",
                    };
                    let (x, yy) = (num(at.x), y(at.y));
                    let rot = if angle.abs() > 1e-12 {
                        format!(
                            r#" transform="rotate({} {x} {yy})""#,
                            num(-angle.to_degrees())
                        )
                    } else {
                        String::new()
                    };
                    let _ = writeln!(
                        out,
                        r#"<text x="{x}" y="{yy}" font-family="Helvetica, Arial, sans-serif" font-size="{}" text-anchor="{anchor}" fill="black" stroke="none"{rot}>{}</text>"#,
                        num(*height),
                        escape(text)
                    );
                }
                Shape::Fill { points } => {
                    let pts = points
                        .iter()
                        .map(|p| format!("{},{}", num(p.x), y(p.y)))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = writeln!(
                        out,
                        r#"<polygon points="{pts}" fill="black" stroke="none"/>"#
                    );
                }
            }
        }
        let _ = writeln!(out, "</g>");
    }
    let _ = writeln!(out, "</svg>");
    out
}

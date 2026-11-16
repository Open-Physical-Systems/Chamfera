//! Line types: our own dash patterns, and lines broken into those dashes.
//!
//! Each pattern is a list of lengths in sheet mm at line type scale 1: dash, gap, dash,
//! gap, ... (ISO 128-20 style families, with values of our choosing). The exports hand the
//! pattern to the format (SVG `stroke-dasharray`, the PDF `d` operator, a DXF `LTYPE`
//! table entry); the screen and the tests break lines into dashes with [`dash_polyline`].
//! The pattern restarts at every entity and, for polylines, at every vertex.

use chamfera_doc::drafting::LineType;
use chamfera_geom::Point2;

/// Most dashes one polyline is broken into: beyond that the rest is drawn whole, so a
/// tiny scale on a long line cannot stall the screen.
pub const MAX_DASHES: usize = 50_000;

/// The dash pattern of `t` at scale 1 (sheet mm; empty for a continuous line).
pub fn pattern(t: LineType) -> &'static [f64] {
    match t {
        LineType::Continuous => &[],
        // Long dashes: 3 on, 1.5 off.
        LineType::Dashed => &[3.0, 1.5],
        // Short dashes for hidden edges drawn by hand: half the dashed pattern.
        LineType::Hidden => &[1.5, 0.75],
        // Long dash, short dash: 8, 1.5, 0.5, 1.5 (the sheet's own centre lines use it).
        LineType::Center => &[8.0, 1.5, 0.5, 1.5],
        // Long dash and two short dashes, for alternate positions and adjacent parts.
        LineType::Phantom => &[10.0, 1.5, 0.5, 1.5, 0.5, 1.5],
        // Dots: a quarter-millimetre dash every 1.5 mm.
        LineType::Dot => &[0.25, 1.25],
    }
}

/// The pattern of `t` with every length multiplied by `scale`.
pub fn scaled(t: LineType, scale: f64) -> Vec<f64> {
    pattern(t).iter().map(|d| d * scale).collect()
}

/// The name a DXF file gives line type `t`. The sheet's own `DASHED` and `CENTER` stay
/// at their fixed scale, so the user's line types (scaled per sheet) carry a `CH-`
/// prefix.
pub fn dxf_name(t: LineType) -> &'static str {
    match t {
        LineType::Continuous => "CONTINUOUS",
        LineType::Dashed => "CH-DASHED",
        LineType::Hidden => "CH-HIDDEN",
        LineType::Center => "CH-CENTER",
        LineType::Phantom => "CH-PHANTOM",
        LineType::Dot => "CH-DOT",
    }
}

/// A short description of `t` for the DXF table and the layer panel.
pub fn describe(t: LineType) -> &'static str {
    match t {
        LineType::Continuous => "Continuous",
        LineType::Dashed => "Dashed  ___ ___ ___",
        LineType::Hidden => "Hidden  _ _ _ _ _",
        LineType::Center => "Center  ____ _ ____",
        LineType::Phantom => "Phantom  ____ _ _ ____",
        LineType::Dot => "Dot  . . . . .",
    }
}

/// The dashes of the polyline through `points` with `pattern` (dash, gap, ...), as
/// segments. The pattern runs on across the vertices (callers that restart it per
/// segment pass one segment at a time). An empty pattern, or one with a length that is
/// not a positive finite number, keeps the line whole.
pub fn dash_polyline(points: &[Point2], pattern: &[f64]) -> Vec<[Point2; 2]> {
    let whole = || -> Vec<[Point2; 2]> {
        points
            .windows(2)
            .filter_map(|w| match w {
                [a, b] => Some([*a, *b]),
                _ => None,
            })
            .collect()
    };
    let total: f64 = pattern.iter().sum();
    if pattern.len() < 2
        || pattern.iter().any(|d| !(d.is_finite() && *d > 0.0))
        || !(total.is_finite() && total > 1e-9)
    {
        return whole();
    }
    let mut out = Vec::new();
    let mut k = 0usize;
    let mut left = pattern[0];
    let mut on = true;
    let mut pieces = 0usize;
    for w in points.windows(2) {
        let [a, b] = [w[0], w[1]];
        let len = a.distance(b);
        if !(len.is_finite() && len > 0.0) {
            continue;
        }
        let dir = (b - a) * (1.0 / len);
        let mut s = 0.0;
        while s < len {
            pieces += 1;
            if pieces > MAX_DASHES {
                out.push([a + dir * s, b]);
                return out;
            }
            let step = left.min(len - s);
            if on {
                out.push([a + dir * s, a + dir * (s + step)]);
            }
            s += step;
            left -= step;
            if left <= 1e-12 {
                k = (k + 1) % pattern.len();
                left = pattern.get(k).copied().unwrap_or(1.0);
                on = !on;
            }
        }
    }
    out
}

/// Points along an arc (counter-clockwise from `start` to `end`), at most `step` mm of
/// arc apart, at least 8 and at most 2048 pieces.
pub fn arc_points(centre: Point2, radius: f64, start: f64, end: f64, step: f64) -> Vec<Point2> {
    let sweep = end - start;
    let len = (radius * sweep).abs();
    let n = if step.is_finite() && step > 0.0 && len.is_finite() {
        ((len / step).ceil() as usize).clamp(8, 2048)
    } else {
        64
    };
    (0..=n)
        .map(|i| {
            let a = start + sweep * i as f64 / n as f64;
            Point2::new(centre.x + radius * a.cos(), centre.y + radius * a.sin())
        })
        .collect()
}

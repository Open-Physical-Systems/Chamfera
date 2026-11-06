//! Annotations, the parts list and the clipping of detail views, all drawn with our own
//! art in sheet mm.
//!
//! - Notes and leaders (ISO 128-22: a leader ends in an arrow on an outline, in a dot
//!   inside one).
//! - The surface texture symbol (ISO 1302 / ISO 21920-1 style): two legs at 60° to the
//!   surface, the longer one with a line carrying the requirement, and the bar that
//!   closes the legs for material removal required.
//! - Feature control frames (ISO 1101, ASME Y14.5): a row of cells, the characteristic's
//!   symbol, the tolerance (with `Ø` for a cylindrical zone), then the datum letters.
//! - Balloons (ISO 6433): a circle with the item number and a leader ending in a dot.
//! - The parts list (ISO 7573): above the title block, its header at the bottom and the
//!   items numbered upwards.

use crate::text::{clean, width};
use crate::{Align, Item, Layer, Shape};
use chamfera_doc::drawing::GdtCharacteristic;
use chamfera_geom::{Point2, Vec2};
use std::f64::consts::{PI, TAU};

/// Annotation text height (mm), ISO 3098 3.5.
pub const TEXT: f64 = 3.5;
/// A feature control frame's height and its symbol and datum cells' width (mm).
pub const FRAME_HEIGHT: f64 = 7.0;
/// A balloon's radius (mm).
pub const BALLOON_RADIUS: f64 = 4.5;
/// The parts list's row height and its columns' widths (mm): item, name, quantity,
/// material.
pub const ROW_HEIGHT: f64 = 7.0;
pub const COLUMNS: [f64; 4] = [14.0, 56.0, 14.0, 40.0];
const ARROW_LENGTH: f64 = 3.0;
const ARROW_HALF_WIDTH: f64 = 0.5;

/// One row of the parts list: a component and how many times the design holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRow {
    /// The component the row stands for, and its linked copies (each counts once).
    pub components: Vec<u32>,
    pub name: String,
    pub quantity: u32,
    /// The named appearance of its bodies, if any.
    pub material: String,
}

pub(crate) fn line(items: &mut Vec<Item>, layer: Layer, a: Point2, b: Point2) {
    if a.distance(b) > 1e-9 {
        items.push(Item {
            layer,
            shape: Shape::Line { a, b },
        });
    }
}

pub(crate) fn text(items: &mut Vec<Item>, at: Point2, height: f64, align: Align, s: &str) {
    if s.is_empty() {
        return;
    }
    items.push(Item {
        layer: Layer::Dim,
        shape: Shape::Text {
            at,
            height,
            angle: 0.0,
            align,
            text: s.to_owned(),
        },
    });
}

fn arrow(items: &mut Vec<Item>, tip: Point2, dir: Vec2) {
    let back = tip - dir * ARROW_LENGTH;
    let n = dir.perp() * ARROW_HALF_WIDTH;
    items.push(Item {
        layer: Layer::Dim,
        shape: Shape::Fill {
            points: [tip, back + n, back - n],
        },
    });
}

/// A filled dot of radius `r`, as a fan of triangles (every writer fills triangles).
fn dot(items: &mut Vec<Item>, c: Point2, r: f64) {
    let n = 12;
    for i in 0..n {
        let a0 = TAU * i as f64 / n as f64;
        let a1 = TAU * (i + 1) as f64 / n as f64;
        items.push(Item {
            layer: Layer::Dim,
            shape: Shape::Fill {
                points: [
                    c,
                    Point2::new(c.x + r * a0.cos(), c.y + r * a0.sin()),
                    Point2::new(c.x + r * a1.cos(), c.y + r * a1.sin()),
                ],
            },
        });
    }
}

fn circle(items: &mut Vec<Item>, layer: Layer, centre: Point2, radius: f64) {
    items.push(Item {
        layer,
        shape: Shape::Circle { centre, radius },
    });
}

/// Free text.
pub fn note(items: &mut Vec<Item>, at: Point2, s: &str, height: f64) {
    text(items, at, height, Align::Left, &clean(s, 256));
}

/// A leader: an arrow at the first point, the polyline, and the text on a short
/// horizontal landing after the last point.
pub fn leader(items: &mut Vec<Item>, points: &[Point2], s: &str) {
    let (Some(&tip), Some(&next), Some(&last)) = (points.first(), points.get(1), points.last())
    else {
        return;
    };
    if let Some(d) = (tip - next).normalized() {
        arrow(items, tip, d);
    }
    for w in points.windows(2) {
        line(items, Layer::Dim, w[0], w[1]);
    }
    let before = points
        .get(points.len().saturating_sub(2))
        .copied()
        .unwrap_or(tip);
    let right = last.x >= before.x;
    let s = clean(s, 256);
    let land = width(&s, TEXT) + 2.0;
    let end = Point2::new(if right { last.x + land } else { last.x - land }, last.y);
    line(items, Layer::Dim, last, end);
    let at = Point2::new(
        if right { last.x + 1.0 } else { last.x - 1.0 },
        last.y + 1.0,
    );
    let align = if right { Align::Left } else { Align::Right };
    text(items, at, TEXT, align, &s);
}

/// The surface texture symbol standing on `at`.
pub fn surface_finish(items: &mut Vec<Item>, at: Point2, s: &str, removal: bool) {
    let (s60, c60) = (PI / 3.0).sin_cos();
    // The short leg (up and to the left) and the long leg (up and to the right), at 60°
    // to the surface, about 1.4 and 2.8 times the text height.
    let short = at + Vec2::new(-c60, s60) * 5.0;
    let long = at + Vec2::new(c60, s60) * 10.0;
    line(items, Layer::Dim, at, short);
    line(items, Layer::Dim, at, long);
    if removal {
        // The bar that closes the legs at the short leg's height.
        line(items, Layer::Dim, short, at + Vec2::new(c60, s60) * 5.0);
    }
    let s = clean(s, 64);
    if !s.is_empty() {
        let w = width(&s, TEXT) + 2.0;
        line(items, Layer::Dim, long, long + Vec2::new(w, 0.0));
        text(
            items,
            long + Vec2::new(1.0, -TEXT - 0.8),
            TEXT,
            Align::Left,
            &s,
        );
    }
}

/// The symbol of a characteristic, drawn in a cell centred on `c`.
fn gdt_symbol(items: &mut Vec<Item>, k: GdtCharacteristic, c: Point2) {
    let p = |x: f64, y: f64| Point2::new(c.x + x, c.y + y);
    let l = |items: &mut Vec<Item>, a: Point2, b: Point2| line(items, Layer::Dim, a, b);
    match k {
        GdtCharacteristic::Straightness => l(items, p(-2.5, 0.0), p(2.5, 0.0)),
        GdtCharacteristic::Flatness => {
            let q = [p(-2.6, -1.2), p(1.4, -1.2), p(2.6, 1.2), p(-1.4, 1.2)];
            for i in 0..4 {
                l(items, q[i], q[(i + 1) % 4]);
            }
        }
        GdtCharacteristic::Circularity => circle(items, Layer::Dim, c, 2.0),
        GdtCharacteristic::Parallelism => {
            l(items, p(-2.2, -2.0), p(-0.2, 2.0));
            l(items, p(0.2, -2.0), p(2.2, 2.0));
        }
        GdtCharacteristic::Perpendicularity => {
            l(items, p(-2.5, -2.0), p(2.5, -2.0));
            l(items, p(0.0, -2.0), p(0.0, 2.3));
        }
        GdtCharacteristic::Position => {
            circle(items, Layer::Dim, c, 1.6);
            l(items, p(-2.6, 0.0), p(2.6, 0.0));
            l(items, p(0.0, -2.6), p(0.0, 2.6));
        }
    }
}

/// The text of a tolerance value: up to three decimals, trailing zeros dropped.
pub fn tolerance_text(v: f64, diameter: bool) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s.is_empty() || s == "-0" { "0" } else { s };
    if diameter {
        format!("Ø{s}")
    } else {
        s.to_owned()
    }
}

/// The cell widths of a frame: symbol, tolerance, then one per datum.
pub fn frame_cells(tolerance: &str, datums: usize) -> Vec<f64> {
    let mut cells = vec![
        FRAME_HEIGHT,
        (width(tolerance, TEXT) + 3.0).max(FRAME_HEIGHT),
    ];
    cells.extend(std::iter::repeat_n(FRAME_HEIGHT, datums));
    cells
}

/// A feature control frame with its lower-left corner at `at`.
pub fn gdt(
    items: &mut Vec<Item>,
    at: Point2,
    k: GdtCharacteristic,
    tolerance: f64,
    diameter: bool,
    datums: &[String],
    leader_to: Option<Point2>,
) {
    let tol = tolerance_text(tolerance, diameter);
    let cells = frame_cells(&tol, datums.len());
    let total: f64 = cells.iter().sum();
    let h = FRAME_HEIGHT;
    let (lo, hi) = (at, Point2::new(at.x + total, at.y + h));
    line(items, Layer::Dim, lo, Point2::new(hi.x, lo.y));
    line(items, Layer::Dim, Point2::new(hi.x, lo.y), hi);
    line(items, Layer::Dim, hi, Point2::new(lo.x, hi.y));
    line(items, Layer::Dim, Point2::new(lo.x, hi.y), lo);
    let mut x = at.x;
    for (i, w) in cells.iter().enumerate() {
        let mid = Point2::new(x + w * 0.5, at.y + h * 0.5);
        let baseline = Point2::new(mid.x, at.y + (h - TEXT) * 0.5 + 0.3);
        match i {
            0 => gdt_symbol(items, k, mid),
            1 => text(items, baseline, TEXT, Align::Centre, &tol),
            _ => {
                if let Some(d) = datums.get(i - 2) {
                    text(items, baseline, TEXT, Align::Centre, d);
                }
            }
        }
        x += w;
        if i + 1 < cells.len() {
            line(
                items,
                Layer::Dim,
                Point2::new(x, at.y),
                Point2::new(x, at.y + h),
            );
        }
    }
    if let Some(target) = leader_to {
        // From the frame's nearer end, with a short horizontal start, to an arrow.
        let start = if target.x <= at.x + total * 0.5 {
            Point2::new(at.x, at.y + h * 0.5)
        } else {
            Point2::new(at.x + total, at.y + h * 0.5)
        };
        let elbow = Point2::new(
            if target.x <= start.x {
                start.x - 3.0
            } else {
                start.x + 3.0
            },
            start.y,
        );
        line(items, Layer::Dim, start, elbow);
        line(items, Layer::Dim, elbow, target);
        if let Some(d) = (target - elbow).normalized() {
            arrow(items, target, d);
        }
    }
}

/// A centre line between two circle centres, reaching 2 mm past each.
pub fn centre_line(items: &mut Vec<Item>, a: Point2, b: Point2, layer: Layer) {
    let Some(u) = (b - a).normalized() else {
        return;
    };
    let layer = if layer == Layer::Dangling {
        Layer::Dangling
    } else {
        Layer::Center
    };
    line(items, layer, a - u * 2.0 - u * 3.0, b + u * 5.0);
}

/// A balloon at `at` with `number`, its leader ending in a dot at `target`.
pub fn balloon(items: &mut Vec<Item>, at: Point2, target: Point2, number: &str, layer: Layer) {
    let start = (target - at)
        .normalized()
        .map(|u| at + u * BALLOON_RADIUS)
        .unwrap_or(at);
    let mut own = Vec::new();
    circle(&mut own, Layer::Dim, at, BALLOON_RADIUS);
    line(&mut own, Layer::Dim, start, target);
    dot(&mut own, target, 0.7);
    text(
        &mut own,
        Point2::new(at.x, at.y - TEXT * 0.5),
        TEXT,
        Align::Centre,
        number,
    );
    for mut i in own {
        i.layer = layer;
        items.push(i);
    }
}

/// The parts list with its lower-right corner at `corner`.
pub fn parts_list(items: &mut Vec<Item>, corner: Point2, rows: &[PartRow]) {
    let total: f64 = COLUMNS.iter().sum();
    let left = corner.x - total;
    let n = rows.len() + 1;
    let top = corner.y + ROW_HEIGHT * n as f64;
    let b = |items: &mut Vec<Item>, p: Point2, q: Point2| line(items, Layer::Border, p, q);
    for k in 0..=n {
        let y = corner.y + ROW_HEIGHT * k as f64;
        b(items, Point2::new(left, y), Point2::new(corner.x, y));
    }
    let mut x = left;
    b(items, Point2::new(x, corner.y), Point2::new(x, top));
    for w in COLUMNS {
        x += w;
        b(items, Point2::new(x, corner.y), Point2::new(x, top));
    }
    let cell = |items: &mut Vec<Item>, col: usize, row: usize, s: &str, h: f64| {
        let x0 = left + COLUMNS.iter().take(col).sum::<f64>();
        let w = COLUMNS.get(col).copied().unwrap_or(10.0);
        let y = corner.y + ROW_HEIGHT * row as f64 + (ROW_HEIGHT - h) * 0.5;
        // Long text shrinks to fit its cell.
        let mut hv = h;
        let room = w - 2.0;
        let tw = width(s, h);
        if tw > room && tw > 0.0 {
            hv = (h * room / tw).max(1.0);
        }
        let (at, align) = if col == 1 || col == 3 {
            (Point2::new(x0 + 1.0, y), Align::Left)
        } else {
            (Point2::new(x0 + w * 0.5, y), Align::Centre)
        };
        items.push(Item {
            layer: Layer::Border,
            shape: Shape::Text {
                at,
                height: hv,
                angle: 0.0,
                align,
                text: s.to_owned(),
            },
        });
    };
    for (c, h) in ["ITEM", "NAME", "QTY", "MATERIAL"].iter().enumerate() {
        cell(items, c, 0, h, 2.5);
    }
    for (i, r) in rows.iter().enumerate() {
        let row = i + 1;
        cell(items, 0, row, &(i + 1).to_string(), TEXT);
        cell(items, 1, row, &clean(&r.name, 64), TEXT);
        cell(items, 2, row, &r.quantity.to_string(), TEXT);
        if !r.material.is_empty() {
            cell(items, 3, row, &clean(&r.material, 40), TEXT);
        }
    }
}

/// The part of segment `ab` inside the disc about `c` of radius `r`.
pub fn clip_line(a: Point2, b: Point2, c: Point2, r: f64) -> Option<[Point2; 2]> {
    let d = b - a;
    let f = a - c;
    let qa = d.dot(d);
    if qa <= 1e-18 {
        return None;
    }
    let qb = 2.0 * f.dot(d);
    let qc = f.dot(f) - r * r;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc <= 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t0 = ((-qb - sq) / (2.0 * qa)).max(0.0);
    let t1 = ((-qb + sq) / (2.0 * qa)).min(1.0);
    (t1 - t0 > 1e-12).then(|| [a + d * t0, a + d * t1])
}

/// The angle ranges of the arc (about `k`, radius `rho`, `start` to `end`) inside the
/// disc about `c` of radius `r`.
pub fn clip_arc(k: Point2, rho: f64, start: f64, end: f64, c: Point2, r: f64) -> Vec<(f64, f64)> {
    let d = k.distance(c);
    let inside: Option<(f64, f64)> = if d + rho <= r {
        Some((start, start + TAU))
    } else if d >= rho + r || rho >= d + r || d <= 1e-12 {
        None
    } else {
        // |k + ρ e(θ) - c| <= r  <=>  cos(θ - φ) >= (ρ² + d² - r²) / (2 ρ d).
        let cos = ((rho * rho + d * d - r * r) / (2.0 * rho * d)).clamp(-1.0, 1.0);
        let delta = cos.acos();
        let phi = (c.y - k.y).atan2(c.x - k.x);
        Some((phi - delta, phi + delta))
    };
    let Some((s0, s1)) = inside else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in -2..=2 {
        let shift = TAU * f64::from(m);
        let (a, b) = ((s0 + shift).max(start), (s1 + shift).min(end));
        if b - a > 1e-12 {
            out.push((a, b));
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-12 && (x.1 - y.1).abs() < 1e-12);
    out
}

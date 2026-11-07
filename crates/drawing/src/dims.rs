//! Dimensions on views: picking what they measure, measuring it, and drawing them in the
//! ISO 129-1 style (extension lines with a small gap and overshoot, closed filled
//! arrowheads, the value above the dimension line and parallel to it).

use crate::hlr::{Curve2, polar};
use crate::sheet::PlacedView;
use crate::text::width;
use crate::{Align, DrawingError, DrawingResult, Item, Layer, Shape};
use chamfera_doc::drawing::{DimensionKind, SheetDimension, ViewDirection};
use chamfera_geom::{Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// How near a pick must be to a vertex, edge or circle to snap to it (sheet mm).
pub const SNAP: f64 = 3.0;
/// Text height (mm), ISO 3098 3.5.
pub const TEXT_HEIGHT: f64 = 3.5;
/// Arrowhead length and half width (mm).
const ARROW_LENGTH: f64 = 3.0;
const ARROW_HALF_WIDTH: f64 = 0.5;
/// Gap between the object and an extension line, and the extension line's overshoot.
const EXT_GAP: f64 = 1.0;
const EXT_OVERSHOOT: f64 = 2.0;
/// Text sits this far from its dimension line.
const TEXT_GAP: f64 = 1.0;

/// What a new dimension stores and measures.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionPlan {
    pub kind: DimensionKind,
    /// In view coordinates (model mm), as [`SheetDimension::points`].
    pub points: Vec<Point2>,
    /// mm, or degrees for an angle.
    pub value: f64,
}

fn bad(msg: impl Into<String>) -> DrawingError {
    DrawingError::BadInput(msg.into())
}

/// The value a dimension shows: mm, or degrees for an angle. `None` when the points
/// do not fit the kind.
pub fn measure(kind: DimensionKind, points: &[Point2]) -> Option<f64> {
    let p = |i: usize| points.get(i).copied();
    let v = match kind {
        DimensionKind::Horizontal => (p(1)?.x - p(0)?.x).abs(),
        DimensionKind::Vertical => (p(1)?.y - p(0)?.y).abs(),
        DimensionKind::Aligned => p(0)?.distance(p(1)?),
        DimensionKind::Diameter => 2.0 * p(0)?.distance(p(1)?),
        DimensionKind::Radius => p(0)?.distance(p(1)?),
        DimensionKind::OrdinateX => p(1)?.x - p(0)?.x,
        DimensionKind::OrdinateY => p(1)?.y - p(0)?.y,
        DimensionKind::Angular => {
            let (v, a, b) = (p(0)?, p(1)?, p(2)?);
            let (u, w) = (a - v, b - v);
            let c = u.cross(w).atan2(u.dot(w)).abs();
            c.to_degrees()
        }
    };
    v.is_finite().then_some(v)
}

/// The text of a value: two decimals at most, trailing zeros dropped (ISO 129-1),
/// with `Ø`, `R` or `°`.
pub fn format_value(kind: DimensionKind, v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s == "-0" { "0" } else { s };
    match kind {
        DimensionKind::Diameter => format!("Ø{s}"),
        DimensionKind::Radius => format!("R{s}"),
        DimensionKind::Angular => format!("{s}°"),
        _ => s.to_owned(),
    }
}

fn nearest_on_segment(p: Point2, a: Point2, b: Point2) -> Point2 {
    let d = b - a;
    let l2 = d.dot(d);
    if l2 <= 0.0 {
        return a;
    }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    a + d * t
}

/// Snaps a pick (sheet mm) to the nearest projected vertex, else to the nearest point on
/// an edge, within [`SNAP`]. Returns view coordinates.
fn snap_point(view: &PlacedView, pick: Point2) -> Option<Point2> {
    let near = |q: Point2| view.to_sheet(q).distance(pick);
    let best = view
        .geometry
        .vertices()
        .into_iter()
        .map(|q| (near(q), q))
        .filter(|(d, _)| *d <= SNAP)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, q)) = best {
        return Some(q);
    }
    let pv = view.to_view(pick);
    let mut best: Option<(f64, Point2)> = None;
    for c in &view.geometry.curves {
        let q = match c.curve {
            Curve2::Line { a, b } => nearest_on_segment(pv, a, b),
            Curve2::Circle { centre, radius } | Curve2::Arc { centre, radius, .. } => {
                let d = pv - centre;
                match d.normalized() {
                    Some(u) => centre + u * radius,
                    None => continue,
                }
            }
        };
        let d = near(q);
        if d <= SNAP && best.is_none_or(|b| d < b.0) {
            best = Some((d, q));
        }
    }
    best.map(|b| b.1)
}

/// The nearest line of the view to a pick, within [`SNAP`].
fn pick_line(view: &PlacedView, pick: Point2) -> Option<(Point2, Point2)> {
    let pv = view.to_view(pick);
    view.geometry
        .curves
        .iter()
        .filter_map(|c| match c.curve {
            Curve2::Line { a, b } => Some((a, b)),
            _ => None,
        })
        .map(|(a, b)| {
            (
                view.to_sheet(nearest_on_segment(pv, a, b)).distance(pick),
                a,
                b,
            )
        })
        .filter(|(d, _, _)| *d <= SNAP)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, a, b)| (a, b))
}

/// Works out a dimension from picks on the sheet (mm): two points for a linear one (each
/// snapped to a projected vertex or edge), one point near a circle or arc for a diameter
/// or radius, one point on each of two edges for an angle.
pub fn plan_dimension(
    view: &PlacedView,
    kind: DimensionKind,
    picks: &[Point2],
) -> DrawingResult<DimensionPlan> {
    if view.direction == ViewDirection::Iso {
        return Err(bad(
            "dimensions go on orthographic views: the isometric view is foreshortened",
        ));
    }
    if !(view.scale.is_finite() && view.scale > 0.0) {
        return Err(bad("the view has no usable scale"));
    }
    let pick = |i: usize| {
        picks
            .get(i)
            .copied()
            .ok_or_else(|| bad(format!("a {} dimension needs more points", kind.id())))
    };
    let points = match kind {
        DimensionKind::Horizontal
        | DimensionKind::Vertical
        | DimensionKind::Aligned
        | DimensionKind::OrdinateX
        | DimensionKind::OrdinateY => {
            let mut out = Vec::new();
            for i in 0..2 {
                let p = pick(i)?;
                out.push(snap_point(view, p).ok_or_else(|| {
                    bad(format!(
                        "no vertex or edge of view {} within {SNAP} mm of ({:.2}, {:.2})",
                        view.id, p.x, p.y
                    ))
                })?);
            }
            out
        }
        DimensionKind::Diameter | DimensionKind::Radius => {
            let p = pick(0)?;
            let pv = view.to_view(p);
            let best = view
                .geometry
                .curves
                .iter()
                .filter_map(|c| match c.curve {
                    Curve2::Circle { centre, radius } | Curve2::Arc { centre, radius, .. } => {
                        Some((centre, radius))
                    }
                    Curve2::Line { .. } => None,
                })
                .map(|(c, r)| (((pv - c).length() - r).abs() * view.scale, c, r))
                .filter(|(d, _, _)| *d <= SNAP)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let (_, c, r) = best.ok_or_else(|| {
                bad(format!(
                    "no circle or arc of view {} within {SNAP} mm of ({:.2}, {:.2})",
                    view.id, p.x, p.y
                ))
            })?;
            let u = (pv - c).normalized().unwrap_or(Vec2::new(
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ));
            vec![c, c + u * r]
        }
        DimensionKind::Angular => {
            let (p1, p2) = (pick(0)?, pick(1)?);
            let none = |p: Point2| {
                bad(format!(
                    "no straight edge of view {} within {SNAP} mm of ({:.2}, {:.2})",
                    view.id, p.x, p.y
                ))
            };
            let l1 = pick_line(view, p1).ok_or_else(|| none(p1))?;
            let l2 = pick_line(view, p2).ok_or_else(|| none(p2))?;
            let (d1, d2) = (l1.1 - l1.0, l2.1 - l2.0);
            let den = d1.cross(d2);
            if den.abs() <= 1e-9 * d1.length() * d2.length() {
                return Err(bad(
                    "the two edges are parallel: there is no angle between them",
                ));
            }
            // The legs' intersection: l1.0 + d1 t.
            let t = (l2.0 - l1.0).cross(d2) / den;
            let v = l1.0 + d1 * t;
            let leg = |(a, b): (Point2, Point2), p: Point2| {
                let q = nearest_on_segment(view.to_view(p), a, b);
                if q.distance(v) > 1e-6 {
                    q
                } else if a.distance(v) > b.distance(v) {
                    a
                } else {
                    b
                }
            };
            vec![v, leg(l1, p1), leg(l2, p2)]
        }
    };
    let value =
        measure(kind, &points).ok_or_else(|| bad("the picked points give no finite value"))?;
    if kind != DimensionKind::Angular && !kind.is_ordinate() && value <= 1e-6 {
        return Err(bad(
            "the two points are the same: the dimension would be zero",
        ));
    }
    Ok(DimensionPlan {
        kind,
        points,
        value,
    })
}

fn push_line(items: &mut Vec<Item>, a: Point2, b: Point2) {
    if a.distance(b) > 1e-9 {
        items.push(Item {
            layer: Layer::Dim,
            shape: Shape::Line { a, b },
        });
    }
}

/// A closed filled arrowhead with its tip at `tip`, pointing along `dir` (unit).
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

fn label(items: &mut Vec<Item>, at: Point2, angle: f64, align: Align, s: String) {
    items.push(Item {
        layer: Layer::Dim,
        shape: Shape::Text {
            at,
            height: TEXT_HEIGHT,
            angle,
            align,
            text: s,
        },
    });
}

/// An extension line from the object point `p` towards the dimension line point `q`.
fn extension(items: &mut Vec<Item>, p: Point2, q: Point2) {
    let d = q - p;
    let len = d.length();
    if len <= EXT_GAP {
        return;
    }
    let u = d * (1.0 / len);
    push_line(items, p + u * EXT_GAP, q + u * EXT_OVERSHOOT);
}

/// A dimension line from `a` to `b` with arrows at both ends pointing outwards and the
/// text centred above it.
fn dimension_line(items: &mut Vec<Item>, a: Point2, b: Point2, s: String) {
    let d = b - a;
    let Some(mut u) = d.normalized() else {
        return;
    };
    push_line(items, a, b);
    arrow(items, a, -u);
    arrow(items, b, u);
    // Read from below or from the right (ISO 129-1 method 1).
    if u.x < -1e-9 || (u.x.abs() <= 1e-9 && u.y < 0.0) {
        u = -u;
    }
    let up = u.perp();
    let mid = a.lerp(b, 0.5);
    label(items, mid + up * TEXT_GAP, u.y.atan2(u.x), Align::Centre, s);
}

/// The offset that moves dimension `d` of `view` by `delta` (sheet mm) as a drag does:
/// a linear dimension's line follows the drag across it (crossing the measured points
/// flips it to the other side), a diameter's or radius's leader and an angle's arc grow
/// or shrink along their direction. `None` when the dimension has nothing to measure.
pub fn moved_offset(d: &SheetDimension, view: &PlacedView, delta: Vec2) -> Option<f64> {
    let pts: Vec<Point2> = d.points.iter().map(|p| view.to_sheet(*p)).collect();
    let off = d.offset;
    // A linear line at `line` across points spanning `lo..hi`, moved by `by`.
    let across = |lo: f64, hi: f64, line: f64, by: f64| {
        let to = line + by;
        if to >= hi {
            to - hi
        } else if to <= lo {
            to - lo
        } else {
            0.0
        }
    };
    match (d.kind, pts.as_slice()) {
        (DimensionKind::Horizontal, &[a, b]) => {
            let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
            let line = if off >= 0.0 { hi + off } else { lo + off };
            Some(across(lo, hi, line, delta.y))
        }
        (DimensionKind::Vertical, &[a, b]) => {
            let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
            let line = if off >= 0.0 { hi + off } else { lo + off };
            Some(across(lo, hi, line, delta.x))
        }
        (DimensionKind::Aligned, &[a, b]) => {
            let n = (b - a).normalized()?.perp();
            Some(off + delta.dot(n))
        }
        (DimensionKind::Diameter | DimensionKind::Radius, &[c, p]) => {
            let u = (p - c).normalized()?;
            Some((off.abs().max(ARROW_LENGTH) + delta.dot(u)).max(ARROW_LENGTH))
        }
        (DimensionKind::OrdinateX, &[_, _]) => Some(off + delta.y),
        (DimensionKind::OrdinateY, &[_, _]) => Some(off + delta.x),
        (DimensionKind::Angular, &[v, a, b]) => {
            let mid = ((a - v).normalized()? + (b - v).normalized()?).normalized()?;
            let r = if off.abs() > ARROW_LENGTH {
                off.abs()
            } else {
                15.0
            };
            Some((r + delta.dot(mid)).max(2.0 * ARROW_LENGTH))
        }
        _ => None,
    }
}

/// Draws one dimension of a placed view.
pub(crate) fn draw(d: &SheetDimension, view: &PlacedView, items: &mut Vec<Item>) {
    let Some(value) = measure(d.kind, &d.points) else {
        return;
    };
    let pts: Vec<Point2> = d.points.iter().map(|p| view.to_sheet(*p)).collect();
    let s = format_value(d.kind, value);
    let off = d.offset;
    match (d.kind, pts.as_slice()) {
        (DimensionKind::Horizontal, &[a, b]) => {
            let y = if off >= 0.0 {
                a.y.max(b.y) + off
            } else {
                a.y.min(b.y) + off
            };
            let (qa, qb) = (Point2::new(a.x, y), Point2::new(b.x, y));
            extension(items, a, qa);
            extension(items, b, qb);
            let (l, r) = if qa.x <= qb.x { (qa, qb) } else { (qb, qa) };
            dimension_line(items, l, r, s);
        }
        (DimensionKind::Vertical, &[a, b]) => {
            let x = if off >= 0.0 {
                a.x.max(b.x) + off
            } else {
                a.x.min(b.x) + off
            };
            let (qa, qb) = (Point2::new(x, a.y), Point2::new(x, b.y));
            extension(items, a, qa);
            extension(items, b, qb);
            let (lo, hi) = if qa.y <= qb.y { (qa, qb) } else { (qb, qa) };
            dimension_line(items, lo, hi, s);
        }
        (DimensionKind::Aligned, &[a, b]) => {
            let Some(u) = (b - a).normalized() else {
                return;
            };
            let n = u.perp();
            let (qa, qb) = (a + n * off, b + n * off);
            extension(items, a, qa);
            extension(items, b, qb);
            dimension_line(items, qa, qb, s);
        }
        (DimensionKind::Diameter | DimensionKind::Radius, &[c, p]) => {
            let Some(u) = (p - c).normalized() else {
                return;
            };
            let r = c.distance(p);
            let end = p + u * off.abs().max(ARROW_LENGTH);
            let diameter = d.kind == DimensionKind::Diameter;
            let start = if diameter { c - u * r } else { c };
            push_line(items, start, end);
            arrow(items, p, u);
            if diameter {
                arrow(items, c - u * r, -u);
            }
            // A short horizontal landing carries the value.
            let sx = if u.x >= 0.0 { 1.0 } else { -1.0 };
            let w = width(&s, TEXT_HEIGHT) + 2.0;
            let land = Point2::new(end.x + sx * w, end.y);
            push_line(items, end, land);
            let align = if sx > 0.0 { Align::Left } else { Align::Right };
            label(
                items,
                Point2::new(end.x + sx, end.y + TEXT_GAP),
                0.0,
                align,
                s,
            );
        }
        (DimensionKind::OrdinateX | DimensionKind::OrdinateY, &[o, p]) => {
            ordinate(
                items,
                d.kind == DimensionKind::OrdinateX,
                o,
                p,
                off,
                value,
                s,
            );
        }
        (DimensionKind::Angular, &[v, a, b]) => {
            let a1 = (a.y - v.y).atan2(a.x - v.x);
            let a2 = (b.y - v.y).atan2(b.x - v.x);
            let sweep = (a2 - a1).rem_euclid(TAU);
            let (start, sweep) = if sweep <= PI {
                (a1, sweep)
            } else {
                (a2, TAU - sweep)
            };
            let r = if off.abs() > ARROW_LENGTH {
                off.abs()
            } else {
                15.0
            };
            items.push(Item {
                layer: Layer::Dim,
                shape: Shape::Arc {
                    centre: v,
                    radius: r,
                    start,
                    end: start + sweep,
                },
            });
            let end = start + sweep;
            arrow(
                items,
                polar(v, r, start),
                Vec2::new(start.sin(), -start.cos()),
            );
            arrow(items, polar(v, r, end), Vec2::new(-end.sin(), end.cos()));
            for leg in [a, b] {
                let ang = (leg.y - v.y).atan2(leg.x - v.x);
                let len = v.distance(leg);
                if len < r {
                    push_line(
                        items,
                        polar(v, len + EXT_GAP, ang),
                        polar(v, r + EXT_OVERSHOOT, ang),
                    );
                }
            }
            let mid = start + sweep * 0.5;
            let mut angle = mid - FRAC_PI_2;
            while angle > FRAC_PI_2 {
                angle -= PI;
            }
            while angle <= -FRAC_PI_2 {
                angle += PI;
            }
            label(items, polar(v, r + TEXT_GAP, mid), angle, Align::Centre, s);
        }
        _ => {}
    }
}

/// The radius of the small circle that marks an ordinate set's origin (ISO 129-1).
const ORIGIN_MARK: f64 = 1.0;

/// An ordinate dimension from origin `o` to the point `p` (sheet mm): an extension line
/// from `p` across the measured axis to the line `offset` beyond the origin (above it
/// along x, right of it along y), the value written along its end. The zero dimension
/// at the origin also gets the origin's small circle.
fn ordinate(
    items: &mut Vec<Item>,
    along_x: bool,
    o: Point2,
    p: Point2,
    off: f64,
    v: f64,
    s: String,
) {
    let end = if along_x {
        Point2::new(p.x, o.y + off)
    } else {
        Point2::new(o.x + off, p.y)
    };
    let d = end - p;
    let len = d.length();
    if len <= EXT_GAP {
        return;
    }
    let u = d * (1.0 / len);
    push_line(items, p + u * EXT_GAP, end);
    // The origin's circle sits just beyond the line's end, clear of the value.
    if v.abs() < 1e-9 {
        items.push(Item {
            layer: Layer::Dim,
            shape: Shape::Circle {
                centre: end + u * ORIGIN_MARK,
                radius: ORIGIN_MARK,
            },
        });
    }
    // The text runs along the line's end, read from below or from the right; it sits
    // above a horizontal line and left of a vertical one, like any dimension's value.
    let (angle, side) = if along_x {
        (FRAC_PI_2, Vec2::new(-TEXT_GAP, 0.0))
    } else {
        (0.0, Vec2::new(0.0, TEXT_GAP))
    };
    let outward = if along_x { u.y > 0.0 } else { u.x > 0.0 };
    let align = if outward { Align::Right } else { Align::Left };
    label(items, end + side, angle, align, s);
}

//! Drafted 2D entities on a sheet: laid out by layer, picked, and edited with grips.
//!
//! [`draft`] turns a sheet's [`Drafting`] into [`DraftStyle`]s (one per layer, with its
//! dash pattern at the sheet's line type scale) and [`DraftShape`]s (the entities of the
//! layers that are on and thawed). Grips are the handles a selected entity shows:
//! [`grips`] lists them and [`drag_grip`] gives the entity with one grip moved to a new
//! point. Everything is exact: a grip lands exactly where it is dragged.

use crate::Shape;
use crate::linetype;
use crate::sheet::shape_distance;
use chamfera_doc::drafting::{Drafting, EntityKind, LineType};
use chamfera_geom::{Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, TAU};

/// How a user layer draws, worked out for one sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftStyle {
    pub name: String,
    /// sRGB; `None` is the sheet's ink colour.
    pub colour: Option<[u8; 3]>,
    pub line_type: LineType,
    /// The dash pattern at the sheet's line type scale, sheet mm (empty: continuous).
    pub pattern: Vec<f64>,
    /// Plotted width, mm.
    pub weight: f64,
    pub on: bool,
    pub frozen: bool,
    pub locked: bool,
}

impl DraftStyle {
    pub fn visible(&self) -> bool {
        self.on && !self.frozen
    }
}

/// One shape of a drafted entity, on layer `layer` (an index into the styles).
#[derive(Debug, Clone, PartialEq)]
pub struct DraftShape {
    pub entity: u32,
    pub layer: usize,
    pub shape: Shape,
}

/// The shapes an entity draws: a polyline is one line per segment (its dash pattern
/// restarts at each vertex).
pub fn shapes_of(kind: &EntityKind) -> Vec<Shape> {
    match kind {
        EntityKind::Line { a, b } => vec![Shape::Line { a: *a, b: *b }],
        EntityKind::Circle { centre, radius } => vec![Shape::Circle {
            centre: *centre,
            radius: *radius,
        }],
        EntityKind::Arc {
            centre,
            radius,
            start,
            end,
        } => vec![Shape::Arc {
            centre: *centre,
            radius: *radius,
            start: *start,
            end: *end,
        }],
        EntityKind::Polyline { points, closed } => segments(points, *closed)
            .into_iter()
            .map(|[a, b]| Shape::Line { a, b })
            .collect(),
    }
}

fn segments(points: &[Point2], closed: bool) -> Vec<[Point2; 2]> {
    let mut out: Vec<[Point2; 2]> = points
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some([*a, *b]),
            _ => None,
        })
        .collect();
    if closed
        && points.len() > 2
        && let (Some(first), Some(last)) = (points.first(), points.last())
    {
        out.push([*last, *first]);
    }
    out
}

/// The layer styles of a sheet, and the shapes of every entity on a layer that is on
/// and thawed (entities of hidden layers are left out, so no export shows them).
pub fn draft(d: &Drafting) -> (Vec<DraftStyle>, Vec<DraftShape>) {
    let styles: Vec<DraftStyle> = d
        .layers
        .iter()
        .map(|l| DraftStyle {
            name: l.name.clone(),
            colour: l.colour,
            line_type: l.line_type,
            pattern: linetype::scaled(l.line_type, d.linetype_scale),
            weight: l.weight,
            on: l.on,
            frozen: l.frozen,
            locked: l.locked,
        })
        .collect();
    let mut shapes = Vec::new();
    for e in &d.entities {
        let Some(i) = styles
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(&e.layer))
        else {
            continue;
        };
        if !styles.get(i).is_some_and(DraftStyle::visible) {
            continue;
        }
        for shape in shapes_of(&e.kind) {
            shapes.push(DraftShape {
                entity: e.id,
                layer: i,
                shape,
            });
        }
    }
    (styles, shapes)
}

/// The entity nearest `p` within `tolerance` sheet mm among the shown ones, skipping
/// entities on locked layers.
pub fn pick_entity(
    styles: &[DraftStyle],
    shapes: &[DraftShape],
    p: Point2,
    tolerance: f64,
) -> Option<u32> {
    let mut best: Option<(f64, u32)> = None;
    for s in shapes {
        if styles.get(s.layer).is_none_or(|l| l.locked || !l.visible()) {
            continue;
        }
        let d = shape_distance(&s.shape, p);
        if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, s.entity));
        }
    }
    best.map(|b| b.1)
}

/// What a grip does when dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GripKind {
    /// A line's end or a polyline's vertex: moves that point only.
    End,
    /// A line's or polyline segment's midpoint: moves the line or segment along.
    Mid,
    /// A circle's or arc's centre: moves the whole circle or arc.
    Centre,
    /// A circle's quadrant point or an arc's midpoint: changes the radius.
    Radius,
    /// An arc's start or end point: turns that end about the centre, radius kept.
    ArcEnd,
}

impl GripKind {
    pub fn id(self) -> &'static str {
        match self {
            GripKind::End => "end",
            GripKind::Mid => "mid",
            GripKind::Centre => "centre",
            GripKind::Radius => "radius",
            GripKind::ArcEnd => "arc_end",
        }
    }
}

/// One grip: where it is and what it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grip {
    pub kind: GripKind,
    pub at: Point2,
}

fn mid(a: Point2, b: Point2) -> Point2 {
    Point2::new(0.5 * (a.x + b.x), 0.5 * (a.y + b.y))
}

fn on_circle(c: Point2, r: f64, angle: f64) -> Point2 {
    Point2::new(c.x + r * angle.cos(), c.y + r * angle.sin())
}

/// The grips of an entity, in a fixed order:
/// - line: start, end, midpoint;
/// - circle: centre, then the quadrants at 0°, 90°, 180° and 270°;
/// - arc: centre, start, end, midpoint of the arc;
/// - polyline: every vertex, then every segment's midpoint (the closing one last).
pub fn grips(kind: &EntityKind) -> Vec<Grip> {
    let g = |kind, at| Grip { kind, at };
    match kind {
        EntityKind::Line { a, b } => vec![
            g(GripKind::End, *a),
            g(GripKind::End, *b),
            g(GripKind::Mid, mid(*a, *b)),
        ],
        EntityKind::Circle { centre, radius } => {
            let mut v = vec![g(GripKind::Centre, *centre)];
            for i in 0..4 {
                v.push(g(
                    GripKind::Radius,
                    on_circle(*centre, *radius, FRAC_PI_2 * f64::from(i)),
                ));
            }
            v
        }
        EntityKind::Arc {
            centre,
            radius,
            start,
            end,
        } => vec![
            g(GripKind::Centre, *centre),
            g(GripKind::ArcEnd, on_circle(*centre, *radius, *start)),
            g(GripKind::ArcEnd, on_circle(*centre, *radius, *end)),
            g(
                GripKind::Radius,
                on_circle(*centre, *radius, 0.5 * (start + end)),
            ),
        ],
        EntityKind::Polyline { points, closed } => {
            let mut v: Vec<Grip> = points.iter().map(|p| g(GripKind::End, *p)).collect();
            for [a, b] in segments(points, *closed) {
                v.push(g(GripKind::Mid, mid(a, b)));
            }
            v
        }
    }
}

/// The grip of `kind` nearest `p` within `tolerance`, by index.
pub fn grip_at(kind: &EntityKind, p: Point2, tolerance: f64) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (i, g) in grips(kind).iter().enumerate() {
        let d = g.at.distance(p);
        if d <= tolerance && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, i));
        }
    }
    best.map(|b| b.1)
}

/// `kind` moved by `d`.
pub fn moved(kind: &EntityKind, d: Vec2) -> EntityKind {
    match kind {
        EntityKind::Line { a, b } => EntityKind::Line {
            a: *a + d,
            b: *b + d,
        },
        EntityKind::Circle { centre, radius } => EntityKind::Circle {
            centre: *centre + d,
            radius: *radius,
        },
        EntityKind::Arc {
            centre,
            radius,
            start,
            end,
        } => EntityKind::Arc {
            centre: *centre + d,
            radius: *radius,
            start: *start,
            end: *end,
        },
        EntityKind::Polyline { points, closed } => EntityKind::Polyline {
            points: points.iter().map(|p| *p + d).collect(),
            closed: *closed,
        },
    }
}

/// `kind` with grip `grip` (an index into [`grips`]) dragged to `to`. The result is
/// checked: a drag that would make a zero-length line or a zero radius is refused.
pub fn drag_grip(kind: &EntityKind, grip: usize, to: Point2) -> Result<EntityKind, String> {
    let all = grips(kind);
    let g = all.get(grip).ok_or_else(|| {
        format!(
            "this {} has grips 0 to {}",
            kind.id(),
            all.len().saturating_sub(1)
        )
    })?;
    let shift = to - g.at;
    let out = match (kind, grip) {
        (EntityKind::Line { b, .. }, 0) => EntityKind::Line { a: to, b: *b },
        (EntityKind::Line { a, .. }, 1) => EntityKind::Line { a: *a, b: to },
        (EntityKind::Line { .. }, _) => moved(kind, shift),
        (EntityKind::Circle { .. }, 0) => moved(kind, shift),
        (EntityKind::Circle { centre, .. }, _) => EntityKind::Circle {
            centre: *centre,
            radius: centre.distance(to),
        },
        (EntityKind::Arc { .. }, 0) => moved(kind, shift),
        (
            EntityKind::Arc {
                centre,
                radius,
                start,
                end,
            },
            1 | 2,
        ) => {
            let angle = (to.y - centre.y).atan2(to.x - centre.x);
            let (mut s, mut e) = if grip == 1 {
                (angle, *end)
            } else {
                (*start, angle)
            };
            // Keep the arc counter-clockwise from start to end, within one turn.
            while e <= s + 1e-12 {
                e += TAU;
            }
            while e - s > TAU {
                e -= TAU;
            }
            // Keep the start angle small so repeated drags never grow it.
            let k = (s / TAU).floor();
            s -= k * TAU;
            e -= k * TAU;
            EntityKind::Arc {
                centre: *centre,
                radius: *radius,
                start: s,
                end: e,
            }
        }
        (
            EntityKind::Arc {
                centre, start, end, ..
            },
            _,
        ) => EntityKind::Arc {
            centre: *centre,
            radius: centre.distance(to),
            start: *start,
            end: *end,
        },
        (EntityKind::Polyline { points, closed }, i) => {
            let n = points.len();
            let mut pts = points.clone();
            if i < n {
                if let Some(p) = pts.get_mut(i) {
                    *p = to;
                }
            } else {
                // Segment j runs from vertex j to vertex j + 1 (the closing one from the
                // last vertex back to the first).
                let j = i - n;
                let k = if j + 1 < n { j + 1 } else { 0 };
                for idx in [j, k] {
                    if let Some(p) = pts.get_mut(idx) {
                        *p = *p + shift;
                    }
                }
            }
            EntityKind::Polyline {
                points: pts,
                closed: *closed,
            }
        }
    };
    out.check()?;
    Ok(out)
}

/// The grips of entity `id` in a drafting, for the screen and `drawing.entity.grips`.
pub fn entity_grips(d: &Drafting, id: u32) -> Option<Vec<Grip>> {
    d.entity(id).map(|e| grips(&e.kind))
}

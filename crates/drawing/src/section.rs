//! Section views (ISO 128-40 and ISO 128-44) and hatching (ISO 128-50).
//!
//! A cutting line from `a` to `b` on a view, with that view's direction of sight, spans
//! the cutting plane. The arrows of the cutting-plane line point along the direction the
//! section is seen in: across the line, to its right (or left when flipped). The model is
//! cut with the kernel's half-space boolean ([`chamfera_kernel::split_body`]: the solid
//! intersected with a box standing on the plane), the material beyond the plane is kept,
//! and the faces the cut made ([`SECTION_FACE`]) are hatched with narrow continuous lines
//! at 45°.
//!
//! The section view's paper axes keep the cutting line's direction: a cutting line that
//! runs up the parent view keeps the parent's up (the view is projected sideways, as a
//! left or right view is), a line that runs across it keeps the parent's right (projected
//! up or down, as a top or bottom view is).

use crate::hlr::Basis;
use crate::{Item, Layer, Shape};
use chamfera_doc::drawing::SectionCut;
use chamfera_geom::{Frame, Point2, Point3, Vec2};
use chamfera_kernel::Body;

/// The name of the faces a cut makes.
pub const SECTION_FACE: &str = "section";
/// Hatch lines run at 45° to the paper's x axis (ISO 128-50); the next part's at 135°,
/// so adjacent parts of an assembly are told apart.
pub const HATCH_ANGLE: f64 = std::f64::consts::FRAC_PI_4;

/// The hatch angle of the `part`-th body of a section.
pub fn hatch_angle(part: usize) -> f64 {
    if part.is_multiple_of(2) {
        HATCH_ANGLE
    } else {
        std::f64::consts::PI - HATCH_ANGLE
    }
}
/// Most hatch lines one region gets (a hostile scale must not make millions).
const MAX_HATCH_LINES: usize = 4_000;
/// Samples along one curved boundary edge of a cut face.
const CURVE_SEGMENTS: usize = 96;

/// The hatch spacing on paper (mm) for a view drawn at `scale`: 2.5 mm at 1:1, wider for
/// enlarged views and narrower for reduced ones, as the square root of the scale, kept
/// between 1 and 6 mm. ISO 128-50 asks for a spacing chosen in relation to the size of
/// the hatched area; this keeps it readable at any scale.
pub fn hatch_spacing(scale: f64) -> f64 {
    if !(scale.is_finite() && scale > 0.0) {
        return 2.5;
    }
    (2.5 * scale.sqrt()).clamp(1.0, 6.0)
}

/// A cutting line in 3D: its plane (normal along the direction of sight of the section,
/// the side kept), the section view's axes, and the line's direction on the parent.
#[derive(Debug, Clone, Copy)]
pub struct Cut {
    pub plane: Frame,
    pub basis: Basis,
    /// The direction of sight on the parent's paper (unit, where the arrows point).
    pub sight: Vec2,
}

/// The plane and view axes of `cut` on a view with axes `parent`.
pub fn cut_of(parent: &Basis, cut: &SectionCut) -> Option<Cut> {
    let w = (cut.b - cut.a).normalized()?;
    let mut sight = Vec2::new(w.y, -w.x);
    if cut.flip {
        sight = -sight;
    }
    let in3 = |v: Vec2| parent.right * v.x + parent.up * v.y;
    let v3 = in3(sight);
    let w3 = in3(w);
    let toward = -v3;
    let (right, up) = if w.y.abs() >= w.x.abs() {
        let up = w3 * w.y.signum();
        (up.cross(toward), up)
    } else {
        let right = w3 * w.x.signum();
        (right, toward.cross(right))
    };
    let origin = Point3::ORIGIN + in3(Vec2::new(cut.a.x, cut.a.y));
    let plane = Frame::from_normal(origin, v3)?;
    Some(Cut {
        plane,
        basis: Basis { right, up, toward },
        sight,
    })
}

fn side(plane: &Frame, p: Point3) -> f64 {
    (p - plane.origin()).dot(plane.z())
}

/// The bodies cut by the plane, keeping the material in front of it (along its normal).
/// A body the plane misses is kept whole when it lies in front and left out when behind.
pub fn cut_bodies(bodies: &[&Body], plane: &Frame) -> Result<Vec<Body>, String> {
    let mut out = Vec::new();
    for b in bodies {
        let Some(bb) = b.bounding_box() else { continue };
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for i in 0..8 {
            let p = Point3::new(
                if i & 1 == 0 { bb.min.x } else { bb.max.x },
                if i & 2 == 0 { bb.min.y } else { bb.max.y },
                if i & 4 == 0 { bb.min.z } else { bb.max.z },
            );
            let s = side(plane, p);
            lo = lo.min(s);
            hi = hi.max(s);
        }
        if lo >= -1e-9 {
            out.push((*b).clone());
        } else if hi > 1e-9 {
            let (_, front) = chamfera_kernel::split_body(b, plane, SECTION_FACE)
                .map_err(|e| format!("the section could not be cut: {e}"))?;
            out.push(front);
        }
    }
    if out.is_empty() {
        return Err("the cutting plane leaves no material in the direction of the arrows".into());
    }
    Ok(out)
}

/// The faces the cut made, projected: each a list of closed boundary loops, with the
/// index of the body it cuts (neighbouring parts are hatched at different angles).
pub fn regions(bodies: &[&Body], basis: &Basis) -> Vec<(usize, Vec<Vec<Point2>>)> {
    let mut out = Vec::new();
    for (bi, b) in bodies.iter().enumerate() {
        for f in b.faces() {
            if b.face_name(f).ok().flatten() != Some(SECTION_FACE) {
                continue;
            }
            let Ok(loops) = b.face_loops(f) else { continue };
            let mut region = Vec::new();
            for lp in loops {
                let mut poly: Vec<Point2> = Vec::new();
                for h in b.loop_half_edges(lp).unwrap_or_default() {
                    let Ok(Some(e)) = b.half_edge_edge(h) else {
                        continue;
                    };
                    let Ok(Some((curve, t0, t1))) = b.edge_curve(e) else {
                        continue;
                    };
                    let forward = match (b.half_edge_origin(h), b.edge_vertices(e)) {
                        (Ok(o), Ok([v0, _])) => o == v0,
                        _ => true,
                    };
                    let n = match curve {
                        chamfera_kernel::Curve::Segment(_) | chamfera_kernel::Curve::Line(_) => 1,
                        _ => CURVE_SEGMENTS,
                    };
                    for k in 0..n {
                        let s = k as f64 / n as f64;
                        let t = if forward {
                            t0 + (t1 - t0) * s
                        } else {
                            t1 - (t1 - t0) * s
                        };
                        poly.push(basis.project(curve.point_at(t)));
                    }
                }
                if poly.len() >= 3 {
                    region.push(poly);
                }
            }
            if !region.is_empty() {
                out.push((bi, region));
            }
        }
    }
    out
}

/// The signed area of a closed polygon (shoelace formula), counter-clockwise positive.
pub fn polygon_area(poly: &[Point2]) -> f64 {
    let n = poly.len();
    let mut a = 0.0;
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        a += p.x * q.y - q.x * p.y;
    }
    0.5 * a
}

/// The area a region covers by the even-odd rule: for loops that do not cross (an outer
/// boundary and the holes in it) the outer loop's area minus the holes'.
pub fn region_area(region: &[Vec<Point2>]) -> f64 {
    let mut areas: Vec<f64> = region.iter().map(|l| polygon_area(l).abs()).collect();
    areas.sort_by(|a, b| b.total_cmp(a));
    match areas.split_first() {
        Some((outer, holes)) => outer - holes.iter().sum::<f64>(),
        None => 0.0,
    }
}

/// Hatch lines filling `loops` (sheet mm) by the even-odd rule, at `angle` and `spacing`,
/// anchored at the sheet origin so neighbouring regions line up.
pub fn hatch(loops: &[Vec<Point2>], spacing: f64, angle: f64) -> Vec<[Point2; 2]> {
    if !(spacing.is_finite() && spacing > 1e-3) {
        return Vec::new();
    }
    let t = Vec2::new(angle.cos(), angle.sin());
    let n = t.perp();
    let along = |p: Point2| (p - Point2::ORIGIN).dot(t);
    let across = |p: Point2| (p - Point2::ORIGIN).dot(n);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for p in loops.iter().flatten() {
        lo = lo.min(across(*p));
        hi = hi.max(across(*p));
    }
    if !(lo.is_finite() && hi.is_finite()) {
        return Vec::new();
    }
    let first = (lo / spacing).ceil();
    let count = ((hi / spacing).floor() - first + 1.0).max(0.0);
    if count > MAX_HATCH_LINES as f64 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for k in 0..(count as usize) {
        let c = (first + k as f64) * spacing;
        let mut hits: Vec<f64> = Vec::new();
        for l in loops {
            let m = l.len();
            for i in 0..m {
                let (p, q) = (l[i], l[(i + 1) % m]);
                let (cp, cq) = (across(p), across(q));
                // Half-open rule: a vertex on the line counts once.
                if (cp <= c) != (cq <= c) {
                    let s = (c - cp) / (cq - cp);
                    hits.push(along(p) + (along(q) - along(p)) * s);
                }
            }
        }
        hits.sort_by(f64::total_cmp);
        for pair in hits.as_chunks::<2>().0 {
            let (s0, s1) = (pair[0], pair[1]);
            if s1 - s0 > 1e-6 {
                let at = |s: f64| Point2::ORIGIN + t * s + n * c;
                out.push([at(s0), at(s1)]);
            }
        }
    }
    out
}

/// The cutting-plane line on the parent view (sheet mm): a long-dash dot narrow line,
/// wide at its ends, with arrows showing the direction of sight and the letter at each.
pub fn cutting_line(a: Point2, b: Point2, sight: Vec2, label: &str, items: &mut Vec<Item>) {
    let Some(w) = (b - a).normalized() else {
        return;
    };
    // Reach a little past the outline, as the cutting line runs across the whole view.
    let (a, b) = (a - w * 4.0, b + w * 4.0);
    let push = |items: &mut Vec<Item>, layer: Layer, p: Point2, q: Point2| {
        items.push(Item {
            layer,
            shape: Shape::Line { a: p, b: q },
        });
    };
    push(items, Layer::Center, a, b);
    let thick = 6.0f64.min(a.distance(b) * 0.25);
    push(items, Layer::Visible, a, a + w * thick);
    push(items, Layer::Visible, b - w * thick, b);
    for end in [a, b] {
        let tip = end + sight * 9.0;
        push(items, Layer::Dim, end, tip);
        let back = tip - sight * 4.0;
        let n = sight.perp() * 1.2;
        items.push(Item {
            layer: Layer::Dim,
            shape: Shape::Fill {
                points: [tip, back + n, back - n],
            },
        });
        // The letter beside the arrow, outside the cutting line's span.
        let out = if end == a { -w } else { w };
        items.push(Item {
            layer: Layer::Dim,
            shape: Shape::Text {
                at: end + sight * 6.0 + out * 2.5 - Vec2::new(0.0, 2.5),
                height: 5.0,
                angle: 0.0,
                align: crate::Align::Centre,
                text: label.to_owned(),
            },
        });
    }
}

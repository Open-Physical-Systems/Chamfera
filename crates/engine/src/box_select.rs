//! Box selection in screen space: which bodies, or which points and curves of the active
//! sketch, a rectangle dragged over the viewport selects.
//!
//! - A window (dragged left to right) selects what lies fully inside the box.
//! - A crossing (dragged right to left) selects what the box touches or contains.
//!
//! Display meshes and curve polylines are projected with the same camera matrices the
//! renderer uses, so the box selects what the user sees inside it. Selection is not a
//! model decision; it only names which bodies or sketch items later commands act on.

use crate::Session;
use crate::sketching::{SketchItem, curve_polyline};
use chamfera_geom::Point3;
use chamfera_render::{Mat4, mat_mul, transform_point};
use serde::{Deserialize, Serialize};

/// Window (fully inside) or crossing (touched or inside).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxMode {
    Window,
    Crossing,
}

impl BoxMode {
    pub fn parse(s: &str) -> Option<BoxMode> {
        match s {
            "window" => Some(BoxMode::Window),
            "crossing" => Some(BoxMode::Crossing),
            _ => None,
        }
    }

    /// The usual CAD convention: dragged to the right is a window, to the left a crossing.
    pub fn from_drag(x1: f64, x2: f64) -> BoxMode {
        if x2 >= x1 {
            BoxMode::Window
        } else {
            BoxMode::Crossing
        }
    }
}

/// An axis-aligned rectangle in viewport pixels (origin top-left), edges included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenBox {
    pub min: (f64, f64),
    pub max: (f64, f64),
}

impl ScreenBox {
    /// The box spanned by two corners, in either order.
    pub fn from_corners(a: (f64, f64), b: (f64, f64)) -> ScreenBox {
        ScreenBox {
            min: (a.0.min(b.0), a.1.min(b.1)),
            max: (a.0.max(b.0), a.1.max(b.1)),
        }
    }

    pub fn contains(&self, p: (f64, f64)) -> bool {
        (self.min.0..=self.max.0).contains(&p.0) && (self.min.1..=self.max.1).contains(&p.1)
    }

    /// Whether any part of segment `a`-`b` lies in the box (Liang-Barsky clipping).
    pub fn touches_segment(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        // Each pair is (-direction, distance to the edge) for the four box edges.
        let edges = [
            (-dx, a.0 - self.min.0),
            (dx, self.max.0 - a.0),
            (-dy, a.1 - self.min.1),
            (dy, self.max.1 - a.1),
        ];
        for (p, q) in edges {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
                continue;
            }
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
        true
    }

    /// Whether triangle `t` and the box overlap: an edge of the triangle reaches into the
    /// box, or the box lies wholly inside the triangle.
    pub fn touches_triangle(&self, t: [(f64, f64); 3]) -> bool {
        if (0..3).any(|i| self.touches_segment(t[i], t[(i + 1) % 3])) {
            return true;
        }
        let centre = (
            (self.min.0 + self.max.0) * 0.5,
            (self.min.1 + self.max.1) * 0.5,
        );
        inside_triangle(centre, t)
    }
}

/// Whether `p` is inside triangle `t` (either winding). Degenerate triangles hold nothing.
fn inside_triangle(p: (f64, f64), t: [(f64, f64); 3]) -> bool {
    let cross =
        |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let d = [cross(t[0], t[1]), cross(t[1], t[2]), cross(t[2], t[0])];
    let area = (t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[1].1 - t[0].1) * (t[2].0 - t[0].0);
    if area.abs() < 1e-12 {
        return false;
    }
    d.iter().all(|v| *v >= 0.0) || d.iter().all(|v| *v <= 0.0)
}

/// Projects model points to pixels of a `w` x `h` viewport, as the renderer does.
struct Projector {
    vp: Mat4,
    w: f64,
    h: f64,
}

impl Projector {
    fn new(s: &Session, w: f64, h: f64) -> Projector {
        let bounds = s.scene.bounds();
        Projector {
            vp: mat_mul(
                &s.camera.projection_matrix(w / h, bounds.as_ref()),
                &s.camera.view_matrix(),
            ),
            w,
            h,
        }
    }

    /// The pixel of `p`, or `None` when it is outside the depth range (behind the eye).
    fn pixel(&self, p: Point3) -> Option<(f64, f64)> {
        let ndc = transform_point(&self.vp, p)?;
        if !(0.0..=1.0).contains(&ndc[2]) {
            return None;
        }
        Some(((ndc[0] + 1.0) * 0.5 * self.w, (1.0 - ndc[1]) * 0.5 * self.h))
    }
}

/// The visible bodies the box selects, in scene order.
pub fn bodies_in_box(s: &Session, b: ScreenBox, mode: BoxMode, w: f64, h: f64) -> Vec<u32> {
    let proj = Projector::new(s, w, h);
    s.scene
        .bodies
        .iter()
        .filter(|body| body.visible)
        .filter(|body| {
            let px: Vec<Option<(f64, f64)>> =
                body.mesh.positions.iter().map(|p| proj.pixel(*p)).collect();
            match mode {
                BoxMode::Window => {
                    !px.is_empty() && px.iter().all(|p| p.is_some_and(|p| b.contains(p)))
                }
                BoxMode::Crossing => {
                    px.iter().flatten().any(|p| b.contains(*p))
                        || body.mesh.triangles.iter().any(|t| {
                            let corner = |i: usize| t.get(i).and_then(|v| *px.get(*v as usize)?);
                            match (corner(0), corner(1), corner(2)) {
                                (Some(p0), Some(p1), Some(p2)) => b.touches_triangle([p0, p1, p2]),
                                _ => false,
                            }
                        })
                }
            }
        })
        .map(|body| body.id)
        .collect()
}

/// The points and curves of the active sketch the box selects: points inside it, and
/// curves fully inside (window) or touched (crossing). Points first, then curves.
pub fn sketch_items_in_box(
    s: &Session,
    b: ScreenBox,
    mode: BoxMode,
    w: f64,
    h: f64,
) -> Vec<SketchItem> {
    let Some(f) = s.active_sketch() else {
        return Vec::new();
    };
    let proj = Projector::new(s, w, h);
    let mut items: Vec<SketchItem> = f
        .sketch
        .points()
        .filter(|(_, p)| {
            proj.pixel(f.plane.from_plane(*p))
                .is_some_and(|p| b.contains(p))
        })
        .map(|(id, _)| SketchItem::Point(id.0))
        .collect();
    for (id, curve) in f.sketch.curves() {
        let px: Vec<Option<(f64, f64)>> = curve_polyline(&f.sketch, curve)
            .into_iter()
            .map(|p| proj.pixel(f.plane.from_plane(p)))
            .collect();
        let hit = match mode {
            BoxMode::Window => {
                !px.is_empty() && px.iter().all(|p| p.is_some_and(|p| b.contains(p)))
            }
            BoxMode::Crossing => {
                px.iter().flatten().any(|p| b.contains(*p))
                    || px.windows(2).any(|seg| match (seg[0], seg[1]) {
                        (Some(a), Some(c)) => b.touches_segment(a, c),
                        _ => false,
                    })
            }
        };
        if hit {
            items.push(SketchItem::Curve(id.0));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> ScreenBox {
        ScreenBox::from_corners((10.0, 10.0), (0.0, 0.0))
    }

    #[test]
    fn corners_in_either_order_make_the_same_box() {
        let b = unit_box();
        assert_eq!(b.min, (0.0, 0.0));
        assert_eq!(b.max, (10.0, 10.0));
        assert!(b.contains((0.0, 10.0)), "edges are inside");
        assert!(!b.contains((10.5, 5.0)));
    }

    #[test]
    fn segments_touch_when_they_cross_end_inside_or_lie_on_an_edge() {
        let b = unit_box();
        // Passes straight through, both ends outside.
        assert!(b.touches_segment((-5.0, 5.0), (15.0, 5.0)));
        // One end inside.
        assert!(b.touches_segment((5.0, 5.0), (50.0, 50.0)));
        // Runs along the top edge.
        assert!(b.touches_segment((-5.0, 0.0), (15.0, 0.0)));
        // Passes the corner (10, 10) at a distance: the line x + y = 21.
        assert!(!b.touches_segment((21.0, 0.0), (0.0, 21.0)));
        // Parallel and outside.
        assert!(!b.touches_segment((-5.0, 11.0), (15.0, 11.0)));
        // Zero length, outside and inside.
        assert!(!b.touches_segment((20.0, 20.0), (20.0, 20.0)));
        assert!(b.touches_segment((3.0, 3.0), (3.0, 3.0)));
    }

    #[test]
    fn a_box_inside_a_large_triangle_touches_it() {
        let b = unit_box();
        // Every corner of the box is inside this triangle, no edge crosses it.
        assert!(b.touches_triangle([(-100.0, -100.0), (200.0, -100.0), (-100.0, 200.0)]));
        // A triangle beyond the box's far corner: x + y >= 30.
        assert!(!b.touches_triangle([(30.0, 0.0), (60.0, 0.0), (0.0, 30.0)]));
        // Degenerate (all on one line) and outside.
        assert!(!b.touches_triangle([(20.0, 0.0), (30.0, 0.0), (40.0, 0.0)]));
    }

    #[test]
    fn the_drag_direction_picks_the_mode() {
        assert_eq!(BoxMode::from_drag(10.0, 50.0), BoxMode::Window);
        assert_eq!(BoxMode::from_drag(50.0, 10.0), BoxMode::Crossing);
        assert_eq!(BoxMode::parse("crossing"), Some(BoxMode::Crossing));
        assert_eq!(BoxMode::parse("Window"), None);
    }
}

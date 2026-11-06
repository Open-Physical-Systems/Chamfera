//! Sketch state in the session: the sketch being edited, its selection, how sketches are
//! shown (overlays), and screen-space picking of sketch points and curves.

use crate::Session;
use chamfera_geom::{Frame, Point2, Point3};
use chamfera_render::{Overlay, OverlayStyle, mat_mul, transform_point};
use chamfera_sketch::{Curve, CurveId, PointId, Sketch};
use serde::{Deserialize, Serialize};

/// Segments used to show a full circle (display only).
pub const CIRCLE_SEGMENTS: usize = 96;
/// How close (in screen pixels) a click must be to pick a sketch point.
pub const PICK_POINT_PX: f64 = 9.0;
/// How close (in screen pixels) a click must be to pick a sketch curve.
pub const PICK_CURVE_PX: f64 = 7.0;

/// A picked or selected part of the active sketch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SketchItem {
    Point(u32),
    Curve(u32),
}

/// A dimension's value and where to show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionLabel {
    pub constraint: u32,
    pub kind: String,
    pub value: f64,
    pub anchor: Point3,
}

/// The curve as a polyline in sketch coordinates (exact for lines). Arcs run
/// counter-clockwise from start to end.
pub fn curve_polyline(sketch: &Sketch, curve: &Curve) -> Vec<Point2> {
    match curve {
        Curve::Line { start, end, .. } => match (sketch.point(*start), sketch.point(*end)) {
            (Some(a), Some(b)) => vec![a, b],
            _ => Vec::new(),
        },
        Curve::Circle { centre, radius, .. } => match sketch.point(*centre) {
            Some(c) => (0..=CIRCLE_SEGMENTS)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / CIRCLE_SEGMENTS as f64;
                    Point2::new(c.x + radius * a.cos(), c.y + radius * a.sin())
                })
                .collect(),
            None => Vec::new(),
        },
        Curve::Arc {
            centre, start, end, ..
        } => {
            let (Some(c), Some(s), Some(e)) = (
                sketch.point(*centre),
                sketch.point(*start),
                sketch.point(*end),
            ) else {
                return Vec::new();
            };
            let r = c.distance(s);
            let a0 = (s - c).angle();
            let mut sweep = (e - c).angle() - a0;
            if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            let n = ((CIRCLE_SEGMENTS as f64) * sweep / std::f64::consts::TAU).ceil() as usize;
            let n = n.max(2);
            (0..=n)
                .map(|k| {
                    let a = a0 + sweep * k as f64 / n as f64;
                    Point2::new(c.x + r * a.cos(), c.y + r * a.sin())
                })
                .collect()
        }
        Curve::Ellipse { .. } | Curve::EllipticalArc { .. } => sketch
            .ellipse_arc_of(curve)
            .map(|e| e.polyline(CIRCLE_SEGMENTS))
            .unwrap_or_default(),
    }
}

impl Session {
    /// The sketch being edited, if any.
    pub fn active_sketch(&self) -> Option<&chamfera_doc::SketchFeature> {
        self.active_sketch.and_then(|id| self.doc.sketch(id).ok())
    }

    /// Rebuilds the overlays from the design's sketches. Fully determined geometry of the
    /// sketch being edited is drawn in the constrained colour, selected items highlighted.
    pub fn refresh_overlays(&mut self) {
        if self.active_sketch().is_none() {
            self.active_sketch = None;
            self.sketch_selection.clear();
        }
        let mut overlays = Vec::new();
        for f in &self.doc.sketches {
            let active = Some(f.id) == self.active_sketch;
            // A finished sketch that a feature has used is hidden, as CAD users expect:
            // its curves lie on the solid's edges and seams. Editing it shows it again.
            if !active && !self.doc.sketch_users(f.id).is_empty() {
                continue;
            }
            let dof = if active {
                f.sketch.analyze().ok()
            } else {
                None
            };
            let mut by_style: Vec<(OverlayStyle, Overlay)> = Vec::new();
            let mut push =
                |style: OverlayStyle, lines: Vec<[Point3; 2]>, points: Vec<Point3>| match by_style
                    .iter_mut()
                    .find(|(s, _)| *s == style)
                {
                    Some((_, o)) => {
                        o.lines.extend(lines);
                        o.points.extend(points);
                    }
                    None => by_style.push((
                        style,
                        Overlay {
                            style,
                            lines,
                            points,
                            on_top: active,
                        },
                    )),
                };
            for (id, curve) in f.sketch.curves() {
                let poly = curve_polyline(&f.sketch, curve);
                let lines = poly
                    .windows(2)
                    .map(|w| [f.plane.from_plane(w[0]), f.plane.from_plane(w[1])])
                    .collect();
                let selected = active && self.sketch_selection.contains(&SketchItem::Curve(id.0));
                let style = if selected {
                    OverlayStyle::Highlight
                } else if curve.is_construction() {
                    OverlayStyle::Construction
                } else if f.projection_of(id.0).is_some() {
                    OverlayStyle::Projected
                } else if dof.as_ref().is_some_and(|d| d.is_curve_determined(id)) || !active {
                    OverlayStyle::Constrained
                } else {
                    OverlayStyle::Sketch
                };
                push(style, lines, Vec::new());
            }
            if active {
                for (id, p) in f.sketch.points() {
                    let selected = self.sketch_selection.contains(&SketchItem::Point(id.0));
                    let style = if selected {
                        OverlayStyle::Highlight
                    } else if dof.as_ref().is_some_and(|d| d.is_point_determined(id)) {
                        OverlayStyle::Constrained
                    } else {
                        OverlayStyle::Sketch
                    };
                    push(style, Vec::new(), vec![f.plane.from_plane(p)]);
                }
            }
            // Highlights last, so they draw over everything else.
            by_style.sort_by_key(|(s, _)| *s == OverlayStyle::Highlight);
            overlays.extend(by_style.into_iter().map(|(_, o)| o));
        }
        // Construction geometry goes first, so sketches draw over it.
        let (mut construct, fills) = self.construct_overlays();
        construct.append(&mut overlays);
        construct.extend(self.toolpath_overlays());
        self.scene.overlays = construct;
        self.scene.fills = fills;
    }

    /// Projects a point of the active sketch's plane to a pixel of a `w` x `h` viewport.
    fn to_screen(&self, frame: &Frame, p: Point2, w: f64, h: f64) -> Option<(f64, f64)> {
        let bounds = self.scene.bounds();
        let vp = mat_mul(
            &self.camera.projection_matrix(w / h, bounds.as_ref()),
            &self.camera.view_matrix(),
        );
        let ndc = transform_point(&vp, frame.from_plane(p))?;
        if !(0.0..=1.0).contains(&ndc[2]) {
            return None;
        }
        Some(((ndc[0] + 1.0) * 0.5 * w, (1.0 - ndc[1]) * 0.5 * h))
    }

    /// The point or curve of the active sketch nearest pixel `(x, y)`: points first
    /// (within [`PICK_POINT_PX`]), then curves (within [`PICK_CURVE_PX`]).
    pub fn pick_sketch_item(&self, x: f64, y: f64, w: f64, h: f64) -> Option<SketchItem> {
        self.pick_sketch_item_filtered(x, y, w, h, &crate::pick_filter::PickFilter::ALL)
    }

    /// [`Session::pick_sketch_item`], skipping what the selection filter leaves out.
    pub fn pick_sketch_item_filtered(
        &self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        filter: &crate::pick_filter::PickFilter,
    ) -> Option<SketchItem> {
        let f = self.active_sketch()?;
        let near_point = f
            .sketch
            .points()
            .filter(|_| filter.vertices)
            .filter_map(|(id, p)| {
                let (sx, sy) = self.to_screen(&f.plane, p, w, h)?;
                Some((id, (sx - x).hypot(sy - y)))
            })
            .filter(|(_, d)| *d <= PICK_POINT_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = near_point {
            return Some(SketchItem::Point(id.0));
        }
        let mut best: Option<(CurveId, f64)> = None;
        for (id, curve) in f.sketch.curves() {
            let allowed = if curve.is_construction() {
                filter.construction
            } else {
                filter.sketch
            };
            if !allowed {
                continue;
            }
            let screen: Vec<(f64, f64)> = curve_polyline(&f.sketch, curve)
                .into_iter()
                .filter_map(|p| self.to_screen(&f.plane, p, w, h))
                .collect();
            for s in screen.windows(2) {
                let d = segment_distance((x, y), s[0], s[1]);
                if d <= PICK_CURVE_PX && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((id, d));
                }
            }
        }
        best.map(|(id, _)| SketchItem::Curve(id.0))
    }

    /// Where pixel `(x, y)` meets the active sketch's plane, in sketch coordinates.
    pub fn sketch_plane_point(&self, x: f64, y: f64, w: f64, h: f64) -> Option<Point2> {
        let f = self.active_sketch()?;
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let n = f.plane.z();
        let denom = n.dot(ray.direction);
        // The plane seen edge-on (within about 0.06 degrees) has no useful hit.
        if denom.abs() < 1e-3 {
            return None;
        }
        let t = n.dot(f.plane.origin() - ray.origin) / denom;
        let p = f.plane.to_plane(ray.at(t));
        p.is_valid().then_some(p)
    }

    /// Dimension values of the active sketch and where to draw them.
    pub fn dimension_labels(&self) -> Vec<DimensionLabel> {
        let Some(f) = self.active_sketch() else {
            return Vec::new();
        };
        let s = &f.sketch;
        let mid = |a: PointId, b: PointId| Some(s.point(a)?.lerp(s.point(b)?, 0.5));
        let line_mid = |l: CurveId| match s.curve(l) {
            Some(Curve::Line { start, end, .. }) => mid(*start, *end),
            _ => None,
        };
        let circle_label = |c: CurveId| {
            let centre = match s.curve(c)? {
                Curve::Circle { centre, .. } | Curve::Arc { centre, .. } => s.point(*centre)?,
                _ => return None,
            };
            let r = s.radius(c)?;
            // Up and to the right of the centre, on the circle.
            let d = std::f64::consts::FRAC_1_SQRT_2 * r;
            Some(Point2::new(centre.x + d, centre.y + d))
        };
        s.constraints()
            // A projected curve's own radius is not a dimension the user set.
            .filter(|(_, c)| {
                !c.curve_refs()
                    .iter()
                    .any(|k| f.projection_of(k.0).is_some())
            })
            .filter_map(|(id, c)| {
                use chamfera_sketch::Constraint as C;
                let value = c.value()?;
                let at = match c {
                    C::Length { line, .. } => line_mid(*line),
                    C::Distance { a, b, .. } => mid(*a, *b),
                    C::Radius { curve, .. } | C::Diameter { curve, .. } => circle_label(*curve),
                    C::PointLineDistance { point, line, .. } => {
                        Some(s.point(*point)?.lerp(line_mid(*line)?, 0.5))
                    }
                    C::Angle { a, b, .. } => Some(line_mid(*a)?.lerp(line_mid(*b)?, 0.5)),
                    _ => None,
                }?;
                Some(DimensionLabel {
                    constraint: id.0,
                    kind: c.kind_name().to_owned(),
                    value,
                    anchor: f.plane.from_plane(at),
                })
            })
            .collect()
    }
}

/// Distance from `p` to the segment `a b` in screen pixels.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

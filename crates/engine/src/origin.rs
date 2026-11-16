//! The origin in the viewport: the three origin planes, the three axes and the origin
//! point, shown or hidden one by one from the Browser's Origin folder (`view.origin`),
//! and picked by pixel (`view.origin_at`) so a click on one can start a sketch or fill
//! a construction tool's input. A view setting: the design does not change.
//!
//! Also here: fading bodies outside the active component.

use crate::Session;
use chamfera_doc::{OriginAxis, OriginPlane, ROOT};
use chamfera_geom::{Aabb, Point3, Ray, Vec3};
use chamfera_render::{Fill, FillStyle, Overlay, OverlayStyle, mat_mul, transform_point};

/// The half size of the origin planes with nothing in the design (mm).
const EMPTY_HALF: f64 = 25.0;
/// The smallest half size of the origin planes (mm).
const MIN_HALF: f64 = 10.0;
/// The planes reach this fraction of the farthest model coordinate from the origin.
const HALF_OF_MODEL: f64 = 0.75;
/// The axes reach this many plane half sizes each way.
const AXIS_REACH: f64 = 1.25;
/// A click within this many pixels of the origin point picks it.
pub const PICK_POINT_PX: f64 = 8.0;
/// A click within this many pixels of an axis picks it.
pub const PICK_AXIS_PX: f64 = 6.0;

/// Which origin items the viewport shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OriginShown {
    pub point: bool,
    pub x: bool,
    pub y: bool,
    pub z: bool,
    pub xy: bool,
    pub xz: bool,
    pub yz: bool,
}

/// One origin item: the point, an axis or a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginItem {
    Point,
    Axis(OriginAxis),
    Plane(OriginPlane),
}

/// Every origin item, in the Browser's order.
pub const ORIGIN_ITEMS: [OriginItem; 7] = [
    OriginItem::Point,
    OriginItem::Axis(OriginAxis::X),
    OriginItem::Axis(OriginAxis::Y),
    OriginItem::Axis(OriginAxis::Z),
    OriginItem::Plane(OriginPlane::Xy),
    OriginItem::Plane(OriginPlane::Xz),
    OriginItem::Plane(OriginPlane::Yz),
];

impl OriginItem {
    /// `"origin"`, an axis (`"x"`) or a plane (`"xy"`): the names construction
    /// commands take as references.
    pub fn name(self) -> &'static str {
        match self {
            OriginItem::Point => "origin",
            OriginItem::Axis(OriginAxis::X) => "x",
            OriginItem::Axis(OriginAxis::Y) => "y",
            OriginItem::Axis(OriginAxis::Z) => "z",
            OriginItem::Plane(OriginPlane::Xy) => "xy",
            OriginItem::Plane(OriginPlane::Xz) => "xz",
            OriginItem::Plane(OriginPlane::Yz) => "yz",
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            OriginItem::Point => "point",
            OriginItem::Axis(_) => "axis",
            OriginItem::Plane(_) => "plane",
        }
    }

    pub fn parse(s: &str) -> Option<OriginItem> {
        ORIGIN_ITEMS.into_iter().find(|i| i.name() == s)
    }
}

impl OriginShown {
    pub fn get(&self, item: OriginItem) -> bool {
        match item {
            OriginItem::Point => self.point,
            OriginItem::Axis(OriginAxis::X) => self.x,
            OriginItem::Axis(OriginAxis::Y) => self.y,
            OriginItem::Axis(OriginAxis::Z) => self.z,
            OriginItem::Plane(OriginPlane::Xy) => self.xy,
            OriginItem::Plane(OriginPlane::Xz) => self.xz,
            OriginItem::Plane(OriginPlane::Yz) => self.yz,
        }
    }

    pub fn set(&mut self, item: OriginItem, on: bool) {
        let slot = match item {
            OriginItem::Point => &mut self.point,
            OriginItem::Axis(OriginAxis::X) => &mut self.x,
            OriginItem::Axis(OriginAxis::Y) => &mut self.y,
            OriginItem::Axis(OriginAxis::Z) => &mut self.z,
            OriginItem::Plane(OriginPlane::Xy) => &mut self.xy,
            OriginItem::Plane(OriginPlane::Xz) => &mut self.xz,
            OriginItem::Plane(OriginPlane::Yz) => &mut self.yz,
        };
        *slot = on;
    }

    pub fn any(&self) -> bool {
        ORIGIN_ITEMS.iter().any(|i| self.get(*i))
    }
}

/// The half size of the origin planes for a model with these bounds: a share of the
/// farthest corner coordinate from the origin, so the planes reach into the model.
pub fn origin_half(model: Option<Aabb>) -> f64 {
    let Some(b) = model else {
        return EMPTY_HALF;
    };
    let far = [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z]
        .iter()
        .map(|c| c.abs())
        .filter(|c| c.is_finite())
        .fold(0.0, f64::max);
    (far * HALF_OF_MODEL).clamp(MIN_HALF, chamfera_geom::MAX_COORDINATE)
}

/// The two in-plane axes of an origin plane (indices 0 = X, 1 = Y, 2 = Z) and its normal.
fn plane_axes(p: OriginPlane) -> (usize, usize, usize) {
    match p {
        OriginPlane::Xy => (0, 1, 2),
        OriginPlane::Xz => (0, 2, 1),
        OriginPlane::Yz => (1, 2, 0),
    }
}

fn point_with(coords: [(usize, f64); 2]) -> Point3 {
    let mut c = [0.0; 3];
    for (i, v) in coords {
        if let Some(slot) = c.get_mut(i) {
            *slot = v;
        }
    }
    Point3::new(c[0], c[1], c[2])
}

/// The corners of an origin plane's square of half size `h`.
pub fn plane_square(p: OriginPlane, h: f64) -> [Point3; 4] {
    let (u, v, _) = plane_axes(p);
    [
        point_with([(u, -h), (v, -h)]),
        point_with([(u, h), (v, -h)]),
        point_with([(u, h), (v, h)]),
        point_with([(u, -h), (v, h)]),
    ]
}

fn coord(p: Point3, i: usize) -> f64 {
    match i {
        0 => p.x,
        1 => p.y,
        _ => p.z,
    }
}

/// Where a ray meets an origin plane's square of half size `h`: the distance along the
/// ray, or `None` when it misses (parallel, behind, or outside the square).
pub fn ray_plane_square(ray: &Ray, p: OriginPlane, h: f64) -> Option<f64> {
    let (u, v, n) = plane_axes(p);
    let dn = coord(Point3::ORIGIN + ray.direction, n);
    if dn.abs() < 1e-12 {
        return None;
    }
    let t = -coord(ray.origin, n) / dn;
    if t < 0.0 || !t.is_finite() {
        return None;
    }
    let q = ray.at(t);
    (coord(q, u).abs() <= h && coord(q, v).abs() <= h).then_some(t)
}

/// The distance from point `p` to segment `a`-`b` in the plane (pixels).
pub fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Along the ray, how far the point of the line through the origin along `dir` nearest
/// to the ray is.
fn ray_param_nearest_line(ray: &Ray, dir: Vec3) -> f64 {
    // Closest points of two lines: o + t d and s e (through the origin).
    let (d, e) = (ray.direction, dir);
    let w = ray.origin - Point3::ORIGIN;
    let (b, c) = (d.dot(e), e.dot(e));
    let denom = c - b * b; // d is unit length
    if denom.abs() < 1e-12 {
        return w.dot(d).abs();
    }
    (b * w.dot(e) - c * w.dot(d)) / denom
}

impl Session {
    /// Which origin items the viewport shows.
    pub fn origin_shown(&self) -> OriginShown {
        self.view_state.origin
    }

    /// Bounds of the visible bodies only (not the overlays, which hold the origin).
    fn body_bounds(&self) -> Option<Aabb> {
        self.scene
            .bodies
            .iter()
            .filter(|b| b.visible)
            .filter_map(|b| b.mesh.bounds())
            .reduce(Aabb::union)
    }

    /// The origin items shown, as overlays (outlines, axes, point) and fills (planes).
    pub(crate) fn origin_overlays(&self) -> (Vec<Overlay>, Vec<Fill>) {
        let shown = self.view_state.origin;
        if !shown.any() {
            return (Vec::new(), Vec::new());
        }
        let h = origin_half(self.body_bounds());
        let mut outline = Vec::new();
        let mut triangles = Vec::new();
        for plane in [OriginPlane::Xy, OriginPlane::Xz, OriginPlane::Yz] {
            if shown.get(OriginItem::Plane(plane)) {
                let [a, b, c, d] = plane_square(plane, h);
                outline.extend([[a, b], [b, c], [c, d], [d, a]]);
                triangles.extend([[a, b, c], [a, c, d]]);
            }
        }
        let mut overlays = Vec::new();
        if !outline.is_empty() || shown.point {
            overlays.push(Overlay {
                style: OverlayStyle::OriginPlane,
                lines: outline,
                points: if shown.point {
                    vec![Point3::ORIGIN]
                } else {
                    Vec::new()
                },
                on_top: false,
            });
        }
        for (axis, style) in [
            (OriginAxis::X, OverlayStyle::AxisX),
            (OriginAxis::Y, OverlayStyle::AxisY),
            (OriginAxis::Z, OverlayStyle::AxisZ),
        ] {
            if shown.get(OriginItem::Axis(axis)) {
                let d = axis.direction() * (h * AXIS_REACH);
                overlays.push(Overlay {
                    style,
                    lines: vec![[Point3::ORIGIN + (-d), Point3::ORIGIN + d]],
                    points: Vec::new(),
                    on_top: false,
                });
            }
        }
        let fills = if triangles.is_empty() {
            Vec::new()
        } else {
            vec![Fill {
                style: FillStyle::Origin,
                triangles,
                on_top: false,
            }]
        };
        (overlays, fills)
    }

    /// The shown origin item under pixel `(x, y)` of a `w` x `h` viewport, unless a body
    /// is in front of it: the point first, then the nearest axis on screen, then the
    /// nearest plane along the ray.
    pub fn origin_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<OriginItem> {
        let shown = self.view_state.origin;
        if !shown.any() {
            return None;
        }
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let body = self
            .scene
            .pick(&ray)
            .map_or(f64::INFINITY, |hit| hit.distance);
        let vp = mat_mul(
            &self.camera.projection_matrix(w / h, bounds.as_ref()),
            &self.camera.view_matrix(),
        );
        let pixel = |p: Point3| {
            let ndc = transform_point(&vp, p)?;
            (0.0..=1.0)
                .contains(&ndc[2])
                .then(|| ((ndc[0] + 1.0) * 0.5 * w, (1.0 - ndc[1]) * 0.5 * h))
        };
        let half = origin_half(self.body_bounds());
        // In front of any body: a little slack so an item on a face still counts.
        let seen = |t: f64| t <= body + 1e-6 * (1.0 + body.abs().min(1e12));
        if shown.point
            && let Some((px, py)) = pixel(Point3::ORIGIN)
            && ((px - x).powi(2) + (py - y).powi(2)).sqrt() <= PICK_POINT_PX
            && seen((Point3::ORIGIN - ray.origin).dot(ray.direction))
        {
            return Some(OriginItem::Point);
        }
        let mut best: Option<(f64, OriginItem)> = None;
        for axis in [OriginAxis::X, OriginAxis::Y, OriginAxis::Z] {
            if !shown.get(OriginItem::Axis(axis)) {
                continue;
            }
            let d = axis.direction() * (half * AXIS_REACH);
            let (Some(a), Some(b)) = (pixel(Point3::ORIGIN + (-d)), pixel(Point3::ORIGIN + d))
            else {
                continue;
            };
            let dist = segment_distance((x, y), a, b);
            if dist <= PICK_AXIS_PX
                && seen(ray_param_nearest_line(&ray, axis.direction()))
                && best.is_none_or(|(b, _)| dist < b)
            {
                best = Some((dist, OriginItem::Axis(axis)));
            }
        }
        if let Some((_, item)) = best {
            return Some(item);
        }
        [OriginPlane::Xy, OriginPlane::Xz, OriginPlane::Yz]
            .into_iter()
            .filter(|p| shown.get(OriginItem::Plane(*p)))
            .filter_map(|p| Some((ray_plane_square(&ray, p, half)?, p)))
            .filter(|(t, _)| seen(*t))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| OriginItem::Plane(p))
    }

    /// Marks the bodies outside the active component as faded (none when the design
    /// itself is active).
    pub(crate) fn fade_inactive(&mut self) {
        for b in &mut self.scene.bodies {
            b.faded = false;
        }
        let asm = &self.doc.assembly;
        let active = asm.active;
        if active == ROOT {
            return;
        }
        let faded: Vec<u32> = self
            .built
            .solids
            .iter()
            .filter(|s| !asm.is_inside(s.component, active))
            .filter_map(|s| s.scene_id)
            .collect();
        for id in faded {
            if let Some(b) = self.scene.body_mut(id) {
                b.faded = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_read_back() {
        for i in ORIGIN_ITEMS {
            assert_eq!(OriginItem::parse(i.name()), Some(i));
        }
        assert_eq!(OriginItem::parse("xyz"), None);
    }

    #[test]
    fn plane_size_follows_the_model() {
        assert_eq!(origin_half(None), EMPTY_HALF);
        // A box from (-10, 0, 0) to (80, 40, 20): the farthest coordinate is 80, so the
        // half size is 0.75 * 80 = 60.
        let b = Aabb {
            min: Point3::new(-10.0, 0.0, 0.0),
            max: Point3::new(80.0, 40.0, 20.0),
        };
        assert!((origin_half(Some(b)) - 60.0).abs() < 1e-12);
        // A tiny part still gets planes MIN_HALF across.
        let b = Aabb {
            min: Point3::ORIGIN,
            max: Point3::new(1.0, 1.0, 1.0),
        };
        assert_eq!(origin_half(Some(b)), MIN_HALF);
    }

    #[test]
    fn a_ray_meets_a_plane_square_only_inside_it() {
        // Straight down from (3, 4, 50) meets XY at distance 50, inside a 10 half size.
        let down = Ray::new(Point3::new(3.0, 4.0, 50.0), -Vec3::Z).unwrap();
        assert_eq!(ray_plane_square(&down, OriginPlane::Xy, 10.0), Some(50.0));
        assert_eq!(ray_plane_square(&down, OriginPlane::Xy, 2.0), None);
        // Parallel to XZ (no Y part): never meets it.
        assert_eq!(ray_plane_square(&down, OriginPlane::Xz, 100.0), None);
        // Pointing away from the plane.
        let up = Ray::new(Point3::new(3.0, 4.0, 50.0), Vec3::Z).unwrap();
        assert_eq!(ray_plane_square(&up, OriginPlane::Xy, 10.0), None);
        // Along +Y from (1, -20, 2): meets XZ (y = 0) at distance 20.
        let along = Ray::new(Point3::new(1.0, -20.0, 2.0), Vec3::Y).unwrap();
        assert_eq!(ray_plane_square(&along, OriginPlane::Xz, 5.0), Some(20.0));
    }

    #[test]
    fn segment_distance_is_to_the_nearest_point() {
        // A 3-4-5 triangle: (3, 4) is 5 from (0, 0) beyond the segment's start.
        assert!((segment_distance((-3.0, 4.0), (0.0, 0.0), (10.0, 0.0)) - 5.0).abs() < 1e-12);
        assert!((segment_distance((5.0, 2.0), (0.0, 0.0), (10.0, 0.0)) - 2.0).abs() < 1e-12);
        assert!((segment_distance((1.0, 1.0), (0.0, 0.0), (0.0, 0.0)) - 2f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn origin_items_are_drawn_and_picked_only_when_shown() {
        let mut s = Session::new();
        let vp = |x: f64, y: f64| json!({"x": x, "y": y, "width": 800, "height": 600});
        s.execute("view.standard", &json!({"view": "top"})).unwrap();
        // Nothing shown: no overlays, nothing picked.
        assert!(s.origin_overlays().0.is_empty());
        let r = s.execute("view.origin_at", &vp(400.0, 300.0)).unwrap();
        assert!(r["item"].is_null());
        s.execute("view.origin", &json!({"item": "xy", "visible": true}))
            .unwrap();
        assert!(s.scene.fills.iter().any(|f| f.style == FillStyle::Origin));
        // Looking straight down, the centre pixel's ray meets XY at the target.
        let r = s.execute("view.origin_at", &vp(400.0, 300.0)).unwrap();
        assert_eq!(r["item"], "xy");
        assert_eq!(r["kind"], "plane");
        // XZ is edge-on from the top; showing it changes nothing at the centre pixel
        // except that the X axis, once shown, wins over the plane there: the target
        // (0, 0, 0) projects to the centre, on the X axis.
        s.execute("view.origin", &json!({"item": "x", "visible": true}))
            .unwrap();
        let r = s.execute("view.origin_at", &vp(400.0, 300.0)).unwrap();
        assert_eq!(r["item"], "x");
        s.execute("view.origin", &json!({"item": "origin", "visible": true}))
            .unwrap();
        let r = s.execute("view.origin_at", &vp(400.0, 300.0)).unwrap();
        assert_eq!(r["item"], "origin");
        // Far off to the side: past every square and axis.
        let r = s.execute("view.origin_at", &vp(2.0, 2.0)).unwrap();
        assert!(r["item"].is_null(), "{r}");
        // All hidden again.
        s.execute("view.origin", &json!({"item": "all", "visible": false}))
            .unwrap();
        assert!(!s.view_state.origin.any());
        assert!(s.scene.fills.iter().all(|f| f.style != FillStyle::Origin));
        assert!(
            s.execute("view.origin", &json!({"item": "w", "visible": true}))
                .is_err()
        );
    }
}

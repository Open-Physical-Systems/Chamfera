//! The geometry behind the viewport's drag handles (manipulators): where a handle is on
//! screen, which handle a pixel grabs, and what a drag from one pixel to another means
//! in the model (a distance along an axis, a move in a plane, a turn about an axis).
//!
//! Plain math on the camera, no drawing and no input: the UI projects the handles with
//! [`ScreenView`], draws them, and turns the pointer into command parameters with these
//! functions. Pixels have their origin at the viewport's top-left corner, y down.

use crate::{Camera, Projection};
use chamfera_geom::{Aabb, Point3, Ray, Vec3};

/// A pixel position in the viewport, origin top-left, y down.
pub type Pixel = [f64; 2];

/// How long a triad's axes and an arrow are drawn, in pixels, whatever the zoom.
pub const HANDLE_PIXELS: f64 = 90.0;
/// How near, in pixels, the pointer must be to a handle's line to grab it.
pub const GRAB_PIXELS: f64 = 10.0;
/// A drag step is at least this many pixels on screen, so steps are not jittery.
pub const MIN_STEP_PIXELS: f64 = 6.0;
/// The angle step of a turn drag (5 degrees), and with Shift (1 degree).
pub const ANGLE_STEP: f64 = 5.0 * std::f64::consts::PI / 180.0;
pub const FINE_ANGLE_STEP: f64 = std::f64::consts::PI / 180.0;

/// Below this |sin| between a drag line and the view ray (about 2.6 degrees), a drag
/// along the line is too ill-conditioned to follow.
const MIN_SIN: f64 = 0.045;
/// Below this |cos| between a plane's normal and the view ray (about 87 degrees), the
/// plane is seen edge-on and a drag in it is ill-conditioned.
const MIN_COS: f64 = 0.05;

/// The camera and viewport a drag happens in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenView {
    pub camera: Camera,
    pub width: f64,
    pub height: f64,
    /// The scene's bounds (orthographic rays start in front of them).
    pub bounds: Option<Aabb>,
}

impl ScreenView {
    /// A view of a `width` x `height` pixel viewport, or `None` for an empty one.
    pub fn new(camera: Camera, width: f64, height: f64, bounds: Option<Aabb>) -> Option<Self> {
        (width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0).then_some(
            ScreenView {
                camera,
                width,
                height,
                bounds,
            },
        )
    }

    /// Half the visible height and width at the target, in mm.
    fn half_extent(&self) -> (f64, f64) {
        let hh = self.camera.half_height();
        (hh, hh * self.width / self.height)
    }

    /// Where world point `p` is on screen, or `None` when it is behind the eye. The exact
    /// inverse of [`ScreenView::ray`]: the ray through the returned pixel passes `p`.
    pub fn project(&self, p: Point3) -> Option<Pixel> {
        let b = self.camera.basis();
        let v = p - self.camera.eye();
        let (hh, hw) = self.half_extent();
        let (x, y) = match self.camera.projection {
            Projection::Perspective => {
                let depth = v.dot(b.forward);
                if depth <= self.camera.distance * 1e-6 {
                    return None;
                }
                let k = self.camera.distance / depth;
                (v.dot(b.right) * k / hw, v.dot(b.up) * k / hh)
            }
            Projection::Orthographic => (v.dot(b.right) / hw, v.dot(b.up) / hh),
        };
        let px = [(x + 1.0) * 0.5 * self.width, (1.0 - y) * 0.5 * self.height];
        (px[0].is_finite() && px[1].is_finite()).then_some(px)
    }

    /// The view ray through a pixel.
    pub fn ray(&self, px: Pixel) -> Option<Ray> {
        self.camera
            .ray_through(px[0], px[1], self.width, self.height, self.bounds.as_ref())
    }

    /// How many mm one pixel spans at world point `p` (across the view), or `None` when
    /// `p` is behind the eye.
    pub fn mm_per_pixel(&self, p: Point3) -> Option<f64> {
        let (hh, _) = self.half_extent();
        let at_target = 2.0 * hh / self.height;
        match self.camera.projection {
            Projection::Orthographic => Some(at_target),
            Projection::Perspective => {
                let depth = (p - self.camera.eye()).dot(self.camera.basis().forward);
                (depth > self.camera.distance * 1e-6)
                    .then(|| at_target * depth / self.camera.distance)
            }
        }
    }

    /// The length in mm that looks `pixels` long at `p`: handles drawn this long keep
    /// their size on screen whatever the zoom.
    pub fn screen_size(&self, p: Point3, pixels: f64) -> Option<f64> {
        self.mm_per_pixel(p).map(|k| k * pixels)
    }
}

/// The parameter `t` of the point `origin + t * dir` on a line that is closest to `ray`,
/// or `None` when the line runs (nearly) along the ray. `dir` need not be unit length;
/// `t` counts in its lengths.
///
/// Minimising |w + t d - s u|^2 over t and s (w = origin - ray origin, u the unit ray
/// direction) gives t (d.d - (d.u)^2) = (d.u)(u.w) - d.w.
pub fn line_parameter(ray: &Ray, origin: Point3, dir: Vec3) -> Option<f64> {
    let u = ray.direction;
    let w = origin - ray.origin;
    let dd = dir.dot(dir);
    let du = dir.dot(u);
    let denom = dd - du * du;
    if dd.is_nan() || dd <= 0.0 || denom <= MIN_SIN * MIN_SIN * dd {
        return None;
    }
    let t = (du * u.dot(w) - dir.dot(w)) / denom;
    t.is_finite().then_some(t)
}

/// A drag along the line through `origin` in direction `dir`: how far (in lengths of
/// `dir`) the pointer moved along it from pixel `from` to pixel `to`.
pub fn axis_drag(
    view: &ScreenView,
    origin: Point3,
    dir: Vec3,
    from: Pixel,
    to: Pixel,
) -> Option<f64> {
    let a = line_parameter(&view.ray(from)?, origin, dir)?;
    let b = line_parameter(&view.ray(to)?, origin, dir)?;
    Some(b - a)
}

/// Where `ray` meets the plane through `origin` with `normal`, or `None` when it runs
/// (nearly) along the plane.
pub fn plane_point(ray: &Ray, origin: Point3, normal: Vec3) -> Option<Point3> {
    let n = normal.normalized()?;
    let along = ray.direction.dot(n);
    if along.abs() < MIN_COS {
        return None;
    }
    let s = (origin - ray.origin).dot(n) / along;
    s.is_finite().then(|| ray.at(s))
}

/// A drag in the plane through `origin` with `normal`: the move in the plane from the
/// point under pixel `from` to the one under `to`.
pub fn plane_drag(
    view: &ScreenView,
    origin: Point3,
    normal: Vec3,
    from: Pixel,
    to: Pixel,
) -> Option<Vec3> {
    let a = plane_point(&view.ray(from)?, origin, normal)?;
    let b = plane_point(&view.ray(to)?, origin, normal)?;
    Some(b - a)
}

/// The signed angle of `b` from `a` about `axis` (right hand), in (-pi, pi].
fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f64 {
    a.cross(b).dot(axis).atan2(a.dot(b))
}

/// A turn about the axis through `centre` in direction `axis`: the angle (radians,
/// right hand about `axis`) the pointer swept around the centre from pixel `from` to
/// pixel `to`. Read where the view rays meet the turn's plane; when that plane is seen
/// edge-on, read around the centre on screen instead, signed by which way the axis
/// faces. `None` when the pointer is on the centre.
pub fn ring_angle(
    view: &ScreenView,
    centre: Point3,
    axis: Vec3,
    from: Pixel,
    to: Pixel,
) -> Option<f64> {
    let n = axis.normalized()?;
    let (ra, rb) = (view.ray(from)?, view.ray(to)?);
    if let (Some(a), Some(b)) = (plane_point(&ra, centre, n), plane_point(&rb, centre, n)) {
        let (va, vb) = (a - centre, b - centre);
        if va.length() > 0.0 && vb.length() > 0.0 {
            return Some(signed_angle(va, vb, n));
        }
        return None;
    }
    let c = view.project(centre)?;
    let (ax, ay) = (from[0] - c[0], c[1] - from[1]);
    let (bx, by) = (to[0] - c[0], c[1] - to[1]);
    if (ax == 0.0 && ay == 0.0) || (bx == 0.0 && by == 0.0) {
        return None;
    }
    // Counter-clockwise on screen (y up) is a positive turn about an axis that faces
    // the viewer.
    let screen = (ax * by - ay * bx).atan2(ax * bx + ay * by);
    let facing = n.dot(view.camera.basis().forward) < 0.0;
    Some(if facing { screen } else { -screen })
}

/// `value` rounded to the nearest multiple of `step` (unchanged for a step that is not a
/// positive finite number).
pub fn snap(value: f64, step: f64) -> f64 {
    if !(step.is_finite() && step > 0.0) || !value.is_finite() {
        return value;
    }
    let s = (value / step).round() * step;
    // Keep -0 out of the fields.
    if s == 0.0 { 0.0 } else { s }
}

/// The length step a drag snaps to: the smallest of 1/10, 1/5, 1/2 and 1 times the
/// grid spacing `grid` that is at least [`MIN_STEP_PIXELS`] on screen (at
/// `mm_per_pixel`), else the grid spacing itself; a tenth of that with `fine` (Shift).
pub fn drag_step(grid: f64, mm_per_pixel: f64, fine: bool) -> f64 {
    let grid = if grid.is_finite() && grid > 0.0 {
        grid
    } else {
        1.0
    };
    let least = if mm_per_pixel.is_finite() && mm_per_pixel > 0.0 {
        MIN_STEP_PIXELS * mm_per_pixel
    } else {
        0.0
    };
    let step = [0.1, 0.2, 0.5, 1.0]
        .into_iter()
        .map(|k| grid * k)
        .find(|s| *s >= least)
        .unwrap_or(grid);
    if fine { step / 10.0 } else { step }
}

/// The distance in pixels from `p` to the segment `a b`.
pub fn distance_to_segment(p: Pixel, a: Pixel, b: Pixel) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    (qx * qx + qy * qy).sqrt()
}

/// The distance in pixels from `p` to an open polyline (infinite for fewer than two
/// points).
pub fn distance_to_polyline(p: Pixel, points: &[Pixel]) -> f64 {
    points
        .windows(2)
        .map(|w| match w {
            [a, b] => distance_to_segment(p, *a, *b),
            _ => f64::INFINITY,
        })
        .fold(f64::INFINITY, f64::min)
}

/// True when `p` is inside the polygon (even-odd rule).
pub fn inside_polygon(p: Pixel, polygon: &[Pixel]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (Some(a), Some(b)) = (polygon.get(i), polygon.get((i + n - 1) % n)) else {
            continue;
        };
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// The polygon's area in square pixels (zero for a degenerate one).
fn polygon_area(polygon: &[Pixel]) -> f64 {
    let n = polygon.len();
    (0..n)
        .filter_map(|i| Some((polygon.get(i)?, polygon.get((i + 1) % n)?)))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        .abs()
        * 0.5
}

/// A straight drag handle: an arrow from `base` along the unit `dir`, `length` mm long.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    pub base: Point3,
    pub dir: Vec3,
    pub length: f64,
}

impl Arrow {
    pub fn tip(&self) -> Point3 {
        self.base + self.dir * self.length
    }

    /// True when pixel `p` is within `tolerance` pixels of the arrow on screen.
    pub fn hit(&self, view: &ScreenView, p: Pixel, tolerance: f64) -> bool {
        match (view.project(self.base), view.project(self.tip())) {
            (Some(a), Some(b)) => distance_to_segment(p, a, b) <= tolerance,
            _ => false,
        }
    }
}

/// A part of a [`Triad`]: an axis arrow, the square of the plane normal to an axis, or
/// the turning arc about an axis. Axes are numbered 0 (X), 1 (Y), 2 (Z).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriadPart {
    Axis(usize),
    Plane(usize),
    Ring(usize),
}

/// Where the plane squares sit along their two axes, as shares of the axis length.
const PLANE_NEAR: f64 = 0.3;
const PLANE_FAR: f64 = 0.5;
/// The turning arcs' radius, as a share of the axis length.
const RING_RADIUS: f64 = 0.8;
/// Where each turning arc starts and ends, in radians from the first of its two axes:
/// clear of both axis arrows (15 to 75 degrees).
const RING_FROM: f64 = 15.0 * std::f64::consts::PI / 180.0;
const RING_TO: f64 = 75.0 * std::f64::consts::PI / 180.0;
/// Points on each turning arc.
const RING_POINTS: usize = 21;

/// The move-and-turn handle: three axis arrows along world X, Y and Z, a square in each
/// coordinate plane between two axes (move in that plane) and an arc about each
/// axis between the other two (turn about it), `size` mm long, centred at `centre`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triad {
    pub centre: Point3,
    pub size: f64,
}

impl Triad {
    /// World axis `i` (0 X, 1 Y, 2 Z).
    pub fn axis(i: usize) -> Vec3 {
        match i {
            0 => Vec3::X,
            1 => Vec3::Y,
            _ => Vec3::Z,
        }
    }

    /// The two axes after `i`, in right-handed order (Y, Z for X; Z, X for Y; X, Y for Z).
    fn others(i: usize) -> (Vec3, Vec3) {
        (Triad::axis((i + 1) % 3), Triad::axis((i + 2) % 3))
    }

    /// Axis `i`'s arrow.
    pub fn arrow(&self, i: usize) -> Arrow {
        Arrow {
            base: self.centre,
            dir: Triad::axis(i),
            length: self.size,
        }
    }

    /// The corners of the square in the plane normal to axis `i`.
    pub fn plane_square(&self, i: usize) -> [Point3; 4] {
        let (u, v) = Triad::others(i);
        let (a, b) = (self.size * PLANE_NEAR, self.size * PLANE_FAR);
        let c = self.centre;
        [
            c + u * a + v * a,
            c + u * b + v * a,
            c + u * b + v * b,
            c + u * a + v * b,
        ]
    }

    /// Points along the arc about axis `i`, between the next axis and the one after
    /// (15 to 75 degrees from the next one, so it stays clear of the arrows).
    pub fn ring(&self, i: usize) -> Vec<Point3> {
        let (u, v) = Triad::others(i);
        let r = self.size * RING_RADIUS;
        (0..RING_POINTS)
            .map(|k| {
                let a = RING_FROM + (RING_TO - RING_FROM) * k as f64 / (RING_POINTS - 1) as f64;
                self.centre + u * (r * a.cos()) + v * (r * a.sin())
            })
            .collect()
    }

    /// The part under pixel `p`, if any: an arrow first, then a square, then an arc.
    /// Parts seen end-on or edge-on (shorter or thinner than the grab tolerance on
    /// screen) cannot be grabbed, since a drag on them would be ill-conditioned.
    pub fn hit(&self, view: &ScreenView, p: Pixel, tolerance: f64) -> Option<TriadPart> {
        let project = |q: Point3| view.project(q);
        let c = project(self.centre)?;
        let mut best: Option<(f64, TriadPart)> = None;
        for i in 0..3 {
            let Some(tip) = project(self.arrow(i).tip()) else {
                continue;
            };
            let long = (tip[0] - c[0]).hypot(tip[1] - c[1]) > 2.0 * tolerance;
            let d = distance_to_segment(p, c, tip);
            if long && d <= tolerance && best.is_none_or(|(e, _)| d < e) {
                best = Some((d, TriadPart::Axis(i)));
            }
        }
        if let Some((_, part)) = best {
            return Some(part);
        }
        for i in 0..3 {
            let square: Option<Vec<Pixel>> =
                self.plane_square(i).into_iter().map(project).collect();
            if let Some(square) = square
                && polygon_area(&square) > tolerance * tolerance
                && inside_polygon(p, &square)
            {
                return Some(TriadPart::Plane(i));
            }
        }
        for i in 0..3 {
            // An arc seen edge-on is a line through nothing to turn: skip it.
            let (u, v) = Triad::others(i);
            let forward = view.camera.basis().forward;
            let facing = Triad::axis(i).dot(forward).abs();
            if facing < MIN_COS || u.cross(v).length() == 0.0 {
                continue;
            }
            let arc: Option<Vec<Pixel>> = self.ring(i).into_iter().map(project).collect();
            let Some(arc) = arc else {
                continue;
            };
            let d = distance_to_polyline(p, &arc);
            if d <= tolerance && best.is_none_or(|(e, _)| d < e) {
                best = Some((d, TriadPart::Ring(i)));
            }
        }
        best.map(|(_, part)| part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StandardView;

    const W: f64 = 1000.0;
    const H: f64 = 800.0;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * (1.0 + b.abs())
    }

    /// A camera looking at the origin from `distance` in a standard view.
    fn view(v: StandardView, projection: Projection, distance: f64) -> ScreenView {
        let mut camera = Camera {
            target: Point3::ORIGIN,
            distance,
            projection,
            ..Camera::default()
        };
        camera.set_view(v);
        ScreenView::new(camera, W, H, None).unwrap()
    }

    /// mm per pixel at the target: the visible height 2 d tan(fov / 2) over H pixels.
    fn mm_per_px(distance: f64) -> f64 {
        2.0 * distance * (35f64.to_radians() * 0.5).tan() / H
    }

    #[test]
    fn the_target_projects_to_the_viewport_centre_and_back() {
        for proj in [Projection::Perspective, Projection::Orthographic] {
            let v = view(StandardView::Iso, proj, 250.0);
            let c = v.project(Point3::ORIGIN).unwrap();
            assert!(close(c[0], W / 2.0) && close(c[1], H / 2.0), "{c:?}");
            // A point projects to a pixel whose ray passes through it.
            let p = Point3::new(12.0, -7.0, 30.0);
            let px = v.project(p).unwrap();
            let ray = v.ray(px).unwrap();
            let off = (p - ray.origin) - ray.direction * (p - ray.origin).dot(ray.direction);
            assert!(off.length() < 1e-9, "{proj:?}: {}", off.length());
        }
    }

    #[test]
    fn mm_per_pixel_is_the_visible_height_over_the_pixels_and_grows_with_depth() {
        // 250 mm away with a 35 degree field: 2 * 250 * tan(17.5 deg) / 800 mm a pixel.
        let v = view(StandardView::Front, Projection::Perspective, 250.0);
        let k = v.mm_per_pixel(Point3::ORIGIN).unwrap();
        assert!(close(k, mm_per_px(250.0)), "{k}");
        // The front view looks along +Y from y = -250: a point at y = +250 is twice as
        // deep, so a pixel spans twice as much there.
        let far = v.mm_per_pixel(Point3::new(0.0, 250.0, 0.0)).unwrap();
        assert!(close(far, 2.0 * k), "{far}");
        // Behind the eye there is no size.
        assert!(v.mm_per_pixel(Point3::new(0.0, -300.0, 0.0)).is_none());
        // Orthographic: the same everywhere.
        let o = view(StandardView::Front, Projection::Orthographic, 250.0);
        let at = o.mm_per_pixel(Point3::new(0.0, 250.0, 0.0)).unwrap();
        assert!(close(at, mm_per_px(250.0)), "{at}");
    }

    #[test]
    fn a_handle_keeps_its_screen_size_when_zooming() {
        let near = view(StandardView::Iso, Projection::Perspective, 100.0);
        let far = view(StandardView::Iso, Projection::Perspective, 400.0);
        let a = near.screen_size(Point3::ORIGIN, HANDLE_PIXELS).unwrap();
        let b = far.screen_size(Point3::ORIGIN, HANDLE_PIXELS).unwrap();
        // Four times as far: four times as long in mm, 90 pixels on screen both times.
        assert!(close(b, 4.0 * a), "{a} {b}");
        assert!(close(a, HANDLE_PIXELS * mm_per_px(100.0)), "{a}");
        let tip = near.project(Point3::new(a, 0.0, 0.0)).unwrap();
        let c = near.project(Point3::ORIGIN).unwrap();
        // The front view's X axis lies across the screen; in iso it is foreshortened,
        // so check the front view for the exact length.
        let front = view(StandardView::Front, Projection::Perspective, 100.0);
        let len = front.screen_size(Point3::ORIGIN, HANDLE_PIXELS).unwrap();
        let t = front.project(Point3::new(len, 0.0, 0.0)).unwrap();
        let c0 = front.project(Point3::ORIGIN).unwrap();
        assert!(close(t[0] - c0[0], HANDLE_PIXELS), "{t:?}");
        assert!(tip != c);
    }

    #[test]
    fn line_parameter_is_the_closest_point_on_the_line() {
        // A ray straight along +Y through x = 3, z = 5 passes closest to the X axis at
        // x = 3; along a direction twice as long, that is t = 1.5.
        let ray = Ray::new(Point3::new(3.0, -10.0, 5.0), Vec3::Y).unwrap();
        let t = line_parameter(&ray, Point3::ORIGIN, Vec3::X).unwrap();
        assert!(close(t, 3.0), "{t}");
        let t2 = line_parameter(&ray, Point3::ORIGIN, Vec3::X * 2.0).unwrap();
        assert!(close(t2, 1.5), "{t2}");
        // A ray along the line itself gives nothing to follow.
        let along = Ray::new(Point3::new(0.0, 0.0, 0.0), Vec3::X).unwrap();
        assert!(line_parameter(&along, Point3::ORIGIN, Vec3::X).is_none());
    }

    #[test]
    fn dragging_along_an_axis_moves_by_the_pixels_times_the_pixel_size() {
        // Front view, perspective, 250 mm away: X runs across the screen through the
        // target, so 100 pixels to the right is 100 * 2 * 250 * tan(17.5 deg) / 800 mm.
        for proj in [Projection::Perspective, Projection::Orthographic] {
            let v = view(StandardView::Front, proj, 250.0);
            let c = [W / 2.0, H / 2.0];
            let d = axis_drag(&v, Point3::ORIGIN, Vec3::X, c, [c[0] + 100.0, c[1]]).unwrap();
            assert!(close(d, 100.0 * mm_per_px(250.0)), "{proj:?}: {d}");
            // Moving the pointer across the axis (straight up the screen) does not
            // move along X.
            let up = axis_drag(&v, Point3::ORIGIN, Vec3::X, c, [c[0], c[1] - 80.0]).unwrap();
            assert!(up.abs() < 1e-9, "{up}");
            // Z runs up the screen: 50 pixels up is +50 pixels' worth along Z.
            let z = axis_drag(&v, Point3::ORIGIN, Vec3::Z, c, [c[0], c[1] - 50.0]).unwrap();
            assert!(close(z, 50.0 * mm_per_px(250.0)), "{z}");
        }
        // Y points straight into the front view: no drag along it.
        let v = view(StandardView::Front, Projection::Perspective, 250.0);
        let c = [W / 2.0, H / 2.0];
        assert!(axis_drag(&v, Point3::ORIGIN, Vec3::Y, c, [c[0] + 10.0, c[1]]).is_none());
    }

    #[test]
    fn dragging_in_a_plane_follows_the_pointer_on_that_plane() {
        // Top view, orthographic: screen right is +X and screen up is +Y (the camera is
        // a hair off vertical, which the plane intersection removes exactly in X).
        let v = view(StandardView::Top, Projection::Orthographic, 250.0);
        let c = [W / 2.0, H / 2.0];
        let k = mm_per_px(250.0);
        let d = plane_drag(&v, Point3::ORIGIN, Vec3::Z, c, [c[0] + 40.0, c[1] - 30.0]).unwrap();
        assert!(close(d.x, 40.0 * k), "{d:?}");
        // Up the screen: 30 px, divided by sin(pitch), which is 1 to 1e-10.
        assert!((d.y - 30.0 * k).abs() < 1e-6, "{d:?}");
        assert!(d.z.abs() < 1e-9, "{d:?}");
        // The XZ plane is seen edge-on from the top: no drag in it.
        assert!(plane_drag(&v, Point3::ORIGIN, Vec3::Y, c, [c[0] + 4.0, c[1]]).is_none());
    }

    #[test]
    fn a_quarter_turn_of_the_pointer_about_the_centre_is_a_quarter_turn() {
        // Top view: from screen right (+X) to screen up (+Y) is +90 degrees about +Z.
        let v = view(StandardView::Top, Projection::Orthographic, 250.0);
        let c = [W / 2.0, H / 2.0];
        let a = ring_angle(
            &v,
            Point3::ORIGIN,
            Vec3::Z,
            [c[0] + 100.0, c[1]],
            [c[0], c[1] - 100.0],
        )
        .unwrap();
        assert!(close(a, std::f64::consts::FRAC_PI_2), "{a}");
        // About -Z the same sweep is -90 degrees.
        let b = ring_angle(
            &v,
            Point3::ORIGIN,
            -Vec3::Z,
            [c[0] + 100.0, c[1]],
            [c[0], c[1] - 100.0],
        )
        .unwrap();
        assert!(close(b, -std::f64::consts::FRAC_PI_2), "{b}");
        // On the centre there is no angle.
        assert!(ring_angle(&v, Point3::ORIGIN, Vec3::Z, c, [c[0] + 5.0, c[1]]).is_none());
    }

    #[test]
    fn an_edge_on_turn_is_read_around_the_centre_on_screen() {
        // Front view: the Y axis points away from the viewer (into the screen), so the
        // XZ plane faces the viewer, but turning about X puts the YZ plane edge-on.
        // About Y (facing away): counter-clockwise on screen from +X (right) to +Z (up)
        // is a turn from X to Z, which is -90 degrees about +Y (Z x X = Y).
        let v = view(StandardView::Front, Projection::Perspective, 250.0);
        let c = [W / 2.0, H / 2.0];
        let a = ring_angle(
            &v,
            Point3::ORIGIN,
            Vec3::Y,
            [c[0] + 50.0, c[1]],
            [c[0], c[1] - 50.0],
        )
        .unwrap();
        assert!(close(a, -std::f64::consts::FRAC_PI_2), "{a}");
        // About X, the plane YZ is edge-on: the screen fallback still answers, and a
        // pointer straight up then straight left is a quarter turn one way or the other.
        let b = ring_angle(
            &v,
            Point3::ORIGIN,
            Vec3::X,
            [c[0], c[1] - 50.0],
            [c[0] - 50.0, c[1]],
        )
        .unwrap();
        assert!(close(b.abs(), std::f64::consts::FRAC_PI_2), "{b}");
    }

    #[test]
    fn snapping_rounds_to_the_nearest_step() {
        assert!(close(snap(7.3, 2.0), 8.0));
        assert!(close(snap(-7.3, 2.0), -8.0));
        assert!(close(snap(6.9, 0.5), 7.0));
        assert_eq!(snap(-0.2, 1.0).to_bits(), 0f64.to_bits(), "no -0");
        assert!(close(snap(7.3, 0.0), 7.3));
        assert!(close(snap(7.3, f64::NAN), 7.3));
    }

    #[test]
    fn the_drag_step_is_the_finest_grid_share_wide_enough_on_screen() {
        // Grid 10 mm, 0.2 mm a pixel: a step must be at least 6 * 0.2 = 1.2 mm, so of
        // 1, 2, 5 and 10 it is 2; Shift makes it 0.2.
        assert!(close(drag_step(10.0, 0.2, false), 2.0));
        assert!(close(drag_step(10.0, 0.2, true), 0.2));
        // Zoomed in (0.01 mm a pixel): 1 mm, the tenth of the grid.
        assert!(close(drag_step(10.0, 0.01, false), 1.0));
        // Zoomed far out: never coarser than the grid.
        assert!(close(drag_step(10.0, 50.0, false), 10.0));
        // A bad grid falls back to 1 mm.
        assert!(close(drag_step(f64::NAN, 0.01, false), 0.1));
    }

    #[test]
    fn segment_polyline_and_polygon_tests() {
        assert!(close(
            distance_to_segment([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]),
            3.0
        ));
        // Past the end: to the end point, a 3-4-5 triangle.
        assert!(close(
            distance_to_segment([13.0, 4.0], [0.0, 0.0], [10.0, 0.0]),
            5.0
        ));
        assert!(close(
            distance_to_segment([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]),
            5.0
        ));
        let l = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert!(close(distance_to_polyline([12.0, 5.0], &l), 2.0));
        assert!(distance_to_polyline([1.0, 1.0], &[[0.0, 0.0]]).is_infinite());
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(inside_polygon([5.0, 5.0], &square));
        assert!(!inside_polygon([15.0, 5.0], &square));
        assert!(!inside_polygon([5.0, -1.0], &square));
        assert!(close(polygon_area(&square), 100.0));
    }

    #[test]
    fn the_triad_parts_are_hit_where_they_are_drawn() {
        // Top view, orthographic, triad of 90 pixels at the origin: X runs right and Y
        // up the screen from the centre; Z points at the viewer.
        let v = view(StandardView::Top, Projection::Orthographic, 250.0);
        let k = mm_per_px(250.0);
        let t = Triad {
            centre: Point3::ORIGIN,
            size: HANDLE_PIXELS * k,
        };
        let c = [W / 2.0, H / 2.0];
        let tol = GRAB_PIXELS;
        // On the X arrow, 80 pixels right; on the Y arrow, 80 pixels up.
        assert_eq!(
            t.hit(&v, [c[0] + 80.0, c[1] + 3.0], tol),
            Some(TriadPart::Axis(0))
        );
        assert_eq!(
            t.hit(&v, [c[0] - 2.0, c[1] - 80.0], tol),
            Some(TriadPart::Axis(1))
        );
        // Z is end-on: the centre is the end of X and Y, never Z.
        assert_ne!(t.hit(&v, c, tol), Some(TriadPart::Axis(2)));
        // The XY square spans 0.3 to 0.5 of 90 pixels (27 to 45) on both axes.
        assert_eq!(
            t.hit(&v, [c[0] + 36.0, c[1] - 36.0], tol),
            Some(TriadPart::Plane(2))
        );
        // The arc about Z has a radius of 0.8 * 90 = 72 pixels: at 45 degrees it is at
        // (50.9, 50.9), up and right.
        let r = 72.0 / 2f64.sqrt();
        assert_eq!(
            t.hit(&v, [c[0] + r, c[1] - r], tol),
            Some(TriadPart::Ring(2))
        );
        // Nothing far away, and nothing in the quadrant without handles.
        assert_eq!(t.hit(&v, [c[0] + 300.0, c[1] + 200.0], tol), None);
        assert_eq!(t.hit(&v, [c[0] - 40.0, c[1] + 40.0], tol), None);
    }

    #[test]
    fn an_arrow_is_hit_along_its_length_only() {
        let v = view(StandardView::Front, Projection::Orthographic, 250.0);
        let k = mm_per_px(250.0);
        let a = Arrow {
            base: Point3::ORIGIN,
            dir: Vec3::Z,
            length: 90.0 * k,
        };
        let c = [W / 2.0, H / 2.0];
        // Z is up on the front view's screen: 60 pixels up is on the arrow.
        assert!(a.hit(&v, [c[0] + 4.0, c[1] - 60.0], GRAB_PIXELS));
        assert!(!a.hit(&v, [c[0] + 20.0, c[1] - 60.0], GRAB_PIXELS));
        assert!(!a.hit(&v, [c[0], c[1] - 120.0], GRAB_PIXELS));
    }
}

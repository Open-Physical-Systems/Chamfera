//! Planar curves (rung K1): lines, segments, circles and arcs in the plane of a [`Frame`],
//! in its 2D coordinates. `on(frame)` places one in space as the matching 3D curve.
//!
//! Parameters: a [`Line2`]'s `t` is the signed distance from its origin; a [`Segment2`]'s
//! `t` runs over `[0, 1]`; a [`Circle2`]'s and an [`Arc2`]'s `t` is the angle from +x,
//! counter-clockwise (`c + r (cos t, sin t)`).

use crate::check;
use crate::curve3::{Arc3, Circle3, ClosestPoint, Line3, Segment3};
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Vec2};
use std::f64::consts::TAU;

/// An infinite line `origin + t direction`, `direction` a unit vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line2 {
    origin: Point2,
    direction: Vec2,
}

impl Line2 {
    pub fn new(origin: Point2, direction: Vec2) -> KernelResult<Line2> {
        Ok(Line2 {
            origin: check::point2("origin", origin)?,
            direction: check::direction2("direction", direction)?,
        })
    }
    pub fn origin(&self) -> Point2 {
        self.origin
    }
    pub fn direction(&self) -> Vec2 {
        self.direction
    }
    pub fn point_at(&self, t: f64) -> Point2 {
        self.origin + self.direction * t
    }
    pub fn closest_point(&self, p: Point2) -> KernelResult<ClosestPoint<Point2>> {
        check::point2("point", p)?;
        let t = (p - self.origin).dot(self.direction);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }
    pub fn reversed(&self) -> Line2 {
        Line2 {
            origin: self.origin,
            direction: -self.direction,
        }
    }
    pub fn on(&self, frame: &Frame) -> KernelResult<Line3> {
        Line3::new(
            frame.from_plane(self.origin),
            frame.vector_from_plane(self.direction),
        )
    }
}

/// A segment `start + t (end - start)`, `t` in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment2 {
    start: Point2,
    end: Point2,
}

impl Segment2 {
    /// A segment, or an error for invalid points or a length within [`LINEAR_TOLERANCE`].
    pub fn new(start: Point2, end: Point2) -> KernelResult<Segment2> {
        let start = check::point2("start", start)?;
        let end = check::point2("end", end)?;
        if start.distance(end) <= LINEAR_TOLERANCE {
            return Err(KernelError::InvalidInput(format!(
                "a segment's `start` and `end` must be more than {LINEAR_TOLERANCE} mm apart"
            )));
        }
        Ok(Segment2 { start, end })
    }
    pub fn start(&self) -> Point2 {
        self.start
    }
    pub fn end(&self) -> Point2 {
        self.end
    }
    pub fn point_at(&self, t: f64) -> Point2 {
        self.start.lerp(self.end, t)
    }
    /// The derivative `end - start`.
    pub fn d1(&self) -> Vec2 {
        self.end - self.start
    }
    pub fn length(&self) -> f64 {
        self.start.distance(self.end)
    }
    pub fn closest_point(&self, p: Point2) -> KernelResult<ClosestPoint<Point2>> {
        check::point2("point", p)?;
        let d = self.d1();
        let t = ((p - self.start).dot(d) / d.dot(d)).clamp(0.0, 1.0);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }
    pub fn reversed(&self) -> Segment2 {
        Segment2 {
            start: self.end,
            end: self.start,
        }
    }
    pub fn on(&self, frame: &Frame) -> KernelResult<Segment3> {
        Segment3::new(frame.from_plane(self.start), frame.from_plane(self.end))
    }
}

/// A full circle, `t` in `[0, 2π)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle2 {
    center: Point2,
    radius: f64,
}

impl Circle2 {
    pub fn new(center: Point2, radius: f64) -> KernelResult<Circle2> {
        Ok(Circle2 {
            center: check::point2("center", center)?,
            radius: check::length("radius", radius)?,
        })
    }
    pub fn center(&self) -> Point2 {
        self.center
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
    pub fn point_at(&self, t: f64) -> Point2 {
        let (s, c) = t.sin_cos();
        self.center + Vec2::new(c, s) * self.radius
    }
    /// The derivative `r (-sin t, cos t)`.
    pub fn d1(&self, t: f64) -> Vec2 {
        let (s, c) = t.sin_cos();
        Vec2::new(-s, c) * self.radius
    }
    pub fn length(&self) -> f64 {
        TAU * self.radius
    }
    /// The nearest point; not unique at the centre (`t = 0` is returned).
    pub fn closest_point(&self, p: Point2) -> KernelResult<ClosestPoint<Point2>> {
        check::point2("point", p)?;
        let v = p - self.center;
        let unique = v.length() > LINEAR_TOLERANCE;
        let t = if unique {
            v.angle().rem_euclid(TAU)
        } else {
            0.0
        };
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique,
        })
    }
    /// Counter-clockwise circle about the plane's normal.
    pub fn on(&self, frame: &Frame) -> KernelResult<Circle3> {
        let f = frame
            .with_origin(frame.from_plane(self.center))
            .ok_or_else(|| {
                KernelError::InvalidInput("the circle's centre lies outside the model range".into())
            })?;
        Circle3::new(f, self.radius)
    }
}

/// An arc from angle `start` through signed `sweep` (positive counter-clockwise,
/// negative clockwise), `0 < |sweep| <= 2π`. Its parameter is the angle, over
/// [`Arc2::range`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arc2 {
    center: Point2,
    radius: f64,
    start: f64,
    sweep: f64,
}

impl Arc2 {
    /// An arc; `start` is any finite angle (normalised into `[0, 2π)`); `sweep` must be
    /// finite, non-zero, at most a full turn, and give an arc longer than
    /// [`LINEAR_TOLERANCE`].
    pub fn new(center: Point2, radius: f64, start: f64, sweep: f64) -> KernelResult<Arc2> {
        let circle = Circle2::new(center, radius)?;
        Ok(Arc2 {
            center: circle.center,
            radius: circle.radius,
            start: check::start_angle("start", start)?,
            sweep: check::sweep("sweep", sweep, radius)?,
        })
    }
    /// The arc from `start` to `end` counter-clockwise about `center`; `end` must be at the
    /// same distance from `center` as `start` (within [`LINEAR_TOLERANCE`]).
    pub fn from_center_start_end(center: Point2, start: Point2, end: Point2) -> KernelResult<Arc2> {
        check::point2("center", center)?;
        check::point2("start", start)?;
        check::point2("end", end)?;
        let (r0, r1) = (center.distance(start), center.distance(end));
        if (r0 - r1).abs() > LINEAR_TOLERANCE {
            return Err(KernelError::InvalidInput(format!(
                "`start` and `end` must be equally far from `center` (within {LINEAR_TOLERANCE} mm)"
            )));
        }
        let a0 = (start - center).angle();
        let mut sweep = ((end - center).angle() - a0).rem_euclid(TAU);
        if start.coincident(end) {
            sweep = TAU;
        }
        Arc2::new(center, r0, a0, sweep)
    }
    pub fn center(&self) -> Point2 {
        self.center
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
    /// The start angle, in `[0, 2π)`.
    pub fn start_angle(&self) -> f64 {
        self.start
    }
    /// The signed sweep.
    pub fn sweep(&self) -> f64 {
        self.sweep
    }
    /// The angle range `(lo, hi)`, `lo < hi`, `hi - lo = |sweep|`: from the start for a
    /// counter-clockwise arc, from the end for a clockwise one.
    pub fn range(&self) -> (f64, f64) {
        let lo = self.start + self.sweep.min(0.0);
        (lo, lo + self.sweep.abs())
    }
    pub fn point_at(&self, t: f64) -> Point2 {
        let (s, c) = t.sin_cos();
        self.center + Vec2::new(c, s) * self.radius
    }
    /// The unit tangent at angle `t` in the arc's direction of travel.
    pub fn tangent(&self, t: f64) -> Vec2 {
        let (s, c) = t.sin_cos();
        Vec2::new(-s, c) * self.sweep.signum()
    }
    pub fn start_point(&self) -> Point2 {
        self.point_at(self.start)
    }
    pub fn end_point(&self) -> Point2 {
        self.point_at(self.start + self.sweep)
    }
    /// `r |sweep|`.
    pub fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
    }
    pub fn is_full_circle(&self) -> bool {
        self.sweep.abs() >= TAU
    }
    /// `theta` as an angle in [`Arc2::range`], or `None` when the point at `theta` is more
    /// than [`LINEAR_TOLERANCE`] (along the circle) outside the arc.
    pub fn parameter_of_angle(&self, theta: f64) -> Option<f64> {
        let (lo, hi) = self.range();
        check::angle_in_range(theta, lo, hi, LINEAR_TOLERANCE / self.radius)
    }
    pub fn closest_point(&self, p: Point2) -> KernelResult<ClosestPoint<Point2>> {
        check::point2("point", p)?;
        let (lo, hi) = self.range();
        let v = p - self.center;
        if v.length() <= LINEAR_TOLERANCE {
            let point = self.point_at(lo);
            return Ok(ClosestPoint {
                t: lo,
                point,
                distance: point.distance(p),
                unique: false,
            });
        }
        if let Some(t) = check::angle_in_range(v.angle(), lo, hi, 0.0) {
            let point = self.point_at(t);
            return Ok(ClosestPoint {
                t,
                point,
                distance: point.distance(p),
                unique: true,
            });
        }
        let (p0, p1) = (self.point_at(lo), self.point_at(hi));
        let (d0, d1) = (p0.distance(p), p1.distance(p));
        let unique = (d0 - d1).abs() > LINEAR_TOLERANCE;
        Ok(if d0 <= d1 {
            ClosestPoint {
                t: lo,
                point: p0,
                distance: d0,
                unique,
            }
        } else {
            ClosestPoint {
                t: hi,
                point: p1,
                distance: d1,
                unique,
            }
        })
    }
    /// The same arc traversed from its end to its start (the parameter range is unchanged).
    pub fn reversed(&self) -> Arc2 {
        Arc2 {
            start: (self.start + self.sweep).rem_euclid(TAU),
            sweep: -self.sweep,
            ..*self
        }
    }
    /// The arc in space, running the same way: a clockwise arc goes on the flipped frame.
    pub fn on(&self, frame: &Frame) -> KernelResult<Arc3> {
        let f = frame
            .with_origin(frame.from_plane(self.center))
            .ok_or_else(|| {
                KernelError::InvalidInput("the arc's centre lies outside the model range".into())
            })?;
        if self.sweep > 0.0 {
            Arc3::new(f, self.radius, self.start, self.sweep)
        } else {
            // On the flipped frame, angle u is plane angle -u.
            Arc3::new(f.flipped(), self.radius, -self.start, -self.sweep)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::{Point3, Vec3};
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn segments_and_lines() {
        let s = Segment2::new(Point2::new(1.0, 1.0), Point2::new(4.0, 5.0)).unwrap();
        assert_eq!(s.length(), 5.0);
        assert_eq!(s.point_at(0.5), Point2::new(2.5, 3.0));
        // (1,1) + 5 * (-4, 3)/5 is perpendicular to the segment at its start.
        let c = s.closest_point(Point2::new(-3.0, 4.0)).unwrap();
        assert_eq!((c.t, c.point), (0.0, s.start()));
        assert!((c.distance - 5.0).abs() < 1e-12);
        assert_eq!(s.reversed().point_at(1.0), s.start());
        let l = Line2::new(Point2::ORIGIN, Vec2::new(0.0, 3.0)).unwrap();
        let c = l.closest_point(Point2::new(2.0, -7.0)).unwrap();
        assert_eq!((c.t, c.distance), (-7.0, 2.0));
        let l3 = l.on(&Frame::XZ).unwrap();
        assert_eq!(l3.direction(), Vec3::Z);
    }

    #[test]
    fn arcs_measure_and_travel() {
        let a = Arc2::new(Point2::new(1.0, 0.0), 2.0, FRAC_PI_2, -PI).unwrap();
        // Clockwise half circle from the top (1, 2) to the bottom (1, -2), through (3, 0).
        assert!(a.start_point().distance(Point2::new(1.0, 2.0)) < 1e-15);
        assert!(a.end_point().distance(Point2::new(1.0, -2.0)) < 1e-15);
        assert!((a.length() - 2.0 * PI).abs() < 1e-15);
        let (lo, hi) = a.range();
        assert!((lo + FRAC_PI_2).abs() < 1e-15 && (hi - FRAC_PI_2).abs() < 1e-15);
        assert!(a.tangent(0.0).x.abs() < 1e-15 && a.tangent(0.0).y == -1.0);
        let r = a.reversed();
        assert!(r.start_point().distance(a.end_point()) < 1e-15);
        assert!(r.sweep() > 0.0);
        // Nearest to (5, 0) is (3, 0) at angle 0; (-5, 0) ties between both ends.
        let c = a.closest_point(Point2::new(5.0, 0.0)).unwrap();
        assert!(c.point.distance(Point2::new(3.0, 0.0)) < 1e-15 && c.unique);
        assert!(!a.closest_point(Point2::new(-5.0, 0.0)).unwrap().unique);
        let f = Arc2::from_center_start_end(
            Point2::ORIGIN,
            Point2::new(0.0, 3.0),
            Point2::new(3.0, 0.0),
        )
        .unwrap();
        // Counter-clockwise from 90 degrees to 0 degrees: three quarters of a turn.
        assert!((f.sweep() - 1.5 * PI).abs() < 1e-15);
    }

    #[test]
    fn planar_curves_land_in_the_frame() {
        let frame = Frame::from_normal(Point3::new(0.0, 0.0, 10.0), Vec3::Z).unwrap();
        let a = Arc2::new(Point2::new(1.0, 1.0), 2.0, 0.5, -1.25).unwrap();
        let a3 = a.on(&frame).unwrap();
        let start = frame.from_plane(a.start_point());
        let end = frame.from_plane(a.end_point());
        assert!(a3.start_point().distance(start) < 1e-12);
        assert!(a3.end_point().distance(end) < 1e-12);
        assert!((a3.length() - a.length()).abs() < 1e-15);
        let c3 = Circle2::new(Point2::new(3.0, 4.0), 1.0)
            .unwrap()
            .on(&Frame::YZ)
            .unwrap();
        assert_eq!(c3.center(), Point3::new(0.0, 3.0, 4.0));
        let s3 = Segment2::new(Point2::ORIGIN, Point2::new(1.0, 1.0))
            .unwrap()
            .on(&Frame::XZ)
            .unwrap();
        assert_eq!(s3.end(), Point3::new(1.0, 0.0, 1.0));
    }

    #[test]
    fn hostile_planar_input_is_an_error() {
        let nan = f64::NAN;
        assert!(Segment2::new(Point2::ORIGIN, Point2::ORIGIN).is_err());
        assert!(Segment2::new(Point2::ORIGIN, Point2::new(nan, 1.0)).is_err());
        assert!(Line2::new(Point2::ORIGIN, Vec2::ZERO).is_err());
        assert!(Circle2::new(Point2::ORIGIN, 0.0).is_err());
        assert!(Circle2::new(Point2::new(1e7, 0.0), 1.0).is_err());
        assert!(Arc2::new(Point2::ORIGIN, 1.0, 0.0, 0.0).is_err());
        assert!(Arc2::new(Point2::ORIGIN, 1.0, f64::INFINITY, 1.0).is_err());
        assert!(
            Arc2::from_center_start_end(
                Point2::ORIGIN,
                Point2::new(1.0, 0.0),
                Point2::new(0.0, 2.0)
            )
            .is_err()
        );
        assert!(
            Arc2::from_center_start_end(Point2::ORIGIN, Point2::ORIGIN, Point2::ORIGIN).is_err()
        );
        let c = Circle2::new(Point2::ORIGIN, 1.0).unwrap();
        assert!(c.closest_point(Point2::new(nan, 0.0)).is_err());
        assert!(!c.closest_point(Point2::ORIGIN).unwrap().unique);
    }
}

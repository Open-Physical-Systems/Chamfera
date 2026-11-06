//! Curves in space (rung K1): lines, segments, circles, circular arcs, ellipses and
//! elliptical arcs.
//!
//! Every curve is built by a validating constructor and is valid afterwards (finite,
//! in range, non-degenerate). Evaluation (`point_at`, `d1`, `d2`) takes any `t` and never
//! panics; a non-finite `t` gives a non-finite point. Queries that take outside input
//! (`closest_point`) validate it and return [`KernelResult`].
//!
//! Circles and ellipses live in a [`Frame`]: centre at its origin, in its x/y plane,
//! parameter `t` the angle from its x axis counter-clockwise about its z axis:
//! `c(t) = o + rx cos(t) x + ry sin(t) y`.

use crate::check;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::TAU;

/// The point of a curve nearest to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosestPoint<P> {
    /// The curve parameter of `point`.
    pub t: f64,
    pub point: P,
    pub distance: f64,
    /// False when other curve points are equally close (within [`LINEAR_TOLERANCE`]),
    /// e.g. the centre of a circle; `point` is then one of them.
    pub unique: bool,
}

/// Relative accuracy of a numeric elliptical-arc length. The adaptive Gauss-Legendre
/// quadrature refines until each panel's 5-point estimate and the sum of its halves'
/// estimates differ by less than this fraction of the total (shared out by width). For
/// the smooth (analytic) arc-length integrand, the halves' estimate is far more accurate
/// than the difference, so this bounds the error with a wide margin. A separate constant
/// because the quadrature's error is relative to the length, not a fixed distance.
pub const ELLIPSE_LENGTH_RELATIVE_TOLERANCE: f64 = 1e-12;
/// Most quadrature panels one elliptical-arc length may use (hostile-input limit).
const MAX_QUADRATURE_PANELS: usize = 100_000;
/// Deepest panel subdivision (a panel of 2π / 2^60 is far below any useful width).
const MAX_QUADRATURE_DEPTH: u32 = 60;
/// Samples per full turn when searching an elliptical arc for local minima of distance.
const ELLIPSE_ARC_SAMPLES: usize = 256;

/// An infinite line `origin + t direction`, with `direction` a unit vector: `t` is the
/// signed distance from `origin`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line3 {
    origin: Point3,
    direction: Vec3,
}

impl Line3 {
    pub fn new(origin: Point3, direction: Vec3) -> KernelResult<Line3> {
        Ok(Line3 {
            origin: check::point3("origin", origin)?,
            direction: check::direction3("direction", direction)?,
        })
    }
    pub fn origin(&self) -> Point3 {
        self.origin
    }
    pub fn direction(&self) -> Vec3 {
        self.direction
    }
    /// The parameter range: the whole real line.
    pub fn range(&self) -> (f64, f64) {
        (f64::NEG_INFINITY, f64::INFINITY)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        self.origin + self.direction * t
    }
    pub fn d1(&self, _t: f64) -> Vec3 {
        self.direction
    }
    pub fn d2(&self, _t: f64) -> Vec3 {
        Vec3::ZERO
    }
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let t = (p - self.origin).dot(self.direction);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }
    /// The same line run the other way: `t -> -t`.
    pub fn reversed(&self) -> Line3 {
        Line3 {
            origin: self.origin,
            direction: -self.direction,
        }
    }
}

/// A line segment `start + t (end - start)`, `t` in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment3 {
    start: Point3,
    end: Point3,
}

impl Segment3 {
    /// A segment, or an error for invalid points or a length within [`LINEAR_TOLERANCE`].
    pub fn new(start: Point3, end: Point3) -> KernelResult<Segment3> {
        let start = check::point3("start", start)?;
        let end = check::point3("end", end)?;
        if start.distance(end) <= LINEAR_TOLERANCE {
            return Err(KernelError::InvalidInput(format!(
                "a segment's `start` and `end` must be more than {LINEAR_TOLERANCE} mm apart"
            )));
        }
        Ok(Segment3 { start, end })
    }
    pub fn start(&self) -> Point3 {
        self.start
    }
    pub fn end(&self) -> Point3 {
        self.end
    }
    pub fn range(&self) -> (f64, f64) {
        (0.0, 1.0)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        self.start + (self.end - self.start) * t
    }
    pub fn d1(&self, _t: f64) -> Vec3 {
        self.end - self.start
    }
    pub fn d2(&self, _t: f64) -> Vec3 {
        Vec3::ZERO
    }
    pub fn length(&self) -> f64 {
        self.start.distance(self.end)
    }
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let d = self.end - self.start;
        let t = ((p - self.start).dot(d) / d.dot(d)).clamp(0.0, 1.0);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }
    pub fn bounding_box(&self) -> Aabb {
        let mut b = Aabb {
            min: self.start,
            max: self.start,
        };
        b.include(self.end);
        b
    }
    pub fn reversed(&self) -> Segment3 {
        Segment3 {
            start: self.end,
            end: self.start,
        }
    }
}

/// A full circle of `radius` about its frame's origin, in the frame's x/y plane;
/// `t` in `[0, 2π)` is the angle from the frame's x axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle3 {
    frame: Frame,
    radius: f64,
}

impl Circle3 {
    pub fn new(frame: Frame, radius: f64) -> KernelResult<Circle3> {
        Ok(Circle3 {
            frame,
            radius: check::length("radius", radius)?,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn center(&self) -> Point3 {
        self.frame.origin()
    }
    pub fn normal(&self) -> Vec3 {
        self.frame.z()
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
    pub fn range(&self) -> (f64, f64) {
        (0.0, TAU)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        conic_point(&self.frame, self.radius, self.radius, t)
    }
    pub fn d1(&self, t: f64) -> Vec3 {
        conic_d1(&self.frame, self.radius, self.radius, t)
    }
    pub fn d2(&self, t: f64) -> Vec3 {
        conic_d2(&self.frame, self.radius, self.radius, t)
    }
    /// `2π r`.
    pub fn length(&self) -> f64 {
        TAU * self.radius
    }
    /// The nearest point; not unique for points on the axis (`t = 0` is returned).
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let (t, unique) = circle_angle(&self.frame, p);
        let t = t.rem_euclid(TAU);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique,
        })
    }
    pub fn bounding_box(&self) -> Aabb {
        conic_box(&self.frame, self.radius, self.radius, None)
    }
    /// The same circle run the other way (frame flipped): `c'(t) = c(-t)`.
    pub fn reversed(&self) -> Circle3 {
        Circle3 {
            frame: self.frame.flipped(),
            radius: self.radius,
        }
    }
}

/// An arc of a [`Circle3`]: `t` from `start` to `start + sweep`, `0 < sweep <= 2π`,
/// counter-clockwise about the frame's z axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arc3 {
    circle: Circle3,
    start: f64,
    sweep: f64,
}

impl Arc3 {
    /// An arc; `start` is any finite angle (normalised into `[0, 2π)`), `sweep` must be
    /// positive, at most 2π, and give an arc longer than [`LINEAR_TOLERANCE`].
    pub fn new(frame: Frame, radius: f64, start: f64, sweep: f64) -> KernelResult<Arc3> {
        let circle = Circle3::new(frame, radius)?;
        let start = check::start_angle("start", start)?;
        let sweep = check::sweep("sweep", sweep, radius)?;
        if sweep < 0.0 {
            return Err(KernelError::InvalidInput(
                "`sweep` of a 3D arc must be positive; flip the frame for a clockwise arc".into(),
            ));
        }
        Ok(Arc3 {
            circle,
            start,
            sweep,
        })
    }
    pub fn circle(&self) -> &Circle3 {
        &self.circle
    }
    pub fn start_angle(&self) -> f64 {
        self.start
    }
    pub fn sweep(&self) -> f64 {
        self.sweep
    }
    pub fn range(&self) -> (f64, f64) {
        (self.start, self.start + self.sweep)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        self.circle.point_at(t)
    }
    pub fn d1(&self, t: f64) -> Vec3 {
        self.circle.d1(t)
    }
    pub fn d2(&self, t: f64) -> Vec3 {
        self.circle.d2(t)
    }
    pub fn start_point(&self) -> Point3 {
        self.point_at(self.start)
    }
    pub fn end_point(&self) -> Point3 {
        self.point_at(self.start + self.sweep)
    }
    /// `r * sweep`.
    pub fn length(&self) -> f64 {
        self.circle.radius * self.sweep
    }
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let (lo, hi) = self.range();
        let (theta, defined) = circle_angle(&self.circle.frame, p);
        if !defined {
            // Every point of the arc is equally far.
            let point = self.point_at(lo);
            return Ok(ClosestPoint {
                t: lo,
                point,
                distance: point.distance(p),
                unique: false,
            });
        }
        if let Some(t) = check::angle_in_range(theta, lo, hi, 0.0) {
            let point = self.point_at(t);
            return Ok(ClosestPoint {
                t,
                point,
                distance: point.distance(p),
                unique: true,
            });
        }
        // Outside the arc, distance grows with the angle from `theta`: an end is nearest.
        Ok(nearer_end(p, lo, self.point_at(lo), hi, self.point_at(hi)))
    }
    pub fn bounding_box(&self) -> Aabb {
        let r = self.circle.radius;
        conic_box(&self.circle.frame, r, r, Some(self.range()))
    }
    /// The same arc run from its end to its start.
    pub fn reversed(&self) -> Arc3 {
        // On the flipped circle, c'(u) = c(-u); u in [-(s + w), -s] covers the same points.
        Arc3 {
            circle: self.circle.reversed(),
            start: (-(self.start + self.sweep)).rem_euclid(TAU),
            sweep: self.sweep,
        }
    }
}

/// An ellipse with semi-axes `rx` along its frame's x axis and `ry` along its y axis:
/// `c(t) = o + rx cos(t) x + ry sin(t) y`, `t` in `[0, 2π)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipse3 {
    frame: Frame,
    rx: f64,
    ry: f64,
}

impl Ellipse3 {
    pub fn new(frame: Frame, rx: f64, ry: f64) -> KernelResult<Ellipse3> {
        Ok(Ellipse3 {
            frame,
            rx: check::length("rx", rx)?,
            ry: check::length("ry", ry)?,
        })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn center(&self) -> Point3 {
        self.frame.origin()
    }
    pub fn rx(&self) -> f64 {
        self.rx
    }
    pub fn ry(&self) -> f64 {
        self.ry
    }
    pub fn range(&self) -> (f64, f64) {
        (0.0, TAU)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        conic_point(&self.frame, self.rx, self.ry, t)
    }
    pub fn d1(&self, t: f64) -> Vec3 {
        conic_d1(&self.frame, self.rx, self.ry, t)
    }
    pub fn d2(&self, t: f64) -> Vec3 {
        conic_d2(&self.frame, self.rx, self.ry, t)
    }
    /// The circumference by the arithmetic-geometric mean (Gauss; see Almkvist and
    /// Berndt, "Gauss, Landen, Ramanujan, the arithmetic-geometric mean, ellipses, π, and
    /// the Ladies Diary", Amer. Math. Monthly 95 (1988)):
    /// `C = 2π / M(a, b) * (a^2 - Σ_{n>=0} 2^(n-1) c_n^2)`, with `a_0 = a`, `b_0 = b`,
    /// `c_0^2 = a^2 - b^2`, `a_{n+1} = (a_n + b_n)/2`, `b_{n+1} = sqrt(a_n b_n)`,
    /// `c_{n+1} = (a_n - b_n)/2`. Convergence is quadratic; the loop stops when `c_n` is
    /// below one ulp of `a_n`, so the relative error is a few ulps (about 1e-15).
    pub fn length(&self) -> f64 {
        let (a, b) = (self.rx.max(self.ry), self.rx.min(self.ry));
        let (mut an, mut bn) = (a, b);
        let mut sum = 0.5 * (a - b) * (a + b);
        let mut weight = 0.5;
        for _ in 0..64 {
            let cn = 0.5 * (an - bn);
            let next_a = 0.5 * (an + bn);
            bn = (an * bn).sqrt();
            an = next_a;
            weight *= 2.0;
            sum += weight * cn * cn;
            if cn.abs() <= f64::EPSILON * an {
                break;
            }
        }
        TAU / an * (a * a - sum)
    }
    /// The nearest point. Not unique at the centre or on the major axis close to the
    /// centre (inside the evolute), where two points are equally near.
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let local = self.frame.to_local(p);
        let (t, unique) = ellipse_closest_angle(self.rx, self.ry, local.x, local.y);
        let t = t.rem_euclid(TAU);
        let point = self.point_at(t);
        Ok(ClosestPoint {
            t,
            point,
            distance: point.distance(p),
            unique,
        })
    }
    pub fn bounding_box(&self) -> Aabb {
        conic_box(&self.frame, self.rx, self.ry, None)
    }
    /// The same ellipse run the other way (frame flipped): `c'(t) = c(-t)`.
    pub fn reversed(&self) -> Ellipse3 {
        Ellipse3 {
            frame: self.frame.flipped(),
            rx: self.rx,
            ry: self.ry,
        }
    }
}

/// An arc of an [`Ellipse3`]: `t` from `start` to `start + sweep`, `0 < sweep <= 2π`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EllipticalArc3 {
    ellipse: Ellipse3,
    start: f64,
    sweep: f64,
}

impl EllipticalArc3 {
    pub fn new(frame: Frame, rx: f64, ry: f64, start: f64, sweep: f64) -> KernelResult<Self> {
        let ellipse = Ellipse3::new(frame, rx, ry)?;
        let start = check::start_angle("start", start)?;
        let sweep = check::sweep("sweep", sweep, rx.min(ry))?;
        if sweep < 0.0 {
            return Err(KernelError::InvalidInput(
                "`sweep` of a 3D elliptical arc must be positive; flip the frame for a clockwise arc"
                    .into(),
            ));
        }
        Ok(EllipticalArc3 {
            ellipse,
            start,
            sweep,
        })
    }
    pub fn ellipse(&self) -> &Ellipse3 {
        &self.ellipse
    }
    pub fn range(&self) -> (f64, f64) {
        (self.start, self.start + self.sweep)
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        self.ellipse.point_at(t)
    }
    pub fn d1(&self, t: f64) -> Vec3 {
        self.ellipse.d1(t)
    }
    pub fn d2(&self, t: f64) -> Vec3 {
        self.ellipse.d2(t)
    }
    pub fn start_point(&self) -> Point3 {
        self.point_at(self.start)
    }
    pub fn end_point(&self) -> Point3 {
        self.point_at(self.start + self.sweep)
    }
    /// The arc length `∫ sqrt(rx^2 sin^2 t + ry^2 cos^2 t) dt` over the range, by
    /// adaptive Gauss-Legendre quadrature to [`ELLIPSE_LENGTH_RELATIVE_TOLERANCE`].
    /// A sweep of a full turn uses the exact AGM circumference instead.
    pub fn length(&self) -> KernelResult<f64> {
        if self.sweep >= TAU {
            return Ok(self.ellipse.length());
        }
        let (lo, hi) = self.range();
        elliptic_arc_length(self.ellipse.rx, self.ellipse.ry, lo, hi)
    }
    /// The nearest point: the best of the arc's ends, the whole ellipse's nearest point
    /// (when it lies on the arc) and every local minimum of distance found by sampling
    /// the arc ([`ELLIPSE_ARC_SAMPLES`] per turn) and bisecting sign changes of the
    /// distance derivative.
    pub fn closest_point(&self, p: Point3) -> KernelResult<ClosestPoint<Point3>> {
        check::point3("point", p)?;
        let (lo, hi) = self.range();
        let e = &self.ellipse;
        let local = e.frame.to_local(p);
        let mut candidates = vec![lo, hi];
        let (t_full, _) = ellipse_closest_angle(e.rx, e.ry, local.x, local.y);
        if let Some(t) = check::angle_in_range(t_full, lo, hi, 0.0) {
            candidates.push(t);
        }
        // g(t) = (c(t) - q) . c'(t) is half the derivative of the squared distance;
        // a minimum is where g goes from negative to positive.
        let g = |t: f64| {
            let (s, c) = t.sin_cos();
            let (px, py) = (e.rx * c - local.x, e.ry * s - local.y);
            -px * e.rx * s + py * e.ry * c
        };
        let samples = ((self.sweep / TAU) * ELLIPSE_ARC_SAMPLES as f64)
            .ceil()
            .max(4.0) as usize;
        let step = self.sweep / samples as f64;
        for i in 0..samples {
            let (mut a, mut b) = (lo + step * i as f64, lo + step * (i + 1) as f64);
            if !(g(a) < 0.0 && g(b) >= 0.0) {
                continue;
            }
            for _ in 0..200 {
                let m = 0.5 * (a + b);
                if m <= a || m >= b {
                    break;
                }
                if g(m) < 0.0 {
                    a = m;
                } else {
                    b = m;
                }
            }
            candidates.push(0.5 * (a + b));
        }
        let mut best: Option<ClosestPoint<Point3>> = None;
        for t in candidates {
            let point = self.point_at(t);
            let distance = point.distance(p);
            best = Some(match best {
                None => ClosestPoint {
                    t,
                    point,
                    distance,
                    unique: true,
                },
                Some(b) if distance < b.distance - LINEAR_TOLERANCE => ClosestPoint {
                    t,
                    point,
                    distance,
                    unique: true,
                },
                Some(mut b) => {
                    if (distance - b.distance).abs() <= LINEAR_TOLERANCE
                        && point.distance(b.point) > LINEAR_TOLERANCE
                    {
                        b.unique = false;
                    }
                    if distance < b.distance {
                        b.t = t;
                        b.point = point;
                        b.distance = distance;
                    }
                    b
                }
            });
        }
        best.ok_or_else(|| KernelError::InvalidResult("no closest point candidate".into()))
    }
    pub fn bounding_box(&self) -> Aabb {
        conic_box(
            &self.ellipse.frame,
            self.ellipse.rx,
            self.ellipse.ry,
            Some(self.range()),
        )
    }
    pub fn reversed(&self) -> EllipticalArc3 {
        EllipticalArc3 {
            ellipse: self.ellipse.reversed(),
            start: (-(self.start + self.sweep)).rem_euclid(TAU),
            sweep: self.sweep,
        }
    }
}

fn conic_point(f: &Frame, rx: f64, ry: f64, t: f64) -> Point3 {
    let (s, c) = t.sin_cos();
    f.origin() + f.x() * (rx * c) + f.y() * (ry * s)
}

fn conic_d1(f: &Frame, rx: f64, ry: f64, t: f64) -> Vec3 {
    let (s, c) = t.sin_cos();
    f.x() * (-rx * s) + f.y() * (ry * c)
}

fn conic_d2(f: &Frame, rx: f64, ry: f64, t: f64) -> Vec3 {
    let (s, c) = t.sin_cos();
    f.x() * (-rx * c) + f.y() * (-ry * s)
}

/// The angle of `p`'s projection onto the frame's plane, and whether it is defined
/// (false when `p` is within [`LINEAR_TOLERANCE`] of the axis).
fn circle_angle(f: &Frame, p: Point3) -> (f64, bool) {
    let l = f.to_local(p);
    if l.x.hypot(l.y) <= LINEAR_TOLERANCE {
        (0.0, false)
    } else {
        (l.y.atan2(l.x), true)
    }
}

fn nearer_end(p: Point3, t0: f64, p0: Point3, t1: f64, p1: Point3) -> ClosestPoint<Point3> {
    let (d0, d1) = (p0.distance(p), p1.distance(p));
    let unique = (d0 - d1).abs() > LINEAR_TOLERANCE || p0.coincident(p1);
    if d0 <= d1 {
        ClosestPoint {
            t: t0,
            point: p0,
            distance: d0,
            unique,
        }
    } else {
        ClosestPoint {
            t: t1,
            point: p1,
            distance: d1,
            unique,
        }
    }
}

/// The box of `o + rx cos(t) x + ry sin(t) y` over `range` (the whole turn for `None`).
/// Coordinate i is `o_i + rx x_i cos t + ry y_i sin t = o_i + A cos(t - φ)` with
/// `A = hypot(rx x_i, ry y_i)` and `φ = atan2(ry y_i, rx x_i)`: largest at `t = φ`,
/// smallest at `t = φ + π`.
fn conic_box(f: &Frame, rx: f64, ry: f64, range: Option<(f64, f64)>) -> Aabb {
    let o = f.origin();
    let comps = |v: Vec3| [v.x, v.y, v.z];
    let (x, y) = (comps(f.x()), comps(f.y()));
    match range {
        None => {
            let h: Vec<f64> = (0..3).map(|i| (rx * x[i]).hypot(ry * y[i])).collect();
            let half = Vec3::new(h[0], h[1], h[2]);
            Aabb {
                min: o + (-half),
                max: o + half,
            }
        }
        Some((lo, hi)) => {
            let p0 = conic_point(f, rx, ry, lo);
            let mut b = Aabb { min: p0, max: p0 };
            b.include(conic_point(f, rx, ry, hi));
            for i in 0..3 {
                let (u, v) = (rx * x[i], ry * y[i]);
                if u == 0.0 && v == 0.0 {
                    continue;
                }
                let phi = v.atan2(u);
                for t in [phi, phi + std::f64::consts::PI] {
                    if let Some(t) = check::angle_in_range(t, lo, hi, 0.0) {
                        b.include(conic_point(f, rx, ry, t));
                    }
                }
            }
            b
        }
    }
}

/// The angle of the point of the ellipse `(rx cos t, ry sin t)` nearest to `(qx, qy)`,
/// and whether it is the only nearest point.
///
/// Derivation (Lagrange multipliers): reflect into the first quadrant and order the
/// semi-axes `e0 >= e1`. For `y = (y0, y1)` with `y0, y1 > 0` the nearest point is
/// `x_i = e_i^2 y_i / (λ + e_i^2)` where λ solves `Σ (e_i y_i / (λ + e_i^2))^2 = 1`.
/// Substituting `λ = e1^2 s`, `r0 = (e0/e1)^2`, `z_i = y_i / e_i`, the root is that of
/// `G(s) = (r0 z0 / (s + r0))^2 + (z1 / (s + 1))^2 - 1`, strictly decreasing for
/// `s > -1`, with `G(z1 - 1) >= 0` and `G(0) = z0^2 + z1^2 - 1` (inside) or
/// `G(|(r0 z0, z1)| - 1) <= 0` (outside); bisection finds it to full precision.
/// On the axes: for `y0 = 0` the nearest point is the minor vertex `(0, e1)`; for
/// `y1 = 0` it is `x0 = e0^2 y0 / (e0^2 - e1^2)`, `x1 = e1 sqrt(1 - (x0/e0)^2)` (two
/// mirror points) while `e0 y0 < e0^2 - e1^2`, else the major vertex `(e0, 0)`.
/// (The same construction appears in D. Eberly, "Distance from a Point to an Ellipse,
/// an Ellipsoid, or a Hyperellipsoid", Geometric Tools technical note.)
fn ellipse_closest_angle(rx: f64, ry: f64, qx: f64, qy: f64) -> (f64, bool) {
    let swapped = rx < ry;
    let (e0, e1, y0, y1) = if swapped {
        (ry, rx, qy.abs(), qx.abs())
    } else {
        (rx, ry, qx.abs(), qy.abs())
    };
    let (x0, x1, unique) = if y1 > 0.0 {
        if y0 > 0.0 {
            let (z0, z1) = (y0 / e0, y1 / e1);
            let g = z0 * z0 + z1 * z1 - 1.0;
            if g == 0.0 {
                (y0, y1, true)
            } else {
                let r0 = (e0 / e1) * (e0 / e1);
                let big_g = |s: f64| {
                    let a = r0 * z0 / (s + r0);
                    let b = z1 / (s + 1.0);
                    a * a + b * b - 1.0
                };
                let mut lo = z1 - 1.0;
                let mut hi = if g < 0.0 {
                    0.0
                } else {
                    (r0 * z0).hypot(z1) - 1.0
                };
                for _ in 0..1100 {
                    let m = 0.5 * (lo + hi);
                    if m <= lo || m >= hi {
                        break;
                    }
                    let v = big_g(m);
                    if v > 0.0 {
                        lo = m;
                    } else if v < 0.0 {
                        hi = m;
                    } else {
                        lo = m;
                        hi = m;
                    }
                }
                let s = 0.5 * (lo + hi);
                (r0 * y0 / (s + r0), y1 / (s + 1.0), true)
            }
        } else {
            (0.0, e1, true)
        }
    } else {
        let numer = e0 * y0;
        let denom = (e0 - e1) * (e0 + e1);
        if numer < denom {
            let xd = numer / denom;
            (e0 * xd, e1 * (1.0 - xd * xd).max(0.0).sqrt(), false)
        } else {
            // At the centre of a circle (e0 = e1) every point is equally near.
            (e0, 0.0, y0 > LINEAR_TOLERANCE)
        }
    };
    // Points within tolerance of the major axis inside the evolute also have a mirror.
    let unique = unique && !(y1 <= LINEAR_TOLERANCE && e0 * y0 < (e0 - e1) * (e0 + e1));
    let unique = unique && !(y0 <= LINEAR_TOLERANCE && y1 <= LINEAR_TOLERANCE);
    // Undo the reflection and the swap.
    let (lx, ly) = if swapped { (x1, x0) } else { (x0, x1) };
    let lx = lx.copysign(if qx == 0.0 { 1.0 } else { qx });
    let ly = ly.copysign(if qy == 0.0 { 1.0 } else { qy });
    ((ly / ry).atan2(lx / rx), unique)
}

/// 5-point Gauss-Legendre nodes and weights on [-1, 1] (Abramowitz and Stegun, table 25.4).
const GL5: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

fn elliptic_arc_length(rx: f64, ry: f64, lo: f64, hi: f64) -> KernelResult<f64> {
    let speed = |t: f64| (rx * t.sin()).hypot(ry * t.cos());
    let gl = |a: f64, b: f64| {
        let (m, h) = (0.5 * (a + b), 0.5 * (b - a));
        h * GL5.iter().map(|(x, w)| w * speed(m + h * x)).sum::<f64>()
    };
    let width = hi - lo;
    // A first estimate sets the absolute target.
    let estimate: f64 = (0..16)
        .map(|i| {
            gl(
                lo + width * i as f64 / 16.0,
                lo + width * (i + 1) as f64 / 16.0,
            )
        })
        .sum();
    let tol = ELLIPSE_LENGTH_RELATIVE_TOLERANCE * estimate;
    let mut stack = vec![(lo, hi, gl(lo, hi), 0_u32)];
    let mut total = 0.0;
    let mut panels = 0_usize;
    while let Some((a, b, whole, depth)) = stack.pop() {
        panels += 1;
        if panels > MAX_QUADRATURE_PANELS {
            return Err(KernelError::NoConvergence(format!(
                "elliptical arc length needs more than {MAX_QUADRATURE_PANELS} panels"
            )));
        }
        let m = 0.5 * (a + b);
        let (left, right) = (gl(a, m), gl(m, b));
        if (left + right - whole).abs() <= tol * (b - a) / width {
            total += left + right;
        } else if depth >= MAX_QUADRATURE_DEPTH {
            return Err(KernelError::NoConvergence(
                "elliptical arc length did not reach its tolerance".into(),
            ));
        } else {
            stack.push((a, m, left, depth + 1));
            stack.push((m, b, right, depth + 1));
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn tilted() -> Frame {
        Frame::from_normal_and_x(
            Point3::new(1.0, -2.0, 3.0),
            Vec3::new(1.0, 2.0, 2.0),
            Vec3::new(2.0, -1.0, 0.0),
        )
        .unwrap()
    }

    fn close(a: Point3, b: Point3, tol: f64) -> bool {
        a.distance(b) <= tol
    }

    /// Gauss-Kummer series: C = π (a + b) Σ binom(1/2, n)^2 h^n, h = ((a-b)/(a+b))^2.
    fn gauss_kummer(a: f64, b: f64) -> f64 {
        let h = ((a - b) / (a + b)).powi(2);
        let mut sum = 0.0;
        let mut coeff = 1.0; // binom(1/2, n)
        let mut hn = 1.0;
        for n in 0..60 {
            sum += coeff * coeff * hn;
            coeff *= (0.5 - n as f64) / (n as f64 + 1.0);
            hn *= h;
        }
        PI * (a + b) * sum
    }

    #[test]
    fn lines_and_segments_evaluate_and_project() {
        let l = Line3::new(Point3::new(1.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 2.0)).unwrap();
        assert_eq!(l.point_at(3.0), Point3::new(1.0, 1.0, 4.0));
        let c = l.closest_point(Point3::new(4.0, 5.0, -2.0)).unwrap();
        // Foot of the perpendicular is (1, 1, -2), at t = -3, distance hypot(3, 4) = 5.
        assert_eq!(c.t, -3.0);
        assert!((c.distance - 5.0).abs() < 1e-12);
        assert_eq!(l.reversed().point_at(3.0), Point3::new(1.0, 1.0, -2.0));

        let s = Segment3::new(Point3::ORIGIN, Point3::new(3.0, 4.0, 0.0)).unwrap();
        assert_eq!(s.length(), 5.0);
        assert_eq!(s.d1(0.2), Vec3::new(3.0, 4.0, 0.0));
        // Beyond the end the end point is nearest.
        let c = s.closest_point(Point3::new(6.0, 8.0, 0.0)).unwrap();
        assert_eq!((c.t, c.point), (1.0, Point3::new(3.0, 4.0, 0.0)));
        // (3, 4) . (-4, 3) = 0: (-4, 3, 0) + (1.5, 2, 0) projects to t = 0.5.
        let c = s.closest_point(Point3::new(-2.5, 5.0, 0.0)).unwrap();
        assert!((c.t - 0.5).abs() < 1e-15 && (c.distance - 5.0).abs() < 1e-12);
        let b = s.bounding_box();
        assert_eq!((b.min, b.max), (Point3::ORIGIN, Point3::new(3.0, 4.0, 0.0)));
        assert_eq!(s.reversed().point_at(0.0), s.end());
    }

    #[test]
    fn circle_evaluation_derivatives_and_length() {
        let c = Circle3::new(Frame::XY, 2.0).unwrap();
        assert!(close(
            c.point_at(FRAC_PI_2),
            Point3::new(0.0, 2.0, 0.0),
            1e-15
        ));
        assert!((c.d1(0.0) - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-15);
        // c'' = -(c - centre) for a circle.
        assert!((c.d2(0.7) + (c.point_at(0.7) - Point3::ORIGIN)).length() < 1e-15);
        assert_eq!(c.length(), 4.0 * PI);
        let t = tilted();
        let c = Circle3::new(t, 5.0).unwrap();
        for k in 0..8 {
            let u = k as f64 * 0.8;
            assert!((c.point_at(u).distance(t.origin()) - 5.0).abs() < 1e-12);
            assert!(c.d1(u).dot(t.z()).abs() < 1e-12);
            // The derivative is perpendicular to the radius and of length r.
            assert!(c.d1(u).dot(c.point_at(u) - t.origin()).abs() < 1e-11);
            assert!((c.d1(u).length() - 5.0).abs() < 1e-12);
        }
    }

    #[test]
    fn circle_closest_point_and_box() {
        let c = Circle3::new(
            Frame::XY.with_origin(Point3::new(1.0, 1.0, 0.0)).unwrap(),
            5.0,
        )
        .unwrap();
        // (1,1,0) + (3,4,12): projection direction (3,4)/5, nearest (4, 5, 0), distance
        // hypot(5 - 5, 12) = 12.
        let q = c.closest_point(Point3::new(4.0, 5.0, 12.0)).unwrap();
        assert!(close(q.point, Point3::new(4.0, 5.0, 0.0), 1e-12));
        assert!((q.distance - 12.0).abs() < 1e-12 && q.unique);
        assert!((q.t - 4f64.atan2(3.0)).abs() < 1e-15);
        let on_axis = c.closest_point(Point3::new(1.0, 1.0, 7.0)).unwrap();
        assert!(!on_axis.unique);
        assert!((on_axis.distance - 74f64.sqrt()).abs() < 1e-12);
        let b = c.bounding_box();
        assert!(close(b.min, Point3::new(-4.0, -4.0, 0.0), 1e-12));
        assert!(close(b.max, Point3::new(6.0, 6.0, 0.0), 1e-12));
        // A circle in the XZ plane is flat in y; its box is r * sqrt(1 - n_i^2) wide.
        let b = Circle3::new(Frame::XZ, 2.0).unwrap().bounding_box();
        assert!(close(b.min, Point3::new(-2.0, 0.0, -2.0), 1e-15));
        // Tilted normal (1,2,2)/3: half extents 5 sqrt(1 - n_i^2) = 5 sqrt(8/9), 5 sqrt(5/9).
        let c = Circle3::new(tilted(), 5.0).unwrap();
        let s = c.bounding_box().size();
        assert!((s.x - 10.0 * (8f64 / 9.0).sqrt()).abs() < 1e-12);
        assert!((s.y - 10.0 * (5f64 / 9.0).sqrt()).abs() < 1e-12);
        assert!((s.z - 10.0 * (5f64 / 9.0).sqrt()).abs() < 1e-12);
    }

    #[test]
    fn reversed_curves_run_backwards_over_the_same_points() {
        let c = Circle3::new(tilted(), 3.0).unwrap();
        let r = c.reversed();
        for k in 0..6 {
            let u = k as f64;
            assert!(close(r.point_at(u), c.point_at(-u), 1e-12));
        }
        let a = Arc3::new(tilted(), 3.0, 0.5, 2.0).unwrap();
        let ar = a.reversed();
        assert!(close(ar.start_point(), a.end_point(), 1e-12));
        assert!(close(ar.end_point(), a.start_point(), 1e-12));
        assert!((ar.length() - a.length()).abs() < 1e-15);
        let e = EllipticalArc3::new(tilted(), 5.0, 3.0, 1.0, 1.5).unwrap();
        let er = e.reversed();
        assert!(close(er.start_point(), e.end_point(), 1e-12));
        assert!(close(er.end_point(), e.start_point(), 1e-12));
        assert!((er.length().unwrap() - e.length().unwrap()).abs() < 1e-12);
        let el = Ellipse3::new(tilted(), 5.0, 3.0).unwrap();
        assert!(close(el.reversed().point_at(0.4), el.point_at(-0.4), 1e-12));
    }

    #[test]
    fn arc_length_closest_point_and_box() {
        // Quarter arc of r = 4 from 0 to π/2: length 2π.
        let a = Arc3::new(Frame::XY, 4.0, 0.0, FRAC_PI_2).unwrap();
        assert!((a.length() - 2.0 * PI).abs() < 1e-15);
        // A point at angle π/4 projects inside the arc.
        let q = a.closest_point(Point3::new(10.0, 10.0, 0.0)).unwrap();
        assert!(close(
            q.point,
            Point3::new(8f64.sqrt(), 8f64.sqrt(), 0.0),
            1e-12
        ));
        // A point at angle -π/3 is nearest to the start (4, 0, 0).
        let q = a
            .closest_point(Point3::new(1.0, -3f64.sqrt(), 0.0))
            .unwrap();
        assert!(close(q.point, Point3::new(4.0, 0.0, 0.0), 1e-12) && q.unique);
        // Opposite the arc's middle (angle 5π/4) both ends are equally near.
        let q = a.closest_point(Point3::new(-1.0, -1.0, 0.0)).unwrap();
        assert!(!q.unique);
        // Box of the quarter arc: [0, 4] x [0, 4].
        let b = a.bounding_box();
        assert!(
            close(b.min, Point3::ORIGIN, 1e-12) && close(b.max, Point3::new(4.0, 4.0, 0.0), 1e-12)
        );
        // An arc from π/4 to 3π/4 rises to the top of the circle, y = 4.
        let b = Arc3::new(Frame::XY, 4.0, PI / 4.0, FRAC_PI_2)
            .unwrap()
            .bounding_box();
        assert!((b.max.y - 4.0).abs() < 1e-12 && (b.min.y - 8f64.sqrt()).abs() < 1e-12);
        assert!((b.max.x - 8f64.sqrt()).abs() < 1e-12 && (b.min.x + 8f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn ellipse_circumference_matches_the_series_and_ramanujan() {
        // a = 5, b = 3: h = (2/8)^2 = 1/16.
        let e = Ellipse3::new(Frame::XY, 5.0, 3.0).unwrap();
        let c = e.length();
        let exact = gauss_kummer(5.0, 3.0);
        assert!((c - exact).abs() / exact < 1e-14);
        // Ramanujan's second approximation, R = π (a+b) (1 + 3h / (10 + sqrt(4 - 3h))),
        // always underestimates; the error is (3/2^17) h^5 π (a+b) to leading order (the
        // series coefficients of the error fall off, so 2.75e-5 h^5 π (a+b) bounds it for
        // h <= 1/16).
        let h: f64 = 1.0 / 16.0;
        let r = PI * 8.0 * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
        assert!(c > r);
        assert!(c - r < 2.75e-5 * h.powi(5) * PI * 8.0);
        assert!(c - r > 3.0 / 131072.0 * h.powi(5) * PI * 8.0);
        // A circle is the special case 2π r, and the axes may come in either order.
        assert!((Ellipse3::new(Frame::XY, 2.0, 2.0).unwrap().length() - 4.0 * PI).abs() < 1e-14);
        assert!((Ellipse3::new(Frame::XY, 3.0, 5.0).unwrap().length() - exact).abs() < 1e-13);
        // A very flat ellipse tends to 4a: a = 1, b = 1e-6 gives 4 + O(b^2 log b).
        let flat = Ellipse3::new(Frame::XY, 1.0, 1e-6).unwrap().length();
        assert!((flat - 4.0).abs() < 1e-9);
    }

    #[test]
    fn elliptical_arc_lengths_add_up() {
        let full = Ellipse3::new(Frame::XY, 5.0, 3.0).unwrap().length();
        // By symmetry each quadrant is a quarter of the circumference.
        let q = EllipticalArc3::new(Frame::XY, 5.0, 3.0, 0.0, FRAC_PI_2).unwrap();
        assert!((q.length().unwrap() - full / 4.0).abs() / full < 1e-12);
        let q = EllipticalArc3::new(Frame::XY, 5.0, 3.0, PI, FRAC_PI_2).unwrap();
        assert!((q.length().unwrap() - full / 4.0).abs() / full < 1e-12);
        // A whole turn, and an arc that becomes a circle (r = 2, sweep 1: length 2).
        let w = EllipticalArc3::new(Frame::XY, 5.0, 3.0, 0.3, TAU).unwrap();
        assert_eq!(w.length().unwrap(), full);
        let c = EllipticalArc3::new(Frame::XY, 2.0, 2.0, 0.3, 1.0).unwrap();
        assert!((c.length().unwrap() - 2.0).abs() < 1e-12);
        // Very eccentric: quarter of a = 1e5, b = 1e-3 is close to a (the flat limit).
        let e = EllipticalArc3::new(Frame::XY, 1e5, 1e-3, 0.0, FRAC_PI_2).unwrap();
        let full = Ellipse3::new(Frame::XY, 1e5, 1e-3).unwrap().length();
        assert!((e.length().unwrap() - full / 4.0).abs() / full < 1e-11);
    }

    #[test]
    fn ellipse_closest_points() {
        let e = Ellipse3::new(Frame::XY, 5.0, 3.0).unwrap();
        // Outside on the axes: the vertices.
        let q = e.closest_point(Point3::new(9.0, 0.0, 0.0)).unwrap();
        assert!(close(q.point, Point3::new(5.0, 0.0, 0.0), 1e-12) && q.unique);
        let q = e.closest_point(Point3::new(0.0, -7.0, 2.0)).unwrap();
        assert!(close(q.point, Point3::new(0.0, -3.0, 0.0), 1e-12));
        // On the major axis inside the evolute: x0 = a^2 y0 / (a^2 - b^2) = 25 * 1 / 16,
        // x1 = 3 sqrt(1 - (x0/5)^2) = 3 sqrt(1 - (5/16)^2): two mirror points (x0, ±x1).
        let q = e.closest_point(Point3::new(1.0, 0.0, 0.0)).unwrap();
        let x0: f64 = 25.0 / 16.0;
        let x1 = 3.0 * (1.0 - (x0 / 5.0) * (x0 / 5.0)).sqrt();
        assert!(!q.unique);
        assert!((q.point.x - x0).abs() < 1e-12 && (q.point.y.abs() - x1).abs() < 1e-12);
        assert!(!e.closest_point(Point3::ORIGIN).unwrap().unique);
        // A point on the ellipse is its own nearest point.
        let on = e.point_at(0.9);
        let q = e.closest_point(on).unwrap();
        assert!(q.distance < 1e-12 && (q.t - 0.9).abs() < 1e-12);
        // Along the normal at t: c(t) + k n(t) has nearest point c(t) for any k while
        // the point stays outside (and k > -ρ inside). n(t) ∝ (b cos t, a sin t).
        let tilted_e = Ellipse3::new(tilted(), 5.0, 3.0).unwrap();
        for k in [-1.0, 0.5, 4.0] {
            for t in [0.3, 2.0, 4.0, 5.5] {
                let (s, c) = f64::sin_cos(t);
                let n = Vec3::new(3.0 * c, 5.0 * s, 0.0).normalized().unwrap();
                let p = tilted()
                    .to_world(Frame::XY.to_world(Point3::new(5.0 * c, 3.0 * s, 0.0)) + n * k);
                let q = tilted_e.closest_point(p).unwrap();
                assert!((q.t - t).abs() < 1e-9, "k={k} t={t} got {}", q.t);
                assert!((q.distance - k.abs()).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn elliptical_arc_closest_points_and_box() {
        // Upper half of a = 5, b = 3. A point below the centre is nearest to... both ends
        // (±5, 0) are 5.x away but (0, 3) is farther: the ends tie.
        let a = EllipticalArc3::new(Frame::XY, 5.0, 3.0, 0.0, PI).unwrap();
        let q = a.closest_point(Point3::new(0.0, -10.0, 0.0)).unwrap();
        assert!(!q.unique && (q.distance - 125f64.sqrt()).abs() < 1e-12);
        let q = a.closest_point(Point3::new(0.0, 10.0, 0.0)).unwrap();
        assert!(close(q.point, Point3::new(0.0, 3.0, 0.0), 1e-9));
        // Arc from π/4 to 3π/4: box y max is b = 3 (top vertex), x within ±5 cos(π/4).
        let b = EllipticalArc3::new(Frame::XY, 5.0, 3.0, PI / 4.0, FRAC_PI_2)
            .unwrap()
            .bounding_box();
        let h = 0.5f64.sqrt();
        assert!((b.max.y - 3.0).abs() < 1e-12 && (b.min.y - 3.0 * h).abs() < 1e-12);
        assert!((b.max.x - 5.0 * h).abs() < 1e-12);
        let b = Ellipse3::new(Frame::XY, 5.0, 3.0).unwrap().bounding_box();
        assert!(close(b.max, Point3::new(5.0, 3.0, 0.0), 1e-12));
    }

    #[test]
    fn hostile_input_is_an_error_not_a_panic() {
        let nan = f64::NAN;
        let inf = f64::INFINITY;
        let o = Point3::ORIGIN;
        assert!(Line3::new(o, Vec3::ZERO).is_err());
        assert!(Line3::new(Point3::new(nan, 0.0, 0.0), Vec3::X).is_err());
        assert!(Line3::new(o, Vec3::new(inf, 0.0, 0.0)).is_err());
        assert!(Segment3::new(o, o).is_err());
        assert!(Segment3::new(o, Point3::new(LINEAR_TOLERANCE / 2.0, 0.0, 0.0)).is_err());
        assert!(Segment3::new(o, Point3::new(1e300, 0.0, 0.0)).is_err());
        for r in [0.0, -1.0, nan, inf, 1e300, LINEAR_TOLERANCE / 2.0] {
            assert!(Circle3::new(Frame::XY, r).is_err(), "r={r}");
            assert!(Ellipse3::new(Frame::XY, 1.0, r).is_err());
            assert!(Arc3::new(Frame::XY, r, 0.0, 1.0).is_err());
        }
        for sweep in [0.0, -1.0, nan, inf, 7.0, 1e-12] {
            assert!(
                Arc3::new(Frame::XY, 1.0, 0.0, sweep).is_err(),
                "sweep={sweep}"
            );
            assert!(EllipticalArc3::new(Frame::XY, 2.0, 1.0, 0.0, sweep).is_err());
        }
        assert!(Arc3::new(Frame::XY, 1.0, nan, 1.0).is_err());
        let c = Circle3::new(Frame::XY, 1.0).unwrap();
        for p in [
            Point3::new(nan, 0.0, 0.0),
            Point3::new(0.0, inf, 0.0),
            Point3::new(0.0, 0.0, 1e300),
        ] {
            assert!(c.closest_point(p).is_err());
            assert!(
                Ellipse3::new(Frame::XY, 2.0, 1.0)
                    .unwrap()
                    .closest_point(p)
                    .is_err()
            );
            assert!(
                Segment3::new(o, Point3::new(1.0, 0.0, 0.0))
                    .unwrap()
                    .closest_point(p)
                    .is_err()
            );
        }
        // Huge start angles are accepted and normalised.
        let a = Arc3::new(Frame::XY, 1.0, 1e12, 1.0).unwrap();
        assert!((0.0..TAU).contains(&a.start_angle()));
        // Evaluating at NaN gives NaN, not a panic.
        assert!(!c.point_at(nan).is_valid());
        // Biggest allowed shapes still answer.
        let big = Ellipse3::new(Frame::XY, 1e6, 1e-6).unwrap();
        assert!(big.closest_point(Point3::new(1e6, 1e6, 1e6)).is_ok());
        assert!(
            EllipticalArc3::new(Frame::XY, 1e6, 1e-6, 0.0, 3.0)
                .unwrap()
                .length()
                .is_ok()
        );
    }
}

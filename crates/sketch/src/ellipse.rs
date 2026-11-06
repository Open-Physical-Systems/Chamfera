//! Ellipse geometry: where an ellipse or elliptical arc of the sketch lies, for profiles,
//! previews, display and projection.
//!
//! A sketch ellipse with centre `c`, major point `m` and minor point `n` is
//! `p(t) = c + a cos t · u + b sin t · v` with `a = |m - c|`, `u = (m - c) / a`,
//! `b = |n - c|` and `v = perp(u)` (u turned a quarter counter-clockwise), so the parameter
//! runs counter-clockwise whichever side of the major axis the minor point is on. The
//! parameter of a point `q` is `atan2(v·(q - c) / b, u·(q - c) / a)`, exact for points on
//! the ellipse.

use crate::entity::{Curve, CurveId};
use crate::sketch::Sketch;
use chamfera_geom::{BSplineCurve2, LINEAR_TOLERANCE, NurbsError, Point2, Vec2};
use std::f64::consts::{FRAC_PI_2, TAU};

/// An ellipse, or a counter-clockwise arc of one (`sweep` = 2π for the whole ellipse).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EllipseArc {
    pub centre: Point2,
    /// Unit direction of the first semi-axis (toward the major point).
    pub axis: Vec2,
    /// Length of the first semi-axis (mm).
    pub a: f64,
    /// Length of the second semi-axis (mm).
    pub b: f64,
    /// The parameter where the arc starts.
    pub t0: f64,
    /// The counter-clockwise sweep of the parameter, in (0, 2π].
    pub sweep: f64,
}

impl EllipseArc {
    /// The ellipse through centre `c`, major point `m` and minor point `n`, whole; `None`
    /// when a semi-axis is shorter than the linear tolerance.
    pub fn full(c: Point2, m: Point2, n: Point2) -> Option<EllipseArc> {
        let d = m - c;
        let a = d.length();
        let b = n.distance(c);
        let ok = |v: f64| v.is_finite() && v > LINEAR_TOLERANCE;
        if !(ok(a) && ok(b)) {
            return None;
        }
        Some(EllipseArc {
            centre: c,
            axis: d * (1.0 / a),
            a,
            b,
            t0: 0.0,
            sweep: TAU,
        })
    }

    /// The counter-clockwise arc of that ellipse from (the parameter of) `s` to `e`.
    pub fn arc(c: Point2, m: Point2, n: Point2, s: Point2, e: Point2) -> Option<EllipseArc> {
        let full = EllipseArc::full(c, m, n)?;
        let t0 = full.param_of(s);
        let mut sweep = full.param_of(e) - t0;
        if sweep <= 0.0 {
            sweep += TAU;
        }
        Some(EllipseArc { t0, sweep, ..full })
    }

    /// The second axis direction.
    pub fn minor_axis(&self) -> Vec2 {
        self.axis.perp()
    }

    pub fn point_at(&self, t: f64) -> Point2 {
        self.centre + self.axis * (self.a * t.cos()) + self.minor_axis() * (self.b * t.sin())
    }

    /// `dp/dt`.
    pub fn tangent_at(&self, t: f64) -> Vec2 {
        self.axis * (-self.a * t.sin()) + self.minor_axis() * (self.b * t.cos())
    }

    /// The parameter of a point (exact on the ellipse; for other points, of the point
    /// where the ray from the centre through it meets the ellipse, scaled to a circle).
    pub fn param_of(&self, p: Point2) -> f64 {
        let d = p - self.centre;
        (self.minor_axis().dot(d) / self.b).atan2(self.axis.dot(d) / self.a)
    }

    pub fn is_full(&self) -> bool {
        self.sweep >= TAU - 1e-12
    }

    /// The arc as an exact rational quadratic B-spline (The NURBS Book, §7.3 and §7.5):
    /// the arc of the unit circle from `t0` through `sweep` in `k = ceil(sweep / (π/2))`
    /// equal pieces of `Δ = sweep / k`, each with end points `(cos θ, sin θ)`, the middle
    /// point `(cos θ_m, sin θ_m) / cos(Δ/2)` where the end tangents meet and weights
    /// `1, cos(Δ/2), 1`, mapped by the affine map `(x, y) -> c + a x u + b y v`. A
    /// rational curve's control points map with the curve, so the result is the ellipse
    /// exactly. Its parameter runs from 0 to `k`.
    pub fn to_nurbs(&self) -> Result<BSplineCurve2, NurbsError> {
        let pieces = (self.sweep / FRAC_PI_2 - 1e-9).ceil().clamp(1.0, 4.0) as usize;
        let delta = self.sweep / pieces as f64;
        let w = (0.5 * delta).cos();
        let map = |x: f64, y: f64| {
            self.centre + self.axis * (self.a * x) + self.minor_axis() * (self.b * y)
        };
        let mut points = vec![self.point_at(self.t0)];
        let mut weights = vec![1.0];
        let mut knots = vec![0.0; 3];
        for k in 0..pieces {
            let mid = self.t0 + delta * (k as f64 + 0.5);
            points.push(map(mid.cos() / w, mid.sin() / w));
            points.push(self.point_at(self.t0 + delta * (k + 1) as f64));
            weights.extend([w, 1.0]);
            let knot = (k + 1) as f64;
            let times = if k + 1 == pieces { 3 } else { 2 };
            knots.extend(std::iter::repeat_n(knot, times));
        }
        BSplineCurve2::new(2, knots, points, Some(weights))
    }

    /// The arc as a polyline with about `per_turn` segments for a full turn (at least 2).
    pub fn polyline(&self, per_turn: usize) -> Vec<Point2> {
        let n = ((per_turn as f64) * self.sweep / TAU).ceil().max(2.0) as usize;
        (0..=n)
            .map(|k| self.point_at(self.t0 + self.sweep * k as f64 / n as f64))
            .collect()
    }
}

impl Sketch {
    /// Where an ellipse or elliptical arc lies; `None` for other curves, or if a point is
    /// missing or a semi-axis has no length.
    pub fn ellipse_arc(&self, id: CurveId) -> Option<EllipseArc> {
        self.ellipse_arc_of(self.curve(id)?)
    }

    /// [`Sketch::ellipse_arc`] for a curve given by value.
    pub fn ellipse_arc_of(&self, curve: &Curve) -> Option<EllipseArc> {
        match curve {
            Curve::Ellipse {
                centre,
                major,
                minor,
                ..
            } => EllipseArc::full(
                self.point(*centre)?,
                self.point(*major)?,
                self.point(*minor)?,
            ),
            Curve::EllipticalArc {
                centre,
                major,
                minor,
                start,
                end,
                ..
            } => EllipseArc::arc(
                self.point(*centre)?,
                self.point(*major)?,
                self.point(*minor)?,
                self.point(*start)?,
                self.point(*end)?,
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn an_ellipse_runs_counter_clockwise_from_its_major_point() {
        let c = Point2::new(1.0, 2.0);
        // a = 10 along +Y, b = 5; the minor point is on the -X side (the right of +Y).
        let e = EllipseArc::full(c, Point2::new(1.0, 12.0), Point2::new(6.0, 2.0)).unwrap();
        assert!(e.point_at(0.0).distance(Point2::new(1.0, 12.0)) < 1e-12);
        // A quarter turn on: v = perp(+Y) = -X, at distance b.
        assert!(e.point_at(FRAC_PI_2).distance(Point2::new(-4.0, 2.0)) < 1e-12);
        assert!((e.param_of(Point2::new(1.0, -8.0)).abs() - PI).abs() < 1e-12);
        assert!(EllipseArc::full(c, c, Point2::new(3.0, 2.0)).is_none());
    }

    #[test]
    fn the_nurbs_of_an_ellipse_lies_on_it_and_encloses_pi_a_b() {
        // a = 10 along (3, 4) / 5, b = 5, centre (1, 2): every point of the spline
        // satisfies (u·d / a)² + (v·d / b)² = 1, and the whole ellipse encloses
        // π a b = 50π (Green's theorem over the closed curve).
        let c = Point2::new(1.0, 2.0);
        let m = c + Vec2::new(6.0, 8.0);
        let n = c + Vec2::new(-4.0, 3.0);
        let e = EllipseArc::full(c, m, n).unwrap();
        let s = e.to_nurbs().unwrap();
        let (lo, hi) = s.domain();
        for i in 0..=200 {
            let p = s.point_at(lo + (hi - lo) * i as f64 / 200.0);
            let d = p - c;
            let x = e.axis.dot(d) / 10.0;
            let y = e.minor_axis().dot(d) / 5.0;
            assert!((x * x + y * y - 1.0).abs() < 1e-12, "{p:?}");
        }
        assert!(s.point_at(lo).distance(m) < 1e-12 && s.point_at(hi).distance(m) < 1e-12);
        let area = s.area_term(lo, hi).unwrap();
        assert!((area - 50.0 * PI).abs() < 1e-9, "{area}");
        // A third of a turn (2π/3) from t = 1, in two pieces
        // of π/3: it starts and ends where the ellipse does, and the joint of its pieces
        // is the ellipse's point at t = 1 + π/3.
        let arc = EllipseArc {
            t0: 1.0,
            sweep: 2.0 * PI / 3.0,
            ..e
        };
        let s = arc.to_nurbs().unwrap();
        let (lo, hi) = s.domain();
        assert!(s.point_at(lo).distance(e.point_at(1.0)) < 1e-12);
        assert!(s.point_at(hi).distance(e.point_at(1.0 + 2.0 * PI / 3.0)) < 1e-12);
        assert!(s.point_at(0.5 * (lo + hi)).distance(e.point_at(1.0 + PI / 3.0)) < 1e-12);
    }

    #[test]
    fn an_arc_sweeps_from_start_to_end() {
        let c = Point2::ORIGIN;
        let (m, n) = (Point2::new(10.0, 0.0), Point2::new(0.0, 5.0));
        // From the top (t = π/2) to the major point (t = 0): three quarters of a turn.
        let a = EllipseArc::arc(c, m, n, Point2::new(0.0, 5.0), m).unwrap();
        assert!((a.t0 - FRAC_PI_2).abs() < 1e-12);
        assert!((a.sweep - 1.5 * PI).abs() < 1e-12);
        let poly = a.polyline(64);
        assert!(poly.first().unwrap().distance(Point2::new(0.0, 5.0)) < 1e-12);
        assert!(poly.last().unwrap().distance(m) < 1e-9);
    }
}

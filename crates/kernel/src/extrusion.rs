//! Surfaces of linear extrusion (rung K10): a planar B-spline swept along its plane's
//! normal, the side face an extruded spline makes.
//!
//! With `o, x, y, z` the frame and `c(u) = (c_x(u), c_y(u))` a [`BSplineCurve2`] in the
//! frame's plane,
//!
//! `S(u, v) = o + c_x(u) x + c_y(u) y + v z`, `S_u = c_x' x + c_y' y`, `S_v = z`,
//!
//! so `S_u × S_v = c_y' x - c_x' y`: the normal is the curve's tangent turned a quarter
//! turn clockwise in the plane, to the right of the direction of travel (outward for a
//! counter-clockwise profile). `u` runs over the curve's domain (not periodic), `v` is
//! the height above the plane (unbounded). The parameterisation is orthogonal and
//! `|S_u × S_v| = |c'(u)|`, so a face's area is `∬ |c'(u)| du dv`. This is ISO 10303-42's
//! `surface_of_linear_extrusion` with a unit extrusion axis along `z`; The NURBS Book
//! §8.4 builds the same surface as a B-spline patch of degree 1 in `v`.
//!
//! Points are projected (`parameters`) by the curve's point inversion (Newton with a
//! subdivision fallback, §6.1) of the point's plane coordinates, `v` being its height.

use crate::check;
use crate::topology::Curve;
use crate::{KernelError, KernelResult, Surface, SurfaceParameters, SurfacePoint};
use chamfera_geom::{ANGULAR_TOLERANCE, BSplineCurve2, BSplineCurve3, Frame, Point2, Point3, Vec3};

/// Samples per knot span when bounding a spline's second derivative for tessellation.
const STEP_SAMPLES_PER_SPAN: usize = 16;
/// Most samples one such bound takes.
const MAX_STEP_SAMPLES: usize = 4096;
/// Margin on the sampled bounds, for peaks between samples.
const STEP_MARGIN: f64 = 1.25;

/// A profile spline as an edge curve on `frame` (the sketch plane, or a moved copy of
/// it), over its whole domain.
pub(crate) fn profile_spline_curve(
    frame: &Frame,
    c: &BSplineCurve2,
) -> KernelResult<(Curve, f64, f64)> {
    let c3: BSplineCurve3 = c.to_3d(frame)?;
    let (lo, hi) = c3.domain();
    Ok((Curve::BSpline(c3), lo, hi))
}

/// The side surface a profile spline sweeps when extruded by `d`, which must be along
/// the sketch normal (the surface is extruded along its frame's z).
pub(crate) fn extruded_side(frame: &Frame, d: Vec3, c: &BSplineCurve2) -> KernelResult<Surface> {
    let Some(dir) = d.normalized() else {
        return Err(KernelError::InvalidInput(
            "the extrusion has no direction".into(),
        ));
    };
    if dir.cross(frame.z()).length() > ANGULAR_TOLERANCE {
        return Err(KernelError::Unsupported(
            "extruding a spline other than along its sketch's normal".into(),
        ));
    }
    Ok(Surface::Extrusion(Extrusion::new(*frame, c.clone())?))
}

/// The largest parameter step of a spline edge over `[t0, t1]` that keeps the chord
/// error within `tol` and the tangent's turn within `angle` (the bounds used for
/// ellipses in `tessellate/edges.rs`): linear interpolation over a step `h` deviates by
/// at most `h² max|C''| / 8`, and the tangent turns at the rate `|C' × C''| / |C'|²`.
/// The maxima are taken over samples on every knot span, with a margin.
pub(crate) fn spline_step(c: &BSplineCurve3, t0: f64, t1: f64, tol: f64, angle: f64) -> f64 {
    let br: Vec<f64> = c.breaks();
    let spans = br
        .windows(2)
        .filter(|w| w[1] > t0 && w[0] < t1)
        .count()
        .max(1);
    let per = STEP_SAMPLES_PER_SPAN.min(MAX_STEP_SAMPLES / spans).max(2);
    let (mut acc, mut turn) = (0.0f64, 0.0f64);
    for w in br.windows(2) {
        let (s, e) = (w[0].max(t0), w[1].min(t1));
        if e <= s {
            continue;
        }
        for i in 0..=per {
            let u = s + (e - s) * i as f64 / per as f64;
            let d = c.derivatives(u, 2);
            let (Some(d1), Some(d2)) = (d.get(1), d.get(2)) else {
                continue;
            };
            acc = acc.max(d2.length());
            let speed2 = d1.dot(*d1);
            if speed2 > 0.0 {
                turn = turn.max(d1.cross(*d2).length() / speed2);
            }
        }
    }
    let (acc, turn) = (acc * STEP_MARGIN, turn * STEP_MARGIN);
    let span = (t1 - t0).abs();
    let chord = if acc > 0.0 {
        (8.0 * tol / acc).sqrt()
    } else {
        span
    };
    let bend = if turn > 0.0 { angle / turn } else { span };
    chord.min(bend).min(span.max(f64::MIN_POSITIVE))
}

/// A surface of linear extrusion (see the module documentation).
#[derive(Debug, Clone, PartialEq)]
pub struct Extrusion {
    pub(crate) frame: Frame,
    pub(crate) curve: BSplineCurve2,
}

impl Extrusion {
    /// The surface swept by `curve` (in `frame`'s plane coordinates) along `frame.z()`.
    pub fn new(frame: Frame, curve: BSplineCurve2) -> KernelResult<Extrusion> {
        let (lo, hi) = curve.hull_box();
        if !(lo.is_valid() && hi.is_valid() && frame.origin().is_valid()) {
            return Err(KernelError::InvalidInput(
                "an extruded curve must lie inside the model's working range".into(),
            ));
        }
        Ok(Extrusion { frame, curve })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    /// The extruded curve, in the frame's plane coordinates.
    pub fn curve(&self) -> &BSplineCurve2 {
        &self.curve
    }

    pub fn point_at(&self, u: f64, v: f64) -> Point3 {
        self.frame.from_plane(self.curve.point_at(u)) + self.frame.z() * v
    }

    /// `(S_u, S_v)`.
    pub fn derivatives(&self, u: f64, _v: f64) -> (Vec3, Vec3) {
        (
            self.frame.vector_from_plane(self.curve.d1(u)),
            self.frame.z(),
        )
    }

    /// The unit normal, the direction of `S_u × S_v` (where the curve's speed vanishes,
    /// the direction of its second derivative's normal, else the frame's x axis).
    pub fn normal_at(&self, u: f64, v: f64) -> Vec3 {
        let (su, sv) = self.derivatives(u, v);
        su.cross(sv)
            .normalized()
            .or_else(|| {
                self.frame
                    .vector_from_plane(self.curve.d2(u))
                    .cross(sv)
                    .normalized()
            })
            .unwrap_or(self.frame.x())
    }

    /// The parameters of the surface point nearest to `p`.
    pub fn parameters(&self, p: Point3) -> SurfaceParameters {
        let l = self.frame.to_local(p);
        let c = self.curve.closest_point(Point2::new(l.x, l.y));
        SurfaceParameters {
            u: c.u,
            v: l.z,
            singular: false,
        }
    }

    /// The surface point nearest to `p`. The nearest point is the nearest point of the
    /// curve in the plane lifted to `p`'s height, so it is unique when the curve's is.
    pub fn closest_point(&self, p: Point3) -> KernelResult<SurfacePoint> {
        check::point3("point", p)?;
        let q = self.parameters(p);
        let point = self.point_at(q.u, q.v);
        Ok(SurfacePoint {
            u: q.u,
            v: q.v,
            point,
            distance: point.distance(p),
            unique: true,
        })
    }

    /// The same surface with the curve's direction reversed (the normal flips).
    pub fn reversed(&self) -> Extrusion {
        Extrusion {
            frame: self.frame,
            curve: self.curve.reversed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    fn quarter() -> BSplineCurve2 {
        BSplineCurve2::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Point2::new(2.0, 0.0),
                Point2::new(2.0, 2.0),
                Point2::new(0.0, 2.0),
            ],
            Some(vec![1.0, FRAC_1_SQRT_2, 1.0]),
        )
        .unwrap()
    }

    #[test]
    fn an_extruded_quarter_circle_is_a_quarter_cylinder() {
        // The rational quarter circle of radius 2 extruded along z is the cylinder
        // x² + y² = 4: every point at distance 2 from the axis, the normal radial and
        // outward (the curve runs counter-clockwise, so its right is outside).
        let e = Extrusion::new(Frame::XY, quarter()).unwrap();
        for i in 0..=10 {
            let u = i as f64 / 10.0;
            for v in [-3.0, 0.0, 5.0] {
                let p = e.point_at(u, v);
                assert!((p.x.hypot(p.y) - 2.0).abs() < 1e-12);
                assert!((p.z - v).abs() < 1e-15);
                let n = e.normal_at(u, v);
                let radial = Vec3::new(p.x, p.y, 0.0) * 0.5;
                assert!((n - radial).length() < 1e-12, "{n:?} vs {radial:?}");
                // Projection recovers the parameters.
                let q = e.parameters(p);
                assert!((q.u - u).abs() < 1e-9 && (q.v - v).abs() < 1e-12);
            }
        }
        // From (3, 3, 1) the nearest point is at 45°, distance 3√2 - 2.
        let c = e.closest_point(Point3::new(3.0, 3.0, 1.0)).unwrap();
        assert!((c.distance - (3.0 * 2f64.sqrt() - 2.0)).abs() < 1e-12);
        assert!((c.point.z - 1.0).abs() < 1e-15);
        assert!(e.closest_point(Point3::new(f64::NAN, 0.0, 0.0)).is_err());
        let r = e.reversed();
        assert!((r.normal_at(0.5, 0.0) + e.normal_at(0.5, 0.0)).length() < 1e-12);
    }
}

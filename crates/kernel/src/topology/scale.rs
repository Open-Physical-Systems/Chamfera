//! Scaling a whole body about a point: by one factor (uniform), or by a factor along each
//! of the world axes (non-uniform).
//!
//! The map is `p' = c + S (p - c)` with `S = diag(sx, sy, sz)`, every factor positive (a
//! negative factor is a mirror: use one). An affine map with `det S > 0` keeps every
//! face's outward side, so the topology is copied unchanged and only the geometry moves:
//!
//! - **Points** are mapped.
//! - **Segments** map their ends (their parameter is `0..1` either way); a **line**'s
//!   direction is stored unit length, so its parameter stretches by `|S d|` and every
//!   edge on it gets its range stretched to match. **B-splines** map their control points
//!   (affine invariance, Piegl and Tiller 1997, section 4.4), so parameters are unchanged.
//! - **Planes** stay planes: the origin is mapped, the normal goes to `S⁻¹ n` (the inverse
//!   transpose of a diagonal map, so it stays square to every mapped in-plane vector) and
//!   the x axis to `S x`.
//! - **Circles, arcs, ellipses** stay what they are when their plane is stretched alike in
//!   every direction (`|S x| = |S y|` and `S x ⟂ S y`): radii grow by that stretch and the
//!   angle parameter is unchanged. Otherwise a circle would become an ellipse at a new
//!   phase, which is refused.
//! - **Cylinders and cones** stay so when their axis is a direction `S` only stretches
//!   (`S z ∥ z`) and the cross-section is stretched alike: the radius grows by the
//!   cross-section stretch `k`, and a cone's `tan(half angle)` by `k / k_axis`.
//!   **Spheres and tori** need the same stretch in all three directions of their frame.
//! - **Extrusions** (a planar B-spline swept along the frame's z) need only `S z ∥ z`: the
//!   curve is mapped into the new frame's plane, any stretch within the plane allowed.
//!
//! Every analytic surface therefore stays analytic (the kernel rule); what would need a
//! NURBS surface (an elliptic cylinder, an ellipsoid) is refused with the reason.

use super::{Body, Curve, Validation};
use crate::curve3::{Arc3, Circle3, Ellipse3, EllipticalArc3, Line3, Segment3};
use crate::surface::Surface;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, Point3, Transform, Vec3};

/// Smallest and largest factor a scale takes (beyond them a body is lost to the working
/// range or to the tolerance).
pub const MIN_SCALE_FACTOR: f64 = 1e-4;
pub const MAX_SCALE_FACTOR: f64 = 1e4;

/// Relative tolerance for "stretched alike" (two lengths or a dot product of the mapped
/// unit axes compared with their size). The axes are unit vectors and the factors at most
/// `MAX_SCALE_FACTOR` apart, so this is far above rounding and far below any real
/// difference a user types.
const SAME_STRETCH: f64 = 1e-9;

fn out_of_range() -> KernelError {
    KernelError::InvalidInput("the scaled body leaves the working range".into())
}

fn unsupported(what: &str) -> KernelError {
    KernelError::Unsupported(format!(
        "a non-uniform scale would turn {what} into a shape the kernel does not model yet; \
         scale it uniformly, or by equal factors across that axis"
    ))
}

/// The scale map: centre and factors.
struct Map {
    c: Point3,
    s: [f64; 3],
}

impl Map {
    fn vector(&self, v: Vec3) -> Vec3 {
        Vec3::new(v.x * self.s[0], v.y * self.s[1], v.z * self.s[2])
    }
    /// `S⁻¹ v`, the map of a normal (inverse transpose of a diagonal matrix).
    fn normal(&self, v: Vec3) -> Vec3 {
        Vec3::new(v.x / self.s[0], v.y / self.s[1], v.z / self.s[2])
    }
    fn point(&self, p: Point3) -> KernelResult<Point3> {
        let q = self.c + self.vector(p - self.c);
        if q.is_valid() {
            Ok(q)
        } else {
            Err(out_of_range())
        }
    }
    fn transform(&self) -> KernelResult<Transform> {
        let m = [
            [self.s[0], 0.0, 0.0],
            [0.0, self.s[1], 0.0],
            [0.0, 0.0, self.s[2]],
        ];
        let t = (self.c - Point3::ORIGIN) - self.vector(self.c - Point3::ORIGIN);
        Transform::from_parts(m, t).ok_or_else(out_of_range)
    }
    /// The stretch of a frame's plane when `S x` and `S y` are as long and square to
    /// each other, else `None`.
    fn plane_stretch(&self, f: &Frame) -> Option<f64> {
        let (a, b) = (self.vector(f.x()), self.vector(f.y()));
        let (la, lb) = (a.length(), b.length());
        let tol = SAME_STRETCH * la.max(lb);
        ((la - lb).abs() <= tol && a.dot(b).abs() <= tol * la.max(lb)).then_some(la)
    }
    /// The stretch along a frame's z when `S z ∥ z`, else `None`.
    fn axis_stretch(&self, f: &Frame) -> Option<f64> {
        let a = self.vector(f.z());
        let l = a.length();
        (a.cross(f.z()).length() <= SAME_STRETCH * l).then_some(l)
    }
    /// The frame with mapped origin and axes `S z`, `S x` (right-handed again).
    fn frame(&self, f: &Frame, z: Vec3) -> KernelResult<Frame> {
        Frame::from_normal_and_x(self.point(f.origin())?, z, self.vector(f.x()))
            .ok_or_else(out_of_range)
    }
    /// A curve's frame: its plane must be stretched alike; returns the new frame and the
    /// stretch.
    fn curve_frame(&self, f: &Frame, what: &str) -> KernelResult<(Frame, f64)> {
        let k = self.plane_stretch(f).ok_or_else(|| unsupported(what))?;
        let z = self.vector(f.x()).cross(self.vector(f.y()));
        Ok((self.frame(f, z)?, k))
    }
}

fn curve(c: &Curve, m: &Map, t: &Transform) -> KernelResult<(Curve, f64)> {
    Ok(match c {
        Curve::Line(l) => {
            let d = m.vector(l.direction());
            let line = Line3::new(m.point(l.origin())?, d)?;
            // `o + d t` maps to `o' + S d t`; the new line's direction is unit length.
            let k = d.length() / line.direction().length();
            (Curve::Line(line), k)
        }
        Curve::Segment(s) => (
            Curve::Segment(Segment3::new(m.point(s.start())?, m.point(s.end())?)?),
            1.0,
        ),
        Curve::Circle(c) => {
            let (f, k) = m.curve_frame(c.frame(), "a circle")?;
            (Curve::Circle(Circle3::new(f, c.radius() * k)?), 1.0)
        }
        Curve::Arc(a) => {
            let (f, k) = m.curve_frame(a.circle().frame(), "an arc")?;
            (
                Curve::Arc(Arc3::new(
                    f,
                    a.circle().radius() * k,
                    a.start_angle(),
                    a.sweep(),
                )?),
                1.0,
            )
        }
        Curve::Ellipse(e) => {
            let (f, k) = m.curve_frame(e.frame(), "an ellipse")?;
            (
                Curve::Ellipse(Ellipse3::new(f, e.rx() * k, e.ry() * k)?),
                1.0,
            )
        }
        Curve::EllipticalArc(a) => {
            let e = a.ellipse();
            let (f, k) = m.curve_frame(e.frame(), "an elliptical arc")?;
            let (start, end) = a.range();
            (
                Curve::EllipticalArc(EllipticalArc3::new(
                    f,
                    e.rx() * k,
                    e.ry() * k,
                    start,
                    end - start,
                )?),
                1.0,
            )
        }
        Curve::BSpline(b) => (Curve::BSpline(b.transformed(t)?), 1.0),
    })
}

/// A revolved surface's frame: axis kept, cross-section stretched alike. Returns the new
/// frame, the cross-section stretch and the axis stretch.
fn axial(m: &Map, f: &Frame, what: &str) -> KernelResult<(Frame, f64, f64)> {
    let (Some(k), Some(ka)) = (m.plane_stretch(f), m.axis_stretch(f)) else {
        return Err(unsupported(what));
    };
    Ok((m.frame(f, m.vector(f.z()))?, k, ka))
}

/// The same stretch in all three directions of a frame.
fn round(m: &Map, f: &Frame, what: &str) -> KernelResult<(Frame, f64)> {
    let (fr, k, ka) = axial(m, f, what)?;
    if (k - ka).abs() > SAME_STRETCH * k.max(ka) {
        return Err(unsupported(what));
    }
    Ok((fr, k))
}

fn surface(s: &Surface, m: &Map, t: &Transform) -> KernelResult<Surface> {
    Ok(match s {
        Surface::Plane(f) => Surface::Plane(
            Frame::from_normal_and_x(m.point(f.origin())?, m.normal(f.z()), m.vector(f.x()))
                .ok_or_else(out_of_range)?,
        ),
        Surface::Cylinder(c) => {
            let (f, k, _) = axial(m, c.frame(), "a cylinder")?;
            Surface::cylinder(f, c.radius() * k)?
        }
        Surface::Cone(c) => {
            let (f, k, ka) = axial(m, c.frame(), "a cone")?;
            // A generator runs out `tan(a)` per unit of axis: x by k, axis by ka.
            let half = (c.half_angle().tan() * k / ka).atan();
            Surface::cone(f, c.radius() * k, half)?
        }
        Surface::Sphere(c) => {
            let (f, k) = round(m, c.frame(), "a sphere")?;
            Surface::sphere(f, c.radius() * k)?
        }
        Surface::Torus(c) => {
            let (f, k) = round(m, c.frame(), "a torus")?;
            Surface::torus(f, c.major_radius() * k, c.minor_radius() * k)?
        }
        Surface::Extrusion(e) => {
            let old = e.frame();
            if m.axis_stretch(old).is_none() {
                return Err(unsupported("an extruded spline face"));
            }
            let f = m.frame(old, m.vector(old.z()))?;
            let curve = e.curve().to_3d(old)?.transformed(t)?.to_plane(&f)?;
            Surface::Extrusion(crate::Extrusion::new(f, curve)?)
        }
        // Affine invariance (The NURBS Book §4.4): the control points are mapped, any
        // positive scale keeps a free-form surface free-form. With `det S > 0`,
        // `(S S_u) × (S S_v) = det S · S^-T (S_u × S_v)` stays on the same side.
        Surface::Nurbs(n) => Surface::Nurbs(n.transformed(t, false)?),
    })
}

impl Body {
    /// A copy of the body scaled about `centre` by `factors` along x, y and z (all equal
    /// for a uniform scale). Each factor must be between [`MIN_SCALE_FACTOR`] and
    /// [`MAX_SCALE_FACTOR`]. Volumes scale by `sx sy sz`. A non-uniform scale that would
    /// make a curved face non-analytic is refused (see the module documentation). The copy
    /// is checked like any new body.
    pub fn scaled(&self, centre: Point3, factors: [f64; 3]) -> KernelResult<Body> {
        if !centre.is_valid() {
            return Err(KernelError::InvalidInput(
                "the scale's centre must be a finite point in the working range".into(),
            ));
        }
        if !factors
            .iter()
            .all(|f| f.is_finite() && (MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(f))
        {
            return Err(KernelError::InvalidInput(format!(
                "scale factors must be between {MIN_SCALE_FACTOR} and {MAX_SCALE_FACTOR} \
                 (mirror a body to turn it inside out)"
            )));
        }
        let m = Map {
            c: centre,
            s: factors,
        };
        let t = m.transform()?;
        let mut out = self.clone();
        for p in out.points.items.iter_mut().flatten() {
            *p = m.point(*p)?;
        }
        // Per curve slot: how much its parameter stretches (lines only).
        let mut stretch = vec![1.0; out.curves.items.len()];
        for (c, k) in out.curves.items.iter_mut().zip(stretch.iter_mut()) {
            if let Some(c) = c.as_mut() {
                let (new, s) = curve(c, &m, &t)?;
                *c = new;
                *k = s;
            }
        }
        for e in out.edges.items.iter_mut().flatten() {
            if let Some(ec) = e.curve.as_mut()
                && let Some(k) = stretch.get(ec.curve.index())
            {
                ec.t0 *= k;
                ec.t1 *= k;
            }
        }
        for s in out.surfaces.items.iter_mut().flatten() {
            *s = surface(s, &m, &t)?;
        }
        let mode = if out.is_open() {
            Validation::Surface
        } else {
            Validation::Solid
        };
        out.validate_as(mode)
            .map_err(|e| KernelError::InvalidResult(format!("the scaled body: {e}")))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Body, Profile};
    use chamfera_geom::{Frame, Point2, Point3, Vec3};
    use std::f64::consts::{PI, TAU};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-6 * b.abs().max(1.0)
    }

    #[test]
    fn a_box_scaled_by_two_has_eight_times_the_volume() {
        // 10 x 20 x 30 box at the origin: volume 6000; scale 2 about the origin gives
        // 20 x 40 x 60 = 48000 (8 times), from (0,0,0) to (20,40,60).
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let s = b.scaled(Point3::ORIGIN, [2.0; 3]).unwrap();
        assert!(close(s.volume().unwrap(), 48_000.0));
        let bb = s.bounding_box().unwrap();
        assert!(bb.max.distance(Point3::new(20.0, 40.0, 60.0)) < 1e-9);
        // About its centre (5, 10, 15) by 0.5: 5 x 10 x 15 from (2.5, 5, 7.5).
        let h = b.scaled(Point3::new(5.0, 10.0, 15.0), [0.5; 3]).unwrap();
        assert!(close(h.volume().unwrap(), 750.0));
        let bb = h.bounding_box().unwrap();
        assert!(bb.min.distance(Point3::new(2.5, 5.0, 7.5)) < 1e-9);
        let r = s.validate().unwrap();
        assert_eq!(r.euler_characteristic(), 2);
    }

    #[test]
    fn a_box_scales_non_uniformly() {
        // 10 x 10 x 10 by (2, 3, 0.5): 20 x 30 x 5 = 3000.
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(10.0, 10.0, 10.0)).unwrap();
        let s = b.scaled(Point3::ORIGIN, [2.0, 3.0, 0.5]).unwrap();
        assert!(close(s.volume().unwrap(), 3000.0));
        // Area 2 (20*30 + 20*5 + 30*5) = 1700.
        assert!(close(s.area().unwrap(), 1700.0));
    }

    #[test]
    fn a_cylinder_scales_along_its_axis_but_not_across_it() {
        // r = 5, h = 10 cylinder on z: 250 pi. Scale (2, 2, 3): r = 10, h = 30, 3000 pi.
        let c = crate::extrude(&Frame::XY, &Profile::circle(Point2::ORIGIN, 5.0), 10.0).unwrap();
        let s = c.scaled(Point3::ORIGIN, [2.0, 2.0, 3.0]).unwrap();
        assert!(close(s.volume().unwrap(), 3000.0 * PI));
        crate::tessellate(&s, &crate::TessOptions::default()).unwrap();
        // Across the axis it would be an elliptic cylinder: refused, not a wrong body.
        let e = c.scaled(Point3::ORIGIN, [2.0, 1.0, 1.0]).unwrap_err();
        assert!(e.to_string().contains("non-uniform"), "{e}");
    }

    #[test]
    fn a_sphere_scales_uniformly_only() {
        // Sphere r = 10 (a half disc turned about z): 4/3 pi 1000; by 2: 4/3 pi 8000.
        let p = Profile::new(
            vec![
                crate::ProfileEdge::Arc {
                    centre: Point2::ORIGIN,
                    radius: 10.0,
                    start_angle: -PI / 2.0,
                    sweep: PI,
                },
                crate::ProfileEdge::line(Point2::new(0.0, 10.0), Point2::new(0.0, -10.0)),
            ],
            vec![],
        );
        let axis = crate::Line2::new(Point2::ORIGIN, chamfera_geom::Vec2::new(0.0, 1.0)).unwrap();
        let sp = crate::revolve(&Frame::XZ, &p, &axis, TAU).unwrap();
        let s = sp.scaled(Point3::new(0.0, 0.0, 10.0), [2.0; 3]).unwrap();
        assert!(close(s.volume().unwrap(), 4.0 / 3.0 * PI * 8000.0));
        // The centre (0, 0, 0) is 10 below the scale centre: now 20 below it.
        let bb = s.bounding_box().unwrap();
        assert!((bb.min.z + 30.0).abs() < 1e-6 && (bb.max.z - 10.0).abs() < 1e-6);
        assert!(sp.scaled(Point3::ORIGIN, [1.0, 1.0, 2.0]).is_err());
    }

    #[test]
    fn a_cone_frustum_scaled_along_its_axis_keeps_its_volume_formula() {
        // Frustum r1 = 6, r2 = 3, h = 4 turned about z: pi h / 3 (r1² + r1 r2 + r2²)
        // = 4 pi / 3 * 63 = 84 pi. Scaled (1, 1, 2): h = 8, 168 pi.
        let p = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(6.0, 0.0),
            Point2::new(3.0, 4.0),
            Point2::new(0.0, 4.0),
        ]);
        let axis = crate::Line2::new(Point2::ORIGIN, chamfera_geom::Vec2::new(0.0, 1.0)).unwrap();
        let cone = crate::revolve(&Frame::XZ, &p, &axis, TAU).unwrap();
        assert!(close(cone.volume().unwrap(), 84.0 * PI));
        let s = cone.scaled(Point3::ORIGIN, [1.0, 1.0, 2.0]).unwrap();
        assert!(close(s.volume().unwrap(), 168.0 * PI));
    }

    #[test]
    fn bad_factors_and_centres_are_refused() {
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
        for f in [
            [0.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [f64::NAN, 1.0, 1.0],
            [f64::INFINITY; 3],
            [1e9; 3],
        ] {
            assert!(b.scaled(Point3::ORIGIN, f).is_err(), "{f:?}");
        }
        assert!(b.scaled(Point3::new(f64::NAN, 0.0, 0.0), [2.0; 3]).is_err());
        // Out of the working range.
        let far = Body::cuboid(Point3::new(1e5, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)).unwrap();
        assert!(far.scaled(Point3::ORIGIN, [1e4; 3]).is_err());
    }
}

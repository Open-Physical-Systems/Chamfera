//! Moving a whole body: every point, curve and surface goes through one rigid transform
//! (translations and rotations). Topology, names and face senses are unchanged, because a
//! rigid motion keeps orientation. Patterns use this.
//!
//! Mirroring (a reflection) turns every face inside out and needs the loops reversed; it
//! is not done here and is refused.

use super::{Body, Curve};
use crate::curve3::{Arc3, Circle3, Ellipse3, EllipticalArc3, Line3, Segment3};
use crate::surface::Surface;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, Transform};

fn frame(f: &Frame, t: &Transform) -> KernelResult<Frame> {
    f.transformed(t)
        .ok_or_else(|| KernelError::InvalidInput("the move leaves the working range".into()))
}

fn curve(c: &Curve, t: &Transform) -> KernelResult<Curve> {
    Ok(match c {
        Curve::Line(l) => Curve::Line(Line3::new(
            t.apply_point(l.origin()),
            t.apply_vector(l.direction()),
        )?),
        Curve::Segment(s) => Curve::Segment(Segment3::new(
            t.apply_point(s.start()),
            t.apply_point(s.end()),
        )?),
        Curve::Circle(c) => Curve::Circle(Circle3::new(frame(c.frame(), t)?, c.radius())?),
        Curve::Arc(a) => Curve::Arc(Arc3::new(
            frame(a.circle().frame(), t)?,
            a.circle().radius(),
            a.start_angle(),
            a.sweep(),
        )?),
        Curve::Ellipse(e) => Curve::Ellipse(Ellipse3::new(frame(e.frame(), t)?, e.rx(), e.ry())?),
        Curve::EllipticalArc(a) => {
            let e = a.ellipse();
            let (start, end) = a.range();
            Curve::EllipticalArc(EllipticalArc3::new(
                frame(e.frame(), t)?,
                e.rx(),
                e.ry(),
                start,
                end - start,
            )?)
        }
        Curve::BSpline(c) => Curve::BSpline(c.transformed(t)?),
    })
}

fn surface(s: &Surface, t: &Transform) -> KernelResult<Surface> {
    Ok(match s {
        Surface::Plane(f) => Surface::Plane(frame(f, t)?),
        Surface::Cylinder(c) => Surface::cylinder(frame(c.frame(), t)?, c.radius())?,
        Surface::Cone(c) => Surface::cone(frame(c.frame(), t)?, c.radius(), c.half_angle())?,
        Surface::Sphere(c) => Surface::sphere(frame(c.frame(), t)?, c.radius())?,
        Surface::Torus(c) => {
            Surface::torus(frame(c.frame(), t)?, c.major_radius(), c.minor_radius())?
        }
        // The curve is in the frame's plane coordinates, which a rigid move keeps.
        Surface::Extrusion(e) => Surface::Extrusion(crate::Extrusion::new(
            frame(e.frame(), t)?,
            e.curve().clone(),
        )?),
    })
}

impl Body {
    /// A copy of the body moved by the rigid transform `t`. The copy is checked like any
    /// new body.
    pub fn transformed(&self, t: &Transform) -> KernelResult<Body> {
        if !t.is_rigid() {
            return Err(KernelError::Unsupported(
                "only moves and rotations: mirroring a body is not supported yet".into(),
            ));
        }
        let mut out = self.clone();
        for p in out.points.items.iter_mut().flatten() {
            let q = t.apply_point(*p);
            if !q.is_valid() {
                return Err(KernelError::InvalidInput(
                    "the move leaves the working range".into(),
                ));
            }
            *p = q;
        }
        for c in out.curves.items.iter_mut().flatten() {
            *c = curve(c, t)?;
        }
        for s in out.surfaces.items.iter_mut().flatten() {
            *s = surface(s, t)?;
        }
        out.validate()
            .map_err(|e| KernelError::InvalidResult(format!("the moved body: {e}")))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Body, Profile};
    use chamfera_geom::{Frame, Point2, Point3, Transform, Vec3};

    #[test]
    fn a_moved_and_turned_body_keeps_its_measures() {
        // A plate with a round hole: volume and area are invariant under rigid motion.
        let profile = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(50.0, 0.0),
            Point2::new(50.0, 30.0),
            Point2::new(0.0, 30.0),
        ])
        .with_hole(vec![crate::ProfileEdge::circle(
            Point2::new(25.0, 15.0),
            5.0,
        )]);
        let b: Body = crate::extrude(&Frame::XY, &profile, 10.0).unwrap();
        let t =
            Transform::rotation_about(Point3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 0.0), 0.7)
                .unwrap()
                .then(&Transform::translation(Vec3::new(100.0, -40.0, 7.0)).unwrap());
        let m = b.transformed(&t).unwrap();
        assert!((m.volume().unwrap() - b.volume().unwrap()).abs() < 1e-6);
        assert!((m.area().unwrap() - b.area().unwrap()).abs() < 1e-6);
        // A pure translation moves the bounding box by exactly that offset.
        let moved = b
            .transformed(&Transform::translation(Vec3::new(5.0, 0.0, 0.0)).unwrap())
            .unwrap();
        let (b0, b1) = (b.bounding_box().unwrap(), moved.bounding_box().unwrap());
        assert!((b1.min.x - b0.min.x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn mirroring_is_refused_for_now() {
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
        // A point reflection (scale by -1) turns the body inside out, like a mirror.
        let flip = Transform::scale(-1.0).unwrap();
        assert!(b.transformed(&flip).is_err());
    }
}

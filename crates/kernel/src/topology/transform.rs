//! Moving a whole body: every point, curve and surface goes through one isometry, a rigid
//! motion (translation and rotation) or an improper one (a mirror, possibly turned too).
//! Patterns, moves and mirrors use this.
//!
//! # Mirroring
//! A reflection `R` keeps lengths but reverses orientation: a right-handed frame
//! `(x, y, z)` goes to the left-handed `(Rx, Ry, Rz)`, and a loop that ran counter-clockwise
//! seen from outside runs clockwise. Outward normals stay outward (`R` maps the solid's
//! outside to the mirrored solid's outside), so what has to change is the bookkeeping:
//!
//! - **Surfaces** get the right-handed frame `(Rx, -Ry, Rz)`. Every analytic surface's
//!   normal is a geometric property relative to its frame (a plane's `z`; away from the axis
//!   or centre for the others), so the mirrored surface's normal is `R` of the original's and
//!   each face keeps its sense. The surface parameters change (`(u, v)` to `(u, -v)` on a
//!   plane, `(-u, v)` on the others); nothing stores them.
//! - **Curves** get the frame `(Rx, Ry, -Rz)`, also right-handed, so `c'(t) = R c(t)` for
//!   the same `t`: circles, arcs and ellipses keep their parameter ranges and every edge
//!   keeps its `t0..t1`.
//! - **Loops** are reversed. A half-edge that ran from `a` to `b` now runs from `b` to `a`:
//!   its origin becomes the old origin of its `next`, and `next` and `prev` swap. An edge's
//!   two halves swap too, so `halves[0]` still runs along the (unchanged) curve direction.
//!   A vertex's half-edge becomes the old `prev` of its old one, which now starts there.
//!
//! This is the usual rule for a reflection of a boundary model (Mantyla 1988, ch. 10, on
//! face orientation): mirror the geometry, then invert every face's loops.

use super::{Body, Curve, HalfEdgeId, VertexId};
use crate::curve3::{Arc3, Circle3, Ellipse3, EllipticalArc3, Line3, Segment3};
use crate::surface::Surface;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, Point3, Transform, Vec3};

fn out_of_range() -> KernelError {
    KernelError::InvalidInput("the move leaves the working range".into())
}

/// The frame of a moved surface: `(Rx, ±Ry, Rz)`, right-handed whatever `t`'s handedness.
fn surface_frame(f: &Frame, t: &Transform) -> KernelResult<Frame> {
    Frame::from_normal_and_x(
        t.apply_point(f.origin()),
        t.apply_vector(f.z()),
        t.apply_vector(f.x()),
    )
    .ok_or_else(out_of_range)
}

/// The frame of a moved curve: `(Rx, Ry, ±Rz)`, right-handed, so the curve's parameter
/// is unchanged (`c'(t) = R c(t)`) under a mirror too.
fn curve_frame(f: &Frame, t: &Transform, mirror: bool) -> KernelResult<Frame> {
    let z = t.apply_vector(f.z());
    Frame::from_normal_and_x(
        t.apply_point(f.origin()),
        if mirror { -z } else { z },
        t.apply_vector(f.x()),
    )
    .ok_or_else(out_of_range)
}

fn curve(c: &Curve, t: &Transform, mirror: bool) -> KernelResult<Curve> {
    let frame = |f: &Frame| curve_frame(f, t, mirror);
    Ok(match c {
        Curve::Line(l) => Curve::Line(Line3::new(
            t.apply_point(l.origin()),
            t.apply_vector(l.direction()),
        )?),
        Curve::Segment(s) => Curve::Segment(Segment3::new(
            t.apply_point(s.start()),
            t.apply_point(s.end()),
        )?),
        Curve::Circle(c) => Curve::Circle(Circle3::new(frame(c.frame())?, c.radius())?),
        Curve::Arc(a) => Curve::Arc(Arc3::new(
            frame(a.circle().frame())?,
            a.circle().radius(),
            a.start_angle(),
            a.sweep(),
        )?),
        Curve::Ellipse(e) => Curve::Ellipse(Ellipse3::new(frame(e.frame())?, e.rx(), e.ry())?),
        Curve::EllipticalArc(a) => {
            let e = a.ellipse();
            let (start, end) = a.range();
            Curve::EllipticalArc(EllipticalArc3::new(
                frame(e.frame())?,
                e.rx(),
                e.ry(),
                start,
                end - start,
            )?)
        }
        Curve::BSpline(c) => Curve::BSpline(c.transformed(t)?),
    })
}

fn surface(s: &Surface, t: &Transform, mirror: bool) -> KernelResult<Surface> {
    let frame = |f: &Frame| surface_frame(f, t);
    Ok(match s {
        Surface::Plane(f) => Surface::Plane(frame(f)?),
        Surface::Cylinder(c) => Surface::cylinder(frame(c.frame())?, c.radius())?,
        Surface::Cone(c) => Surface::cone(frame(c.frame())?, c.radius(), c.half_angle())?,
        Surface::Sphere(c) => Surface::sphere(frame(c.frame())?, c.radius())?,
        Surface::Torus(c) => Surface::torus(frame(c.frame())?, c.major_radius(), c.minor_radius())?,
        // The curve is in the frame's plane coordinates, so the frame must be `(Rx, Ry, ±Rz)`
        // like a curve's: with `(Rx, -Ry, Rz)` a mirror would need the curve reflected too.
        // The extrusion direction flips under a mirror, and with it the sign of
        // `S_u x S_v`: `R S_u x (-R z) = R (S_u x z)` for `det R = -1`, so the normal is `R`
        // of the original's and the face keeps its sense.
        Surface::Extrusion(e) => Surface::Extrusion(crate::Extrusion::new(
            curve_frame(e.frame(), t, mirror)?,
            e.curve().clone(),
        )?),
    })
}

/// A half-edge's links after its loop is reversed: (origin, next, prev).
type Reversed = (VertexId, HalfEdgeId, HalfEdgeId);

impl Body {
    /// A copy of the body moved by the isometry `t`: a rigid motion, or a mirror (an
    /// orthogonal map with determinant -1), which also reverses every loop. Scaling and
    /// shearing are refused. The copy is checked like any new body.
    pub fn transformed(&self, t: &Transform) -> KernelResult<Body> {
        let mirror = t.is_improper_rigid();
        if !mirror && !t.is_rigid() {
            return Err(KernelError::Unsupported(
                "only moves, rotations and mirrors: a body cannot be scaled or sheared".into(),
            ));
        }
        let mut out = self.clone();
        for p in out.points.items.iter_mut().flatten() {
            let q = t.apply_point(*p);
            if !q.is_valid() {
                return Err(out_of_range());
            }
            *p = q;
        }
        for c in out.curves.items.iter_mut().flatten() {
            *c = curve(c, t, mirror)?;
        }
        for s in out.surfaces.items.iter_mut().flatten() {
            *s = surface(s, t, mirror)?;
        }
        if mirror {
            out.reverse_loops()?;
        }
        out.validate()
            .map_err(|e| KernelError::InvalidResult(format!("the moved body: {e}")))?;
        Ok(out)
    }

    /// The mirror image of the body in the plane through `point` with normal `normal`.
    /// Its volume and area equal the original's.
    pub fn mirrored(&self, point: Point3, normal: Vec3) -> KernelResult<Body> {
        let t = Transform::reflection(point, normal).ok_or_else(|| {
            KernelError::InvalidInput(
                "a mirror plane needs a finite point and a non-zero normal".into(),
            )
        })?;
        self.transformed(&t)
    }

    /// Reverses every loop (see the module documentation): each half-edge runs the other
    /// way, each edge's halves swap and each vertex gets a half-edge that starts at it.
    /// All new links are read from the old structure first, so nothing is half-changed.
    fn reverse_loops(&mut self) -> KernelResult<()> {
        let mut new: Vec<Option<Reversed>> = vec![None; self.half_edges.slots()];
        for (i, h) in self.half_edges.live() {
            let next = self.he(h.next)?;
            if let Some(slot) = new.get_mut(i) {
                *slot = Some((next.origin, h.prev, h.next));
            }
        }
        let mut vertex_he = Vec::new();
        for (i, v) in self.vertices.live() {
            vertex_he.push((i, self.he(v.half_edge)?.prev));
        }
        for (h, links) in self.half_edges.items.iter_mut().zip(new) {
            if let (Some(h), Some((origin, next, prev))) = (h.as_mut(), links) {
                h.origin = origin;
                h.next = next;
                h.prev = prev;
            }
        }
        for e in self.edges.items.iter_mut().flatten() {
            e.halves.swap(0, 1);
        }
        for (i, he) in vertex_he {
            if let Some(v) = self.vertices.get_mut(i) {
                v.half_edge = he;
            }
        }
        Ok(())
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
    fn scaling_a_body_is_refused_but_a_point_reflection_is_a_mirror() {
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
        assert!(b.transformed(&Transform::scale(2.0).unwrap()).is_err());
        // Scale by -1 is orthogonal with determinant -1: a mirror combined with a half turn.
        let flipped = b.transformed(&Transform::scale(-1.0).unwrap()).unwrap();
        assert!((flipped.volume().unwrap() - 1.0).abs() < 1e-9);
        let bb = flipped.bounding_box().unwrap();
        assert!(bb.min.distance(Point3::new(-1.0, -1.0, -1.0)) < 1e-9);
    }

    #[test]
    fn a_box_mirrored_in_the_yz_plane_lands_on_the_other_side_with_the_same_volume() {
        // Box x 0..50, y 0..30, z 0..10: volume 15000, area 2(1500 + 500 + 300) = 4600.
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(50.0, 30.0, 10.0)).unwrap();
        let m = b.mirrored(Point3::ORIGIN, Vec3::X).unwrap();
        m.validate().unwrap();
        assert!((m.volume().unwrap() - 15_000.0).abs() < 1e-6);
        assert!((m.area().unwrap() - 4_600.0).abs() < 1e-6);
        let bb = m.bounding_box().unwrap();
        assert!((bb.min.x + 50.0).abs() < 1e-9 && bb.max.x.abs() < 1e-9);
        assert!(bb.min.y.abs() < 1e-9 && (bb.max.y - 30.0).abs() < 1e-9);
        // Every flat face's outward normal points away from the mirrored box's centre
        // (-25, 15, 5); each face's frame origin is one of its corners.
        let c = Point3::new(-25.0, 15.0, 5.0);
        for f in m.faces() {
            if let Some((crate::Surface::Plane(frame), same)) = m.face_surface(f).unwrap() {
                let n = if same { frame.z() } else { -frame.z() };
                assert!((frame.origin() - c).dot(n) > 0.0, "{f}");
            }
        }
        let s = m.summary();
        // Euler-Poincare for one closed shell: V - E + F = 2.
        assert_eq!(s.vertices as i64 - s.edges as i64 + s.faces as i64, 2);
    }

    #[test]
    fn mirroring_twice_gives_the_original_back() {
        // A 4 x 5 x 6 box: volume 120.
        let b = Body::cuboid(Point3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)).unwrap();
        let (p, n) = (Point3::new(9.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.5));
        let back = b.mirrored(p, n).unwrap().mirrored(p, n).unwrap();
        let (b0, b1) = (b.bounding_box().unwrap(), back.bounding_box().unwrap());
        assert!(b0.min.distance(b1.min) < 1e-9 && b0.max.distance(b1.max) < 1e-9);
        assert!((back.volume().unwrap() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn a_plate_with_a_hole_mirrors_its_cylinder_and_circles() {
        // 50 x 30 x 10 plate less a r = 5 hole through it: 15000 - 250 pi.
        let profile = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(50.0, 0.0),
            Point2::new(50.0, 30.0),
            Point2::new(0.0, 30.0),
        ])
        .with_hole(vec![crate::ProfileEdge::circle(
            Point2::new(20.0, 15.0),
            5.0,
        )]);
        let b: Body = crate::extrude(&Frame::XY, &profile, 10.0).unwrap();
        let want = 15_000.0 - 250.0 * std::f64::consts::PI;
        for (p, n) in [
            (Point3::ORIGIN, Vec3::X),
            (Point3::new(0.0, 0.0, -2.0), Vec3::Z),
            (Point3::new(3.0, -1.0, 4.0), Vec3::new(1.0, -2.0, 3.0)),
        ] {
            let m = b.mirrored(p, n).unwrap();
            assert!((m.volume().unwrap() - want).abs() < 1e-6, "{n:?}");
            assert!(
                (m.area().unwrap() - b.area().unwrap()).abs() < 1e-6,
                "{n:?}"
            );
            crate::tessellate(&m, &crate::TessOptions::default()).unwrap();
        }
        // About the YZ plane, x 0..50 becomes x -50..0.
        let m = b.mirrored(Point3::ORIGIN, Vec3::X).unwrap();
        let bb = m.bounding_box().unwrap();
        assert!((bb.min.x + 50.0).abs() < 1e-9 && bb.max.x.abs() < 1e-9);
    }

    #[test]
    fn a_revolved_body_mirrors_with_its_curved_faces_outward() {
        // A rectangle 0..5 x 0..10 in the XZ plane turned a full circle about the z axis:
        // a cylinder r = 5, h = 10, volume 250 pi.
        let profile = Profile::polygon(&[
            Point2::new(0.0, 0.0),
            Point2::new(5.0, 0.0),
            Point2::new(5.0, 10.0),
            Point2::new(0.0, 10.0),
        ]);
        let axis =
            crate::Line2::new(Point2::new(0.0, 0.0), chamfera_geom::Vec2::new(0.0, 1.0)).unwrap();
        let b = crate::revolve(&Frame::XZ, &profile, &axis, std::f64::consts::TAU).unwrap();
        let m = b.mirrored(Point3::new(0.0, 0.0, 20.0), Vec3::Z).unwrap();
        assert!((m.volume().unwrap() - 250.0 * std::f64::consts::PI).abs() < 1e-6);
        let bb = m.bounding_box().unwrap();
        // z 0..10 mirrored in the plane z = 20 is z 30..40.
        assert!((bb.min.z - 30.0).abs() < 1e-6 && (bb.max.z - 40.0).abs() < 1e-6);
        crate::tessellate(&m, &crate::TessOptions::default()).unwrap();
    }

    #[test]
    fn a_bad_mirror_plane_is_refused() {
        let b = Body::cuboid(Point3::ORIGIN, Vec3::new(1.0, 1.0, 1.0)).unwrap();
        assert!(b.mirrored(Point3::ORIGIN, Vec3::ZERO).is_err());
        assert!(
            b.mirrored(Point3::new(f64::NAN, 0.0, 0.0), Vec3::X)
                .is_err()
        );
        assert!(b.mirrored(Point3::new(1e300, 0.0, 0.0), Vec3::X).is_err());
    }
}

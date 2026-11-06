//! Blends of a circular edge between a plane and a cylinder whose axis is perpendicular
//! to it: the closed rim of a hole or of a boss, or an arc of such a rim (the rounded end
//! of an extruded slot or of a rounded rectangle).
//!
//! The corner between the plane and the cylinder is a right angle in every meridian
//! half-plane, so the setback is the blend size `s` on both faces. In the meridian
//! half-plane through a point `V` of the edge, with `w_P` the direction into the plane
//! face (toward or away from the axis) and `w_C` the direction into the cylinder face
//! (along the axis), the new plane circle passes through `V + s w_P` and the new cylinder
//! circle through `V + s w_C`. A fillet's ball centre circle passes through
//! `V + s (w_P + w_C)`: the fillet is the [`Torus`] about the axis with that circle as its
//! tube centre and minor radius `s` (the rolling ball of Choi and Ju, 1989, about a
//! circle). A chamfer is the [`Cone`] through the two new circles (half-angle ±45°).
//!
//! The section of the blend by a meridian plane is a quarter circle about the tube centre
//! (fillet) or a segment (chamfer). That is the band's seam on a closed rim (in the
//! meridian half-plane through the rim's vertex, which the surface's own seam `u = 0`
//! follows), and the end curve of an arc's blend at each end: where the arc continues
//! tangentially into another blended edge (whose blend has the same section there), or
//! ends on a planar face through the axis.

use super::Kind;
use super::build::{
    Conic, FacePlane, VKey, edge_label, face_plane, internal, too_large, unsupported,
};
use crate::{
    Body, Circle3, Cone, Curve, EdgeId, FaceId, HalfEdgeId, KernelResult, Segment3, Surface, Torus,
    VertexId,
};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::TAU;

/// Largest sine of the angle between the cylinder's axis and the plane's normal for the
/// rim to count as perpendicular. The torus and cone are exact only for a perpendicular
/// rim; this is the same "parallel within rounding" band K6 uses for directions.
pub(super) const AXIS_PARALLEL_TOLERANCE: f64 = 1e-9;

/// A circular rim, or an arc of one, between a plane and a perpendicular cylinder.
#[derive(Debug, Clone, Copy)]
pub(in crate::blend) struct RimBlend {
    /// `faces[0]` holds the edge's first half-edge.
    pub faces: [FaceId; 2],
    /// The edge's end vertices (equal for a closed rim).
    pub ends: [VertexId; 2],
    pub convex: bool,
    kind: Kind,
    size: f64,
    pub plane_face: FaceId,
    pub cylinder_face: FaceId,
    /// The circle's centre (on the axis, in the plane).
    center: Point3,
    /// The normal of the edge's circle frame (keeps the edge's sense of rotation).
    circle_normal: Vec3,
    /// From the axis toward the first vertex, and toward the middle of an arc.
    radial: Vec3,
    mid_radial: Vec3,
    /// `+1` when the direction into the plane face points away from the axis (a hole's
    /// rim), `-1` when it points toward it (a boss).
    plane_sign: f64,
    /// Into the cylinder face (along the axis).
    into_cylinder: Vec3,
    /// The plane's outward normal.
    plane_normal: Vec3,
    cylinder_radius: f64,
    /// The radius of the new circle on the plane.
    plane_radius: f64,
    cylinder_same_sense: bool,
    vertex_point: Point3,
}

impl RimBlend {
    pub(in crate::blend) fn analyse(
        body: &Body,
        e: EdgeId,
        kind: Kind,
        size: f64,
    ) -> KernelResult<RimBlend> {
        let label = edge_label(body, e)?;
        let faces = body.edge_faces(e)?;
        let ends = body.edge_vertices(e)?;
        let Some((curve, t0, t1)) = body.edge_curve(e)? else {
            return Err(internal(format!("{label} has no curve")));
        };
        let circle = match curve {
            Curve::Circle(c) => c,
            Curve::Arc(a) => *a.circle(),
            _ => return Err(internal("a rim is not circular")),
        };
        let mut plane: Option<(FaceId, FacePlane)> = None;
        let mut cylinder = None;
        for f in faces {
            if let Some(p) = face_plane(body, f)? {
                plane = Some((f, p));
            } else if let Some((Surface::Cylinder(c), same)) = body.face_surface(f)? {
                cylinder = Some((f, c, same));
            }
        }
        let (Some((plane_face, pl)), Some((cylinder_face, cyl, cylinder_same_sense))) =
            (plane, cylinder)
        else {
            return Err(unsupported(format!(
                "{label}: a circular edge can be blended only between a plane and a cylinder so far; the rolling-ball surface along other circular edges is not a torus"
            )));
        };
        let axis = cyl.frame().z();
        if axis.cross(pl.normal).length() > AXIS_PARALLEL_TOLERANCE {
            return Err(unsupported(format!(
                "{label}: the cylinder's axis is not perpendicular to the plane"
            )));
        }
        let center = circle.center();
        let w = center - cyl.frame().origin();
        if (w - axis * w.dot(axis)).length() > LINEAR_TOLERANCE
            || (circle.radius() - cyl.radius()).abs() > LINEAR_TOLERANCE
        {
            return Err(unsupported(format!(
                "{label} is not the cylinder's section by the plane"
            )));
        }
        let vertex_point = body.vertex_point(ends[0])?;
        let radial = (vertex_point - center)
            .normalized()
            .ok_or_else(|| internal("a rim's vertex is at its centre"))?;
        let mid_radial = (curve.point_at(0.5 * (t0 + t1)) - center)
            .normalized()
            .ok_or_else(|| internal("a rim's middle is at its centre"))?;
        let along = curve
            .d1(t0)
            .normalized()
            .ok_or_else(|| internal("a rim has no tangent"))?;
        let cyl_normal = if cylinder_same_sense { radial } else { -radial };
        // A face lies on the left of its half-edge seen from outside: into it is n × t.
        let tangent_in = |f: FaceId| if f == faces[0] { along } else { -along };
        let into = |n: Vec3, f: FaceId| {
            n.cross(tangent_in(f))
                .normalized()
                .ok_or_else(|| internal("a rim face has no inward direction"))
        };
        let into_plane = into(pl.normal, plane_face)?;
        let into_cylinder = into(cyl_normal, cylinder_face)?;
        let convex = into_plane.dot(cyl_normal) < 0.0;
        let plane_sign = into_plane.dot(radial).signum();
        let plane_radius = circle.radius() + size * plane_sign;
        if plane_radius <= LINEAR_TOLERANCE {
            return Err(too_large(
                kind,
                format!("it would consume the face inside {label}"),
            ));
        }
        Ok(RimBlend {
            faces,
            ends,
            convex,
            kind,
            size,
            plane_face,
            cylinder_face,
            center,
            circle_normal: circle.frame().z(),
            radial,
            mid_radial,
            plane_sign,
            into_cylinder,
            plane_normal: pl.normal,
            cylinder_radius: circle.radius(),
            plane_radius,
            cylinder_same_sense,
            vertex_point,
        })
    }

    /// True for a whole circle (one vertex), false for an arc.
    pub(in crate::blend) fn closed(&self) -> bool {
        self.ends[0] == self.ends[1]
    }

    /// The axis direction (the plane's outward normal).
    pub(in crate::blend) fn axis(&self) -> Vec3 {
        self.plane_normal
    }

    /// The point on the axis level with the plane.
    pub(in crate::blend) fn center(&self) -> Point3 {
        self.center
    }

    /// The unit direction from the axis toward the edge point `p`.
    fn radial_at(&self, p: Point3) -> KernelResult<Vec3> {
        let w = p - self.center;
        (w - self.plane_normal * w.dot(self.plane_normal))
            .normalized()
            .ok_or_else(|| internal("a rim point is on its axis"))
    }

    /// The directions into the plane face and into the cylinder face at edge point `p`.
    fn directions_at(&self, p: Point3) -> KernelResult<(Vec3, Vec3)> {
        Ok((self.radial_at(p)? * self.plane_sign, self.into_cylinder))
    }

    /// The new point of face `f` (the plane or the cylinder) level with edge point `p`.
    pub(in crate::blend) fn contact_point(&self, f: FaceId, p: Point3) -> KernelResult<Point3> {
        let (wp, wc) = self.directions_at(p)?;
        Ok(if f == self.plane_face {
            p + wp * self.size
        } else {
            p + wc * self.size
        })
    }

    /// The ball centre (fillet) level with edge point `p`: on the torus's tube circle.
    pub(in crate::blend) fn ball_at(&self, p: Point3) -> KernelResult<Point3> {
        let (wp, wc) = self.directions_at(p)?;
        Ok(p + (wp + wc) * self.size)
    }

    /// The whole contact circle on face `f`.
    pub(in crate::blend) fn contact_circle(&self, f: FaceId) -> KernelResult<Circle3> {
        let (center, radius) = if f == self.plane_face {
            (self.center, self.plane_radius)
        } else {
            (
                self.center + self.into_cylinder * self.size,
                self.cylinder_radius,
            )
        };
        let frame = Frame::from_normal_and_x(center, self.circle_normal, self.radial)
            .ok_or_else(|| internal("a rim contact circle is degenerate"))?;
        Circle3::new(frame, radius)
    }

    /// The new points of the face corners at a closed rim's vertex: the half-edges leaving
    /// it (the plane's circle, the cylinder's circle and the cylinder's seam), each with
    /// its corner, key and point.
    pub(in crate::blend) fn corners(
        &self,
        body: &Body,
        fan: &[HalfEdgeId],
    ) -> KernelResult<Vec<(HalfEdgeId, super::build::Corner, VKey, Point3)>> {
        let v = self.ends[0];
        let edges = body.vertex_edges(v)?;
        let seam = edges.iter().copied().find(|e| {
            body.edge_faces(*e)
                .is_ok_and(|[a, b]| a == b && a == self.cylinder_face)
        });
        let Some(seam) = seam.filter(|_| edges.len() == 2 && fan.len() == 3) else {
            return Err(unsupported(format!(
                "the rim vertex of {} must have only the rim and the cylinder's seam",
                body.vertex_name(v)?.unwrap_or("a rim")
            )));
        };
        if !matches!(
            body.edge_curve(seam)?,
            Some((Curve::Line(_) | Curve::Segment(_), _, _))
        ) {
            return Err(unsupported("a rim's cylinder seam must be straight"));
        }
        let on_plane = (
            VKey::Corner(self.plane_face, v),
            self.contact_point(self.plane_face, self.vertex_point)?,
        );
        let on_cylinder = (
            VKey::OnEdge(seam, v),
            self.contact_point(self.cylinder_face, self.vertex_point)?,
        );
        let mut out = Vec::with_capacity(3);
        for h in fan {
            let f = body.loop_face(body.half_edge_loop(*h)?)?;
            let (key, p) = if f == self.plane_face {
                on_plane
            } else {
                on_cylinder
            };
            out.push((*h, super::build::Corner::One(key), key, p));
        }
        Ok(out)
    }

    /// The contact edge on face `f` from `start` to `end`, turning the same way as the
    /// edge (a whole circle when they coincide).
    pub(in crate::blend) fn contact_curve(
        &self,
        f: FaceId,
        start: Point3,
        end: Point3,
    ) -> KernelResult<(Curve, f64, f64)> {
        let c = self.contact_circle(f)?;
        let center = c.center();
        let frame = Frame::from_normal_and_x(center, self.circle_normal, start - center)
            .ok_or_else(|| internal("a rim contact circle is degenerate"))?;
        let circle = Circle3::new(frame, c.radius())?;
        if self.closed() {
            return Ok((Curve::Circle(circle), 0.0, TAU));
        }
        let l = frame.to_local(end);
        let mut sweep = l.y.atan2(l.x);
        if sweep <= 0.0 {
            sweep += TAU;
        }
        if !circle.point_at(sweep).coincident(end) {
            return Err(internal("a rim contact arc misses its end"));
        }
        Ok((Curve::Circle(circle), 0.0, sweep))
    }

    /// The meridian section of the blend through edge point `vp`, between its corner `x`
    /// (on the edge's first face) and `y`: a closed rim's seam, or an arc's end curve.
    pub(in crate::blend) fn section_curve(
        &self,
        vp: Point3,
        x: Point3,
        y: Point3,
    ) -> KernelResult<(Curve, f64, f64, bool)> {
        if self.kind == Kind::Chamfer {
            return Ok((Curve::Segment(Segment3::new(x, y)?), 0.0, 1.0, true));
        }
        let s = self.size;
        let tube = self.ball_at(vp)?;
        let frame = Frame::from_normal_and_x(tube, (x - tube).cross(y - tube), x - tube)
            .ok_or_else(|| internal("a rim fillet's seam is degenerate"))?;
        Conic {
            frame,
            rx: s,
            ry: s,
        }
        .arc_between(x, y, vp)
    }

    /// The band's surface and whether the face's outward normal is the surface normal.
    /// A closed band's surface seam (`u = 0`) runs through the rim's vertex; an arc's lies
    /// opposite the arc's middle, so the face never crosses it.
    pub(in crate::blend) fn surface(&self) -> KernelResult<(Surface, bool)> {
        let s = self.size;
        let seam = if self.closed() {
            self.radial
        } else {
            -self.mid_radial
        };
        match self.kind {
            Kind::Fillet => {
                let origin = self.center + self.into_cylinder * s;
                let frame = Frame::from_normal_and_x(origin, self.plane_normal, seam)
                    .ok_or_else(|| internal("a rim fillet's frame is degenerate"))?;
                Ok((
                    Surface::Torus(Torus::new(frame, self.plane_radius, s)?),
                    self.convex,
                ))
            }
            Kind::Chamfer => {
                let frame = Frame::from_normal_and_x(self.center, self.into_cylinder, seam)
                    .ok_or_else(|| internal("a rim chamfer's frame is degenerate"))?;
                let half_angle = (self.cylinder_radius - self.plane_radius).atan2(s);
                Ok((
                    Surface::Cone(Cone::new(frame, self.plane_radius, half_angle)?),
                    self.cylinder_same_sense,
                ))
            }
        }
    }
}

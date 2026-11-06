//! Blends of a circular rim: the closed circle edge between a plane and a cylinder whose
//! axis is perpendicular to it (the rim of a hole or of a boss).
//!
//! The corner between the plane and the cylinder is a right angle in every meridian
//! half-plane, so the setback is the blend size `s` on both faces. In the meridian
//! half-plane through the rim's vertex, with `w_P` the direction into the plane face
//! (toward or away from the axis) and `w_C` the direction into the cylinder face (along
//! the axis), the new plane circle passes through `V + s w_P` and the new cylinder circle
//! through `V + s w_C`. A fillet's ball centre circle passes through `V + s (w_P + w_C)`:
//! the fillet is the [`Torus`] about the axis with that circle as its tube centre and
//! minor radius `s`. A chamfer is the [`Cone`] through the two new circles (half-angle
//! ±45°). The band gets a seam edge in the meridian half-plane through the vertex (a
//! quarter circle about the tube centre, or a segment), which the surface's own seam
//! (`u = 0`, the frame's x axis through the vertex) follows.

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
const AXIS_PARALLEL_TOLERANCE: f64 = 1e-9;

/// A circular rim between a plane and a perpendicular cylinder.
#[derive(Debug, Clone, Copy)]
pub(in crate::blend) struct RimBlend {
    /// `faces[0]` holds the edge's first half-edge.
    pub faces: [FaceId; 2],
    pub vertex: VertexId,
    pub convex: bool,
    kind: Kind,
    size: f64,
    plane_face: FaceId,
    cylinder_face: FaceId,
    /// The circle's centre (on the axis, in the plane).
    center: Point3,
    /// The normal of the edge's circle frame (keeps the edge's sense of rotation).
    circle_normal: Vec3,
    /// From the axis toward the vertex.
    radial: Vec3,
    /// Into the plane face and into the cylinder face, at the vertex.
    into_plane: Vec3,
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
        let [v0, v1] = body.edge_vertices(e)?;
        if v0 != v1 {
            return Err(unsupported(format!(
                "{label} is an arc, not a closed rim; only full circles between a plane and a cylinder can be blended so far"
            )));
        }
        let Some((curve, t0, _)) = body.edge_curve(e)? else {
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
                "{label}: a curved edge can be blended only between a plane and a cylinder so far"
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
        let vertex_point = body.vertex_point(v0)?;
        let radial = (vertex_point - center)
            .normalized()
            .ok_or_else(|| internal("a rim's vertex is at its centre"))?;
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
        let plane_radius = circle.radius() + size * into_plane.dot(radial).signum();
        if plane_radius <= LINEAR_TOLERANCE {
            return Err(too_large(
                kind,
                format!("it would consume the face inside {label}"),
            ));
        }
        Ok(RimBlend {
            faces,
            vertex: v0,
            convex,
            kind,
            size,
            plane_face,
            cylinder_face,
            center,
            circle_normal: circle.frame().z(),
            radial,
            into_plane,
            into_cylinder,
            plane_normal: pl.normal,
            cylinder_radius: circle.radius(),
            plane_radius,
            cylinder_same_sense,
            vertex_point,
        })
    }

    /// The new points of the face corners at the rim's vertex: the half-edges leaving it
    /// (the plane's circle, the cylinder's circle and the cylinder's seam), each with its
    /// corner, key and point.
    pub(in crate::blend) fn corners(
        &self,
        body: &Body,
        fan: &[HalfEdgeId],
    ) -> KernelResult<Vec<(HalfEdgeId, super::build::Corner, VKey, Point3)>> {
        let v = self.vertex;
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
        let s = self.size;
        let on_plane = (
            VKey::Corner(self.plane_face, v),
            self.vertex_point + self.into_plane * s,
        );
        let on_cylinder = (
            VKey::OnEdge(seam, v),
            self.vertex_point + self.into_cylinder * s,
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

    /// The contact circle on face `f`, starting and ending at `start`, turning the same
    /// way as the rim.
    pub(in crate::blend) fn contact_curve(
        &self,
        f: FaceId,
        start: Point3,
    ) -> KernelResult<(Curve, f64, f64)> {
        let (center, radius) = if f == self.plane_face {
            (self.center, self.plane_radius)
        } else {
            (
                self.center + self.into_cylinder * self.size,
                self.cylinder_radius,
            )
        };
        let frame = Frame::from_normal_and_x(center, self.circle_normal, start - center)
            .ok_or_else(|| internal("a rim contact circle is degenerate"))?;
        Ok((Curve::Circle(Circle3::new(frame, radius)?), 0.0, TAU))
    }

    /// The band's seam between its corner `x` (on the edge's first face) and `y`.
    pub(in crate::blend) fn seam_curve(
        &self,
        x: Point3,
        y: Point3,
    ) -> KernelResult<(Curve, f64, f64, bool)> {
        if self.kind == Kind::Chamfer {
            return Ok((Curve::Segment(Segment3::new(x, y)?), 0.0, 1.0, true));
        }
        let s = self.size;
        let tube = self.vertex_point + (self.into_plane + self.into_cylinder) * s;
        let frame = Frame::from_normal_and_x(tube, (x - tube).cross(y - tube), x - tube)
            .ok_or_else(|| internal("a rim fillet's seam is degenerate"))?;
        Conic {
            frame,
            rx: s,
            ry: s,
        }
        .arc_between(x, y, self.vertex_point)
    }

    /// The band's surface and whether the face's outward normal is the surface normal.
    pub(in crate::blend) fn surface(&self) -> KernelResult<(Surface, bool)> {
        let s = self.size;
        match self.kind {
            Kind::Fillet => {
                let origin = self.center + self.into_cylinder * s;
                let frame = Frame::from_normal_and_x(origin, self.plane_normal, self.radial)
                    .ok_or_else(|| internal("a rim fillet's frame is degenerate"))?;
                Ok((
                    Surface::Torus(Torus::new(frame, self.plane_radius, s)?),
                    self.convex,
                ))
            }
            Kind::Chamfer => {
                let frame = Frame::from_normal_and_x(self.center, self.into_cylinder, self.radial)
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

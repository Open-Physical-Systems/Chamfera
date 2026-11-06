//! The outline of a flat pattern: the edges of its face on the base's sketch plane, in
//! that plane's coordinates.

use crate::topology::{Body, Curve};
use crate::{KernelError, KernelResult};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2};
use std::collections::BTreeSet;
use std::f64::consts::TAU;

/// One edge of a flat outline, in the plane coordinates of the sheet's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlatEdge {
    Line {
        a: Point2,
        b: Point2,
    },
    /// Counter-clockwise from `start_angle` to `end_angle` (radians).
    Arc {
        centre: Point2,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    Circle {
        centre: Point2,
        radius: f64,
    },
}

fn angle_of(c: Point2, p: Point2) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

/// `a` turned into `[0, 2π)`.
fn turn(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r.is_finite() { r } else { 0.0 }
}

/// Every edge of the faces of `flat` that lie on `frame`'s plane (facing either way), in
/// that plane's coordinates. Fails if no face lies there or an edge is not a line or an
/// arc in the plane.
pub fn flat_outline(flat: &Body, frame: &Frame) -> KernelResult<Vec<FlatEdge>> {
    let mut edges = BTreeSet::new();
    let mut any = false;
    for f in flat.faces() {
        let Some((crate::Surface::Plane(pf), _)) = flat.face_surface(f)? else {
            continue;
        };
        let on = frame.to_local(pf.origin()).z.abs() <= LINEAR_TOLERANCE
            && (pf.z().dot(frame.z()).abs() - 1.0).abs() <= super::SQUARE_COS_TOLERANCE;
        if !on {
            continue;
        }
        any = true;
        for e in flat.face_edges(f)? {
            edges.insert(e.index());
        }
    }
    if !any {
        return Err(KernelError::InvalidInput(
            "the flat pattern has no face on the base's sketch plane".into(),
        ));
    }
    let mut out = Vec::with_capacity(edges.len());
    for e in flat.edges() {
        if !edges.contains(&e.index()) {
            continue;
        }
        let Some((curve, t0, t1)) = flat.edge_curve(e)? else {
            continue;
        };
        let p0 = frame.to_plane(curve.point_at(t0));
        let p1 = frame.to_plane(curve.point_at(t1));
        let circle = match curve {
            Curve::Line(_) | Curve::Segment(_) => {
                out.push(FlatEdge::Line { a: p0, b: p1 });
                continue;
            }
            Curve::Circle(c) => c,
            Curve::Arc(a) => *a.circle(),
            Curve::Ellipse(_) | Curve::EllipticalArc(_) => {
                return Err(KernelError::Unsupported(
                    "an ellipse in a flat pattern outline".into(),
                ));
            }
        };
        let centre = frame.to_plane(circle.frame().origin());
        let radius = circle.radius();
        if p0.distance(p1) <= LINEAR_TOLERANCE {
            out.push(FlatEdge::Circle { centre, radius });
            continue;
        }
        let pm = frame.to_plane(curve.point_at((t0 + t1) * 0.5));
        let (a0, a1, am) = (
            angle_of(centre, p0),
            angle_of(centre, p1),
            angle_of(centre, pm),
        );
        let (start_angle, end_angle) = if turn(am - a0) < turn(a1 - a0) {
            (a0, a1)
        } else {
            (a1, a0)
        };
        out.push(FlatEdge::Arc {
            centre,
            radius,
            start_angle,
            end_angle,
        });
    }
    Ok(out)
}

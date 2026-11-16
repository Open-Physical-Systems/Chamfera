//! Building primitives and scales on the timeline, the translation Align makes, and the
//! validity report of a body. Exact geometry throughout: a primitive is an extrude or a
//! full revolve of an exact profile, a scale is the kernel's `Body::scaled`, Align works
//! on the picked entities' exact points, axes and planes.

use chamfera_doc::{Document, Primitive, PrimitiveAxis, PrimitiveValues, Scale};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Point3, Vec2, Vec3};
use chamfera_kernel::{Body, Curve, Entity, Line2, Profile, ProfileEdge, Surface, Validation};
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// Bodies built so far: (feature, name, body).
type Made = Vec<(u32, String, Body)>;

/// The frame a primitive is built in: origin at its point, z along its axis.
fn frame(p: &Primitive) -> Result<Frame, String> {
    let (z, x) = match p.axis {
        PrimitiveAxis::X => (Vec3::X, Vec3::Y),
        PrimitiveAxis::Y => (Vec3::Y, Vec3::Z),
        PrimitiveAxis::Z => (Vec3::Z, Vec3::X),
    };
    Frame::from_normal_and_x(p.origin, z, x).ok_or_else(|| "the position is out of range".into())
}

/// The plane a turned primitive's half profile is drawn in: its x along the frame's x,
/// its y along the axis, so turning about the plane's y axis turns about the axis.
fn turning_plane(f: &Frame) -> Result<Frame, String> {
    Frame::from_normal_and_x(f.origin(), f.x().cross(f.z()), f.x())
        .ok_or_else(|| "the position is out of range".into())
}

fn turned(f: &Frame, profile: &Profile) -> Result<Body, String> {
    let plane = turning_plane(f)?;
    let axis = Line2::new(Point2::ORIGIN, Vec2::new(0.0, 1.0)).map_err(|e| e.to_string())?;
    chamfera_kernel::revolve(&plane, profile, &axis, TAU).map_err(|e| e.to_string())
}

/// The body a primitive makes (faces named by role, as an extrude or revolve names them).
pub(crate) fn build(doc: &Document, p: &Primitive) -> Result<Vec<Body>, String> {
    let values = doc.primitive_values(p).map_err(|e| e.to_string())?;
    let f = frame(p)?;
    let k = |e: chamfera_kernel::KernelError| format!("{}: {e}", p.shape.label());
    let body = match values {
        PrimitiveValues::Box {
            length,
            width,
            height,
        } => {
            let (a, b) = (length / 2.0, width / 2.0);
            let rect = Profile::polygon(&[
                Point2::new(-a, -b),
                Point2::new(a, -b),
                Point2::new(a, b),
                Point2::new(-a, b),
            ]);
            chamfera_kernel::extrude(&f, &rect, height).map_err(k)?
        }
        PrimitiveValues::Cylinder { radius, height } => {
            chamfera_kernel::extrude(&f, &Profile::circle(Point2::ORIGIN, radius), height)
                .map_err(k)?
        }
        // A half disc turned about its straight side.
        PrimitiveValues::Sphere { radius } => turned(
            &f,
            &Profile::new(
                vec![
                    ProfileEdge::Arc {
                        centre: Point2::ORIGIN,
                        radius,
                        start_angle: -FRAC_PI_2,
                        sweep: PI,
                    },
                    ProfileEdge::line(Point2::new(0.0, radius), Point2::new(0.0, -radius)),
                ],
                vec![],
            ),
        )
        .map_err(|e| format!("Sphere: {e}"))?,
        // A circle beside the axis turned about it.
        PrimitiveValues::Torus { major, minor } => {
            turned(&f, &Profile::circle(Point2::new(major, 0.0), minor))
                .map_err(|e| format!("Torus: {e}"))?
        }
        // The half section: base, slanted side, top (or the apex), axis.
        PrimitiveValues::Cone {
            radius,
            top,
            height,
        } => {
            let mut pts = vec![Point2::ORIGIN, Point2::new(radius, 0.0)];
            if top > LINEAR_TOLERANCE {
                pts.push(Point2::new(top, height));
            }
            pts.push(Point2::new(0.0, height));
            turned(&f, &Profile::polygon(&pts)).map_err(|e| format!("Cone: {e}"))?
        }
    };
    Ok(vec![body])
}

/// The bodies after a scale, and the names of the ones it changed. Each keeps its name
/// and the feature that made it (so its appearance and material stay with it).
pub(crate) fn scale_bodies(
    doc: &Document,
    sc: &Scale,
    made: &[(u32, String, Body)],
) -> Result<(Made, Vec<String>), String> {
    let factors = doc.scale_factors(sc).map_err(|e| e.to_string())?;
    let mut next = made.to_vec();
    let mut changed = Vec::new();
    for name in &sc.bodies {
        let slot = next
            .iter_mut()
            .find(|m| m.1 == *name)
            .ok_or_else(|| format!("there is no body named {name:?} at this point"))?;
        slot.2 = slot
            .2
            .scaled(sc.origin, factors)
            .map_err(|e| format!("scale {name}: {e}"))?;
        if !changed.contains(name) {
            changed.push(name.clone());
        }
    }
    Ok((next, changed))
}

/// What Align lines up: a point, a line (straight edge, cylinder or cone axis) or a
/// plane (flat face).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    Point(Point3),
    Line { point: Point3, dir: Vec3 },
    Plane { point: Point3, normal: Vec3 },
}

impl Anchor {
    /// A point of the anchor, where a move starts.
    pub fn point(&self) -> Point3 {
        match *self {
            Anchor::Point(p) | Anchor::Line { point: p, .. } | Anchor::Plane { point: p, .. } => p,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Anchor::Point(_) => "point",
            Anchor::Line { .. } => "line",
            Anchor::Plane { .. } => "plane",
        }
    }
}

/// The anchor of an entity of `body`: a vertex's point; a straight edge's line; a
/// circle's, arc's or ellipse's centre; a flat face's plane; a cylinder's or cone's axis;
/// a sphere's or torus's centre.
pub fn anchor(body: &Body, e: Entity) -> Result<Anchor, String> {
    let k = |x: chamfera_kernel::KernelError| x.to_string();
    Ok(match e {
        Entity::Point(p) => Anchor::Point(p),
        Entity::Vertex(v) => Anchor::Point(body.vertex_point(v).map_err(k)?),
        Entity::Edge(edge) => match body.edge_curve(edge).map_err(k)? {
            Some((Curve::Line(l), ..)) => Anchor::Line {
                point: l.origin(),
                dir: l.direction(),
            },
            Some((Curve::Segment(s), ..)) => Anchor::Line {
                point: s.start(),
                dir: s.end() - s.start(),
            },
            Some((Curve::Circle(c), ..)) => Anchor::Point(c.frame().origin()),
            Some((Curve::Arc(a), ..)) => Anchor::Point(a.circle().frame().origin()),
            Some((Curve::Ellipse(c), ..)) => Anchor::Point(c.frame().origin()),
            Some((Curve::EllipticalArc(a), ..)) => Anchor::Point(a.ellipse().frame().origin()),
            Some((c @ Curve::BSpline(_), t0, t1)) => Anchor::Point(c.point_at((t0 + t1) / 2.0)),
            None => return Err("the edge has no curve".into()),
        },
        Entity::Face(f) => match body.face_surface(f).map_err(k)? {
            Some((Surface::Plane(fr), _)) => Anchor::Plane {
                point: fr.origin(),
                normal: fr.z(),
            },
            Some((Surface::Cylinder(c), _)) => Anchor::Line {
                point: c.frame().origin(),
                dir: c.frame().z(),
            },
            Some((Surface::Cone(c), _)) => Anchor::Line {
                point: c.frame().origin(),
                dir: c.frame().z(),
            },
            Some((Surface::Sphere(c), _)) => Anchor::Point(c.frame().origin()),
            Some((Surface::Torus(c), _)) => Anchor::Point(c.frame().origin()),
            Some((Surface::Extrusion(_), _)) => {
                return Err("a curved spline face has no point, axis or plane to align".into());
            }
            None => return Err("the face has no surface".into()),
        },
        Entity::Body => {
            return Err("pick a vertex, an edge or a face, not a whole body".into());
        }
    })
}

/// The foot of `p` on the line through `a` along unit `u`.
fn foot(p: Point3, a: Point3, u: Vec3) -> Point3 {
    a + u * (p - a).dot(u)
}

fn unit(v: Vec3) -> Result<Vec3, String> {
    v.normalized()
        .ok_or_else(|| "a picked edge or face has no direction".into())
}

fn parallel(a: Vec3, b: Vec3) -> bool {
    a.cross(b).length() <= chamfera_geom::ANGULAR_TOLERANCE
}

/// The shortest translation that brings `from` onto `to` (Align moves, it never turns):
/// point to point is the vector between them; onto a line or plane, the perpendicular
/// step; a line onto a parallel line or plane, a plane onto a parallel plane, the
/// perpendicular offset between them; a line onto a skew line, the step between their
/// closest points. A plane onto a plane that is not parallel is refused.
pub fn align_translation(from: Anchor, to: Anchor) -> Result<Vec3, String> {
    use Anchor::{Line, Plane, Point as P};
    let d = match (from, to) {
        (P(a), P(b)) => b - a,
        (P(a), Line { point, dir }) => foot(a, point, unit(dir)?) - a,
        (Line { point, dir }, P(b)) => b - foot(b, point, unit(dir)?),
        (P(a), Plane { point, normal }) | (Plane { point, normal }, P(a)) => {
            let n = unit(normal)?;
            let s = (point - a).dot(n);
            // Point onto plane moves the point; plane onto point moves the plane.
            if matches!(from, P(_)) {
                n * s
            } else {
                -(n * s)
            }
        }
        (Line { point: p1, dir: d1 }, Line { point: p2, dir: d2 }) => {
            let (u1, u2) = (unit(d1)?, unit(d2)?);
            if parallel(u1, u2) {
                let w = p2 - p1;
                w - u2 * w.dot(u2)
            } else {
                // Closest points of two skew lines (Ericson 2005, 5.1.8).
                let r = p1 - p2;
                let b = u1.dot(u2);
                let (c, f) = (u1.dot(r), u2.dot(r));
                let den = 1.0 - b * b;
                let s = (b * f - c) / den;
                let t = (f - b * c) / den;
                (p2 + u2 * t) - (p1 + u1 * s)
            }
        }
        (Line { point, dir }, Plane { point: q, normal })
        | (Plane { point: q, normal }, Line { point, dir }) => {
            let (u, n) = (unit(dir)?, unit(normal)?);
            if u.dot(n).abs() > chamfera_geom::ANGULAR_TOLERANCE {
                return Err("the line crosses the plane: they already meet".into());
            }
            let s = (q - point).dot(n);
            if matches!(from, Line { .. }) {
                n * s
            } else {
                -(n * s)
            }
        }
        (
            Plane {
                point: q1,
                normal: n1,
            },
            Plane {
                point: q2,
                normal: n2,
            },
        ) => {
            let (a, b) = (unit(n1)?, unit(n2)?);
            if !parallel(a, b) {
                return Err(
                    "the faces are not parallel: Align moves without turning, so turn the \
                     body first (Move with an angle)"
                        .into(),
                );
            }
            b * (q2 - q1).dot(b)
        }
    };
    if !d.is_finite() {
        return Err("the move is out of range".into());
    }
    if d.length() <= LINEAR_TOLERANCE {
        return Err("they already line up: nothing to move".into());
    }
    Ok(d)
}

/// The validity report of one body: the kernel's full check (topology, geometry,
/// orientation, enclosed volume), with counts.
pub fn check_report(name: &str, body: &Body) -> Value {
    let surface = body.is_open();
    let mode = if surface {
        Validation::Surface
    } else {
        Validation::Solid
    };
    let s = body.summary();
    let mut out = json!({
        "name": name,
        "kind": if surface { "surface" } else { "solid" },
        "shells": s.shells,
        "faces": s.faces,
        "edges": s.edges,
        "vertices": s.vertices,
    });
    match body.validate_as(mode) {
        Ok(r) => {
            out["valid"] = json!(true);
            out["genus"] = json!(r.genus);
            out["euler"] = json!(r.euler_characteristic());
            out["error"] = Value::Null;
            if !surface {
                out["volume"] = json!(body.volume().ok());
            }
        }
        Err(e) => {
            out["valid"] = json!(false);
            out["error"] = json!(e.to_string());
        }
    }
    out
}

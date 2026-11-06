//! Building construction geometry (planes, axes, points) in a rebuild, picking what they
//! are built from, and drawing them.
//!
//! Every construction is worked out again from exact geometry each rebuild: faces are
//! found by their names among the bodies built so far ([`crate::solids::face_key`]),
//! edges as the edge two named faces share, vertices as the vertex named faces meet at.

use crate::Session;
use crate::solids::{face_key, split_face_key};
use chamfera_doc::{
    AxisRef, Construction, Document, EdgeRef, FaceRef, PlaneRef, PointRef, expr::Dim,
};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, Point3, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId, FaceId, Surface};
use chamfera_render::{Fill, FillStyle, Overlay, OverlayStyle};
use std::collections::BTreeMap;

/// The sine of the angle under which two planes, or a plane and an axis, count as
/// parallel. Normals from exact frames and from sin/cos turns carry rounding of about
/// 1e-16, far below this; planes closer than this to parallel would cross so far away
/// (about 1e9 times their offset) that the line is useless.
pub(crate) const PARALLEL_SIN: f64 = 1e-9;

/// What a construction feature built.
#[derive(Debug, Clone, PartialEq)]
pub enum Constructed {
    /// A plane; `extent` is the geometry it was made from (for its drawn size).
    Plane {
        frame: Frame,
        extent: Option<Aabb>,
    },
    /// An infinite line through `origin` along the unit `direction`; `span` is the part
    /// worth drawing, when the axis was made from something with ends.
    Axis {
        origin: Point3,
        direction: Vec3,
        span: Option<(Point3, Point3)>,
    },
    Point(Point3),
}

/// The bodies built so far in a rebuild: (feature, name, body).
pub type Made = [(u32, String, Body)];

/// A face by name among the bodies built so far.
fn find_face<'a>(made: &'a Made, r: &FaceRef) -> Result<(&'a Body, FaceId), String> {
    let wanted = face_key(r.feature, &r.face);
    for (_, _, body) in made {
        for face in body.faces() {
            if body.face_name(face).ok().flatten() == Some(wanted.as_str()) {
                return Ok((body, face));
            }
        }
    }
    Err(format!(
        "the face {:?} of feature {} is gone (that feature changed or failed)",
        r.face, r.feature
    ))
}

/// The face ids of `faces`, all in one body.
fn find_faces<'a>(made: &'a Made, faces: &[FaceRef]) -> Result<(&'a Body, Vec<FaceId>), String> {
    let first = faces.first().ok_or("no faces named")?;
    let (body, _) = find_face(made, first)?;
    let mut ids = Vec::new();
    for r in faces {
        let wanted = face_key(r.feature, &r.face);
        let id = body
            .faces()
            .into_iter()
            .find(|f| body.face_name(*f).ok().flatten() == Some(wanted.as_str()))
            .ok_or_else(|| format!("the faces named are not all on one body ({:?})", r.face))?;
        ids.push(id);
    }
    Ok((body, ids))
}

/// The edge two named faces share (the first one, if they share several).
fn find_edge<'a>(made: &'a Made, e: &EdgeRef) -> Result<(&'a Body, EdgeId), String> {
    let (body, ids) = find_faces(made, &e.faces)?;
    let (Some(a), Some(b)) = (ids.first(), ids.get(1)) else {
        return Err("an edge is named by two faces".into());
    };
    let edges = body.face_edges(*a).map_err(|x| x.to_string())?;
    for edge in edges {
        let faces = body.edge_faces(edge).map_err(|x| x.to_string())?;
        if faces.contains(b) && faces.contains(a) && a != b {
            return Ok((body, edge));
        }
    }
    Err(format!(
        "the faces {:?} and {:?} do not meet in an edge",
        e.faces[0].face, e.faces[1].face
    ))
}

/// A face's frame and the box around its edges.
fn face_extent(body: &Body, face: FaceId) -> Option<Aabb> {
    let edges = body.face_edges(face).ok()?;
    let mut points = Vec::new();
    for e in edges {
        if let Ok(Some((c, t0, t1))) = body.edge_curve(e) {
            for k in 0..=8 {
                points.push(c.point_at(t0 + (t1 - t0) * f64::from(k) / 8.0));
            }
        }
    }
    Aabb::from_points(points.into_iter().filter(|p| p.is_valid()))
}

pub(crate) fn plane_of(
    made: &Made,
    built: &BTreeMap<u32, Constructed>,
    r: &PlaneRef,
) -> Result<(Frame, Option<Aabb>), String> {
    match r {
        PlaneRef::Origin { plane } => Ok((plane.frame(), None)),
        PlaneRef::Face(f) => {
            let (body, face) = find_face(made, f)?;
            let frame = crate::solids::face_plane(body, face)
                .ok_or_else(|| format!("the face {:?} is not flat", f.face))?;
            Ok((frame, face_extent(body, face)))
        }
        PlaneRef::Construct { feature } => match built.get(feature) {
            Some(Constructed::Plane { frame, extent }) => Ok((*frame, *extent)),
            _ => Err(format!(
                "plane {feature} it is built on is suppressed or could not be built"
            )),
        },
    }
}

/// An axis: a point on it, its unit direction and a span to draw.
type Axis = (Point3, Vec3, Option<(Point3, Point3)>);

fn axis_of(made: &Made, built: &BTreeMap<u32, Constructed>, r: &AxisRef) -> Result<Axis, String> {
    match r {
        AxisRef::Origin { axis } => Ok((Point3::ORIGIN, axis.direction(), None)),
        AxisRef::Edge(e) => edge_axis(made, e),
        AxisRef::Construct { feature } => match built.get(feature) {
            Some(Constructed::Axis {
                origin,
                direction,
                span,
            }) => Ok((*origin, *direction, *span)),
            _ => Err(format!(
                "axis {feature} it is built on is suppressed or could not be built"
            )),
        },
    }
}

fn edge_axis(made: &Made, e: &EdgeRef) -> Result<Axis, String> {
    let (body, edge) = find_edge(made, e)?;
    match body.edge_curve(edge).map_err(|x| x.to_string())? {
        Some((Curve::Line(_) | Curve::Segment(_), _, _)) => {}
        _ => return Err("the edge is not straight".into()),
    }
    let [va, vb] = body.edge_vertices(edge).map_err(|x| x.to_string())?;
    let a = body.vertex_point(va).map_err(|x| x.to_string())?;
    let b = body.vertex_point(vb).map_err(|x| x.to_string())?;
    let d = (b - a).normalized().ok_or("the edge has no length")?;
    Ok((a, d, Some((a, b))))
}

fn eval_length(doc: &Document, e: &str) -> Result<f64, String> {
    doc.evaluate_feature_value(e, Dim::LENGTH)
        .map_err(|x| x.to_string())
}

fn point_of(
    doc: &Document,
    made: &Made,
    built: &BTreeMap<u32, Constructed>,
    r: &PointRef,
) -> Result<Point3, String> {
    let p = match r {
        PointRef::Origin => Point3::ORIGIN,
        PointRef::Coordinates { x, y, z } => Point3::new(
            eval_length(doc, x)?,
            eval_length(doc, y)?,
            eval_length(doc, z)?,
        ),
        PointRef::Vertex { faces } => {
            let (body, ids) = find_faces(made, faces)?;
            let first = *ids.first().ok_or("no faces named")?;
            let loops = body.face_loops(first).map_err(|x| x.to_string())?;
            let mut found = None;
            'search: for l in loops {
                for v in body.loop_vertices(l).map_err(|x| x.to_string())? {
                    let around = body.vertex_faces(v).map_err(|x| x.to_string())?;
                    if ids.iter().all(|f| around.contains(f)) {
                        found = Some(v);
                        break 'search;
                    }
                }
            }
            let v = found.ok_or("the faces named do not meet at a vertex")?;
            body.vertex_point(v).map_err(|x| x.to_string())?
        }
        PointRef::Centre { edge } => {
            let (body, e) = find_edge(made, edge)?;
            match body.edge_curve(e).map_err(|x| x.to_string())? {
                Some((Curve::Circle(c), _, _)) => c.center(),
                Some((Curve::Arc(a), _, _)) => a.circle().center(),
                _ => return Err("the edge is not a circle or an arc".into()),
            }
        }
        PointRef::Construct { feature } => match built.get(feature) {
            Some(Constructed::Point(p)) => *p,
            _ => {
                return Err(format!(
                    "point {feature} it is built on is suppressed or could not be built"
                ));
            }
        },
    };
    if !p.is_valid() {
        return Err("the point is outside the working range".into());
    }
    Ok(p)
}

/// Rotates `v` about the unit axis `k` by `angle` (Rodrigues' formula; `v` is
/// perpendicular to `k` here, so the `k (k · v)` term vanishes).
fn turn(v: Vec3, k: Vec3, angle: f64) -> Vec3 {
    let (s, c) = angle.sin_cos();
    v * c + k.cross(v) * s
}

/// Builds one construction from the design and the bodies built before it.
pub fn build(
    doc: &Document,
    c: &Construction,
    made: &Made,
    built: &BTreeMap<u32, Constructed>,
) -> Result<Constructed, String> {
    let range = || "the result leaves the working range".to_owned();
    Ok(match c {
        Construction::PlaneOffset { base, distance } => {
            let (frame, extent) = plane_of(made, built, base)?;
            let d = eval_length(doc, distance)?;
            let frame = frame
                .with_origin(frame.origin() + frame.z() * d)
                .ok_or_else(range)?;
            // The drawn extent moves with the plane.
            let extent = extent.map(|b| Aabb {
                min: b.min + frame.z() * d,
                max: b.max + frame.z() * d,
            });
            Constructed::Plane { frame, extent }
        }
        Construction::PlaneAngle { axis, angle, base } => {
            let (origin, d, span) = axis_of(made, built, axis)?;
            let a = doc
                .evaluate_feature_value(angle, Dim::ANGLE)
                .map_err(|x| x.to_string())?;
            // The plane at 0: through the axis, with the normal of `base` (or of the first
            // origin plane not perpendicular to the axis) made perpendicular to the axis.
            let normals: Vec<Vec3> = match base {
                Some(b) => vec![plane_of(made, built, b)?.0.z()],
                None => vec![Vec3::Z, Vec3::new(0.0, -1.0, 0.0), Vec3::X],
            };
            let n0 = normals
                .iter()
                .find_map(|n| {
                    let p = *n - d * n.dot(d);
                    (p.length() > PARALLEL_SIN)
                        .then(|| p.normalized())
                        .flatten()
                })
                .ok_or("the reference plane is perpendicular to the axis")?;
            let n = turn(n0, d, a);
            let frame = Frame::from_normal_and_x(origin, n, d).ok_or_else(range)?;
            let extent = span.and_then(|(p, q)| Aabb::from_points([p, q]));
            Constructed::Plane { frame, extent }
        }
        Construction::PlaneMidplane { planes } => {
            let (a, ea) = plane_of(made, built, &planes[0])?;
            let (b, eb) = plane_of(made, built, &planes[1])?;
            if a.z().cross(b.z()).length() > PARALLEL_SIN {
                return Err("the two planes are not parallel".into());
            }
            // Signed distance from a to b along a's normal; the midplane is half of it.
            let gap = a.z().dot(b.origin() - a.origin());
            let frame = a
                .with_origin(a.origin() + a.z() * (gap * 0.5))
                .ok_or_else(range)?;
            let extent = match (ea, eb) {
                (Some(x), Some(y)) => Some(x.union(y)),
                (x, y) => x.or(y),
            };
            Constructed::Plane { frame, extent }
        }
        Construction::PlaneThreePoints { points } => {
            let p: Vec<Point3> = points
                .iter()
                .map(|r| point_of(doc, made, built, r))
                .collect::<Result<_, _>>()?;
            let (p0, p1, p2) = (p[0], p[1], p[2]);
            let n = (p1 - p0).cross(p2 - p0);
            let scale = (p1 - p0).length().max((p2 - p0).length());
            if n.length() <= LINEAR_TOLERANCE * scale.max(1.0) {
                return Err("the three points are on one line".into());
            }
            let frame = Frame::from_normal_and_x(p0, n, p1 - p0).ok_or_else(range)?;
            Constructed::Plane {
                frame,
                extent: Aabb::from_points(p),
            }
        }
        Construction::PlaneTangent { face, point } => {
            let (body, f) = find_face(made, face)?;
            let cyl = match body.face_surface(f).map_err(|x| x.to_string())? {
                Some((Surface::Cylinder(c), _)) => c,
                _ => return Err(format!("the face {:?} is not cylindrical", face.face)),
            };
            let q = point_of(doc, made, built, point)?;
            let (o, k) = (cyl.frame().origin(), cyl.frame().z());
            let v = q - o;
            let along = o + k * v.dot(k);
            let radial = (v - k * v.dot(k))
                .normalized()
                .ok_or("the point is on the cylinder's axis")?;
            let touch = along + radial * cyl.radius();
            let frame = Frame::from_normal_and_x(touch, radial, k).ok_or_else(range)?;
            Constructed::Plane {
                frame,
                extent: face_extent(body, f),
            }
        }
        Construction::AxisCylinder { face } => {
            let (body, f) = find_face(made, face)?;
            let frame = match body.face_surface(f).map_err(|x| x.to_string())? {
                Some((Surface::Cylinder(c), _)) => *c.frame(),
                Some((Surface::Cone(c), _)) => *c.frame(),
                _ => {
                    return Err(format!(
                        "the face {:?} is not cylindrical or conical",
                        face.face
                    ));
                }
            };
            let (o, k) = (frame.origin(), frame.z());
            // Draw the axis over the face's length.
            let span = face_extent(body, f).map(|b| {
                let corners = corners(&b);
                let ts = corners.iter().map(|c| k.dot(*c - o));
                let lo = ts.clone().fold(f64::INFINITY, f64::min);
                let hi = ts.fold(f64::NEG_INFINITY, f64::max);
                (o + k * lo, o + k * hi)
            });
            Constructed::Axis {
                origin: o,
                direction: k,
                span,
            }
        }
        Construction::AxisEdge { edge } => {
            let (origin, direction, span) = edge_axis(made, edge)?;
            Constructed::Axis {
                origin,
                direction,
                span,
            }
        }
        Construction::AxisTwoPoints { points } => {
            let a = point_of(doc, made, built, &points[0])?;
            let b = point_of(doc, made, built, &points[1])?;
            let direction = (b - a)
                .normalized()
                .filter(|_| !a.coincident(b))
                .ok_or("the two points are the same")?;
            Constructed::Axis {
                origin: a,
                direction,
                span: Some((a, b)),
            }
        }
        Construction::AxisTwoPlanes { planes } => {
            let (a, _) = plane_of(made, built, &planes[0])?;
            let (b, _) = plane_of(made, built, &planes[1])?;
            let (n1, n2) = (a.z(), b.z());
            let u = n1.cross(n2);
            let s2 = u.dot(u);
            if s2.sqrt() <= PARALLEL_SIN {
                return Err("the two planes are parallel".into());
            }
            // The point of the line nearest the origin is p = a n1 + b n2 with
            // n1 · p = d1 and n2 · p = d2; with c = n1 · n2 and 1 - c² = |n1 × n2|²:
            // a = (d1 - c d2) / (1 - c²), b = (d2 - c d1) / (1 - c²).
            let (d1, d2) = (
                n1.dot(a.origin() - Point3::ORIGIN),
                n2.dot(b.origin() - Point3::ORIGIN),
            );
            let c = n1.dot(n2);
            let p = Point3::ORIGIN + n1 * ((d1 - c * d2) / s2) + n2 * ((d2 - c * d1) / s2);
            if !p.is_valid() {
                return Err(range());
            }
            Constructed::Axis {
                origin: p,
                direction: u.normalized().ok_or("the two planes are parallel")?,
                span: None,
            }
        }
        Construction::Point { at } => Constructed::Point(point_of(doc, made, built, at)?),
    })
}

fn corners(b: &Aabb) -> [Point3; 8] {
    let (l, h) = (b.min, b.max);
    [
        Point3::new(l.x, l.y, l.z),
        Point3::new(h.x, l.y, l.z),
        Point3::new(l.x, h.y, l.z),
        Point3::new(h.x, h.y, l.z),
        Point3::new(l.x, l.y, h.z),
        Point3::new(h.x, l.y, h.z),
        Point3::new(l.x, h.y, h.z),
        Point3::new(h.x, h.y, h.z),
    ]
}

/// Half the side of a plane drawn with nothing around it to size it by (mm).
const DEFAULT_HALF: f64 = 25.0;
/// The drawn size around a plane's geometry, as a fraction of that geometry's size.
const MARGIN: f64 = 0.15;

/// The rectangle a plane is drawn as: its four corners, covering `extent` (or the model)
/// seen along the plane's normal.
pub fn plane_rectangle(frame: &Frame, extent: Option<Aabb>, model: Option<Aabb>) -> [Point3; 4] {
    let b = extent.or(model);
    let (mut lo, mut hi) = ((-DEFAULT_HALF, -DEFAULT_HALF), (DEFAULT_HALF, DEFAULT_HALF));
    if let Some(b) = b {
        let pts: Vec<_> = corners(&b).iter().map(|c| frame.to_plane(*c)).collect();
        let (x0, x1) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), p| {
                (a.min(p.x), z.max(p.x))
            });
        let (y0, y1) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, z), p| {
                (a.min(p.y), z.max(p.y))
            });
        let m = ((x1 - x0).max(y1 - y0) * MARGIN).max(2.0);
        if x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite() {
            lo = (x0 - m, y0 - m);
            hi = (x1 + m, y1 + m);
        }
    }
    let p = |x: f64, y: f64| frame.from_plane(chamfera_geom::Point2::new(x, y));
    [p(lo.0, lo.1), p(hi.0, lo.1), p(hi.0, hi.1), p(lo.0, hi.1)]
}

/// The segment an axis is drawn as.
pub fn axis_segment(
    origin: Point3,
    direction: Vec3,
    span: Option<(Point3, Point3)>,
    model: Option<Aabb>,
) -> [Point3; 2] {
    let (lo, hi) = match (span, model) {
        (Some((a, b)), _) => {
            let (ta, tb) = (direction.dot(a - origin), direction.dot(b - origin));
            (ta.min(tb), ta.max(tb))
        }
        (None, Some(m)) => {
            let ts: Vec<f64> = corners(&m)
                .iter()
                .map(|c| direction.dot(*c - origin))
                .collect();
            (
                ts.iter().copied().fold(f64::INFINITY, f64::min),
                ts.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        }
        (None, None) => (-DEFAULT_HALF, DEFAULT_HALF),
    };
    let m = ((hi - lo) * MARGIN).max(5.0);
    [origin + direction * (lo - m), origin + direction * (hi + m)]
}

/// What `construct.pick` finds under a pixel: the face, its nearest edge and vertex.
#[derive(Debug, Clone)]
pub struct Picked {
    pub face: FaceRef,
    /// `plane`, `cylinder`, `cone`, `sphere`, `torus` or `unknown`.
    pub surface: &'static str,
    pub point: Point3,
    /// The face's edge nearest the pick, named by its two faces, and its curve kind.
    pub edge: Option<(EdgeRef, &'static str)>,
    /// The face's vertex nearest the pick: the named faces meeting there, and where.
    pub vertex: Option<(Vec<FaceRef>, Point3)>,
}

fn face_ref_of(body: &Body, face: FaceId) -> Option<FaceRef> {
    let name = body.face_name(face).ok().flatten()?;
    let (feature, role) = split_face_key(name)?;
    Some(FaceRef {
        feature,
        face: role.to_owned(),
    })
}

impl Session {
    /// The constructions of the last rebuild, by feature id.
    pub fn constructs(&self) -> &BTreeMap<u32, Constructed> {
        &self.built.constructs
    }

    /// The bounds of the solids, for sizing drawn construction geometry.
    fn model_bounds(&self) -> Option<Aabb> {
        self.built
            .solids
            .iter()
            .filter_map(|s| s.body.bounding_box())
            .reduce(Aabb::union)
    }

    /// The face of a solid under pixel `(x, y)`, with its nearest edge and vertex.
    pub fn construct_pick(&self, x: f64, y: f64, w: f64, h: f64) -> Option<Picked> {
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.scene.pick(&ray)?;
        let solid = self
            .built
            .solids
            .iter()
            .find(|s| s.scene_id == Some(hit.body))?;
        let face = *solid.faces.get(hit.triangle)?;
        let body = &solid.body;
        let face_ref = face_ref_of(body, face)?;
        let surface = match body.face_surface(face).ok().flatten() {
            Some((Surface::Plane(_), _)) => "plane",
            Some((Surface::Cylinder(_), _)) => "cylinder",
            Some((Surface::Cone(_), _)) => "cone",
            Some((Surface::Sphere(_), _)) => "sphere",
            Some((Surface::Torus(_), _)) => "torus",
            Some((Surface::Extrusion(_), _)) => "extrusion",
            None => "unknown",
        };
        let p = hit.point;
        // The nearest edge that joins this face to another named face.
        let mut edge: Option<(f64, EdgeRef, &'static str)> = None;
        for e in body.face_edges(face).ok()? {
            let Ok(Some((curve, t0, t1))) = body.edge_curve(e) else {
                continue;
            };
            let Ok(faces) = body.edge_faces(e) else {
                continue;
            };
            let Some(other) = faces.iter().find(|f| **f != face) else {
                continue;
            };
            let Some(other_ref) = face_ref_of(body, *other) else {
                continue;
            };
            let Ok(d) = curve.distance_on(t0, t1, p) else {
                continue;
            };
            let kind = match curve {
                Curve::Line(_) | Curve::Segment(_) => "line",
                Curve::Circle(_) => "circle",
                Curve::Arc(_) => "arc",
                Curve::Ellipse(_) | Curve::EllipticalArc(_) => "ellipse",
                Curve::BSpline(_) => "spline",
            };
            if edge.as_ref().is_none_or(|b| d < b.0) {
                edge = Some((
                    d,
                    EdgeRef {
                        faces: [face_ref.clone(), other_ref],
                    },
                    kind,
                ));
            }
        }
        // The nearest vertex of this face, named by every named face around it.
        let mut vertex: Option<(f64, Vec<FaceRef>, Point3)> = None;
        for l in body.face_loops(face).ok()? {
            for v in body.loop_vertices(l).ok()? {
                let Ok(q) = body.vertex_point(v) else {
                    continue;
                };
                let d = q.distance(p);
                if vertex.as_ref().is_some_and(|b| d >= b.0) {
                    continue;
                }
                let mut names: Vec<FaceRef> = body
                    .vertex_faces(v)
                    .ok()?
                    .into_iter()
                    .filter_map(|f| face_ref_of(body, f))
                    .collect();
                names.dedup();
                if (2..=chamfera_doc::construct::MAX_VERTEX_FACES).contains(&names.len()) {
                    vertex = Some((d, names, q));
                }
            }
        }
        Some(Picked {
            face: face_ref,
            surface,
            point: p,
            edge: edge.map(|(_, e, k)| (e, k)),
            vertex: vertex.map(|(_, f, q)| (f, q)),
        })
    }

    /// The construction geometry as overlays (outlines, axes, points) and fills (planes),
    /// for the viewport. Display only.
    pub(crate) fn construct_overlays(&self) -> (Vec<Overlay>, Vec<Fill>) {
        let model = self.model_bounds();
        let mut lines = Vec::new();
        let mut points = Vec::new();
        let mut triangles = Vec::new();
        for c in self.built.constructs.values() {
            match c {
                Constructed::Plane { frame, extent } => {
                    let [a, b, cc, d] = plane_rectangle(frame, *extent, model);
                    lines.extend([[a, b], [b, cc], [cc, d], [d, a]]);
                    triangles.extend([[a, b, cc], [a, cc, d]]);
                }
                Constructed::Axis {
                    origin,
                    direction,
                    span,
                } => {
                    let [a, b] = axis_segment(*origin, *direction, *span, model);
                    lines.push([a, b]);
                }
                Constructed::Point(p) => points.push(*p),
            }
        }
        let mut overlays = Vec::new();
        if !lines.is_empty() || !points.is_empty() {
            overlays.push(Overlay {
                style: OverlayStyle::Construct,
                lines,
                points,
                on_top: false,
            });
        }
        let fills = if triangles.is_empty() {
            Vec::new()
        } else {
            vec![Fill {
                style: FillStyle::Construct,
                triangles,
            }]
        };
        (overlays, fills)
    }
}

#[cfg(test)]
mod tests;

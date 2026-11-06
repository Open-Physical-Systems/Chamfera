//! Measuring between entities of solids: minimum distance with the two closest points,
//! the angle between straight edges and flat faces, edge lengths and radii.
//!
//! # Minimum distance
//! The distance between two point sets is the minimum over their pieces: a body is its
//! faces, a face is its surface patch bounded by its edges. Between two pieces the
//! minimum lies either on the boundary of one of them or at interior points where the
//! segment joining them is normal to both (the first-order condition of a smooth
//! minimum; see Patrikalakis and Maekawa, *Shape Interrogation for CAD and
//! Manufacturing*, section 6.6 on distance functions). So:
//! - point to edge: the exact closest point of the edge's curve within its range
//!   (lines, circles and arcs in closed form, ellipses as `curve3.rs` does);
//! - point to face: the surface's exact closest point when the inside-face test says it
//!   is on the face, otherwise the nearest boundary edge;
//! - edge to edge: two straight edges in closed form (Ericson, *Real-Time Collision
//!   Detection*, section 5.1.9); curved ones from the best of sampled pairs and the end
//!   points, refined by alternating closest points;
//! - edge to face: the end points, the face's boundary edges, the points where the edge
//!   crosses the face (distance zero, from the exact curve-surface intersection), and for
//!   curved pieces interior critical points by alternating closest points;
//! - face to face: each face's boundary edges against the other face, plus interior
//!   critical points on curved faces by alternating closest points.
//!
//! For flat faces and straight edges every case is exact: a line and a plane have no
//! interior critical point unless they are parallel, and then the boundary attains the
//! minimum too. Alternating closest points (von Neumann's alternating projections)
//! decrease the distance monotonically and stop at a pair normal to both pieces; seeded
//! from several samples they find the minimum of the analytic pieces a CAD model is made
//! of, though not every local minimum of a general surface.
//!
//! A body inside another is at the distance between their boundaries, not zero; solids
//! that overlap or touch are at zero, as their faces meet.

use crate::ssi::intersect_curve_surface_on;
use crate::topology::Curve;
use crate::{
    Arc3, Body, EdgeId, EllipticalArc3, FaceId, KernelError, KernelResult, PointOnFace, Segment3,
    Surface, VertexId,
};
use chamfera_geom::{LINEAR_TOLERANCE, Point3, Vec3};

/// Most face pairs one distance query may compare (hostile-input limit).
pub const MAX_DISTANCE_PAIRS: usize = 250_000;
/// Samples along a curved edge to seed the refinement.
const EDGE_SAMPLES: usize = 24;
/// Most rounds of alternating closest points from one seed.
const MAX_ROUNDS: usize = 64;

/// A piece of a body to measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entity {
    /// A free point in space (the body is not used).
    Point(Point3),
    Vertex(VertexId),
    Edge(EdgeId),
    Face(FaceId),
    /// The whole body (its boundary).
    Body,
}

/// The minimum distance between two entities and where it is attained.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closest {
    pub distance: f64,
    /// The closest point on the first entity.
    pub on_a: Point3,
    /// The closest point on the second entity.
    pub on_b: Point3,
}

impl Closest {
    fn new(on_a: Point3, on_b: Point3) -> Closest {
        Closest {
            distance: on_a.distance(on_b),
            on_a,
            on_b,
        }
    }
    fn swapped(self) -> Closest {
        Closest {
            distance: self.distance,
            on_a: self.on_b,
            on_b: self.on_a,
        }
    }
}

fn better(best: &mut Option<Closest>, c: Closest) {
    if best.is_none_or(|b| c.distance < b.distance) {
        *best = Some(c);
    }
}

/// An edge's curve with its range.
#[derive(Debug, Clone)]
struct EdgeGeo {
    curve: Curve,
    t0: f64,
    t1: f64,
}

impl EdgeGeo {
    fn of(body: &Body, e: EdgeId) -> KernelResult<EdgeGeo> {
        let (curve, t0, t1) = body
            .edge_curve(e)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{e} has no curve yet")))?;
        Ok(EdgeGeo { curve, t0, t1 })
    }

    fn straight(&self) -> bool {
        matches!(self.curve, Curve::Line(_) | Curve::Segment(_))
    }

    fn point_at(&self, s: f64) -> Point3 {
        self.curve.point_at(self.t0 + (self.t1 - self.t0) * s)
    }

    fn ends(&self) -> [Point3; 2] {
        [self.point_at(0.0), self.point_at(1.0)]
    }

    /// The point of the edge nearest to `p`.
    fn closest(&self, p: Point3) -> KernelResult<Point3> {
        let (t0, t1) = (self.t0, self.t1);
        Ok(match &self.curve {
            Curve::Line(_) | Curve::Segment(_) => {
                let [a, b] = self.ends();
                if a.coincident(b) {
                    a
                } else {
                    Segment3::new(a, b)?.closest_point(p)?.point
                }
            }
            Curve::Circle(c) => {
                Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?
                    .closest_point(p)?
                    .point
            }
            Curve::Arc(a) => {
                let c = a.circle();
                Arc3::new(*c.frame(), c.radius(), t0, t1 - t0)?
                    .closest_point(p)?
                    .point
            }
            Curve::Ellipse(e) => {
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?
                    .closest_point(p)?
                    .point
            }
            Curve::EllipticalArc(a) => {
                let e = a.ellipse();
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?
                    .closest_point(p)?
                    .point
            }
        })
    }

    fn length(&self) -> KernelResult<f64> {
        let (t0, t1) = (self.t0, self.t1);
        Ok(match &self.curve {
            Curve::Line(_) | Curve::Segment(_) => {
                let [a, b] = self.ends();
                a.distance(b)
            }
            Curve::Circle(c) => c.radius() * (t1 - t0),
            Curve::Arc(a) => a.circle().radius() * (t1 - t0),
            Curve::Ellipse(e) => {
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?.length()?
            }
            Curve::EllipticalArc(a) => {
                let e = a.ellipse();
                EllipticalArc3::new(*e.frame(), e.rx(), e.ry(), t0, t1 - t0)?.length()?
            }
        })
    }
}

/// The closest points of two segments `p1 q1` and `p2 q2` (Ericson, *Real-Time Collision
/// Detection*, section 5.1.9, `ClosestPtSegmentSegment`).
fn segment_segment(p1: Point3, q1: Point3, p2: Point3, q2: Point3) -> Closest {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let eps = LINEAR_TOLERANCE * LINEAR_TOLERANCE;
    let (s, t);
    if a <= eps && e <= eps {
        return Closest::new(p1, p2);
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > eps * a.max(e) {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    Closest::new(p1 + d1 * s, p2 + d2 * t)
}

/// A face with what the distance needs.
struct FaceGeo<'a> {
    body: &'a Body,
    face: FaceId,
    surface: Surface,
    edges: Vec<EdgeGeo>,
}

impl<'a> FaceGeo<'a> {
    fn of(body: &'a Body, face: FaceId) -> KernelResult<FaceGeo<'a>> {
        let (surface, _) = body
            .face_surface(face)?
            .ok_or_else(|| KernelError::InvalidInput(format!("{face} has no surface yet")))?;
        let edges = body
            .face_edges(face)?
            .into_iter()
            .map(|e| EdgeGeo::of(body, e))
            .collect::<KernelResult<Vec<_>>>()?;
        Ok(FaceGeo {
            body,
            face,
            surface,
            edges,
        })
    }

    fn planar(&self) -> bool {
        matches!(self.surface, Surface::Plane(_))
    }

    fn contains(&self, p: Point3) -> KernelResult<bool> {
        Ok(match self.body.classify_point_on_face(self.face, p) {
            Ok(PointOnFace::Inside | PointOnFace::Boundary) => true,
            Ok(PointOnFace::Outside) => false,
            // Faces the inside test does not support yet fall back to their boundary.
            Err(KernelError::Unsupported(_)) => false,
            Err(e) => return Err(e),
        })
    }

    /// The point of the face nearest to `p`.
    fn closest(&self, p: Point3) -> KernelResult<Point3> {
        let mut best: Option<Closest> = None;
        for e in &self.edges {
            better(&mut best, Closest::new(p, e.closest(p)?));
        }
        let sp = self.surface.closest_point(p)?;
        if best.is_none_or(|b| sp.distance < b.distance) && self.contains(sp.point)? {
            better(&mut best, Closest::new(p, sp.point));
        }
        best.map(|b| b.on_b)
            .ok_or_else(|| KernelError::InvalidInput(format!("{} has no edges", self.face)))
    }
}

/// Alternating closest points from `a0` on piece A: `b = B(a)`, `a = A(b)`, until the
/// distance stops falling.
fn alternate(
    a0: Point3,
    on_a: &dyn Fn(Point3) -> KernelResult<Point3>,
    on_b: &dyn Fn(Point3) -> KernelResult<Point3>,
) -> KernelResult<Closest> {
    let mut a = a0;
    let mut b = on_b(a)?;
    let mut best = Closest::new(a, b);
    for _ in 0..MAX_ROUNDS {
        a = on_a(b)?;
        b = on_b(a)?;
        let c = Closest::new(a, b);
        let gain = best.distance - c.distance;
        if c.distance < best.distance {
            best = c;
        }
        if gain <= LINEAR_TOLERANCE * 1e-3 {
            break;
        }
    }
    Ok(best)
}

fn edge_edge(a: &EdgeGeo, b: &EdgeGeo) -> KernelResult<Closest> {
    if a.straight() && b.straight() {
        let ([p1, q1], [p2, q2]) = (a.ends(), b.ends());
        return Ok(segment_segment(p1, q1, p2, q2));
    }
    let mut best: Option<Closest> = None;
    for p in a.ends() {
        better(&mut best, Closest::new(p, b.closest(p)?));
    }
    for p in b.ends() {
        better(&mut best, Closest::new(a.closest(p)?, p));
    }
    // Seeds: the nearest sample pairs.
    let n = EDGE_SAMPLES;
    let sa: Vec<Point3> = (0..=n).map(|i| a.point_at(i as f64 / n as f64)).collect();
    let sb: Vec<Point3> = (0..=n).map(|i| b.point_at(i as f64 / n as f64)).collect();
    let mut pairs: Vec<(f64, Point3)> = Vec::new();
    for p in &sa {
        let d = sb.iter().fold(f64::INFINITY, |m, q| m.min(p.distance(*q)));
        pairs.push((d, *p));
    }
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (_, p) in pairs.iter().take(3) {
        better(
            &mut best,
            alternate(*p, &|q| a.closest(q), &|q| b.closest(q))?,
        );
    }
    best.ok_or_else(|| KernelError::InvalidInput("an edge has no points".into()))
}

fn edge_face(e: &EdgeGeo, f: &FaceGeo) -> KernelResult<Closest> {
    let mut best: Option<Closest> = None;
    for p in e.ends() {
        better(&mut best, Closest::new(p, f.closest(p)?));
    }
    for b in &f.edges {
        better(&mut best, edge_edge(e, b)?);
    }
    // Where the edge crosses the face: distance zero.
    if let Ok(hits) = intersect_curve_surface_on(&e.curve, e.t0, e.t1, &f.surface) {
        for h in hits.points() {
            if f.contains(h.point)? {
                better(&mut best, Closest::new(h.point, h.point));
            }
        }
    }
    if !(e.straight() && f.planar()) {
        let n = EDGE_SAMPLES / 2;
        let mut seeds: Vec<(f64, Point3)> = Vec::new();
        for i in 0..=n {
            let p = e.point_at(i as f64 / n as f64);
            seeds.push((f.surface.distance(p), p));
        }
        seeds.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (_, p) in seeds.iter().take(2) {
            better(
                &mut best,
                alternate(*p, &|q| e.closest(q), &|q| f.closest(q))?,
            );
        }
    }
    best.ok_or_else(|| KernelError::InvalidInput("an edge has no points".into()))
}

fn face_face(a: &FaceGeo, b: &FaceGeo) -> KernelResult<Closest> {
    let mut best: Option<Closest> = None;
    for e in &a.edges {
        better(&mut best, edge_face(e, b)?);
        if best.is_some_and(|c| c.distance == 0.0) {
            return best.ok_or(KernelError::EmptyResult);
        }
    }
    for e in &b.edges {
        better(&mut best, edge_face(e, a)?.swapped());
    }
    if !(a.planar() && b.planar()) {
        // Interior critical points: seeds on A nearest to B's edge points.
        let mut seeds: Vec<Point3> = Vec::new();
        for e in &b.edges {
            for p in e.ends() {
                seeds.push(a.closest(p)?);
            }
            seeds.push(a.closest(e.point_at(0.5))?);
        }
        seeds.truncate(8);
        for p in seeds {
            better(
                &mut best,
                alternate(p, &|q| a.closest(q), &|q| b.closest(q))?,
            );
        }
    }
    best.ok_or_else(|| KernelError::InvalidInput("a face has no edges".into()))
}

/// What one entity is made of, for the distance.
enum Piece<'a> {
    Point(Point3),
    Edge(EdgeGeo),
    Face(FaceGeo<'a>),
}

fn pieces(body: &Body, e: Entity) -> KernelResult<Vec<Piece<'_>>> {
    Ok(match e {
        Entity::Point(p) => vec![Piece::Point(crate::check::point3("point", p)?)],
        Entity::Vertex(v) => vec![Piece::Point(body.vertex_point(v)?)],
        Entity::Edge(x) => vec![Piece::Edge(EdgeGeo::of(body, x)?)],
        Entity::Face(f) => vec![Piece::Face(FaceGeo::of(body, f)?)],
        Entity::Body => {
            let faces = body.faces();
            if faces.is_empty() {
                body.vertices()
                    .into_iter()
                    .map(|v| Ok(Piece::Point(body.vertex_point(v)?)))
                    .collect::<KernelResult<Vec<_>>>()?
            } else {
                faces
                    .into_iter()
                    .map(|f| Ok(Piece::Face(FaceGeo::of(body, f)?)))
                    .collect::<KernelResult<Vec<_>>>()?
            }
        }
    })
}

fn piece_piece(a: &Piece, b: &Piece) -> KernelResult<Closest> {
    Ok(match (a, b) {
        (Piece::Point(p), Piece::Point(q)) => Closest::new(*p, *q),
        (Piece::Point(p), Piece::Edge(e)) => Closest::new(*p, e.closest(*p)?),
        (Piece::Edge(e), Piece::Point(p)) => Closest::new(e.closest(*p)?, *p),
        (Piece::Point(p), Piece::Face(f)) => Closest::new(*p, f.closest(*p)?),
        (Piece::Face(f), Piece::Point(p)) => Closest::new(f.closest(*p)?, *p),
        (Piece::Edge(x), Piece::Edge(y)) => edge_edge(x, y)?,
        (Piece::Edge(e), Piece::Face(f)) => edge_face(e, f)?,
        (Piece::Face(f), Piece::Edge(e)) => edge_face(e, f)?.swapped(),
        (Piece::Face(f), Piece::Face(g)) => face_face(f, g)?,
    })
}

/// The minimum distance between entity `a` of `body_a` and entity `b` of `body_b`, with
/// the two closest points (see the module documentation for the method).
pub fn closest_points(body_a: &Body, a: Entity, body_b: &Body, b: Entity) -> KernelResult<Closest> {
    let pa = pieces(body_a, a)?;
    let pb = pieces(body_b, b)?;
    if pa.len().saturating_mul(pb.len()) > MAX_DISTANCE_PAIRS {
        return Err(KernelError::InvalidInput(format!(
            "a distance may compare at most {MAX_DISTANCE_PAIRS} pairs of faces"
        )));
    }
    let mut best: Option<Closest> = None;
    for x in &pa {
        for y in &pb {
            better(&mut best, piece_piece(x, y)?);
            if best.is_some_and(|c| c.distance == 0.0) {
                return best.ok_or(KernelError::EmptyResult);
            }
        }
    }
    best.ok_or_else(|| KernelError::InvalidInput("there is nothing to measure".into()))
}

/// The direction of a straight edge, from `from` when that is one of its ends.
fn edge_direction(body: &Body, e: EdgeId, from: Option<VertexId>) -> KernelResult<Option<Vec3>> {
    let g = EdgeGeo::of(body, e)?;
    if !g.straight() {
        return Ok(None);
    }
    let [a, b] = g.ends();
    let [va, _] = body.edge_vertices(e)?;
    let d = match from {
        Some(v) if v != va => a - b,
        _ => b - a,
    };
    Ok(d.normalized())
}

/// The outward normal of a flat face.
fn face_normal(body: &Body, f: FaceId) -> KernelResult<Option<Vec3>> {
    Ok(match body.face_surface(f)? {
        Some((Surface::Plane(frame), same)) => Some(if same { frame.z() } else { -frame.z() }),
        _ => None,
    })
}

/// The angle between two entities, in radians, or `None` when they have no angle:
/// - two straight edges: with a common vertex (same body), the angle between them seen
///   from that vertex (`0..=π`, 90° at a box corner); otherwise between their lines
///   (`0..=π/2`);
/// - two flat faces: the angle between their outward normals (`0..=π`: 90° between
///   adjacent faces of a box, 180° between opposite ones);
/// - a straight edge and a flat face: between the line and the plane (`0..=π/2`).
pub fn angle_between(
    body_a: &Body,
    a: Entity,
    body_b: &Body,
    b: Entity,
) -> KernelResult<Option<f64>> {
    let same_body = std::ptr::eq(body_a, body_b);
    Ok(match (a, b) {
        (Entity::Edge(x), Entity::Edge(y)) => {
            let shared = if same_body {
                let [x0, x1] = body_a.edge_vertices(x)?;
                let ys = body_b.edge_vertices(y)?;
                [x0, x1].into_iter().find(|v| ys.contains(v))
            } else {
                None
            };
            let (Some(dx), Some(dy)) = (
                edge_direction(body_a, x, shared)?,
                edge_direction(body_b, y, shared)?,
            ) else {
                return Ok(None);
            };
            let t = dx.angle_to(dy);
            if shared.is_some() || x == y && same_body {
                t
            } else {
                t.map(|t| t.min(std::f64::consts::PI - t))
            }
        }
        (Entity::Face(x), Entity::Face(y)) => {
            match (face_normal(body_a, x)?, face_normal(body_b, y)?) {
                (Some(n), Some(m)) => n.angle_to(m),
                _ => None,
            }
        }
        (Entity::Edge(e), Entity::Face(f)) => line_plane(body_a, e, body_b, f)?,
        (Entity::Face(f), Entity::Edge(e)) => line_plane(body_b, e, body_a, f)?,
        _ => None,
    })
}

fn line_plane(be: &Body, e: EdgeId, bf: &Body, f: FaceId) -> KernelResult<Option<f64>> {
    Ok(match (edge_direction(be, e, None)?, face_normal(bf, f)?) {
        (Some(d), Some(n)) => Some(d.dot(n).abs().clamp(0.0, 1.0).asin()),
        _ => None,
    })
}

impl Body {
    /// The length of an edge (exact for lines, circles and arcs; ellipses to the
    /// accuracy of `EllipticalArc3::length`).
    pub fn edge_length(&self, e: EdgeId) -> KernelResult<f64> {
        EdgeGeo::of(self, e)?.length()
    }

    /// The radius of a circular edge (circle or arc), else `None`.
    pub fn edge_radius(&self, e: EdgeId) -> KernelResult<Option<f64>> {
        Ok(match self.edge_curve(e)? {
            Some((Curve::Circle(c), _, _)) => Some(c.radius()),
            Some((Curve::Arc(a), _, _)) => Some(a.circle().radius()),
            _ => None,
        })
    }

    /// The radius of a cylindrical or spherical face, else `None`.
    pub fn face_radius(&self, f: FaceId) -> KernelResult<Option<f64>> {
        Ok(match self.face_surface(f)? {
            Some((Surface::Cylinder(c), _)) => Some(c.radius()),
            Some((Surface::Sphere(s), _)) => Some(s.radius()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Profile, extrude};
    use chamfera_geom::{Frame, Point2};
    use std::f64::consts::{FRAC_PI_2, PI};

    fn block(x0: f64, y0: f64, z0: f64, dx: f64, dy: f64, dz: f64) -> Body {
        let f = Frame::XY.with_origin(Point3::new(0.0, 0.0, z0)).unwrap();
        let pts = [
            Point2::new(x0, y0),
            Point2::new(x0 + dx, y0),
            Point2::new(x0 + dx, y0 + dy),
            Point2::new(x0, y0 + dy),
        ];
        extrude(&f, &Profile::polygon(&pts), dz).unwrap()
    }

    fn face_named(b: &Body, name: &str) -> FaceId {
        b.faces()
            .into_iter()
            .find(|f| b.face_name(*f).ok().flatten() == Some(name))
            .unwrap()
    }

    #[test]
    fn two_separated_boxes_are_as_far_apart_as_their_gap() {
        // [0,10]³ and [15,25] x [3,13] x [2,12]: the gap is along x only, 5 mm, between
        // the faces x = 10 and x = 15, which overlap in y [3, 10] and z [2, 10].
        let a = block(0.0, 0.0, 0.0, 10.0, 10.0, 10.0);
        let b = block(15.0, 3.0, 2.0, 10.0, 10.0, 10.0);
        let c = closest_points(&a, Entity::Body, &b, Entity::Body).unwrap();
        assert!((c.distance - 5.0).abs() < 1e-12, "{c:?}");
        assert!((c.on_a.x - 10.0).abs() < 1e-12 && (c.on_b.x - 15.0).abs() < 1e-12);
        assert!((c.on_a.y - c.on_b.y).abs() < 1e-12 && (3.0..=10.0).contains(&c.on_a.y));
        // Diagonal: [30,40] x [30,40] x [0,10] is √(20² + 20²) from the corner (10,10).
        let d = block(30.0, 30.0, 0.0, 10.0, 10.0, 10.0);
        let c = closest_points(&a, Entity::Body, &d, Entity::Body).unwrap();
        assert!((c.distance - 800.0f64.sqrt()).abs() < 1e-12, "{c:?}");
        // Touching boxes are at zero.
        let t = block(10.0, 0.0, 0.0, 5.0, 5.0, 5.0);
        let c = closest_points(&a, Entity::Body, &t, Entity::Body).unwrap();
        assert!(c.distance < 1e-12);
    }

    #[test]
    fn a_point_above_a_face_projects_onto_it_and_beside_it_onto_an_edge() {
        let a = block(0.0, 0.0, 0.0, 10.0, 10.0, 10.0);
        let top = face_named(&a, "top");
        let far = block(4.0, 4.0, 20.0, 1.0, 1.0, 1.0);
        let v = far.vertices()[0];
        let p = far.vertex_point(v).unwrap();
        let c = closest_points(&far, Entity::Vertex(v), &a, Entity::Face(top)).unwrap();
        assert!((c.distance - (p.z - 10.0)).abs() < 1e-12, "{c:?}");
        // A point beside the top, off its edge x = 10: distance √(5² + 3²) to the edge.
        let side = block(15.0, 2.0, 13.0, 1.0, 1.0, 1.0);
        let sv = side
            .vertices()
            .into_iter()
            .find(|v| side.vertex_point(*v).unwrap() == Point3::new(15.0, 2.0, 13.0))
            .unwrap();
        let c = closest_points(&side, Entity::Vertex(sv), &a, Entity::Face(top)).unwrap();
        assert!((c.distance - 34.0f64.sqrt()).abs() < 1e-12, "{c:?}");
    }

    #[test]
    fn a_point_and_a_cylinder_meet_at_the_radius() {
        // Cylinder r = 5 about z, h = 10; the point (12, 0, 5) is 7 from its side.
        let cyl = extrude(
            &Frame::XY,
            &Profile::circle(Point2::new(0.0, 0.0), 5.0),
            10.0,
        )
        .unwrap();
        let probe = block(12.0, 0.0, 5.0, 1.0, 1.0, 1.0);
        let v = probe
            .vertices()
            .into_iter()
            .find(|v| probe.vertex_point(*v).unwrap() == Point3::new(12.0, 0.0, 5.0))
            .unwrap();
        let c = closest_points(&probe, Entity::Vertex(v), &cyl, Entity::Body).unwrap();
        assert!((c.distance - 7.0).abs() < 1e-9, "{c:?}");
        // Two cylinders side by side: centres 20 apart, radii 5: gap 10.
        let other = extrude(
            &Frame::XY,
            &Profile::circle(Point2::new(20.0, 0.0), 5.0),
            10.0,
        )
        .unwrap();
        let c = closest_points(&cyl, Entity::Body, &other, Entity::Body).unwrap();
        assert!((c.distance - 10.0).abs() < 1e-7, "{c:?}");
        // The circular edges have radius 5 and length 10π.
        let e = cyl
            .edges()
            .into_iter()
            .find(|e| cyl.edge_radius(*e).unwrap().is_some())
            .unwrap();
        assert_eq!(cyl.edge_radius(e).unwrap(), Some(5.0));
        assert!((cyl.edge_length(e).unwrap() - 10.0 * PI).abs() < 1e-12);
    }

    #[test]
    fn adjacent_box_faces_are_at_ninety_degrees_and_opposite_ones_at_one_eighty() {
        let a = block(0.0, 0.0, 0.0, 50.0, 30.0, 10.0);
        let top = face_named(&a, "top");
        let bottom = face_named(&a, "bottom");
        let side = a
            .faces()
            .into_iter()
            .find(|f| {
                a.face_name(*f)
                    .ok()
                    .flatten()
                    .is_some_and(|n| n.starts_with("side"))
            })
            .unwrap();
        let t = angle_between(&a, Entity::Face(top), &a, Entity::Face(side)).unwrap();
        assert!((t.unwrap() - FRAC_PI_2).abs() < 1e-12);
        let t = angle_between(&a, Entity::Face(top), &a, Entity::Face(bottom)).unwrap();
        assert!((t.unwrap() - PI).abs() < 1e-12);
        // Two edges at a corner: 90°; an edge and the face it lies in: 0°.
        let v = a.vertices()[0];
        let es = a.vertex_edges(v).unwrap();
        let t = angle_between(&a, Entity::Edge(es[0]), &a, Entity::Edge(es[1])).unwrap();
        assert!((t.unwrap() - FRAC_PI_2).abs() < 1e-12);
        let e = a.face_edges(top).unwrap()[0];
        let t = angle_between(&a, Entity::Edge(e), &a, Entity::Face(top)).unwrap();
        assert!(t.unwrap().abs() < 1e-12);
        let t = angle_between(&a, Entity::Edge(e), &a, Entity::Face(side)).unwrap();
        assert!(t.is_some());
        // A body has no angle.
        assert_eq!(
            angle_between(&a, Entity::Body, &a, Entity::Face(top)).unwrap(),
            None
        );
    }

    #[test]
    fn crossing_segments_meet_and_skew_ones_are_apart() {
        let c = segment_segment(
            Point3::new(-1.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, -1.0, 2.0),
            Point3::new(0.0, 1.0, 2.0),
        );
        assert!((c.distance - 2.0).abs() < 1e-15);
        assert_eq!(c.on_a, Point3::new(0.0, 0.0, 0.0));
        // Parallel overlapping: distance between the lines.
        let c = segment_segment(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
            Point3::new(6.0, 3.0, 0.0),
        );
        assert!((c.distance - 3.0).abs() < 1e-15);
    }
}

//! Associative dimensions: what a dimension's points are on the model, and where they
//! are now.
//!
//! When a dimension is made, each snapped 2D point is matched to the model entity that
//! projects onto it: a named vertex, else the start, end or centre of a named edge (the
//! one nearest the viewer when several project onto the point). The reference keeps the
//! entity's unique name and its K8 signature (`chamfera_doc::naming`). When the sheet is
//! drawn, every reference goes through the one resolver, `naming::resolve`, and the
//! points are projected again, so the dimension follows edits of the model. A reference
//! that does not resolve makes the dimension dangling: it is drawn at its stored points
//! on [`crate::Layer::Dangling`] with the reason, never dropped.

use crate::hlr::Basis;
use chamfera_doc::drawing::{DimensionKind, ModelRef, RefKind, RefPoint};
use chamfera_doc::naming::{self, Found, Kind, Want};
use chamfera_geom::{Point2, Point3, Vec3};
use chamfera_kernel::{Body, Curve, Entity};

/// Two 2D points closer than this (mm) are the same point.
const SAME: f64 = 1e-5;

/// A straight line, as two points.
type Seg = (Point3, Point3);

fn circle_of(curve: &Curve) -> Option<(Point3, Vec3, f64)> {
    match curve {
        Curve::Circle(c) => Some((c.center(), c.normal(), c.radius())),
        Curve::Arc(a) => {
            let c = a.circle();
            Some((c.center(), c.normal(), c.radius()))
        }
        _ => None,
    }
}

fn segment_of(curve: &Curve, t0: f64, t1: f64) -> Option<Seg> {
    match curve {
        Curve::Segment(_) | Curve::Line(_) => Some((curve.point_at(t0), curve.point_at(t1))),
        _ => None,
    }
}

fn depth(basis: &Basis, p: Point3) -> f64 {
    Vec3::new(p.x, p.y, p.z).dot(basis.toward)
}

/// The best candidate: the one nearest the viewer.
fn nearest(c: Vec<(f64, Found, RefPoint, Point3)>) -> Option<(Found, RefPoint, Point3)> {
    c.into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f, at, p)| (f, at, p))
}

fn reference(bodies: &[&Body], found: Found, at: RefPoint, p: Point3) -> Option<ModelRef> {
    let name = naming::reference_name(bodies, found)?;
    let kind = match found.entity {
        Entity::Vertex(_) => RefKind::Vertex,
        Entity::Edge(_) => RefKind::Edge,
        _ => return None,
    };
    let r = ModelRef {
        kind,
        signature: naming::signature_of(bodies, found, Some(p)),
        name,
        at,
    };
    r.check().is_ok().then_some(r)
}

/// The model point that projects onto `q`: a named vertex, else an edge's end or centre.
fn point_ref(bodies: &[&Body], basis: &Basis, q: Point2) -> Option<ModelRef> {
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    for (bi, b) in bodies.iter().enumerate() {
        for v in b.vertices() {
            let Ok(p) = b.vertex_point(v) else { continue };
            let found = Found {
                body: bi,
                entity: Entity::Vertex(v),
            };
            if basis.project(p).distance(q) < SAME && naming::base_name(b, found.entity).is_some() {
                vertices.push((depth(basis, p), found, RefPoint::Point, p));
            }
        }
        for e in b.edges() {
            let Ok(Some((curve, t0, t1))) = b.edge_curve(e) else {
                continue;
            };
            let found = Found {
                body: bi,
                entity: Entity::Edge(e),
            };
            if naming::base_name(b, found.entity).is_none() {
                continue;
            }
            let mut cands = vec![
                (RefPoint::Start, curve.point_at(t0)),
                (RefPoint::End, curve.point_at(t1)),
            ];
            if let Some((c, _, _)) = circle_of(&curve) {
                cands.push((RefPoint::Centre, c));
            }
            for (at, p) in cands {
                if basis.project(p).distance(q) < SAME {
                    edges.push((depth(basis, p), found, at, p));
                }
            }
        }
    }
    let (f, at, p) = nearest(vertices).or_else(|| nearest(edges))?;
    reference(bodies, f, at, p)
}

/// The circle seen end-on with its centre at `c` and radius `r`.
fn circle_ref(bodies: &[&Body], basis: &Basis, c: Point2, r: f64) -> Option<ModelRef> {
    let mut cands = Vec::new();
    for (bi, b) in bodies.iter().enumerate() {
        for e in b.edges() {
            let Ok(Some((curve, _, _))) = b.edge_curve(e) else {
                continue;
            };
            let Some((centre, normal, radius)) = circle_of(&curve) else {
                continue;
            };
            let found = Found {
                body: bi,
                entity: Entity::Edge(e),
            };
            if normal.dot(basis.toward).abs() > 1.0 - 1e-9
                && (radius - r).abs() < SAME
                && basis.project(centre).distance(c) < SAME
                && naming::base_name(b, found.entity).is_some()
            {
                cands.push((depth(basis, centre), found, RefPoint::Centre, centre));
            }
        }
    }
    let (f, at, p) = nearest(cands)?;
    reference(bodies, f, at, p)
}

/// The straight edge whose projection holds `leg` on the line through `v`.
fn line_ref(bodies: &[&Body], basis: &Basis, v: Point2, leg: Point2) -> Option<ModelRef> {
    let dir = (leg - v).normalized()?;
    let mut cands = Vec::new();
    for (bi, b) in bodies.iter().enumerate() {
        for e in b.edges() {
            let Ok(Some((curve, t0, t1))) = b.edge_curve(e) else {
                continue;
            };
            let Some((a3, b3)) = segment_of(&curve, t0, t1) else {
                continue;
            };
            let (a, c) = (basis.project(a3), basis.project(b3));
            let on_line = |p: Point2| dir.cross(p - v).abs() < SAME;
            let found = Found {
                body: bi,
                entity: Entity::Edge(e),
            };
            if a.distance(c) > SAME
                && on_line(a)
                && on_line(c)
                && distance_to_segment(leg, a, c) < SAME
                && naming::base_name(b, found.entity).is_some()
            {
                let mid = a3 + (b3 - a3) * 0.5;
                cands.push((depth(basis, mid), found, RefPoint::Line, mid));
            }
        }
    }
    let (f, at, p) = nearest(cands)?;
    reference(bodies, f, at, p)
}

fn distance_to_segment(p: Point2, a: Point2, b: Point2) -> f64 {
    let d = b - a;
    let l2 = d.dot(d);
    if l2 <= 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    p.distance(a + d * t)
}

/// The references of a new dimension with `points` (view coordinates), or none when one
/// of its points is on no named vertex or edge (it then keeps its points, as before).
pub fn make_refs(
    kind: DimensionKind,
    points: &[Point2],
    basis: &Basis,
    bodies: &[&Body],
) -> Vec<ModelRef> {
    let got: Option<Vec<ModelRef>> = match (kind, points) {
        (DimensionKind::Diameter | DimensionKind::Radius, &[c, p]) => {
            circle_ref(bodies, basis, c, c.distance(p)).map(|r| vec![r])
        }
        (DimensionKind::Angular, &[v, a, b]) => {
            match (line_ref(bodies, basis, v, a), line_ref(bodies, basis, v, b)) {
                (Some(x), Some(y)) => Some(vec![x, y]),
                _ => None,
            }
        }
        (_, &[a, b]) => match (point_ref(bodies, basis, a), point_ref(bodies, basis, b)) {
            (Some(x), Some(y)) => Some(vec![x, y]),
            _ => None,
        },
        _ => None,
    };
    got.unwrap_or_default()
}

/// What a reference stands for now: a point, a circle (centre and radius) or a line.
enum Now {
    Point(Point3),
    Circle(Point3, f64),
    Line(Seg),
}

fn resolve_ref(bodies: &[&Body], r: &ModelRef) -> Result<Now, String> {
    let kind = match r.kind {
        RefKind::Vertex => Kind::Vertex,
        RefKind::Edge => Kind::Edge,
    };
    let found = naming::resolve_one(bodies, kind, &r.name, r.signature.as_ref(), Want::One)
        .map_err(|e| e.to_string())?;
    let body = bodies
        .get(found.body)
        .ok_or_else(|| format!("{:?} is not in this view", r.name))?;
    let gone = || format!("{:?} has no {:?} any more", r.name, r.at);
    match (found.entity, r.at) {
        (Entity::Vertex(v), RefPoint::Point) => body
            .vertex_point(v)
            .map(Now::Point)
            .map_err(|e| e.to_string()),
        (Entity::Edge(e), at) => {
            let (curve, t0, t1) = body
                .edge_curve(e)
                .map_err(|x| x.to_string())?
                .ok_or_else(gone)?;
            match at {
                RefPoint::Start => Ok(Now::Point(curve.point_at(t0))),
                RefPoint::End => Ok(Now::Point(curve.point_at(t1))),
                RefPoint::Centre => circle_of(&curve)
                    .map(|(c, _, r)| Now::Circle(c, r))
                    .ok_or_else(gone),
                RefPoint::Line => segment_of(&curve, t0, t1).map(Now::Line).ok_or_else(gone),
                RefPoint::Point => Err(gone()),
            }
        }
        _ => Err(gone()),
    }
}

/// Where a dimension's points are now (view coordinates), from its references; `stored`
/// are the points it keeps. `Err` says why it is dangling.
pub fn resolve_points(
    kind: DimensionKind,
    stored: &[Point2],
    refs: &[ModelRef],
    basis: &Basis,
    bodies: &[&Body],
) -> Result<Vec<Point2>, String> {
    if refs.is_empty() {
        return Ok(stored.to_vec());
    }
    let now: Vec<Now> = refs
        .iter()
        .map(|r| resolve_ref(bodies, r))
        .collect::<Result<_, _>>()?;
    let pt = |n: &Now| match n {
        Now::Point(p) | Now::Circle(p, _) => Some(basis.project(*p)),
        Now::Line(_) => None,
    };
    let bad = || "a reference no longer fits the dimension".to_owned();
    match (kind, now.as_slice()) {
        (DimensionKind::Diameter | DimensionKind::Radius, [Now::Circle(c, r)]) => {
            let c2 = basis.project(*c);
            let u = match stored {
                [a, b] => (*b - *a).normalized(),
                _ => None,
            }
            .unwrap_or(chamfera_geom::Vec2::new(
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ));
            Ok(vec![c2, c2 + u * *r])
        }
        (DimensionKind::Angular, [Now::Line(l1), Now::Line(l2)]) => {
            let (a1, b1) = (basis.project(l1.0), basis.project(l1.1));
            let (a2, b2) = (basis.project(l2.0), basis.project(l2.1));
            let (d1, d2) = (b1 - a1, b2 - a2);
            let den = d1.cross(d2);
            if den.abs() <= 1e-9 * d1.length() * d2.length() {
                return Err("the two edges are parallel now".into());
            }
            let v = a1 + d1 * ((a2 - a1).cross(d2) / den);
            let leg = |a: Point2, b: Point2, old: Option<&Point2>| {
                let d = b - a;
                let l2 = d.dot(d);
                let q = old.map(|o| a + d * ((*o - a).dot(d) / l2).clamp(0.0, 1.0));
                match q {
                    Some(q) if q.distance(v) > 1e-6 => q,
                    _ if a.distance(v) > b.distance(v) => a,
                    _ => b,
                }
            };
            Ok(vec![
                v,
                leg(a1, b1, stored.get(1)),
                leg(a2, b2, stored.get(2)),
            ])
        }
        (_, [a, b]) => match (pt(a), pt(b)) {
            (Some(a), Some(b)) => Ok(vec![a, b]),
            _ => Err(bad()),
        },
        _ => Err(bad()),
    }
}

/// A centre line's two circle centres now, from its references (or its stored points).
pub fn resolve_centres(
    stored: &[Point2; 2],
    refs: &[ModelRef],
    basis: &Basis,
    bodies: &[&Body],
) -> Result<[Point2; 2], String> {
    if refs.is_empty() {
        return Ok(*stored);
    }
    let mut out = *stored;
    for (i, r) in refs.iter().take(2).enumerate() {
        match resolve_ref(bodies, r)? {
            Now::Circle(c, _) | Now::Point(c) => {
                if let Some(o) = out.get_mut(i) {
                    *o = basis.project(c);
                }
            }
            Now::Line(_) => return Err("a centre line needs two circles".into()),
        }
    }
    Ok(out)
}

/// References for a centre line between the circles centred at `a` and `b`.
pub fn centre_refs(
    a: (Point2, f64),
    b: (Point2, f64),
    basis: &Basis,
    bodies: &[&Body],
) -> Vec<ModelRef> {
    match (
        circle_ref(bodies, basis, a.0, a.1),
        circle_ref(bodies, basis, b.0, b.1),
    ) {
        (Some(x), Some(y)) => vec![x, y],
        _ => Vec::new(),
    }
}

/// The reference of the circle centred at `c` with radius `r` (view coordinates), for a
/// hole callout.
pub fn hole_ref(c: Point2, r: f64, basis: &Basis, bodies: &[&Body]) -> Option<ModelRef> {
    circle_ref(bodies, basis, c, r)
}

/// A hole callout's circle now (view coordinates: centre, radius), from its reference;
/// `None` radius when it keeps its stored centre.
pub fn resolve_circle(
    stored: Point2,
    refs: &[ModelRef],
    basis: &Basis,
    bodies: &[&Body],
) -> Result<(Point2, Option<f64>), String> {
    let Some(r) = refs.first() else {
        return Ok((stored, None));
    };
    match resolve_ref(bodies, r)? {
        Now::Circle(c, radius) => Ok((basis.project(c), Some(radius))),
        Now::Point(c) => Ok((basis.project(c), None)),
        Now::Line(_) => Err("a hole callout needs a circle".into()),
    }
}

/// The hole feature a reference name is made by: names of the faces and edges a Hole
/// feature makes hold `<feature>:hole <k> ...` (`between 5:hole 0 wall and 1:top`).
pub fn hole_feature(name: &str) -> Option<u32> {
    let mut rest = name;
    while let Some(i) = rest.find(":hole ") {
        let (before, after) = (rest.get(..i)?, rest.get(i + ":hole ".len()..)?);
        let digits: String = before
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
        if !digits.is_empty()
            && after.chars().next().is_some_and(|c| c.is_ascii_digit())
            && let Ok(id) = digits.parse::<u32>()
        {
            return Some(id);
        }
        rest = after;
    }
    None
}

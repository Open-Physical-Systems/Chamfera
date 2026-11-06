//! Persistent naming (rung K8): how a stored reference finds its face, edge or vertex
//! again after the design rebuilds.
//!
//! Every entity a feature makes carries a name derived from what made it: the feature id
//! and the role of the profile curve or operand face (`2:top`, `2:side 0`, `4:fillet of
//! 2:top 0`, `2:top x 5:side 1`). This is the generative naming of Capoyleas, Chen and
//! Hoffmann ("Generic naming in generative, constraint-based design", CAD 28(1), 1996) and
//! of Kripac ("A mechanism for persistently naming topological entities in history-based
//! parametric solid models", CAD 29(2), 1997): a name records the history of an entity,
//! not its index in one particular build.
//!
//! Two problems remain with history names alone, and this module handles both:
//!
//! - **Splits.** A cut can divide a face or edge into several pieces that all inherit its
//!   name. Kripac resolves a reference to such a face to the *set* of its descendants, and
//!   asks for more information when a single one is needed. Here the pieces get unique
//!   names `<name>#1`, `<name>#2`, ... ordered by a geometric key (their anchor point,
//!   compared x, then y, then z), and a reference can carry a [`Signature`] that records
//!   where the entity was when it was picked. A reference to the whole entity (no `#k`)
//!   means every piece to a consumer that takes many (a fillet, a shell), and the piece
//!   containing the signature's point to a consumer that needs one. When neither the point
//!   nor an index decides, the resolver says so ("the face "2:top" split into 2 pieces;
//!   pick one: ...") instead of choosing silently.
//! - **Renames.** When an upstream edit changes the topology, an entity can come out of a
//!   boolean under another name. Marcheix and Pierra ("A survey of the persistent naming
//!   problem", ACM Solid Modeling 2002) classify the remedies; we use the geometric one as
//!   a fallback: an entity that is no longer found by name is found again by its
//!   signature (the same kind of surface or curve, the same normal or axis, through the
//!   recorded point, sharing a neighbouring face name).
//!
//! Every consumer (sketches on faces, extrudes to a face, holes, fillets, shells,
//! construction geometry, joints, projections, face edits, sheet metal, CAM and the
//! measuring tools) goes through [`resolve`], so all of them follow the same rule.

use chamfera_geom::{MAX_COORDINATE, Point3, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId, Entity, FaceId, Surface, VertexId};
use serde::{Deserialize, Serialize};

/// What separates a name from its piece number: `2:top#2`.
pub const PIECE_SEPARATOR: char = '#';
/// Longest reference a design may store (shared with projections).
pub const MAX_REFERENCE_LEN: usize = crate::MAX_EDGE_REF_LEN;
/// Most neighbouring face names one signature keeps.
pub const MAX_NEIGHBOURS: usize = 64;
/// Most signatures one timeline item may keep.
pub const MAX_SIGNATURES_PER_ITEM: usize = 4096;
/// A piece that does not contain the reference point wins only when it is at most this
/// fraction of the distance of the next nearest piece; closer calls are ambiguous.
pub const NEAREST_PIECE_RATIO: f64 = 0.5;
/// How far a recorded point may lie from an entity and still count as on it. Points are
/// stored as decimal text and recomputed after rebuilds, so this is looser than
/// `LINEAR_TOLERANCE` by the error of one rebuild's arithmetic.
pub const ON_ENTITY_TOLERANCE: f64 = 1e-6;
/// Two unit directions closer than this (1 - cos of their angle) count as the same, for
/// the same reason as [`ON_ENTITY_TOLERANCE`].
pub const DIRECTION_TOLERANCE: f64 = 1e-9;

/// What a reference remembers about the entity it was made on, to tell split pieces apart
/// and to find the entity again if its name changes. Every field is optional, so a
/// reference without one (every design before format 27) resolves by name alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    /// The surface or curve type: `plane`, `cylinder`, `cone`, `sphere`, `torus`,
    /// `extrusion`; `line`, `circle`, `ellipse`, `spline`; `point` for a vertex.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shape: String,
    /// The outward normal of a plane, the axis of a cylinder, cone or torus, the direction
    /// of a straight edge or the axis of a circle or ellipse (unit length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
    /// A point on the entity where it was picked (mm, world coordinates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<[f64; 3]>,
    /// The names of the faces next to it, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub neighbours: Vec<String>,
}

impl Signature {
    /// Refuses what a design file must not hold: non-finite or out-of-range numbers,
    /// over-long names, too many neighbours.
    pub fn validate(&self) -> Result<(), String> {
        if self.shape.chars().count() > 32 {
            return Err("the shape name is longer than 32 characters".into());
        }
        if let Some(d) = self.direction
            && !(d.iter().all(|x| x.is_finite() && x.abs() <= 1.0 + 1e-6) && unit(d).is_some())
        {
            return Err("the direction must be a finite unit vector".into());
        }
        if let Some(p) = self.point
            && !p.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE)
        {
            return Err(format!(
                "the point must be finite and within {MAX_COORDINATE} mm"
            ));
        }
        if self.neighbours.len() > MAX_NEIGHBOURS {
            return Err(format!("at most {MAX_NEIGHBOURS} neighbours"));
        }
        if self
            .neighbours
            .iter()
            .any(|n| n.chars().count() > MAX_REFERENCE_LEN)
        {
            return Err("a neighbour name is too long".into());
        }
        Ok(())
    }

    /// The recorded point, if it is usable.
    pub fn anchor(&self) -> Option<Point3> {
        let p = self.point?;
        p.iter()
            .all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE)
            .then(|| Point3::new(p[0], p[1], p[2]))
    }

    /// The recorded direction as a unit vector, if it is usable.
    pub fn axis(&self) -> Option<Vec3> {
        unit(self.direction?)
    }
}

fn unit(d: [f64; 3]) -> Option<Vec3> {
    if !d.iter().all(|x| x.is_finite()) {
        return None;
    }
    Vec3::new(d[0], d[1], d[2]).normalized()
}

/// The kind of entity a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Face,
    Edge,
    Vertex,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Face => "face",
            Kind::Edge => "edge",
            Kind::Vertex => "vertex",
        }
    }
}

/// What the consumer of a reference can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Exactly one entity (a joint origin, a construction point, a measurement).
    One,
    /// One surface: several pieces are fine when they all lie on one plane or cylinder
    /// (a sketch plane, the plane of a hole, an extrude up to a face).
    Surface,
    /// Every piece of the named entity (a fillet rounds every piece of a split edge, a
    /// shell opens every piece of a split face).
    All,
}

/// An entity found in one of the bodies given to [`resolve`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    /// The index of its body in the list given.
    pub body: usize,
    pub entity: Entity,
}

impl Found {
    pub fn face(&self) -> Option<FaceId> {
        match self.entity {
            Entity::Face(f) => Some(f),
            _ => None,
        }
    }
    pub fn edge(&self) -> Option<EdgeId> {
        match self.entity {
            Entity::Edge(e) => Some(e),
            _ => None,
        }
    }
    pub fn vertex(&self) -> Option<VertexId> {
        match self.entity {
            Entity::Vertex(v) => Some(v),
            _ => None,
        }
    }
}

/// Why a reference did not resolve.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NameError {
    #[error("the {kind} reference {reference:?} is not a valid name")]
    BadReference {
        kind: &'static str,
        reference: String,
    },
    #[error("the {kind} {reference:?} is no longer there")]
    Gone {
        kind: &'static str,
        reference: String,
    },
    #[error(
        "the {kind} {reference:?} split into {} pieces; pick one: {}",
        pieces.len(),
        pieces.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", ")
    )]
    Split {
        kind: &'static str,
        reference: String,
        pieces: Vec<String>,
    },
    #[error("the {kind} {reference:?} has {count} pieces now, not {piece}")]
    NoSuchPiece {
        kind: &'static str,
        reference: String,
        piece: usize,
        count: usize,
    },
}

/// How a fillet or chamfer refers to an edge: its name (`2:top 0`), or for an edge
/// without one, its two faces (`between 2:side 0 and 2:top`).
pub fn edge_ref(body: &Body, e: EdgeId) -> Option<String> {
    if let Some(n) = body.edge_name(e).ok().flatten() {
        return Some(n.to_owned());
    }
    let [a, b] = body.edge_faces(e).ok()?;
    let mut names = [
        body.face_name(a).ok().flatten()?.to_owned(),
        body.face_name(b).ok().flatten()?.to_owned(),
    ];
    names.sort();
    Some(format!("between {} and {}", names[0], names[1]))
}

/// The name an entity inherited from its history (shared by all pieces of a split one).
pub fn base_name(body: &Body, entity: Entity) -> Option<String> {
    match entity {
        Entity::Face(f) => body.face_name(f).ok().flatten().map(str::to_owned),
        Entity::Edge(e) => edge_ref(body, e),
        Entity::Vertex(v) => body.vertex_name(v).ok().flatten().map(str::to_owned),
        Entity::Point(_) | Entity::Body => None,
    }
}

/// Splits `2:top#2` into (`2:top`, Some(2)); a name without a piece number is whole.
pub fn split_piece(reference: &str) -> (&str, Option<usize>) {
    if let Some((base, k)) = reference.rsplit_once(PIECE_SEPARATOR)
        && !base.is_empty()
        && !k.is_empty()
        && k.len() <= 6
        && k.bytes().all(|b| b.is_ascii_digit())
        && let Ok(n) = k.parse::<usize>()
        && n >= 1
    {
        return (base, Some(n));
    }
    (reference, None)
}

/// `2:top` and piece 2 as `2:top#2`.
pub fn piece_name(base: &str, piece: usize) -> String {
    format!("{base}{PIECE_SEPARATOR}{piece}")
}

fn entities(body: &Body, kind: Kind) -> Vec<Entity> {
    match kind {
        Kind::Face => body.faces().into_iter().map(Entity::Face).collect(),
        Kind::Edge => body.edges().into_iter().map(Entity::Edge).collect(),
        Kind::Vertex => body.vertices().into_iter().map(Entity::Vertex).collect(),
    }
}

fn entity_index(e: Entity) -> usize {
    match e {
        Entity::Face(f) => f.index(),
        Entity::Edge(x) => x.index(),
        Entity::Vertex(v) => v.index(),
        Entity::Point(_) | Entity::Body => usize::MAX,
    }
}

/// A point on the entity that stands for it: a vertex's position, an edge's parameter
/// midpoint, and for a face the point on it nearest the average of its vertices.
pub fn anchor_point(body: &Body, entity: Entity) -> Option<Point3> {
    match entity {
        Entity::Vertex(v) => body.vertex_point(v).ok(),
        Entity::Edge(e) => {
            let (c, t0, t1) = body.edge_curve(e).ok()??;
            let p = c.point_at(0.5 * (t0 + t1));
            p.is_valid().then_some(p)
        }
        Entity::Face(f) => {
            let mut sum = Vec3::new(0.0, 0.0, 0.0);
            let mut n = 0usize;
            for l in body.face_loops(f).ok()? {
                for v in body.loop_vertices(l).ok()? {
                    let p = body.vertex_point(v).ok()?;
                    sum = sum + (p - Point3::ORIGIN);
                    n += 1;
                }
            }
            if n == 0 {
                return None;
            }
            let avg = Point3::ORIGIN + sum * (1.0 / n as f64);
            let c = chamfera_kernel::closest_points(body, Entity::Point(avg), body, entity).ok()?;
            c.on_b.is_valid().then_some(c.on_b)
        }
        Entity::Point(_) | Entity::Body => None,
    }
}

/// The distance from `p` to the entity, or infinity when it cannot be measured.
fn distance_to(body: &Body, entity: Entity, p: Point3) -> f64 {
    chamfera_kernel::closest_points(body, Entity::Point(p), body, entity)
        .map(|c| c.distance)
        .ok()
        .filter(|d| d.is_finite())
        .unwrap_or(f64::INFINITY)
}

/// The ordering key of a piece: its anchor on a 1e-6 mm grid, then where it is.
fn order_key(bodies: &[&Body], f: Found) -> ([i64; 3], usize, usize) {
    let grid = |x: f64| {
        let q = (x / 1e-6).round();
        if q.is_finite() && q.abs() < 9.0e15 {
            q as i64
        } else {
            i64::MAX
        }
    };
    let a = bodies
        .get(f.body)
        .and_then(|b| anchor_point(b, f.entity))
        .map_or([i64::MAX; 3], |p| [grid(p.x), grid(p.y), grid(p.z)]);
    (a, f.body, entity_index(f.entity))
}

/// Every entity of `kind` whose inherited name is `base`, in piece order.
pub fn pieces(bodies: &[&Body], kind: Kind, base: &str) -> Vec<Found> {
    let mut out = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        for e in entities(b, kind) {
            if base_name(b, e).as_deref() == Some(base) {
                out.push(Found { body: i, entity: e });
            }
        }
    }
    if out.len() > 1 {
        let mut keyed: Vec<_> = out.into_iter().map(|f| (order_key(bodies, f), f)).collect();
        keyed.sort_by_key(|(k, _)| *k);
        out = keyed.into_iter().map(|(_, f)| f).collect();
    }
    out
}

/// The unique name of an entity: its inherited name, with `#k` when that name is shared
/// by several pieces in `bodies`.
pub fn reference_name(bodies: &[&Body], found: Found) -> Option<String> {
    let base = base_name(bodies.get(found.body)?, found.entity)?;
    let all = pieces(bodies, kind_of(found.entity)?, &base);
    if all.len() <= 1 {
        return Some(base);
    }
    let k = all.iter().position(|f| *f == found)?;
    Some(piece_name(&base, k + 1))
}

/// Every named entity of `kind` with its unique name (see [`reference_name`]).
pub fn reference_names(bodies: &[&Body], kind: Kind) -> Vec<(Found, String)> {
    let mut groups: std::collections::BTreeMap<String, Vec<Found>> = Default::default();
    for (i, b) in bodies.iter().enumerate() {
        for e in entities(b, kind) {
            if let Some(n) = base_name(b, e) {
                groups
                    .entry(n)
                    .or_default()
                    .push(Found { body: i, entity: e });
            }
        }
    }
    let mut out = Vec::new();
    for (base, list) in groups {
        if list.len() == 1 {
            out.extend(list.into_iter().map(|f| (f, base.clone())));
        } else {
            for (k, f) in pieces(bodies, kind, &base).into_iter().enumerate() {
                out.push((f, piece_name(&base, k + 1)));
            }
        }
    }
    out
}

fn kind_of(e: Entity) -> Option<Kind> {
    match e {
        Entity::Face(_) => Some(Kind::Face),
        Entity::Edge(_) => Some(Kind::Edge),
        Entity::Vertex(_) => Some(Kind::Vertex),
        Entity::Point(_) | Entity::Body => None,
    }
}

/// The type and direction of a face's surface or an edge's curve.
fn shape_of(body: &Body, entity: Entity) -> (String, Option<Vec3>) {
    match entity {
        Entity::Face(f) => match body.face_surface(f).ok().flatten() {
            Some((Surface::Plane(frame), same)) => {
                let n = frame.z();
                ("plane".into(), Some(if same { n } else { n * -1.0 }))
            }
            Some((Surface::Cylinder(c), _)) => ("cylinder".into(), Some(c.frame().z())),
            Some((Surface::Cone(c), _)) => ("cone".into(), Some(c.frame().z())),
            Some((Surface::Sphere(_), _)) => ("sphere".into(), None),
            Some((Surface::Torus(t), _)) => ("torus".into(), Some(t.frame().z())),
            Some((Surface::Extrusion(_), _)) => ("extrusion".into(), None),
            None => (String::new(), None),
        },
        Entity::Edge(e) => match body.edge_curve(e).ok().flatten() {
            Some((Curve::Line(l), _, _)) => ("line".into(), Some(l.direction())),
            Some((Curve::Segment(s), _, _)) => ("line".into(), (s.end() - s.start()).normalized()),
            Some((Curve::Circle(c), _, _)) => ("circle".into(), Some(c.normal())),
            Some((Curve::Arc(a), _, _)) => ("circle".into(), Some(a.circle().normal())),
            Some((Curve::Ellipse(x), _, _)) => ("ellipse".into(), Some(x.frame().z())),
            Some((Curve::EllipticalArc(_), _, _)) => ("ellipse".into(), None),
            Some((Curve::BSpline(_), _, _)) => ("spline".into(), None),
            None => (String::new(), None),
        },
        Entity::Vertex(_) => ("point".into(), None),
        Entity::Point(_) | Entity::Body => (String::new(), None),
    }
}

/// The sorted names of the faces next to an entity: across a face's edges, on both sides
/// of an edge, around a vertex.
fn neighbours_of(body: &Body, entity: Entity) -> Vec<String> {
    let mut faces = Vec::new();
    match entity {
        Entity::Face(f) => {
            for e in body.face_edges(f).unwrap_or_default() {
                if let Ok(pair) = body.edge_faces(e) {
                    faces.extend(pair.into_iter().filter(|x| *x != f));
                }
            }
        }
        Entity::Edge(e) => faces.extend(body.edge_faces(e).into_iter().flatten()),
        Entity::Vertex(v) => faces.extend(body.vertex_faces(v).unwrap_or_default()),
        Entity::Point(_) | Entity::Body => {}
    }
    let mut names: Vec<String> = faces
        .into_iter()
        .filter_map(|f| body.face_name(f).ok().flatten().map(str::to_owned))
        .collect();
    names.sort();
    names.dedup();
    names.truncate(MAX_NEIGHBOURS);
    names
}

/// The signature of an entity: its shape, direction and neighbours, and `point` (where
/// it was picked) if that lies on it, else its anchor point.
pub fn signature_of(bodies: &[&Body], found: Found, point: Option<Point3>) -> Option<Signature> {
    let body = bodies.get(found.body)?;
    let (shape, direction) = shape_of(body, found.entity);
    let on = point
        .filter(|p| p.is_valid() && distance_to(body, found.entity, *p) <= ON_ENTITY_TOLERANCE);
    let p = on.or_else(|| anchor_point(body, found.entity));
    let sig = Signature {
        shape,
        direction: direction.map(|d| [d.x, d.y, d.z]),
        point: p.map(|p| [p.x, p.y, p.z]),
        neighbours: neighbours_of(body, found.entity),
    };
    sig.validate().is_ok().then_some(sig)
}

/// The piece containing `p`, or failing that the clearly nearest one.
fn piece_at(bodies: &[&Body], list: &[Found], p: Point3) -> Option<Found> {
    let mut d: Vec<(f64, Found)> = list
        .iter()
        .map(|f| {
            let dist = bodies
                .get(f.body)
                .map_or(f64::INFINITY, |b| distance_to(b, f.entity, p));
            (dist, *f)
        })
        .collect();
    let containing: Vec<Found> = d
        .iter()
        .filter(|(x, _)| *x <= ON_ENTITY_TOLERANCE)
        .map(|(_, f)| *f)
        .collect();
    match containing.len() {
        1 => return containing.first().copied(),
        0 => {}
        // On the boundary between pieces: neither is the one.
        _ => return None,
    }
    d.sort_by(|a, b| a.0.total_cmp(&b.0));
    match (d.first(), d.get(1)) {
        (Some((a, f)), Some((b, _))) if a.is_finite() && *a <= NEAREST_PIECE_RATIO * *b => Some(*f),
        (Some((a, f)), None) if a.is_finite() => Some(*f),
        _ => None,
    }
}

fn surface_of(bodies: &[&Body], f: Found) -> Option<Surface> {
    bodies
        .get(f.body)?
        .face_surface(f.face()?)
        .ok()
        .flatten()
        .map(|(s, _)| s)
}

/// True when every face in `list` lies on one plane (with the same outward side) or on
/// one cylinder.
fn one_surface(bodies: &[&Body], list: &[Found]) -> bool {
    let Some(first) = list.first() else {
        return false;
    };
    let Some(body) = bodies.get(first.body) else {
        return false;
    };
    let (shape, dir) = shape_of(body, first.entity);
    let Some(s0) = surface_of(bodies, *first) else {
        return false;
    };
    list.iter().all(|f| {
        let Some(b) = bodies.get(f.body) else {
            return false;
        };
        let (sh, d) = shape_of(b, f.entity);
        let same_dir = match (dir, d) {
            (Some(a), Some(b)) => 1.0 - a.dot(b).abs() <= DIRECTION_TOLERANCE,
            _ => false,
        };
        let Some(s) = surface_of(bodies, *f) else {
            return false;
        };
        sh == shape
            && same_dir
            && match (&s0, &s) {
                (Surface::Plane(a), Surface::Plane(b)) => {
                    // Same outward normal (not just parallel) and the same plane.
                    dir.zip(d).is_some_and(|(x, y)| x.dot(y) > 0.0)
                        && (b.origin() - a.origin()).dot(a.z()).abs() <= ON_ENTITY_TOLERANCE
                }
                (Surface::Cylinder(a), Surface::Cylinder(b)) => {
                    (a.radius() - b.radius()).abs() <= ON_ENTITY_TOLERANCE
                        && (b.frame().origin() - a.frame().origin())
                            .cross(a.frame().z())
                            .length()
                            <= ON_ENTITY_TOLERANCE
                }
                _ => false,
            }
    })
}

/// Finds an entity whose name changed by its signature: the same shape and direction,
/// through the recorded point, next to at least one recorded neighbour.
fn refind(bodies: &[&Body], kind: Kind, sig: &Signature) -> Option<Found> {
    let p = sig.anchor()?;
    let want_dir = sig.axis();
    let mut best: Vec<(usize, Found)> = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        for e in entities(b, kind) {
            let (shape, dir) = shape_of(b, e);
            if !sig.shape.is_empty() && shape != sig.shape {
                continue;
            }
            if let Some(w) = want_dir {
                let ok = dir.is_some_and(|d| {
                    let c = w.dot(d);
                    // A plane's normal keeps its sign; an axis may flip.
                    if shape == "plane" {
                        1.0 - c <= DIRECTION_TOLERANCE
                    } else {
                        1.0 - c.abs() <= DIRECTION_TOLERANCE
                    }
                });
                if !ok {
                    continue;
                }
            }
            if distance_to(b, e, p) > ON_ENTITY_TOLERANCE {
                continue;
            }
            let shared = if sig.neighbours.is_empty() {
                0
            } else {
                let n = neighbours_of(b, e);
                let shared = n.iter().filter(|x| sig.neighbours.contains(x)).count();
                if shared == 0 {
                    continue;
                }
                shared
            };
            best.push((shared, Found { body: i, entity: e }));
        }
    }
    best.sort_by_key(|a| std::cmp::Reverse(a.0));
    match (best.first(), best.get(1)) {
        (Some((a, f)), Some((b, _))) if a > b => Some(*f),
        (Some((_, f)), None) => Some(*f),
        _ => None,
    }
}

/// Finds what `reference` names in `bodies`: the central resolver every consumer uses.
///
/// - A name found once is that entity, as before format 27.
/// - A name shared by several pieces: a whole reference gives all of them to [`Want::All`];
///   otherwise the piece containing the signature's point (or the clearly nearest) wins,
///   then the piece number `#k` of the reference, then for [`Want::Surface`] all pieces
///   if they lie on one surface. Anything else is [`NameError::Split`], naming the pieces.
/// - A name not found at all is looked for by its signature (see the module notes).
pub fn resolve(
    bodies: &[&Body],
    kind: Kind,
    reference: &str,
    signature: Option<&Signature>,
    want: Want,
) -> Result<Vec<Found>, NameError> {
    let word = kind.word();
    if reference.is_empty()
        || reference.chars().count() > MAX_REFERENCE_LEN
        || reference.chars().any(char::is_control)
    {
        return Err(NameError::BadReference {
            kind: word,
            reference: reference.chars().take(64).collect(),
        });
    }
    // A name that really ends in `#k` is taken as it is.
    let exact = pieces(bodies, kind, reference);
    let (base, piece, list) = if exact.is_empty() {
        let (base, piece) = split_piece(reference);
        (base, piece, pieces(bodies, kind, base))
    } else {
        (reference, None, exact)
    };
    let signature = signature.filter(|s| s.validate().is_ok());
    if list.is_empty() {
        if let Some(f) = signature.and_then(|s| refind(bodies, kind, s)) {
            return Ok(vec![f]);
        }
        return Err(NameError::Gone {
            kind: word,
            reference: reference.to_owned(),
        });
    }
    if list.len() == 1 {
        return Ok(list);
    }
    if piece.is_none() && want == Want::All {
        return Ok(list);
    }
    let split = || NameError::Split {
        kind: word,
        reference: base.to_owned(),
        pieces: (1..=list.len()).map(|k| piece_name(base, k)).collect(),
    };
    let surface = want == Want::Surface && kind == Kind::Face && one_surface(bodies, &list);
    if let Some(p) = signature.and_then(Signature::anchor) {
        return match piece_at(bodies, &list, p) {
            Some(f) => Ok(vec![f]),
            None if surface => Ok(list),
            None => Err(split()),
        };
    }
    if let Some(k) = piece {
        return list
            .get(k - 1)
            .map(|f| vec![*f])
            .ok_or(NameError::NoSuchPiece {
                kind: word,
                reference: base.to_owned(),
                piece: k,
                count: list.len(),
            });
    }
    if surface {
        return Ok(list);
    }
    Err(split())
}

/// [`resolve`] for one entity: the first of what it found.
pub fn resolve_one(
    bodies: &[&Body],
    kind: Kind,
    reference: &str,
    signature: Option<&Signature>,
    want: Want,
) -> Result<Found, NameError> {
    resolve(bodies, kind, reference, signature, want)?
        .into_iter()
        .next()
        .ok_or(NameError::Gone {
            kind: kind.word(),
            reference: reference.to_owned(),
        })
}

/// Checks the signatures a design file holds: each belongs to a sketch or feature in it,
/// within the limits, with valid numbers.
pub(crate) fn check(
    references: &std::collections::BTreeMap<u32, std::collections::BTreeMap<String, Signature>>,
    items: &std::collections::BTreeSet<u32>,
) -> Result<(), crate::DocError> {
    let bad = |reason: String| crate::DocError::BadFeature { reason };
    for (owner, map) in references {
        if !items.contains(owner) {
            return Err(bad(format!(
                "references of item {owner}, which is not in the design"
            )));
        }
        if map.len() > MAX_SIGNATURES_PER_ITEM {
            return Err(bad(format!(
                "item {owner} keeps more than {MAX_SIGNATURES_PER_ITEM} references"
            )));
        }
        for (name, sig) in map {
            if name.is_empty() || name.chars().count() > MAX_REFERENCE_LEN {
                return Err(bad(format!(
                    "item {owner} has a reference name that is empty or too long"
                )));
            }
            sig.validate()
                .map_err(|e| bad(format!("item {owner}, reference {name:?}: {e}")))?;
        }
    }
    Ok(())
}

impl crate::Document {
    /// The signature item `owner` keeps for `reference`, if any.
    pub fn signature(&self, owner: u32, reference: &str) -> Option<&Signature> {
        self.references.get(&owner)?.get(reference)
    }

    /// Records what `reference` of item `owner` was made on. Refuses an invalid signature
    /// or one over the limits.
    pub fn set_signature(
        &mut self,
        owner: u32,
        reference: &str,
        signature: Signature,
    ) -> Result<(), crate::DocError> {
        let bad = |reason: String| crate::DocError::BadFeature { reason };
        signature.validate().map_err(bad)?;
        if reference.is_empty() || reference.chars().count() > MAX_REFERENCE_LEN {
            return Err(bad("the reference name is empty or too long".into()));
        }
        let map = self.references.entry(owner).or_default();
        if !map.contains_key(reference) && map.len() >= MAX_SIGNATURES_PER_ITEM {
            return Err(bad(format!(
                "an item keeps at most {MAX_SIGNATURES_PER_ITEM} references"
            )));
        }
        map.insert(reference.to_owned(), signature);
        Ok(())
    }

    /// Keeps only the signatures of `owner` whose names are in `keep` (after an edit
    /// changed what the item refers to).
    pub fn retain_signatures(&mut self, owner: u32, keep: &[&str]) {
        if let Some(map) = self.references.get_mut(&owner) {
            map.retain(|k, _| keep.contains(&k.as_str()));
            if map.is_empty() {
                self.references.remove(&owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Vec3;

    fn named_box(min: Point3, feature: &str) -> Body {
        let mut b = Body::cuboid(min, Vec3::new(10.0, 10.0, 10.0)).unwrap();
        for f in b.faces() {
            let n = b.face_name(f).unwrap().map(str::to_owned);
            if let Some(n) = n {
                b.set_face_name(f, &format!("{feature}:{n}")).unwrap();
            }
        }
        b
    }

    fn top_of(b: &Body) -> FaceId {
        *b.faces()
            .iter()
            .find(|f| {
                b.face_name(**f)
                    .unwrap()
                    .is_some_and(|n| n.ends_with("top"))
            })
            .unwrap()
    }

    #[test]
    fn piece_names_split_and_join() {
        assert_eq!(split_piece("2:top#2"), ("2:top", Some(2)));
        assert_eq!(split_piece("2:top"), ("2:top", None));
        assert_eq!(split_piece("2:top#0"), ("2:top#0", None));
        assert_eq!(split_piece("#3"), ("#3", None));
        assert_eq!(split_piece("2:top#x"), ("2:top#x", None));
        assert_eq!(piece_name("2:top", 3), "2:top#3");
    }

    #[test]
    fn pieces_of_one_name_are_ordered_by_x_and_found_by_their_point() {
        // Two unit-10 boxes whose faces carry the same names, at x 0..10 and x 20..30:
        // the tops are 2:top#1 (x 0..10) and 2:top#2 (x 20..30).
        let a = named_box(Point3::new(20.0, 0.0, 0.0), "2");
        let b = named_box(Point3::new(0.0, 0.0, 0.0), "2");
        let bodies = [&a, &b];
        let names = reference_names(&bodies, Kind::Face);
        let top1 = names.iter().find(|(_, n)| n == "2:top#1").unwrap().0;
        assert_eq!(top1.body, 1, "the piece at x 0..10 comes first");
        // A whole reference gives every piece to a consumer that takes many ...
        assert_eq!(
            resolve(&bodies, Kind::Face, "2:top", None, Want::All)
                .unwrap()
                .len(),
            2
        );
        // ... and is ambiguous for one that needs a single face, naming the pieces.
        let e = resolve(&bodies, Kind::Face, "2:top", None, Want::One).unwrap_err();
        assert_eq!(
            e.to_string(),
            "the face \"2:top\" split into 2 pieces; pick one: \"2:top#1\", \"2:top#2\""
        );
        // Both tops lie on z = 10 facing up: one surface.
        assert_eq!(
            resolve(&bodies, Kind::Face, "2:top", None, Want::Surface)
                .unwrap()
                .len(),
            2
        );
        // A point decides: (25, 5, 10) is on the top of the box at x 20..30 (body 0).
        let sig = Signature {
            point: Some([25.0, 5.0, 10.0]),
            ..Signature::default()
        };
        let f = resolve_one(&bodies, Kind::Face, "2:top", Some(&sig), Want::One).unwrap();
        assert_eq!((f.body, f.face()), (0, Some(top_of(&a))));
        // A piece number without a point.
        let f = resolve_one(&bodies, Kind::Face, "2:top#1", None, Want::One).unwrap();
        assert_eq!((f.body, f.face()), (1, Some(top_of(&b))));
        assert!(matches!(
            resolve(&bodies, Kind::Face, "2:top#3", None, Want::One),
            Err(NameError::NoSuchPiece { count: 2, .. })
        ));
        // A point halfway between the pieces (x = 15) is equally far from both.
        let mid = Signature {
            point: Some([15.0, 5.0, 10.0]),
            ..Signature::default()
        };
        assert!(matches!(
            resolve(&bodies, Kind::Face, "2:top", Some(&mid), Want::One),
            Err(NameError::Split { .. })
        ));
        // A piece number of a face that is whole again is that face.
        let one = [&a];
        let f = resolve_one(&one, Kind::Face, "2:top#2", None, Want::One).unwrap();
        assert_eq!(f.face(), Some(top_of(&a)));
    }

    #[test]
    fn a_renamed_face_is_found_again_by_its_signature() {
        let a = named_box(Point3::new(0.0, 0.0, 0.0), "2");
        let bodies = [&a];
        let top = Found {
            body: 0,
            entity: Entity::Face(top_of(&a)),
        };
        let sig = signature_of(&bodies, top, Some(Point3::new(3.0, 4.0, 10.0))).unwrap();
        assert_eq!(sig.shape, "plane");
        assert_eq!(sig.direction, Some([0.0, 0.0, 1.0]));
        assert_eq!(sig.point, Some([3.0, 4.0, 10.0]));
        assert_eq!(sig.neighbours.len(), 4, "the four sides");
        // The same box under another feature's names: `9:top` is found for `2:old`
        // only by its signature (plane, +Z, through (3, 4, 10), next to the sides) once
        // the neighbours keep their names.
        let mut b = a.clone();
        b.set_face_name(top_of(&b), "9:top").unwrap();
        let bodies = [&b];
        let f = resolve_one(&bodies, Kind::Face, "2:old", Some(&sig), Want::One).unwrap();
        assert_eq!(f.face(), Some(top_of(&b)));
        assert!(matches!(
            resolve(&bodies, Kind::Face, "2:old", None, Want::One),
            Err(NameError::Gone { .. })
        ));
    }

    #[test]
    fn hostile_signatures_and_references_never_panic() {
        let a = named_box(Point3::new(0.0, 0.0, 0.0), "2");
        let bodies = [&a];
        let bad = [
            Signature {
                point: Some([f64::NAN, 0.0, 0.0]),
                ..Signature::default()
            },
            Signature {
                point: Some([1e300, -1e300, f64::INFINITY]),
                direction: Some([0.0, 0.0, 0.0]),
                ..Signature::default()
            },
            Signature {
                direction: Some([f64::NAN, 1.0, 0.0]),
                shape: "x".repeat(10_000),
                neighbours: vec!["y".repeat(100_000); 3],
                ..Signature::default()
            },
        ];
        for s in &bad {
            assert!(s.validate().is_err());
            for r in [
                "2:top",
                "2:nothing",
                "",
                "#",
                "2:top#99999999999",
                "2:top#0",
            ] {
                for k in [Kind::Face, Kind::Edge, Kind::Vertex] {
                    for w in [Want::One, Want::Surface, Want::All] {
                        let _ = resolve(&bodies, k, r, Some(s), w);
                    }
                }
            }
        }
        assert!(resolve(&bodies, Kind::Face, &"z".repeat(10_000), None, Want::One).is_err());
        assert!(resolve(&[], Kind::Face, "2:top", None, Want::One).is_err());
    }
}

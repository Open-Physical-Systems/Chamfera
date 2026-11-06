//! Building the timeline: each feature becomes kernel bodies, tessellated for the viewport.
//!
//! The design stores only what the user asked for (sketches, features, parameters); the
//! solids are worked out again from it after every change, so a parameter change rebuilds
//! everything downstream. A feature that cannot be built keeps its error (shown in the
//! timeline) and the rest of the timeline still builds, as CAD users expect.

use crate::Session;
use chamfera_doc::{Blend, BlendKind, Extrude, ExtrudeDirection, FeatureKind, Operation, Revolve};
use chamfera_geom::{Point2, Point3};
use chamfera_kernel::{Body, EdgeId, Line2, Profile, ProfileEdge, TessOptions, tessellate};
use chamfera_sketch::{Curve, ProfileLoop, Region, Sketch};
use std::collections::BTreeMap;

/// A solid built by a feature.
#[derive(Debug, Clone)]
pub struct Solid {
    pub feature: u32,
    pub name: String,
    pub body: Body,
    /// The scene body that shows it, if it could be displayed.
    pub scene_id: Option<u32>,
    /// The kernel face behind each displayed triangle (for picking faces).
    pub faces: Vec<chamfera_kernel::FaceId>,
    /// Each true edge as the polyline that is drawn (for picking edges).
    pub edges: Vec<(EdgeId, Vec<Point3>)>,
}

/// How far from an edge on screen, in pixels, a click still picks it.
pub const EDGE_PICK_PIXELS: f64 = 8.0;

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

/// Gives every edge of a feature's new body its full name, like [`name_faces`].
fn name_edges(body: &mut Body, feature: u32) {
    for edge in body.edges() {
        let role = body.edge_name(edge).ok().flatten().map(str::to_owned);
        if let Some(role) = role
            && split_face_key(&role).is_none()
        {
            let _ = body.set_edge_name(edge, &face_key(feature, &role));
        }
    }
}

/// Fillets or chamfers the named edges of the bodies built so far. Every name must still
/// be found; a body without any of them is left alone.
fn build_blend(
    doc: &chamfera_doc::Document,
    b: &Blend,
    feature: u32,
    made: &[(u32, String, Body)],
) -> Result<Vec<(u32, String, Body)>, String> {
    let size = doc
        .evaluate_feature_value(&b.size, chamfera_doc::expr::Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    let word = match b.blend {
        BlendKind::Fillet => "fillet",
        BlendKind::Chamfer => "chamfer",
    };
    let mut found = vec![false; b.edges.len()];
    let mut next = made.to_vec();
    for (_, _, body) in &mut next {
        let mut edges = Vec::new();
        for e in body.edges() {
            if let Some(r) = edge_ref(body, e)
                && let Some(i) = b.edges.iter().position(|x| *x == r)
            {
                edges.push(e);
                if let Some(f) = found.get_mut(i) {
                    *f = true;
                }
            }
        }
        if edges.is_empty() {
            continue;
        }
        let mut out = match b.blend {
            BlendKind::Fillet => chamfera_kernel::fillet(body, &edges, size),
            BlendKind::Chamfer => chamfera_kernel::chamfer(body, &edges, size),
        }
        .map_err(|x| format!("{word}: {x}"))?;
        name_faces(&mut out, feature);
        name_edges(&mut out, feature);
        *body = out;
    }
    if let Some(i) = found.iter().position(|f| !f) {
        let name = b.edges.get(i).map_or("", String::as_str);
        return Err(format!("{word}: the edge {name:?} is no longer there"));
    }
    Ok(next)
}

/// The plane of a flat face, with its normal pointing out of the solid.
pub fn face_plane(body: &Body, face: chamfera_kernel::FaceId) -> Option<chamfera_geom::Frame> {
    match body.face_surface(face).ok()?? {
        (chamfera_kernel::Surface::Plane(frame), same_sense) => {
            Some(if same_sense { frame } else { frame.flipped() })
        }
        _ => None,
    }
}

/// A face's full name in built bodies: the feature that made it, then its role, like
/// `2:top`. Booleans keep these names, so after a boss is joined onto a plate, the plate's
/// top is still `2:top` and the boss's is `4:top`.
pub fn face_key(feature: u32, role: &str) -> String {
    format!("{feature}:{role}")
}

/// Splits `2:top` into (2, "top").
pub fn split_face_key(name: &str) -> Option<(u32, &str)> {
    let (f, role) = name.split_once(':')?;
    Some((f.parse().ok()?, role))
}

/// Gives every face of a feature's new body its full name (see [`face_key`]).
fn name_faces(body: &mut Body, feature: u32) {
    for face in body.faces() {
        let role = body.face_name(face).ok().flatten().map(str::to_owned);
        if let Some(role) = role
            && split_face_key(&role).is_none()
        {
            let _ = body.set_face_name(face, &face_key(feature, &role));
        }
    }
}

/// True when `body` has a face that `feature` made (it made or changed the body).
pub fn body_has_feature(body: &Body, feature: u32) -> bool {
    body.faces().into_iter().any(|f| {
        body.face_name(f)
            .ok()
            .flatten()
            .and_then(split_face_key)
            .is_some_and(|(id, _)| id == feature)
    })
}

/// The plane of the face `r.face` of feature `r.feature`, from bodies built so far in this
/// rebuild (whichever body it ended up in after joins and cuts).
fn attached_plane(
    made: &[(u32, String, Body)],
    r: &chamfera_doc::FaceRef,
) -> Result<chamfera_geom::Frame, String> {
    let wanted = face_key(r.feature, &r.face);
    for (_, _, body) in made {
        for face in body.faces() {
            if body.face_name(face).ok().flatten() == Some(wanted.as_str()) {
                return face_plane(body, face)
                    .ok_or_else(|| format!("the face {:?} is not flat", r.face));
            }
        }
    }
    Err(format!(
        "the face {:?} it is drawn on is gone (feature {} changed or failed)",
        r.face, r.feature
    ))
}

/// What the last rebuild produced.
#[derive(Debug, Default, Clone)]
pub struct Built {
    pub solids: Vec<Solid>,
    /// Features that could not be built, and why.
    pub errors: BTreeMap<u32, String>,
    /// Features whose solids were built but cannot be shown yet, and why.
    pub display_errors: BTreeMap<u32, String>,
}

/// A sketch loop as exact kernel profile edges: lines stay lines, arcs and circles stay
/// arcs and circles, in the loop's direction.
fn loop_edges(sketch: &Sketch, l: &ProfileLoop) -> Result<Vec<ProfileEdge>, String> {
    let gone = || "a curve of the profile is gone".to_owned();
    l.segments
        .iter()
        .map(|seg| {
            let curve = sketch.curve(seg.curve).ok_or_else(gone)?;
            let edge = match curve {
                Curve::Line { start, end, .. } => {
                    let (a, b) = (
                        sketch.point(*start).ok_or_else(gone)?,
                        sketch.point(*end).ok_or_else(gone)?,
                    );
                    ProfileEdge::line(a, b)
                }
                Curve::Circle { centre, radius, .. } => {
                    ProfileEdge::circle(sketch.point(*centre).ok_or_else(gone)?, *radius)
                }
                Curve::Arc {
                    centre, start, end, ..
                } => ProfileEdge::arc_from_points(
                    sketch.point(*centre).ok_or_else(gone)?,
                    sketch.point(*start).ok_or_else(gone)?,
                    sketch.point(*end).ok_or_else(gone)?,
                )
                .map_err(|e| format!("an arc of the profile: {e}"))?,
                Curve::Spline { .. } => {
                    // A spline's own curve; a closed one (a loop by itself) as two halves,
                    // as the kernel wants every loop edge to have two ends.
                    let c = sketch
                        .spline_of(curve)
                        .ok_or_else(|| "a spline of the profile has no curve".to_owned())?;
                    let mut pieces = if c.is_closed() {
                        let (lo, hi) = c.domain();
                        let (a, b) = c
                            .split_at(0.5 * (lo + hi))
                            .map_err(|e| format!("a spline of the profile: {e}"))?;
                        vec![ProfileEdge::Spline(a), ProfileEdge::Spline(b)]
                    } else {
                        vec![ProfileEdge::Spline(c)]
                    };
                    if seg.reversed {
                        pieces.reverse();
                        for p in &mut pieces {
                            *p = p.reversed();
                        }
                    }
                    return Ok(pieces);
                }
            };
            Ok(vec![if seg.reversed { edge.reversed() } else { edge }])
        })
        .collect::<Result<Vec<Vec<ProfileEdge>>, String>>()
        .map(|v| v.into_iter().flatten().collect())
}

/// A sketch region as an exact kernel profile (outer loop and holes).
fn region_profile(sketch: &Sketch, r: &Region) -> Result<Profile, String> {
    let outer = loop_edges(sketch, &r.outer)?;
    let holes = r
        .holes
        .iter()
        .map(|h| loop_edges(sketch, h))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Profile::new(outer, holes))
}

/// The profiles a feature uses, found from its seed points.
fn feature_profiles(
    f: &chamfera_doc::SketchFeature,
    seeds: &[Point2],
) -> Result<Vec<Profile>, String> {
    seeds
        .iter()
        .map(|seed| {
            let region = f.sketch.region_at(*seed).ok_or_else(|| {
                format!(
                    "the profile at ({:.2}, {:.2}) of {} is no longer closed",
                    seed.x, seed.y, f.name
                )
            })?;
            region_profile(&f.sketch, &region)
        })
        .collect()
}

/// Boolean operations on kernel bodies. The engine calls these only; kernel rung K7
/// provides them.
mod booleans {
    use chamfera_kernel::{Body, KernelError};

    pub fn union(a: &Body, b: &Body) -> Result<Body, String> {
        chamfera_kernel::union(a, b).map_err(|e| format!("join: {e}"))
    }

    /// `None` when nothing is left.
    pub fn subtract(a: &Body, b: &Body) -> Result<Option<Body>, String> {
        match chamfera_kernel::subtract(a, b) {
            Ok(body) => Ok(Some(body)),
            Err(KernelError::EmptyResult) => Ok(None),
            Err(e) => Err(format!("cut: {e}")),
        }
    }

    /// `None` when they don't overlap.
    pub fn intersect(a: &Body, b: &Body) -> Result<Option<Body>, String> {
        match chamfera_kernel::intersect(a, b) {
            Ok(body) => Ok(Some(body)),
            Err(KernelError::EmptyResult) => Ok(None),
            Err(e) => Err(format!("intersect: {e}")),
        }
    }
}

/// True when two bodies' bounding boxes overlap (or touch).
fn boxes_overlap(a: &Body, b: &Body) -> bool {
    match (a.bounding_box(), b.bounding_box()) {
        (Some(x), Some(y)) => {
            let t = chamfera_geom::LINEAR_TOLERANCE;
            x.min.x <= y.max.x + t
                && y.min.x <= x.max.x + t
                && x.min.y <= y.max.y + t
                && y.min.y <= x.max.y + t
                && x.min.z <= y.max.z + t
                && y.min.z <= x.max.z + t
        }
        _ => false,
    }
}

/// Applies a feature's new shapes (`tools`) to the bodies built so far: Join merges each
/// tool with every body it overlaps, Cut removes it from them, Intersect keeps only the
/// overlap. The targets are the bodies whose bounds meet the tool, like Fusion users
/// expect, so nothing has to be referenced by name.
fn apply_operation(
    made: &mut Vec<(u32, String, Body)>,
    tools: &[Body],
    operation: Operation,
) -> Result<(), String> {
    let verb = match operation {
        Operation::NewBody => return Ok(()),
        Operation::Join => "join",
        Operation::Cut => "cut",
        Operation::Intersect => "intersect",
    };
    for tool in tools {
        let targets: Vec<usize> = made
            .iter()
            .enumerate()
            .filter(|(_, m)| boxes_overlap(&m.2, tool))
            .map(|(i, _)| i)
            .collect();
        if targets.is_empty() {
            return Err(format!(
                "nothing to {verb}: the new shape does not reach any body"
            ));
        }
        match operation {
            Operation::Join => {
                // Everything the tool touches becomes one body, named after the first.
                let mut acc = tool.clone();
                for &i in targets.iter().rev() {
                    let Some(target) = made.get(i) else { continue };
                    acc = booleans::union(&target.2, &acc)?;
                }
                let first = targets[0];
                for &i in targets.iter().skip(1).rev() {
                    made.remove(i);
                }
                if let Some(m) = made.get_mut(first) {
                    m.2 = acc;
                }
            }
            Operation::Cut | Operation::Intersect => {
                for &i in targets.iter().rev() {
                    let Some(target) = made.get(i) else { continue };
                    let result = if operation == Operation::Cut {
                        booleans::subtract(&target.2, tool)?
                    } else {
                        booleans::intersect(&target.2, tool)?
                    };
                    match result {
                        Some(b) => {
                            if let Some(m) = made.get_mut(i) {
                                m.2 = b;
                            }
                        }
                        None => {
                            made.remove(i);
                        }
                    }
                }
            }
            Operation::NewBody => {}
        }
    }
    Ok(())
}

/// The moved copies a pattern makes of its source's shapes (copy 1 is the source itself,
/// so copies 2..=count are returned), and the operation they do.
fn pattern_copies(
    doc: &chamfera_doc::Document,
    p: &chamfera_doc::Pattern,
    pattern_id: u32,
    tools: &BTreeMap<u32, (Vec<Body>, Operation)>,
) -> Result<(Vec<Body>, Operation), String> {
    use chamfera_doc::PatternLayout;
    use chamfera_geom::Transform;
    let (source, op) = tools.get(&p.source).ok_or_else(|| {
        format!(
            "feature {} it repeats is suppressed or could not be built",
            p.source
        )
    })?;
    let count = doc.pattern_count(p).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for k in 1..count {
        let t = match &p.layout {
            PatternLayout::Linear { direction, spacing } => {
                let s = doc
                    .evaluate_feature_value(spacing, chamfera_doc::expr::Dim::LENGTH)
                    .map_err(|e| e.to_string())?;
                let d = direction
                    .normalized()
                    .ok_or("the direction has no length")?;
                Transform::translation(d * (s * k as f64))
            }
            PatternLayout::Circular {
                origin,
                axis,
                angle,
            } => {
                let a = doc
                    .evaluate_feature_value(angle, chamfera_doc::expr::Dim::ANGLE)
                    .map_err(|e| e.to_string())?;
                // A full circle spreads `count` copies with none on top of the first;
                // a partial one puts the last copy at the end of the angle.
                let full =
                    (a.abs() - std::f64::consts::TAU).abs() < chamfera_geom::ANGULAR_TOLERANCE;
                let step = if full {
                    a / count as f64
                } else {
                    a / (count - 1) as f64
                };
                Transform::rotation_about(*origin, *axis, step * k as f64)
            }
        }
        .ok_or("the pattern's move leaves the working range")?;
        for body in source {
            let mut copy = body.transformed(&t).map_err(|e| format!("pattern: {e}"))?;
            // Copy k's faces are named `<pattern>:<k> <role>` so each copy is its own.
            for face in copy.faces() {
                let role = copy
                    .face_name(face)
                    .ok()
                    .flatten()
                    .and_then(split_face_key)
                    .map(|(_, r)| r.to_owned());
                if let Some(role) = role {
                    let _ = copy
                        .set_face_name(face, &face_key(pattern_id, &format!("{} {role}", k + 1)));
                }
            }
            out.push(copy);
        }
    }
    Ok((out, *op))
}

/// The bodies one extrude makes: one per profile.
fn build_extrude(doc: &chamfera_doc::Document, e: &Extrude) -> Result<Vec<Body>, String> {
    let f = doc.sketch(e.sketch).map_err(|x| x.to_string())?;
    let distance = doc
        .evaluate_feature_value(&e.distance, chamfera_doc::expr::Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    feature_profiles(f, &e.profiles)?
        .iter()
        .map(|profile| {
            match e.direction {
                ExtrudeDirection::Normal => chamfera_kernel::extrude(&f.plane, profile, distance),
                ExtrudeDirection::Reversed => {
                    chamfera_kernel::extrude(&f.plane, profile, -distance)
                }
                ExtrudeDirection::Symmetric => {
                    chamfera_kernel::extrude_symmetric(&f.plane, profile, distance)
                }
            }
            .map_err(|x| format!("extrude: {x}"))
        })
        .collect()
}

/// The bodies one revolve makes: one per profile, about a line of the same sketch.
fn build_revolve(doc: &chamfera_doc::Document, r: &Revolve) -> Result<Vec<Body>, String> {
    let f = doc.sketch(r.sketch).map_err(|x| x.to_string())?;
    let angle = doc
        .evaluate_feature_value(&r.angle, chamfera_doc::expr::Dim::ANGLE)
        .map_err(|x| x.to_string())?;
    let axis = match f.sketch.curve(chamfera_sketch::CurveId(r.axis)) {
        Some(Curve::Line { start, end, .. }) => {
            let (Some(a), Some(b)) = (f.sketch.point(*start), f.sketch.point(*end)) else {
                return Err("the axis line has lost a point".into());
            };
            Line2::new(a, b - a).map_err(|e| format!("the axis: {e}"))?
        }
        _ => return Err(format!("the axis {} is not a line of {}", r.axis, f.name)),
    };
    feature_profiles(f, &r.profiles)?
        .iter()
        .map(|profile| {
            chamfera_kernel::revolve(&f.plane, profile, &axis, angle)
                .map_err(|x| format!("revolve: {x}"))
        })
        .collect()
}

impl Session {
    /// Rebuilds every feature, in timeline order, and replaces the solid bodies in the
    /// scene. Meshes and other bodies (imported STL) are left alone.
    pub fn rebuild(&mut self) {
        let mut built = Built::default();
        let mut made: Vec<(u32, String, Body)> = Vec::new();
        // Each built feature's own shapes (before they were combined), for patterns.
        let mut tools: BTreeMap<u32, (Vec<Body>, Operation)> = BTreeMap::new();
        let features = self.doc.features.clone();
        for feature in &features {
            if feature.suppressed {
                continue;
            }
            // A sketch drawn on a face follows that face (built earlier in the timeline).
            if let Some(sketch) = feature.kind.sketch()
                && let Err(e) = self.follow_attached_sketch(sketch, &made)
            {
                built.errors.insert(feature.id, e);
                continue;
            }
            if let FeatureKind::Pattern(p) = &feature.kind {
                let copies = pattern_copies(&self.doc, p, feature.id, &tools);
                match copies {
                    Ok((bodies, Operation::NewBody)) => {
                        for (k, b) in bodies.into_iter().enumerate() {
                            made.push((feature.id, format!("{} ({})", feature.name, k + 2), b));
                        }
                    }
                    Ok((bodies, op)) => {
                        let mut next = made.clone();
                        match apply_operation(&mut next, &bodies, op) {
                            Ok(()) => made = next,
                            Err(e) => {
                                built.errors.insert(feature.id, e);
                            }
                        }
                    }
                    Err(e) => {
                        built.errors.insert(feature.id, e);
                    }
                }
                continue;
            }
            if let FeatureKind::Blend(b) = &feature.kind {
                match build_blend(&self.doc, b, feature.id, &made) {
                    Ok(next) => made = next,
                    Err(e) => {
                        built.errors.insert(feature.id, e);
                    }
                }
                continue;
            }
            let result = match &feature.kind {
                FeatureKind::Extrude(e) => build_extrude(&self.doc, e),
                FeatureKind::Revolve(r) => build_revolve(&self.doc, r),
                FeatureKind::Pattern(_) | FeatureKind::Blend(_) => continue,
            };
            let operation = feature.kind.operation();
            let result = result.map(|bodies| {
                bodies
                    .into_iter()
                    .map(|mut b| {
                        name_faces(&mut b, feature.id);
                        name_edges(&mut b, feature.id);
                        b
                    })
                    .collect::<Vec<_>>()
            });
            if let Ok(bodies) = &result {
                tools.insert(feature.id, (bodies.clone(), operation));
            }
            match result {
                Ok(bodies) if operation == Operation::NewBody => {
                    let n = bodies.len();
                    for (k, body) in bodies.into_iter().enumerate() {
                        let name = if n == 1 {
                            feature.name.clone()
                        } else {
                            format!("{} ({})", feature.name, k + 1)
                        };
                        made.push((feature.id, name, body));
                    }
                }
                Ok(tools) => {
                    // Work on a copy so a failure leaves the bodies built so far intact.
                    let mut next = made.clone();
                    match apply_operation(&mut next, &tools, operation) {
                        Ok(()) => made = next,
                        Err(e) => {
                            built.errors.insert(feature.id, e);
                        }
                    }
                }
                Err(e) => {
                    built.errors.insert(feature.id, e);
                }
            }
        }
        // Sketches on faces that no feature uses yet follow their face too.
        let attached: Vec<u32> = self
            .doc
            .sketches
            .iter()
            .filter(|s| s.attached.is_some())
            .map(|s| s.id)
            .collect();
        for id in attached {
            let _ = self.follow_attached_sketch(id, &made);
        }
        // Replace the scene's solid bodies.
        let old: Vec<u32> = self
            .built
            .solids
            .iter()
            .filter_map(|s| s.scene_id)
            .collect();
        self.scene.bodies.retain(|b| !old.contains(&b.id));
        if self.scene.selection.is_some_and(|h| old.contains(&h.body)) {
            self.scene.selection = None;
        }
        let options = TessOptions::default();
        for (feature, name, body) in made {
            // A solid that cannot be shown is still built: its measures and export work.
            let mut faces = Vec::new();
            let mut edges = Vec::new();
            let scene_id = match tessellate(&body, &options) {
                Ok(m) => {
                    faces = m.face_of_triangle;
                    edges = m.edge_polylines;
                    let id = self.scene.add(&name, m.mesh);
                    if id.is_none() {
                        built
                            .display_errors
                            .insert(feature, "the scene holds no more bodies".into());
                    }
                    id
                }
                Err(e) => {
                    built
                        .display_errors
                        .insert(feature, format!("the solid cannot be shown yet: {e}"));
                    None
                }
            };
            built.solids.push(Solid {
                faces,
                edges,
                feature,
                name,
                body,
                scene_id,
            });
        }
        self.show_imported(&mut built);
        self.built = built;
    }

    /// Moves an attached sketch's plane onto its face, as built so far.
    fn follow_attached_sketch(
        &mut self,
        sketch: u32,
        made: &[(u32, String, Body)],
    ) -> Result<(), String> {
        let Ok(f) = self.doc.sketch(sketch) else {
            return Ok(());
        };
        let Some(r) = f.attached.clone() else {
            return Ok(());
        };
        let plane = attached_plane(made, &r)?;
        if let Ok(f) = self.doc.sketch_mut(sketch) {
            f.plane = plane;
        }
        Ok(())
    }

    /// The solid face under pixel `(x, y)`: (feature, face name, plane), for flat faces
    /// with a name.
    pub fn face_at(
        &self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> Option<(u32, String, chamfera_geom::Frame)> {
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.scene.pick(&ray)?;
        let solid = self
            .built
            .solids
            .iter()
            .find(|s| s.scene_id == Some(hit.body))?;
        let face = *solid.faces.get(hit.triangle)?;
        let name = solid.body.face_name(face).ok().flatten()?;
        let (feature, role) = split_face_key(name)?;
        let plane = face_plane(&solid.body, face)?;
        Some((feature, role.to_owned(), plane))
    }

    /// The true edge nearest pixel `(x, y)` on screen, within [`EDGE_PICK_PIXELS`], that is
    /// not hidden behind a face: (solid index, edge, its reference name).
    pub fn edge_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<(usize, EdgeId, String)> {
        use chamfera_render::{mat_mul, transform_point};
        if !(w >= 1.0 && h >= 1.0 && x.is_finite() && y.is_finite()) {
            return None;
        }
        let bounds = self.scene.bounds();
        let vp = mat_mul(
            &self.camera.projection_matrix(w / h, bounds.as_ref()),
            &self.camera.view_matrix(),
        );
        let screen = |p: Point3| -> Option<(f64, f64)> {
            let n = transform_point(&vp, p)?;
            if !(0.0..=1.0).contains(&n[2]) {
                return None;
            }
            Some(((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h))
        };
        // (pixel distance, solid, edge, the nearest segment of the edge, the pixel on it)
        type Segment = (Point3, Point3);
        type Near = (f64, usize, EdgeId, Segment, (f64, f64));
        let mut near: Vec<Near> = Vec::new();
        for (si, s) in self.built.solids.iter().enumerate() {
            let shown = s
                .scene_id
                .and_then(|id| self.scene.bodies.iter().find(|b| b.id == id))
                .is_some_and(|b| b.visible);
            if !shown {
                continue;
            }
            for (e, line) in &s.edges {
                let mut best: Option<(f64, Segment, (f64, f64))> = None;
                for pair in line.windows(2) {
                    let [a, b] = pair else { continue };
                    let (Some(sa), Some(sb)) = (screen(*a), screen(*b)) else {
                        continue;
                    };
                    let (dx, dy) = (sb.0 - sa.0, sb.1 - sa.1);
                    let len2 = dx * dx + dy * dy;
                    let t = if len2 > 1e-12 {
                        (((x - sa.0) * dx + (y - sa.1) * dy) / len2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let px = (sa.0 + t * dx, sa.1 + t * dy);
                    let d = ((px.0 - x).powi(2) + (px.1 - y).powi(2)).sqrt();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, (*a, *b), px));
                    }
                }
                if let Some((d, p, px)) = best
                    && d <= EDGE_PICK_PIXELS
                {
                    near.push((d, si, *e, p, px));
                }
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, si, e, (a, b), px) in near {
            // Hidden when a face is clearly nearer the eye along the ray through it. The
            // edge's depth is where the ray passes closest to the segment (a point taken
            // by its place on screen would be off under perspective).
            let ray = self.camera.ray_through(px.0, px.1, w, h, bounds.as_ref())?;
            let u = b - a;
            let w0 = a - ray.origin;
            let (uu, ud) = (u.dot(u), u.dot(ray.direction));
            let denom = uu - ud * ud;
            let s = if denom > 1e-12 {
                ((ud * ray.direction.dot(w0) - u.dot(w0)) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let along = (a + u * s - ray.origin).dot(ray.direction);
            let hidden = self
                .scene
                .pick(&ray)
                .is_some_and(|hit| hit.distance < along - (1e-3 * along.abs()).max(0.05));
            if hidden {
                continue;
            }
            let solid = self.built.solids.get(si)?;
            if let Some(r) = edge_ref(&solid.body, e) {
                return Some((si, e, r));
            }
        }
        None
    }

    /// The solids of the last rebuild.
    pub fn solids(&self) -> &[Solid] {
        &self.built.solids
    }

    /// Features that failed in the last rebuild.
    pub fn feature_errors(&self) -> &BTreeMap<u32, String> {
        &self.built.errors
    }

    /// Features built but not displayable (for example curved faces before their
    /// tessellation exists).
    pub fn display_errors(&self) -> &BTreeMap<u32, String> {
        &self.built.display_errors
    }

    /// The profile under pixel `(x, y)`: the nearest sketch plane the ray meets where a
    /// closed region lies. Returns (sketch id, seed point inside the region, its area).
    pub fn profile_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<(u32, Point2, f64)> {
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let mut best: Option<(f64, u32, Point2, f64)> = None;
        for f in &self.doc.sketches {
            let n = f.plane.z();
            let denom = n.dot(ray.direction);
            if denom.abs() < 1e-6 {
                continue;
            }
            let t = n.dot(f.plane.origin() - ray.origin) / denom;
            if t < 0.0 {
                continue;
            }
            let p = f.plane.to_plane(ray.at(t));
            if let Some(region) = f.sketch.region_at(p)
                && best.is_none_or(|b| t < b.0)
            {
                // Remember the click point itself: it is inside this region and is what
                // the user meant.
                best = Some((t, f.id, p, region.area));
            }
        }
        best.map(|(_, s, p, a)| (s, p, a))
    }

    /// The outline (outer loop and holes) of the region at `seed`, as world line segments.
    pub fn profile_outline(&self, sketch: u32, seed: Point2) -> Vec<[Point3; 2]> {
        let Ok(f) = self.doc.sketch(sketch) else {
            return Vec::new();
        };
        let Some(region) = f.sketch.region_at(seed) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for l in std::iter::once(&region.outer).chain(region.holes.iter()) {
            let poly = f.sketch.loop_polyline(l);
            for i in 0..poly.len() {
                if let (Some(a), Some(b)) = (poly.get(i), poly.get((i + 1) % poly.len())) {
                    out.push([f.plane.from_plane(*a), f.plane.from_plane(*b)]);
                }
            }
        }
        out
    }
}

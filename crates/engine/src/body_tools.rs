//! Building the body tools on the timeline: combine, split body, split face, thicken and
//! remove. Each works on the bodies built so far (found by name) and returns the bodies
//! after it, or an error and leaves them as they were.

use crate::Session;
use crate::construct::Constructed;
use crate::solids::{face_key, name_edges, name_faces};
use chamfera_doc::{
    Combine, Document, Feature, FeatureKind, Operation, PlaneRef, RemoveBodies, SplitBody,
    SplitFace, SplitFaceTool, Thicken,
};
use chamfera_geom::Frame;
use chamfera_kernel::{Body, FaceId, KernelError};
use std::collections::BTreeMap;

type Made = Vec<(u32, String, Body)>;

/// The bodies after body tool `feature`, from the bodies built so far.
pub(crate) fn build(
    doc: &Document,
    feature: &Feature,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<Made, String> {
    match &feature.kind {
        FeatureKind::Combine(c) => combine(c, made),
        FeatureKind::SplitBody(s) => split_body(doc, s, feature, made, constructs),
        FeatureKind::SplitFace(s) => split_face(doc, s, feature.id, made, constructs),
        FeatureKind::Thicken(t) => thicken(doc, t, feature, made),
        FeatureKind::Remove(r) => remove(r, made),
        _ => Err("not a body tool".into()),
    }
}

fn index_of(made: &[(u32, String, Body)], name: &str) -> Result<usize, String> {
    made.iter()
        .position(|m| m.1 == name)
        .ok_or_else(|| format!("there is no body named {name:?} at this point"))
}

fn combine(c: &Combine, made: &[(u32, String, Body)]) -> Result<Made, String> {
    let target = index_of(made, &c.target)?;
    let mut tools = Vec::new();
    for t in &c.tools {
        let i = index_of(made, t)?;
        if i == target {
            return Err("the target cannot also be a tool".into());
        }
        if !tools.contains(&i) {
            tools.push(i);
        }
    }
    let mut acc = made
        .get(target)
        .map(|m| m.2.clone())
        .ok_or("the target is missing")?;
    for &i in &tools {
        let (_, name, tool) = made.get(i).ok_or("a tool is missing")?;
        acc = match c.operation {
            Operation::Join => crate::perf::boolean(|| chamfera_kernel::union(&acc, tool)),
            Operation::Cut => crate::perf::boolean(|| chamfera_kernel::subtract(&acc, tool)),
            Operation::Intersect => crate::perf::boolean(|| chamfera_kernel::intersect(&acc, tool)),
            Operation::NewBody => {
                return Err("combine joins, cuts or intersects".into());
            }
        }
        .map_err(|e| match e {
            KernelError::EmptyResult if c.operation == Operation::Cut => {
                format!("combine: cutting {name} leaves nothing of {}", c.target)
            }
            KernelError::EmptyResult => {
                format!("combine: {name} and {} share no volume", c.target)
            }
            e => format!("combine: {e}"),
        })?;
    }
    let mut next = made.to_vec();
    if let Some(slot) = next.get_mut(target) {
        slot.2 = acc;
    }
    if !c.keep_tools {
        let mut k = 0;
        next.retain(|_| {
            let keep = !tools.contains(&k);
            k += 1;
            keep
        });
    }
    Ok(next)
}

/// How a plane is named in the faces and edges a split makes: `xy`, `plane 5` (a
/// construction plane) or `2:top` (a face).
fn plane_label(p: &PlaneRef) -> String {
    match p {
        PlaneRef::Origin { plane } => match plane {
            chamfera_doc::OriginPlane::Xy => "xy".into(),
            chamfera_doc::OriginPlane::Xz => "xz".into(),
            chamfera_doc::OriginPlane::Yz => "yz".into(),
        },
        PlaneRef::Construct { feature } => format!("plane {feature}"),
        PlaneRef::Face(f) => face_key(f.feature, &f.face),
    }
}

/// A face plane goes through the resolver (rung K8) with the signature item `owner`
/// keeps for it.
fn plane_frame(
    doc: &Document,
    owner: u32,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
    p: &PlaneRef,
) -> Result<Frame, String> {
    crate::construct::plane_of_made(doc, owner, made, constructs, p).map(|(f, _)| f)
}

fn split_body(
    doc: &Document,
    s: &SplitBody,
    feature: &Feature,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<Made, String> {
    let frame = plane_frame(doc, feature.id, made, constructs, &s.plane)?;
    let face = format!("split {}", plane_label(&s.plane));
    let mut next = made.to_vec();
    let n = s.bodies.len();
    for (k, name) in s.bodies.iter().enumerate() {
        let i = index_of(&next, name)?;
        let body = next
            .get(i)
            .map(|m| m.2.clone())
            .ok_or("a body is missing")?;
        let (mut back, mut front) = chamfera_kernel::split_body(&body, &frame, &face)
            .map_err(|e| format!("split body {name}: {e}"))?;
        for b in [&mut back, &mut front] {
            name_faces(b, feature.id);
            name_edges(b, feature.id);
        }
        if let Some(slot) = next.get_mut(i) {
            slot.2 = back;
        }
        let label = if n == 1 {
            feature.name.clone()
        } else {
            format!("{} ({})", feature.name, k + 1)
        };
        next.push((feature.id, label, front));
    }
    Ok(next)
}

/// The plane a split face cuts with: a plane, or the plane through a sketch line square
/// to its sketch.
fn split_face_plane(
    doc: &Document,
    owner: u32,
    tool: &SplitFaceTool,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<(Frame, String), String> {
    match tool {
        SplitFaceTool::Plane { plane } => Ok((
            plane_frame(doc, owner, made, constructs, plane)?,
            plane_label(plane),
        )),
        SplitFaceTool::SketchLine { sketch, curve } => {
            let f = doc.sketch(*sketch).map_err(|e| e.to_string())?;
            let c = f
                .sketch
                .curve(chamfera_sketch::CurveId(*curve))
                .filter(|c| c.is_line())
                .ok_or_else(|| format!("curve {curve} is not a line of {}", f.name))?;
            let (a, b) = c
                .ends()
                .and_then(|(a, b)| Some((f.sketch.point(a)?, f.sketch.point(b)?)))
                .ok_or("the line has no ends")?;
            let (pa, pb) = (f.plane.from_plane(a), f.plane.from_plane(b));
            let normal = f
                .plane
                .z()
                .cross(pb - pa)
                .normalized()
                .ok_or("the line has no length")?;
            let frame =
                Frame::from_normal(pa, normal).ok_or("the line leaves the working range")?;
            Ok((frame, format!("line {curve} of {}", f.name)))
        }
    }
}

fn split_face(
    doc: &Document,
    s: &SplitFace,
    id: u32,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<Made, String> {
    let (frame, label) = split_face_plane(doc, id, &s.tool, made, constructs)?;
    // The faces through the resolver (rung K8): every piece of an already split face, or
    // the piece a `#k` name or the signature picks.
    let mut per_body: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    {
        let bodies = crate::naming::bodies(made);
        for n in &s.faces {
            let found = crate::naming::resolve(
                doc,
                id,
                &bodies,
                crate::naming::Kind::Face,
                n,
                crate::naming::Want::All,
            )
            .map_err(|e| format!("split face: {e}"))?;
            for f in found {
                if let Some(face) = f.face() {
                    let list = per_body.entry(f.body).or_default();
                    if !list.contains(&face) {
                        list.push(face);
                    }
                }
            }
        }
    }
    let mut next = made.to_vec();
    for (i, (_, _, body)) in next.iter_mut().enumerate() {
        let Some(ids) = per_body.remove(&i) else {
            continue;
        };
        let edge = face_key(id, &format!("split {label}"));
        let (mut out, front) = chamfera_kernel::split_faces(body, &ids, &frame, &edge)
            .map_err(|e| format!("split face: {e}"))?;
        for f in front {
            let old = out.face_name(f).ok().flatten().unwrap_or("").to_owned();
            let _ = out.set_face_name(f, &face_key(id, &format!("split {label} of {old}")));
        }
        *body = out;
    }
    Ok(next)
}

fn thicken(
    doc: &Document,
    t: &Thicken,
    feature: &Feature,
    made: &[(u32, String, Body)],
) -> Result<Made, String> {
    let thickness = doc
        .evaluate_feature_value(&t.thickness, chamfera_doc::expr::Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    let (lo, hi) = t.direction.span(thickness);
    let mut shapes = Vec::new();
    // Each face through the resolver (rung K8): every piece of a split face is thickened.
    // Surface bodies whose faces are thickened become the solids (they are used up).
    let bodies = crate::naming::bodies(made);
    let mut faces = Vec::new();
    let mut used_surfaces: Vec<usize> = Vec::new();
    for name in &t.faces {
        let found = crate::naming::resolve(
            doc,
            feature.id,
            &bodies,
            crate::naming::Kind::Face,
            name,
            crate::naming::Want::All,
        )
        .map_err(|e| format!("thicken: {e}"))?;
        for f in found {
            if let (Some(b), Some(face)) = (bodies.get(f.body), f.face()) {
                if b.is_open() && !used_surfaces.contains(&f.body) {
                    used_surfaces.push(f.body);
                }
                faces.push((name, *b, face));
            }
        }
    }
    for (name, body, face) in faces {
        let mut out = chamfera_kernel::thicken(body, face, lo, hi)
            .map_err(|e| format!("thicken {name}: {e}"))?;
        name_faces(&mut out, feature.id);
        name_edges(&mut out, feature.id);
        if shapes.is_empty() {
            shapes.push(out);
        } else {
            // Faces after the first are told apart by their index.
            let k = shapes.len() + 1;
            rename_with_index(&mut out, feature.id, k);
            shapes.push(out);
        }
    }
    let mut next: Made = made
        .iter()
        .enumerate()
        .filter(|(k, _)| !used_surfaces.contains(k))
        .map(|(_, m)| m.clone())
        .collect();
    if t.operation == Operation::NewBody {
        let n = shapes.len();
        for (k, b) in shapes.into_iter().enumerate() {
            let label = if n == 1 {
                feature.name.clone()
            } else {
                format!("{} ({})", feature.name, k + 1)
            };
            next.push((feature.id, label, b));
        }
    } else {
        crate::solids::apply_operation(&mut next, &shapes, t.operation)?;
    }
    Ok(next)
}

/// Renames `7:top` to `7:2 top`, so each face's thickened solid has its own names.
fn rename_with_index(body: &mut Body, feature: u32, k: usize) {
    let prefix = format!("{feature}:");
    for f in body.faces() {
        if let Some(n) = body.face_name(f).ok().flatten().map(str::to_owned)
            && let Some(role) = n.strip_prefix(&prefix)
        {
            let _ = body.set_face_name(f, &face_key(feature, &format!("{k} {role}")));
        }
    }
    for e in body.edges() {
        if let Some(n) = body.edge_name(e).ok().flatten().map(str::to_owned)
            && let Some(role) = n.strip_prefix(&prefix)
        {
            let _ = body.set_edge_name(e, &face_key(feature, &format!("{k} {role}")));
        }
    }
}

fn remove(r: &RemoveBodies, made: &[(u32, String, Body)]) -> Result<Made, String> {
    for name in &r.bodies {
        index_of(made, name)?;
    }
    Ok(made
        .iter()
        .filter(|m| !r.bodies.contains(&m.1))
        .cloned()
        .collect())
}

impl Session {
    /// The names of every selected solid body, in the order they were selected (what the
    /// body tools act on when no names are given).
    pub fn selected_solid_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in &self.scene.selected {
            for x in self.solids() {
                if x.scene_id == Some(*id) && !out.contains(&x.name) {
                    out.push(x.name.clone());
                }
            }
        }
        if out.is_empty() {
            out = self.selected_body_names();
        }
        out
    }
}

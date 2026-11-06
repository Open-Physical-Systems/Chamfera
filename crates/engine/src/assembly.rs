//! Solving assemblies: where each component is, from its placement and its joints.
//!
//! Positions are worked out by walking from the grounded components along the joint tree
//! (`chamfera_doc::joint::drivers`): each joint places its far component exactly from the
//! near one, so a chain composes transforms and needs no iterative solver. A component no
//! joint reaches keeps its placement inside its parent.
//!
//! For joint `j` with origin frames `Fa` (on A, in A's coordinates) and `Fb` (on B), and
//! the joint's motion `M` (a turn and/or slide in the origin frame), B's coordinates map
//! into A's by `Fb⁻¹`, then `M`, then `Fa`; so `world(B) = world(A) ∘ Fa ∘ M ∘ Fb⁻¹`.

use crate::Session;
use crate::naming::{Kind, Want};
use crate::solids::{Built, face_plane};
use chamfera_doc::component::ROOT;
use chamfera_doc::{Assembly, EdgeEnd, Joint, JointKind, JointOrigin, OriginAt};
use chamfera_geom::{Frame, Point3, Transform, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId, FaceId};
use std::collections::BTreeMap;

/// Where every component is, and the joints that could not be solved.
#[derive(Debug, Default, Clone)]
pub struct Placements {
    /// Component id → its world transform (from its own coordinates to the world).
    pub world: BTreeMap<u32, Transform>,
    /// Joint id → why it could not place its component (that component keeps its own
    /// placement).
    pub errors: BTreeMap<u32, String>,
}

impl Placements {
    /// A component's world transform (identity for the root or an unknown id).
    pub fn of(&self, component: u32) -> Transform {
        self.world
            .get(&component)
            .copied()
            .unwrap_or(Transform::IDENTITY)
    }
}

/// The joint's motion in its origin frame: a turn about z, a slide, or both.
pub fn joint_motion(j: &Joint) -> Option<Transform> {
    let turn = |a: f64| Transform::rotation(Vec3::Z, a);
    match j.kind {
        JointKind::Rigid => Some(Transform::IDENTITY),
        JointKind::Revolute => turn(j.value),
        JointKind::Slider => Transform::translation(Vec3::Z * j.value),
        JointKind::Cylindrical => {
            Some(turn(j.value)?.then(&Transform::translation(Vec3::Z * j.distance)?))
        }
        JointKind::Planar => Some(turn(j.value)?.then(&Transform::translation(Vec3::new(
            j.offset[0],
            j.offset[1],
            0.0,
        ))?)),
    }
}

/// The transform from B's coordinates into A's that joint `j` makes, given its origin
/// frames on A and on B (each in its own component's coordinates).
pub fn joint_relative(j: &Joint, fa: &Frame, fb: &Frame) -> Option<Transform> {
    let fb_inverse = fb.local_to_world().inverse_rigid()?;
    let flip = if j.flip {
        Transform::rotation(Vec3::X, std::f64::consts::PI)?
    } else {
        Transform::IDENTITY
    };
    Some(
        fb_inverse
            .then(&flip)
            .then(&joint_motion(j)?)
            .then(&fa.local_to_world()),
    )
}

/// Solves every component's world transform. `frame` finds a joint origin's frame in its
/// component's own coordinates.
pub fn solve(asm: &Assembly, frame: &dyn Fn(&JointOrigin) -> Result<Frame, String>) -> Placements {
    let mut out = Placements::default();
    out.world.insert(ROOT, Transform::IDENTITY);
    let drivers = chamfera_doc::joint::drivers(asm);
    // What each component's position depends on: a joint and the component it is reached
    // from, or its own placement inside its parent.
    enum Rule {
        Joint(Transform, u32),
        Parent(Transform, u32),
    }
    let mut rules: Vec<(u32, Rule)> = Vec::new();
    for c in &asm.components {
        let own = Rule::Parent(c.placement, c.parent);
        let rule = match drivers.get(&c.id) {
            None => own,
            Some(d) => {
                let made = asm.joint(d.joint).map_err(|e| e.to_string()).and_then(|j| {
                    let fa = frame(&j.a).map_err(|e| format!("its origin on A: {e}"))?;
                    let fb = frame(&j.b).map_err(|e| format!("its origin on B: {e}"))?;
                    let rel = joint_relative(j, &fa, &fb)
                        .ok_or("its origins do not make a proper move")?;
                    if d.forward {
                        Ok(rel)
                    } else {
                        rel.inverse_rigid()
                            .ok_or_else(|| "its origins do not make a proper move".to_owned())
                    }
                });
                match made {
                    Ok(t) => Rule::Joint(t, d.from),
                    Err(e) => {
                        out.errors.insert(d.joint, e);
                        own
                    }
                }
            }
        };
        rules.push((c.id, rule));
    }
    // Resolve in passes: each pass places every component whose reference is placed. A
    // tree needs at most one pass per level; anything left after that depends on itself
    // (a joint against the component's own parent chain) and keeps its plain placement.
    for _ in 0..=rules.len() {
        let mut progress = false;
        for (id, rule) in &rules {
            if out.world.contains_key(id) {
                continue;
            }
            let (t, from) = match rule {
                Rule::Joint(t, from) | Rule::Parent(t, from) => (t, from),
            };
            if let Some(base) = out.world.get(from).copied() {
                out.world.insert(*id, t.then(&base));
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    for (id, rule) in &rules {
        if !out.world.contains_key(id) {
            if let (Rule::Joint(..), Some(d)) = (rule, drivers.get(id)) {
                out.errors.insert(
                    d.joint,
                    "it positions a component against that component's own parent; \
                     joint it to another component"
                        .into(),
                );
            }
            let own = asm
                .component(*id)
                .map_or(Transform::IDENTITY, |c| c.placement);
            out.world.insert(*id, own);
        }
    }
    out
}

/// The centre of a flat face: a circle's centre if its outer boundary is one, else the
/// mean of the outer boundary's corners.
fn face_centre(body: &Body, face: FaceId) -> Option<Point3> {
    let outer = body.face_outer_loop(face).ok()?;
    // A boundary made only of circles and arcs about one centre (a disc's rim, split
    // at a seam or not) is centred there: its one or two seam vertices are not.
    let mut centre: Option<Point3> = None;
    let mut round = true;
    for h in body.loop_half_edges(outer).ok()? {
        let c = body
            .half_edge_edge(h)
            .ok()
            .flatten()
            .and_then(|e| body.edge_curve(e).ok().flatten())
            .and_then(|(curve, _, _)| match curve {
                Curve::Circle(c) => Some(c.center()),
                Curve::Arc(a) => Some(a.circle().center()),
                _ => None,
            });
        match (c, centre) {
            (Some(c), None) => centre = Some(c),
            (Some(c), Some(k)) if c.distance(k) <= chamfera_geom::LINEAR_TOLERANCE => {}
            _ => round = false,
        }
    }
    if round && let Some(c) = centre {
        return Some(c);
    }
    let points: Vec<Point3> = body
        .loop_vertices(outer)
        .ok()?
        .into_iter()
        .filter_map(|v| body.vertex_point(v).ok())
        .collect();
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f64;
    let sum = points
        .iter()
        .fold(Vec3::ZERO, |acc, p| acc + (*p - Point3::ORIGIN));
    Some(Point3::ORIGIN + sum * (1.0 / n))
}

/// A face's joint frame: its centre and outward normal.
pub fn face_frame(body: &Body, face: FaceId) -> Result<Frame, String> {
    let plane = face_plane(body, face).ok_or("pick a flat face or a circular edge")?;
    let centre = face_centre(body, face).ok_or("the face has no boundary")?;
    Frame::from_normal(centre, plane.z()).ok_or_else(|| "the face has no normal".to_owned())
}

/// An edge's joint frame: a circle's centre and axis, or a line's midpoint and direction.
pub fn edge_frame(body: &Body, e: EdgeId) -> Result<Frame, String> {
    let (curve, _, _) = body
        .edge_curve(e)
        .map_err(|x| x.to_string())?
        .ok_or("the edge has no curve")?;
    let (origin, axis) = match curve {
        Curve::Circle(c) => (c.center(), c.normal()),
        Curve::Arc(a) => (a.circle().center(), a.circle().normal()),
        _ => {
            let [v0, v1] = body.edge_vertices(e).map_err(|x| x.to_string())?;
            let (p0, p1) = (
                body.vertex_point(v0).map_err(|x| x.to_string())?,
                body.vertex_point(v1).map_err(|x| x.to_string())?,
            );
            (p0 + (p1 - p0) * 0.5, p1 - p0)
        }
    };
    Frame::from_normal(origin, axis).ok_or_else(|| "the edge has no direction".to_owned())
}

/// One end of an edge, with the z axis.
fn vertex_frame(body: &Body, e: EdgeId, end: EdgeEnd) -> Result<Frame, String> {
    let [v0, v1] = body.edge_vertices(e).map_err(|x| x.to_string())?;
    let v = if end == EdgeEnd::Start { v0 } else { v1 };
    let p = body.vertex_point(v).map_err(|x| x.to_string())?;
    Frame::from_normal(p, Vec3::Z).ok_or_else(|| "no frame at the vertex".to_owned())
}

/// A joint origin's frame, looked up by name among `bodies` (the component's bodies, in
/// its own coordinates).
pub fn origin_frame<'a>(
    at: &OriginAt,
    bodies: impl Iterator<Item = &'a Body> + Clone,
) -> Result<Frame, String> {
    match at {
        OriginAt::Point { point, axis } => {
            Frame::from_normal(*point, *axis).ok_or_else(|| "the axis has no length".to_owned())
        }
        // Through the resolver (rung K8): a joint needs one face or edge, so a name shared
        // by split pieces must say which (`2:top#2`).
        OriginAt::Face { face } => {
            let list: Vec<&Body> = bodies.collect();
            let f = crate::naming::resolve_one(&list, Kind::Face, face, None, Want::One)
                .map_err(|e| e.to_string())?;
            match (list.get(f.body), f.face()) {
                (Some(body), Some(id)) => face_frame(body, id),
                _ => Err(format!("the face {face:?} is no longer there")),
            }
        }
        OriginAt::Edge { edge } | OriginAt::Vertex { edge, .. } => {
            let list: Vec<&Body> = bodies.collect();
            let f = crate::naming::resolve_one(&list, Kind::Edge, edge, None, Want::One)
                .map_err(|e| e.to_string())?;
            match (list.get(f.body), f.edge()) {
                (Some(body), Some(e)) => match at {
                    OriginAt::Vertex { end, .. } => vertex_frame(body, e, *end),
                    _ => edge_frame(body, e),
                },
                _ => Err(format!("the edge {edge:?} is no longer there")),
            }
        }
    }
}

/// A body placed in the world: (feature that made it, component, name, body).
pub type Placed = (u32, u32, String, Body);

impl Session {
    /// Solves the assembly from the bodies the timeline built (in their components' own
    /// coordinates) and moves each body to its component's place. Linked copies get
    /// their own moved copies of their original's bodies. Records the placements and
    /// joint errors in `built`.
    pub(crate) fn place_components(
        &self,
        made: Vec<(u32, String, Body)>,
        built: &mut Built,
    ) -> Vec<Placed> {
        // Joints at the values a drag or the motion study shows.
        let shown = self.shown_assembly();
        let asm: &Assembly = &shown;
        let owner = |feature: u32| asm.owner_of(feature);
        let frame = |o: &JointOrigin| {
            let original = asm.original_of(o.component);
            origin_frame(
                &o.at,
                made.iter()
                    .filter(move |m| owner(m.0) == original)
                    .map(|m| &m.2),
            )
        };
        let placements = solve(asm, &frame);
        let mut out: Vec<Placed> = Vec::new();
        let mut place = |feature: u32, component: u32, name: String, body: &Body| {
            let t = placements.of(component);
            if t == Transform::IDENTITY {
                out.push((feature, component, name, body.clone()));
                return;
            }
            match body.transformed(&t) {
                Ok(b) => out.push((feature, component, name, b)),
                Err(e) => {
                    built
                        .errors
                        .insert(feature, format!("the component cannot be placed: {e}"));
                }
            }
        };
        for (feature, name, body) in &made {
            place(*feature, owner(*feature), name.clone(), body);
        }
        for c in asm.components.iter().filter(|c| c.occurrence_of.is_some()) {
            let original = asm.original_of(c.id);
            for (feature, name, body) in made.iter().filter(|m| owner(m.0) == original) {
                place(*feature, c.id, name.clone(), body);
            }
        }
        built.placements = placements;
        out
    }

    /// The joint origin under pixel `(x, y)`: an edge near the pointer, else the flat face
    /// under it. Returns the origin (on the component that shows it) and its frame in the
    /// world.
    pub fn joint_origin_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<(JointOrigin, Frame)> {
        let solids = self.solids();
        if let Some((si, e, name)) = self.edge_at(x, y, w, h) {
            let solid = solids.get(si)?;
            let frame = edge_frame(&solid.body, e).ok()?;
            return Some((
                JointOrigin {
                    component: solid.component,
                    at: OriginAt::Edge { edge: name },
                },
                frame,
            ));
        }
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.scene.pick(&ray)?;
        let (si, solid) = solids
            .iter()
            .enumerate()
            .find(|(_, s)| s.scene_id == Some(hit.body))?;
        let face = *solid.faces.get(hit.triangle)?;
        let name = self.reference_name(si, chamfera_kernel::Entity::Face(face))?;
        let frame = face_frame(&solid.body, face).ok()?;
        Some((
            JointOrigin {
                component: solid.component,
                at: OriginAt::Face { face: name },
            },
            frame,
        ))
    }

    /// Gives a new scene body its component's tint, if that component has one: the
    /// component's place in the list picks a theme colour.
    pub(crate) fn tint_body(
        asm: &Assembly,
        scene: &mut chamfera_render::Scene,
        scene_id: Option<u32>,
        component: u32,
    ) {
        let Some(index) = asm
            .components
            .iter()
            .position(|c| c.id == component && c.tint)
        else {
            return;
        };
        if let Some(b) = scene_id.and_then(|id| scene.body_mut(id)) {
            b.tint = Some((index % usize::from(u8::MAX)) as u8);
        }
    }

    /// The component a scene body belongs to, if it shows a solid.
    pub fn component_of_body(&self, scene_id: u32) -> Option<u32> {
        self.solids()
            .iter()
            .find(|s| s.scene_id == Some(scene_id))
            .map(|s| s.component)
    }

    /// The last solve's placements and joint errors.
    pub fn placements(&self) -> &Placements {
        &self.built.placements
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_doc::{Assembly, ROOT};

    fn at_origin(component: u32, axis: Vec3) -> JointOrigin {
        JointOrigin {
            component,
            at: OriginAt::Point {
                point: Point3::ORIGIN,
                axis,
            },
        }
    }

    fn joint(kind: JointKind, a: JointOrigin, b: JointOrigin, value: f64) -> Joint {
        Joint {
            id: 0,
            name: String::new(),
            kind,
            a,
            b,
            value,
            distance: 0.0,
            offset: [0.0, 0.0],
            limits: None,
            flip: false,
        }
    }

    fn frames(o: &JointOrigin) -> Result<Frame, String> {
        origin_frame(&o.at, std::iter::empty())
    }

    fn near(p: Point3, q: Point3) -> bool {
        p.distance(q) < 1e-9
    }

    #[test]
    fn a_revolute_joint_at_90_degrees_about_z_turns_x_into_y() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        asm.component_mut(a).unwrap().grounded = true;
        let j = joint(
            JointKind::Revolute,
            at_origin(a, Vec3::Z),
            at_origin(b, Vec3::Z),
            std::f64::consts::FRAC_PI_2,
        );
        asm.add_joint(j).unwrap();
        let p = solve(&asm, &frames);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        // A right-handed quarter turn about +z: (20, 0, 0) -> (0, 20, 0).
        let q = p.of(b).apply_point(Point3::new(20.0, 0.0, 0.0));
        assert!(near(q, Point3::new(0.0, 20.0, 0.0)), "{q:?}");
    }

    #[test]
    fn a_chain_composes_from_the_grounded_end() {
        // A grounded at x = 100. B: revolute 90° about z through A's (0, 0, 0). C: slider
        // 5 along B's x axis, from B's point (10, 0, 0).
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let c = asm.add_component("C", ROOT, &[]).unwrap();
        let ca = asm.component_mut(a).unwrap();
        ca.grounded = true;
        ca.placement = Transform::translation(Vec3::new(100.0, 0.0, 0.0)).unwrap();
        asm.add_joint(joint(
            JointKind::Revolute,
            at_origin(a, Vec3::Z),
            at_origin(b, Vec3::Z),
            std::f64::consts::FRAC_PI_2,
        ))
        .unwrap();
        let on_b = JointOrigin {
            component: b,
            at: OriginAt::Point {
                point: Point3::new(10.0, 0.0, 0.0),
                axis: Vec3::X,
            },
        };
        asm.add_joint(joint(JointKind::Slider, on_b, at_origin(c, Vec3::X), 5.0))
            .unwrap();
        let p = solve(&asm, &frames);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        // C's origin sits at B's (10 + 5, 0, 0) = (15, 0, 0) in B; B is turned 90° about z
        // and A's origin is at world (100, 0, 0): (15, 0, 0) -> (0, 15, 0) -> (100, 15, 0).
        let q = p.of(c).apply_point(Point3::ORIGIN);
        assert!(near(q, Point3::new(100.0, 15.0, 0.0)), "{q:?}");
        // C's own x axis lies along B's x axis, which the turn takes to world y.
        let x = p.of(c).apply_vector(Vec3::X);
        assert!((x - Vec3::Y).length() < 1e-12, "{x:?}");
    }

    #[test]
    fn a_missing_origin_is_reported_and_the_component_stays_put() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let on_face = JointOrigin {
            component: b,
            at: OriginAt::Face {
                face: "7:top".into(),
            },
        };
        let id = asm
            .add_joint(joint(JointKind::Rigid, at_origin(a, Vec3::Z), on_face, 0.0))
            .unwrap();
        let p = solve(&asm, &frames);
        assert!(p.errors.get(&id).is_some_and(|e| e.contains("7:top")));
        assert_eq!(p.of(b), Transform::IDENTITY);
    }
}

#[cfg(test)]
#[path = "assembly_tests.rs"]
mod session_tests;

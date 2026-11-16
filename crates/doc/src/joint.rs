//! Joints: how two components move relative to each other (format 15).
//!
//! A joint connects a joint origin on component A to one on component B. Each origin is a
//! frame (a point and an axis) on its component, found by name from the bodies on every
//! rebuild, so it follows edits: a flat face (its centre and outward normal), an edge (a
//! circle's centre and axis, or a line's midpoint and direction), an end of an edge (a
//! vertex), or a fixed point and axis in the component's coordinates.
//!
//! B is placed so that its origin frame matches A's after the joint's motion: a turn about
//! the axis (revolute), a slide along it (slider), both (cylindrical), a slide in the
//! plane across it plus a turn (planar), or nothing (rigid).
//!
//! The engine solves positions by walking out from the grounded components along a
//! spanning tree of the joints ([`topology`]), an exact chain with no iteration. A joint
//! the tree does not use closes a loop (a closed kinematic chain, such as a four-bar
//! linkage): the engine then solves the free values of the loop's joints so that every
//! closing joint fits (format 41). A joint marked `driven` keeps its value in a loop.
//!
//! An as-built joint (`as_built`) joins two components where they already sit: its two
//! origins are one frame in the world, stored in each component's coordinates
//! ([`OriginAt::Frame`]), so at value 0 nothing moves.

use crate::DocError;
use crate::component::{Assembly, ROOT, check_name};
use chamfera_geom::{Frame, MAX_COORDINATE, Point3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Most joints one design may hold.
pub const MAX_JOINTS: usize = 2_000;
/// Largest joint angle magnitude, in radians (a thousand turns).
pub const MAX_JOINT_ANGLE: f64 = 1e3 * std::f64::consts::TAU;

/// How a joint lets B move relative to A.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    /// No motion: B is fixed to A.
    Rigid,
    /// A turn about the axis; the value is the angle.
    Revolute,
    /// A slide along the axis; the value is the distance.
    Slider,
    /// A turn about the axis (the value) and a slide along it (`distance`).
    Cylindrical,
    /// A turn about the axis (the value) and a slide in the plane across it (`offset`).
    Planar,
}

impl JointKind {
    pub fn parse(s: &str) -> Option<JointKind> {
        match s {
            "rigid" => Some(JointKind::Rigid),
            "revolute" => Some(JointKind::Revolute),
            "slider" => Some(JointKind::Slider),
            "cylindrical" => Some(JointKind::Cylindrical),
            "planar" => Some(JointKind::Planar),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            JointKind::Rigid => "rigid",
            JointKind::Revolute => "revolute",
            JointKind::Slider => "slider",
            JointKind::Cylindrical => "cylindrical",
            JointKind::Planar => "planar",
        }
    }

    /// What the joint's value measures: an angle, a length, or nothing (rigid).
    pub fn value_unit(self) -> Option<crate::Unit> {
        match self {
            JointKind::Rigid => None,
            JointKind::Slider => Some(crate::Unit::Length),
            JointKind::Revolute | JointKind::Cylindrical | JointKind::Planar => {
                Some(crate::Unit::Angle)
            }
        }
    }

    pub const ALL: [JointKind; 5] = [
        JointKind::Rigid,
        JointKind::Revolute,
        JointKind::Slider,
        JointKind::Cylindrical,
        JointKind::Planar,
    ];
}

/// Which end of an edge a vertex origin sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeEnd {
    Start,
    End,
}

/// Where on its component a joint origin is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum OriginAt {
    /// A fixed point and axis, in the component's own coordinates.
    Point { point: Point3, axis: Vec3 },
    /// The centre and outward normal of a flat face, by its name (`2:top`).
    Face { face: String },
    /// A circular edge's centre and axis, or a straight edge's midpoint and direction, by
    /// the edge's reference (as fillets name edges).
    Edge { edge: String },
    /// One end of a named edge; the axis is the component's z axis.
    Vertex { edge: String, end: EdgeEnd },
    /// A whole frame (origin and axes) in the component's own coordinates: what an
    /// as-built joint stores, so the two sides keep their relative turn exactly.
    Frame { frame: Frame },
}

/// A joint origin: a place on a component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointOrigin {
    pub component: u32,
    #[serde(flatten)]
    pub at: OriginAt,
}

/// A joint between components `a` and `b`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    pub id: u32,
    pub name: String,
    pub kind: JointKind,
    pub a: JointOrigin,
    pub b: JointOrigin,
    /// The angle (radians) or distance (mm) the joint is driven to.
    #[serde(default)]
    pub value: f64,
    /// Cylindrical: the slide along the axis, in mm.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub distance: f64,
    /// Planar: the slide across the axis, along the origin frame's x and y, in mm.
    #[serde(default, skip_serializing_if = "is_zero_pair")]
    pub offset: [f64; 2],
    /// The range `value` must stay in, if limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<[f64; 2]>,
    /// Turn B over (half a turn about the origin frame's x axis), so faces meet face to face.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// In a loop, the value is held (the loop's input) instead of solved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub driven: bool,
    /// Made where the components already sat: at value 0 B keeps its placement exactly.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub as_built: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_zero_pair(v: &[f64; 2]) -> bool {
    v[0] == 0.0 && v[1] == 0.0
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

/// The largest magnitude a joint's value may have.
fn value_limit(kind: JointKind) -> f64 {
    match kind.value_unit() {
        Some(crate::Unit::Angle) => MAX_JOINT_ANGLE,
        _ => MAX_COORDINATE,
    }
}

/// A value shown with its unit, for messages.
pub fn show_value(kind: JointKind, v: f64) -> String {
    let short = |x: f64| {
        let t = format!("{x:.6}");
        let t = t.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" {
            "0".to_owned()
        } else {
            t.to_owned()
        }
    };
    match kind.value_unit() {
        Some(crate::Unit::Angle) => format!("{}°", short(v.to_degrees())),
        _ => format!("{} mm", short(v)),
    }
}

impl JointOrigin {
    fn check(&self, asm: &Assembly) -> Result<(), DocError> {
        if !asm.exists(self.component) {
            return Err(bad(format!(
                "a joint origin is on component {}, which does not exist",
                self.component
            )));
        }
        let name_ok = |s: &str| !s.is_empty() && s.chars().count() <= 1024;
        match &self.at {
            OriginAt::Point { point, axis } => {
                let p = [point.x, point.y, point.z];
                if !p.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE) {
                    return Err(bad("a joint origin point must be finite and within range"));
                }
                if !axis.is_finite() || axis.normalized().is_none() {
                    return Err(bad("a joint origin axis must be a finite, non-zero vector"));
                }
            }
            OriginAt::Frame { frame } => {
                let o = frame.origin();
                if ![o.x, o.y, o.z].iter().all(|c| c.abs() <= MAX_COORDINATE) {
                    return Err(bad("a joint origin frame must be within range"));
                }
            }
            OriginAt::Face { face: n }
            | OriginAt::Edge { edge: n }
            | OriginAt::Vertex { edge: n, .. } => {
                if !name_ok(n) {
                    return Err(bad(
                        "a joint origin names a face or edge of 1 to 1024 characters",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Joint {
    /// Checks a joint against the assembly.
    pub fn check(&self, asm: &Assembly) -> Result<(), DocError> {
        check_name(&self.name)?;
        self.a.check(asm)?;
        self.b.check(asm)?;
        if self.a.component == self.b.component {
            return Err(bad("a joint connects two different components"));
        }
        let limit = value_limit(self.kind);
        let finite = |v: f64| v.is_finite() && v.abs() <= limit;
        if !finite(self.value) {
            return Err(bad(format!(
                "joint {:?}: the value must be finite and at most {limit} in size",
                self.name
            )));
        }
        let mm = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        if !mm(self.distance) || !self.offset.iter().all(|v| mm(*v)) {
            return Err(bad(format!(
                "joint {:?}: slides must be finite and within range",
                self.name
            )));
        }
        if let Some([lo, hi]) = self.limits {
            if !(finite(lo) && finite(hi) && lo <= hi) {
                return Err(bad(format!(
                    "joint {:?}: limits must be finite with the lower one first",
                    self.name
                )));
            }
            self.check_value(self.value)?;
        }
        Ok(())
    }

    /// Fails when `value` is outside the joint's limits.
    pub fn check_value(&self, value: f64) -> Result<(), DocError> {
        if self.kind == JointKind::Rigid && value != 0.0 {
            return Err(bad("a rigid joint has no value to drive"));
        }
        if let Some([lo, hi]) = self.limits {
            // A little slack, so a value typed at the limit is not refused by rounding.
            let slack = match self.kind.value_unit() {
                Some(crate::Unit::Angle) => chamfera_geom::ANGULAR_TOLERANCE,
                _ => chamfera_geom::LINEAR_TOLERANCE,
            };
            if value < lo - slack || value > hi + slack {
                return Err(bad(format!(
                    "{} is outside the limits of {:?}: {} to {}",
                    show_value(self.kind, value),
                    self.name,
                    show_value(self.kind, lo),
                    show_value(self.kind, hi)
                )));
            }
        }
        Ok(())
    }
}

/// How the engine reaches a component when it solves positions: through `joint`, from
/// the already placed component `from`. `forward` is true when `from` is the joint's A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Driver {
    pub joint: u32,
    pub from: u32,
    pub forward: bool,
}

/// The joint graph as the engine walks it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topology {
    /// For every component a joint positions, the joint and the side it is reached from.
    pub drivers: BTreeMap<u32, Driver>,
    /// The joints the walk does not use, in order: each one closes a loop.
    pub closing: Vec<u32>,
    /// The joints of each loop group, sorted: loops that share a joint are one group, as
    /// they must be solved together.
    pub loops: Vec<Vec<u32>>,
}

impl Topology {
    /// The loop group a joint is in, if any.
    pub fn loop_of(&self, joint: u32) -> Option<&[u32]> {
        self.loops
            .iter()
            .find(|l| l.contains(&joint))
            .map(Vec::as_slice)
    }
}

/// Walks the joints from the grounded components outward.
///
/// Joints group components; each group is walked breadth-first from its anchors: its
/// grounded components (the root counts as grounded), or, in a group with none, the A
/// side of its first joint. Anchors keep their own placement; every other member is
/// placed by the joint it was first reached through. A joint between two components
/// already reached closes a loop: it, and the tree joints between its two ends (through
/// the ground when both ends hang from grounded components), form the loop.
pub fn topology(asm: &Assembly, joints: &[Joint]) -> Topology {
    let mut adjacent: BTreeMap<u32, Vec<(u32, u32, bool)>> = BTreeMap::new();
    for j in joints {
        adjacent
            .entry(j.a.component)
            .or_default()
            .push((j.id, j.b.component, true));
        adjacent
            .entry(j.b.component)
            .or_default()
            .push((j.id, j.a.component, false));
    }
    let grounded = |id: u32| id == ROOT || asm.component(id).is_ok_and(|c| c.grounded);
    let mut out = Topology::default();
    let mut used: BTreeSet<u32> = BTreeSet::new();
    // Every grounded member is an anchor before any walk starts, so no joint "reaches" a
    // grounded component; then the A side of each joint, for groups with none grounded.
    let mut seen: BTreeSet<u32> = adjacent.keys().copied().filter(|c| grounded(*c)).collect();
    let mut anchors: Vec<u32> = seen.iter().copied().collect();
    anchors.extend(joints.iter().map(|j| j.a.component));
    let mut walked: BTreeSet<u32> = BTreeSet::new();
    for anchor in anchors {
        if !walked.insert(anchor) || (!grounded(anchor) && !seen.insert(anchor)) {
            continue;
        }
        let mut queue = std::collections::VecDeque::from([anchor]);
        while let Some(at) = queue.pop_front() {
            for &(joint, other, forward) in adjacent.get(&at).map_or(&[][..], Vec::as_slice) {
                if seen.insert(other) {
                    used.insert(joint);
                    walked.insert(other);
                    out.drivers.insert(
                        other,
                        Driver {
                            joint,
                            from: at,
                            forward,
                        },
                    );
                    queue.push_back(other);
                }
            }
        }
    }
    out.closing = joints
        .iter()
        .map(|j| j.id)
        .filter(|id| !used.contains(id))
        .collect();
    // The chain of (component, joint that placed it) up to its anchor. Bounded: each step
    // follows a tree edge and the tree has one per placed component.
    let chain = |from: u32| -> Vec<(u32, Option<u32>)> {
        let mut at = from;
        let mut nodes = Vec::new();
        for _ in 0..=out.drivers.len() {
            match out.drivers.get(&at) {
                Some(d) => {
                    nodes.push((at, Some(d.joint)));
                    at = d.from;
                }
                None => break,
            }
        }
        nodes.push((at, None));
        nodes
    };
    // Union-find over joint ids: the joints of one loop, and loops sharing a joint, join.
    let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
    let find = |parent: &mut BTreeMap<u32, u32>, x: u32| -> u32 {
        let mut root = x;
        for _ in 0..=parent.len() {
            match parent.get(&root) {
                Some(p) if *p != root => root = *p,
                _ => break,
            }
        }
        parent.insert(x, root);
        root
    };
    for &id in &out.closing {
        let Some(j) = joints.iter().find(|j| j.id == id) else {
            continue;
        };
        let (ca, cb) = (chain(j.a.component), chain(j.b.component));
        // The lowest common component of the two chains; none when they hang from two
        // different anchors (both grounded: the loop runs through the ground).
        let meet = cb
            .iter()
            .map(|(n, _)| *n)
            .find(|n| ca.iter().any(|(m, _)| m == n));
        let mut cycle = vec![id];
        for c in [&ca, &cb] {
            for (node, joint) in c.iter() {
                if Some(*node) == meet {
                    break;
                }
                if let Some(t) = joint {
                    cycle.push(*t);
                }
            }
        }
        let first = find(&mut parent, id);
        for t in cycle {
            let r = find(&mut parent, t);
            if r != first {
                parent.insert(r, first);
            }
        }
    }
    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let members: Vec<u32> = parent.keys().copied().collect();
    for id in members {
        let r = find(&mut parent, id);
        groups.entry(r).or_default().push(id);
    }
    out.loops = groups.into_values().collect();
    out
}

/// For every component a joint (or a rigid group) positions, the joint and the side it
/// is reached from.
pub fn drivers(asm: &Assembly) -> BTreeMap<u32, Driver> {
    topology(asm, &asm.solving_joints()).drivers
}

impl Assembly {
    pub fn joint(&self, id: u32) -> Result<&Joint, DocError> {
        self.joints
            .iter()
            .find(|j| j.id == id)
            .ok_or_else(|| bad(format!("no joint {id}")))
    }

    pub fn joint_mut(&mut self, id: u32) -> Result<&mut Joint, DocError> {
        self.joints
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| bad(format!("no joint {id}")))
    }

    /// Adds a joint, refusing loops. Fails without changing the assembly.
    pub fn add_joint(&mut self, mut joint: Joint) -> Result<u32, DocError> {
        if self.joints.len() >= MAX_JOINTS {
            return Err(bad(format!("a design holds at most {MAX_JOINTS} joints")));
        }
        let before = self.clone();
        joint.id = self.next_id()?;
        if joint.name.is_empty() {
            let n = self.joints.len() + 1;
            joint.name = format!("Joint{n}");
        }
        if let Err(e) = joint.check(self) {
            *self = before;
            return Err(e);
        }
        self.joints.push(joint);
        self.joints
            .last()
            .map(|j| j.id)
            .ok_or_else(|| bad("no joint"))
    }

    pub fn remove_joint(&mut self, id: u32) -> Result<Joint, DocError> {
        let i = self
            .joints
            .iter()
            .position(|j| j.id == id)
            .ok_or_else(|| bad(format!("no joint {id}")))?;
        Ok(self.joints.remove(i))
    }

    /// The joint that positions a component, if any (a rigid group's link counts: see
    /// [`Assembly::group_of`]).
    pub fn driver_of(&self, component: u32) -> Option<Driver> {
        drivers(self).get(&component).copied()
    }

    /// The joint graph with every joint and rigid group link.
    pub fn topology(&self) -> Topology {
        topology(self, &self.solving_joints())
    }

    /// Makes a joint its loop's input: it is driven, and the other joints of its loop
    /// group are solved. A joint in no loop is left as it is.
    pub fn make_driver(&mut self, id: u32) {
        let topo = self.topology();
        if let Some(group) = topo.loop_of(id) {
            for j in self.joints.iter_mut().filter(|j| group.contains(&j.id)) {
                j.driven = j.id == id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(component: u32) -> JointOrigin {
        JointOrigin {
            component,
            at: OriginAt::Point {
                point: Point3::ORIGIN,
                axis: Vec3::Z,
            },
        }
    }

    fn joint(kind: JointKind, a: u32, b: u32) -> Joint {
        Joint {
            id: 0,
            name: String::new(),
            kind,
            a: origin(a),
            b: origin(b),
            value: 0.0,
            distance: 0.0,
            offset: [0.0, 0.0],
            limits: None,
            flip: false,
            driven: false,
            as_built: false,
        }
    }

    #[test]
    fn a_ring_of_joints_is_one_loop_and_a_tail_off_it_is_not() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let c = asm.add_component("C", ROOT, &[]).unwrap();
        let d = asm.add_component("D", ROOT, &[]).unwrap();
        asm.component_mut(a).unwrap().grounded = true;
        let j1 = asm.add_joint(joint(JointKind::Revolute, a, b)).unwrap();
        let j2 = asm.add_joint(joint(JointKind::Slider, b, c)).unwrap();
        let tail = asm.add_joint(joint(JointKind::Rigid, c, d)).unwrap();
        assert!(asm.topology().loops.is_empty());
        // C back to A closes the ring A - B - C - A; D hangs off C, outside the ring. The
        // walk goes out from grounded A along J1 and J3 first, so J2 is the one that closes.
        let j3 = asm.add_joint(joint(JointKind::Revolute, c, a)).unwrap();
        let t = asm.topology();
        assert_eq!(t.closing, vec![j2]);
        assert_eq!(t.loops, vec![vec![j1, j2, j3]]);
        assert!(t.loop_of(tail).is_none());
        // Making J2 the loop's input drives it alone.
        asm.joint_mut(j1).unwrap().driven = true;
        asm.make_driver(j2);
        let driven: Vec<bool> = asm.joints.iter().map(|j| j.driven).collect();
        assert_eq!(driven, vec![false, true, false, false]);
        // A component jointed to itself is still refused, and changes nothing.
        let before = asm.clone();
        assert!(asm.add_joint(joint(JointKind::Rigid, a, a)).is_err());
        assert_eq!(asm, before);
    }

    #[test]
    fn a_chain_between_two_grounded_components_is_a_loop_through_the_ground() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let c = asm.add_component("C", ROOT, &[]).unwrap();
        asm.component_mut(a).unwrap().grounded = true;
        asm.component_mut(c).unwrap().grounded = true;
        let j1 = asm.add_joint(joint(JointKind::Revolute, a, b)).unwrap();
        let j2 = asm.add_joint(joint(JointKind::Revolute, b, c)).unwrap();
        let t = asm.topology();
        // Both grounded components are anchors: B is reached from A, and J2 closes.
        assert_eq!(t.closing, vec![j2]);
        assert_eq!(t.loops, vec![vec![j1, j2]]);
        assert!(!t.drivers.contains_key(&c));
    }

    #[test]
    fn positions_are_solved_outward_from_the_grounded_component() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let c = asm.add_component("C", ROOT, &[]).unwrap();
        // Joints B -> C (B is A side) and C -> A, with only C grounded: the walk starts at
        // C, so both A and B are placed by joints, reached from C.
        asm.component_mut(c).unwrap().grounded = true;
        let j1 = asm.add_joint(joint(JointKind::Revolute, b, c)).unwrap();
        let j2 = asm.add_joint(joint(JointKind::Slider, c, a)).unwrap();
        let d = drivers(&asm);
        assert_eq!(
            d.get(&a).map(|x| (x.joint, x.from, x.forward)),
            Some((j2, c, true))
        );
        assert_eq!(
            d.get(&b).map(|x| (x.joint, x.from, x.forward)),
            Some((j1, c, false))
        );
        assert!(!d.contains_key(&c));
    }

    #[test]
    fn values_outside_the_limits_are_refused() {
        let mut j = joint(JointKind::Revolute, 1, 2);
        j.limits = Some([0.0, std::f64::consts::FRAC_PI_2]);
        assert!(j.check_value(0.5).is_ok());
        let e = j.check_value(2.0).unwrap_err().to_string();
        assert!(e.contains("90°"), "{e}");
        assert!(joint(JointKind::Rigid, 1, 2).check_value(1.0).is_err());
    }
}

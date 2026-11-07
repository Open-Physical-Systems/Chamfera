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
//! Joints and grounding must form a tree: the engine solves positions by walking out from
//! the grounded components along joints, an exact chain with no iteration. Loops are
//! refused for now ([`check_tree`]).

use crate::DocError;
use crate::component::{Assembly, ROOT, check_name};
use chamfera_geom::{MAX_COORDINATE, Point3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Checks a joint against the assembly (not the tree rule: see [`check_tree`]).
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

/// Union-find over component ids plus a virtual "ground" node.
struct Forest {
    parent: BTreeMap<u64, u64>,
}

/// The virtual node every grounded component (and the root) is tied to.
const GROUND: u64 = u64::MAX;

impl Forest {
    fn find(&mut self, x: u64) -> u64 {
        let mut root = x;
        // Bounded: each step moves up a tree that only ever gets shallower.
        for _ in 0..=self.parent.len() {
            match self.parent.get(&root) {
                Some(p) if *p != root => root = *p,
                _ => break,
            }
        }
        self.parent.insert(x, root);
        root
    }

    /// Joins two sets; false when they were already one (the edge closes a loop).
    fn union(&mut self, a: u64, b: u64) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.parent.insert(ra, rb);
        true
    }
}

/// Refuses joints that close a loop: two joints between the same components, a ring of
/// joints, or a chain of joints between two grounded components.
pub fn check_tree(asm: &Assembly) -> Result<(), DocError> {
    let mut forest = Forest {
        parent: BTreeMap::new(),
    };
    forest.union(u64::from(ROOT), GROUND);
    for c in asm.components.iter().filter(|c| c.grounded) {
        forest.union(u64::from(c.id), GROUND);
    }
    for j in &asm.joints {
        if !forest.union(u64::from(j.a.component), u64::from(j.b.component)) {
            return Err(bad(format!(
                "joint {:?} would close a loop: {} and {} are already held together by \
                 other joints or by being grounded. Loops of joints are not supported yet; \
                 delete a joint or unground a component",
                j.name,
                asm.name_of(j.a.component),
                asm.name_of(j.b.component)
            )));
        }
    }
    Ok(())
}

/// How the engine reaches a component when it solves positions: through `joint`, from
/// the already placed component `from`. `forward` is true when `from` is the joint's A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Driver {
    pub joint: u32,
    pub from: u32,
    pub forward: bool,
}

/// For every component a joint positions, the joint and the side it is reached from.
///
/// Joints group components; each group is walked breadth-first from its anchors: its
/// grounded components (the root counts as grounded), or, in a group with none, the A
/// side of its first joint. Anchors keep their own placement; every other member is
/// placed by the joint it was reached through. The tree rule ([`check_tree`]) guarantees
/// each component is reached once.
pub fn drivers(asm: &Assembly) -> BTreeMap<u32, Driver> {
    let mut adjacent: BTreeMap<u32, Vec<(u32, u32, bool)>> = BTreeMap::new();
    for j in &asm.joints {
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
    let mut out: BTreeMap<u32, Driver> = BTreeMap::new();
    let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    // Anchors: every grounded member first, then the A side of each joint in id order
    // for groups that have no grounded member.
    let mut anchors: Vec<u32> = adjacent.keys().copied().filter(|c| grounded(*c)).collect();
    anchors.extend(asm.joints.iter().map(|j| j.a.component));
    for anchor in anchors {
        if !seen.insert(anchor) {
            continue;
        }
        let mut queue = std::collections::VecDeque::from([anchor]);
        while let Some(at) = queue.pop_front() {
            for &(joint, other, forward) in adjacent.get(&at).map_or(&[][..], Vec::as_slice) {
                if seen.insert(other) {
                    out.insert(
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
    out
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
        joint.check(self)?;
        self.joints.push(joint);
        if let Err(e) = check_tree(self) {
            *self = before;
            return Err(e);
        }
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

    /// The joint that positions a component, if any.
    pub fn driver_of(&self, component: u32) -> Option<Driver> {
        drivers(self).get(&component).copied()
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
        }
    }

    #[test]
    fn loops_of_joints_are_refused_and_change_nothing() {
        let mut asm = Assembly::default();
        let a = asm.add_component("A", ROOT, &[]).unwrap();
        let b = asm.add_component("B", ROOT, &[]).unwrap();
        let c = asm.add_component("C", ROOT, &[]).unwrap();
        asm.component_mut(a).unwrap().grounded = true;
        asm.add_joint(joint(JointKind::Revolute, a, b)).unwrap();
        asm.add_joint(joint(JointKind::Slider, b, c)).unwrap();
        let before = asm.clone();
        // C back to A closes the ring A - B - C - A.
        let e = asm.add_joint(joint(JointKind::Rigid, c, a)).unwrap_err();
        assert!(e.to_string().contains("loop"), "{e}");
        // A second joint between the same two.
        assert!(asm.add_joint(joint(JointKind::Rigid, a, b)).is_err());
        // C to the (grounded) root: C is already held through A, which is grounded.
        assert!(asm.add_joint(joint(JointKind::Rigid, ROOT, c)).is_err());
        assert_eq!(asm, before);
        // A component jointed to itself.
        assert!(asm.add_joint(joint(JointKind::Rigid, a, a)).is_err());
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

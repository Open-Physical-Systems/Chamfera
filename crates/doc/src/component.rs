//! Components: the tree a design's bodies are organised in (format 15).
//!
//! The root component is the design itself (id [`ROOT`]); it is never stored and is always
//! grounded at the world origin. Every other component has a parent, a name, a placement
//! (a rigid transform from its own coordinates into its parent's) and owns the bodies of
//! the features listed in `features`. A body belongs to the component that owns the
//! feature that first made it; features nobody lists belong to the root, so a design
//! from before format 15 opens as one root component.
//!
//! A component can be an occurrence of another (`occurrence_of`): a linked copy that
//! shows the same bodies at its own placement and owns no features itself.
//!
//! Joints between components are in [`crate::joint`]; the engine solves them.

use crate::DocError;
use crate::joint::Joint;
use chamfera_geom::{MAX_COORDINATE, Transform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The root component's id: the design itself.
pub const ROOT: u32 = 0;
/// Most components one design may hold.
pub const MAX_COMPONENTS: usize = 2_000;
/// Longest component or joint name, in characters.
pub const MAX_COMPONENT_NAME: usize = 256;
/// Most features one component may list.
pub const MAX_COMPONENT_FEATURES: usize = 100_000;

/// One component of the design (not the root).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub id: u32,
    pub name: String,
    /// The component this one sits in; [`ROOT`] for a top-level component.
    #[serde(default)]
    pub parent: u32,
    /// The features (and sketches) whose bodies this component owns, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<u32>,
    /// From this component's coordinates into its parent's.
    #[serde(default)]
    pub placement: Transform,
    /// A grounded component stays where it is placed: joints move the other side.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grounded: bool,
    /// A linked copy of this component: it shows that component's bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_of: Option<u32>,
    /// Draw this component's bodies with a subtle tint of their own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tint: bool,
}

/// Most rigid groups one design may hold.
pub const MAX_GROUPS: usize = 1_000;

/// One member of a rigid group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupMember {
    pub component: u32,
    /// From this member's coordinates into the group's first member's, as they sat when
    /// the group was made (the identity for the first member).
    pub relative: Transform,
}

/// Components locked together (format 41): they move as one, wherever joints or
/// placements put any of them. Solved as rigid links from the first member to each other
/// one, at the relative positions they had when grouped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigidGroup {
    pub id: u32,
    pub name: String,
    pub members: Vec<GroupMember>,
}

/// The components and joints of a design.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Assembly {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<Joint>,
    /// Rigid groups (format 41).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<RigidGroup>,
    /// The component new features go into ([`ROOT`] when none is active).
    #[serde(default)]
    pub active: u32,
    /// The last component or joint id handed out (they share one counter); never reused.
    #[serde(default)]
    pub last_id: u32,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

/// True for a placement a file or a command may hold: a proper rotation with a
/// translation inside the working range.
pub fn is_valid_placement(t: &Transform) -> bool {
    let v = t.translation_part();
    t.is_rigid()
        && [v.x, v.y, v.z]
            .iter()
            .all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE)
}

/// Checks a name for a component or joint.
pub fn check_name(name: &str) -> Result<(), DocError> {
    if name.trim().is_empty() {
        return Err(bad("a name cannot be empty"));
    }
    if name.chars().count() > MAX_COMPONENT_NAME {
        return Err(bad(format!(
            "a name is at most {MAX_COMPONENT_NAME} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(bad("a name cannot hold control characters"));
    }
    Ok(())
}

impl Assembly {
    /// True when the design has no components or joints (the root only).
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
            && self.joints.is_empty()
            && self.groups.is_empty()
            && self.active == ROOT
            && self.last_id == 0
    }

    pub fn component(&self, id: u32) -> Result<&Component, DocError> {
        self.components
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| bad(format!("no component {id}")))
    }

    pub fn component_mut(&mut self, id: u32) -> Result<&mut Component, DocError> {
        self.components
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| bad(format!("no component {id}")))
    }

    /// True for the root or a stored component.
    pub fn exists(&self, id: u32) -> bool {
        id == ROOT || self.components.iter().any(|c| c.id == id)
    }

    /// A component's name; the root is called "(root)" here, the UI shows the design name.
    pub fn name_of(&self, id: u32) -> String {
        if id == ROOT {
            return "the root component".to_owned();
        }
        self.component(id)
            .map_or_else(|_| format!("component {id}"), |c| c.name.clone())
    }

    /// The component that owns a feature's bodies: the one listing it, else the root.
    pub fn owner_of(&self, feature: u32) -> u32 {
        self.components
            .iter()
            .find(|c| c.features.contains(&feature))
            .map_or(ROOT, |c| c.id)
    }

    /// The next id for a component or joint.
    pub(crate) fn next_id(&mut self) -> Result<u32, DocError> {
        let id = self
            .last_id
            .checked_add(1)
            .ok_or_else(|| bad("no more component ids"))?;
        self.last_id = id;
        Ok(id)
    }

    /// Records that the active component owns a new feature or sketch.
    pub(crate) fn adopt(&mut self, feature: u32) {
        let active = self.active;
        if let Ok(c) = self.component_mut(active)
            && c.occurrence_of.is_none()
            && c.features.len() < MAX_COMPONENT_FEATURES
        {
            c.features.push(feature);
        }
    }

    /// True when `id` is `ancestor` or sits (at any depth) inside it.
    pub fn is_inside(&self, id: u32, ancestor: u32) -> bool {
        let mut at = id;
        for _ in 0..=self.components.len() {
            if at == ancestor {
                return true;
            }
            if at == ROOT {
                return false;
            }
            match self.component(at) {
                Ok(c) => at = c.parent,
                Err(_) => return false,
            }
        }
        false
    }

    /// The original a component shows: itself, or what it is an occurrence of.
    pub fn original_of(&self, id: u32) -> u32 {
        self.component(id)
            .ok()
            .and_then(|c| c.occurrence_of)
            .unwrap_or(id)
    }

    /// Adds a component inside `parent` that takes over the given features (from whichever
    /// component held them). Returns its id.
    pub fn add_component(
        &mut self,
        name: &str,
        parent: u32,
        features: &[u32],
    ) -> Result<u32, DocError> {
        check_name(name)?;
        if self.components.len() >= MAX_COMPONENTS {
            return Err(bad(format!(
                "a design holds at most {MAX_COMPONENTS} components"
            )));
        }
        if !self.exists(parent) {
            return Err(bad(format!("no component {parent} to put it in")));
        }
        if self
            .component(parent)
            .is_ok_and(|c| c.occurrence_of.is_some())
        {
            return Err(bad(
                "a linked copy shows another component and cannot hold components of its own",
            ));
        }
        if features.len() > MAX_COMPONENT_FEATURES {
            return Err(bad("too many bodies for one component"));
        }
        let id = self.next_id()?;
        for c in &mut self.components {
            c.features.retain(|f| !features.contains(f));
        }
        let mut owned: Vec<u32> = Vec::new();
        for f in features {
            if !owned.contains(f) {
                owned.push(*f);
            }
        }
        self.components.push(Component {
            id,
            name: name.to_owned(),
            parent,
            features: owned,
            placement: Transform::IDENTITY,
            grounded: false,
            occurrence_of: None,
            tint: false,
        });
        Ok(id)
    }

    /// Adds a linked copy of `source` beside it, at `placement`. Returns its id.
    pub fn add_occurrence(&mut self, source: u32, placement: Transform) -> Result<u32, DocError> {
        if source == ROOT {
            return Err(bad("the root component cannot be copied"));
        }
        if !is_valid_placement(&placement) {
            return Err(bad(
                "the placement must be a move and rotation within range",
            ));
        }
        let original = self.original_of(source);
        let src = self.component(source)?.clone();
        let base = self.component(original)?.name.clone();
        let n = self
            .components
            .iter()
            .filter(|c| c.id == original || c.occurrence_of == Some(original))
            .count();
        let name: String = format!("{base}:{}", n + 1)
            .chars()
            .take(MAX_COMPONENT_NAME)
            .collect();
        let id = self.add_component(&name, src.parent, &[])?;
        let c = self.component_mut(id)?;
        c.occurrence_of = Some(original);
        c.placement = placement;
        c.tint = src.tint;
        Ok(id)
    }

    /// Removes a component. Its bodies and child components move to its parent; its
    /// linked copies and every joint that uses any of them are removed too. Returns the
    /// removed component ids.
    pub fn remove_component(&mut self, id: u32) -> Result<Vec<u32>, DocError> {
        if id == ROOT {
            return Err(bad(
                "the root component is the design and cannot be deleted",
            ));
        }
        let c = self.component(id)?.clone();
        let mut gone = vec![id];
        gone.extend(
            self.components
                .iter()
                .filter(|x| x.occurrence_of == Some(id))
                .map(|x| x.id),
        );
        for x in &mut self.components {
            if gone.contains(&x.parent) {
                x.parent = c.parent;
            }
        }
        if let Ok(p) = self.component_mut(c.parent) {
            for f in &c.features {
                if p.features.len() < MAX_COMPONENT_FEATURES {
                    p.features.push(*f);
                }
            }
        }
        self.components.retain(|x| !gone.contains(&x.id));
        self.joints
            .retain(|j| !gone.contains(&j.a.component) && !gone.contains(&j.b.component));
        self.drop_from_groups(&gone);
        if gone.contains(&self.active) {
            self.active = ROOT;
        }
        Ok(gone)
    }

    /// Validates the assembly as read from a file (hostile input) or after a change.
    pub fn check(&self, last_feature_id: u32) -> Result<(), DocError> {
        if self.components.len() > MAX_COMPONENTS {
            return Err(bad(format!(
                "a design holds at most {MAX_COMPONENTS} components"
            )));
        }
        if self.joints.len() > crate::joint::MAX_JOINTS {
            return Err(bad(format!(
                "a design holds at most {} joints",
                crate::joint::MAX_JOINTS
            )));
        }
        let mut ids = BTreeSet::new();
        let mut owned = BTreeSet::new();
        for c in &self.components {
            if c.id == ROOT || c.id > self.last_id || !ids.insert(c.id) {
                return Err(bad(format!(
                    "component {} has a duplicate or out-of-range id",
                    c.id
                )));
            }
            check_name(&c.name)?;
            if !is_valid_placement(&c.placement) {
                return Err(bad(format!(
                    "component {:?} has a placement that is not a move and rotation within range",
                    c.name
                )));
            }
            if c.features.len() > MAX_COMPONENT_FEATURES {
                return Err(bad(format!("component {:?} lists too many bodies", c.name)));
            }
            for f in &c.features {
                if *f == 0 || *f > last_feature_id || !owned.insert(*f) {
                    return Err(bad(format!(
                        "component {:?} lists feature {f}, which is out of range or owned twice",
                        c.name
                    )));
                }
            }
        }
        for c in &self.components {
            if !self.exists(c.parent) || c.parent == c.id {
                return Err(bad(format!("component {:?} has no valid parent", c.name)));
            }
            if !self.is_inside(c.parent, ROOT) || self.is_inside(c.parent, c.id) {
                return Err(bad(format!("component {:?} sits inside itself", c.name)));
            }
            if let Some(o) = c.occurrence_of {
                let original = self
                    .component(o)
                    .map_err(|_| bad(format!("{:?} copies a missing component", c.name)))?;
                if original.occurrence_of.is_some() || o == c.id || !c.features.is_empty() {
                    return Err(bad(format!(
                        "{:?} must copy an original component and own no bodies",
                        c.name
                    )));
                }
            }
            if self
                .component(c.parent)
                .is_ok_and(|p| p.occurrence_of.is_some())
            {
                return Err(bad(format!("{:?} sits inside a linked copy", c.name)));
            }
        }
        if !self.exists(self.active)
            || self
                .component(self.active)
                .is_ok_and(|c| c.occurrence_of.is_some())
        {
            return Err(bad(format!(
                "the active component {} is missing or a linked copy",
                self.active
            )));
        }
        let mut joint_ids = BTreeSet::new();
        for j in &self.joints {
            if j.id == ROOT || j.id > self.last_id || ids.contains(&j.id) || !joint_ids.insert(j.id)
            {
                return Err(bad(format!(
                    "joint {} has a duplicate or out-of-range id",
                    j.id
                )));
            }
            j.check(self)?;
        }
        if self.groups.len() > MAX_GROUPS {
            return Err(bad(format!(
                "a design holds at most {MAX_GROUPS} rigid groups"
            )));
        }
        let mut grouped = BTreeSet::new();
        for g in &self.groups {
            if g.id == ROOT || g.id > self.last_id || ids.contains(&g.id) || !joint_ids.insert(g.id)
            {
                return Err(bad(format!(
                    "rigid group {} has a duplicate or out-of-range id",
                    g.id
                )));
            }
            check_name(&g.name)?;
            check_members(self, g.members.iter().map(|m| m.component), &mut grouped)?;
            for (i, m) in g.members.iter().enumerate() {
                let first_is_identity = i > 0 || m.relative == Transform::IDENTITY;
                if !is_valid_placement(&m.relative) || !first_is_identity {
                    return Err(bad(format!(
                        "rigid group {:?} holds a member at a placement that is not a move \
                         and rotation within range",
                        g.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// The joints the engine solves: the design's joints, then one rigid link from each
    /// rigid group's first member to each other member. A link's id is above every id
    /// handed out (`last_id`), so it never clashes; see [`Assembly::link_group`].
    pub fn solving_joints(&self) -> Vec<Joint> {
        let mut out = self.joints.clone();
        let mut next = u64::from(self.last_id) + 1;
        for g in &self.groups {
            let Some(first) = g.members.first() else {
                continue;
            };
            for m in g.members.iter().skip(1) {
                let Ok(id) = u32::try_from(next) else {
                    return out;
                };
                next += 1;
                let frame = |t: &Transform| {
                    chamfera_geom::Frame::from_normal_and_x(
                        t.apply_point(chamfera_geom::Point3::ORIGIN),
                        t.apply_vector(chamfera_geom::Vec3::Z),
                        t.apply_vector(chamfera_geom::Vec3::X),
                    )
                };
                let (Some(on_first), Some(on_member)) =
                    (frame(&m.relative), frame(&Transform::IDENTITY))
                else {
                    continue;
                };
                out.push(Joint {
                    id,
                    name: g.name.clone(),
                    kind: crate::JointKind::Rigid,
                    a: crate::JointOrigin {
                        component: first.component,
                        at: crate::OriginAt::Frame { frame: on_first },
                    },
                    b: crate::JointOrigin {
                        component: m.component,
                        at: crate::OriginAt::Frame { frame: on_member },
                    },
                    value: 0.0,
                    distance: 0.0,
                    offset: [0.0, 0.0],
                    limits: None,
                    flip: false,
                    driven: false,
                    as_built: true,
                });
            }
        }
        out
    }

    /// The rigid group a solving link (from [`Assembly::solving_joints`]) belongs to, or
    /// `None` for a design joint.
    pub fn link_group(&self, joint: u32) -> Option<u32> {
        if joint <= self.last_id {
            return None;
        }
        let mut next = u64::from(self.last_id) + 1;
        for g in &self.groups {
            let links = g.members.len().saturating_sub(1) as u64;
            if u64::from(joint) < next + links {
                return Some(g.id);
            }
            next += links;
        }
        None
    }

    /// The rigid group a component is in, if any.
    pub fn group_of(&self, component: u32) -> Option<&RigidGroup> {
        self.groups
            .iter()
            .find(|g| g.members.iter().any(|m| m.component == component))
    }

    pub fn group(&self, id: u32) -> Result<&RigidGroup, DocError> {
        self.groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| bad(format!("no rigid group {id}")))
    }

    /// Adds a rigid group of `members`, each given with its world placement now (from its
    /// coordinates to the world): they keep these relative positions. Returns its id.
    pub fn add_group(&mut self, name: &str, members: &[(u32, Transform)]) -> Result<u32, DocError> {
        if self.groups.len() >= MAX_GROUPS {
            return Err(bad(format!(
                "a design holds at most {MAX_GROUPS} rigid groups"
            )));
        }
        check_name(name)?;
        let mut grouped = BTreeSet::new();
        for g in &self.groups {
            grouped.extend(g.members.iter().map(|m| m.component));
        }
        check_members(self, members.iter().map(|m| m.0), &mut grouped)?;
        let Some((first_id, first_world)) = members.first() else {
            return Err(bad("a rigid group needs at least two components"));
        };
        let to_first = first_world
            .inverse_rigid()
            .ok_or_else(|| bad("the first component's placement is not a move and rotation"))?;
        let mut list = Vec::with_capacity(members.len());
        for (i, (c, world)) in members.iter().enumerate() {
            let relative = if i == 0 {
                Transform::IDENTITY
            } else {
                world.then(&to_first)
            };
            if !is_valid_placement(&relative) {
                return Err(bad(format!(
                    "{} is too far from {} to group",
                    self.name_of(*c),
                    self.name_of(*first_id)
                )));
            }
            list.push(GroupMember {
                component: *c,
                relative,
            });
        }
        let id = self.next_id()?;
        self.groups.push(RigidGroup {
            id,
            name: name.to_owned(),
            members: list,
        });
        Ok(id)
    }

    pub fn remove_group(&mut self, id: u32) -> Result<RigidGroup, DocError> {
        let i = self
            .groups
            .iter()
            .position(|g| g.id == id)
            .ok_or_else(|| bad(format!("no rigid group {id}")))?;
        Ok(self.groups.remove(i))
    }

    /// Takes removed components out of their groups; a group left with fewer than two
    /// members goes. When the first member goes, the next becomes the first and the
    /// others' relative positions are re-based on it.
    fn drop_from_groups(&mut self, gone: &[u32]) {
        for g in &mut self.groups {
            let first_gone = g
                .members
                .first()
                .is_some_and(|m| gone.contains(&m.component));
            g.members.retain(|m| !gone.contains(&m.component));
            if first_gone
                && let Some(base) = g.members.first().and_then(|m| m.relative.inverse_rigid())
            {
                for m in &mut g.members {
                    m.relative = m.relative.then(&base);
                }
                if let Some(m) = g.members.first_mut() {
                    m.relative = Transform::IDENTITY;
                }
            }
        }
        self.groups.retain(|g| g.members.len() >= 2);
    }
}

/// Checks a rigid group's members: two or more distinct components (not the root), each
/// in no other group (`grouped` collects the ones already taken).
fn check_members(
    asm: &Assembly,
    members: impl Iterator<Item = u32>,
    grouped: &mut BTreeSet<u32>,
) -> Result<(), DocError> {
    let mut n = 0usize;
    let mut own = BTreeSet::new();
    for c in members {
        n += 1;
        if n > MAX_COMPONENTS {
            return Err(bad("a rigid group lists too many components"));
        }
        if c == ROOT || !asm.exists(c) {
            return Err(bad(format!(
                "a rigid group holds components (not the root); {c} is not one"
            )));
        }
        if !own.insert(c) {
            return Err(bad(format!(
                "{} is listed twice in a rigid group",
                asm.name_of(c)
            )));
        }
        if !grouped.insert(c) {
            return Err(bad(format!(
                "{} is already in a rigid group: ungroup it first",
                asm.name_of(c)
            )));
        }
    }
    if n < 2 {
        return Err(bad("a rigid group needs at least two components"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_component_takes_its_bodies_from_whoever_held_them() {
        let mut a = Assembly::default();
        let c1 = a.add_component("Base", ROOT, &[2, 3]).unwrap();
        assert_eq!(a.owner_of(2), c1);
        assert_eq!(a.owner_of(9), ROOT, "unlisted features stay in the root");
        let c2 = a.add_component("Lid", c1, &[3]).unwrap();
        assert_eq!(a.owner_of(3), c2);
        assert_eq!(a.component(c1).unwrap().features, vec![2]);
        assert!(a.is_inside(c2, c1));
        a.check(10).unwrap();
    }

    #[test]
    fn deleting_a_component_hands_its_bodies_and_children_to_its_parent() {
        let mut a = Assembly::default();
        let c1 = a.add_component("Base", ROOT, &[2]).unwrap();
        let c2 = a.add_component("Lid", c1, &[3]).unwrap();
        let copy = a.add_occurrence(c1, Transform::IDENTITY).unwrap();
        a.active = c1;
        let gone = a.remove_component(c1).unwrap();
        assert_eq!(gone, vec![c1, copy], "its linked copies go with it");
        assert_eq!(a.owner_of(2), ROOT);
        assert_eq!(a.component(c2).unwrap().parent, ROOT);
        assert_eq!(a.active, ROOT);
        assert!(a.remove_component(ROOT).is_err());
        a.check(10).unwrap();
    }

    #[test]
    fn a_rigid_group_keeps_relative_places_and_rebases_when_its_first_member_goes() {
        use chamfera_geom::Vec3;
        let mut a = Assembly::default();
        let c1 = a.add_component("A", ROOT, &[]).unwrap();
        let c2 = a.add_component("B", ROOT, &[]).unwrap();
        let c3 = a.add_component("C", ROOT, &[]).unwrap();
        let at = |x: f64| Transform::translation(Vec3::new(x, 0.0, 0.0)).unwrap();
        // A at x = 10, B at x = 30, C at x = 70: B is 20 from A, C 60 from A.
        let g = a
            .add_group("G", &[(c1, at(10.0)), (c2, at(30.0)), (c3, at(70.0))])
            .unwrap();
        let rel =
            |a: &Assembly, i: usize| a.group(g).unwrap().members[i].relative.translation_part().x;
        assert_eq!((rel(&a, 0), rel(&a, 1), rel(&a, 2)), (0.0, 20.0, 60.0));
        assert_eq!(
            a.solving_joints().len(),
            2,
            "two links from the first member"
        );
        assert_eq!(a.link_group(a.last_id + 1), Some(g));
        a.check(10).unwrap();
        // A component is in one group at most.
        assert!(a.add_group("H", &[(c2, at(0.0)), (c3, at(0.0))]).is_err());
        // Removing A makes B the first: C is now 40 from it.
        a.remove_component(c1).unwrap();
        assert_eq!(a.group(g).unwrap().members.len(), 2);
        assert_eq!((rel(&a, 0), rel(&a, 1)), (0.0, 40.0));
        a.check(10).unwrap();
        // Removing B leaves a group of one, which goes.
        a.remove_component(c2).unwrap();
        assert!(a.groups.is_empty());
    }

    #[test]
    fn hostile_assemblies_are_refused() {
        let mut a = Assembly::default();
        let c1 = a.add_component("Base", ROOT, &[2]).unwrap();
        // A parent loop.
        let mut b = a.clone();
        b.component_mut(c1).unwrap().parent = c1;
        assert!(b.check(10).is_err());
        // A feature out of range.
        assert!(a.check(1).is_err());
        // A placement that scales.
        let mut b = a.clone();
        b.component_mut(c1).unwrap().placement = Transform::scale(2.0).unwrap();
        assert!(b.check(10).is_err());
        // Names.
        assert!(a.add_component("", ROOT, &[]).is_err());
        assert!(a.add_component(&"x".repeat(300), ROOT, &[]).is_err());
        assert!(a.add_component("a\u{0}", ROOT, &[]).is_err());
        assert!(a.add_component("ok", 999, &[]).is_err());
    }
}

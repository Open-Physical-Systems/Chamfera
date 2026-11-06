//! The timeline's order, its rollback marker and its groups.
//!
//! Sketches and features live in two lists ([`Document::sketches`], [`Document::features`]);
//! the timeline says in which order they build. Items before the marker are built, items
//! after it are kept but not built (the design is "rolled back"), and a new sketch or
//! feature goes in at the marker. A group gathers adjacent items under one name.
//!
//! Designs before format 16 have no timeline: they open in id order (the order they were
//! made in, as the timeline showed them) with the marker at the end.

use crate::{DocError, Document, FeatureKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Most groups one design may hold.
pub const MAX_GROUPS: usize = 10_000;
/// Longest name of a sketch, feature or group.
pub const MAX_ITEM_NAME_LEN: usize = 256;

/// Adjacent timeline items shown together under one name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: u32,
    pub name: String,
    /// Sketch and feature ids, in timeline order. Always adjacent in the timeline.
    pub items: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Timeline {
    /// Sketch and feature ids in build order.
    #[serde(default)]
    pub order: Vec<u32>,
    /// How many items (from the start) are built; `None` builds them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

/// What a timeline id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Sketch,
    Feature,
    Group,
}

fn timeline_err(reason: impl Into<String>) -> DocError {
    DocError::Timeline(reason.into())
}

/// Checks a name a user typed for a sketch, feature or group, and returns it trimmed.
pub fn check_item_name(name: &str) -> Result<String, DocError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(timeline_err("a name cannot be empty"));
    }
    if name.chars().count() > MAX_ITEM_NAME_LEN {
        return Err(timeline_err(format!(
            "a name is at most {MAX_ITEM_NAME_LEN} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(timeline_err("a name cannot hold control characters"));
    }
    Ok(name.to_owned())
}

/// The feature ids an edge name refers to: `2:top 0` names feature 2, `between 2:side 0
/// and 4:top` names 2 and 4.
fn edge_features(edge: &str) -> Vec<u32> {
    edge.split_whitespace()
        .filter_map(|w| w.split_once(':'))
        .filter_map(|(id, _)| id.parse::<u32>().ok())
        .collect()
}

/// The sketches and features a feature uses, sorted.
pub fn kind_dependencies(kind: &FeatureKind) -> Vec<u32> {
    let mut deps = Vec::new();
    if let Some(s) = kind.sketch() {
        deps.push(s);
    }
    match kind {
        FeatureKind::Pattern(p) => deps.push(p.source),
        FeatureKind::Blend(b) => deps.extend(b.edges.iter().flat_map(|e| edge_features(e))),
        // Open faces are named like edges: `2:top`.
        FeatureKind::Shell(sh) => deps.extend(sh.faces.iter().flat_map(|f| edge_features(f))),
        FeatureKind::Extrude(_) | FeatureKind::Revolve(_) => {}
    }
    deps.sort_unstable();
    deps.dedup();
    deps
}

impl Document {
    /// What `id` names on the timeline, if anything.
    pub fn item_kind(&self, id: u32) -> Option<ItemKind> {
        if self.sketches.iter().any(|s| s.id == id) {
            Some(ItemKind::Sketch)
        } else if self.features.iter().any(|f| f.id == id) {
            Some(ItemKind::Feature)
        } else if self.timeline.groups.iter().any(|g| g.id == id) {
            Some(ItemKind::Group)
        } else {
            None
        }
    }

    /// The name of a sketch, feature or group.
    pub fn item_name(&self, id: u32) -> Option<String> {
        match self.item_kind(id)? {
            ItemKind::Sketch => self.sketch(id).ok().map(|s| s.name.clone()),
            ItemKind::Feature => self.feature(id).ok().map(|f| f.name.clone()),
            ItemKind::Group => self.group(id).map(|g| g.name.clone()),
        }
    }

    fn name_or_id(&self, id: u32) -> String {
        self.item_name(id).unwrap_or_else(|| format!("item {id}"))
    }

    pub fn group(&self, id: u32) -> Option<&Group> {
        self.timeline.groups.iter().find(|g| g.id == id)
    }

    /// The group an item is in, if any.
    pub fn group_of(&self, item: u32) -> Option<&Group> {
        self.timeline
            .groups
            .iter()
            .find(|g| g.items.contains(&item))
    }

    /// Every sketch and feature id in build order. Items the stored order misses (made by
    /// code that does not know about the timeline) are placed at the marker, like a new
    /// item; ids the order holds that no longer exist are left out.
    pub fn timeline_order(&self) -> Vec<u32> {
        self.timeline_view().0
    }

    /// How many items of [`Document::timeline_order`] are built.
    pub fn marker(&self) -> usize {
        self.timeline_view().1
    }

    /// True when the marker is before the last item.
    pub fn rolled_back(&self) -> bool {
        let (order, m) = self.timeline_view();
        m < order.len()
    }

    /// The sketches and features before the marker.
    pub fn built_items(&self) -> BTreeSet<u32> {
        let (order, m) = self.timeline_view();
        order.into_iter().take(m).collect()
    }

    fn timeline_view(&self) -> (Vec<u32>, usize) {
        let exists: BTreeSet<u32> = self
            .sketches
            .iter()
            .map(|s| s.id)
            .chain(self.features.iter().map(|f| f.id))
            .collect();
        let mut seen = BTreeSet::new();
        let mut order = Vec::with_capacity(exists.len());
        let mut marker = None;
        for (k, id) in self.timeline.order.iter().enumerate() {
            if self.timeline.marker == Some(k) {
                marker = Some(order.len());
            }
            if exists.contains(id) && seen.insert(*id) {
                order.push(*id);
            }
        }
        // `None`, or a marker at or past the end, builds everything.
        let m = match (self.timeline.marker, marker) {
            (Some(_), Some(m)) => m,
            _ => order.len(),
        };
        let missing: Vec<u32> = exists.difference(&seen).copied().collect();
        let n = missing.len();
        let at = m.min(order.len());
        order.splice(at..at, missing);
        (order, at + n)
    }

    /// Writes the effective order back: missing items placed, gone ones dropped, groups
    /// cleaned up and the marker `None` when it is at the end.
    pub fn normalize_timeline(&mut self) {
        let (order, m) = self.timeline_view();
        let index: BTreeMap<u32, usize> =
            order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        for g in &mut self.timeline.groups {
            g.items.retain(|i| index.contains_key(i));
            g.items
                .sort_by_key(|i| index.get(i).copied().unwrap_or(usize::MAX));
            g.items.dedup();
        }
        self.timeline.groups.retain(|g| !g.items.is_empty());
        self.timeline.marker = (m < order.len()).then_some(m);
        self.timeline.order = order;
    }

    /// Puts a new sketch or feature (already in its list) at the marker: the view places
    /// items the stored order misses there.
    pub(crate) fn timeline_insert(&mut self, id: u32) {
        self.timeline.order.retain(|x| *x != id);
        self.normalize_timeline();
    }

    /// Drops a deleted item (already gone from its list) from the order and its group; the
    /// marker keeps its place between the remaining items.
    pub(crate) fn timeline_remove(&mut self) {
        self.normalize_timeline();
    }

    /// Moves the marker so the first `position` items are built (`position` from 0 to the
    /// number of items; that number, or more, is the end).
    pub fn roll_to(&mut self, position: usize) -> Result<(), DocError> {
        self.normalize_timeline();
        let n = self.timeline.order.len();
        if position > n {
            return Err(timeline_err(format!(
                "the timeline has {n} items: roll to 0 to {n}"
            )));
        }
        self.timeline.marker = (position < n).then_some(position);
        Ok(())
    }

    /// What an item needs built before it: a feature's sketch, a pattern's source, the
    /// features whose edges a fillet rounds, the feature whose face a sketch is drawn on.
    pub fn item_dependencies(&self, id: u32) -> Vec<u32> {
        let mut deps = Vec::new();
        if let Ok(s) = self.sketch(id)
            && let Some(a) = &s.attached
        {
            deps.push(a.feature);
        }
        if let Ok(f) = self.feature(id) {
            deps = kind_dependencies(&f.kind);
        }
        deps.retain(|d| *d != id);
        deps
    }

    /// Refuses an order in which an item comes before something it uses.
    fn check_order(&self, order: &[u32]) -> Result<(), DocError> {
        let index: BTreeMap<u32, usize> =
            order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        for (i, id) in order.iter().enumerate() {
            for dep in self.item_dependencies(*id) {
                if index.get(&dep).is_some_and(|j| *j > i) {
                    return Err(timeline_err(format!(
                        "{} uses {}, so it cannot come before it",
                        self.name_or_id(*id),
                        self.name_or_id(dep)
                    )));
                }
            }
        }
        Ok(())
    }

    /// Checks that a new feature only uses items that are built (before the marker).
    pub(crate) fn check_new_item_dependencies(&self, deps: &[u32]) -> Result<(), DocError> {
        let built = self.built_items();
        for d in deps {
            if self.item_kind(*d).is_some() && !built.contains(d) {
                return Err(timeline_err(format!(
                    "{} is after the timeline marker: roll forward to use it",
                    self.name_or_id(*d)
                )));
            }
        }
        Ok(())
    }

    /// Moves a sketch, feature or group so that it starts at index `to` of the new order.
    /// Refused when an item would come before something it uses. An item keeps whether it
    /// is built unless it crosses the marker. A moved item leaves its group when it moves
    /// out of it and joins a group it lands inside.
    pub fn move_item(&mut self, item: u32, to: usize) -> Result<(), DocError> {
        self.normalize_timeline();
        let (order, m) = self.timeline_view();
        let block: Vec<u32> = match self.item_kind(item) {
            Some(ItemKind::Group) => self
                .group(item)
                .map(|g| g.items.clone())
                .unwrap_or_default(),
            Some(_) => vec![item],
            None => return Err(timeline_err(format!("no sketch, feature or group {item}"))),
        };
        let Some(first) = block
            .first()
            .and_then(|b| order.iter().position(|x| x == b))
        else {
            return Err(timeline_err("the group is empty"));
        };
        let was_built = first < m;
        let mut rest: Vec<u32> = order
            .iter()
            .copied()
            .filter(|x| !block.contains(x))
            .collect();
        if to > rest.len() {
            return Err(timeline_err(format!(
                "`to` must be 0 to {} for {}",
                rest.len(),
                self.name_or_id(item)
            )));
        }
        let built_before = order.iter().take(m).filter(|x| block.contains(x)).count();
        let m_rest = m - built_before;
        let built = if was_built { to <= m_rest } else { to < m_rest };
        // A group cannot be dropped inside another group.
        let landing_group = (to > 0 && to < rest.len())
            .then(|| {
                let (a, b) = (rest[to - 1], rest[to]);
                self.timeline
                    .groups
                    .iter()
                    .find(|g| g.items.contains(&a) && g.items.contains(&b))
                    .map(|g| g.id)
            })
            .flatten();
        if landing_group.is_some() && self.item_kind(item) == Some(ItemKind::Group) {
            return Err(timeline_err("a group cannot go inside another group"));
        }
        rest.splice(to..to, block.iter().copied());
        self.check_order(&rest)?;
        let m = if built { m_rest + block.len() } else { m_rest };
        // Group membership follows where the item landed.
        if self.item_kind(item) != Some(ItemKind::Group) {
            let index: BTreeMap<u32, usize> =
                rest.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            for g in &mut self.timeline.groups {
                if Some(g.id) == landing_group {
                    if !g.items.contains(&item) {
                        g.items.push(item);
                    }
                    continue;
                }
                if g.items.contains(&item) {
                    let others: Vec<usize> = g
                        .items
                        .iter()
                        .filter(|i| **i != item)
                        .filter_map(|i| index.get(i).copied())
                        .collect();
                    let (lo, hi) = (others.iter().min(), others.iter().max());
                    let inside = match (lo, hi) {
                        (Some(lo), Some(hi)) => to + 1 >= *lo && to <= hi + 1,
                        _ => true,
                    };
                    if !inside {
                        g.items.retain(|i| *i != item);
                    }
                }
            }
        }
        self.timeline.marker = (m < rest.len()).then_some(m);
        self.timeline.order = rest;
        self.normalize_timeline();
        self.check_groups()
    }

    /// Makes a group of adjacent items, named `name` (or GroupN), and returns its id.
    pub fn group_items(&mut self, items: &[u32], name: Option<&str>) -> Result<u32, DocError> {
        self.normalize_timeline();
        if items.is_empty() {
            return Err(timeline_err("pick at least one item to group"));
        }
        if self.timeline.groups.len() >= MAX_GROUPS {
            return Err(timeline_err(format!(
                "a design holds at most {MAX_GROUPS} groups"
            )));
        }
        let order = &self.timeline.order;
        let mut at: Vec<usize> = Vec::with_capacity(items.len());
        for i in items {
            let Some(k) = order.iter().position(|x| x == i) else {
                return Err(timeline_err(format!(
                    "no sketch or feature {i} on the timeline"
                )));
            };
            if let Some(g) = self.group_of(*i) {
                return Err(timeline_err(format!(
                    "{} is already in {}",
                    self.name_or_id(*i),
                    g.name
                )));
            }
            at.push(k);
        }
        at.sort_unstable();
        at.dedup();
        if at.windows(2).any(|w| w[1] != w[0] + 1) {
            return Err(timeline_err("only adjacent timeline items can be grouped"));
        }
        let name = match name {
            Some(n) => check_item_name(n)?,
            None => format!("Group{}", self.timeline.groups.len() + 1),
        };
        let id = self
            .last_feature_id
            .checked_add(1)
            .ok_or_else(|| timeline_err("no ids left"))?;
        self.last_feature_id = id;
        let members = at.iter().filter_map(|k| order.get(*k).copied()).collect();
        self.timeline.groups.push(Group {
            id,
            name,
            items: members,
        });
        Ok(id)
    }

    /// Dissolves a group; its items stay where they are.
    pub fn ungroup(&mut self, group: u32) -> Result<Group, DocError> {
        let i = self
            .timeline
            .groups
            .iter()
            .position(|g| g.id == group)
            .ok_or_else(|| timeline_err(format!("no group {group}")))?;
        Ok(self.timeline.groups.remove(i))
    }

    /// Renames a sketch, feature or group.
    pub fn rename_item(&mut self, id: u32, name: &str) -> Result<(), DocError> {
        let name = check_item_name(name)?;
        match self.item_kind(id) {
            Some(ItemKind::Sketch) => self.sketch_mut(id)?.name = name,
            Some(ItemKind::Feature) => self.feature_mut(id)?.name = name,
            Some(ItemKind::Group) => {
                if let Some(g) = self.timeline.groups.iter_mut().find(|g| g.id == id) {
                    g.name = name;
                }
            }
            None => return Err(timeline_err(format!("no sketch, feature or group {id}"))),
        }
        Ok(())
    }

    /// Every group's items are adjacent in the order, and in no other group.
    fn check_groups(&self) -> Result<(), DocError> {
        let index: BTreeMap<u32, usize> = self
            .timeline
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        let mut used = BTreeSet::new();
        for g in &self.timeline.groups {
            let mut at = Vec::new();
            for i in &g.items {
                if !used.insert(*i) {
                    return Err(timeline_err(format!("item {i} is in two groups")));
                }
                at.push(*index.get(i).ok_or_else(|| {
                    timeline_err(format!("group {} holds missing item {i}", g.id))
                })?);
            }
            at.sort_unstable();
            if at.windows(2).any(|w| w[1] != w[0] + 1) {
                return Err(timeline_err(format!("group {} is not adjacent", g.name)));
            }
        }
        Ok(())
    }

    /// Checks the timeline of a design read from a file (hostile input). `seen` holds the
    /// ids already in use; group ids are added to it.
    pub(crate) fn check_timeline(&mut self, seen: &mut BTreeSet<u32>) -> Result<(), DocError> {
        let exists: BTreeSet<u32> = self
            .sketches
            .iter()
            .map(|s| s.id)
            .chain(self.features.iter().map(|f| f.id))
            .collect();
        let mut once = BTreeSet::new();
        for id in &self.timeline.order {
            if !exists.contains(id) || !once.insert(*id) {
                return Err(timeline_err(format!(
                    "the timeline order names {id} twice or names no sketch or feature"
                )));
            }
        }
        if self
            .timeline
            .marker
            .is_some_and(|m| m > self.timeline.order.len())
        {
            return Err(timeline_err("the timeline marker is past the end"));
        }
        if self.timeline.groups.len() > MAX_GROUPS {
            return Err(timeline_err("too many groups"));
        }
        for g in &self.timeline.groups {
            if !seen.insert(g.id) || g.id == 0 || g.id > self.last_feature_id {
                return Err(timeline_err(format!(
                    "group {} has a duplicate or out-of-range id",
                    g.id
                )));
            }
            check_item_name(&g.name)?;
            if g.items.iter().any(|i| !once.contains(i)) && !self.timeline.order.is_empty() {
                return Err(timeline_err(format!(
                    "group {} holds an unknown item",
                    g.id
                )));
            }
        }
        self.normalize_timeline();
        // Order and groups must make sense together.
        self.check_groups()?;
        self.check_order(&self.timeline.order.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Extrude, ExtrudeDirection, Operation};
    use chamfera_geom::Point2;

    /// Sketch 1 (a rectangle), extrude 2 of it, sketch 3.
    fn design() -> Document {
        let mut doc = Document::default();
        let id = doc.add_sketch(chamfera_geom::Frame::XY).unwrap();
        let s = &mut doc.sketch_mut(id).unwrap().sketch;
        let p: Vec<_> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .iter()
            .map(|(x, y)| s.add_point(Point2::new(*x, *y)).unwrap())
            .collect();
        for i in 0..4 {
            s.add_line(p[i], p[(i + 1) % 4]).unwrap();
        }
        doc.add_feature(FeatureKind::Extrude(Extrude {
            sketch: id,
            profiles: vec![Point2::new(5.0, 5.0)],
            distance: "1".into(),
            direction: ExtrudeDirection::Normal,
            operation: Operation::NewBody,
        }))
        .unwrap();
        doc.add_sketch(chamfera_geom::Frame::XZ).unwrap();
        doc
    }

    #[test]
    fn new_items_go_in_at_the_marker() {
        let mut doc = design();
        assert_eq!(doc.timeline_order(), [1, 2, 3]);
        assert!(!doc.rolled_back());
        doc.roll_to(1).unwrap();
        assert_eq!(doc.built_items(), [1].into());
        let s = doc.add_sketch(chamfera_geom::Frame::YZ).unwrap();
        assert_eq!(doc.timeline_order(), [1, s, 2, 3]);
        assert_eq!(doc.marker(), 2);
        doc.roll_to(4).unwrap();
        assert_eq!(doc.timeline.marker, None);
        assert!(doc.roll_to(5).is_err());
    }

    #[test]
    fn a_feature_cannot_move_before_its_sketch() {
        let mut doc = design();
        let before = doc.clone();
        assert!(doc.move_item(2, 0).is_err());
        // Sketch 3 uses nothing: it may go first.
        doc.move_item(3, 0).unwrap();
        assert_eq!(doc.timeline_order(), [3, 1, 2]);
        assert!(doc.move_item(1, 3).is_err(), "past the end");
        doc.move_item(3, 2).unwrap();
        assert_eq!(doc.timeline, before.timeline);
    }

    #[test]
    fn moving_across_the_marker_changes_what_is_built() {
        let mut doc = design();
        doc.roll_to(2).unwrap(); // 1, 2 | 3
        doc.move_item(3, 0).unwrap(); // 3 is now first and built
        assert_eq!(doc.timeline_order(), [3, 1, 2]);
        assert_eq!(doc.marker(), 3);
        doc.roll_to(1).unwrap(); // 3 | 1, 2
        doc.move_item(2, 1).unwrap_err(); // before its sketch
        // 1 was after the marker; dropped before it, it is built.
        doc.move_item(1, 0).unwrap();
        assert_eq!(doc.timeline_order(), [1, 3, 2]);
        assert_eq!(doc.marker(), 2);
        // Moving an item to where it already is changes nothing.
        let t = doc.timeline.clone();
        doc.move_item(2, 2).unwrap();
        doc.move_item(3, 1).unwrap();
        assert_eq!(doc.timeline, t);
    }

    #[test]
    fn groups_hold_adjacent_items_and_follow_moves() {
        let mut doc = design();
        let four = doc.add_sketch(chamfera_geom::Frame::YZ).unwrap();
        assert!(doc.group_items(&[1, 3], None).is_err(), "not adjacent");
        let g = doc.group_items(&[1, 2], Some(" Base ")).unwrap();
        assert_eq!(doc.group(g).unwrap().name, "Base");
        assert!(doc.group_items(&[2, 3], None).is_err(), "2 is grouped");
        // Moving 3 into the middle of the group joins it.
        doc.move_item(3, 1).unwrap();
        assert_eq!(doc.timeline_order(), [1, 3, 2, four]);
        assert_eq!(doc.group(g).unwrap().items, [1, 3, 2]);
        // Moving it away from the group leaves it.
        doc.move_item(3, 3).unwrap();
        assert_eq!(doc.timeline_order(), [1, 2, four, 3]);
        assert_eq!(doc.group(g).unwrap().items, [1, 2]);
        // A group moves as one block, and not into another group.
        doc.move_item(g, 1).unwrap();
        assert_eq!(doc.timeline_order(), [four, 1, 2, 3]);
        let h = doc.group_items(&[four], None).unwrap();
        assert!(doc.move_item(h, 1).is_err(), "inside Base");
        doc.rename_item(g, "Plate").unwrap();
        assert!(doc.rename_item(g, "  ").is_err());
        assert!(doc.rename_item(g, "a\u{7}b").is_err());
        assert_eq!(doc.ungroup(g).unwrap().name, "Plate");
        assert_eq!(doc.timeline.groups.len(), 1);
    }

    #[test]
    fn the_timeline_survives_a_round_trip_and_old_files_open_in_id_order() {
        let mut doc = design();
        doc.move_item(3, 0).unwrap();
        doc.group_items(&[1, 2], Some("Plate")).unwrap();
        doc.roll_to(1).unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        // Format 15 and older: no timeline, id order, marker at the end.
        let mut v: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        v["version"] = serde_json::json!(9);
        v.as_object_mut().unwrap().remove("timeline");
        let old = Document::from_json(&v.to_string()).unwrap();
        assert_eq!(old.timeline_order(), [1, 2, 3]);
        assert!(!old.rolled_back());
        // A hostile order is refused.
        for bad in [
            serde_json::json!({ "order": [1, 1, 2, 3] }),
            serde_json::json!({ "order": [1, 2, 3, 99] }),
            serde_json::json!({ "order": [2, 1, 3] }),
            serde_json::json!({ "order": [1, 2, 3], "marker": 4 }),
            serde_json::json!({ "order": [1, 2, 3], "groups": [{ "id": 1, "name": "x", "items": [1] }] }),
            serde_json::json!({ "order": [1, 2, 3], "groups": [{ "id": 4, "name": "x", "items": [1, 3] }] }),
        ] {
            let mut v: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
            v["timeline"] = bad.clone();
            v["last_feature_id"] = serde_json::json!(10);
            assert!(Document::from_json(&v.to_string()).is_err(), "{bad}");
        }
    }

    #[test]
    fn edge_names_give_the_features_they_come_from() {
        assert_eq!(edge_features("2:top 0"), [2]);
        assert_eq!(edge_features("between 2:side 0 and 14:top"), [2, 14]);
        assert!(edge_features("nonsense").is_empty());
    }
}

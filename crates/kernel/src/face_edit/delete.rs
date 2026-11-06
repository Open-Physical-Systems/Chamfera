//! Deleting faces and healing the gap (the topology half; the geometry is worked out
//! again by [`Draft::build`]).
//!
//! Two healings, tried in this order for each connected group of deleted faces:
//!
//! 1. **Cap.** Every edge between the group and the rest of the body belongs to an inner
//!    loop of a remaining face, made only of such edges: a hole's wall and floor, a
//!    through hole's wall, a pocket, a boss. Those loops are dropped, so each remaining
//!    face's own surface covers the gap (the hole is filled, the boss is gone).
//! 2. **Collapse a blend.** A single face with one loop of four edges, two opposite
//!    *rails* shared with faces `A` and `B` and two *ends*, each a seam of the face itself
//!    or an edge on an end face `E`: a fillet or chamfer of an edge, or of a circular rim.
//!    The face is the result of a vertex-to-edge expansion of the edge `A ∩ B` (Mantyla,
//!    *An Introduction to Solid Modeling*, chapter 9, Euler operators; Stroud,
//!    *Boundary Representation Modelling Techniques*, the chapter on blending, which builds
//!    a blend the same way), so deleting it is the inverse: each end edge collapses to a
//!    point (its two vertices merge), the two rails become one edge between `A` and `B`,
//!    and the vertices and edges are worked out again on the extended surfaces: the new
//!    edge is their K6 intersection curve, the new corner vertices lie on `A`, `B` and
//!    each end face. Both pairings of opposite edges are tried; the first that gives a
//!    valid body is kept.

use super::draft::Draft;
use crate::{KernelError, KernelResult, intersect_surfaces};
use std::collections::BTreeSet;

/// Deletes the faces `faces` (draft indices) from `draft` and heals the gaps.
pub(super) fn delete_in_draft(draft: &mut Draft, faces: &BTreeSet<usize>) -> KernelResult<()> {
    for group in groups(draft, faces) {
        if cap(draft, &group)? {
            continue;
        }
        let [f] = group.iter().copied().collect::<Vec<_>>()[..] else {
            return Err(KernelError::Unsupported(
                "deleting touching faces that do not fill a hole or remove a boss together; delete them one at a time".into(),
            ));
        };
        collapse(draft, f)?;
    }
    Ok(())
}

/// The connected groups (across shared edges) of `faces`.
fn groups(draft: &Draft, faces: &BTreeSet<usize>) -> Vec<BTreeSet<usize>> {
    let mut left = faces.clone();
    let mut out = Vec::new();
    while let Some(&start) = left.iter().next() {
        let mut group = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(f) = stack.pop() {
            if !left.remove(&f) {
                continue;
            }
            group.insert(f);
            for (_, g) in draft.neighbours(f) {
                if left.contains(&g) {
                    stack.push(g);
                }
            }
        }
        out.push(group);
    }
    out
}

/// Removes the faces of `group` when their boundary is a set of whole inner loops of
/// remaining faces; false (and nothing changed) otherwise.
fn cap(draft: &mut Draft, group: &BTreeSet<usize>) -> KernelResult<bool> {
    let uses = draft.edge_uses();
    let boundary: BTreeSet<usize> = group
        .iter()
        .flat_map(|f| draft.neighbours(*f))
        .filter(|(e, _)| {
            uses.get(*e)
                .is_some_and(|u| u.iter().any(|g| !group.contains(g)))
        })
        .map(|(e, _)| e)
        .collect();
    if boundary.is_empty() {
        return Ok(false);
    }
    // Every boundary edge must lie in an inner loop made only of boundary edges.
    let mut drop: Vec<(usize, usize)> = Vec::new();
    let mut covered = BTreeSet::new();
    for (fi, f) in draft.faces.iter().enumerate() {
        let Some(f) = f else { continue };
        if group.contains(&fi) {
            continue;
        }
        for (li, lp) in f.loops.iter().enumerate() {
            let in_boundary = lp.iter().filter(|(e, _)| boundary.contains(e)).count();
            if in_boundary == 0 {
                continue;
            }
            if li == 0 || in_boundary != lp.len() {
                return Ok(false);
            }
            drop.push((fi, li));
            covered.extend(lp.iter().map(|(e, _)| *e));
        }
    }
    if covered != boundary {
        return Ok(false);
    }
    for (fi, li) in drop.into_iter().rev() {
        if let Some(Some(f)) = draft.faces.get_mut(fi)
            && li < f.loops.len()
        {
            f.loops.remove(li);
        }
    }
    remove_faces(draft, group);
    Ok(true)
}

/// Removes faces and every edge only they used.
fn remove_faces(draft: &mut Draft, group: &BTreeSet<usize>) {
    for f in group {
        if let Some(slot) = draft.faces.get_mut(*f) {
            *slot = None;
        }
    }
    let uses = draft.edge_uses();
    for (e, slot) in draft.edges.iter_mut().enumerate() {
        if uses.get(e).is_some_and(Vec::is_empty) {
            *slot = None;
        }
    }
}

/// Deletes the blend face `f` by collapsing its ends and merging its rails, trying both
/// pairings of opposite edges.
fn collapse(draft: &mut Draft, f: usize) -> KernelResult<()> {
    let Some(Some(face)) = draft.faces.get(f) else {
        return Err(KernelError::InvalidInput(format!("face {f} is gone")));
    };
    let [lp] = &face.loops[..] else {
        return Err(not_a_blend("it has more than one boundary loop"));
    };
    let [u0, u1, u2, u3] = lp[..] else {
        return Err(not_a_blend("its boundary does not have four edges"));
    };
    let mut why = String::new();
    for (rails, ends) in [([u0, u2], [u1, u3]), ([u1, u3], [u0, u2])] {
        let mut trial = draft.clone();
        match collapse_pairing(&mut trial, f, rails, ends) {
            Ok(()) => match trial.build() {
                Ok(_) => {
                    *draft = trial;
                    return Ok(());
                }
                Err(e) => why = e.to_string(),
            },
            Err(e) => why = e.to_string(),
        }
    }
    Err(KernelError::Unsupported(format!(
        "the faces around this face cannot be extended to close the gap ({why})"
    )))
}

fn not_a_blend(why: &str) -> KernelError {
    KernelError::Unsupported(format!(
        "deleting this face: {why}; faces that can be deleted are fillets and chamfers (four edges), or groups of faces that fill a hole or remove a boss"
    ))
}

fn collapse_pairing(
    draft: &mut Draft,
    f: usize,
    rails: [(usize, bool); 2],
    ends: [(usize, bool); 2],
) -> KernelResult<()> {
    let nb = draft.neighbours(f);
    let other = |e: usize| {
        nb.iter()
            .find(|(x, _)| *x == e)
            .map(|(_, g)| *g)
            .ok_or_else(|| not_a_blend("an edge has no face on its other side"))
    };
    let (ra, rb) = (rails[0].0, rails[1].0);
    let (a, b) = (other(ra)?, other(rb)?);
    if ra == rb || a == b || a == f || b == f {
        return Err(not_a_blend("its long edges do not lie on two other faces"));
    }
    let surface = |g: usize| {
        draft
            .faces
            .get(g)
            .and_then(Option::as_ref)
            .map(|x| x.surface)
            .ok_or_else(|| not_a_blend("a neighbour is gone"))
    };
    if intersect_surfaces(&surface(a)?, &surface(b)?)?
        .curves()
        .is_empty()
    {
        return Err(not_a_blend("the faces on either side never meet"));
    }
    for (e, _) in ends {
        let g = other(e)?;
        if g == a || g == b {
            return Err(not_a_blend("an end edge lies on a side face"));
        }
    }
    // Collapse each end: merge its vertices and drop it from its end face.
    for (e, _) in ends {
        let Some(Some(edge)) = draft.edges.get(e) else {
            return Err(not_a_blend("an end edge is gone"));
        };
        let [v0, v1] = edge.vertices;
        draft.merge(v0, v1);
        let g = other(e)?;
        if g != f
            && let Some(Some(end)) = draft.faces.get_mut(g)
        {
            for lp in &mut end.loops {
                lp.retain(|(x, _)| *x != e);
            }
            if end.loops.iter().any(Vec::is_empty) {
                return Err(not_a_blend("an end face would lose its whole boundary"));
            }
        }
    }
    // Merge the rails: `B` uses rail `ra` the other way round from `A`.
    let fwd_in_a = {
        let Some(Some(fa)) = draft.faces.get(a) else {
            return Err(not_a_blend("a side face is gone"));
        };
        fa.loops
            .iter()
            .flatten()
            .find(|(x, _)| *x == ra)
            .map(|(_, d)| *d)
            .ok_or_else(|| not_a_blend("a rail is not on its side face"))?
    };
    let (Some(Some(ea)), Some(Some(eb))) = (draft.edges.get(ra), draft.edges.get(rb)) else {
        return Err(not_a_blend("a rail is gone"));
    };
    let (ea_v, eb_v) = (ea.vertices, eb.vertices);
    let fwd_in_b = {
        let Some(Some(fb)) = draft.faces.get(b) else {
            return Err(not_a_blend("a side face is gone"));
        };
        fb.loops
            .iter()
            .flatten()
            .find(|(x, _)| *x == rb)
            .map(|(_, d)| *d)
            .ok_or_else(|| not_a_blend("a rail is not on its side face"))?
    };
    // A runs p -> q along rail a; B must run q -> p.
    let (p, q) = if fwd_in_a {
        (ea_v[0], ea_v[1])
    } else {
        (ea_v[1], ea_v[0])
    };
    let (s, t) = if fwd_in_b {
        (eb_v[0], eb_v[1])
    } else {
        (eb_v[1], eb_v[0])
    };
    if !(draft.root(s) == draft.root(q) && draft.root(t) == draft.root(p)) {
        return Err(not_a_blend("its long edges do not run side by side"));
    }
    if let Some(Some(fb)) = draft.faces.get_mut(b) {
        for lp in &mut fb.loops {
            for u in lp.iter_mut() {
                if u.0 == rb {
                    *u = (ra, !fwd_in_a);
                }
            }
        }
    }
    if let Some(Some(edge)) = draft.edges.get_mut(ra) {
        edge.dirty = true;
    }
    let mut group = BTreeSet::new();
    group.insert(f);
    remove_faces(draft, &group);
    Ok(())
}

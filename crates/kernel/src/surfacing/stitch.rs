//! Stitching surface bodies along the edges they share.
//!
//! The faces of every body go into one face list. Vertices within [`LINEAR_TOLERANCE`] of
//! each other become one; two rim edges (edges with one face in their own body) with the
//! same end vertices that run through the same points (their midpoints, and for closed
//! edges their quarter points, coincide) become one edge. A shared edge must be used once
//! in each direction, so a body whose faces run the wrong way relative to its neighbour
//! is reversed as a whole: the bodies are 2-coloured by "reversed or not" across the
//! shared edges (a breadth-first search; a contradiction, as a Moebius-like join gives, is
//! refused). When no rim is left the result is a closed solid, turned outward if it came
//! out inside out; otherwise it is a surface body.

use super::{invalid, same_point_on};
use crate::topology::{FaceList, Validation};
use crate::{Body, KernelError, KernelResult, ListEdge, ListFace, ListVertex};
use chamfera_geom::{LINEAR_TOLERANCE, Point3};
use std::collections::HashMap;

/// Most bodies one stitch may join.
pub const MAX_STITCH_BODIES: usize = 1_000;
/// Most vertices all bodies of one stitch may have together.
const MAX_STITCH_VERTICES: usize = 1_000_000;

/// A grid cell for finding coincident points: several tolerances wide, so points within
/// the tolerance are in the same or a neighbouring cell.
fn cell(p: Point3) -> (i64, i64, i64) {
    let s = 4.0 * LINEAR_TOLERANCE;
    let q = |x: f64| (x / s).floor() as i64;
    (q(p.x), q(p.y), q(p.z))
}

/// The bodies joined into one along their shared edges (see the module documentation).
/// Every body must be a valid surface body; at least two bodies, or one body whose own rim
/// edges meet, and at least one shared edge.
pub fn stitch(bodies: &[&Body]) -> KernelResult<Body> {
    if bodies.is_empty() {
        return Err(invalid("pick the surfaces to stitch"));
    }
    if bodies.len() > MAX_STITCH_BODIES {
        return Err(invalid(format!(
            "at most {MAX_STITCH_BODIES} bodies can be stitched at once"
        )));
    }
    let mut lists = Vec::with_capacity(bodies.len());
    let mut total = 0usize;
    for (k, b) in bodies.iter().enumerate() {
        if !b.is_open() {
            return Err(invalid(format!(
                "body {} is a closed solid; stitch joins surface bodies",
                k + 1
            )));
        }
        b.validate_as(Validation::Surface)
            .map_err(|e| invalid(format!("body {} is not a valid surface: {e}", k + 1)))?;
        let list = b.face_list(|_| true)?;
        total = total.saturating_add(list.vertices.len());
        if total > MAX_STITCH_VERTICES {
            return Err(invalid("the surfaces have too many vertices to stitch"));
        }
        lists.push(list);
    }

    // Merge vertices.
    let mut vertices: Vec<ListVertex> = Vec::new();
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut vmap: Vec<Vec<usize>> = Vec::with_capacity(lists.len());
    for list in &lists {
        let mut m = Vec::with_capacity(list.vertices.len());
        for v in &list.vertices {
            let (cx, cy, cz) = cell(v.point);
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(ids) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                            for &i in ids {
                                if vertices[i].point.coincident(v.point) {
                                    found = Some(i);
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            let id = match found {
                Some(i) => i,
                None => {
                    vertices.push(v.clone());
                    grid.entry((cx, cy, cz))
                        .or_default()
                        .push(vertices.len() - 1);
                    vertices.len() - 1
                }
            };
            m.push(id);
        }
        vmap.push(m);
    }

    // Edges: rim edges of different bodies (or of one body) that coincide become one.
    let mut edges: Vec<ListEdge> = Vec::new();
    // Per body and its edge: (global edge, same direction as the global edge).
    let mut emap: Vec<Vec<(usize, bool)>> = Vec::with_capacity(lists.len());
    // Rim edges by their sorted end vertices: (global edge, body).
    let mut rims: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
    // Shared edges: (body a, body b, the uses run the same way so one must flip).
    let mut links: Vec<(usize, usize, bool)> = Vec::new();
    for (bi, list) in lists.iter().enumerate() {
        let mut uses = vec![(0usize, false); list.edges.len()];
        let mut dir_of = vec![true; list.edges.len()];
        for f in &list.faces {
            for l in &f.loops {
                for &(e, fwd) in l {
                    if let Some(u) = uses.get_mut(e) {
                        u.0 += 1;
                    }
                    if let Some(d) = dir_of.get_mut(e) {
                        *d = fwd;
                    }
                }
            }
        }
        let mut m = Vec::with_capacity(list.edges.len());
        for (ei, e) in list.edges.iter().enumerate() {
            let vv = [vmap[bi][e.vertices[0]], vmap[bi][e.vertices[1]]];
            let rim = uses.get(ei).is_some_and(|u| u.0 == 1);
            let key = (vv[0].min(vv[1]), vv[0].max(vv[1]));
            let mut matched = None;
            if rim && let Some(cands) = rims.get_mut(&key) {
                for (k, &(g, gb)) in cands.iter().enumerate() {
                    let ge = &edges[g];
                    let tm = 0.5 * (e.t0 + e.t1);
                    let gm = 0.5 * (ge.t0 + ge.t1);
                    if !same_point_on(&e.curve, tm, &ge.curve, gm) {
                        continue;
                    }
                    let same = if vv[0] == vv[1] {
                        // A closed edge: compare quarter points to find its direction.
                        let tq = e.t0 + 0.25 * (e.t1 - e.t0);
                        let gq = ge.t0 + 0.25 * (ge.t1 - ge.t0);
                        let gq3 = ge.t0 + 0.75 * (ge.t1 - ge.t0);
                        if same_point_on(&e.curve, tq, &ge.curve, gq) {
                            true
                        } else if same_point_on(&e.curve, tq, &ge.curve, gq3) {
                            false
                        } else {
                            continue;
                        }
                    } else {
                        ge.vertices == vv
                    };
                    matched = Some((k, g, gb, same));
                    break;
                }
            }
            match matched {
                Some((k, g, gb, same)) => {
                    if let Some(c) = rims.get_mut(&key) {
                        c.remove(k);
                    }
                    // This body's use, in the global edge's direction, against the other's.
                    let mine = dir_of.get(ei).copied().unwrap_or(true) == same;
                    let done = emap.get(gb).map(Vec::as_slice).unwrap_or(&[]);
                    let theirs =
                        rim_direction(&lists[gb], done, g, gb == bi, &m).ok_or_else(|| {
                            KernelError::InvalidResult("a shared edge lost its use".into())
                        })?;
                    links.push((bi, gb, mine == theirs));
                    m.push((g, same));
                }
                None => {
                    edges.push(ListEdge {
                        vertices: vv,
                        ..e.clone()
                    });
                    let g = edges.len() - 1;
                    if rim {
                        rims.entry(key).or_default().push((g, bi));
                    }
                    m.push((g, true));
                }
            }
        }
        emap.push(m);
    }
    if links.is_empty() {
        return Err(invalid(
            "the surfaces share no edges (within the tolerance), so there is nothing to stitch",
        ));
    }

    // Which bodies to reverse.
    let mut flip: Vec<Option<bool>> = vec![None; lists.len()];
    for start in 0..lists.len() {
        if flip[start].is_some() {
            continue;
        }
        flip[start] = Some(false);
        let mut queue = vec![start];
        while let Some(b) = queue.pop() {
            let fb = flip[b].unwrap_or(false);
            for &(x, y, differ) in &links {
                let other = if x == b {
                    y
                } else if y == b {
                    x
                } else {
                    continue;
                };
                let want = fb != differ;
                match flip[other] {
                    None => {
                        flip[other] = Some(want);
                        queue.push(other);
                    }
                    Some(f) if f != want => {
                        return Err(KernelError::Unsupported(
                            "the surfaces cannot be given one consistent side (a one-sided, Moebius-like join)".into(),
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    let mut out = FaceList {
        vertices,
        edges,
        faces: Vec::new(),
    };
    for (bi, list) in lists.iter().enumerate() {
        let mut faces: Vec<ListFace> = list
            .faces
            .iter()
            .map(|f| ListFace {
                loops: f
                    .loops
                    .iter()
                    .map(|l| {
                        l.iter()
                            .map(|&(e, fwd)| {
                                let (g, same) = emap[bi].get(e).copied().unwrap_or((0, true));
                                (g, fwd == same)
                            })
                            .collect()
                    })
                    .collect(),
                ..f.clone()
            })
            .collect();
        if flip[bi] == Some(true) {
            let mut tmp = FaceList {
                faces: std::mem::take(&mut faces),
                ..FaceList::default()
            };
            tmp.reverse();
            faces = tmp.faces;
        }
        out.faces.extend(faces);
    }
    match out.assemble() {
        Ok(b) => Ok(b),
        Err(first) => {
            // A closed result inside out: turn it outward.
            out.reverse();
            out.assemble().map_err(|_| first)
        }
    }
}

/// The direction (relative to global edge `g`) in which body `gb` uses `g`, from the
/// mapping of that body's edges (`current` while it is still being built).
fn rim_direction(
    list: &FaceList,
    done: &[(usize, bool)],
    g: usize,
    same_body: bool,
    current: &[(usize, bool)],
) -> Option<bool> {
    let map = if same_body { current } else { done };
    let (ei, &(_, same)) = map.iter().enumerate().find(|(_, (x, _))| *x == g)?;
    for f in &list.faces {
        for l in &f.loops {
            for &(e, fwd) in l {
                if e == ei {
                    return Some(fwd == same);
                }
            }
        }
    }
    None
}

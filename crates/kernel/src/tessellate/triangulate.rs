//! Triangulation of a polygon with holes: hole bridging, then ear clipping, with every
//! decision taken by the exact `orient2d` predicate.
//!
//! The method is the classic one (Meisters' two-ears theorem; de Berg et al.,
//! *Computational Geometry*, chapter 3; D. Eberly, "Triangulation by Ear Clipping"),
//! written here from those descriptions:
//!
//! 1. **Bridging.** Holes are taken in order of decreasing largest x. For a hole's
//!    rightmost vertex `H`, the polygon built so far (outer boundary plus the holes
//!    already joined) has a vertex `P` with `P.x > H.x` that `H` sees: the ray from `H`
//!    in +x leaves the hole at once (H is its rightmost point), meets no hole still
//!    waiting (they all lie at `x ≤ H.x`), and so first hits the polygon; inside the
//!    triangle of `H`, the hit point and the hit edge's right end, the vertex nearest the
//!    ray is visible (Eberly, section 3). Instead of intersecting the ray in floating
//!    point, candidates `P` are tried nearest first and accepted by an exact test: the
//!    segment `HP` must leave `H` into the face (outside the hole) and enter the face at
//!    `P` (inside the angle of that use of `P`), and touch no edge of the polygon or of
//!    any hole except at its own ends. The hole is spliced in as `… P H h1 … hk H P …`,
//!    a weakly simple polygon in which `H` and `P` appear twice.
//! 2. **Ear clipping.** A vertex `b` with neighbours `a`, `c` is an ear when `a b c`
//!    turns strictly left and no other vertex lies in the closed triangle, except copies
//!    of `a`, `b`, `c` (from bridges), which may touch it only if none of their edges
//!    points into the triangle's angle there (or along the new diagonal `ca`). Then `ac`
//!    is a diagonal, the ear is cut off, and the walk goes on from `a`. A uniform grid of
//!    the vertices keeps the emptiness test local.
//!
//! Robustness: no decision uses a computed intersection point or an epsilon. Collinear
//! vertices are never clipped as zero-area ears (they would leave a T-junction against
//! the neighbouring face), slivers are accepted as long as they turn strictly left, and
//! holes touching nothing, many holes, and holes with equal x are handled by the exact
//! tests. If floating-point projection ever leaves a polygon with no valid ear (it would
//! have to be no longer simple), the walk still finishes by clipping the first convex
//! vertex, so the mesh stays closed; [`Triangulation::forced`] counts such clips.

use crate::{KernelError, KernelResult};
use chamfera_geom::{Point2, Sign, orient2d};

/// The result of [`triangulate`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Triangulation {
    /// Counter-clockwise triangles as indices into the input points.
    pub triangles: Vec<[u32; 3]>,
    /// Ears clipped without passing the ear test (zero for every simple polygon).
    #[cfg_attr(not(test), allow(dead_code))]
    pub forced: usize,
}

/// Exact orientation; the caller has checked the coordinates are usable, so the `None`
/// of out-of-range input cannot happen and reads as collinear.
fn orient(a: Point2, b: Point2, c: Point2) -> Sign {
    orient2d(a, b, c).unwrap_or(Sign::Zero)
}

fn same(a: Point2, b: Point2) -> bool {
    a.x == b.x && a.y == b.y
}

/// Twice the signed area of a ring of points (shoelace), positive counter-clockwise.
pub(super) fn twice_signed_area(pts: &[Point2], ring: &[u32]) -> f64 {
    let n = ring.len();
    let at = |i: usize| {
        ring.get(i % n)
            .and_then(|&k| pts.get(k as usize))
            .copied()
            .unwrap_or_default()
    };
    let Some(&first) = ring.first().and_then(|&k| pts.get(k as usize)) else {
        return 0.0;
    };
    // Relative to the first point to keep the sum accurate far from the origin.
    (0..n)
        .map(|i| (at(i) - first).cross(at(i + 1) - first))
        .sum()
}

/// Whether the segment from `p` towards `q` starts strictly inside the angle of the
/// polygon at `p`, whose previous and next vertices are `a` and `b` (the polygon's
/// interior is on the left of `a → p → b`).
fn enters_wedge(a: Point2, p: Point2, b: Point2, q: Point2) -> bool {
    let ob = orient(p, b, q);
    let oa = orient(p, a, q);
    match orient(a, p, b) {
        Sign::Positive => ob == Sign::Positive && oa == Sign::Negative,
        Sign::Negative => !(oa != Sign::Negative && ob != Sign::Positive),
        Sign::Zero => {
            if (a - p).dot(b - p) < 0.0 {
                // Straight through: the interior is the left half-plane.
                ob == Sign::Positive
            } else {
                // A spike (`a` and `b` in the same direction): everything but its ray.
                !(ob == Sign::Zero && (q - p).dot(b - p) > 0.0)
            }
        }
    }
}

/// Whether closed segments `ab` and `cd` share a point (Cormen et al., section 33.1,
/// with exact signs).
fn segments_touch(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let within = |a: Point2, b: Point2, c: Point2| {
        c.x >= a.x.min(b.x) && c.x <= a.x.max(b.x) && c.y >= a.y.min(b.y) && c.y <= a.y.max(b.y)
    };
    let (o1, o2) = (orient(a, b, c), orient(a, b, d));
    let (o3, o4) = (orient(c, d, a), orient(c, d, b));
    let opposite = |x: Sign, y: Sign| {
        matches!(
            (x, y),
            (Sign::Positive, Sign::Negative) | (Sign::Negative, Sign::Positive)
        )
    };
    (opposite(o1, o2) && opposite(o3, o4))
        || (o1 == Sign::Zero && within(a, b, c))
        || (o2 == Sign::Zero && within(a, b, d))
        || (o3 == Sign::Zero && within(c, d, a))
        || (o4 == Sign::Zero && within(c, d, b))
}

/// Whether polygon edge `ab` blocks the bridge `hp`: they share a point other than an
/// end of the bridge that is also an end of the edge (where they may meet, unless they
/// run along each other).
fn blocks(h: Point2, p: Point2, a: Point2, b: Point2) -> bool {
    if !segments_touch(h, p, a, b) {
        return false;
    }
    for (end, toward) in [(h, p), (p, h)] {
        let other = if same(a, end) {
            b
        } else if same(b, end) {
            a
        } else {
            continue;
        };
        if orient(h, p, other) != Sign::Zero {
            // Not collinear: two segments with a common end meet only there.
            return false;
        }
        // Collinear: they overlap if the edge runs from `end` along the bridge.
        return (other - end).dot(toward - end) > 0.0;
    }
    true
}

/// Joins the holes to the outer ring by bridges (step 1 of the module documentation).
fn bridge_holes(pts: &[Point2], outer: &[u32], holes: &[Vec<u32>]) -> KernelResult<Vec<u32>> {
    let pt = |k: u32| pts.get(k as usize).copied().unwrap_or_default();
    // Each hole rotated to start at its rightmost vertex (largest x, then largest y).
    let mut pending: Vec<Vec<u32>> = Vec::with_capacity(holes.len());
    for hole in holes {
        let Some(start) = (0..hole.len()).max_by(|&i, &j| {
            let (p, q) = (pt(hole[i]), pt(hole[j]));
            p.x.total_cmp(&q.x).then(p.y.total_cmp(&q.y))
        }) else {
            continue;
        };
        let mut h = hole[start..].to_vec();
        h.extend_from_slice(&hole[..start]);
        pending.push(h);
    }
    pending.sort_by(|a, b| {
        let (pa, pb) = (pt(a[0]), pt(b[0]));
        pb.x.total_cmp(&pa.x).then(pb.y.total_cmp(&pa.y))
    });

    let mut merged: Vec<u32> = outer.to_vec();
    for (hi, hole) in pending.iter().enumerate() {
        let hp = pt(hole[0]);
        let hm = hole.len();
        let (h_prev, h_next) = (pt(hole[hm - 1]), pt(hole[1 % hm]));
        let n = merged.len();
        let ring_at = |i: usize| pt(merged[i % n]);
        let visible = |k: usize| -> bool {
            let p = ring_at(k);
            if !enters_wedge(h_prev, hp, h_next, p) {
                return false;
            }
            if !enters_wedge(ring_at(k + n - 1), p, ring_at(k + 1), hp) {
                return false;
            }
            if (0..n).any(|i| blocks(hp, p, ring_at(i), ring_at(i + 1))) {
                return false;
            }
            // This hole and the ones still waiting (joined holes are in `merged`).
            !pending[hi..].iter().any(|w| {
                let m = w.len();
                (0..m).any(|i| blocks(hp, p, pt(w[i]), pt(w[(i + 1) % m])))
            })
        };
        let dist = |k: usize| {
            let d = ring_at(k) - hp;
            d.dot(d)
        };
        // Right of H first (where a visible vertex always exists), then the rest.
        let mut right: Vec<usize> = (0..n).filter(|&k| ring_at(k).x > hp.x).collect();
        right.sort_by(|&i, &j| dist(i).total_cmp(&dist(j)));
        let mut found = right.iter().copied().find(|&k| visible(k));
        if found.is_none() {
            let mut rest: Vec<usize> = (0..n).filter(|&k| ring_at(k).x <= hp.x).collect();
            rest.sort_by(|&i, &j| dist(i).total_cmp(&dist(j)));
            found = rest.into_iter().find(|&k| visible(k));
        }
        let Some(k) = found else {
            return Err(KernelError::InvalidResult(
                "a hole of a face could not be joined to its boundary; the face's loops overlap"
                    .into(),
            ));
        };
        // … P H h1 … hk H P …
        let mut spliced = Vec::with_capacity(n + hm + 2);
        spliced.extend_from_slice(&merged[..=k]);
        spliced.extend_from_slice(hole);
        spliced.push(hole[0]);
        spliced.push(merged[k]);
        spliced.extend_from_slice(&merged[k + 1..]);
        merged = spliced;
    }
    Ok(merged)
}

/// A uniform grid of ring nodes by position, for local emptiness tests.
struct Grid {
    min: Point2,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(points: &[Point2]) -> Grid {
        let mut min = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut max = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in points {
            min = Point2::new(min.x.min(p.x), min.y.min(p.y));
            max = Point2::new(max.x.max(p.x), max.y.max(p.y));
        }
        let (w, h) = ((max.x - min.x).max(0.0), (max.y - min.y).max(0.0));
        // About one node per cell.
        let n = points.len().max(1) as f64;
        let cell = ((w * h) / n)
            .sqrt()
            .max(w.max(h) / n.sqrt().max(1.0) / 64.0);
        let cell = if cell.is_finite() && cell > 0.0 {
            cell
        } else {
            1.0
        };
        let count = |len: f64, cell: f64| ((len / cell).floor() as usize).saturating_add(1);
        // At most a few cells per node, so a thin polygon cannot ask for a huge grid.
        let most = 4 * points.len() + 16;
        let mut cell = cell;
        while count(w, cell).saturating_mul(count(h, cell)) > most {
            cell *= 2.0;
        }
        let (nx, ny) = (count(w, cell), count(h, cell));
        let mut grid = Grid {
            min,
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
        };
        for (i, p) in points.iter().enumerate() {
            let (cx, cy) = grid.cell_of(*p);
            if let Some(c) = grid.cells.get_mut(cy * nx + cx) {
                c.push(i as u32);
            }
        }
        grid
    }

    fn cell_of(&self, p: Point2) -> (usize, usize) {
        let f = |v: f64, lo: f64, n: usize| {
            let c = ((v - lo) / self.cell).floor();
            if c.is_finite() && c > 0.0 {
                (c as usize).min(n - 1)
            } else {
                0
            }
        };
        (f(p.x, self.min.x, self.nx), f(p.y, self.min.y, self.ny))
    }

    /// Calls `visit` with every node in the cells overlapping the box of `a`, `b`, `c`;
    /// stops early when it returns true, and says whether it did.
    fn any_in_box(
        &self,
        a: Point2,
        b: Point2,
        c: Point2,
        mut visit: impl FnMut(u32) -> bool,
    ) -> bool {
        let lo = Point2::new(a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y));
        let hi = Point2::new(a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y));
        let (x0, y0) = self.cell_of(lo);
        let (x1, y1) = self.cell_of(hi);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if let Some(cell) = self.cells.get(cy * self.nx + cx)
                    && cell.iter().any(|&i| visit(i))
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Triangulates the region inside `outer` (counter-clockwise) and outside every ring of
/// `holes` (clockwise), all as indices into `pts`. The rings must bound a region in the
/// sense of a valid face: simple, holes inside the outer ring and apart from each other
/// (they may share points with each other only as copies, like bridges). For a ring
/// total of `m` points and `h` holes the result has exactly `m + 2h - 2` triangles.
pub(super) fn triangulate(
    pts: &[Point2],
    outer: &[u32],
    holes: &[Vec<u32>],
) -> KernelResult<Triangulation> {
    let rings = std::iter::once(outer).chain(holes.iter().map(Vec::as_slice));
    for ring in rings {
        if ring.len() < 3 {
            return Err(KernelError::InvalidInput(
                "a face loop needs at least three points to enclose an area".into(),
            ));
        }
        if ring.iter().any(|&k| pts.get(k as usize).is_none()) {
            return Err(KernelError::InvalidResult(
                "a face loop refers to a point that does not exist".into(),
            ));
        }
    }
    let ring = bridge_holes(pts, outer, holes)?;
    Ok(clip_ears(pts, &ring))
}

/// Ear clipping of a weakly simple counter-clockwise ring (step 2 of the module
/// documentation).
fn clip_ears(pts: &[Point2], ring: &[u32]) -> Triangulation {
    let m = ring.len();
    let pos: Vec<Point2> = ring
        .iter()
        .map(|&k| pts.get(k as usize).copied().unwrap_or_default())
        .collect();
    let mut prev: Vec<usize> = (0..m).map(|i| (i + m - 1) % m).collect();
    let mut next: Vec<usize> = (0..m).map(|i| (i + 1) % m).collect();
    let mut alive = vec![true; m];
    let grid = Grid::new(&pos);
    let mut out = Triangulation {
        triangles: Vec::with_capacity(m.saturating_sub(2)),
        forced: 0,
    };

    let is_ear = |b: usize, prev: &[usize], next: &[usize], alive: &[bool]| -> bool {
        let (a, c) = (prev[b], next[b]);
        let (pa, pb, pc) = (pos[a], pos[b], pos[c]);
        if orient(pa, pb, pc) != Sign::Positive {
            return false;
        }
        // Does polygon edge from corner `k` (position `pk`) toward `q` enter the ear?
        let enters = |pk: Point2, q: Point2| -> bool {
            if same(pk, pa) {
                (orient(pa, pb, q) == Sign::Positive && orient(pa, pc, q) == Sign::Negative)
                    || (orient(pa, pc, q) == Sign::Zero && (q - pa).dot(pc - pa) > 0.0)
            } else if same(pk, pb) {
                orient(pb, pc, q) == Sign::Positive && orient(pb, pa, q) == Sign::Negative
            } else {
                (orient(pc, pa, q) == Sign::Positive && orient(pc, pb, q) == Sign::Negative)
                    || (orient(pc, pa, q) == Sign::Zero && (q - pc).dot(pa - pc) > 0.0)
            }
        };
        let blocked = grid.any_in_box(pa, pb, pc, |j| {
            let j = j as usize;
            if j == a || j == b || j == c || !alive.get(j).copied().unwrap_or(false) {
                return false;
            }
            let p = pos[j];
            if same(p, pa) || same(p, pb) || same(p, pc) {
                enters(p, pos[prev[j]]) || enters(p, pos[next[j]])
            } else {
                orient(pa, pb, p) != Sign::Negative
                    && orient(pb, pc, p) != Sign::Negative
                    && orient(pc, pa, p) != Sign::Negative
            }
        });
        !blocked
    };

    let mut remaining = m;
    let mut cur = 0usize;
    let mut misses = 0usize;
    while remaining > 3 {
        let b = if is_ear(cur, &prev, &next, &alive) {
            cur
        } else {
            misses += 1;
            if misses <= remaining {
                cur = next[cur];
                continue;
            }
            // No ear in a whole round: force the first convex vertex (or any).
            out.forced += 1;
            let mut pick = cur;
            let mut k = cur;
            for _ in 0..remaining {
                if orient(pos[prev[k]], pos[k], pos[next[k]]) == Sign::Positive {
                    pick = k;
                    break;
                }
                k = next[k];
            }
            pick
        };
        let (a, c) = (prev[b], next[b]);
        out.triangles.push([ring[a], ring[b], ring[c]]);
        next[a] = c;
        prev[c] = a;
        alive[b] = false;
        remaining -= 1;
        cur = a;
        misses = 0;
    }
    if remaining == 3 {
        let (a, c) = (prev[cur], next[cur]);
        out.triangles.push([ring[a], ring[cur], ring[c]]);
    }
    out
}

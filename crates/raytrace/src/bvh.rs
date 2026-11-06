//! A bounding volume hierarchy over triangles, built with the surface area heuristic over
//! binned centroids (MacDonald and Booth 1990; binning as in Wald 2007, "On fast
//! Construction of SAH-based Bounding Volume Hierarchies"), and the ray-triangle test of
//! Möller and Trumbore (1997).

use crate::v3::V3;

/// One triangle: a corner, two edges, the shading normal at each corner and its material.
#[derive(Debug, Clone, Copy)]
pub struct Tri {
    pub a: V3,
    pub e1: V3,
    pub e2: V3,
    pub n: [V3; 3],
    /// Unit geometric normal (from the winding).
    pub ng: V3,
    pub material: u32,
}

impl Tri {
    pub fn new(a: V3, b: V3, c: V3, n: [V3; 3], material: u32) -> Option<Tri> {
        let (e1, e2) = (b - a, c - a);
        let ng = e1.cross(e2).normalized()?;
        Some(Tri {
            a,
            e1,
            e2,
            n,
            ng,
            material,
        })
    }

    fn bounds(&self) -> (V3, V3) {
        let (b, c) = (self.a + self.e1, self.a + self.e2);
        (self.a.min(b).min(c), self.a.max(b).max(c))
    }

    fn centroid(&self) -> V3 {
        self.a + (self.e1 + self.e2) * (1.0 / 3.0)
    }

    /// Distance along the ray and the barycentric `(u, v)` of the hit, if it is within
    /// `(t_min, t_max)`. Both sides count.
    pub fn hit(&self, o: V3, d: V3, t_min: f64, t_max: f64) -> Option<(f64, f64, f64)> {
        let p = d.cross(self.e2);
        let det = self.e1.dot(p);
        if det.abs() < 1e-300 {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - self.a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(self.e1);
        let v = d.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = self.e2.dot(q) * inv;
        (t > t_min && t < t_max).then_some((t, u, v))
    }

    /// The interpolated shading normal at `(u, v)`, or the geometric one.
    pub fn shading_normal(&self, u: f64, v: f64) -> V3 {
        let [n0, n1, n2] = self.n;
        (n0 * (1.0 - u - v) + n1 * u + n2 * v)
            .normalized()
            .unwrap_or(self.ng)
    }
}

#[derive(Debug, Clone, Copy)]
struct Node {
    min: V3,
    max: V3,
    /// A leaf's first triangle, or an inner node's second child (the first follows it).
    index: u32,
    /// Triangles in a leaf; 0 for an inner node.
    count: u32,
}

/// Leaves hold at most this many triangles (unless the split cannot separate them).
const LEAF: usize = 4;
const BINS: usize = 12;
/// Deepest the tree goes; traversal's stack is sized from it.
const MAX_DEPTH: usize = 60;

#[derive(Debug, Clone, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    pub tris: Vec<Tri>,
}

/// What a ray hit.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub t: f64,
    pub tri: usize,
    pub u: f64,
    pub v: f64,
}

fn area(min: V3, max: V3) -> f64 {
    let d = max - min;
    if !(d.x >= 0.0 && d.y >= 0.0 && d.z >= 0.0) {
        return 0.0;
    }
    2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
}

impl Bvh {
    pub fn build(tris: Vec<Tri>) -> Bvh {
        let mut bvh = Bvh {
            nodes: Vec::with_capacity(tris.len().saturating_mul(2).max(1)),
            tris,
        };
        if bvh.tris.is_empty() {
            return bvh;
        }
        let n = bvh.tris.len();
        bvh.nodes.push(Node {
            min: V3::ZERO,
            max: V3::ZERO,
            index: 0,
            count: 0,
        });
        bvh.split(0, 0, n, 0);
        bvh
    }

    fn bounds_of(&self, first: usize, end: usize) -> (V3, V3, V3, V3) {
        let inf = V3::splat(f64::INFINITY);
        let (mut lo, mut hi, mut clo, mut chi) = (inf, -inf, inf, -inf);
        for t in self.tris.get(first..end).unwrap_or(&[]) {
            let (a, b) = t.bounds();
            lo = lo.min(a);
            hi = hi.max(b);
            let c = t.centroid();
            clo = clo.min(c);
            chi = chi.max(c);
        }
        (lo, hi, clo, chi)
    }

    fn make_leaf(&mut self, node: usize, first: usize, end: usize) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.index = first as u32;
            n.count = (end - first) as u32;
        }
    }

    fn split(&mut self, node: usize, first: usize, end: usize, depth: usize) {
        let (lo, hi, clo, chi) = self.bounds_of(first, end);
        if let Some(n) = self.nodes.get_mut(node) {
            n.min = lo;
            n.max = hi;
        }
        let count = end - first;
        if count <= LEAF || depth >= MAX_DEPTH {
            return self.make_leaf(node, first, end);
        }
        // The best binned SAH split over the three axes.
        let mut best: Option<(f64, usize, f64)> = None;
        let extent = chi - clo;
        for axis in 0..3 {
            let (c0, size) = (clo.axis(axis), extent.axis(axis));
            if size.is_nan() || size <= 1e-300 {
                continue;
            }
            let inf = V3::splat(f64::INFINITY);
            let mut bins = [(0usize, inf, -inf); BINS];
            let k = BINS as f64 / size;
            for t in self.tris.get(first..end).unwrap_or(&[]) {
                let b = (((t.centroid().axis(axis) - c0) * k) as usize).min(BINS - 1);
                let (a, z) = t.bounds();
                if let Some(bin) = bins.get_mut(b) {
                    bin.0 += 1;
                    bin.1 = bin.1.min(a);
                    bin.2 = bin.2.max(z);
                }
            }
            // Sweep from the left and from the right.
            let mut right_cost = [0.0; BINS];
            let (mut n, mut rlo, mut rhi) = (0usize, inf, -inf);
            for i in (1..BINS).rev() {
                let (c, a, z) = bins[i];
                n += c;
                rlo = rlo.min(a);
                rhi = rhi.max(z);
                right_cost[i] = n as f64 * area(rlo, rhi);
            }
            let (mut n, mut llo, mut lhi) = (0usize, inf, -inf);
            for i in 0..BINS - 1 {
                let (c, a, z) = bins[i];
                n += c;
                llo = llo.min(a);
                lhi = lhi.max(z);
                if n == 0 || n == count {
                    continue;
                }
                let cost = n as f64 * area(llo, lhi) + right_cost[i + 1];
                if best.is_none_or(|(b, _, _)| cost < b) {
                    best = Some((cost, axis, c0 + size * (i + 1) as f64 / BINS as f64));
                }
            }
        }
        let leaf_cost = count as f64 * area(lo, hi);
        let Some((cost, axis, at)) = best else {
            return self.make_leaf(node, first, end);
        };
        if cost >= leaf_cost && count <= 16 {
            return self.make_leaf(node, first, end);
        }
        // Partition in place.
        let mut mid = first;
        for i in first..end {
            if self
                .tris
                .get(i)
                .is_some_and(|t| t.centroid().axis(axis) < at)
            {
                self.tris.swap(i, mid);
                mid += 1;
            }
        }
        if mid == first || mid == end {
            return self.make_leaf(node, first, end);
        }
        let blank = Node {
            min: V3::ZERO,
            max: V3::ZERO,
            index: 0,
            count: 0,
        };
        let left = self.nodes.len();
        self.nodes.push(blank);
        self.split(left, first, mid, depth + 1);
        let right = self.nodes.len();
        self.nodes.push(blank);
        self.split(right, mid, end, depth + 1);
        if let Some(n) = self.nodes.get_mut(node) {
            n.index = right as u32;
            n.count = 0;
        }
    }

    /// Entry distance of the ray into a node's box, if it enters before `t_max`.
    fn enter(n: &Node, o: V3, inv: V3, t_max: f64) -> Option<f64> {
        let mut t0: f64 = 0.0;
        let mut t1 = t_max;
        for axis in 0..3 {
            let i = inv.axis(axis);
            let (a, b) = (
                (n.min.axis(axis) - o.axis(axis)) * i,
                (n.max.axis(axis) - o.axis(axis)) * i,
            );
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            // NaN (0 * inf on a slab boundary) leaves the interval as it is.
            if a > t0 {
                t0 = a;
            }
            if b < t1 {
                t1 = b;
            }
            if t0 > t1 {
                return None;
            }
        }
        Some(t0)
    }

    /// The nearest hit between `t_min` and `t_max`, or with `any`, the first found.
    fn traverse(&self, o: V3, d: V3, t_min: f64, t_max: f64, any: bool) -> Option<Hit> {
        let root = self.nodes.first()?;
        let inv = V3 {
            x: 1.0 / d.x,
            y: 1.0 / d.y,
            z: 1.0 / d.z,
        };
        let mut best: Option<Hit> = None;
        let mut reach = t_max;
        Self::enter(root, o, inv, reach)?;
        let mut stack = [0u32; MAX_DEPTH * 2 + 4];
        let mut top = 1usize;
        while top > 0 {
            top -= 1;
            let here = stack.get(top).copied().unwrap_or(0) as usize;
            let Some(n) = self.nodes.get(here) else {
                continue;
            };
            if Self::enter(n, o, inv, reach).is_none() {
                continue;
            }
            if n.count > 0 {
                let first = n.index as usize;
                for i in first..first + n.count as usize {
                    let Some(t) = self.tris.get(i) else { continue };
                    if let Some((t, u, v)) = t.hit(o, d, t_min, reach) {
                        reach = t;
                        best = Some(Hit { t, tri: i, u, v });
                        if any {
                            return best;
                        }
                    }
                }
                continue;
            }
            // The first child follows its parent; the second is at `index`. The nearer
            // one is pushed last, so it is visited first.
            let (a, b) = (here + 1, n.index as usize);
            let da = self
                .nodes
                .get(a)
                .and_then(|x| Self::enter(x, o, inv, reach));
            let db = self
                .nodes
                .get(b)
                .and_then(|x| Self::enter(x, o, inv, reach));
            let order = match (da, db) {
                (Some(x), Some(y)) if x <= y => [Some(b), Some(a)],
                (Some(_), Some(_)) => [Some(a), Some(b)],
                (Some(_), None) => [Some(a), None],
                (None, Some(_)) => [Some(b), None],
                (None, None) => [None, None],
            };
            for i in order.into_iter().flatten() {
                if let Some(slot) = stack.get_mut(top) {
                    *slot = i as u32;
                    top += 1;
                }
            }
        }
        best
    }

    pub fn intersect(&self, o: V3, d: V3, t_min: f64, t_max: f64) -> Option<Hit> {
        self.traverse(o, d, t_min, t_max, false)
    }

    pub fn occluded(&self, o: V3, d: V3, t_min: f64, t_max: f64) -> bool {
        self.traverse(o, d, t_min, t_max, true).is_some()
    }

    /// The nearest hit by testing every triangle (the reference for tests).
    pub fn brute_force(&self, o: V3, d: V3, t_min: f64, t_max: f64) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for (i, t) in self.tris.iter().enumerate() {
            let reach = best.map_or(t_max, |b| b.t);
            if let Some((t, u, v)) = t.hit(o, d, t_min, reach) {
                best = Some(Hit { t, tri: i, u, v });
            }
        }
        best
    }
}

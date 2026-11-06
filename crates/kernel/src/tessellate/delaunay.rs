//! Improving and refining a triangulation in the (u, v) domain.
//!
//! - **Delaunay flips** (Lawson's algorithm; de Berg et al., *Computational Geometry*,
//!   section 9.2): an interior edge `pq` shared by `pqr` and `qps` is flipped to `rs`
//!   when `s` lies strictly inside the circle through `p, q, r` (exact `incircle`) and
//!   the quadrilateral is strictly convex (exact `orient2d`). Boundary edges have one
//!   triangle and are never flipped, so the boundary (the shared edge points) is kept:
//!   the result is a constrained Delaunay triangulation of the same points. Each flip
//!   strictly raises the lifted surface (de Berg, Lemma 9.4 argument), so flipping ends;
//!   a flip budget bounds it anyway.
//! - **Refinement** for curved surfaces: a triangle whose centroid or interior-edge
//!   midpoints are further than the chord tolerance from the surface, or whose normal
//!   is further than the angle tolerance from the surface normal, gets its (u, v)
//!   centroid inserted (it splits into three, then flips restore the Delaunay property).
//!   A centroid is strictly inside its triangle, so boundary edges are never split and
//!   the mesh stays watertight against the neighbouring faces.

use super::domain::SurfaceDomain;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Point2, Point3, Sign, Vec3, incircle, orient2d};
use std::collections::HashMap;

/// Rounds of refinement before giving up (each round splits every triangle still too
/// far from the surface, so the error falls geometrically and a few rounds suffice).
const MAX_REFINE_ROUNDS: usize = 48;

/// A triangulation of points in the (u, v) domain, with their points in space.
pub(super) struct Mesh2 {
    pub uv: Vec<Point2>,
    pub xyz: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Directed edge to the triangle that has it. Edges two triangles claim in the same
    /// direction (only possible after a forced ear) are frozen.
    edges: HashMap<(u32, u32), usize>,
}

fn orient(a: Point2, b: Point2, c: Point2) -> Sign {
    orient2d(a, b, c).unwrap_or(Sign::Zero)
}

impl Mesh2 {
    pub fn new(uv: Vec<Point2>, xyz: Vec<Point3>, triangles: Vec<[u32; 3]>) -> Mesh2 {
        let mut mesh = Mesh2 {
            uv,
            xyz,
            triangles,
            edges: HashMap::new(),
        };
        let mut frozen = Vec::new();
        for (t, tri) in mesh.triangles.iter().enumerate() {
            for (a, b) in Self::sides(*tri) {
                if mesh.edges.insert((a, b), t).is_some() {
                    frozen.push((a, b));
                }
            }
        }
        for e in frozen {
            mesh.edges.remove(&e);
            mesh.edges.remove(&(e.1, e.0));
        }
        mesh
    }

    fn sides([a, b, c]: [u32; 3]) -> [(u32, u32); 3] {
        [(a, b), (b, c), (c, a)]
    }

    fn p(&self, i: u32) -> Point2 {
        self.uv.get(i as usize).copied().unwrap_or_default()
    }

    fn x(&self, i: u32) -> Point3 {
        self.xyz.get(i as usize).copied().unwrap_or(Point3::ORIGIN)
    }

    /// The corner of triangle `t` opposite its directed side `(a, b)`.
    fn apex(&self, t: usize, a: u32, b: u32) -> Option<u32> {
        let tri = self.triangles.get(t)?;
        tri.iter().copied().find(|&v| v != a && v != b)
    }

    /// Flips `(p, q)` if it is an interior, non-Delaunay edge of a convex quadrilateral.
    /// Returns the four outer sides to re-check after a flip.
    fn try_flip(&mut self, p: u32, q: u32) -> Option<[(u32, u32); 4]> {
        let t1 = *self.edges.get(&(p, q))?;
        let t2 = *self.edges.get(&(q, p))?;
        if t1 == t2 {
            return None;
        }
        let r = self.apex(t1, p, q)?;
        let s = self.apex(t2, q, p)?;
        let (pp, pq, pr, ps) = (self.p(p), self.p(q), self.p(r), self.p(s));
        if incircle(pp, pq, pr, ps) != Some(Sign::Positive) {
            return None;
        }
        if orient(pp, ps, pr) != Sign::Positive || orient(ps, pq, pr) != Sign::Positive {
            return None;
        }
        // A new side already in use (can happen only around frozen edges): leave it.
        if self.edges.contains_key(&(s, r)) || self.edges.contains_key(&(r, s)) {
            return None;
        }
        // pqr, qps -> psr, sqr
        *self.triangles.get_mut(t1)? = [p, s, r];
        *self.triangles.get_mut(t2)? = [s, q, r];
        self.edges.remove(&(p, q));
        self.edges.remove(&(q, p));
        self.edges.insert((p, s), t1);
        self.edges.insert((s, r), t1);
        self.edges.insert((r, p), t1);
        self.edges.insert((s, q), t2);
        self.edges.insert((q, r), t2);
        self.edges.insert((r, s), t2);
        Some([(p, s), (s, q), (q, r), (r, p)])
    }

    /// Flips until the given sides (and everything a flip exposes) are locally
    /// Delaunay, or the flip budget is spent.
    fn legalize(&mut self, mut stack: Vec<(u32, u32)>, budget: &mut usize) {
        while let Some((a, b)) = stack.pop() {
            if *budget == 0 {
                return;
            }
            if let Some(more) = self.try_flip(a, b) {
                *budget -= 1;
                stack.extend(more);
            }
        }
    }

    /// Makes the whole triangulation (constrained) Delaunay.
    pub fn make_delaunay(&mut self) {
        let stack: Vec<(u32, u32)> = self
            .triangles
            .iter()
            .flat_map(|t| Self::sides(*t))
            .collect();
        let mut budget = 64 * self.triangles.len() + 1024;
        self.legalize(stack, &mut budget);
    }

    /// Inserts the centroid of triangle `t`, splitting it into three.
    fn split(&mut self, t: usize, domain: &dyn SurfaceDomain, budget: &mut usize) -> Option<()> {
        let [a, b, c] = *self.triangles.get(t)?;
        let (pa, pb, pc) = (self.p(a), self.p(b), self.p(c));
        let uv = Point2::new((pa.x + pb.x + pc.x) / 3.0, (pa.y + pb.y + pc.y) / 3.0);
        // Rounding could put the centroid of a needle on a side; skip such a triangle.
        if orient(pa, pb, uv) != Sign::Positive
            || orient(pb, pc, uv) != Sign::Positive
            || orient(pc, pa, uv) != Sign::Positive
        {
            return None;
        }
        let m = u32::try_from(self.uv.len()).ok()?;
        self.uv.push(uv);
        self.xyz.push(domain.surface_point(uv));
        let t2 = self.triangles.len();
        let t3 = t2 + 1;
        *self.triangles.get_mut(t)? = [a, b, m];
        self.triangles.push([b, c, m]);
        self.triangles.push([c, a, m]);
        self.edges.insert((a, b), t);
        self.edges.insert((b, m), t);
        self.edges.insert((m, a), t);
        self.edges.insert((b, c), t2);
        self.edges.insert((c, m), t2);
        self.edges.insert((m, b), t2);
        self.edges.insert((c, a), t3);
        self.edges.insert((a, m), t3);
        self.edges.insert((m, c), t3);
        self.legalize(vec![(a, b), (b, c), (c, a)], budget);
        Some(())
    }

    /// Whether triangle `t` is further from the surface than the tolerances allow.
    fn too_far(&self, t: usize, domain: &dyn SurfaceDomain, tol: f64, angle: f64) -> bool {
        let Some(&[a, b, c]) = self.triangles.get(t) else {
            return false;
        };
        let (xa, xb, xc) = (self.x(a), self.x(b), self.x(c));
        let (pa, pb, pc) = (self.p(a), self.p(b), self.p(c));
        let mean = |p: &[Point3]| {
            let n = p.len() as f64;
            let s = p.iter().fold(Vec3::ZERO, |s, q| s + (*q - Point3::ORIGIN));
            Point3::ORIGIN + s * (1.0 / n)
        };
        let centroid_uv = Point2::new((pa.x + pb.x + pc.x) / 3.0, (pa.y + pb.y + pc.y) / 3.0);
        if domain
            .surface_point(centroid_uv)
            .distance(mean(&[xa, xb, xc]))
            > tol
        {
            return true;
        }
        for (i, j) in Self::sides([a, b, c]) {
            // Interior sides only: boundary sides are edge chords, already within tolerance.
            if !self.edges.contains_key(&(j, i)) {
                continue;
            }
            let (pi, pj) = (self.p(i), self.p(j));
            let mid = Point2::new((pi.x + pj.x) / 2.0, (pi.y + pj.y) / 2.0);
            if domain
                .surface_point(mid)
                .distance(mean(&[self.x(i), self.x(j)]))
                > tol
            {
                return true;
            }
        }
        match (
            (xb - xa).cross(xc - xa).normalized(),
            domain.surface_normal(centroid_uv).normalized(),
        ) {
            (Some(n), Some(s)) => n.angle_to(s).is_some_and(|g| g > angle),
            _ => false,
        }
    }

    /// Refines until every triangle is within the tolerances of the surface. Fails when
    /// that would need more than `max_triangles` triangles or does not settle.
    pub fn refine(
        &mut self,
        domain: &dyn SurfaceDomain,
        tol: f64,
        angle: f64,
        max_triangles: usize,
    ) -> KernelResult<()> {
        let mut flips = 64 * max_triangles.min(1 << 24) + 1024;
        for _ in 0..MAX_REFINE_ROUNDS {
            let bad: Vec<usize> = (0..self.triangles.len())
                .filter(|&t| self.too_far(t, domain, tol, angle))
                .collect();
            if bad.is_empty() {
                return Ok(());
            }
            if self.triangles.len() + 2 * bad.len() > max_triangles {
                return Err(KernelError::InvalidInput(format!(
                    "a curved face would need more than `max_triangles` = {max_triangles} triangles at this tolerance"
                )));
            }
            let mut progressed = false;
            for t in bad {
                // Flips may have changed `t` since the scan; re-check it.
                if self.too_far(t, domain, tol, angle) {
                    progressed |= self.split(t, domain, &mut flips).is_some();
                }
            }
            if !progressed {
                break;
            }
        }
        Err(KernelError::NoConvergence(
            "a curved face did not come within the tolerances of its surface".into(),
        ))
    }
}

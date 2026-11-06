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
//! - **Refinement** for curved surfaces: a triangle whose centroid or interior-side
//!   midpoints are further than the chord tolerance from the surface, or along one of
//!   whose interior sides the surface normal turns by more than twice the angle
//!   tolerance (the facet's normal lies between its corners', so it stays within about
//!   the angle tolerance of the surface),
//!   gets its (u, v) centroid inserted (it splits into three, then flips restore the
//!   Delaunay property). The normal is compared along sides rather than against the
//!   facet's own normal: a thin triangle on a curved boundary chord tilts with the chord,
//!   and splitting it only makes thinner ones, so that test would never settle.
//!   A centroid is strictly inside its triangle, so boundary edges are never split and
//!   the mesh stays watertight against the neighbouring faces. A triangle with an
//!   interior side longer in u or v than the domain's [`SurfaceDomain::max_span`] is
//!   split too (near a pole the chord test alone would let it fold over), and a
//!   triangle on two copies of one point (a pole run) is left alone: it is a segment in
//!   space and is dropped from the mesh.

use super::domain::SurfaceDomain;
use crate::{KernelError, KernelResult};
use chamfera_geom::{Point2, Point3, Sign, Vec2, Vec3, incircle, orient2d};
use std::collections::HashMap;

/// Rounds of refinement before giving up (each round splits every triangle still too
/// far from the surface, so the error falls geometrically and a few rounds suffice).
const MAX_REFINE_ROUNDS: usize = 48;

/// The least rise of the smaller fit (a cosine) for which [`Mesh2::improve_in_space`]
/// flips an edge: far above rounding, far below any visible change.
const FIT_GAIN: f64 = 1e-9;

/// How to mend a triangle outside the tolerances.
enum Fix {
    /// Insert the midpoint of this interior side.
    Side(u32, u32),
    /// Insert the centroid.
    Centroid,
}

/// Rounds of smoothing and flips in space after the refinement.
const IMPROVE_PASSES: usize = 3;

/// The surface and tolerances that flips judged in space work against.
struct Space<'a> {
    domain: &'a dyn SurfaceDomain,
    tol: f64,
    angle: f64,
}

/// A triangulation of points in the (u, v) domain, with their points in space.
pub(super) struct Mesh2 {
    pub uv: Vec<Point2>,
    pub xyz: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Directed edge to the triangle that has it. Edges two triangles claim in the same
    /// direction (only possible after a forced ear) are frozen.
    edges: HashMap<(u32, u32), usize>,
    /// Points below this index are the boundary's and never move.
    fixed: usize,
}

fn orient(a: Point2, b: Point2, c: Point2) -> Sign {
    orient2d(a, b, c).unwrap_or(Sign::Zero)
}

impl Mesh2 {
    pub fn new(uv: Vec<Point2>, xyz: Vec<Point3>, triangles: Vec<[u32; 3]>) -> Mesh2 {
        let fixed = uv.len();
        let mut mesh = Mesh2 {
            uv,
            xyz,
            triangles,
            edges: HashMap::new(),
            fixed,
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

    /// How well triangle `tri` follows the surface: the cosine of the angle between its
    /// facet normal and the surface normal at its (u, v) centroid (-1 for a triangle
    /// with no area, which is worst).
    fn fit(&self, tri: [u32; 3], domain: &dyn SurfaceDomain) -> f64 {
        let [a, b, c] = tri;
        let (pa, pb, pc) = (self.p(a), self.p(b), self.p(c));
        let centroid = Point2::new((pa.x + pb.x + pc.x) / 3.0, (pa.y + pb.y + pc.y) / 3.0);
        let (xa, xb, xc) = (self.x(a), self.x(b), self.x(c));
        match (
            (xb - xa).cross(xc - xa).normalized(),
            domain.surface_normal(centroid).normalized(),
        ) {
            (Some(n), Some(s)) => n.dot(s),
            _ => -1.0,
        }
    }

    /// Whether a triangle has two corners at one point in space (two copies of a pole).
    fn pinched(&self, [a, b, c]: [u32; 3]) -> bool {
        let (xa, xb, xc) = (self.x(a), self.x(b), self.x(c));
        xa == xb || xb == xc || xc == xa
    }

    /// Flips `(p, q)` if it is an interior edge of a strictly convex quadrilateral and
    /// the flip improves it: without `space`, when the edge is not Delaunay in (u, v);
    /// with it, when the worse of the two triangles follows the surface better after the
    /// flip ([`Mesh2::fit`]) and both new triangles are within the tolerances. Returns
    /// the four outer sides to re-check after a flip.
    fn try_flip(&mut self, p: u32, q: u32, space: Option<&Space<'_>>) -> Option<[(u32, u32); 4]> {
        let t1 = *self.edges.get(&(p, q))?;
        let t2 = *self.edges.get(&(q, p))?;
        if t1 == t2 {
            return None;
        }
        let r = self.apex(t1, p, q)?;
        let s = self.apex(t2, q, p)?;
        let (pp, pq, pr, ps) = (self.p(p), self.p(q), self.p(r), self.p(s));
        match space {
            None => {
                if incircle(pp, pq, pr, ps) != Some(Sign::Positive) {
                    return None;
                }
            }
            Some(&Space { domain, .. }) => {
                let before = [[p, q, r], [q, p, s]];
                let after = [[p, s, r], [s, q, r]];
                // Pole runs stay as they are: their pinched triangles are dropped later.
                if before.iter().chain(&after).any(|t| self.pinched(*t)) {
                    return None;
                }
                let worst = |ts: &[[u32; 3]; 2]| {
                    ts.iter()
                        .map(|t| self.fit(*t, domain))
                        .fold(f64::INFINITY, f64::min)
                };
                // A clear gain only, so flips cannot cycle on rounding.
                if worst(&after) <= worst(&before) + FIT_GAIN {
                    return None;
                }
            }
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
        if let Some(sp) = space
            && (self.too_far(t1, sp.domain, sp.tol, sp.angle)
                || self.too_far(t2, sp.domain, sp.tol, sp.angle))
        {
            // Undo: psr, sqr -> pqr, qps.
            *self.triangles.get_mut(t1)? = [p, q, r];
            *self.triangles.get_mut(t2)? = [q, p, s];
            self.edges.remove(&(s, r));
            self.edges.remove(&(r, s));
            self.edges.insert((p, q), t1);
            self.edges.insert((q, r), t1);
            self.edges.insert((r, p), t1);
            self.edges.insert((q, p), t2);
            self.edges.insert((p, s), t2);
            self.edges.insert((s, q), t2);
            return None;
        }
        Some([(p, s), (s, q), (q, r), (r, p)])
    }

    /// Flips until the given sides (and everything a flip exposes) are locally
    /// Delaunay, or the flip budget is spent.
    fn legalize(&mut self, mut stack: Vec<(u32, u32)>, budget: &mut usize) {
        while let Some((a, b)) = stack.pop() {
            if *budget == 0 {
                return;
            }
            if let Some(more) = self.try_flip(a, b, None) {
                *budget -= 1;
                stack.extend(more);
            }
        }
    }

    /// Flips interior edges while that makes the worse of their two triangles follow
    /// the surface better ([`Mesh2::fit`]), within a flip budget. The (u, v) Delaunay
    /// criterion is blind to how (u, v) stretches near a pole and to the tilt of a thin
    /// triangle on a curved boundary chord; this pass judges in space. Each flip raises
    /// the smaller fit of its pair by more than [`FIT_GAIN`], and is undone if either new
    /// triangle is outside the tolerances, so the refinement's guarantees hold after it.
    fn improve_in_space(&mut self, space: &Space<'_>) {
        let mut stack: Vec<(u32, u32)> = self
            .triangles
            .iter()
            .flat_map(|t| Self::sides(*t))
            .collect();
        let mut budget = 16 * self.triangles.len() + 1024;
        while let Some((a, b)) = stack.pop() {
            if budget == 0 {
                return;
            }
            if let Some(more) = self.try_flip(a, b, Some(space)) {
                budget -= 1;
                stack.extend(more);
            }
        }
    }

    /// Moves each point the refinement added toward the mean of its neighbours, keeping
    /// a move only if every triangle around the point still turns counter-clockwise in
    /// (u, v), stays within the tolerances, and the worst fit among them rises
    /// (Laplacian smoothing with a quality guard). The mean is taken in space first
    /// (projected onto the surface and back to (u, v) near the point: right near a pole,
    /// where (u, v) is stretched), then in (u, v). Boundary points never move, so the
    /// mesh stays watertight.
    fn smooth(&mut self, space: &Space<'_>) {
        let domain = space.domain;
        let mut around: Vec<Vec<usize>> = vec![Vec::new(); self.uv.len()];
        for (t, tri) in self.triangles.iter().enumerate() {
            for &v in tri {
                if let Some(list) = around.get_mut(v as usize) {
                    list.push(t);
                }
            }
        }
        for v in self.fixed..self.uv.len() {
            let Some(ts) = around.get(v) else { continue };
            let Ok(vi) = u32::try_from(v) else { continue };
            let (mut sum_uv, mut sum_xyz, mut count) = (Vec2::ZERO, Vec3::ZERO, 0.0);
            for &t in ts {
                let Some(tri) = self.triangles.get(t) else {
                    continue;
                };
                for &w in tri {
                    if w != vi {
                        sum_uv = sum_uv + (self.p(w) - Point2::ORIGIN);
                        sum_xyz = sum_xyz + (self.x(w) - Point3::ORIGIN);
                        count += 1.0;
                    }
                }
            }
            if count == 0.0 {
                continue;
            }
            let (old_uv, old_xyz) = (self.p(vi), self.x(vi));
            let in_space = {
                let q = domain.surface_uv(Point3::ORIGIN + sum_xyz * (1.0 / count));
                Point2::new(
                    super::domain::nearest(old_uv.x, q.x, domain.u_period()),
                    super::domain::nearest(old_uv.y, q.y, domain.v_period()),
                )
            };
            let in_uv = Point2::ORIGIN + sum_uv * (1.0 / count);
            let worst = |m: &Mesh2| {
                ts.iter()
                    .filter_map(|&t| m.triangles.get(t).copied())
                    .filter(|tri| !m.pinched(*tri))
                    .map(|tri| m.fit(tri, domain))
                    .fold(f64::INFINITY, f64::min)
            };
            let before = worst(self);
            for target in [in_space, in_uv] {
                if !(target.x.is_finite() && target.y.is_finite()) {
                    continue;
                }
                self.place(v, target, domain.surface_point(target));
                let keep = worst(self) > before + FIT_GAIN
                    && ts.iter().all(|&t| {
                        self.triangles.get(t).is_some_and(|&[a, b, c]| {
                            orient(self.p(a), self.p(b), self.p(c)) == Sign::Positive
                        }) && !self.too_far(t, domain, space.tol, space.angle)
                    });
                if keep {
                    break;
                }
                self.place(v, old_uv, old_xyz);
            }
        }
    }

    fn place(&mut self, v: usize, uv: Point2, xyz: Point3) {
        if let (Some(u), Some(x)) = (self.uv.get_mut(v), self.xyz.get_mut(v)) {
            *u = uv;
            *x = xyz;
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

    /// Inserts the midpoint of the interior side `(p, q)`, splitting its two triangles
    /// into four.
    fn split_side(
        &mut self,
        p: u32,
        q: u32,
        domain: &dyn SurfaceDomain,
        budget: &mut usize,
    ) -> Option<()> {
        let t1 = *self.edges.get(&(p, q))?;
        let t2 = *self.edges.get(&(q, p))?;
        if t1 == t2 {
            return None;
        }
        let r = self.apex(t1, p, q)?;
        let s = self.apex(t2, q, p)?;
        let (pp, pq, pr, ps) = (self.p(p), self.p(q), self.p(r), self.p(s));
        let uv = Point2::new((pp.x + pq.x) / 2.0, (pp.y + pq.y) / 2.0);
        // Rounding could put the midpoint off the side; skip such a side.
        let ok = [(pp, uv, pr), (uv, pq, pr), (pq, uv, ps), (uv, pp, ps)]
            .iter()
            .all(|&(a, b, c)| orient(a, b, c) == Sign::Positive);
        if !ok {
            return None;
        }
        let m = u32::try_from(self.uv.len()).ok()?;
        self.uv.push(uv);
        self.xyz.push(domain.surface_point(uv));
        let t3 = self.triangles.len();
        let t4 = t3 + 1;
        *self.triangles.get_mut(t1)? = [p, m, r];
        *self.triangles.get_mut(t2)? = [q, m, s];
        self.triangles.push([m, q, r]);
        self.triangles.push([m, p, s]);
        self.edges.remove(&(p, q));
        self.edges.remove(&(q, p));
        for (e, t) in [
            ((p, m), t1),
            ((m, r), t1),
            ((r, p), t1),
            ((q, m), t2),
            ((m, s), t2),
            ((s, q), t2),
            ((m, q), t3),
            ((q, r), t3),
            ((r, m), t3),
            ((m, p), t4),
            ((p, s), t4),
            ((s, m), t4),
        ] {
            self.edges.insert(e, t);
        }
        self.legalize(vec![(q, r), (r, p), (p, s), (s, q)], budget);
        Some(())
    }

    /// Whether triangle `t` is further from the surface than the tolerances allow.
    fn too_far(&self, t: usize, domain: &dyn SurfaceDomain, tol: f64, angle: f64) -> bool {
        self.defect(t, domain, tol, angle).is_some()
    }

    /// What is wrong with triangle `t`, if anything: one of its interior sides (the
    /// first that spans too much of the surface, strays from it at its midpoint, or
    /// along which the normal turns too far), or its centroid's distance.
    fn defect(&self, t: usize, domain: &dyn SurfaceDomain, tol: f64, angle: f64) -> Option<Fix> {
        let [a, b, c] = *self.triangles.get(t)?;
        let (xa, xb, xc) = (self.x(a), self.x(b), self.x(c));
        let (pa, pb, pc) = (self.p(a), self.p(b), self.p(c));
        // Two copies of a pole: a segment in space, dropped from the mesh; splitting it
        // would only fold triangles over the pole.
        if self.pinched([a, b, c]) {
            return None;
        }
        let mean = |p: &[Point3]| {
            let n = p.len() as f64;
            let s = p.iter().fold(Vec3::ZERO, |s, q| s + (*q - Point3::ORIGIN));
            Point3::ORIGIN + s * (1.0 / n)
        };
        let (max_du, max_dv) = domain.max_span();
        for (i, j) in Self::sides([a, b, c]) {
            // Interior sides only: boundary sides are edge chords, already within
            // tolerance.
            if !self.edges.contains_key(&(j, i)) {
                continue;
            }
            let (pi, pj) = (self.p(i), self.p(j));
            // Longer in u or v than the domain allows.
            if (pi.x - pj.x).abs() > max_du || (pi.y - pj.y).abs() > max_dv {
                return Some(Fix::Side(i, j));
            }
            let mid = Point2::new((pi.x + pj.x) / 2.0, (pi.y + pj.y) / 2.0);
            if domain
                .surface_point(mid)
                .distance(mean(&[self.x(i), self.x(j)]))
                > tol
            {
                return Some(Fix::Side(i, j));
            }
            // The surface normal turns by more than twice the angle tolerance along the
            // side: the facet's normal lies between its corners' normals, so it is then
            // within about the angle tolerance of the surface.
            let turn = domain
                .surface_normal(pi)
                .angle_to(domain.surface_normal(pj));
            if turn.is_some_and(|g| g > 2.0 * angle) {
                return Some(Fix::Side(i, j));
            }
        }
        let centroid_uv = Point2::new((pa.x + pb.x + pc.x) / 3.0, (pa.y + pb.y + pc.y) / 3.0);
        if domain
            .surface_point(centroid_uv)
            .distance(mean(&[xa, xb, xc]))
            > tol
        {
            return Some(Fix::Centroid);
        }
        None
    }

    /// Refines until every triangle is within the tolerances of the surface, then
    /// improves the triangles' fit in space ([`Mesh2::improve_in_space`]). Fails when
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
                let space = Space { domain, tol, angle };
                for _ in 0..IMPROVE_PASSES {
                    self.smooth(&space);
                    self.improve_in_space(&space);
                }
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
                let done = match self.defect(t, domain, tol, angle) {
                    None => None,
                    Some(Fix::Side(i, j)) => self
                        .split_side(i, j, domain, &mut flips)
                        .or_else(|| self.split(t, domain, &mut flips)),
                    Some(Fix::Centroid) => self.split(t, domain, &mut flips),
                };
                progressed |= done.is_some();
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

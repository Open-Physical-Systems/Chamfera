//! Pencil finishing: one pass along each concave corner of the part, where the tool
//! touches it in two places at once (or where a wall stops it), to clean up what the
//! area-clearing passes leave there.
//!
//! Where a ball (or any tool of revolution) touches two faces at once, its
//! cutter-location surface (the tip heights of the drop-cutter, [`crate::drop`]) is the
//! larger of the two faces' offset surfaces, so it has a crease: its slope jumps up
//! across the line of double contact (Kim and Choi, "Pencil-cut tool path generation
//! from the cutter-location surface", after the double-tangency formulation of Ren, Yau
//! and Lee, "Clean-up tool path generation by contraction tool method for machining
//! complex polyhedral models", Computers in Industry 54, 2004). Beside a vertical wall it
//! jumps up in height instead. Where a concave fillet is narrower than the tool the tool
//! cannot follow it and the same crease appears, so this also finds corners by
//! curvature: fillets of a smaller radius than the tool.
//!
//! 1. The cutter-location heights are sampled on a grid (a quarter of the tool radius).
//! 2. Along every grid row and column, an interval where the slope angle grows by more
//!    than half the threshold between its neighbours is a candidate.
//! 3. Each candidate is located on the exact drop-cutter by bisection on the local slope
//!    between the nodes either side of its interval (secants `ε` = an eighth of the
//!    tolerance wide), and kept only when the slope angle
//!    grows by more than `min_angle` across it at that small scale (a smooth surface,
//!    however curved, does not), or the height jumps there. The point is the lower side
//!    of the crease: there the tool touches both faces, or the floor beside the wall.
//! 4. Points on the floor the tool rests on outside the part are dropped, and so are
//!    creases where the part under the tip (a pin-point drop) is more than a tool radius
//!    below it: there the tool spans a gap (two rims of an opening) instead of sitting in
//!    a corner. The rest are chained into passes (nearest neighbours within 1.6 grid
//!    cells, never turning back).
//!
//! Limits: a V-groove narrower than 60 degrees puts its bottom more than a radius below
//! the ball's tip and is not traced; creases are found to within the tolerance but the
//! tool moves straight between the points found, about a grid cell apart.
//!
//! Every evaluation is the exact drop-cutter and counts against [`MAX_DROPS`].

use crate::CamError;
use crate::cancel;
use crate::grid::{self, Grid};
use crate::ops::Cutter;
use crate::ops3d::{Area, Cl, MAX_DROPS, check_tol, cl_grid, cutter_path, emit, simplify, spend};
use crate::toolpath::Toolpath;
use chamfera_geom::{Point2, Point3, Vec2};
use std::collections::HashMap;

/// Most crease points one operation may find.
pub const MAX_PENCIL_POINTS: usize = 400_000;

/// What locating one crease needs.
struct Finder<'a, 'b> {
    cl: &'a Cl<'b>,
    eps: f64,
    min_angle: f64,
    budget: usize,
}

impl Finder<'_, '_> {
    fn at(&mut self, p: Point2) -> Result<f64, CamError> {
        spend(&mut self.budget, 1)?;
        Ok(self.cl.at(p.x, p.y))
    }

    /// The slope along `t` at `p`, over a secant `2 ε` wide.
    fn slope(&mut self, p: Point2, t: Vec2) -> Result<f64, CamError> {
        let a = self.at(p - t * self.eps)?;
        let b = self.at(p + t * self.eps)?;
        Ok((b - a) / (2.0 * self.eps))
    }

    /// The crease between `lo` and `hi` (along `t`), where the slope passes `m`, as the
    /// lower side's point; `None` when there is no real crease there.
    fn locate(
        &mut self,
        lo: Point2,
        hi: Point2,
        t: Vec2,
        m: f64,
    ) -> Result<Option<Point3>, CamError> {
        let len = lo.distance(hi);
        if self.slope(lo, t)? > m || self.slope(hi, t)? <= m {
            return Ok(None);
        }
        let (mut a, mut b) = (0.0f64, len);
        for _ in 0..60 {
            if b - a <= self.eps {
                break;
            }
            let mid = (a + b) / 2.0;
            if self.slope(lo + t * mid, t)? > m {
                b = mid;
            } else {
                a = mid;
            }
        }
        let c = lo + t * ((a + b) / 2.0);
        // The small-scale test: secant angles left, across and right of the crease.
        let d = 4.0 * self.eps;
        let z: Vec<f64> = [-2.0, -1.0, 1.0, 2.0]
            .iter()
            .map(|k| self.at(c + t * (d * k)))
            .collect::<Result<_, _>>()?;
        let ang = |z0: f64, z1: f64, w: f64| ((z1 - z0) / w).atan();
        let (al, am, ar) = (
            ang(z[0], z[1], d),
            ang(z[1], z[2], 2.0 * d),
            ang(z[2], z[3], d),
        );
        let rise = (am - al).max(ar - am).max(ar - al);
        if rise <= self.min_angle {
            return Ok(None);
        }
        let p0 = lo + t * (a - self.eps);
        let p1 = lo + t * (b + self.eps);
        let (z0, z1) = (self.at(p0)?, self.at(p1)?);
        Ok(Some(if z0 <= z1 {
            Point3::new(p0.x, p0.y, z0)
        } else {
            Point3::new(p1.x, p1.y, z1)
        }))
    }

    /// Scans one line of grid nodes `nodes` (with heights `z`, `g` apart along `t`).
    fn scan(
        &mut self,
        nodes: &[Point2],
        z: &[f64],
        t: Vec2,
        g: f64,
        out: &mut Vec<Point3>,
    ) -> Result<(), CamError> {
        let n = nodes.len().min(z.len());
        if n < 4 {
            return Ok(());
        }
        let a: Vec<f64> = (0..n - 1).map(|k| ((z[k + 1] - z[k]) / g).atan()).collect();
        let rise = |k: usize| -> f64 {
            match (k.checked_sub(1).and_then(|i| a.get(i)), a.get(k + 1)) {
                (Some(l), Some(r)) => r - l,
                _ => f64::NEG_INFINITY,
            }
        };
        for k in 1..a.len().saturating_sub(1) {
            let kk = rise(k);
            if !(kk > self.min_angle / 2.0 && kk >= rise(k - 1) && kk >= rise(k + 1)) {
                continue;
            }
            let (Some(al), Some(ar)) = (a.get(k - 1), a.get(k + 1)) else {
                continue;
            };
            let m = ((al + ar) / 2.0).tan();
            let (Some(p), Some(q)) = (nodes.get(k), nodes.get(k + 1)) else {
                continue;
            };
            // From a node before to a node after: a crease exactly on a node is inside.
            if let Some(c) = self.locate(*p - t * g, *q + t * g, t, m)? {
                if out.len() >= MAX_PENCIL_POINTS {
                    return Err(CamError::TooComplex(format!(
                        "more than {MAX_PENCIL_POINTS} pencil points: use a larger tolerance"
                    )));
                }
                out.push(c);
            }
        }
        Ok(())
    }
}

/// Pencil passes over `area` along the part's concave corners (creases of the
/// cutter-location surface steeper than `min_angle`, radians), located to `tol`.
pub fn pencil(
    cl: &Cl,
    cutter: &Cutter,
    area: Area,
    tol: f64,
    min_angle: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    let tol = check_tol(tol)?;
    if !(min_angle.is_finite() && (1e-3..=std::f64::consts::FRAC_PI_2).contains(&min_angle)) {
        return Err(CamError::Invalid(
            "the crease angle must be from 0.06 to 90 degrees".into(),
        ));
    }
    let r = cl.tool.radius;
    let g = Grid::covering(
        area.min,
        area.max,
        (r / 4.0).clamp(0.05, 2.0),
        grid::MAX_GRID_SIDE,
        r,
    )?;
    let field = cl_grid(cl, &g)?;
    let mut f = Finder {
        cl,
        eps: (tol / 8.0).max(1e-5),
        min_angle,
        budget: MAX_DROPS.saturating_sub(g.len()),
    };
    let mut pts: Vec<Point3> = Vec::new();
    for j in 0..g.ny {
        cancel::check()?;
        let nodes: Vec<Point2> = (0..g.nx).map(|i| g.at(i, j)).collect();
        let z: Vec<f64> = (0..g.nx)
            .map(|i| field.get(g.index(i, j)).copied().unwrap_or(cl.floor))
            .collect();
        f.scan(&nodes, &z, Vec2::new(1.0, 0.0), g.cell, &mut pts)?;
    }
    for i in 0..g.nx {
        cancel::check()?;
        let nodes: Vec<Point2> = (0..g.ny).map(|j| g.at(i, j)).collect();
        let z: Vec<f64> = (0..g.ny)
            .map(|j| field.get(g.index(i, j)).copied().unwrap_or(cl.floor))
            .collect();
        f.scan(&nodes, &z, Vec2::new(0.0, 1.0), g.cell, &mut pts)?;
    }
    // Off the part (the tool resting on the floor below the model) is no corner, nor is a
    // crease whose part lies more than a tool radius below the tip (the tool spanning a
    // gap, as over a pocket's corner, resting on its two rims).
    let pin = crate::drop::ToolShape::new(1e-6, 0.0)?;
    let reach = r + cl.leave;
    pts.retain(|p| {
        p.z > cl.floor + tol
            && cl
                .model
                .drop(&pin, p.x, p.y)
                .is_some_and(|h| p.z - h <= reach + tol)
    });
    let chains = chain(&pts, g.cell);
    let mut m = Vec::new();
    for c in chains {
        let c = into_corners(c, g.cell);
        emit(&simplify(c), cutter, clearance, &mut m);
    }
    cutter_path(cutter, m)
}

/// Where a pass turns a corner between two straight runs of crease points (by more than
/// 30 degrees), the corner itself: the meeting point of the run before and the run after,
/// when it lies within 1.5 cells of both ends (so the tool goes into the corner instead of
/// cutting across it), at the lower of their heights.
fn into_corners(c: Vec<Point3>, cell: f64) -> Vec<Point3> {
    let closed = c.len() >= 5
        && c.first()
            .zip(c.last())
            .is_some_and(|(a, b)| a.distance(*b) <= 1e-12);
    // The distinct points; a closed pass turns its corners all the way round.
    let core: Vec<Point3> = if closed {
        c[..c.len() - 1].to_vec()
    } else {
        c.clone()
    };
    let n = core.len();
    if n < 4 {
        return c;
    }
    let at = |i: usize| core[i % n];
    let mut out = Vec::with_capacity(c.len() + 8);
    for (i, &b) in core.iter().enumerate() {
        out.push(b);
        if !closed && (i == 0 || i + 2 >= n) {
            continue;
        }
        let (a, p, q) = (at(i + n - 1), at(i + 1), at(i + 2));
        let d1 = Vec2::new(b.x - a.x, b.y - a.y);
        let d2 = Vec2::new(q.x - p.x, q.y - p.y);
        let (Some(u1), Some(u2)) = (d1.normalized(), d2.normalized()) else {
            continue;
        };
        if u1.dot(u2) > 30f64.to_radians().cos() {
            continue;
        }
        // b + u1 s = p + u2 t.
        let den = u1.cross(u2);
        if den.abs() < 1e-9 {
            continue;
        }
        let w = Vec2::new(p.x - b.x, p.y - b.y);
        let s = w.cross(u2) / den;
        let x = Point2::new(b.x + u1.x * s, b.y + u1.y * s);
        let dist = |r: Point3| x.distance(Point2::new(r.x, r.y));
        if s > 0.0
            && dist(b) <= 1.5 * cell
            && dist(p) <= 1.5 * cell
            && dist(b) > 1e-9
            && dist(p) > 1e-9
        {
            out.push(Point3::new(x.x, x.y, b.z.min(p.z)));
        }
    }
    if closed {
        out.push(core[0]);
    }
    out
}

/// Joins crease points into passes: duplicates within a third of a cell dropped, then
/// nearest neighbours within 1.6 cells chained, never turning back on themselves.
pub(crate) fn chain(pts: &[Point3], cell: f64) -> Vec<Vec<Point3>> {
    let key = |p: &Point3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut kept: Vec<Point3> = Vec::new();
    let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let near = |buckets: &HashMap<(i64, i64), Vec<usize>>, kept: &[Point3], p: &Point3, d: f64| {
        let (i, j) = key(p);
        let reach = (d / cell).ceil() as i64;
        let mut out = Vec::new();
        for di in -reach..=reach {
            for dj in -reach..=reach {
                if let Some(list) = buckets.get(&(i + di, j + dj)) {
                    for &k in list {
                        if kept.get(k).is_some_and(|q| q.distance(*p) <= d) {
                            out.push(k);
                        }
                    }
                }
            }
        }
        out
    };
    for p in pts {
        if near(&buckets, &kept, p, cell / 3.0).is_empty() {
            buckets.entry(key(p)).or_default().push(kept.len());
            kept.push(*p);
        }
    }
    let link = 1.6 * cell;
    let mut used = vec![false; kept.len()];
    let degree: Vec<usize> = kept
        .iter()
        .map(|p| near(&buckets, &kept, p, link).len().saturating_sub(1))
        .collect();
    // Start at ends (one neighbour) first, then anywhere left.
    let mut order: Vec<usize> = (0..kept.len()).collect();
    order.sort_by_key(|k| (degree.get(*k).copied().unwrap_or(0) != 1, *k));
    let mut chains = Vec::new();
    for start in order {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let Some(&first) = kept.get(start) else {
            continue;
        };
        used[start] = true;
        let mut c = vec![first];
        let mut at = start;
        while let Some(&here) = kept.get(at) {
            let dir = (c.len() >= 2).then(|| {
                let prev = c[c.len() - 2];
                Vec2::new(here.x - prev.x, here.y - prev.y)
            });
            let next = near(&buckets, &kept, &here, link)
                .into_iter()
                .filter(|k| !used.get(*k).copied().unwrap_or(true))
                .filter(|k| {
                    dir.is_none_or(|d| {
                        kept.get(*k)
                            .is_some_and(|q| Vec2::new(q.x - here.x, q.y - here.y).dot(d) > 0.0)
                    })
                })
                .min_by(|a, b| {
                    let da = kept.get(*a).map_or(f64::INFINITY, |q| q.distance(here));
                    let db = kept.get(*b).map_or(f64::INFINITY, |q| q.distance(here));
                    da.total_cmp(&db)
                });
            let Some(k) = next else { break };
            used[k] = true;
            if let Some(&q) = kept.get(k) {
                c.push(q);
            }
            at = k;
        }
        if c.len() >= 3
            && let (Some(a), Some(b)) = (c.first(), c.last())
            && a.distance(*b) <= link
        {
            c.push(*a);
        }
        if c.len() >= 2 {
            chains.push(c);
        }
    }
    chains
}

//! Stock simulation on a dexel heightfield (van Hook, "Real-time shaded NC milling
//! display", SIGGRAPH 1986): the stock is a grid of vertical columns over its XY box,
//! each with the height of its top, lowered by sweeping the tool along the toolpath.
//!
//! Each column is cut exactly at its centre: for a straight move from `a` to `b` the
//! tool's lowest surface over a column at distance `ρ(s)` from the axis is
//! `z(s) + h(ρ(s))`, which is convex in `s` (`z` is linear, `h` convex and increasing,
//! `ρ` convex along a line), so its minimum is found by golden-section search (exactly a
//! closed form, `z + h(ρmin)`, when the move is level). Arcs are cut as chords no farther
//! than a hundredth of a millimetre from the arc.
//!
//! The part's own height over each column (a pin-point drop on the exact model) gives the
//! rest material (stock left above the part) and gouges (stock cut below the part by more
//! than a tolerance).
//!
//! With a holder ([`crate::holder`]), every move is first checked for the shank and
//! holder touching the stock or the part: at points along it at most a cell apart, each
//! column under the holder taller than the tip plus the holder's lowest surface over it
//! (by more than the tolerance) is a holder collision. The move is checked against the
//! stock as it is before that move cuts, so the flutes cutting below the shank do not
//! clear its way: a tool plunged deeper than its flutes collides with its shank.

use crate::CamError;
use crate::drop::{DropModel, ToolShape, golden};
use crate::holder::NonCutting;
use crate::toolpath::{Move, Toolpath};
use chamfera_geom::{Point2, Point3, TriMesh};

/// Most columns along either side of the stock.
pub const MAX_SIM_RESOLUTION: usize = 1024;
/// How far an arc's chords may stray from it (mm).
const ARC_CHORD: f64 = 0.01;
/// Most column checks the holder check makes in one simulation; past it the check stops
/// and says it is incomplete.
pub const MAX_HOLDER_WORK: usize = 400_000_000;

/// Where the holder or shank hit the stock or the part in a simulation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HolderHits {
    /// Per column: touched by the holder or shank.
    pub hit: Vec<bool>,
    /// Moves (straight pieces) that collided.
    pub moves: usize,
    /// Deepest a column reached into the holder (mm).
    pub max_depth: f64,
    /// Column checks made so far.
    pub work: usize,
    /// The work limit was reached before every move was checked.
    pub incomplete: bool,
}

impl HolderHits {
    pub fn new(d: &Dexels) -> HolderHits {
        HolderHits {
            hit: vec![false; d.nx * d.ny],
            ..HolderHits::default()
        }
    }

    /// Columns touched.
    pub fn cells(&self) -> usize {
        self.hit.iter().filter(|h| **h).count()
    }
}

/// The stock as columns: `nx` by `ny` square cells of `cell`, the column of cell
/// `(i, j)` centred at `min + (i + ½, j + ½) cell`.
#[derive(Debug, Clone, PartialEq)]
pub struct Dexels {
    pub min: Point2,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    pub top: f64,
    pub bottom: f64,
    pub z: Vec<f64>,
}

impl Dexels {
    /// A block from `min` to `max` (top `max.z`), `resolution` cells along its longer side.
    pub fn new(min: Point3, max: Point3, resolution: usize) -> Result<Dexels, CamError> {
        if resolution == 0 || resolution > MAX_SIM_RESOLUTION {
            return Err(CamError::TooComplex(format!(
                "the resolution must be from 1 to {MAX_SIM_RESOLUTION} cells per side"
            )));
        }
        let (w, h) = (max.x - min.x, max.y - min.y);
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && max.z > min.z) {
            return Err(CamError::Invalid("the stock has no size".into()));
        }
        let cell = w.max(h) / resolution as f64;
        let nx = ((w / cell).round() as usize).clamp(1, MAX_SIM_RESOLUTION);
        let ny = ((h / cell).round() as usize).clamp(1, MAX_SIM_RESOLUTION);
        Ok(Dexels {
            min: Point2::new(min.x, min.y),
            cell,
            nx,
            ny,
            top: max.z,
            bottom: min.z,
            z: vec![max.z; nx * ny],
        })
    }

    pub fn centre(&self, i: usize, j: usize) -> Point2 {
        Point2::new(
            self.min.x + (i as f64 + 0.5) * self.cell,
            self.min.y + (j as f64 + 0.5) * self.cell,
        )
    }

    /// One cell's area (mm²).
    pub fn area(&self) -> f64 {
        self.cell * self.cell
    }

    /// The volume cut away so far.
    pub fn removed_volume(&self) -> f64 {
        self.z.iter().map(|z| self.top - z).sum::<f64>() * self.area()
    }

    /// Lowers the columns the tool passes over going straight from `a` to `b`.
    pub fn cut_line(&mut self, tool: &ToolShape, a: Point3, b: Point3) {
        let rr = tool.radius;
        if a.z.min(b.z) >= self.top {
            return;
        }
        let lo = Point2::new(a.x.min(b.x) - rr, a.y.min(b.y) - rr);
        let hi = Point2::new(a.x.max(b.x) + rr, a.y.max(b.y) + rr);
        let i0 = (((lo.x - self.min.x) / self.cell).floor().max(0.0)) as usize;
        let j0 = (((lo.y - self.min.y) / self.cell).floor().max(0.0)) as usize;
        let i1 = (((hi.x - self.min.x) / self.cell).ceil().max(0.0) as usize).min(self.nx);
        let j1 = (((hi.y - self.min.y) / self.cell).ceil().max(0.0) as usize).min(self.ny);
        let (dx, dy, dz) = (b.x - a.x, b.y - a.y, b.z - a.z);
        let dd = dx * dx + dy * dy;
        let level = dz.abs() <= 1e-12;
        for j in j0..j1 {
            for i in i0..i1 {
                let c = self.centre(i, j);
                let (ax, ay) = (a.x - c.x, a.y - c.y);
                let low = if dd <= 1e-24 {
                    tool.height(ax.hypot(ay)).map(|h| a.z.min(b.z) + h)
                } else if level {
                    let s = (-(ax * dx + ay * dy) / dd).clamp(0.0, 1.0);
                    let rho = (ax + dx * s).hypot(ay + dy * s);
                    tool.height(rho).map(|h| a.z + h)
                } else {
                    // The part of the move with the column under the tool, then the
                    // convex minimum on it.
                    let bq = ax * dx + ay * dy;
                    let cq = ax * ax + ay * ay - rr * rr;
                    let disc = bq * bq - dd * cq;
                    if disc < 0.0 {
                        None
                    } else {
                        let sq = disc.sqrt();
                        let s0 = ((-bq - sq) / dd).max(0.0);
                        let s1 = ((-bq + sq) / dd).min(1.0);
                        if s0 > s1 {
                            None
                        } else {
                            let f = |s: f64| {
                                let rho = (ax + dx * s).hypot(ay + dy * s);
                                tool.height(rho).map_or(f64::INFINITY, |h| a.z + dz * s + h)
                            };
                            let (_, v) = golden(s0, s1, &|s| -f(s));
                            Some((-v).min(f(s0)).min(f(s1)))
                        }
                    }
                };
                if let Some(l) = low
                    && let Some(z) = self.z.get_mut(j * self.nx + i)
                    && l < *z
                {
                    *z = l.max(self.bottom);
                }
            }
        }
    }

    /// Cuts the toolpath's moves, the first `fraction` (0 to 1) of its cycle time at
    /// `rapid_rate` (mm/min). Rapids cut too: a rapid into the stock is a crash, and the
    /// gouge check shows it.
    pub fn cut_toolpath(
        &mut self,
        tool: &ToolShape,
        tp: &Toolpath,
        rapid_rate: f64,
        fraction: f64,
    ) {
        self.cut_toolpath_checked(tool, None, tp, rapid_rate, fraction);
    }

    /// [`Dexels::cut_toolpath`], checking each move for holder collisions first when a
    /// holder is given: `(holder, part heights, tolerance, hits)`.
    pub fn cut_toolpath_checked(
        &mut self,
        tool: &ToolShape,
        mut holder: Option<(&NonCutting, &[Option<f64>], f64, &mut HolderHits)>,
        tp: &Toolpath,
        rapid_rate: f64,
        fraction: f64,
    ) {
        let segs = segments(tp);
        let total: f64 = segs.iter().map(|s| s.minutes(rapid_rate)).sum();
        let mut budget = total * fraction.clamp(0.0, 1.0);
        let part_max = holder.as_ref().map_or(f64::NEG_INFINITY, |h| {
            h.1.iter()
                .flatten()
                .fold(f64::NEG_INFINITY, |a, b| a.max(*b))
        });
        for s in &segs {
            let t = s.minutes(rapid_rate);
            let (a, b, last) = if t <= budget || fraction >= 1.0 {
                budget -= t;
                (s.a, s.b, false)
            } else if t > 0.0 && budget > 0.0 {
                (s.a, s.a + (s.b - s.a) * (budget / t), true)
            } else {
                break;
            };
            if let Some((nc, part, tol, hits)) = holder.as_mut() {
                self.check_holder(nc, part, part_max, *tol, a, b, hits);
            }
            self.cut_line(tool, a, b);
            if last {
                break;
            }
        }
    }

    /// Marks the columns the holder or shank touches along `a`–`b`.
    #[allow(clippy::too_many_arguments)]
    fn check_holder(
        &self,
        nc: &NonCutting,
        part: &[Option<f64>],
        part_max: f64,
        tol: f64,
        a: Point3,
        b: Point3,
        hits: &mut HolderHits,
    ) {
        if hits.incomplete {
            return;
        }
        let lowest = nc.lowest();
        let top = self.top.max(part_max);
        if a.z.min(b.z) + lowest >= top - tol {
            return;
        }
        let rr = nc.max_radius();
        let len = (b - a).length();
        let n = ((len / self.cell).ceil() as usize).clamp(1, 100_000);
        let mut collided = false;
        for s in 0..=n {
            let p = a + (b - a) * (s as f64 / n as f64);
            if p.z + lowest >= top - tol {
                continue;
            }
            let i0 = (((p.x - rr - self.min.x) / self.cell).floor().max(0.0)) as usize;
            let j0 = (((p.y - rr - self.min.y) / self.cell).floor().max(0.0)) as usize;
            let i1 = (((p.x + rr - self.min.x) / self.cell).ceil().max(0.0) as usize).min(self.nx);
            let j1 = (((p.y + rr - self.min.y) / self.cell).ceil().max(0.0) as usize).min(self.ny);
            let window = i1.saturating_sub(i0).saturating_mul(j1.saturating_sub(j0));
            hits.work = hits.work.saturating_add(window);
            if hits.work > MAX_HOLDER_WORK {
                hits.incomplete = true;
                break;
            }
            for j in j0..j1 {
                for i in i0..i1 {
                    let c = self.centre(i, j);
                    let rho = (c.x - p.x).hypot(c.y - p.y);
                    let Some(low) = nc.low(rho) else { continue };
                    let k = j * self.nx + i;
                    let stock = self.z.get(k).copied().unwrap_or(self.bottom);
                    let solid = part
                        .get(k)
                        .copied()
                        .flatten()
                        .map_or(stock, |h| h.max(stock));
                    let depth = solid - (p.z + low);
                    if depth > tol {
                        collided = true;
                        hits.max_depth = hits.max_depth.max(depth);
                        if let Some(h) = hits.hit.get_mut(k) {
                            *h = true;
                        }
                    }
                }
            }
        }
        if collided {
            hits.moves += 1;
        }
    }
}

/// A straight piece of a toolpath, at feed or at rapid.
#[derive(Debug, Clone, Copy)]
pub struct Piece {
    pub a: Point3,
    pub b: Point3,
    /// mm/min; `None` at rapid.
    pub feed: Option<f64>,
}

impl Piece {
    fn minutes(&self, rapid: f64) -> f64 {
        let l = (self.b - self.a).length();
        let rate = self.feed.unwrap_or(rapid);
        if rate > 0.0 { l / rate } else { 0.0 }
    }
}

/// The toolpath as straight pieces (drill cycles expanded, arcs as chords).
pub fn segments(tp: &Toolpath) -> Vec<Piece> {
    let mut out = Vec::new();
    let Some(mut at) = tp.start() else {
        return out;
    };
    for m in tp.expanded() {
        match m {
            Move::Rapid { to } => {
                out.push(Piece {
                    a: at,
                    b: to,
                    feed: None,
                });
                at = to;
            }
            Move::Line { to, feed } | Move::Plunge { to, feed } => {
                out.push(Piece {
                    a: at,
                    b: to,
                    feed: Some(feed),
                });
                at = to;
            }
            Move::Arc {
                to,
                centre,
                clockwise,
                feed,
            } => {
                let from2 = Point2::new(at.x, at.y);
                let r = from2.distance(centre);
                let a0 = (from2 - centre).angle();
                let a1 = (Point2::new(to.x, to.y) - centre).angle();
                let mut sweep = crate::path2::norm_angle(if clockwise { a0 - a1 } else { a1 - a0 });
                if sweep * r <= crate::path2::JOIN_TOLERANCE {
                    sweep = std::f64::consts::TAU;
                }
                // A chord of angle θ strays r (1 - cos θ/2) from the arc.
                let step = if r > ARC_CHORD {
                    2.0 * (1.0 - ARC_CHORD / r).clamp(-1.0, 1.0).acos()
                } else {
                    sweep
                };
                let n = ((sweep / step.max(1e-3)).ceil() as usize).clamp(1, 4096);
                let mut prev = at;
                for k in 1..=n {
                    let f = k as f64 / n as f64;
                    let ang = if clockwise {
                        a0 - sweep * f
                    } else {
                        a0 + sweep * f
                    };
                    let p = if k == n {
                        to
                    } else {
                        Point3::new(
                            centre.x + r * ang.cos(),
                            centre.y + r * ang.sin(),
                            at.z + (to.z - at.z) * f,
                        )
                    };
                    out.push(Piece {
                        a: prev,
                        b: p,
                        feed: Some(feed),
                    });
                    prev = p;
                }
                at = to;
            }
            Move::Drill { .. } => {}
        }
    }
    out
}

/// The part's height over each column (`None` where no part is under it).
pub fn part_heights(model: &DropModel, d: &Dexels) -> Result<Vec<Option<f64>>, CamError> {
    let pin = ToolShape::new(1e-6, 0.0)?;
    let mut out = Vec::with_capacity(d.nx * d.ny);
    for j in 0..d.ny {
        for i in 0..d.nx {
            let c = d.centre(i, j);
            out.push(model.drop(&pin, c.x, c.y));
        }
    }
    Ok(out)
}

/// What a simulation left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Report {
    pub removed_volume: f64,
    /// Stock left above the part (and above the stock's bottom where there is no part).
    pub rest_volume: f64,
    /// Columns cut below the part by more than the tolerance.
    pub gouge_cells: usize,
    pub gouge_volume: f64,
    pub max_gouge: f64,
}

/// Measures the stock against the part's heights.
pub fn report(d: &Dexels, part: &[Option<f64>], tolerance: f64) -> Report {
    let mut r = Report {
        removed_volume: d.removed_volume(),
        ..Report::default()
    };
    let a = d.area();
    for (k, z) in d.z.iter().enumerate() {
        let h = part.get(k).copied().flatten();
        let floor = h.map_or(d.bottom, |h| h.max(d.bottom));
        r.rest_volume += (z - floor).max(0.0) * a;
        if let Some(h) = h
            && *z < h - tolerance
        {
            let depth = h.min(d.top) - z;
            r.gouge_cells += 1;
            r.gouge_volume += depth.max(0.0) * a;
            r.max_gouge = r.max_gouge.max(depth);
        }
    }
    r
}

/// The stock as display meshes: `(stock, gouges)`, the gouged columns' tops apart so they
/// can take their own colour. At most `max_side` display cells per side (columns are
/// merged in blocks, each shown at its lowest column).
pub fn meshes(
    d: &Dexels,
    part: &[Option<f64>],
    tolerance: f64,
    max_side: usize,
) -> (TriMesh, TriMesh) {
    let (stock, bad, _) = meshes_with(d, part, tolerance, max_side, &[]);
    (stock, bad)
}

/// [`meshes`], and the tops of the columns the holder touched (`holder_hit`, per column)
/// apart again: `(stock, gouges, holder collisions)`.
pub fn meshes_with(
    d: &Dexels,
    part: &[Option<f64>],
    tolerance: f64,
    max_side: usize,
    holder_hit: &[bool],
) -> (TriMesh, TriMesh, TriMesh) {
    let k = d.nx.max(d.ny).div_ceil(max_side.max(1)).max(1);
    let bx = d.nx.div_ceil(k);
    let by = d.ny.div_ceil(k);
    let mut hz = vec![d.top; bx * by];
    let mut gouge = vec![false; bx * by];
    let mut held = vec![false; bx * by];
    for j in 0..d.ny {
        for i in 0..d.nx {
            let b = (j / k) * bx + i / k;
            let z = d.z[j * d.nx + i];
            if let Some(slot) = hz.get_mut(b) {
                *slot = slot.min(z);
            }
            if let Some(Some(h)) = part.get(j * d.nx + i)
                && z < h - tolerance
                && let Some(g) = gouge.get_mut(b)
            {
                *g = true;
            }
            if holder_hit.get(j * d.nx + i).copied().unwrap_or(false)
                && let Some(g) = held.get_mut(b)
            {
                *g = true;
            }
        }
    }
    let cs = d.cell * k as f64;
    let x = |i: usize| (d.min.x + i as f64 * cs).min(d.min.x + d.nx as f64 * d.cell);
    let y = |j: usize| (d.min.y + j as f64 * cs).min(d.min.y + d.ny as f64 * d.cell);
    let mut stock = TriMesh::default();
    let mut bad = TriMesh::default();
    let mut hold = TriMesh::default();
    let quad = |m: &mut TriMesh, p: [Point3; 4]| {
        let n = m.positions.len() as u32;
        m.positions.extend(p);
        m.triangles.push([n, n + 1, n + 2]);
        m.triangles.push([n, n + 2, n + 3]);
    };
    let h = |i: usize, j: usize| hz[j * bx + i];
    for j in 0..by {
        for i in 0..bx {
            let z = h(i, j);
            let top = [
                Point3::new(x(i), y(j), z),
                Point3::new(x(i + 1), y(j), z),
                Point3::new(x(i + 1), y(j + 1), z),
                Point3::new(x(i), y(j + 1), z),
            ];
            if held[j * bx + i] {
                quad(&mut hold, top);
            } else if gouge[j * bx + i] {
                quad(&mut bad, top);
            } else {
                quad(&mut stock, top);
            }
            // Walls to the +x and +y neighbours (or down to the bottom at the border),
            // facing the lower side.
            let zx = if i + 1 < bx { h(i + 1, j) } else { d.bottom };
            if (zx - z).abs() > 1e-9 {
                let (lo, hi) = (zx.min(z), zx.max(z));
                let xx = x(i + 1);
                let w = [
                    Point3::new(xx, y(j), lo),
                    Point3::new(xx, y(j + 1), lo),
                    Point3::new(xx, y(j + 1), hi),
                    Point3::new(xx, y(j), hi),
                ];
                quad(
                    &mut stock,
                    if zx < z { w } else { [w[1], w[0], w[3], w[2]] },
                );
            }
            let zy = if j + 1 < by { h(i, j + 1) } else { d.bottom };
            if (zy - z).abs() > 1e-9 {
                let (lo, hi) = (zy.min(z), zy.max(z));
                let yy = y(j + 1);
                let w = [
                    Point3::new(x(i + 1), yy, lo),
                    Point3::new(x(i), yy, lo),
                    Point3::new(x(i), yy, hi),
                    Point3::new(x(i + 1), yy, hi),
                ];
                quad(
                    &mut stock,
                    if zy < z { w } else { [w[1], w[0], w[3], w[2]] },
                );
            }
            if i == 0 {
                let w = [
                    Point3::new(x(0), y(j + 1), d.bottom),
                    Point3::new(x(0), y(j), d.bottom),
                    Point3::new(x(0), y(j), z),
                    Point3::new(x(0), y(j + 1), z),
                ];
                quad(&mut stock, w);
            }
            if j == 0 {
                let w = [
                    Point3::new(x(i), y(0), d.bottom),
                    Point3::new(x(i + 1), y(0), d.bottom),
                    Point3::new(x(i + 1), y(0), z),
                    Point3::new(x(i), y(0), z),
                ];
                quad(&mut stock, w);
            }
        }
    }
    let b = d.bottom;
    quad(
        &mut stock,
        [
            Point3::new(x(0), y(0), b),
            Point3::new(x(0), y(by), b),
            Point3::new(x(bx), y(by), b),
            Point3::new(x(bx), y(0), b),
        ],
    );
    (stock, bad, hold)
}

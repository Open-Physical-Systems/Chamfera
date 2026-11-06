//! 3-axis operations: parallel (raster) finishing, waterline (Z-level) finishing and
//! adaptive (bounded-engagement) roughing, all on the exact drop-cutter ([`crate::drop`]).
//!
//! - **Parallel**: raster passes at a stepover, the drop-cutter along each pass with
//!   adaptive sampling (a span is halved while its midpoint's height is farther than the
//!   tolerance from the chord), joined end to end along the area's border so the tool
//!   never lifts (Choi and Jerard, "Sculptured Surface Machining", 1998, ch. 8). For a
//!   ball tool the stepover can come from a scallop height `h`: two passes `s` apart on a
//!   flat floor leave a cusp of `h` where `s = 2 sqrt(2 R h - h²)` (ibid., §8.3).
//! - **Waterline**: at each Z level the tool's cutter-location surface (the drop-cutter
//!   heights over a grid) is sliced at that height, by marching squares with each crossing
//!   refined by bisection on the exact drop-cutter, so the contour points touch the part
//!   to the tolerance (Kim and Choi, Z-level machining from the offset surface).
//! - **Adaptive**: see [`adaptive`].
//!
//! Every sampling is capped ([`MAX_DROPS`], [`grid::MAX_GRID_SIDE`]); jobs above the
//! caps are refused, not truncated.

use crate::CamError;
use crate::cancel;
use crate::drop::{DropModel, ToolShape};
use crate::grid::{self, Grid};
use crate::holder::PartCheck;
use crate::ops::{Cutter, FEED_HEIGHT, depths};
use crate::toolpath::{Move, Toolpath};
use chamfera_doc::cam::{HolderCheck, Tool, ToolKind};
use chamfera_geom::{Point2, Point3, Vec2};
use std::f64::consts::{PI, TAU};

/// Largest tool radius the 3D operations take (mm).
pub const MAX_TOOL_RADIUS: f64 = 500.0;
/// Most drop-cutter evaluations one operation may make.
pub const MAX_DROPS: usize = 3_000_000;
/// Most times a parallel span is halved (each halving doubles the samples there).
pub const MAX_SUBDIVISION: u32 = 8;
/// Most Z levels of a waterline or adaptive operation.
pub const MAX_LEVELS: usize = 500;
/// Finest tolerance the 3D operations take (mm).
pub const MIN_TOLERANCE: f64 = 1e-4;

/// The cutter shape of a library tool, for 3D work: flat, ball and bull-nose mills.
pub fn tool_shape(t: &Tool) -> Result<ToolShape, CamError> {
    let r = t.diameter / 2.0;
    match t.kind {
        ToolKind::FlatEndMill => ToolShape::new(r, 0.0),
        ToolKind::BallEndMill => ToolShape::new(r, r),
        ToolKind::BullNose => ToolShape::new(r, t.corner_radius),
        ToolKind::Drill | ToolKind::ChamferMill => Err(CamError::Refused(format!(
            "a {} cannot cut a 3D operation: use a flat, ball or bull nose mill",
            t.kind.words()
        ))),
    }
}

/// The stepover (mm) a ball of radius `r` needs to leave cusps of height `h` on a flat
/// floor: `s = 2 sqrt(2 r h - h²)` (for `0 < h <= r`).
pub fn scallop_stepover(r: f64, h: f64) -> Result<f64, CamError> {
    if !(r.is_finite() && h.is_finite() && r > 0.0 && h > 0.0 && h <= r) {
        return Err(CamError::Invalid(
            "the scallop height must be above 0 and at most the ball radius".into(),
        ));
    }
    Ok(2.0 * (2.0 * r * h - h * h).sqrt())
}

/// A drop-cutter that leaves `leave` on the part: the tool grown by `leave` (a tool of
/// radius `R + l` and corner `r + l` is the tool's offset by `l`) is dropped; the tool
/// itself sits `l` above the grown tool's tip, so its tip height is the grown drop plus
/// `l`, everywhere `l` clear of the part. Below the model the tool rests at `floor`.
#[derive(Debug, Clone)]
pub struct Cl<'a> {
    pub model: &'a DropModel,
    pub tool: ToolShape,
    grown: ToolShape,
    pub leave: f64,
    pub floor: f64,
}

impl<'a> Cl<'a> {
    pub fn new(
        model: &'a DropModel,
        tool: ToolShape,
        leave: f64,
        floor: f64,
    ) -> Result<Self, CamError> {
        if !(leave.is_finite() && (0.0..=MAX_TOOL_RADIUS).contains(&leave)) {
            return Err(CamError::Invalid(
                "the stock to leave must be 0 or more".into(),
            ));
        }
        let grown = ToolShape::new(tool.radius + leave, tool.corner + leave)?;
        Ok(Cl {
            model,
            tool,
            grown,
            leave,
            floor,
        })
    }

    /// The tip height at `(x, y)`.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        self.model
            .drop(&self.grown, x, y)
            .map_or(self.floor, |z| (z + self.leave).max(self.floor))
    }
}

/// The rectangle the tool's centre covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub min: Point2,
    pub max: Point2,
}

/// The holder check of a 3D operation: the part check and what to do on a collision.
#[derive(Debug, Clone, Copy)]
pub struct HolderUse<'a> {
    pub check: &'a PartCheck,
    pub mode: HolderCheck,
}

/// Spends `k` drop-cutter evaluations of the budget.
pub(crate) fn spend(budget: &mut usize, k: usize) -> Result<(), CamError> {
    *budget = budget.checked_sub(k).ok_or_else(|| {
        CamError::TooComplex(format!(
            "the operation would need more than {MAX_DROPS} drop-cutter points: use a \
             larger stepover or tolerance"
        ))
    })?;
    Ok(())
}

/// The holder check along sampled points: with `Lift` each point is raised to where the
/// holder clears the part; with `Refuse` the points where it does not are dropped and the
/// run is split there (the tool lifts over them). Returns the runs to cut.
fn holder_runs(
    cl: &Cl,
    holder: Option<HolderUse>,
    pts: Vec<Point3>,
    budget: &mut usize,
) -> Result<Vec<Vec<Point3>>, CamError> {
    let Some(h) = holder.filter(|h| h.mode != HolderCheck::Ignore) else {
        return Ok(vec![pts]);
    };
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for (i, mut p) in pts.into_iter().enumerate() {
        if i.is_multiple_of(256) {
            cancel::check()?;
        }
        spend(budget, 1)?;
        let need = h.check.required(cl.model, p.x, p.y);
        if need > p.z + 1e-9 {
            match h.mode {
                HolderCheck::Lift | HolderCheck::Ignore => p.z = need,
                HolderCheck::Refuse => {
                    if run.len() >= 2 {
                        runs.push(std::mem::take(&mut run));
                    } else {
                        run.clear();
                    }
                    continue;
                }
            }
        }
        run.push(p);
    }
    if run.len() >= 2 {
        runs.push(run);
    }
    Ok(runs)
}

pub(crate) fn check_tol(t: f64) -> Result<f64, CamError> {
    if !(t.is_finite() && (MIN_TOLERANCE..=10.0).contains(&t)) {
        return Err(CamError::Invalid(format!(
            "the tolerance must be from {MIN_TOLERANCE} to 10 mm"
        )));
    }
    Ok(t)
}

/// Drops `cl` along `a`–`b` at most `spacing` apart, halving each span whose midpoint
/// is farther than `tol` from the chord (at most [`MAX_SUBDIVISION`] times). Appends
/// the points after `a` (with `a` first when `out` is empty).
fn sample_line(
    cl: &Cl,
    a: Point2,
    b: Point2,
    spacing: f64,
    tol: f64,
    budget: &mut usize,
    out: &mut Vec<Point3>,
) -> Result<(), CamError> {
    cancel::check()?;
    let len = a.distance(b);
    let n = ((len / spacing).ceil() as usize).max(1);
    let p = |t: f64| a + (b - a) * t;
    let mut z_of = |t: f64| -> Result<Point3, CamError> {
        spend(budget, 1)?;
        let q = p(t);
        Ok(Point3::new(q.x, q.y, cl.at(q.x, q.y)))
    };
    let mut prev = z_of(0.0)?;
    if out.is_empty() {
        out.push(prev);
    }
    let mut prev_t = 0.0;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let end = z_of(t)?;
        // Depth-first halving, emitting in order.
        let mut stack = vec![(prev_t, prev, t, end, 0u32)];
        while let Some((t0, p0, t1, p1, depth)) = stack.pop() {
            if depth < MAX_SUBDIVISION && (t1 - t0) * len > 1e-3 {
                let tm = (t0 + t1) / 2.0;
                let pm = z_of(tm)?;
                if (pm.z - (p0.z + p1.z) / 2.0).abs() > tol {
                    stack.push((tm, pm, t1, p1, depth + 1));
                    stack.push((t0, p0, tm, pm, depth + 1));
                    continue;
                }
            }
            out.push(p1);
        }
        prev = end;
        prev_t = t;
    }
    Ok(())
}

/// Drops points that lie on the line through their neighbours (within 1e-6 mm).
pub(crate) fn simplify(points: Vec<Point3>) -> Vec<Point3> {
    let mut out: Vec<Point3> = Vec::with_capacity(points.len());
    for p in points {
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let ab = b - a;
            let ap = p - a;
            let l = ap.length();
            if l > 0.0 && ab.dot(ap) > 0.0 && ab.length() < l && ab.cross(ap).length() / l <= 1e-6 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// Feed moves along `pts`, entered from the clearance height and left back up to it.
pub(crate) fn emit(pts: &[Point3], cutter: &Cutter, clearance: f64, m: &mut Vec<Move>) {
    let Some(first) = pts.first() else { return };
    m.push(Move::Rapid {
        to: Point3::new(first.x, first.y, clearance),
    });
    let feed_from = (first.z + FEED_HEIGHT).min(clearance);
    m.push(Move::Rapid {
        to: Point3::new(first.x, first.y, feed_from),
    });
    m.push(Move::Plunge {
        to: *first,
        feed: cutter.plunge_feed,
    });
    for p in pts.iter().skip(1) {
        m.push(Move::Line {
            to: *p,
            feed: cutter.feed,
        });
    }
    if let Some(last) = pts.last() {
        m.push(Move::Rapid {
            to: Point3::new(last.x, last.y, clearance),
        });
    }
}

/// Parallel finishing over `area`: passes along `angle` (radians from +X), `stepover`
/// mm apart, adaptively sampled to `tol`, zigzag joined along the border.
#[allow(clippy::too_many_arguments)]
pub fn parallel(
    cl: &Cl,
    cutter: &Cutter,
    area: Area,
    stepover: f64,
    angle: f64,
    tol: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    parallel_with(cl, cutter, area, stepover, angle, tol, clearance, None)
}

/// [`parallel`] with a holder check: where the holder or shank would hit the part the
/// tool is lifted until it clears, or the pass is broken there ([`HolderCheck`]).
#[allow(clippy::too_many_arguments)]
pub fn parallel_with(
    cl: &Cl,
    cutter: &Cutter,
    area: Area,
    stepover: f64,
    angle: f64,
    tol: f64,
    clearance: f64,
    holder: Option<HolderUse>,
) -> Result<Toolpath, CamError> {
    let tol = check_tol(tol)?;
    if !(stepover.is_finite() && stepover > 0.0) {
        return Err(CamError::Invalid("the stepover must be positive".into()));
    }
    if !angle.is_finite() {
        return Err(CamError::Invalid(
            "the angle must be a finite number".into(),
        ));
    }
    let u = Vec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let corners = [
        area.min,
        Point2::new(area.max.x, area.min.y),
        area.max,
        Point2::new(area.min.x, area.max.y),
    ];
    let o = Point2::new(0.0, 0.0);
    let (mut u0, mut u1, mut v0, mut v1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for c in corners {
        let d = c - o;
        u0 = u0.min(d.dot(u));
        u1 = u1.max(d.dot(u));
        v0 = v0.min(d.dot(v));
        v1 = v1.max(d.dot(v));
    }
    if !(u1 > u0 && v1 > v0) {
        return Err(CamError::Invalid("the area to machine has no size".into()));
    }
    let passes = ((v1 - v0) / stepover - 1e-9).ceil().max(1.0);
    let spacing = (cl.tool.radius / 4.0).clamp(0.05, 2.0);
    let est = passes * ((u1 - u0) / spacing + 2.0);
    if !(est.is_finite() && est <= MAX_DROPS as f64) {
        return Err(CamError::TooComplex(format!(
            "about {est:.0} drop-cutter points: more than {MAX_DROPS}; use a larger \
             stepover"
        )));
    }
    let passes = passes as usize;
    let dv = (v1 - v0) / passes as f64;
    let at = |s: f64, t: f64| o + u * s + v * t;
    let mut budget = MAX_DROPS;
    let mut pts: Vec<Point3> = Vec::new();
    for k in 0..=passes {
        let t = v0 + dv * k as f64;
        let (a, b) = if k % 2 == 0 { (u0, u1) } else { (u1, u0) };
        if k > 0 {
            // The link along the border from the last pass's end.
            let t_prev = v0 + dv * (k - 1) as f64;
            sample_line(
                cl,
                at(a, t_prev),
                at(a, t),
                spacing,
                tol,
                &mut budget,
                &mut pts,
            )?;
        }
        sample_line(cl, at(a, t), at(b, t), spacing, tol, &mut budget, &mut pts)?;
    }
    let mut m = Vec::new();
    for run in holder_runs(cl, holder, pts, &mut budget)? {
        emit(&simplify(run), cutter, clearance, &mut m);
    }
    cutter_path(cutter, m)
}

pub(crate) fn cutter_path(c: &Cutter, moves: Vec<Move>) -> Result<Toolpath, CamError> {
    if moves.len() > crate::ops::MAX_MOVES {
        return Err(CamError::TooComplex(format!(
            "the toolpath would have more than {} moves",
            crate::ops::MAX_MOVES
        )));
    }
    Ok(Toolpath {
        tool_number: c.number,
        tool_name: c.name.clone(),
        spindle_rpm: c.spindle_rpm,
        moves,
    })
}

/// The cutter-location heights over `g`.
pub(crate) fn cl_grid(cl: &Cl, g: &Grid) -> Result<Vec<f64>, CamError> {
    if g.len() > MAX_DROPS {
        return Err(CamError::TooComplex("the grid is too fine".into()));
    }
    let mut out = Vec::with_capacity(g.len());
    for j in 0..g.ny {
        cancel::check()?;
        for i in 0..g.nx {
            let p = g.at(i, j);
            out.push(cl.at(p.x, p.y));
        }
    }
    Ok(out)
}

/// Waterline finishing: at each level from the model's top down to `bottom`, at most
/// `stepdown` apart, the closed contours where the tool's tip at that height touches the
/// part. `tol` is the grid's spacing bound and the crossing refinement's tolerance.
#[allow(clippy::too_many_arguments)]
pub fn waterline(
    cl: &Cl,
    cutter: &Cutter,
    area: Area,
    top: f64,
    bottom: f64,
    stepdown: f64,
    tol: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    let tol = check_tol(tol)?;
    let r = cl.tool.radius;
    // A grid fine enough to see every contour wider than a quarter of the tool.
    let g = Grid::covering(
        area.min,
        area.max,
        (r / 4.0).max(tol),
        grid::MAX_GRID_SIDE,
        r,
    )?;
    let field = cl_grid(cl, &g)?;
    let levels = depths(top, bottom, stepdown)?;
    if levels.len() > MAX_LEVELS {
        return Err(CamError::TooComplex(format!(
            "more than {MAX_LEVELS} levels"
        )));
    }
    let mut budget = MAX_DROPS.saturating_sub(g.len());
    let mut m = Vec::new();
    for z in levels {
        cancel::check()?;
        // Just above the floor there is nothing to follow.
        if z <= cl.floor + 1e-9 {
            continue;
        }
        let mut fail = None;
        let mut refine = |a: Point2, b: Point2, va: f64, vb: f64| -> Point2 {
            // Bisection on the exact drop-cutter between the two nodes.
            let (mut lo, mut hi) = (0.0f64, 1.0f64);
            let lo_in = va > z;
            let lin = ((va - z) / (va - vb)).clamp(0.0, 1.0);
            if !(va.is_finite() && vb.is_finite()) {
                return a + (b - a) * lin;
            }
            for _ in 0..40 {
                if (hi - lo) * a.distance(b) <= tol / 4.0 {
                    break;
                }
                if budget == 0 {
                    fail = Some(());
                    break;
                }
                budget -= 1;
                let mid = (lo + hi) / 2.0;
                let p = a + (b - a) * mid;
                if (cl.at(p.x, p.y) > z) == lo_in {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            // The outer end: the tool there is at or below `z` (it does not touch).
            let t = if lo_in { hi } else { lo };
            a + (b - a) * t
        };
        let loops = grid::contours(&g, &field, z, &mut refine);
        if fail.is_some() {
            return Err(CamError::TooComplex(format!(
                "the operation would need more than {MAX_DROPS} drop-cutter points"
            )));
        }
        for c in loops {
            let mut pts: Vec<Point3> = c.points.iter().map(|p| Point3::new(p.x, p.y, z)).collect();
            if c.closed
                && let Some(f) = pts.first().copied()
            {
                pts.push(f);
            }
            emit(&simplify(pts), cutter, clearance, &mut m);
        }
    }
    cutter_path(cutter, m)
}

/// The settings of an adaptive clearing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveParams {
    /// Stock box in XY and its top.
    pub stock_min: Point2,
    pub stock_max: Point2,
    pub stock_top: f64,
    /// Lowest level to clear.
    pub bottom: f64,
    pub stepdown: f64,
    /// Largest engagement angle (radians, 0 to π).
    pub max_engagement: f64,
    pub clearance: f64,
}

/// What an adaptive clearing made, and what it had to leave.
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveResult {
    pub toolpath: Toolpath,
    /// Places it could not reach within the engagement limit (left as rest material).
    pub skipped: usize,
    /// The grid cell it planned on (mm): engagement is measured at this depth.
    pub cell: f64,
}

/// Samples round the tool's circumference for the engagement (one per degree).
pub const ENGAGEMENT_SAMPLES: usize = 360;

/// The engagement angle of a tool of radius `radius` arriving at `c`, against the
/// material grid before it cuts there: the angle of its circumference (taken `depth`
/// inside it; 0 for the circumference itself) over material. Measured after a step `s`
/// from the last position, the circumference's points outside the last position's disk
/// are its leading half (to within `s / 2R` radians), so this is the arc in contact with
/// the material the step cuts.
pub fn engagement(g: &Grid, material: &[bool], c: Point2, radius: f64, depth: f64) -> f64 {
    let rr = (radius - depth).max(0.0);
    let mut n = 0usize;
    for k in 0..ENGAGEMENT_SAMPLES {
        let a = TAU * k as f64 / ENGAGEMENT_SAMPLES as f64;
        let p = Point2::new(c.x + rr * a.cos(), c.y + rr * a.sin());
        if let Some((i, j)) = g.nearest(p)
            && material.get(g.index(i, j)).copied().unwrap_or(false)
        {
            n += 1;
        }
    }
    TAU * n as f64 / ENGAGEMENT_SAMPLES as f64
}

/// Clears the material nodes within `radius` of `c`, logging each into `undo`.
fn carve(g: &Grid, material: &mut [bool], c: Point2, radius: f64, undo: &mut Vec<usize>) {
    let n = (radius / g.cell).ceil() as i64 + 1;
    let Some((ci, cj)) = g.nearest(c).or_else(|| {
        // Off the grid: carve only what the grid holds.
        let fi = ((c.x - g.min.x) / g.cell)
            .round()
            .clamp(0.0, (g.nx - 1) as f64) as usize;
        let fj = ((c.y - g.min.y) / g.cell)
            .round()
            .clamp(0.0, (g.ny - 1) as f64) as usize;
        Some((fi, fj))
    }) else {
        return;
    };
    for dj in -n..=n {
        for di in -n..=n {
            let (i, j) = (ci as i64 + di, cj as i64 + dj);
            if i < 0 || j < 0 || i >= g.nx as i64 || j >= g.ny as i64 {
                continue;
            }
            let (i, j) = (i as usize, j as usize);
            if g.at(i, j).distance(c) <= radius {
                let k = g.index(i, j);
                if material.get(k).copied().unwrap_or(false) {
                    material[k] = false;
                    undo.push(k);
                }
            }
        }
    }
}

/// Any material within `radius - depth` of `c` (a plunge there would cut it).
fn any_material(g: &Grid, material: &[bool], c: Point2, radius: f64) -> bool {
    let n = (radius / g.cell).ceil() as i64 + 1;
    let Some((ci, cj)) = g.nearest(c) else {
        return false;
    };
    for dj in -n..=n {
        for di in -n..=n {
            let (i, j) = (ci as i64 + di, cj as i64 + dj);
            if i < 0 || j < 0 || i >= g.nx as i64 || j >= g.ny as i64 {
                continue;
            }
            let (i, j) = (i as usize, j as usize);
            if g.at(i, j).distance(c) <= radius && material[g.index(i, j)] {
                return true;
            }
        }
    }
    false
}

/// Adaptive (bounded-engagement) roughing of the stock minus the part, level by level.
///
/// A simple, robust version of constant-engagement clearing (after Stori and Wright,
/// "Constant engagement tool path generation for convex geometries", J. Manuf. Systems
/// 19(3), 2000, and Ibaraki et al. on engagement-controlled pocketing), not a
/// medial-axis method:
/// 1. At each level the tool may go where its tip, at that height, stays clear of the
///    part (the cutter-location heights over a grid, exact at the nodes). Material is the
///    stock above the part at that height.
/// 2. Passes are offsets of the forbidden region (the iso-lines of the Euclidean distance
///    to it), `a_e` apart where a straight cut of radial depth `a_e` engages
///    `acos(1 - a_e / R)`, set to 80 % of the limit; they are cut from the farthest
///    (out in the air, or a closed pocket's middle) to the part.
/// 3. The engagement (the angle of the tool's circumference over uncut material, see
///    [`engagement`]) is measured along every step against the material left so far.
///    Where a step would exceed the limit the tool goes there by a trochoidal slot
///    instead (circles advancing a little each turn, the advance halved until every point
///    is within the limit). Where even that fails, the place is left uncut (rest
///    material, counted in `skipped`) and the tool lifts and comes back down where it can.
/// 4. A closed pocket is entered by a helix; ramps, helices and plunges are not counted
///    as engagement (their load is limited by the ramp angle instead).
///
/// Limits: engagement is measured on the planning grid (one cell deep); offsets come from
/// a sampled distance field, so passes are within a cell of the exact offsets; the part
/// is kept clear exactly at the grid's nodes and, near the part, at every emitted point
/// (checked with the exact drop-cutter).
pub fn adaptive(cl: &Cl, cutter: &Cutter, p: AdaptiveParams) -> Result<AdaptiveResult, CamError> {
    adaptive_with(cl, cutter, p, None)
}

/// [`adaptive`] with a holder check: a node where the holder or shank would hit the part
/// at a level is not entered at that level (`Lift` and `Refuse` both leave it uncut
/// there). Checked at the planning grid's nodes against the part, not the stock.
pub fn adaptive_with(
    cl: &Cl,
    cutter: &Cutter,
    p: AdaptiveParams,
    holder: Option<HolderUse>,
) -> Result<AdaptiveResult, CamError> {
    let r = cl.tool.radius;
    if !(p.max_engagement.is_finite() && p.max_engagement > 0.05 && p.max_engagement <= PI) {
        return Err(CamError::Invalid(
            "the engagement limit must be from 3 to 180 degrees".into(),
        ));
    }
    let margin = r + 2.0;
    let gmin = Point2::new(p.stock_min.x - margin, p.stock_min.y - margin);
    let gmax = Point2::new(p.stock_max.x + margin, p.stock_max.y + margin);
    let g = Grid::covering(gmin, gmax, r / 16.0, grid::MAX_GRID_SIDE, r / 4.0)?;
    let cell = g.cell;
    let field = cl_grid(cl, &g)?;
    // The part's own height at each node (a pin-point drop): material is stock above it.
    let pin = ToolShape::new(1e-6, 0.0)?;
    let part: Vec<f64> = (0..g.len())
        .map(|k| {
            let q = g.at(k % g.nx, k / g.nx);
            cl.model.drop(&pin, q.x, q.y).unwrap_or(f64::NEG_INFINITY)
        })
        .collect();
    // The lowest tip height at each node at which the holder clears the part.
    let holder_need: Option<Vec<f64>> = match holder.filter(|h| h.mode != HolderCheck::Ignore) {
        None => None,
        Some(h) => {
            let mut v = Vec::with_capacity(g.len());
            for k in 0..g.len() {
                if k.is_multiple_of(g.nx.max(1)) {
                    cancel::check()?;
                }
                let q = g.at(k % g.nx, k / g.nx);
                v.push(h.check.required(cl.model, q.x, q.y));
            }
            Some(v)
        }
    };
    let in_stock = |q: Point2| {
        q.x >= p.stock_min.x && q.x <= p.stock_max.x && q.y >= p.stock_min.y && q.y <= p.stock_max.y
    };
    let plan_limit = 0.8 * p.max_engagement;
    let ae = (r * (1.0 - plan_limit.min(PI / 2.0).cos())).clamp(cell, 2.0 * r);
    let levels = depths(p.stock_top, p.bottom, p.stepdown)?;
    if levels.len() > MAX_LEVELS {
        return Err(CamError::TooComplex(format!(
            "more than {MAX_LEVELS} levels"
        )));
    }
    let mut m: Vec<Move> = Vec::new();
    let mut skipped = 0usize;
    let mut budget = MAX_DROPS.saturating_sub(2 * g.len());
    let mut prev_z = p.stock_top;
    for z in levels {
        cancel::check()?;
        let allowed: Vec<bool> = field
            .iter()
            .enumerate()
            .map(|(k, c)| {
                *c <= z + 1e-9
                    && holder_need
                        .as_ref()
                        .is_none_or(|h| h.get(k).is_none_or(|need| *need <= z + 1e-9))
            })
            .collect();
        let mut material: Vec<bool> = (0..g.len())
            .map(|k| {
                let q = g.at(k % g.nx, k / g.nx);
                in_stock(q) && part.get(k).is_some_and(|h| *h < z - 1e-9)
            })
            .collect();
        if !material.iter().any(|x| *x) {
            prev_z = z;
            continue;
        }
        let mut forbidden: Vec<bool> = allowed.iter().map(|a| !a).collect();
        if !forbidden.iter().any(|x| *x) {
            // Nothing of the part at this level: offsets of the stock's centre.
            let c = Point2::new(
                (p.stock_min.x + p.stock_max.x) / 2.0,
                (p.stock_min.y + p.stock_max.y) / 2.0,
            );
            if let Some((i, j)) = g.nearest(c) {
                forbidden[g.index(i, j)] = true;
            }
        }
        let dist = grid::distance_transform(&g, &forbidden);
        // The farthest offset that still reaches material.
        let dmax = (0..g.len())
            .filter(|k| material[*k])
            .map(|k| dist[k])
            .filter(|d| d.is_finite())
            .fold(0.0f64, f64::max)
            + r;
        let kmax = (dmax / ae).ceil() as usize;
        let mut at: Option<Point2> = None;
        let mut lifted = true;
        let mut lin = |a: Point2, b: Point2, va: f64, vb: f64| {
            let t = if (va - vb).abs() > 1e-300 {
                ((va) / (va - vb)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            a + (b - a) * t
        };
        for k in (0..=kmax).rev() {
            // At least a cell off the part, so interpolated points stay clear of it.
            let d = (k as f64 * ae).max(cell);
            let level: Vec<f64> = dist.iter().map(|x| d - x.min(1e9)).collect();
            let mut loops = grid::contours(&g, &level, 0.0, &mut lin);
            // Nearest loop first.
            while !loops.is_empty() {
                let here = at.unwrap_or(gmin);
                let (li, start) = loops
                    .iter()
                    .enumerate()
                    .flat_map(|(i, c)| c.points.iter().enumerate().map(move |(s, q)| (i, s, *q)))
                    .min_by(|a, b| a.2.distance(here).total_cmp(&b.2.distance(here)))
                    .map_or((0, 0), |x| (x.0, x.1));
                let c = loops.swap_remove(li);
                let mut route: Vec<Point2> = if c.closed {
                    // Joined a tool diameter along from the nearest point, so the step in
                    // from the last pass is spread over that length (a cut whose depth
                    // grows from 0 to a_e engages no more than the pass itself).
                    let n = c.points.len();
                    let mut j = start;
                    let mut run = 0.0;
                    for s in 1..n {
                        let (Some(a), Some(b)) = (
                            c.points.get((start + s - 1) % n),
                            c.points.get((start + s) % n),
                        ) else {
                            break;
                        };
                        run += a.distance(*b);
                        j = (start + s) % n;
                        if run >= 2.0 * r {
                            break;
                        }
                    }
                    if run < 2.0 * r {
                        j = start;
                    }
                    (0..=n)
                        .filter_map(|s| c.points.get((j + s) % n).copied())
                        .collect()
                } else {
                    let (Some(a), Some(b)) = (c.points.first(), c.points.last()) else {
                        continue;
                    };
                    if a.distance(here) <= b.distance(here) {
                        c.points.clone()
                    } else {
                        c.points.iter().rev().copied().collect()
                    }
                };
                route = densify(&route, cell / 2.0);
                // Skip loops that never come near material.
                if !route.iter().any(|q| any_material(&g, &material, *q, r)) {
                    continue;
                }
                let mut ctx = Walk {
                    g: &g,
                    material: &mut material,
                    allowed: &allowed,
                    dist: &dist,
                    cl,
                    budget: &mut budget,
                    r,
                    z,
                    prev_z,
                    limit: p.max_engagement * 0.95,
                    ae,
                    cutter,
                    clearance: p.clearance,
                    m: &mut m,
                    at: &mut at,
                    lifted: &mut lifted,
                    skipped: &mut skipped,
                };
                ctx.follow(&route)?;
            }
        }
        if let Some(a) = at {
            m.push(Move::Rapid {
                to: Point3::new(a.x, a.y, p.clearance),
            });
        }
        prev_z = z;
    }
    Ok(AdaptiveResult {
        toolpath: cutter_path(cutter, m)?,
        skipped,
        cell,
    })
}

/// Points along `route` at most `step` apart.
fn densify(route: &[Point2], step: f64) -> Vec<Point2> {
    let mut out = Vec::new();
    for w in route.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((a.distance(b) / step).ceil() as usize).clamp(1, 100_000);
        if out.is_empty() {
            out.push(a);
        }
        for i in 1..=n {
            out.push(a + (b - a) * (i as f64 / n as f64));
        }
    }
    if out.is_empty() {
        out.extend(route.first());
    }
    out
}

/// One level's walk along the planned passes.
struct Walk<'a, 'b> {
    g: &'a Grid,
    material: &'a mut Vec<bool>,
    allowed: &'a [bool],
    dist: &'a [f64],
    cl: &'a Cl<'b>,
    budget: &'a mut usize,
    r: f64,
    z: f64,
    prev_z: f64,
    limit: f64,
    ae: f64,
    cutter: &'a Cutter,
    clearance: f64,
    m: &'a mut Vec<Move>,
    at: &'a mut Option<Point2>,
    lifted: &'a mut bool,
    skipped: &'a mut usize,
}

impl Walk<'_, '_> {
    fn p3(&self, q: Point2) -> Point3 {
        Point3::new(q.x, q.y, self.z)
    }

    /// The tool may be at `q` at this level: allowed at the nearest node and, near the
    /// part, by the exact drop-cutter.
    fn clear(&mut self, q: Point2) -> bool {
        let Some((i, j)) = self.g.nearest(q) else {
            return true;
        };
        let k = self.g.index(i, j);
        if !self.allowed.get(k).copied().unwrap_or(false) {
            return false;
        }
        if self.dist.get(k).copied().unwrap_or(f64::INFINITY) < 2.0 * self.g.cell + self.r {
            if *self.budget == 0 {
                return false;
            }
            *self.budget -= 1;
            return self.cl.at(q.x, q.y) <= self.z + 1e-9;
        }
        true
    }

    fn engagement(&self, q: Point2) -> f64 {
        engagement(self.g, self.material, q, self.r, 0.0)
    }

    fn line(&mut self, q: Point2) {
        let mut undo = Vec::new();
        carve(self.g, self.material, q, self.r, &mut undo);
        self.m.push(Move::Line {
            to: self.p3(q),
            feed: self.cutter.feed,
        });
        *self.at = Some(q);
    }

    /// Goes down at `q`: by a helix where there is material under the tool, else
    /// straight down (in the air or over cut material).
    fn enter(&mut self, q: Point2) {
        if let Some(a) = *self.at {
            self.m.push(Move::Rapid {
                to: Point3::new(a.x, a.y, self.clearance),
            });
        }
        self.m.push(Move::Rapid {
            to: Point3::new(q.x, q.y, self.clearance),
        });
        let above = (self.prev_z + FEED_HEIGHT).min(self.clearance);
        let full = any_material(self.g, self.material, q, self.r);
        let room = self
            .g
            .nearest(q)
            .and_then(|(i, j)| self.dist.get(self.g.index(i, j)).copied())
            .unwrap_or(0.0);
        let rh = (self.r / 2.0).min(room - self.g.cell);
        let mut undo = Vec::new();
        if full && rh >= self.g.cell {
            // A helix of radius rh about q, 3 degrees down, from above the last level.
            let start = Point2::new(q.x + rh, q.y);
            self.m.push(Move::Rapid {
                to: Point3::new(start.x, start.y, above),
            });
            let per_turn = TAU * rh * 3f64.to_radians().tan();
            let turns = (((above - self.z) / per_turn).ceil() as usize).clamp(1, 1000);
            let dz = (above - self.z) / turns as f64;
            for t in 0..turns {
                let z0 = above - dz * t as f64;
                let half = Point2::new(q.x - rh, q.y);
                self.m.push(Move::Arc {
                    to: Point3::new(half.x, half.y, z0 - dz / 2.0),
                    centre: q,
                    clockwise: false,
                    feed: self.cutter.plunge_feed,
                });
                self.m.push(Move::Arc {
                    to: Point3::new(start.x, start.y, z0 - dz),
                    centre: q,
                    clockwise: false,
                    feed: self.cutter.plunge_feed,
                });
            }
            carve(self.g, self.material, q, rh + self.r, &mut undo);
            self.m.push(Move::Line {
                to: self.p3(q),
                feed: self.cutter.feed,
            });
        } else {
            self.m.push(Move::Rapid {
                to: Point3::new(q.x, q.y, above),
            });
            self.m.push(Move::Plunge {
                to: self.p3(q),
                feed: self.cutter.plunge_feed,
            });
            carve(self.g, self.material, q, self.r, &mut undo);
        }
        *self.at = Some(q);
        *self.lifted = false;
    }

    /// A trochoidal slot from the current point to `to`: circles of radius `rho`
    /// advancing `adv` per turn. Every point is checked; on any point over the limit
    /// (or into the part) the material is restored and `false` returned.
    fn trochoid(&mut self, to: Point2) -> bool {
        let Some(from) = *self.at else { return false };
        let len = from.distance(to);
        let Some(t) = (to - from).normalized() else {
            return true;
        };
        let n = t.perp();
        let mut rho = (0.5 * self.r).max(self.g.cell);
        while rho >= self.g.cell {
            let mut adv = self.ae;
            while adv >= self.g.cell / 2.0 {
                let turns = (len / adv).ceil().max(1.0);
                if turns > 2000.0 {
                    break;
                }
                let steps_per_turn =
                    ((TAU * rho / (self.g.cell / 2.0)).ceil() as usize).clamp(16, 2000);
                let total = turns as usize * steps_per_turn;
                let mut pts = Vec::with_capacity(total + 2);
                let mut undo = Vec::new();
                let mut ok = true;
                // Centre c(φ) = from - ρ n + t (len φ / Φ); point = c + ρ (n cos φ + t sin φ).
                for s in 1..=total {
                    let phi = TAU * s as f64 / steps_per_turn as f64;
                    let frac = s as f64 / total as f64;
                    let c = from - n * rho + t * (len * frac);
                    let q = c + n * (rho * phi.cos()) + t * (rho * phi.sin());
                    if !self.clear(q) || self.engagement(q) > self.limit {
                        ok = false;
                        break;
                    }
                    carve(self.g, self.material, q, self.r, &mut undo);
                    pts.push(q);
                }
                // The last point is `to` - ρ n + ρ n = `to` (φ a whole number of turns).
                if ok && (!self.clear(to) || self.engagement(to) > self.limit) {
                    ok = false;
                }
                if ok {
                    carve(self.g, self.material, to, self.r, &mut undo);
                    for q in pts {
                        self.m.push(Move::Line {
                            to: self.p3(q),
                            feed: self.cutter.feed,
                        });
                    }
                    self.m.push(Move::Line {
                        to: self.p3(to),
                        feed: self.cutter.feed,
                    });
                    *self.at = Some(to);
                    return true;
                }
                for k in undo {
                    if let Some(x) = self.material.get_mut(k) {
                        *x = true;
                    }
                }
                adv /= 2.0;
            }
            rho /= 2.0;
        }
        false
    }

    fn lift(&mut self) {
        if let Some(a) = *self.at
            && !*self.lifted
        {
            self.m.push(Move::Rapid {
                to: Point3::new(a.x, a.y, self.clearance),
            });
        }
        *self.lifted = true;
    }

    fn follow(&mut self, route: &[Point2]) -> Result<(), CamError> {
        cancel::check()?;
        if self.m.len() > crate::ops::MAX_MOVES {
            return Err(CamError::TooComplex(
                "the toolpath has too many moves".into(),
            ));
        }
        let mut i = 0;
        while i < route.len() {
            let q = route[i];
            if !self.clear(q) {
                self.lift();
                i += 1;
                continue;
            }
            if *self.lifted {
                // Come down only where the tool's circle is within the limit.
                if self.engagement(q) <= self.limit {
                    self.enter(q);
                } else {
                    i += 1;
                    if any_material(self.g, self.material, q, self.r) {
                        *self.skipped += 1;
                    }
                    continue;
                }
                i += 1;
                continue;
            }
            // Linking from where the tool is: a straight feed if the way is clear and
            // short, else lift.
            let from = self.at.unwrap_or(q);
            if from.distance(q) > 2.0 * self.ae + self.g.cell {
                let way = densify(&[from, q], self.g.cell / 2.0);
                if way.iter().all(|w| self.clear(*w)) {
                    for w in way.into_iter().skip(1) {
                        self.step(w)?;
                    }
                } else {
                    self.lift();
                    continue;
                }
                i += 1;
                continue;
            }
            if self.engagement(q) <= self.limit {
                self.line(q);
                i += 1;
                continue;
            }
            // Over the limit: a trochoidal slot to a point up to a tool diameter ahead,
            // else to the next point, else leave it.
            let mut j = i;
            while j + 1 < route.len() && route[j + 1].distance(from) <= 2.0 * self.r {
                j += 1;
            }
            if self.trochoid(route[j]) {
                i = j + 1;
            } else if j > i && self.trochoid(q) {
                i += 1;
            } else {
                *self.skipped += 1;
                self.lift();
                i += 1;
            }
        }
        Ok(())
    }

    /// One step to `q`: straight if within the limit, else a trochoidal slot, else left.
    fn step(&mut self, q: Point2) -> Result<(), CamError> {
        if *self.lifted {
            self.enter(q);
            return Ok(());
        }
        if !self.clear(q) {
            self.lift();
            return Ok(());
        }
        if self.engagement(q) <= self.limit {
            self.line(q);
            return Ok(());
        }
        // Over the limit: a trochoidal slot to a point a little ahead.
        if self.trochoid(q) {
            return Ok(());
        }
        *self.skipped += 1;
        self.lift();
        Ok(())
    }
}

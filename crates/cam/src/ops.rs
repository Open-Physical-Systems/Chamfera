//! The 2.5D operations: facing, contour, pocket and drilling, from plain geometry.
//!
//! Each takes exact loops of lines and arcs and heights, and returns a [`Toolpath`] in
//! model coordinates. Pocketing is contour-parallel (Held, "On the Computational
//! Geometry of Pocket Machining", 1991, ch. 1 and 6): offsets of the boundary at the tool
//! radius plus whole stepovers, cut from the innermost out, so the last pass at each depth
//! is the wall offset.

use crate::CamError;
use crate::offset::{offset_left, offset_right, start_lowest};
use crate::path2::{
    JOIN_TOLERANCE, Loop, Seg, loop_length, nearest_point, reverse_loop, signed_area, start_nearest,
};
use crate::toolpath::{Move, Toolpath};
use chamfera_doc::cam::{Entry, Side, Tool};
use chamfera_geom::{Point2, Point3};
use std::f64::consts::{PI, TAU};

/// How far past the stock's edge a facing pass runs, beyond the tool radius (mm), so the
/// tool is clear of the stock where it steps over.
pub const FACING_OVERRUN: f64 = 1.0;
/// How far above the stock top (or above the last floor cut) the tool comes down at rapid
/// before it feeds (mm).
pub const FEED_HEIGHT: f64 = 1.0;
/// Most depth levels one operation may cut (hostile-input limit).
pub const MAX_DEPTHS: usize = 1_000;
/// Most offset levels one pocket may have (hostile-input limit).
pub const MAX_POCKET_LEVELS: usize = 2_000;
/// Most passes one facing depth may have.
pub const MAX_FACING_PASSES: usize = 100_000;
/// Most moves one toolpath may hold.
pub const MAX_MOVES: usize = 2_000_000;

/// The tool's speeds and size, as the generators use them.
#[derive(Debug, Clone, PartialEq)]
pub struct Cutter {
    pub number: u32,
    pub name: String,
    pub diameter: f64,
    pub spindle_rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
}

impl From<&Tool> for Cutter {
    fn from(t: &Tool) -> Cutter {
        Cutter {
            number: t.number,
            name: t.name.clone(),
            diameter: t.diameter,
            spindle_rpm: t.spindle_rpm,
            feed: t.feed,
            plunge_feed: t.plunge_feed,
        }
    }
}

impl Cutter {
    fn radius(&self) -> f64 {
        self.diameter / 2.0
    }

    fn check(&self) -> Result<(), CamError> {
        let ok = |v: f64| v.is_finite() && v > 0.0;
        if !(ok(self.diameter) && ok(self.feed) && ok(self.plunge_feed) && ok(self.spindle_rpm)) {
            return Err(CamError::Invalid(
                "the tool needs a positive diameter, feeds and spindle speed".into(),
            ));
        }
        Ok(())
    }

    fn path(&self, moves: Vec<Move>) -> Result<Toolpath, CamError> {
        if moves.len() > MAX_MOVES {
            return Err(CamError::TooComplex(format!(
                "the toolpath would have more than {MAX_MOVES} moves"
            )));
        }
        Ok(Toolpath {
            tool_number: self.number,
            tool_name: self.name.clone(),
            spindle_rpm: self.spindle_rpm,
            moves,
        })
    }
}

fn p3(p: Point2, z: f64) -> Point3 {
    Point3::new(p.x, p.y, z)
}

fn finite(what: &str, v: f64) -> Result<f64, CamError> {
    if v.is_finite() && v.abs() <= chamfera_geom::MAX_COORDINATE {
        Ok(v)
    } else {
        Err(CamError::Invalid(format!("{what} must be a finite number")))
    }
}

/// The depths cut from `top` down to `bottom`, at most `stepdown` apart, in equal steps.
/// One depth at `bottom` when there is nothing above it to remove.
pub fn depths(top: f64, bottom: f64, stepdown: f64) -> Result<Vec<f64>, CamError> {
    finite("the top", top)?;
    finite("the bottom", bottom)?;
    if !(stepdown.is_finite() && stepdown > 0.0) {
        return Err(CamError::Invalid("the stepdown must be positive".into()));
    }
    let h = top - bottom;
    if h <= JOIN_TOLERANCE {
        return Ok(vec![bottom]);
    }
    let n = (h / stepdown - 1e-9).ceil().max(1.0);
    if n > MAX_DEPTHS as f64 {
        return Err(CamError::TooComplex(format!(
            "a stepdown of {stepdown} mm over {h} mm makes more than {MAX_DEPTHS} depths"
        )));
    }
    let n = n as usize;
    Ok((1..=n)
        .map(|i| {
            if i == n {
                bottom
            } else {
                top - h * i as f64 / n as f64
            }
        })
        .collect())
}

/// Moves along `s` from height `z0` to `z1` (linear in length): a line or an arc, a full
/// circle split in two halves so every arc posts with distinct ends.
fn seg_moves(s: &Seg, z0: f64, z1: f64, feed: f64, out: &mut Vec<Move>) {
    match *s {
        Seg::Line { b, .. } => out.push(Move::Line {
            to: p3(b, z1),
            feed,
        }),
        Seg::Arc { c, sweep, .. } => {
            let parts: &[(f64, f64)] = if sweep.abs() > PI + 1e-9 {
                &[(0.0, 0.5), (0.5, 1.0)]
            } else {
                &[(0.0, 1.0)]
            };
            for &(t0, t1) in parts {
                let piece = s.sub(t0, t1);
                out.push(Move::Arc {
                    to: p3(piece.end(), z0 + (z1 - z0) * t1),
                    centre: c,
                    clockwise: sweep < 0.0,
                    feed,
                });
            }
        }
    }
}

/// A closed loop cut at height `z`.
fn loop_moves(l: &[Seg], z: f64, feed: f64, out: &mut Vec<Move>) {
    for s in l {
        seg_moves(s, z, z, feed, out);
    }
}

/// Facing: zigzag passes along X over the stock top, `stepover` (a fraction of the tool
/// diameter) apart at most, in depths of `stepdown` from `top` down to `bottom`.
///
/// The tool centre runs along both Y edges of the stock and at equal steps between them:
/// for a stock `w` wide, `n = ceil(w / stepover)` steps of `w / n`, so `n + 1` passes.
/// Each pass runs from `FACING_OVERRUN + r` before the stock to as far past it.
#[allow(clippy::too_many_arguments)]
pub fn facing(
    stock_min: Point2,
    stock_max: Point2,
    top: f64,
    bottom: f64,
    tool: &Cutter,
    stepover: f64,
    stepdown: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    tool.check()?;
    if !(stepover.is_finite() && stepover > 0.0 && stepover <= 1.0) {
        return Err(CamError::Invalid(
            "the stepover is a fraction from 0 to 1".into(),
        ));
    }
    for v in [
        stock_min.x,
        stock_min.y,
        stock_max.x,
        stock_max.y,
        clearance,
    ] {
        finite("the stock", v)?;
    }
    let w = stock_max.y - stock_min.y;
    if !(w > 0.0 && stock_max.x > stock_min.x) {
        return Err(CamError::Invalid("the stock has no size".into()));
    }
    let step = stepover * tool.diameter;
    let n = (w / step - 1e-9).ceil().max(1.0);
    if n > MAX_FACING_PASSES as f64 {
        return Err(CamError::TooComplex(
            "the stepover is too small for this stock".into(),
        ));
    }
    let n = n as usize;
    let dy = w / n as f64;
    let reach = tool.radius() + FACING_OVERRUN;
    let (x0, x1) = (stock_min.x - reach, stock_max.x + reach);
    let mut m = vec![Move::Rapid {
        to: Point3::new(x0, stock_min.y, clearance),
    }];
    for z in depths(top, bottom, stepdown)? {
        m.push(Move::Rapid {
            to: Point3::new(x0, stock_min.y, clearance),
        });
        m.push(Move::Rapid {
            to: Point3::new(x0, stock_min.y, top + FEED_HEIGHT),
        });
        m.push(Move::Plunge {
            to: Point3::new(x0, stock_min.y, z),
            feed: tool.plunge_feed,
        });
        let mut at_x0 = true;
        for k in 0..=n {
            let y = if k == n {
                stock_max.y
            } else {
                stock_min.y + dy * k as f64
            };
            if k > 0 {
                let x = if at_x0 { x0 } else { x1 };
                m.push(Move::Line {
                    to: Point3::new(x, y, z),
                    feed: tool.feed,
                });
            }
            let x = if at_x0 { x1 } else { x0 };
            m.push(Move::Line {
                to: Point3::new(x, y, z),
                feed: tool.feed,
            });
            at_x0 = !at_x0;
        }
        let x = if at_x0 { x0 } else { x1 };
        m.push(Move::Rapid {
            to: Point3::new(x, stock_max.y, clearance),
        });
        if m.len() > MAX_MOVES {
            break;
        }
    }
    tool.path(m)
}

/// The loop counter-clockwise.
fn ccw(l: &[Seg]) -> Loop {
    if signed_area(l) < 0.0 {
        reverse_loop(l)
    } else {
        l.to_vec()
    }
}

/// Contour: the tool runs round `boundary` (a closed loop at height `boundary_z`) at its
/// radius from it, on `side` of the counter-clockwise loop (right is outside), from `top`
/// down to `boundary_z - bottom_offset` in depths of at most `stepdown`.
///
/// With `lead_radius > 0` each pass starts and ends with a quarter-circle arc of that
/// radius, tangent to the path, on the side away from the material.
#[allow(clippy::too_many_arguments)]
pub fn contour(
    boundary: &[Seg],
    boundary_z: f64,
    side: Side,
    climb: bool,
    tool: &Cutter,
    top: f64,
    stepdown: f64,
    lead_radius: f64,
    bottom_offset: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    tool.check()?;
    finite("the clearance height", clearance)?;
    let b = ccw(boundary);
    let paths = match side {
        Side::Right => offset_right(&[b], tool.radius())?,
        Side::Left => offset_left(&[b], tool.radius())?,
    };
    if paths.is_empty() {
        return Err(CamError::Refused(format!(
            "a {} mm tool does not fit inside this boundary",
            tool.diameter
        )));
    }
    // Climb milling with a clockwise spindle keeps the tool on the left of its travel.
    let reverse = (side == Side::Right) == climb;
    let paths: Vec<Loop> = paths
        .into_iter()
        .map(|l| {
            if reverse {
                start_lowest(&reverse_loop(&l))
            } else {
                l
            }
        })
        .collect();
    let bottom = boundary_z - bottom_offset;
    let zs = depths(top.max(bottom), bottom, stepdown)?;
    let first = paths
        .first()
        .and_then(|l| l.first())
        .map_or(Point2::ORIGIN, Seg::start);
    let mut m = vec![Move::Rapid {
        to: p3(first, clearance),
    }];
    for l in &paths {
        let (Some(s0), Some(last)) = (l.first(), l.last()) else {
            continue;
        };
        let p0 = s0.start();
        let t0 = s0.tangent_at(0.0);
        let t1 = last.tangent_at(1.0);
        // Away from the material: left of travel when climbing, right otherwise.
        let away = |t: chamfera_geom::Vec2| if climb { t.perp() } else { -t.perp() };
        let lead = lead_radius > JOIN_TOLERANCE;
        let c_in = p0 + away(t0) * lead_radius;
        let s_in = c_in - t0 * lead_radius;
        let c_out = p0 + away(t1) * lead_radius;
        let e_out = c_out + t1 * lead_radius;
        let entry = if lead { s_in } else { p0 };
        let mut prev = top.max(bottom);
        for &z in &zs {
            m.push(Move::Rapid {
                to: p3(entry, clearance),
            });
            m.push(Move::Rapid {
                to: p3(entry, prev.min(top) + FEED_HEIGHT),
            });
            m.push(Move::Plunge {
                to: p3(entry, z),
                feed: tool.plunge_feed,
            });
            if lead {
                m.push(Move::Arc {
                    to: p3(p0, z),
                    centre: c_in,
                    clockwise: !climb,
                    feed: tool.feed,
                });
            }
            loop_moves(l, z, tool.feed, &mut m);
            let end = if lead {
                m.push(Move::Arc {
                    to: p3(e_out, z),
                    centre: c_out,
                    clockwise: !climb,
                    feed: tool.feed,
                });
                e_out
            } else {
                p0
            };
            m.push(Move::Rapid {
                to: p3(end, clearance),
            });
            prev = z;
        }
    }
    tool.path(m)
}

/// A flat-bottomed region: the outer boundary and the islands left standing in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub outer: Loop,
    pub islands: Vec<Loop>,
}

/// The offset passes of a pocket, innermost first: (distance from the walls, loop).
pub fn pocket_passes(
    region: &Region,
    tool_radius: f64,
    step: f64,
) -> Result<Vec<(f64, Loop)>, CamError> {
    if !(tool_radius > 0.0 && step > 0.0 && tool_radius.is_finite() && step.is_finite()) {
        return Err(CamError::Invalid(
            "the tool radius and stepover must be positive".into(),
        ));
    }
    let mut boundary = vec![ccw(&region.outer)];
    for i in &region.islands {
        boundary.push(reverse_loop(&ccw(i)));
    }
    let mut levels: Vec<(f64, Vec<Loop>)> = Vec::new();
    for k in 0..=MAX_POCKET_LEVELS {
        let d = tool_radius + step * k as f64;
        let loops = offset_left(&boundary, d)?;
        if loops.is_empty() {
            // A stepover wider than the radius could leave a strip uncut along the middle;
            // half a step further in covers it (every point is then within r of a pass).
            if step > tool_radius && k > 0 {
                let extra = offset_left(&boundary, d - step / 2.0)?;
                if !extra.is_empty() {
                    levels.push((d - step / 2.0, extra));
                }
            }
            break;
        }
        if k == MAX_POCKET_LEVELS {
            return Err(CamError::TooComplex(format!(
                "the stepover is too small: more than {MAX_POCKET_LEVELS} passes"
            )));
        }
        levels.push((d, loops));
    }
    if levels.is_empty() {
        return Err(CamError::Refused(format!(
            "a {} mm tool does not fit in this pocket",
            tool_radius * 2.0
        )));
    }
    let mut out = Vec::new();
    for (d, loops) in levels.into_iter().rev() {
        for l in loops {
            out.push((d, l));
        }
    }
    Ok(out)
}

/// Pocket: contour-parallel passes over `region` (walls vertical, floor at `floor_z`),
/// from `top` down to the floor in depths of at most `stepdown`. `stepover` is a fraction
/// of the tool diameter. Each depth starts at the innermost pass with the `entry` move;
/// a pass is joined to the next with a straight cut when that cut provably stays at least
/// the tool radius from every wall (the distance to the walls changes by at most the
/// length moved), and otherwise the tool lifts and comes down again.
#[allow(clippy::too_many_arguments)]
pub fn pocket(
    region: &Region,
    floor_z: f64,
    top: f64,
    tool: &Cutter,
    stepover: f64,
    stepdown: f64,
    entry: Entry,
    ramp_angle: f64,
    helix_radius: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    tool.check()?;
    finite("the clearance height", clearance)?;
    if !(stepover.is_finite() && stepover > 0.0 && stepover <= 1.0) {
        return Err(CamError::Invalid(
            "the stepover is a fraction from 0 to 1".into(),
        ));
    }
    if !(ramp_angle.is_finite() && ramp_angle > 0.0 && ramp_angle < PI / 2.0) {
        return Err(CamError::Invalid(
            "the ramp angle must be between 0 and 90 degrees".into(),
        ));
    }
    let r = tool.radius();
    let passes = pocket_passes(region, r, stepover * tool.diameter)?;
    // The cutting order: innermost level first, nearest loop next within it. Each entry
    // says whether it is reached by a straight cut from the one before.
    let mut order: Vec<(f64, Loop, bool)> = Vec::new();
    let mut remaining = passes;
    let mut at: Option<(Point2, f64)> = None;
    while !remaining.is_empty() {
        let level = remaining.first().map_or(0.0, |p| p.0);
        let pick = remaining
            .iter()
            .enumerate()
            .filter(|(_, p)| (p.0 - level).abs() <= JOIN_TOLERANCE)
            .map(|(i, p)| {
                let d = at
                    .and_then(|(q, _)| nearest_point(&p.1, q).map(|n| n.1))
                    .unwrap_or(0.0);
                (i, d)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |x| x.0);
        let (d, l) = remaining.remove(pick);
        let (l, linked) = match at {
            Some((q, d_from)) => match nearest_point(&l, q) {
                Some((n, len)) if len <= d_from + d - 2.0 * r - JOIN_TOLERANCE => {
                    (start_nearest(&l, n), true)
                }
                _ => (l, false),
            },
            None => (l, false),
        };
        at = l.first().map(|s| (s.start(), d));
        order.push((d, l, linked));
    }
    let first = order
        .first()
        .and_then(|o| o.1.first())
        .map_or(Point2::ORIGIN, Seg::start);
    let mut m = vec![Move::Rapid {
        to: p3(first, clearance),
    }];
    let top = top.max(floor_z);
    let retract = top + FEED_HEIGHT;
    let mut prev = top;
    for z in depths(top, floor_z, stepdown)? {
        let from = prev + FEED_HEIGHT;
        let mut down = false;
        for (d, l, linked) in &order {
            let Some(p0) = l.first().map(Seg::start) else {
                continue;
            };
            if down && *linked {
                m.push(Move::Line {
                    to: p3(p0, z),
                    feed: tool.feed,
                });
            } else {
                if down && let Some(at) = last_point(&m) {
                    m.push(Move::Rapid {
                        to: Point3::new(at.x, at.y, retract),
                    });
                }
                enter(
                    l,
                    p0,
                    *d - r,
                    z,
                    from,
                    retract,
                    tool,
                    entry,
                    ramp_angle,
                    helix_radius,
                    &mut m,
                );
            }
            loop_moves(l, z, tool.feed, &mut m);
            down = true;
            if m.len() > MAX_MOVES {
                return tool.path(m);
            }
        }
        if let Some(at) = last_point(&m) {
            m.push(Move::Rapid {
                to: Point3::new(at.x, at.y, retract),
            });
        }
        prev = z;
    }
    if let Some(at) = last_point(&m) {
        m.push(Move::Rapid {
            to: Point3::new(at.x, at.y, clearance),
        });
    }
    tool.path(m)
}

fn last_point(m: &[Move]) -> Option<Point3> {
    m.last().map(|mv| match *mv {
        Move::Rapid { to } | Move::Line { to, .. } | Move::Plunge { to, .. } => to,
        Move::Arc { to, .. } => to,
        Move::Drill { at, r, .. } => p3(at, r),
    })
}

/// Gets the tool from above the stock down to `z` at `p0`, the start of loop `l`, which
/// is `room` farther from the walls than the tool radius (so a helix of that radius about
/// `p0` stays clear of them).
#[allow(clippy::too_many_arguments)]
fn enter(
    l: &[Seg],
    p0: Point2,
    room: f64,
    z: f64,
    from: f64,
    retract: f64,
    tool: &Cutter,
    entry: Entry,
    ramp_angle: f64,
    helix_radius: f64,
    m: &mut Vec<Move>,
) {
    let slope = ramp_angle.tan();
    let from = from.max(z);
    let r_h = helix_radius.min(room);
    let helix_ok = entry == Entry::Helix && r_h >= (0.05 * tool.radius()).max(0.01);
    let ramp_ok = matches!(entry, Entry::Ramp | Entry::Helix) && loop_length(l) > 1e-3;
    if helix_ok {
        // Down a helix about p0, starting and ending at s = p0 + (r_h, 0), one flat turn
        // at the bottom, then in to p0.
        let s = Point2::new(p0.x + r_h, p0.y);
        let far = Point2::new(p0.x - r_h, p0.y);
        m.push(Move::Rapid { to: p3(s, retract) });
        m.push(Move::Rapid { to: p3(s, from) });
        let pitch = TAU * r_h * slope;
        let turns = ((from - z) / pitch).ceil().clamp(1.0, 10_000.0) as usize;
        let dz = (from - z) / turns as f64;
        let mut zc = from;
        for _ in 0..=turns {
            let z_mid = (zc - dz / 2.0).max(z);
            let z_end = (zc - dz).max(z);
            m.push(Move::Arc {
                to: p3(far, z_mid),
                centre: p0,
                clockwise: false,
                feed: tool.plunge_feed,
            });
            m.push(Move::Arc {
                to: p3(s, z_end),
                centre: p0,
                clockwise: false,
                feed: tool.plunge_feed,
            });
            zc = z_end;
        }
        m.push(Move::Line {
            to: p3(p0, z),
            feed: tool.feed,
        });
    } else if ramp_ok {
        // Down along the loop itself in whole laps at no more than the ramp angle.
        let len = loop_length(l);
        m.push(Move::Rapid {
            to: p3(p0, retract),
        });
        m.push(Move::Rapid { to: p3(p0, from) });
        let laps = ((from - z) / (len * slope)).ceil().clamp(1.0, 1_000.0) as usize;
        let total = len * laps as f64;
        let mut done = 0.0;
        for _ in 0..laps {
            for s in l {
                let z0 = from - (from - z) * done / total;
                done += s.length();
                let z1 = from - (from - z) * (done / total).min(1.0);
                seg_moves(s, z0, z1, tool.plunge_feed, m);
            }
        }
    } else {
        m.push(Move::Rapid {
            to: p3(p0, retract),
        });
        m.push(Move::Rapid { to: p3(p0, from) });
        m.push(Move::Plunge {
            to: p3(p0, z),
            feed: tool.plunge_feed,
        });
    }
}

/// A vertical hole to drill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hole {
    pub centre: Point2,
    pub diameter: f64,
    /// Heights of the hole's top and bottom.
    pub top: f64,
    pub bottom: f64,
}

/// Drilling: one cycle per hole, nearest hole next, each to its bottom minus
/// `breakthrough`, with the retract plane `r` (absolute height).
pub fn drill(
    holes: &[Hole],
    tool: &Cutter,
    r: f64,
    peck: Option<f64>,
    breakthrough: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    tool.check()?;
    finite("the retract plane", r)?;
    finite("the clearance height", clearance)?;
    if holes.is_empty() {
        return Err(CamError::Refused(
            "there are no vertical holes to drill".into(),
        ));
    }
    if let Some(h) = holes
        .iter()
        .find(|h| tool.diameter > h.diameter + JOIN_TOLERANCE)
    {
        return Err(CamError::Refused(format!(
            "the {} mm tool is bigger than the {} mm hole at ({}, {})",
            tool.diameter,
            fmt_mm(h.diameter),
            fmt_mm(h.centre.x),
            fmt_mm(h.centre.y)
        )));
    }
    let mut left: Vec<Hole> = holes.to_vec();
    left.sort_by(|a, b| {
        a.centre
            .x
            .total_cmp(&b.centre.x)
            .then(a.centre.y.total_cmp(&b.centre.y))
    });
    let mut m = Vec::new();
    let mut at: Option<Point2> = None;
    while !left.is_empty() {
        let i = match at {
            None => 0,
            Some(p) => left
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.centre.distance(p).total_cmp(&b.1.centre.distance(p)))
                .map_or(0, |x| x.0),
        };
        let h = left.remove(i);
        m.push(Move::Rapid {
            to: p3(h.centre, clearance),
        });
        m.push(Move::Drill {
            at: h.centre,
            r: r.max(h.top),
            bottom: h.bottom - breakthrough,
            peck,
            feed: tool.plunge_feed,
        });
        at = Some(h.centre);
    }
    tool.path(m)
}

fn fmt_mm(v: f64) -> String {
    crate::post::number(v)
}

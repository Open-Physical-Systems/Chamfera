//! Scallop (constant-stepover) finishing, **experimental**: passes a constant distance
//! apart measured along the surface, so a steep slope gets as many passes per millimetre
//! of surface as a flat one, unlike parallel passes, which are a constant distance apart
//! in plan.
//!
//! After the offset-curve formulations of constant-scallop machining (Suresh and Yang,
//! "Constant scallop-height machining of free-form surfaces", J. Eng. Ind. 116, 1994;
//! Choi and Jerard, "Sculptured Surface Machining", 1998, §8.4), on the cutter-location
//! surface instead of the part:
//! 1. The cutter-location heights are sampled on a grid (spacing the smaller of a
//!    quarter of the tool radius and a third of the stepover).
//! 2. Nodes where the tool rests below the part (on the floor beside it) or where the
//!    surface is steeper than `max_slope` between neighbours are borders, not machined.
//! 3. The distance along the cutter-location surface from the borders is found by
//!    Dijkstra's method over the nodes with 16 neighbours (the 8 adjacent and the 8 a
//!    knight's move away), each step as long as the 3D chord between the two heights.
//!    Along the grid's axes and diagonals the steps are exact; in between the distance
//!    is at most about 2.7 % long (the 16-neighbour metric's worst case in the plane).
//! 4. Passes are the iso-lines of that distance at half a cell and then every stepover,
//!    each point dropped onto the part exactly.
//!
//! Refused as too hard (rather than machined badly): a surface with nothing below the
//! slope limit, more than [`MAX_REGIONS`] separate areas to machine, more than
//! [`crate::ops3d::MAX_LEVELS`] passes deep.

use crate::CamError;
use crate::cancel;
use crate::grid::{self, Grid};
use crate::ops::Cutter;
use crate::ops3d::{
    Area, Cl, MAX_DROPS, MAX_LEVELS, check_tol, cl_grid, cutter_path, emit, simplify, spend,
};
use crate::toolpath::Toolpath;
use chamfera_geom::Point3;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Most separate areas one scallop operation machines.
pub const MAX_REGIONS: usize = 64;

/// The 16 neighbours: the 8 adjacent nodes and the 8 a knight's move away.
const STEPS: [(i64, i64); 16] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (2, 1),
    (2, -1),
    (-2, 1),
    (-2, -1),
    (1, 2),
    (1, -2),
    (-1, 2),
    (-1, -2),
];

/// Scallop finishing over `area`: passes `stepover` mm apart along the surface.
pub fn scallop(
    cl: &Cl,
    cutter: &Cutter,
    area: Area,
    stepover: f64,
    tol: f64,
    max_slope: f64,
    clearance: f64,
) -> Result<Toolpath, CamError> {
    let tol = check_tol(tol)?;
    if !(stepover.is_finite() && stepover > 0.0) {
        return Err(CamError::Invalid("the stepover must be positive".into()));
    }
    if !(max_slope.is_finite() && (0.05..=1.5).contains(&max_slope)) {
        return Err(CamError::Invalid(
            "the steepest slope must be from 3 to 85 degrees".into(),
        ));
    }
    let r = cl.tool.radius;
    let g = Grid::covering(
        area.min,
        area.max,
        (r / 4.0).min(stepover / 3.0).max(0.02),
        grid::MAX_GRID_SIDE,
        (stepover / 2.0).max(r / 4.0),
    )?;
    let z = cl_grid(cl, &g)?;
    let (nx, ny) = (g.nx as i64, g.ny as i64);
    let idx = |i: i64, j: i64| -> Option<usize> {
        (i >= 0 && j >= 0 && i < nx && j < ny).then(|| g.index(i as usize, j as usize))
    };
    let h = |k: usize| z.get(k).copied().unwrap_or(cl.floor);
    let steep = max_slope.tan() * g.cell;
    // Borders: off the part, or steeper than the limit to a neighbour.
    let mut blocked = vec![false; g.len()];
    for j in 0..ny {
        for i in 0..nx {
            let Some(k) = idx(i, j) else { continue };
            let zk = h(k);
            let mut b = zk <= cl.floor + 1e-9;
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(n) = idx(i + di, j + dj)
                    && (h(n) - zk).abs() > steep
                {
                    b = true;
                }
            }
            if let Some(x) = blocked.get_mut(k) {
                *x = b;
            }
        }
    }
    if blocked.iter().all(|b| *b) {
        return Err(CamError::Refused(
            "scallop finishing (experimental): the whole surface is steeper than the slope \
             limit or off the part"
                .into(),
        ));
    }
    // Separate areas (4-connected), refused past the limit.
    let mut region = vec![usize::MAX; g.len()];
    let mut regions = 0usize;
    for start in 0..g.len() {
        if blocked.get(start).copied().unwrap_or(true) || region[start] != usize::MAX {
            continue;
        }
        regions += 1;
        if regions > MAX_REGIONS {
            return Err(CamError::Refused(format!(
                "scallop finishing (experimental): the surface falls into more than \
                 {MAX_REGIONS} separate areas; use parallel or waterline passes"
            )));
        }
        let mut stack = vec![start];
        region[start] = regions;
        while let Some(k) = stack.pop() {
            let (i, j) = ((k % g.nx) as i64, (k / g.nx) as i64);
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(n) = idx(i + di, j + dj)
                    && !blocked[n]
                    && region[n] == usize::MAX
                {
                    region[n] = regions;
                    stack.push(n);
                }
            }
        }
    }
    // Distance along the surface from the borders (and the grid's edge).
    let mut dist = vec![f64::INFINITY; g.len()];
    let mut heap = BinaryHeap::new();
    for j in 0..ny {
        for i in 0..nx {
            let Some(k) = idx(i, j) else { continue };
            if blocked[k] {
                dist[k] = 0.0;
                continue;
            }
            let edge = i == 0 || j == 0 || i == nx - 1 || j == ny - 1;
            let beside = (-1..=1)
                .any(|di| (-1..=1).any(|dj| idx(i + di, j + dj).is_some_and(|n| blocked[n])));
            if edge || beside {
                dist[k] = 0.0;
                heap.push(Reverse((0u64, k)));
            }
        }
    }
    let mut budget = MAX_DROPS.saturating_sub(g.len());
    let mut popped = 0usize;
    while let Some(Reverse((bits, k))) = heap.pop() {
        let d = f64::from_bits(bits);
        if d > dist[k] {
            continue;
        }
        popped += 1;
        if popped.is_multiple_of(4096) {
            cancel::check()?;
        }
        let (i, j) = ((k % g.nx) as i64, (k / g.nx) as i64);
        for (di, dj) in STEPS {
            let Some(n) = idx(i + di, j + dj) else {
                continue;
            };
            if blocked[n] {
                continue;
            }
            let dx = di as f64 * g.cell;
            let dy = dj as f64 * g.cell;
            let dz = h(n) - h(k);
            let nd = d + (dx * dx + dy * dy + dz * dz).sqrt();
            if nd < dist[n] {
                dist[n] = nd;
                heap.push(Reverse((nd.to_bits(), n)));
            }
        }
    }
    let far = dist
        .iter()
        .filter(|d| d.is_finite())
        .fold(0.0f64, |a, b| a.max(*b));
    let mut levels = vec![g.cell / 2.0];
    let mut k = 1usize;
    while (k as f64) * stepover < far {
        if levels.len() >= MAX_LEVELS {
            return Err(CamError::TooComplex(format!(
                "scallop finishing would need more than {MAX_LEVELS} passes: use a larger \
                 stepover"
            )));
        }
        levels.push(k as f64 * stepover);
        k += 1;
    }
    let field: Vec<f64> = dist.iter().map(|d| d.min(1e12)).collect();
    let mut m = Vec::new();
    for level in levels {
        cancel::check()?;
        let mut cross = |a: chamfera_geom::Point2, b: chamfera_geom::Point2, va: f64, vb: f64| {
            let t = if (va - vb).abs() > 1e-300 {
                ((va - level) / (va - vb)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            a + (b - a) * t
        };
        for c in grid::contours(&g, &field, level, &mut cross) {
            let mut flat: Vec<Point3> = Vec::with_capacity(c.points.len() + 1);
            for q in &c.points {
                spend(&mut budget, 1)?;
                flat.push(Point3::new(q.x, q.y, cl.at(q.x, q.y)));
            }
            if c.closed
                && let Some(f) = flat.first().copied()
            {
                flat.push(f);
            }
            // Between contour points, halved while the part strays more than `tol` from
            // the chord (at most four times).
            let mut pts: Vec<Point3> = Vec::with_capacity(flat.len());
            for w in flat.windows(2) {
                if pts.is_empty() {
                    pts.push(w[0]);
                }
                let mut stack = vec![(w[0], w[1], 0u32)];
                while let Some((a, b, depth)) = stack.pop() {
                    if depth < 4 {
                        let mid = a + (b - a) * 0.5;
                        spend(&mut budget, 1)?;
                        let zm = cl.at(mid.x, mid.y);
                        if (zm - mid.z).abs() > tol {
                            let pm = Point3::new(mid.x, mid.y, zm);
                            stack.push((pm, b, depth + 1));
                            stack.push((a, pm, depth + 1));
                            continue;
                        }
                    }
                    pts.push(b);
                }
            }
            if pts.is_empty() {
                pts = flat;
            }
            emit(&simplify(pts), cutter, clearance, &mut m);
        }
    }
    cutter_path(cutter, m)
}

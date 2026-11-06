//! Regular grids in XY for 3D machining: iso-contours (marching squares) and the exact
//! Euclidean distance transform.
//!
//! Marching squares: Lorensen and Cline, "Marching cubes" (SIGGRAPH 1987), in two
//! dimensions, saddles decided by the cell's centre value. Distance transform:
//! Felzenszwalb and Huttenlocher, "Distance Transforms of Sampled Functions" (Theory of
//! Computing 8, 2012), the lower envelope of parabolas, exact on the grid.

use crate::CamError;
use chamfera_geom::Point2;
use std::collections::HashMap;

/// Most nodes along one side of any grid (hostile-input and memory limit).
pub const MAX_GRID_SIDE: usize = 1024;

/// Nodes `min + (i, j) * cell`, `i < nx`, `j < ny`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub min: Point2,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
}

impl Grid {
    /// A grid covering `min..max` with square cells of `cell` or larger, so neither side
    /// has more than `max_side` nodes. Refuses a grid that would need cells coarser than
    /// `coarsest`.
    pub fn covering(
        min: Point2,
        max: Point2,
        cell: f64,
        max_side: usize,
        coarsest: f64,
    ) -> Result<Grid, CamError> {
        let (w, h) = (max.x - min.x, max.y - min.y);
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && cell.is_finite()) {
            return Err(CamError::Invalid("the area to machine has no size".into()));
        }
        let side = max_side.clamp(2, MAX_GRID_SIDE);
        let need = (w.max(h) / (side - 1) as f64).max(cell);
        if need > coarsest {
            return Err(CamError::TooComplex(format!(
                "the area is too large for this tool: it would need grid cells of {need:.3} mm \
                 (at most {side} per side), coarser than {coarsest:.3} mm"
            )));
        }
        let nx = ((w / need).ceil() as usize + 1).clamp(2, side);
        let ny = ((h / need).ceil() as usize + 1).clamp(2, side);
        Ok(Grid {
            min,
            cell: need,
            nx,
            ny,
        })
    }

    pub fn len(&self) -> usize {
        self.nx * self.ny
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn at(&self, i: usize, j: usize) -> Point2 {
        Point2::new(
            self.min.x + i as f64 * self.cell,
            self.min.y + j as f64 * self.cell,
        )
    }

    pub fn index(&self, i: usize, j: usize) -> usize {
        j * self.nx + i
    }

    /// The node nearest `p`, if `p` is over the grid.
    pub fn nearest(&self, p: Point2) -> Option<(usize, usize)> {
        let fi = ((p.x - self.min.x) / self.cell).round();
        let fj = ((p.y - self.min.y) / self.cell).round();
        if !(fi.is_finite() && fj.is_finite()) || fi < 0.0 || fj < 0.0 {
            return None;
        }
        let (i, j) = (fi as usize, fj as usize);
        (i < self.nx && j < self.ny).then_some((i, j))
    }
}

/// One iso-line: its points in order, and whether it closes on itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    pub points: Vec<Point2>,
    pub closed: bool,
}

/// The iso-lines `value = level` of a field sampled at the grid's nodes. Each grid edge
/// it crosses gives one point, placed by `cross(a, b, va, vb)` between the two nodes
/// (linear interpolation, or an exact refinement).
pub fn contours(
    g: &Grid,
    values: &[f64],
    level: f64,
    cross: &mut dyn FnMut(Point2, Point2, f64, f64) -> Point2,
) -> Vec<Contour> {
    let inside = |i: usize, j: usize| values.get(g.index(i, j)).is_some_and(|v| *v > level);
    let val = |i: usize, j: usize| values.get(g.index(i, j)).copied().unwrap_or(f64::NAN);
    // Edge keys: 2k for the edge from node k to its +x neighbour, 2k + 1 to +y.
    let h = |i: usize, j: usize| 2 * g.index(i, j);
    let v = |i: usize, j: usize| 2 * g.index(i, j) + 1;
    let mut segs: Vec<(usize, usize)> = Vec::new();
    if g.nx < 2 || g.ny < 2 || values.len() < g.len() {
        return Vec::new();
    }
    for j in 0..g.ny - 1 {
        for i in 0..g.nx - 1 {
            let c = usize::from(inside(i, j))
                | usize::from(inside(i + 1, j)) << 1
                | usize::from(inside(i + 1, j + 1)) << 2
                | usize::from(inside(i, j + 1)) << 3;
            let (e0, e1, e2, e3) = (h(i, j), v(i + 1, j), h(i, j + 1), v(i, j));
            let centre_in =
                (val(i, j) + val(i + 1, j) + val(i + 1, j + 1) + val(i, j + 1)) / 4.0 > level;
            match c {
                1 | 14 => segs.push((e3, e0)),
                2 | 13 => segs.push((e0, e1)),
                3 | 12 => segs.push((e3, e1)),
                4 | 11 => segs.push((e1, e2)),
                6 | 9 => segs.push((e0, e2)),
                7 | 8 => segs.push((e3, e2)),
                5 => {
                    if centre_in {
                        segs.push((e0, e1));
                        segs.push((e2, e3));
                    } else {
                        segs.push((e3, e0));
                        segs.push((e1, e2));
                    }
                }
                10 => {
                    if centre_in {
                        segs.push((e3, e0));
                        segs.push((e1, e2));
                    } else {
                        segs.push((e0, e1));
                        segs.push((e2, e3));
                    }
                }
                _ => {}
            }
        }
    }
    let mut by_edge: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, (a, b)) in segs.iter().enumerate() {
        by_edge.entry(*a).or_default().push(k);
        by_edge.entry(*b).or_default().push(k);
    }
    let mut points: HashMap<usize, Point2> = HashMap::new();
    let mut point_of = |key: usize| -> Point2 {
        if let Some(p) = points.get(&key) {
            return *p;
        }
        let node = key / 2;
        let (i, j) = (node % g.nx, node / g.nx);
        let (i2, j2) = if key.is_multiple_of(2) {
            (i + 1, j)
        } else {
            (i, j + 1)
        };
        let p = cross(g.at(i, j), g.at(i2, j2), val(i, j), val(i2, j2));
        points.insert(key, p);
        p
    };
    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    for start in 0..segs.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        used[start] = true;
        let (a, b) = segs[start];
        // Walk forward from b, then backward from a.
        let mut fwd = vec![a, b];
        let mut closed = false;
        let walk = |from: usize, chain: &mut Vec<usize>, used: &mut Vec<bool>| -> bool {
            let mut at = from;
            for _ in 0..segs.len() {
                let next = by_edge
                    .get(&at)
                    .and_then(|l| l.iter().find(|k| !used.get(**k).copied().unwrap_or(true)));
                let Some(&k) = next else {
                    return false;
                };
                used[k] = true;
                let (p, q) = segs[k];
                at = if p == at { q } else { p };
                if at == a {
                    return true;
                }
                chain.push(at);
            }
            false
        };
        if walk(b, &mut fwd, &mut used) {
            closed = true;
        } else {
            let mut back = Vec::new();
            walk(a, &mut back, &mut used);
            back.reverse();
            back.extend(fwd);
            fwd = back;
        }
        let pts: Vec<Point2> = fwd.into_iter().map(&mut point_of).collect();
        if pts.len() >= 2 {
            out.push(Contour {
                points: pts,
                closed,
            });
        }
    }
    out
}

/// The exact Euclidean distance (mm) from each node to the nearest `source` node;
/// infinite when there is none.
pub fn distance_transform(g: &Grid, source: &[bool]) -> Vec<f64> {
    let n = g.len();
    let big = 1e30;
    let mut d: Vec<f64> = (0..n)
        .map(|k| {
            if source.get(k).copied().unwrap_or(false) {
                0.0
            } else {
                big
            }
        })
        .collect();
    let mut f = Vec::new();
    let mut out = Vec::new();
    // Columns, then rows: the squared distance separates.
    for i in 0..g.nx {
        f.clear();
        f.extend((0..g.ny).map(|j| d[g.index(i, j)]));
        edt_1d(&f, &mut out);
        for j in 0..g.ny {
            d[g.index(i, j)] = out[j];
        }
    }
    for j in 0..g.ny {
        f.clear();
        f.extend((0..g.nx).map(|i| d[g.index(i, j)]));
        edt_1d(&f, &mut out);
        for i in 0..g.nx {
            d[g.index(i, j)] = out[i];
        }
    }
    d.into_iter()
        .map(|s| {
            if s >= big / 2.0 {
                f64::INFINITY
            } else {
                s.sqrt() * g.cell
            }
        })
        .collect()
}

/// The 1D squared distance transform of `f` (Felzenszwalb and Huttenlocher, alg. 1).
fn edt_1d(f: &[f64], out: &mut Vec<f64>) {
    let n = f.len();
    out.clear();
    out.resize(n, 0.0);
    if n == 0 {
        return;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let sq = |q: usize| (q * q) as f64;
    for q in 1..n {
        // z[0] is -inf, so k never goes below 0.
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + sq(q)) - (f[p] + sq(p))) / (2.0 * (q as f64 - p as f64));
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    let mut k = 0usize;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f64 - p as f64;
        *o = dq * dq + f[p];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_transform_is_euclidean() {
        // One source at node (2, 3) of a 1 mm grid: node (5, 7) is 5 mm away (3-4-5).
        let g = Grid {
            min: Point2::new(0.0, 0.0),
            cell: 1.0,
            nx: 10,
            ny: 10,
        };
        let mut s = vec![false; g.len()];
        s[g.index(2, 3)] = true;
        let d = distance_transform(&g, &s);
        assert!((d[g.index(5, 7)] - 5.0).abs() < 1e-12);
        assert!(d[g.index(2, 3)].abs() < 1e-12);
    }

    #[test]
    fn a_circle_contours_to_one_closed_loop() {
        // f = radius 3 about (5, 5): the contour f = 3 is one closed loop whose points
        // (linearly placed) lie within a cell of the circle.
        let g = Grid {
            min: Point2::new(0.0, 0.0),
            cell: 0.25,
            nx: 41,
            ny: 41,
        };
        let vals: Vec<f64> = (0..g.len())
            .map(|k| {
                let p = g.at(k % g.nx, k / g.nx);
                3.0 - p.distance(Point2::new(5.0, 5.0))
            })
            .collect();
        let mut lin = |a: Point2, b: Point2, va: f64, vb: f64| {
            let t = va / (va - vb);
            a + (b - a) * t
        };
        let c = contours(&g, &vals, 0.0, &mut lin);
        assert_eq!(c.len(), 1);
        assert!(c[0].closed);
        for p in &c[0].points {
            assert!((p.distance(Point2::new(5.0, 5.0)) - 3.0).abs() < 0.02);
        }
    }
}

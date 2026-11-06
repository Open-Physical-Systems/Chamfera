//! Exact offsets of closed loops of lines and arcs.
//!
//! The method is the classical one for curvilinear polygons (Held, "On the
//! Computational Geometry of Pocket Machining", 1991, ch. 3; also Choi and Jerard,
//! "Sculptured Surface Machining", 1998, §8 on contour-parallel offsets):
//!
//! 1. offset every segment by `d` to its left (the "raw" offset: lines move sideways,
//!    arcs change radius, arcs that would turn inside out are inverted);
//! 2. close the gaps at the corners: an arc of radius `d` about the corner where the
//!    path turns right (a reflex corner of the region on the left), and a connector
//!    through the corner itself where it turns left (those pieces are always removed);
//! 3. split every piece where any two pieces cross;
//! 4. keep only the pieces whose points are at least `d` from the original boundary
//!    (pairwise trimming by distance: a piece nearer than `d` lies inside some other
//!    part's offset band);
//! 5. join the kept pieces end to start into closed loops.
//!
//! The loops keep their sense: the region stays on the left. So a region given as a
//! counter-clockwise outer loop and clockwise islands shrinks by `d`, the islands grow.

use crate::CamError;
use crate::path2::{JOIN_TOLERANCE, Loop, Seg, distance_to, intersect, signed_area, simplify};
use chamfera_geom::{Point2, Vec2};

/// Most raw offset pieces one offset may hold (hostile-input limit: trimming compares
/// every pair).
pub const MAX_OFFSET_PIECES: usize = 6_000;

/// A kept piece may be this much nearer than `d` to the boundary (rounding), in mm.
const KEEP_TOLERANCE: f64 = 1e-6;

/// Loops of smaller area than this (mm²) are slivers from rounding, not regions.
const MIN_LOOP_AREA: f64 = 1e-6;

/// The loops `d > 0` to the left of `loops` (see the module notes).
pub fn offset_left(loops: &[Loop], d: f64) -> Result<Vec<Loop>, CamError> {
    if !(d.is_finite() && d > 0.0) {
        return Err(CamError::Invalid(format!(
            "an offset distance must be a positive number, not {d}"
        )));
    }
    let mut raw: Vec<Seg> = Vec::new();
    for l in loops {
        raw_offset(l, d, &mut raw)?;
    }
    let pieces = split_all(&raw)?;
    let kept: Vec<Seg> = pieces
        .into_iter()
        .filter(|s| {
            [0.25, 0.5, 0.75]
                .iter()
                .all(|t| distance_to(loops, s.point_at(*t)) >= d - KEEP_TOLERANCE)
        })
        .collect();
    Ok(join(kept))
}

/// The offset of a closed loop to its right by `d` (the left offset of the reversed loop,
/// turned back to the original sense).
pub fn offset_right(loops: &[Loop], d: f64) -> Result<Vec<Loop>, CamError> {
    let rev: Vec<Loop> = loops
        .iter()
        .map(|l| crate::path2::reverse_loop(l))
        .collect();
    let out = offset_left(&rev, d)?;
    Ok(out
        .iter()
        .map(|l| start_lowest(&crate::path2::reverse_loop(l)))
        .collect())
}

fn raw_offset(l: &[Seg], d: f64, raw: &mut Vec<Seg>) -> Result<(), CamError> {
    let n = l.len();
    for (i, s) in l.iter().enumerate() {
        if let Some(o) = s.offset_left(d) {
            raw.push(o);
        }
        let Some(next) = l.get((i + 1) % n) else {
            continue;
        };
        let v = s.end();
        let t_in = s.tangent_at(1.0);
        let t_out = next.tangent_at(0.0);
        let p_in = v + t_in.perp() * d;
        let p_out = v + t_out.perp() * d;
        if p_in.distance(p_out) > JOIN_TOLERANCE {
            let turn = t_in.cross(t_out);
            if turn < 0.0 || (turn.abs() <= 1e-12 && t_in.dot(t_out) < 0.0) {
                // Turning right: the offsets leave a gap, closed by an arc about the corner.
                raw.push(Seg::arc_between(v, p_in, p_out, false));
            } else {
                // Turning left: the offsets overlap; join them through the corner.
                raw.push(Seg::line(p_in, v));
                raw.push(Seg::line(v, p_out));
            }
        }
        if raw.len() > MAX_OFFSET_PIECES {
            return Err(too_many());
        }
    }
    Ok(())
}

fn too_many() -> CamError {
    CamError::TooComplex(format!(
        "the offset has more than {MAX_OFFSET_PIECES} pieces"
    ))
}

/// Every piece split wherever another crosses or touches it.
fn split_all(raw: &[Seg]) -> Result<Vec<Seg>, CamError> {
    let mut cuts: Vec<Vec<f64>> = vec![Vec::new(); raw.len()];
    for (i, a) in raw.iter().enumerate() {
        for (j, b) in raw.iter().enumerate().skip(i + 1) {
            for (t, u) in intersect(a, b) {
                if let Some(c) = cuts.get_mut(i) {
                    c.push(t);
                }
                if let Some(c) = cuts.get_mut(j) {
                    c.push(u);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (s, mut ts) in raw.iter().zip(cuts) {
        let len = s.length();
        if len <= JOIN_TOLERANCE {
            continue;
        }
        ts.sort_by(f64::total_cmp);
        // Cut points closer than the tolerance to each other or to an end are one point.
        let mut ks = vec![0.0];
        for t in ts {
            let last = ks.last().copied().unwrap_or(0.0);
            if (t - last) * len > JOIN_TOLERANCE && (1.0 - t) * len > JOIN_TOLERANCE {
                ks.push(t);
            }
        }
        ks.push(1.0);
        for w in ks.windows(2) {
            out.push(s.sub(w[0], w[1]));
        }
        if out.len() > MAX_OFFSET_PIECES * 4 {
            return Err(too_many());
        }
    }
    Ok(out)
}

/// Joins pieces end to start into closed loops; open leftovers are dropped.
fn join(pieces: Vec<Seg>) -> Vec<Loop> {
    let mut used = vec![false; pieces.len()];
    let mut loops = Vec::new();
    for first in 0..pieces.len() {
        if used.get(first).copied().unwrap_or(true) {
            continue;
        }
        let Some(start_seg) = pieces.get(first) else {
            continue;
        };
        let start = start_seg.start();
        let mut chain = vec![first];
        if let Some(u) = used.get_mut(first) {
            *u = true;
        }
        let mut end = start_seg.end();
        let mut closed = end.distance(start) <= JOIN_TOLERANCE * 10.0;
        while !closed {
            // The unused piece starting nearest the current end, preferring the sharpest
            // left turn when several start there (keeps touching loops apart).
            let came = pieces
                .get(chain.last().copied().unwrap_or(first))
                .map_or(Vec2::X, |s| s.tangent_at(1.0));
            let mut best: Option<(usize, f64)> = None;
            for (k, s) in pieces.iter().enumerate() {
                if used.get(k).copied().unwrap_or(true)
                    || s.start().distance(end) > JOIN_TOLERANCE * 10.0
                {
                    continue;
                }
                let t = s.tangent_at(0.0);
                let turn = came.cross(t).atan2(came.dot(t));
                if best.is_none_or(|b| turn > b.1) {
                    best = Some((k, turn));
                }
            }
            let Some((k, _)) = best else {
                break;
            };
            if let Some(u) = used.get_mut(k) {
                *u = true;
            }
            chain.push(k);
            end = pieces.get(k).map_or(end, Seg::end);
            closed = end.distance(start) <= JOIN_TOLERANCE * 10.0;
        }
        if closed {
            let l: Loop = chain
                .iter()
                .filter_map(|k| pieces.get(*k).copied())
                .collect();
            let l = simplify(&l);
            if signed_area(&l).abs() > MIN_LOOP_AREA {
                loops.push(start_lowest(&l));
            }
        }
    }
    loops
}

/// The loop started at its lowest segment start (least y, then least x), so offsets come
/// out the same whatever order the pieces were joined in.
pub fn start_lowest(l: &[Seg]) -> Loop {
    let key = |p: Point2| (p.y, p.x);
    let i = l
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let (ka, kb) = (key(a.1.start()), key(b.1.start()));
            let ya = (ka.0 - kb.0).abs() <= JOIN_TOLERANCE;
            if ya {
                ka.1.total_cmp(&kb.1)
            } else {
                ka.0.total_cmp(&kb.0)
            }
        })
        .map_or(0, |(i, _)| i);
    l.iter().skip(i).chain(l.iter().take(i)).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path2::{is_closed, loop_length};
    use std::f64::consts::{PI, TAU};

    fn p(x: f64, y: f64) -> Point2 {
        Point2::new(x, y)
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        vec![
            Seg::line(p(x0, y0), p(x1, y0)),
            Seg::line(p(x1, y0), p(x1, y1)),
            Seg::line(p(x1, y1), p(x0, y1)),
            Seg::line(p(x0, y1), p(x0, y0)),
        ]
    }

    #[test]
    fn a_50_by_30_rectangle_grown_by_3_has_round_corners_and_the_expected_length() {
        // Outside offset by r = 3: the four sides keep their lengths (2 (50 + 30)) and
        // each corner becomes a quarter circle of radius 3: four quarters are one full
        // circle, 2π·3. Total 160 + 6π.
        let out = offset_right(&[rect(0.0, 0.0, 50.0, 30.0)], 3.0).unwrap();
        assert_eq!(out.len(), 1);
        let l = &out[0];
        assert!(is_closed(l));
        assert!(
            (loop_length(l) - (160.0 + 6.0 * PI)).abs() < 1e-9,
            "{}",
            loop_length(l)
        );
        let arcs: Vec<_> = l
            .iter()
            .filter_map(|s| match *s {
                Seg::Arc { c, r, sweep, .. } => Some((c, r, sweep)),
                Seg::Line { .. } => None,
            })
            .collect();
        assert_eq!(arcs.len(), 4);
        for (c, r, sweep) in arcs {
            assert!((r - 3.0).abs() < 1e-12);
            assert!((sweep - PI / 2.0).abs() < 1e-9, "{sweep}");
            // Each arc is centred on a corner of the rectangle.
            assert!(
                [p(0.0, 0.0), p(50.0, 0.0), p(50.0, 30.0), p(0.0, 30.0)]
                    .iter()
                    .any(|k| k.distance(c) < 1e-12)
            );
        }
        // The sense is kept: still counter-clockwise, and it starts at (0, -3).
        assert!(signed_area(l) > 0.0);
        assert!(l[0].start().distance(p(0.0, -3.0)) < 1e-12);
    }

    #[test]
    fn a_rectangle_shrunk_by_3_is_the_inner_rectangle_with_sharp_corners() {
        // Inside offset of 40 x 20 by 3: 34 x 14, perimeter 96, area 476.
        let out = offset_left(&[rect(0.0, 0.0, 40.0, 20.0)], 3.0).unwrap();
        assert_eq!(out.len(), 1);
        assert!((loop_length(&out[0]) - 96.0).abs() < 1e-9);
        assert!((signed_area(&out[0]) - 476.0).abs() < 1e-9);
        assert_eq!(out[0].len(), 4);
        // Shrunk past half the width (10) nothing is left.
        assert!(
            offset_left(&[rect(0.0, 0.0, 40.0, 20.0)], 10.5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_island_grows_and_cuts_the_region_where_they_meet() {
        // A 40 x 20 region with a round island r = 4 at its centre (20, 10), the island
        // clockwise. At d = 3 the island's offset is a circle of radius 7, which crosses
        // the shrunk outer rectangle 34 x 14 (y from 3 to 17: 7 from the centre exactly
        // touches it). At d = 2 they stay apart: two loops.
        let island = vec![Seg::Arc {
            c: p(20.0, 10.0),
            r: 4.0,
            start: 0.0,
            sweep: -TAU,
        }];
        let out = offset_left(&[rect(0.0, 0.0, 40.0, 20.0), island.clone()], 2.0).unwrap();
        assert_eq!(out.len(), 2);
        let areas: Vec<f64> = out.iter().map(|l| signed_area(l)).collect();
        // Outer 36 x 16 = 576 counter-clockwise; island circle r = 6, clockwise: -36π.
        assert!(areas.iter().any(|a| (a - 576.0).abs() < 1e-6), "{areas:?}");
        assert!(
            areas.iter().any(|a| (a + 36.0 * PI).abs() < 1e-6),
            "{areas:?}"
        );
        // At d = 5 the circle (r = 9) cuts through the band (y 5..15): two regions left
        // and right of the island.
        let out = offset_left(&[rect(0.0, 0.0, 40.0, 20.0), island], 5.0).unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        for l in &out {
            assert!(signed_area(l) > 0.0);
            for s in l {
                for t in [0.0, 0.5, 1.0] {
                    assert!(s.point_at(t).distance(p(20.0, 10.0)) >= 9.0 - 1e-6);
                }
            }
        }
    }

    #[test]
    fn a_rounded_corner_smaller_than_the_offset_becomes_sharp() {
        // A 20 x 20 square with corner fillets of radius 1, shrunk by 3: the fillets'
        // offsets would turn inside out, so the result is the sharp 14 x 14 square.
        let r = 1.0;
        let l = vec![
            Seg::line(p(r, 0.0), p(20.0 - r, 0.0)),
            Seg::Arc {
                c: p(20.0 - r, r),
                r,
                start: -PI / 2.0,
                sweep: PI / 2.0,
            },
            Seg::line(p(20.0, r), p(20.0, 20.0 - r)),
            Seg::Arc {
                c: p(20.0 - r, 20.0 - r),
                r,
                start: 0.0,
                sweep: PI / 2.0,
            },
            Seg::line(p(20.0 - r, 20.0), p(r, 20.0)),
            Seg::Arc {
                c: p(r, 20.0 - r),
                r,
                start: PI / 2.0,
                sweep: PI / 2.0,
            },
            Seg::line(p(0.0, 20.0 - r), p(0.0, r)),
            Seg::Arc {
                c: p(r, r),
                r,
                start: PI,
                sweep: PI / 2.0,
            },
        ];
        let out = offset_left(&[l], 3.0).unwrap();
        assert_eq!(out.len(), 1);
        assert!((loop_length(&out[0]) - 56.0).abs() < 1e-9, "{:?}", out[0]);
        assert!((signed_area(&out[0]) - 196.0).abs() < 1e-9);
    }

    #[test]
    fn bad_distances_are_refused() {
        for d in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(offset_left(&[rect(0.0, 0.0, 1.0, 1.0)], d).is_err());
        }
    }
}

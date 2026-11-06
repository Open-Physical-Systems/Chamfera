//! Toolpaths: the tool's moves in model coordinates, with feeds, and what they cost.

use chamfera_geom::{Point2, Point3};
use std::f64::consts::TAU;

/// How far above the previous peck's bottom a G83 cycle comes back down at rapid before
/// feeding again. RS274/NGC (NIST interpreter spec v3, §3.5.16, G83) says "a little";
/// LinuxCNC documents 0.010 inch, which is this in millimetres.
pub const PECK_CLEARANCE: f64 = 0.254;

/// One move of the tool. Positions are absolute, in model coordinates (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Move {
    /// At the machine's rapid rate (G0).
    Rapid { to: Point3 },
    /// A straight cut at `feed` mm/min (G1).
    Line { to: Point3, feed: f64 },
    /// A straight cut down along Z only, at the plunge feed (G1 Z).
    Plunge { to: Point3, feed: f64 },
    /// A circular cut about `centre` in XY (G2 clockwise, G3 counter-clockwise); Z goes
    /// linearly from the start to `to.z` on the way (a helix when they differ). The end is
    /// on the same circle as the start; equal ends make a full turn.
    Arc {
        to: Point3,
        centre: Point2,
        clockwise: bool,
        feed: f64,
    },
    /// A drilling cycle at `at` (G81, or G83 with `peck`): rapid to the retract plane
    /// `r`, feed to `bottom` (in pecks), back to the height it started at.
    Drill {
        at: Point2,
        r: f64,
        bottom: f64,
        peck: Option<f64>,
        feed: f64,
    },
}

/// What one operation makes: its tool and its moves.
#[derive(Debug, Clone, PartialEq)]
pub struct Toolpath {
    /// The tool's number in the changer (`T` word) and its spindle speed (`S`).
    pub tool_number: u32,
    pub tool_name: String,
    pub spindle_rpm: f64,
    pub moves: Vec<Move>,
}

/// Lengths and time of a toolpath.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stats {
    /// Length cut at feed (lines, plunges, arcs, the feed strokes of drill cycles), mm.
    pub feed_length: f64,
    /// Length moved at rapid, mm.
    pub rapid_length: f64,
    /// Time at the given feeds and rapid rate, minutes.
    pub minutes: f64,
}

fn dist(a: Point3, b: Point3) -> f64 {
    (b - a).length()
}

/// Length of an XY arc (with linear Z) from `from` to `to` about `c`.
pub fn arc_length(from: Point3, to: Point3, c: Point2, clockwise: bool) -> f64 {
    let a0 = (Point2::new(from.x, from.y) - c).angle();
    let a1 = (Point2::new(to.x, to.y) - c).angle();
    let r = Point2::new(from.x, from.y).distance(c);
    let mut sweep = crate::path2::norm_angle(if clockwise { a0 - a1 } else { a1 - a0 });
    if sweep * r <= crate::path2::JOIN_TOLERANCE {
        sweep = TAU;
    }
    (r * sweep).hypot(to.z - from.z)
}

impl Toolpath {
    /// The first position the toolpath goes to (its first move's end).
    pub fn start(&self) -> Option<Point3> {
        self.moves.first().map(|m| match *m {
            Move::Rapid { to } | Move::Line { to, .. } | Move::Plunge { to, .. } => to,
            Move::Arc { to, .. } => to,
            Move::Drill { at, r, .. } => Point3::new(at.x, at.y, r),
        })
    }

    /// The same path with drill cycles written out as the rapids and feeds they make,
    /// as RS274/NGC defines G81 and G83 (NIST spec v3, §3.5.16) with G98 (return to the
    /// height before the cycle).
    pub fn expanded(&self) -> Vec<Move> {
        let mut out = Vec::with_capacity(self.moves.len());
        let mut at = self.start().unwrap_or_default();
        for m in &self.moves {
            match *m {
                Move::Drill {
                    at: xy,
                    r,
                    bottom,
                    peck,
                    feed,
                } => {
                    let start_z = at.z;
                    let p = |z: f64| Point3::new(xy.x, xy.y, z);
                    out.push(Move::Rapid { to: p(start_z) });
                    out.push(Move::Rapid { to: p(r) });
                    match peck {
                        Some(q) if q > 0.0 => {
                            let mut depth = r;
                            // Bounded: at most this many pecks (the peck is at least
                            // 1/10000 of the stroke).
                            for _ in 0..10_000 {
                                let next = (depth - q).max(bottom);
                                if depth < r {
                                    out.push(Move::Rapid {
                                        to: p((depth + PECK_CLEARANCE).min(r)),
                                    });
                                }
                                out.push(Move::Plunge { to: p(next), feed });
                                out.push(Move::Rapid { to: p(r) });
                                depth = next;
                                if depth <= bottom {
                                    break;
                                }
                            }
                        }
                        _ => {
                            out.push(Move::Plunge {
                                to: p(bottom),
                                feed,
                            });
                        }
                    }
                    out.push(Move::Rapid { to: p(start_z) });
                    at = p(start_z);
                }
                Move::Rapid { to } | Move::Line { to, .. } | Move::Plunge { to, .. } => {
                    out.push(*m);
                    at = to;
                }
                Move::Arc { to, .. } => {
                    out.push(*m);
                    at = to;
                }
            }
        }
        out
    }

    /// Feed and rapid lengths and the cycle time at `rapid_rate` mm/min.
    pub fn stats(&self, rapid_rate: f64) -> Stats {
        let mut s = Stats::default();
        let Some(mut at) = self.start() else {
            return s;
        };
        let rapid = if rapid_rate > 0.0 { rapid_rate } else { 1.0 };
        for m in self.expanded() {
            let (to, len, feed) = match m {
                Move::Rapid { to } => {
                    let l = dist(at, to);
                    s.rapid_length += l;
                    s.minutes += l / rapid;
                    at = to;
                    continue;
                }
                Move::Line { to, feed } | Move::Plunge { to, feed } => (to, dist(at, to), feed),
                Move::Arc {
                    to,
                    centre,
                    clockwise,
                    feed,
                } => (to, arc_length(at, to, centre, clockwise), feed),
                Move::Drill { .. } => continue,
            };
            s.feed_length += len;
            if feed > 0.0 {
                s.minutes += len / feed;
            }
            at = to;
        }
        s
    }

    /// The path as straight pieces for display: (rapid pieces, feed pieces). Arcs are cut
    /// into chords at most `max_angle` radians each.
    pub fn polylines(&self, max_angle: f64) -> (Vec<[Point3; 2]>, Vec<[Point3; 2]>) {
        let mut rapids = Vec::new();
        let mut feeds = Vec::new();
        let Some(mut at) = self.start() else {
            return (rapids, feeds);
        };
        let step = if max_angle > 1e-3 { max_angle } else { 0.1 };
        for m in self.expanded() {
            match m {
                Move::Rapid { to } => {
                    rapids.push([at, to]);
                    at = to;
                }
                Move::Line { to, .. } | Move::Plunge { to, .. } => {
                    feeds.push([at, to]);
                    at = to;
                }
                Move::Arc {
                    to,
                    centre,
                    clockwise,
                    ..
                } => {
                    let from2 = Point2::new(at.x, at.y);
                    let r = from2.distance(centre);
                    let a0 = (from2 - centre).angle();
                    let a1 = (Point2::new(to.x, to.y) - centre).angle();
                    let mut sweep =
                        crate::path2::norm_angle(if clockwise { a0 - a1 } else { a1 - a0 });
                    if sweep * r <= crate::path2::JOIN_TOLERANCE {
                        sweep = TAU;
                    }
                    let n = ((sweep / step).ceil() as usize).clamp(1, 720);
                    let mut prev = at;
                    for i in 1..=n {
                        let f = i as f64 / n as f64;
                        let a = if clockwise {
                            a0 - sweep * f
                        } else {
                            a0 + sweep * f
                        };
                        let p = if i == n {
                            to
                        } else {
                            Point3::new(
                                centre.x + r * a.cos(),
                                centre.y + r * a.sin(),
                                at.z + (to.z - at.z) * f,
                            )
                        };
                        feeds.push([prev, p]);
                        prev = p;
                    }
                    at = to;
                }
                Move::Drill { .. } => {}
            }
        }
        (rapids, feeds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    #[test]
    fn cycle_time_adds_each_length_over_its_rate() {
        // Rapid 0 -> (0,0,10) is the start; rapid down 10 at 5000, feed 100 at 1000, a
        // half circle of radius 10 (10π) at 500.
        let t = Toolpath {
            tool_number: 1,
            tool_name: "t".into(),
            spindle_rpm: 1000.0,
            moves: vec![
                Move::Rapid {
                    to: p(0.0, 0.0, 10.0),
                },
                Move::Rapid {
                    to: p(0.0, 0.0, 0.0),
                },
                Move::Line {
                    to: p(100.0, 0.0, 0.0),
                    feed: 1000.0,
                },
                Move::Arc {
                    to: p(120.0, 0.0, 0.0),
                    centre: Point2::new(110.0, 0.0),
                    clockwise: true,
                    feed: 500.0,
                },
            ],
        };
        let s = t.stats(5000.0);
        let half = 10.0 * std::f64::consts::PI;
        assert!((s.rapid_length - 10.0).abs() < 1e-12);
        assert!((s.feed_length - (100.0 + half)).abs() < 1e-9);
        let minutes = 10.0 / 5000.0 + 100.0 / 1000.0 + half / 500.0;
        assert!((s.minutes - minutes).abs() < 1e-12);
    }

    #[test]
    fn a_peck_cycle_comes_back_up_after_each_peck() {
        // From z 10, R = 2, bottom -5, peck 3: pecks to -1, -4, -5 (three strokes), each
        // followed by a rapid back to R; between pecks a rapid down to 0.254 above the
        // last depth. Feed length: 3 + (3 + 0.254) + (1 + 0.254) = 7.508.
        let t = Toolpath {
            tool_number: 2,
            tool_name: "drill".into(),
            spindle_rpm: 1000.0,
            moves: vec![
                Move::Rapid {
                    to: p(5.0, 5.0, 10.0),
                },
                Move::Drill {
                    at: Point2::new(5.0, 5.0),
                    r: 2.0,
                    bottom: -5.0,
                    peck: Some(3.0),
                    feed: 100.0,
                },
            ],
        };
        let e = t.expanded();
        let plunges: Vec<f64> = e
            .iter()
            .filter_map(|m| match m {
                Move::Plunge { to, .. } => Some(to.z),
                _ => None,
            })
            .collect();
        assert_eq!(plunges, vec![-1.0, -4.0, -5.0]);
        assert!((t.stats(5000.0).feed_length - 7.508).abs() < 1e-9);
        assert_eq!(
            e.last(),
            Some(&Move::Rapid {
                to: p(5.0, 5.0, 10.0)
            })
        );
    }
}

//! Queries on plane curves: ray crossings for point-in-region tests, and polylines.

use super::BSplineCurve2;
use crate::Point2;

/// Most points in one polyline.
const MAX_POLYLINE_POINTS: usize = 16_384;
/// Most samples per knot span when looking for the turning points of `y(u)`.
const TURN_SAMPLES: usize = 32;
/// Bisection steps for a turning point or a crossing (to the last bit of a span).
const BISECTION_STEPS: usize = 64;

impl BSplineCurve2 {
    /// How many times the ray from `p` toward +x crosses the spline, half-open in y like the
    /// other curves (an end exactly at `p.y` counts on one side only). `a` and `b` are the
    /// exact end points (the curve's end vertices), used for the ends so that a chain of curves
    /// through a vertex counts it consistently.
    ///
    /// The domain is cut into pieces monotone in y at the zeros of `y'(u)` (found by sampling
    /// each knot span and bisecting sign changes); on each piece at most one crossing exists,
    /// found by bisection on `y(u) - p.y`.
    pub fn ray_crossings(&self, a: Point2, b: Point2, p: Point2) -> usize {
        let c = self;
        let (lo, hi) = c.domain();
        let point = |u: f64| {
            if u <= lo {
                a
            } else if u >= hi {
                b
            } else {
                c.point_at(u)
            }
        };
        let dy = |u: f64| c.d1(u).y;
        // Monotone pieces.
        let mut cuts = vec![lo];
        for w in c.breaks().windows(2) {
            let (s, e) = (w[0], w[1]);
            let mut prev = (s, dy(s));
            for i in 1..=TURN_SAMPLES {
                let u = s + (e - s) * i as f64 / TURN_SAMPLES as f64;
                let cur = (u, dy(u));
                if (prev.1 > 0.0) != (cur.1 > 0.0) {
                    let (mut l, mut r) = (prev.0, cur.0);
                    let left_positive = prev.1 > 0.0;
                    for _ in 0..BISECTION_STEPS {
                        let m = 0.5 * (l + r);
                        if m <= l || m >= r {
                            break;
                        }
                        if (dy(m) > 0.0) == left_positive {
                            l = m;
                        } else {
                            r = m;
                        }
                    }
                    cuts.push(0.5 * (l + r));
                }
                prev = cur;
            }
            cuts.push(e);
        }
        cuts.dedup();
        let mut count = 0;
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            let (ys, ye) = (point(s).y, point(e).y);
            if (ys > p.y) == (ye > p.y) {
                continue;
            }
            let (mut l, mut r) = (s, e);
            let left_above = ys > p.y;
            for _ in 0..BISECTION_STEPS {
                let m = 0.5 * (l + r);
                if m <= l || m >= r {
                    break;
                }
                if (point(m).y > p.y) == left_above {
                    l = m;
                } else {
                    r = m;
                }
            }
            if point(0.5 * (l + r)).x > p.x {
                count += 1;
            }
        }
        count
    }

    /// The curve as a polyline from its start to its end, `per_span` points evenly spaced
    /// in the parameter on each knot span (at most `MAX_POLYLINE_POINTS` in all).
    pub fn polyline(&self, per_span: usize) -> Vec<Point2> {
        let br = self.breaks();
        let spans = br.len().saturating_sub(1).max(1);
        let per = per_span.min(MAX_POLYLINE_POINTS / spans).max(1);
        let mut out = Vec::with_capacity(spans * per + 1);
        for w in br.windows(2) {
            for i in 0..per {
                out.push(self.point_at(w[0] + (w[1] - w[0]) * i as f64 / per as f64));
            }
        }
        let (_, hi) = self.domain();
        out.push(self.point_at(hi));
        out
    }
}

//! Sweep paths: chains of lines and circular arcs in space, and the distance bounds the
//! self-intersection test uses.

use crate::{KernelError, KernelResult};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, Point3, Transform, Vec3};
use std::f64::consts::{PI, TAU};

/// Most segments one sweep path may have (hostile-input limit).
pub const MAX_PATH_SEGMENTS: usize = 1_000;

/// How far apart (radians) the tangents of two path segments may be where they meet and
/// still count as tangent. Sketch tangency is solved numerically, to about 1e-9 rad; a
/// visible corner is far larger. The swept body follows the first segment's frame
/// transported exactly, so the deviation this allows never breaks its validity.
pub const PATH_TANGENT_TOLERANCE: f64 = 1e-7;

/// One segment of a sweep path, in world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSegment {
    /// The straight segment from `start` to `end`.
    Line { start: Point3, end: Point3 },
    /// The circular arc about `centre` that starts at `start` and turns `angle` radians
    /// counter-clockwise about the unit vector `axis` (right-hand rule),
    /// `0 < angle < 2π`. `start - centre` must be perpendicular to `axis`.
    Arc {
        centre: Point3,
        axis: Vec3,
        start: Point3,
        angle: f64,
    },
}

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

impl PathSegment {
    /// The arc about `centre` from `start` to `end`, counter-clockwise about `axis`.
    pub fn arc_from_points(
        centre: Point3,
        axis: Vec3,
        start: Point3,
        end: Point3,
    ) -> KernelResult<PathSegment> {
        let n = axis
            .normalized()
            .ok_or_else(|| invalid("the arc's axis has no direction"))?;
        let (a, b) = (start - centre, end - centre);
        let ra = a.length();
        if ra <= LINEAR_TOLERANCE {
            return Err(invalid("the arc's radius must be above 0"));
        }
        let mut angle = n.dot(a.cross(b)).atan2(a.dot(b));
        if angle <= ANGULAR_TOLERANCE {
            angle += TAU;
        }
        let seg = PathSegment::Arc {
            centre,
            axis: n,
            start,
            angle,
        };
        seg.check()?;
        if seg.end().distance(end) > LINEAR_TOLERANCE.max(ra * 1e-9) {
            return Err(invalid("the arc's end is not on its circle"));
        }
        Ok(seg)
    }

    /// Checks the segment: finite, in range, long enough, an arc's start perpendicular
    /// to its axis and its angle below a whole turn.
    pub fn check(&self) -> KernelResult<()> {
        match *self {
            PathSegment::Line { start, end } => {
                if !(start.is_valid() && end.is_valid()) {
                    return Err(invalid("a path line has a point out of range"));
                }
                if start.distance(end) <= LINEAR_TOLERANCE {
                    return Err(invalid("a path line has no length"));
                }
            }
            PathSegment::Arc {
                centre,
                axis,
                start,
                angle,
            } => {
                if !(centre.is_valid() && start.is_valid() && axis.is_finite()) {
                    return Err(invalid("a path arc has a point out of range"));
                }
                if (axis.length() - 1.0).abs() > 1e-9 {
                    return Err(invalid("a path arc's axis must be a unit vector"));
                }
                let r = (start - centre).length();
                if !(r > LINEAR_TOLERANCE && r <= MAX_COORDINATE) {
                    return Err(invalid("a path arc's radius must be above 0"));
                }
                if (start - centre).dot(axis).abs() > LINEAR_TOLERANCE {
                    return Err(invalid(
                        "a path arc's start must lie in the plane through its centre perpendicular to its axis",
                    ));
                }
                if !(angle.is_finite() && angle * r > LINEAR_TOLERANCE && angle < TAU) {
                    return Err(invalid(
                        "a path arc must turn by more than 0 and less than a whole turn",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn start(&self) -> Point3 {
        match *self {
            PathSegment::Line { start, .. } | PathSegment::Arc { start, .. } => start,
        }
    }

    pub fn end(&self) -> Point3 {
        match *self {
            PathSegment::Line { end, .. } => end,
            PathSegment::Arc { .. } => self.point_at(1.0),
        }
    }

    /// The point a fraction `f` of the way along the segment.
    pub fn point_at(&self, f: f64) -> Point3 {
        match *self {
            PathSegment::Line { start, end } => start + (end - start) * f,
            PathSegment::Arc {
                centre,
                axis,
                start,
                angle,
            } => Transform::rotation_about(centre, axis, angle * f)
                .map_or(start, |t| t.apply_point(start)),
        }
    }

    /// The unit tangent (in the direction of travel) a fraction `f` of the way along.
    pub fn tangent_at(&self, f: f64) -> Option<Vec3> {
        match *self {
            PathSegment::Line { start, end } => (end - start).normalized(),
            PathSegment::Arc { centre, axis, .. } => {
                axis.cross(self.point_at(f) - centre).normalized()
            }
        }
    }

    pub fn length(&self) -> f64 {
        match *self {
            PathSegment::Line { start, end } => start.distance(end),
            PathSegment::Arc {
                centre,
                start,
                angle,
                ..
            } => (start - centre).length() * angle,
        }
    }

    /// The same segment traversed the other way.
    pub fn reversed(&self) -> PathSegment {
        match *self {
            PathSegment::Line { start, end } => PathSegment::Line {
                start: end,
                end: start,
            },
            PathSegment::Arc {
                centre,
                axis,
                angle,
                ..
            } => PathSegment::Arc {
                centre,
                axis: -axis,
                start: self.end(),
                angle,
            },
        }
    }

    /// The segment split into pieces that each turn by at most a half turn (an arc of
    /// more than π is built in two halves, so each swept piece stays on one side of its
    /// start plane; see the module documentation).
    pub(crate) fn halves(&self) -> Vec<PathSegment> {
        match *self {
            PathSegment::Arc {
                centre,
                axis,
                start,
                angle,
            } if angle > PI => {
                let half = angle * 0.5;
                let mid = self.point_at(0.5);
                vec![
                    PathSegment::Arc {
                        centre,
                        axis,
                        start,
                        angle: half,
                    },
                    PathSegment::Arc {
                        centre,
                        axis,
                        start: mid,
                        angle: half,
                    },
                ]
            }
            other => vec![other],
        }
    }

    /// A polyline through the segment and an upper bound on how far the segment strays
    /// from it: 0 for a line; `R (1 - cos(Δ/2))` (the sagitta) for an arc cut into steps
    /// of `Δ`.
    pub(crate) fn polyline(&self) -> (Vec<Point3>, f64) {
        match *self {
            PathSegment::Line { start, end } => (vec![start, end], 0.0),
            PathSegment::Arc {
                centre,
                start,
                angle,
                ..
            } => {
                let r = (start - centre).length();
                let n = (angle / (PI / 32.0)).ceil().clamp(1.0, 64.0) as usize;
                let step = angle / n as f64;
                let pts = (0..=n)
                    .map(|k| self.point_at(k as f64 / n as f64))
                    .collect();
                (pts, r * (1.0 - (step * 0.5).cos()))
            }
        }
    }
}

/// Orders `segments` into one open chain, each running from the previous one's end,
/// starting at the free end that `starts_here` accepts (the nearer of the two to `near`
/// when both do). Fails for an empty, branching, broken or closed path.
pub(crate) fn chain(
    segments: &[PathSegment],
    starts_here: impl Fn(Point3) -> bool,
    near: Point3,
) -> KernelResult<Vec<PathSegment>> {
    if segments.is_empty() {
        return Err(invalid("the path needs at least one line or arc"));
    }
    if segments.len() > MAX_PATH_SEGMENTS {
        return Err(invalid(format!(
            "a path may have at most {MAX_PATH_SEGMENTS} segments"
        )));
    }
    for s in segments {
        s.check()?;
    }
    let n = segments.len();
    // Endpoint k of segment i is index 2i + k; its partner endpoints on other segments.
    let end = |e: usize| -> Point3 {
        let s = segments[e / 2];
        if e.is_multiple_of(2) {
            s.start()
        } else {
            s.end()
        }
    };
    let mut partner: Vec<Option<usize>> = vec![None; 2 * n];
    for (a, slot) in partner.iter_mut().enumerate() {
        for b in 0..2 * n {
            if a / 2 == b / 2 || end(a).distance(end(b)) > LINEAR_TOLERANCE {
                continue;
            }
            if slot.is_some_and(|p| p != b) {
                return Err(invalid(format!(
                    "the path branches at ({:.3}, {:.3}, {:.3}): it must be a single chain",
                    end(a).x,
                    end(a).y,
                    end(a).z
                )));
            }
            *slot = Some(b);
        }
    }
    let free: Vec<usize> = (0..2 * n).filter(|e| partner[*e].is_none()).collect();
    if free.is_empty() {
        return Err(KernelError::Unsupported(
            "sweeping along a closed path".into(),
        ));
    }
    if free.len() != 2 {
        return Err(invalid(
            "the path's lines and arcs must join end to end into one chain",
        ));
    }
    let candidates: Vec<usize> = free
        .iter()
        .copied()
        .filter(|e| starts_here(end(*e)))
        .collect();
    let first = match candidates.as_slice() {
        [] => {
            return Err(invalid(
                "the path must start on the profile's plane (one of its ends must touch it)",
            ));
        }
        [one] => *one,
        [a, b, ..] => {
            if end(*a).distance(near) <= end(*b).distance(near) {
                *a
            } else {
                *b
            }
        }
    };
    let mut out = Vec::with_capacity(n);
    let mut at = first;
    for _ in 0..n {
        let s = segments[at / 2];
        out.push(if at.is_multiple_of(2) {
            s
        } else {
            s.reversed()
        });
        let leaving = at ^ 1;
        match partner[leaving] {
            Some(p) => at = p,
            None => break,
        }
    }
    if out.len() != n {
        return Err(invalid(
            "the path's lines and arcs must join end to end into one chain",
        ));
    }
    Ok(out)
}

/// The distance between segments `p0 p1` and `q0 q1` (Ericson, *Real-Time Collision
/// Detection*, 2005, section 5.1.9, closest points of two segments).
pub(crate) fn segment_distance(p0: Point3, p1: Point3, q0: Point3, q1: Point3) -> f64 {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let eps = 1e-300;
    let (s, t);
    if a <= eps && e <= eps {
        return p0.distance(q0);
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > eps {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (p0 + d1 * s).distance(q0 + d2 * t)
}

/// A lower bound on the distance between two path segments: the distance between their
/// polylines less both polylines' deviation bounds.
pub(crate) fn distance_lower_bound(a: &PathSegment, b: &PathSegment) -> f64 {
    let (pa, da) = a.polyline();
    let (pb, db) = b.polyline();
    let mut best = f64::INFINITY;
    for x in pa.windows(2) {
        for y in pb.windows(2) {
            if let ([x0, x1], [y0, y1]) = (x, y) {
                best = best.min(segment_distance(*x0, *x1, *y0, *y1));
            }
        }
    }
    best - da - db
}

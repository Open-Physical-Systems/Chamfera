//! Thin profiles: the region a wall of constant thickness covers along a chain of lines
//! and arcs, which a thin extrude sweeps like any profile.
//!
//! # Geometry
//! The wall is bounded by two offsets of the chain, the classical constant-distance
//! offsets of planar curves (Hoffmann, *Geometric and Solid Modeling* (1989), section
//! 7.3; Patrikalakis and Maekawa, *Shape Interrogation* (2002), section 11.2): a line
//! moves by `d` along its left normal, an arc keeps its centre and changes its radius by
//! `∓d` (minus for a counter-clockwise arc, whose left is its centre). Where two edges
//! meet:
//!
//! - smoothly (a tangent join), both offsets start at the same point, `v + d n`;
//! - at a sharp corner of two lines, the offset lines meet at the miter point
//!   `v + d (n₁ + n₂) / (1 + n₁·n₂)` (`n₁`, `n₂` the unit left normals), so the wall keeps
//!   sharp corners;
//! - at a sharp corner involving an arc the offsets meet on a circle, which needs the
//!   intersection of the offset curves; that is refused for now
//!   ([`KernelError::Unsupported`]).
//!
//! A closed chain gives two closed loops, the outer one and a hole: the profile of a
//! tube. An open chain gives one loop: the left offset, a straight cap across the end, the
//! right offset backwards and a cap across the start. The result is checked like any
//! profile, so a wall too thick for a bend (an inner arc radius reaching zero, a line
//! turning round, the loop crossing itself) is refused rather than built wrong.
//!
//! Splines and ellipses are refused: their offsets are not splines (Farouki and Neff,
//! "Analytic properties of plane offset curves", CAGD 7, 1990), so no exact profile holds
//! them.

use crate::profile::ProfileEdge;
use crate::{KernelError, KernelResult, PROFILE_JOIN_TOLERANCE, Profile};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Vec2};

/// Most edges a thin profile's chain may have (hostile-input limit).
pub const MAX_THIN_EDGES: usize = 10_000;

/// Largest sine of the angle between two edges' tangents for their join to count as
/// smooth. Sketch tangency is solved far below this; a join this close to smooth moves
/// an offset by at most `d · 1e-9`, below the join tolerance for any wall up to 50 mm.
const SMOOTH_TOLERANCE: f64 = 1e-9;

/// Which side of the chain the wall lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinSide {
    /// Inside a closed chain; to the left of an open one (seen along its direction).
    Inside,
    /// Outside a closed chain; to the right of an open one.
    Outside,
    /// Half the thickness each side.
    Both,
}

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// The profile of a wall of `thickness` along `chain` (lines and arcs joined end to end,
/// closed when the last ends where the first starts), on `side` of it.
pub fn thin_profile(
    chain: &[ProfileEdge],
    side: ThinSide,
    thickness: f64,
) -> KernelResult<Profile> {
    if !(thickness.is_finite() && thickness > LINEAR_TOLERANCE && thickness <= MAX_COORDINATE) {
        return Err(invalid(format!(
            "the wall thickness must be a finite length with {LINEAR_TOLERANCE} < thickness <= {MAX_COORDINATE} mm"
        )));
    }
    if chain.is_empty() {
        return Err(invalid("a thin profile needs at least one line or arc"));
    }
    if chain.len() > MAX_THIN_EDGES {
        return Err(invalid(format!(
            "a thin profile may have at most {MAX_THIN_EDGES} edges"
        )));
    }
    for (k, e) in chain.iter().enumerate() {
        match e {
            ProfileEdge::Line { a, b } => {
                if !(a.is_valid() && b.is_valid()) || a.distance(*b) <= LINEAR_TOLERANCE {
                    return Err(invalid(format!("edge {k} of the chain has no length")));
                }
            }
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                if !(centre.is_valid()
                    && radius.is_finite()
                    && *radius > LINEAR_TOLERANCE
                    && *radius <= MAX_COORDINATE
                    && start_angle.is_finite()
                    && sweep.is_finite()
                    && sweep.abs() > ANGULAR_TOLERANCE
                    && sweep.abs() <= std::f64::consts::TAU + ANGULAR_TOLERANCE)
                {
                    return Err(invalid(format!("edge {k} of the chain is not a valid arc")));
                }
            }
            ProfileEdge::Spline(_) => {
                return Err(KernelError::Unsupported(
                    "a thin wall along a spline or ellipse: the offset of such a curve is not a spline, so no exact profile holds it; draw the path with lines and arcs".into(),
                ));
            }
        }
    }
    for k in 1..chain.len() {
        let (Some(p), Some(c)) = (chain.get(k - 1), chain.get(k)) else {
            continue;
        };
        if p.end().distance(c.start()) > PROFILE_JOIN_TOLERANCE {
            return Err(invalid(format!(
                "edge {k} of the chain does not start where edge {} ends; give the curves in order along the chain",
                k - 1
            )));
        }
    }
    let (first, last) = match (chain.first(), chain.last()) {
        (Some(f), Some(l)) => (f, l),
        _ => return Err(invalid("a thin profile needs at least one line or arc")),
    };
    let closed = last.end().distance(first.start()) <= PROFILE_JOIN_TOLERANCE;
    let t = thickness;
    if closed {
        let area: f64 = chain.iter().map(ProfileEdge::area_term).sum();
        if !area.is_finite() || area.abs() <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
            return Err(invalid("the closed chain encloses no area"));
        }
        // Left of a counter-clockwise loop is its inside.
        let ccw = area > 0.0;
        let (inward, outward) = match side {
            ThinSide::Inside => (t, 0.0),
            ThinSide::Outside => (0.0, t),
            ThinSide::Both => (0.5 * t, 0.5 * t),
        };
        let (left, right) = if ccw {
            (inward, outward)
        } else {
            (outward, inward)
        };
        let a = offset_chain(chain, true, left)?;
        let b = offset_chain(chain, true, -right)?;
        // The loop toward the outside is the outer loop.
        let (outer, hole) = if ccw { (b, a) } else { (a, b) };
        let p = Profile::new(outer, vec![hole]);
        check(&p)?;
        return Ok(p);
    }
    let (left, right) = match side {
        ThinSide::Inside => (t, 0.0),
        ThinSide::Outside => (0.0, t),
        ThinSide::Both => (0.5 * t, 0.5 * t),
    };
    let a = offset_chain(chain, false, left)?;
    let b = offset_chain(chain, false, -right)?;
    let (Some(a0), Some(a1), Some(b0), Some(b1)) = (
        a.first().map(ProfileEdge::start),
        a.last().map(ProfileEdge::end),
        b.first().map(ProfileEdge::start),
        b.last().map(ProfileEdge::end),
    ) else {
        return Err(KernelError::InvalidResult(
            "a thin profile lost its edges".into(),
        ));
    };
    let mut lp = a;
    lp.push(ProfileEdge::line(a1, b1));
    lp.extend(b.iter().rev().map(ProfileEdge::reversed));
    lp.push(ProfileEdge::line(b0, a0));
    let p = Profile::new(lp, Vec::new());
    check(&p)?;
    Ok(p)
}

fn check(p: &Profile) -> KernelResult<()> {
    p.check().map(|_| ()).map_err(|e| {
        invalid(format!(
            "the wall overlaps itself or vanishes ({e}); use a thinner wall, or the other side"
        ))
    })
}

/// The unit left normal of an edge at its start (`at_end` false) or end.
fn left_normal(e: &ProfileEdge, at_end: bool) -> Vec2 {
    e.tangent_at_fraction(if at_end { 1.0 } else { 0.0 }).perp()
}

/// `chain` moved by `d` to its left (negative: to its right), edge for edge.
fn offset_chain(chain: &[ProfileEdge], closed: bool, d: f64) -> KernelResult<Vec<ProfileEdge>> {
    if d == 0.0 {
        return Ok(chain.to_vec());
    }
    let n = chain.len();
    let edge = |k: usize| {
        chain
            .get(k)
            .ok_or_else(|| KernelError::InvalidResult("a thin profile lost an edge".into()))
    };
    let is_arc = |e: &ProfileEdge| matches!(e, ProfileEdge::Arc { .. });
    // The moved start of every edge, and for an open chain the moved end of the last.
    let mut moved: Vec<Point2> = Vec::with_capacity(n + 1);
    for k in 0..n {
        let cur = edge(k)?;
        let v = cur.start();
        let prev = if k > 0 {
            Some(edge(k - 1)?)
        } else if closed {
            Some(edge(n - 1)?)
        } else {
            None
        };
        let Some(prev) = prev else {
            moved.push(v + left_normal(cur, false) * d);
            continue;
        };
        let (n1, n2) = (left_normal(prev, true), left_normal(cur, false));
        let smooth = n1.cross(n2).abs() <= SMOOTH_TOLERANCE && n1.dot(n2) > 0.0;
        let step = if smooth {
            // Along an arc's own radius, so the moved arc starts or ends exactly there.
            if is_arc(prev) && !is_arc(cur) { n1 } else { n2 }
        } else if is_arc(cur) || is_arc(prev) {
            return Err(KernelError::Unsupported(format!(
                "a thin wall needs every corner at an arc to be smooth (tangent), but the corner at the start of edge {k} is sharp"
            )));
        } else {
            let denom = 1.0 + n1.dot(n2);
            if denom <= ANGULAR_TOLERANCE.sqrt() {
                return Err(invalid(format!(
                    "the chain folds back on itself at the start of edge {k}, which a wall cannot follow"
                )));
            }
            (n1 + n2) * (1.0 / denom)
        };
        moved.push(v + step * d);
    }
    if !closed {
        let last = edge(n - 1)?;
        moved.push(last.end() + left_normal(last, true) * d);
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let vanish = || {
            invalid(format!(
                "the wall makes edge {k} of the chain vanish or turn round; use a thinner wall"
            ))
        };
        let next = if closed { (k + 1) % n } else { k + 1 };
        match edge(k)? {
            ProfileEdge::Line { a, b } => {
                let (Some(a2), Some(b2)) = (moved.get(k), moved.get(next)) else {
                    return Err(KernelError::InvalidResult(
                        "a thin profile lost a point".into(),
                    ));
                };
                let along = (*b2 - *a2).dot(*b - *a) / (*b - *a).length().max(LINEAR_TOLERANCE);
                if !along.is_finite()
                    || along <= LINEAR_TOLERANCE
                    || !a2.is_valid()
                    || !b2.is_valid()
                {
                    return Err(vanish());
                }
                out.push(ProfileEdge::Line { a: *a2, b: *b2 });
            }
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                let r = radius - d * sweep.signum();
                if !r.is_finite() || r <= LINEAR_TOLERANCE {
                    return Err(vanish());
                }
                out.push(ProfileEdge::Arc {
                    centre: *centre,
                    radius: r,
                    start_angle: *start_angle,
                    sweep: *sweep,
                });
            }
            ProfileEdge::Spline(_) => {
                return Err(KernelError::Unsupported(
                    "a thin wall along a spline or ellipse".into(),
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extrude, polygon_edges};
    use chamfera_geom::Frame;
    use std::f64::consts::PI;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn rect() -> Vec<ProfileEdge> {
        polygon_edges(&[
            Point2::new(0.0, 0.0),
            Point2::new(50.0, 0.0),
            Point2::new(50.0, 30.0),
            Point2::new(0.0, 30.0),
        ])
    }

    #[test]
    fn a_closed_rectangle_wall_has_the_area_between_its_offsets() {
        // Inside, t = 2: 50 x 30 less 46 x 26.
        let p = thin_profile(&rect(), ThinSide::Inside, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 1500.0 - 46.0 * 26.0));
        // Outside: 54 x 34 less 50 x 30.
        let p = thin_profile(&rect(), ThinSide::Outside, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 54.0 * 34.0 - 1500.0));
        // Both: 52 x 32 less 48 x 28.
        let p = thin_profile(&rect(), ThinSide::Both, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 52.0 * 32.0 - 48.0 * 28.0));
        // A clockwise loop has the same inside.
        let mut cw: Vec<ProfileEdge> = rect().iter().rev().map(ProfileEdge::reversed).collect();
        let p = thin_profile(&cw, ThinSide::Inside, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 1500.0 - 46.0 * 26.0));
        // A wall as thick as half the short side leaves nothing inside.
        cw.truncate(4);
        assert!(thin_profile(&cw, ThinSide::Inside, 15.0).is_err());
        // Extruded 10, the tube of 1500 - 1196 = 304 mm² is 3040 mm³.
        let p = thin_profile(&rect(), ThinSide::Inside, 2.0).unwrap();
        let b = extrude(&Frame::XY, &p, 10.0).unwrap();
        let r = b.validate().unwrap();
        assert_eq!(r.genus, 1);
        assert!(close(b.volume().unwrap(), 3040.0));
    }

    #[test]
    fn a_circle_wall_is_an_annulus() {
        let c = vec![ProfileEdge::circle(Point2::new(0.0, 0.0), 10.0)];
        let p = thin_profile(&c, ThinSide::Outside, 1.0).unwrap();
        assert!(close(p.area().unwrap(), PI * (121.0 - 100.0)));
        let p = thin_profile(&c, ThinSide::Both, 2.0).unwrap();
        assert!(close(p.area().unwrap(), PI * (121.0 - 81.0)));
        assert!(thin_profile(&c, ThinSide::Inside, 10.0).is_err());
    }

    #[test]
    fn an_open_chain_wall_is_its_length_times_its_thickness() {
        // An L: 40 along x then 20 up y, t = 2 to the left (inside the turn). The left
        // offset lines meet at the miter (38, 2): the wall is the L's area
        // 40 x 2 + 2 x 18 = 116 (the region between the L and its offset).
        let l = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0)),
            ProfileEdge::line(Point2::new(40.0, 0.0), Point2::new(40.0, 20.0)),
        ];
        let p = thin_profile(&l, ThinSide::Inside, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 40.0 * 2.0 + 2.0 * 18.0));
        // To the right (outside the turn): 42 x 2 + 2 x 20 = 124.
        let p = thin_profile(&l, ThinSide::Outside, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 42.0 * 2.0 + 2.0 * 20.0));
        // Both sides: 41 x 2 + 2 x 19 = 120, the centre line's length times t.
        let p = thin_profile(&l, ThinSide::Both, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 120.0));
        // A line and a tangent quarter arc of radius 10 about (40, 10): t = 2 centred
        // gives 2 x (40 + 10π/2), the arc's annulus sector being π/4 (11² - 9²) = 20π/2.
        let j = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0)),
            ProfileEdge::Arc {
                centre: Point2::new(40.0, 10.0),
                radius: 10.0,
                start_angle: -PI / 2.0,
                sweep: PI / 2.0,
            },
        ];
        let p = thin_profile(&j, ThinSide::Both, 2.0).unwrap();
        assert!(close(p.area().unwrap(), 80.0 + 10.0 * PI));
        // Thicker than the arc's radius on its inside: refused.
        assert!(thin_profile(&j, ThinSide::Inside, 10.5).is_err());
    }

    #[test]
    fn hostile_thin_profiles_are_refused() {
        let l = vec![ProfileEdge::line(
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
        )];
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-12, 1e300] {
            assert!(thin_profile(&l, ThinSide::Both, t).is_err(), "{t}");
        }
        assert!(thin_profile(&[], ThinSide::Both, 1.0).is_err());
        // Not joined end to end.
        let gap = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
            ProfileEdge::line(Point2::new(11.0, 0.0), Point2::new(20.0, 0.0)),
        ];
        assert!(thin_profile(&gap, ThinSide::Both, 1.0).is_err());
        // Folding straight back.
        let back = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
            ProfileEdge::line(Point2::new(10.0, 0.0), Point2::new(0.0, 0.0)),
        ];
        assert!(thin_profile(&back, ThinSide::Both, 1.0).is_err());
        // A sharp corner at an arc.
        let sharp = vec![
            ProfileEdge::line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
            ProfileEdge::Arc {
                centre: Point2::new(10.0, 5.0),
                radius: 5.0,
                start_angle: 0.0,
                sweep: PI / 2.0,
            },
        ];
        assert!(thin_profile(&sharp, ThinSide::Both, 1.0).is_err());
        let degenerate = vec![ProfileEdge::line(
            Point2::new(1.0, 1.0),
            Point2::new(1.0, 1.0),
        )];
        assert!(thin_profile(&degenerate, ThinSide::Both, 1.0).is_err());
        let nan = vec![ProfileEdge::line(
            Point2::new(f64::NAN, 1.0),
            Point2::new(1.0, 1.0),
        )];
        assert!(thin_profile(&nan, ThinSide::Both, 1.0).is_err());
    }
}

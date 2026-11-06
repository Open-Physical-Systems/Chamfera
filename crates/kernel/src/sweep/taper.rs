//! Tapered extrusion: the profile shrinks (or grows) linearly toward the far end, so the
//! side faces lean in by the draft angle.
//!
//! # Geometry
//! A taper of angle `θ` over a height `h` moves every edge of the profile by
//! `d = h tan θ` toward the material (to the left of each checked edge: the outer loop
//! runs counter-clockwise, holes clockwise), the classical constant-distance offset of a
//! planar region (Hoffmann, *Geometric and Solid Modeling* (1989), section 7.3 on offsets;
//! Patrikalakis and Maekawa, *Shape Interrogation* (2002), section 11.2 on offsets of
//! planar curves). A positive angle shrinks the profile toward the far end, a negative
//! one grows it. Between the two ends every cross-section is the offset by `z tan θ`, so:
//!
//! - a line sweeps the plane through it and its offset copy (a tilted plane);
//! - an arc or circle sweeps the cone about its centre's normal line with radius `r` at
//!   the sketch plane and `r ∓ d` at the far end (half-angle `atan(∓d / h)`, `∓` by the
//!   arc's sense);
//! - a corner between two lines moves to the miter point `v + d (n₁ + n₂) / (1 + n₁·n₂)`
//!   (`n₁`, `n₂` the unit left normals of the two edges at the corner: the only point at
//!   distance `d` from both offset lines), so the strut joining it to `v` lies in both
//!   side planes and is a straight segment;
//! - a smooth (tangent) corner at an arc moves along the common normal by `d`: the strut
//!   is then a generator of the cone and lies in the tangent plane or the other cone.
//!
//! A sharp corner at an arc would make the strut a conic section (the intersection of a
//! tilted plane and a cone, or of two cones), which the kernel does not represent yet:
//! such profiles are refused with [`KernelError::Unsupported`].
//!
//! # Refusals
//! The far profile is rebuilt from the moved edges and passed through the same checks as
//! any profile ([`crate::Profile`]): a line that would shrink to nothing or turn round, an
//! arc whose radius would reach zero, a loop that would cross itself or another loop, or a
//! loop that turns inside out, all refuse the taper. The offsets at a quarter, half and
//! three quarters of the height are checked too, so a profile that collides part-way is
//! refused even when its far end would happen to look valid.

use crate::profile::{CheckedLoop, CheckedProfile, ProfileEdge};
use crate::{KernelError, KernelResult, Profile};
use chamfera_geom::{ANGULAR_TOLERANCE, LINEAR_TOLERANCE, Point2, Vec2};

/// The steepest taper allowed: 89 degrees. At 90 degrees the side faces would lie in the
/// sketch plane and the offset would be infinite.
pub const MAX_TAPER: f64 = 89.0 * std::f64::consts::PI / 180.0;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

fn loop_label(l: &CheckedLoop) -> String {
    match l.hole {
        None => "the outer loop".into(),
        Some(j) => format!("hole {j}"),
    }
}

/// The unit left normal of an edge at a fraction `f` along it (toward the material).
fn left_normal(e: &ProfileEdge, f: f64) -> Vec2 {
    e.tangent_at_fraction(f).perp()
}

/// The far profile of a tapered extrusion: every edge of `p` moved by `d` toward the
/// material (see the module documentation), checked as a profile of its own. Checked
/// edge `k` of each loop of the result is the moved copy of checked edge `k` of the same
/// loop of `p`.
pub(super) fn offset_profile(p: &CheckedProfile, d: f64) -> KernelResult<CheckedProfile> {
    let outer = offset_loop(&p.outer, d)?;
    let holes = p
        .holes
        .iter()
        .map(|h| offset_loop(h, d))
        .collect::<KernelResult<Vec<_>>>()?;
    let checked = Profile::new(outer, holes).check().map_err(|e| {
        invalid(format!(
            "the taper makes the profile overlap itself or vanish toward the far end ({e}); use a smaller angle or distance"
        ))
    })?;
    let same = |a: &CheckedLoop, b: &CheckedLoop| {
        a.len() == b.len() && b.edges.iter().enumerate().all(|(k, e)| e.source == k)
    };
    let all_same = same(&p.outer, &checked.outer)
        && checked.holes.len() == p.holes.len()
        && p.holes.iter().zip(&checked.holes).all(|(a, b)| same(a, b));
    if !all_same {
        return Err(invalid(
            "the taper turns the profile inside out toward the far end; use a smaller angle or distance",
        ));
    }
    Ok(checked)
}

/// One loop's edges moved by `d` to their left, in the same order.
fn offset_loop(l: &CheckedLoop, d: f64) -> KernelResult<Vec<ProfileEdge>> {
    let n = l.len();
    let edge = |k: usize| {
        l.edges
            .get(k % n)
            .map(|e| e.edge)
            .ok_or_else(|| KernelError::InvalidResult("the taper lost an edge".into()))
    };
    let mut moved: Vec<Point2> = Vec::with_capacity(n);
    for k in 0..n {
        let (prev, cur) = (edge(k + n - 1)?, edge(k)?);
        let (n1, n2) = (left_normal(&prev, 1.0), left_normal(&cur, 0.0));
        let is_arc = |e: &ProfileEdge| matches!(e, ProfileEdge::Arc { .. });
        let smooth = n1.cross(n2).abs() <= ANGULAR_TOLERANCE && n1.dot(n2) > 0.0;
        let v = l.vertex(k);
        let step = if is_arc(&cur) && smooth {
            // Along the arc's own radius, so the far arc starts exactly there.
            n2
        } else if is_arc(&prev) && smooth {
            n1
        } else if is_arc(&cur) || is_arc(&prev) {
            return Err(KernelError::Unsupported(format!(
                "a taper needs every corner at an arc to be smooth (tangent), but vertex {} of {} is sharp",
                l.edges.get(k).map_or(k, |e| e.vertex_source),
                loop_label(l)
            )));
        } else {
            let denom = 1.0 + n1.dot(n2);
            if denom <= ANGULAR_TOLERANCE {
                return Err(invalid(format!(
                    "{} folds back on itself at vertex {}, which a taper cannot offset",
                    loop_label(l),
                    l.edges.get(k).map_or(k, |e| e.vertex_source)
                )));
            }
            (n1 + n2) * (1.0 / denom)
        };
        moved.push(v + step * d);
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let source = l.edges.get(k).map_or(k, |e| e.source);
        let vanish = || {
            invalid(format!(
                "the taper makes edge {source} of {} vanish toward the far end; use a smaller angle or distance",
                loop_label(l)
            ))
        };
        match edge(k)? {
            ProfileEdge::Line { a, b } => {
                let (a2, b2) = (moved[k], moved[(k + 1) % n]);
                let along = (b2 - a2).dot(b - a) / (b - a).length().max(LINEAR_TOLERANCE);
                if !along.is_finite() || along <= LINEAR_TOLERANCE {
                    return Err(vanish());
                }
                out.push(ProfileEdge::Line { a: a2, b: b2 });
            }
            ProfileEdge::Arc {
                centre,
                radius,
                start_angle,
                sweep,
            } => {
                // Left of a counter-clockwise arc is its centre: the radius shrinks.
                let r = radius - d * sweep.signum();
                if !r.is_finite() || r <= LINEAR_TOLERANCE {
                    return Err(vanish());
                }
                out.push(ProfileEdge::Arc {
                    centre,
                    radius: r,
                    start_angle,
                    sweep,
                });
            }
        }
    }
    Ok(out)
}

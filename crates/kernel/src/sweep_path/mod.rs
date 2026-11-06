//! Sweeps along a path (rung K12): [`sweep_path`] moves a planar [`Profile`] along a
//! tangent-continuous chain of lines and circular arcs ([`PathSegment`]) into a valid
//! solid [`Body`] with exact surfaces.
//!
//! # Orientation
//! The profile is kept perpendicular to the path by a rotation-minimising frame (Wang,
//! Jüttler, Zheng and Liu, "Computation of rotation minimizing frames", *ACM Transactions
//! on Graphics* 27(1), 2008; Bishop, "There is more than one way to frame a curve",
//! *American Mathematical Monthly* 82(3), 1975). Along a line such a frame is translated;
//! along a circular arc it is rotated about the arc's axis, because the arc's normal
//! plane turns about that axis and a rotation about it has no twist about the tangent.
//! Both are exact here, so every piece is an exact rigid sweep of the profile.
//!
//! # Construction
//! The path is split into pieces: one per line, one per arc (two for an arc of more than
//! a half turn). The frame starts as the profile's sketch frame, which must be
//! perpendicular to the path where it starts, and each piece is built from the frame it
//! is handed: a line by [`extrude`](crate::extrude) along the frame's normal by the
//! line's length (planes and cylinders), an arc by [`revolve`](crate::revolve) about the
//! arc's axis by its angle (planes, cylinders, cones, spheres and tori, by the profile
//! edge's position against the axis, as for any revolve: a circle swept along an arc is
//! a torus piece). The frame is then moved like the profile and handed to the next
//! piece. Consecutive pieces are glued across their shared cap (see `stitch`), so the
//! result needs no surface intersection, and the whole is checked with
//! [`Body::validate`].
//!
//! # What is refused
//! - A path that is empty, branches, is closed, or does not start on the profile's plane;
//!   a profile plane not perpendicular to the path there.
//! - Corners: consecutive segments whose tangents differ by more than
//!   [`PATH_TANGENT_TOLERANCE`]. Mitred corners are not built.
//! - An arc whose radius the profile reaches: the profile must stay strictly on the
//!   outside of the arc's axis, or the swept piece would fold through itself.
//! - Any sweep that might cross itself. Every swept point lies within `ρ` of the path
//!   point it was swept from, `ρ` being the profile's largest distance from the path's
//!   start, so two pieces that are not neighbours are disjoint when their path segments
//!   are more than `2ρ` apart; the distance is bounded from below with polylines and their
//!   sagitta. Neighbouring pieces cannot cross: each piece of a line or of an arc of at
//!   most a half turn lies on the far side of its start plane and the near side of its
//!   end plane. The test is conservative: a sweep that passes close to itself without
//!   touching is refused too.
//!
//! # Names
//! With `k` the piece index (from 0, along the path) and `i` the profile element as in
//! the sweeps of `sweep` (prefixed `hole j ` for hole loops):
//!
//! | Entity | Name |
//! |---|---|
//! | the cap at the path's start, and at its end | `start`, `end` |
//! | face swept by edge `i` along piece `k` | `segment k side i` |
//! | edge and vertex `i` at the start, at the end | `start i`, `end i` |
//! | edge and vertex `i` where piece `k - 1` meets piece `k` | `joint k i` |
//! | edge swept by vertex `i` along piece `k` | `segment k sweep i` |

mod path;
mod stitch;
#[cfg(test)]
mod tests;

pub use path::{MAX_PATH_SEGMENTS, PATH_TANGENT_TOLERANCE, PathSegment};

use crate::profile::CheckedProfile;
use crate::{Body, KernelError, KernelResult, Line2, Profile, ProfileEdge};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Point3, Transform, Vec2};
use stitch::List;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// `profile` (in `frame`'s plane coordinates) swept along `path`. The path's segments may
/// come in any order and direction; they must join into one open, tangent-continuous
/// chain, one of whose ends lies on the profile's plane, perpendicular to it. The sweep
/// starts at that end (at the end nearer the profile when both lie on the plane). See
/// the module documentation for the surfaces, the refusals and the names.
pub fn sweep_path(frame: &Frame, profile: &Profile, path: &[PathSegment]) -> KernelResult<Body> {
    let checked = profile.check()?;
    let on_plane = |p: Point3| frame.z().dot(p - frame.origin()).abs() <= LINEAR_TOLERANCE;
    let centre = frame.from_plane(profile_centre(&checked));
    let chain = path::chain(path, on_plane, centre)?;
    let Some(first) = chain.first() else {
        return Err(invalid("the path needs at least one line or arc"));
    };
    let p0 = first.start();
    let t0 = first
        .tangent_at(0.0)
        .ok_or_else(|| invalid("the path has no direction where it starts"))?;
    if frame.z().cross(t0).length() > PATH_TANGENT_TOLERANCE {
        return Err(invalid(
            "the profile's plane must be perpendicular to the path where the path starts",
        ));
    }
    // Work in a frame whose normal is the path's direction.
    let (mut frame, profile, checked) = if frame.z().dot(t0) > 0.0 {
        (*frame, profile.clone(), checked)
    } else {
        let m = profile.mirrored();
        let c = m.check()?;
        (frame.flipped(), m, c)
    };
    let rho = reach(&checked, frame.to_plane(p0));
    // Pieces, with tangency at every joint.
    let pieces: Vec<PathSegment> = chain.iter().flat_map(PathSegment::halves).collect();
    for w in pieces.windows(2) {
        let [a, b] = w else { continue };
        let (Some(ta), Some(tb)) = (a.tangent_at(1.0), b.tangent_at(0.0)) else {
            return Err(invalid("a path segment has no direction"));
        };
        if ta.cross(tb).length() > PATH_TANGENT_TOLERANCE || ta.dot(tb) <= 0.0 {
            let p = a.end();
            return Err(invalid(format!(
                "the path turns a corner at ({:.3}, {:.3}, {:.3}): its lines and arcs must meet tangentially",
                p.x, p.y, p.z
            )));
        }
    }
    let n = pieces.len();
    let mut acc: Option<List> = None;
    for (k, piece) in pieces.iter().enumerate() {
        let (body, next_frame, near, far) = build_piece(&frame, &profile, &checked, piece)?;
        let mut list = List::of(&body)?;
        rename(&mut list, k, n);
        acc = Some(match acc {
            None => list,
            Some(prev) => {
                let end = prev.face_named(FAR_CAP)?;
                let start = list.face_named(near)?;
                prev.glue(end, list, start)?
            }
        });
        // The far cap is found by its name in the next round.
        if let Some(a) = acc.as_mut()
            && let Ok(f) = a.face_named(far)
            && let Some(face) = a.faces.get_mut(f)
        {
            face.name = Some(if k + 1 == n { "end" } else { FAR_CAP }.to_owned());
        }
        frame = next_frame;
    }
    // After the pieces, so a profile too large for an arc is reported as such.
    for (i, a) in pieces.iter().enumerate() {
        for b in pieces.iter().skip(i + 2) {
            if path::distance_lower_bound(a, b) <= 2.0 * rho + LINEAR_TOLERANCE {
                return Err(invalid(
                    "the sweep may cross itself: two parts of the path come within twice the profile's size of each other",
                ));
            }
        }
    }
    let list = acc.ok_or_else(|| invalid("the path needs at least one line or arc"))?;
    let body = list.into_body()?;
    body.validate()?;
    Ok(body)
}

/// The temporary name of the open end while the pieces are glued.
const FAR_CAP: &str = "far cap (being swept)";

/// One piece in `frame`: the body, the frame at its far end, and the names of its near
/// and far caps after [`rename`].
fn build_piece(
    frame: &Frame,
    profile: &Profile,
    checked: &CheckedProfile,
    piece: &PathSegment,
) -> KernelResult<(Body, Frame, &'static str, &'static str)> {
    match *piece {
        PathSegment::Line { .. } => {
            let d = piece.length();
            let body = crate::extrude(frame, profile, d)?;
            let next = frame
                .with_origin(frame.origin() + frame.z() * d)
                .ok_or_else(|| invalid("the sweep must lie within the working range"))?;
            Ok((body, next, "piece bottom", "piece top"))
        }
        PathSegment::Arc {
            centre,
            axis,
            start,
            angle,
        } => {
            // The axis in the profile's plane (it lies there for a tangent path; the
            // projection removes rounding).
            let q2 = frame.to_plane(centre);
            let d2 = Vec2::new(axis.dot(frame.x()), axis.dot(frame.y()))
                .normalized()
                .ok_or_else(|| invalid("an arc's axis is not in the profile's plane"))?;
            let r = frame.to_plane(start).distance(q2);
            // The profile must not reach the axis: its extent toward the centre, measured
            // from the path point, must stay below the radius.
            let toward = (q2 - frame.to_plane(start))
                .normalized()
                .ok_or_else(|| invalid("an arc of the path has no radius"))?;
            let (_, hi) = extent(checked, frame.to_plane(start), toward);
            if hi >= r - LINEAR_TOLERANCE {
                return Err(invalid(format!(
                    "the profile reaches {hi:.3} mm toward the centre of a path arc of radius {r:.3} mm: it must stay inside the radius"
                )));
            }
            let axis2 = Line2::new(q2, d2)?;
            let body = crate::revolve(frame, profile, &axis2, angle)?;
            let q3 = frame.from_plane(q2);
            let a3 = frame.vector_from_plane(d2);
            let next = Transform::rotation_about(q3, a3, angle)
                .and_then(|t| frame.transformed(&t))
                .ok_or_else(|| invalid("the sweep must lie within the working range"))?;
            Ok((body, next, "piece start cap", "piece end cap"))
        }
    }
}

/// Renames piece `k` of `n`'s faces, edges and vertices (see the module documentation).
/// Caps become `piece <cap>`, found again when gluing.
fn rename(list: &mut List, k: usize, n: usize) {
    for f in &mut list.faces {
        if let Some(name) = f.name.take() {
            f.name = Some(match name.as_str() {
                "bottom" | "top" | "start cap" | "end cap" => format!("piece {name}"),
                _ => entity_name(&name, k, n),
            });
        }
    }
    for e in &mut list.edges {
        e.name = e.name.take().map(|s| entity_name(&s, k, n));
    }
    for v in &mut list.vertices {
        v.name = v.name.take().map(|s| entity_name(&s, k, n));
    }
    if k == 0 {
        for f in &mut list.faces {
            if matches!(f.name.as_deref(), Some("piece bottom" | "piece start cap")) {
                f.name = Some("start".into());
            }
        }
    }
}

/// The swept name of a piece's edge, vertex or side face named `name`.
fn entity_name(name: &str, k: usize, n: usize) -> String {
    let (hole, rest) = match name.strip_prefix("hole ") {
        Some(r) => match r.split_once(' ') {
            Some((j, rest)) => (format!("hole {j} "), rest),
            None => (String::new(), name),
        },
        None => (String::new(), name),
    };
    let Some((role, i)) = rest.rsplit_once(' ') else {
        return name.to_owned();
    };
    match role {
        "bottom" | "start" if k == 0 => format!("{hole}start {i}"),
        // Glued away: the previous piece's far entities are kept.
        "bottom" | "start" => format!("{hole}joint {k} {i}"),
        "top" | "end" if k + 1 == n => format!("{hole}end {i}"),
        "top" | "end" => format!("{hole}joint {} {i}", k + 1),
        "side" | "sweep" => format!("segment {k} {hole}{role} {i}"),
        _ => format!("segment {k} {name}"),
    }
}

/// `(min, max)` of `n · (p - o)` over the profile.
fn extent(p: &CheckedProfile, o: Point2, n: Vec2) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for l in p.loops() {
        for e in &l.edges {
            let (a, b) = e.edge.extent_along(o, n);
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    (lo, hi)
}

/// An upper bound on the profile's distance from `c`: endpoints of lines, and
/// `|centre - c| + r` for arcs.
fn reach(p: &CheckedProfile, c: Point2) -> f64 {
    p.outer
        .edges
        .iter()
        .map(|e| match e.edge {
            ProfileEdge::Line { a, b } => a.distance(c).max(b.distance(c)),
            ProfileEdge::Arc { centre, radius, .. } => centre.distance(c) + radius,
        })
        .fold(0.0, f64::max)
}

/// The middle of the profile's bounding box.
fn profile_centre(p: &CheckedProfile) -> Point2 {
    let (x0, x1) = extent(p, Point2::ORIGIN, Vec2::X);
    let (y0, y1) = extent(p, Point2::ORIGIN, Vec2::Y);
    Point2::new(0.5 * (x0 + x1), 0.5 * (y0 + y1))
}

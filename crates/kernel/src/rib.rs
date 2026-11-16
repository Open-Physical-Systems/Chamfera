//! Ribs and webs: a thin wall grown from an open chain of lines and arcs in a plane until
//! it meets a solid, joined to it.
//!
//! # Geometry
//! The chain lies in the plane of `frame`; the wall is `thickness` thick across that
//! plane, half on each side (like a symmetric extrude), and grows in the plane, away from
//! the chain, along the depth direction `n`: the unit normal of the chord from the chain's
//! first point to its last, turned towards the solid (the side where points stepped out
//! from the chain's edges first fall inside it), or away with `flip`. Its region in the
//! plane is the chain, the two lines from its ends along `n`, and the line joining their
//! far ends, at a depth past the far
//! side of the solid's bounding box: a slab. That slab is clipped to the box around the
//! solid and the chain, so nothing is added past the solid's outer extent.
//!
//! - A **rib** stops at the first faces of the solid it meets: of the slab outside the
//!   solid (`slab − solid`, rung K7) only the lumps touching the chain are kept (each is
//!   found by a point just past the middle of a chain edge, [`classify_point`]).
//! - A **web** goes through: the whole clipped slab is joined, filling every gap of the
//!   solid it crosses (between two walls, under a cross bar).
//!
//! The kept material is joined to the solid ([`union`]); a wall that does not touch the
//! solid (the result would have another lump) is refused. Faces keep the names of the
//! extrude they come from (`side k` along chain edge `k`, `side n`, `side n+1` and
//! `side n+2` for the two ends and the far line, `bottom` and `top` across the thickness)
//! and of the clipping box (`limit k`), for the caller to prefix with its feature.
//!
//! [`classify_point`]: crate::classify_point

use crate::face_edit::lumps;
use crate::ssi::{PointInSolid, classify_point};
use crate::{
    Body, KernelError, KernelResult, MAX_THIN_EDGES, PROFILE_JOIN_TOLERANCE, Profile, ProfileEdge,
    extrude, extrude_symmetric, intersect, subtract, union,
};
use chamfera_geom::{Aabb, Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Point3, Vec2};

/// How far past a chain edge (mm) the point that finds a rib's lump lies: far above the
/// linear tolerance, far below any wall worth building.
const PROBE: f64 = 1e-3;

/// How far a rib's wall grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibExtent {
    /// Up to the first faces of the solid it meets (a rib).
    ToBody,
    /// Through the solid, filling every gap it crosses (a web).
    Through,
}

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

/// The point halfway along `e`.
fn middle(e: &ProfileEdge) -> Point2 {
    match *e {
        ProfileEdge::Arc {
            centre,
            radius,
            start_angle,
            sweep,
        } => {
            let a = start_angle + 0.5 * sweep;
            Point2::new(centre.x + radius * a.cos(), centre.y + radius * a.sin())
        }
        ref other => other.start().lerp(other.end(), 0.5),
    }
}

/// `body` with a rib (or web) of `thickness` grown from `chain` (lines and arcs in
/// `frame`'s plane coordinates, joined end to end, open). See the module documentation.
pub fn rib(
    body: &Body,
    frame: &Frame,
    chain: &[ProfileEdge],
    thickness: f64,
    extent: RibExtent,
    flip: bool,
) -> KernelResult<Body> {
    if !(thickness.is_finite() && thickness > LINEAR_TOLERANCE && thickness <= MAX_COORDINATE) {
        return Err(invalid(format!(
            "the rib thickness must be a finite length with {LINEAR_TOLERANCE} < thickness <= {MAX_COORDINATE} mm"
        )));
    }
    if chain.is_empty() || chain.len() > MAX_THIN_EDGES {
        return Err(invalid(format!(
            "a rib needs 1 to {MAX_THIN_EDGES} lines or arcs"
        )));
    }
    if chain.iter().any(|e| matches!(e, ProfileEdge::Spline(_))) {
        return Err(KernelError::Unsupported(
            "a rib along a spline: lines and arcs only for now".into(),
        ));
    }
    for w in chain.windows(2) {
        if let [a, b] = w
            && a.end().distance(b.start()) > PROFILE_JOIN_TOLERANCE
        {
            return Err(invalid(
                "the rib's lines and arcs must join end to end, in order",
            ));
        }
    }
    body.validate()
        .map_err(|e| invalid(format!("the body for the rib is not a valid solid: {e}")))?;
    if body.is_open() {
        return Err(invalid("a rib is joined to a solid, not to a surface"));
    }
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else {
        return Err(invalid("a rib needs a line or an arc"));
    };
    let (p0, pn) = (first.start(), last.end());
    let chord = pn - p0;
    let Some(along) = chord
        .normalized()
        .filter(|_| chord.length() > LINEAR_TOLERANCE)
    else {
        return Err(invalid(
            "the rib's chain must be open: its two ends are at the same point",
        ));
    };
    let bounds = body
        .bounding_box()
        .ok_or_else(|| invalid("the body for the rib is empty"))?;
    let corners = box_corners(&bounds);
    let reach = bounds.min.distance(bounds.max)
        + corners
            .iter()
            .map(|c| frame.from_plane(p0).distance(*c))
            .fold(0.0, f64::max);
    let mut n = along.perp();
    let ahead = first_hit(body, frame, chain, n, reach)?;
    let behind = first_hit(body, frame, chain, -n, reach)?;
    match (ahead, behind) {
        (None, None) if !flip => {
            return Err(invalid(
                "the rib's line does not face the solid: neither side of it meets the solid",
            ));
        }
        (None, Some(_)) => n = -n,
        (Some(a), Some(b)) if b < a => n = -n,
        _ => {}
    }
    if flip {
        n = -n;
    }
    // Points of the chain (ends and edge middles) for the depth and the clipping box.
    let mut chain_points: Vec<Point2> = chain.iter().flat_map(|e| [e.start(), middle(e)]).collect();
    chain_points.push(pn);
    let depth_of = |p: Point2| (p - p0).dot(n);
    let far = corners
        .iter()
        .map(|c| depth_of(frame.to_plane(*c)))
        .fold(f64::NEG_INFINITY, f64::max);
    let near = chain_points
        .iter()
        .map(|p| depth_of(*p))
        .fold(f64::INFINITY, f64::min);
    if far <= LINEAR_TOLERANCE {
        return Err(invalid(
            "the solid is not on that side of the rib's line: flip the direction",
        ));
    }
    let depth = far - near.min(0.0) + thickness + 1.0;
    let (a, b) = (pn + n * depth, p0 + n * depth);
    let mut edges = chain.to_vec();
    edges.push(ProfileEdge::line(pn, a));
    edges.push(ProfileEdge::line(a, b));
    edges.push(ProfileEdge::line(b, p0));
    let slab = extrude_symmetric(frame, &Profile::new(edges, Vec::new()), thickness).map_err(
        |e| match e {
            KernelError::InvalidInput(m) => invalid(format!(
                "the rib's region is not a simple shape (does the chain turn back across its direction?): {m}"
            )),
            other => other,
        },
    )?;
    let half = frame.z() * (0.5 * thickness);
    let mut hull = bounds;
    for p in &chain_points {
        let q = frame.from_plane(*p);
        grow(&mut hull, q + half);
        grow(&mut hull, q + -half);
    }
    let limit = box_body(&hull)?;
    let clipped = match intersect(&slab, &limit) {
        Ok(b) => b,
        Err(KernelError::EmptyResult) => {
            return Err(invalid("the rib does not reach the solid"));
        }
        Err(e) => return Err(e),
    };
    let tools: Vec<Body> = match extent {
        RibExtent::Through => vec![clipped],
        RibExtent::ToBody => {
            let gaps = match subtract(&clipped, body) {
                Ok(b) => b,
                Err(KernelError::EmptyResult) => {
                    return Err(invalid(
                        "the rib lies inside the solid: there is nothing to add",
                    ));
                }
                Err(e) => return Err(e),
            };
            let mut kept: Vec<Body> = Vec::new();
            let pieces = lumps(&gaps)?;
            let mut taken = vec![false; pieces.len()];
            for e in chain {
                let probe = frame.from_plane(middle(e) + n * PROBE);
                for (k, piece) in pieces.iter().enumerate() {
                    if taken.get(k).copied().unwrap_or(true) {
                        continue;
                    }
                    if classify_point(piece, probe)? == PointInSolid::Inside {
                        if let Some(t) = taken.get_mut(k) {
                            *t = true;
                        }
                        kept.push(piece.clone());
                    }
                }
            }
            if kept.is_empty() {
                return Err(invalid(
                    "the rib's line must lie outside the solid, with the solid on the rib's side",
                ));
            }
            kept
        }
    };
    let mut out = body.clone();
    for t in &tools {
        out = union(&out, t)?;
    }
    if out.shells().len() != body.shells().len() {
        return Err(invalid(
            "the rib does not meet the solid: flip its direction or move its line",
        ));
    }
    Ok(out)
}

/// Samples along each side of the chain, for which way it faces the solid.
const FACING_SAMPLES: usize = 256;

/// How far from the chain (mm) the solid first is along `n`, sampled from every edge's
/// middle at `reach / FACING_SAMPLES` steps; `None` when it is never met within `reach`.
/// It only picks the default side: the wall itself is cut by exact booleans.
fn first_hit(
    body: &Body,
    frame: &Frame,
    chain: &[ProfileEdge],
    n: Vec2,
    reach: f64,
) -> KernelResult<Option<f64>> {
    let step = reach / FACING_SAMPLES as f64;
    let mut best: Option<f64> = None;
    for e in chain.iter().take(FACING_SAMPLES) {
        let m = middle(e);
        for k in 1..=FACING_SAMPLES {
            let d = step * k as f64;
            if best.is_some_and(|b| d >= b) {
                break;
            }
            if classify_point(body, frame.from_plane(m + n * d))? == PointInSolid::Inside {
                best = Some(d);
                break;
            }
        }
    }
    Ok(best)
}

fn box_corners(b: &Aabb) -> [Point3; 8] {
    let (l, h) = (b.min, b.max);
    [
        Point3::new(l.x, l.y, l.z),
        Point3::new(h.x, l.y, l.z),
        Point3::new(l.x, h.y, l.z),
        Point3::new(h.x, h.y, l.z),
        Point3::new(l.x, l.y, h.z),
        Point3::new(h.x, l.y, h.z),
        Point3::new(l.x, h.y, h.z),
        Point3::new(h.x, h.y, h.z),
    ]
}

fn grow(b: &mut Aabb, p: Point3) {
    b.min = Point3::new(b.min.x.min(p.x), b.min.y.min(p.y), b.min.z.min(p.z));
    b.max = Point3::new(b.max.x.max(p.x), b.max.y.max(p.y), b.max.z.max(p.z));
}

/// The solid box `b`, every name of its faces, edges and vertices starting `limit `.
fn box_body(b: &Aabb) -> KernelResult<Body> {
    let frame = Frame::XY
        .with_origin(Point3::new(0.0, 0.0, b.min.z))
        .ok_or_else(|| invalid("the rib leaves the working range"))?;
    let rect = [
        Point2::new(b.min.x, b.min.y),
        Point2::new(b.max.x, b.min.y),
        Point2::new(b.max.x, b.max.y),
        Point2::new(b.min.x, b.max.y),
    ];
    let mut body = extrude(&frame, &Profile::polygon(&rect), b.max.z - b.min.z)?;
    for f in body.faces() {
        if let Some(role) = body.face_name(f)?.map(str::to_owned) {
            body.set_face_name(f, &format!("limit {role}"))?;
        }
    }
    for e in body.edges() {
        if let Some(role) = body.edge_name(e)?.map(str::to_owned) {
            body.set_edge_name(e, &format!("limit {role}"))?;
        }
    }
    for v in body.vertices() {
        if let Some(role) = body.vertex_name(v)?.map(str::to_owned) {
            body.set_vertex_name(v, &format!("limit {role}"))?;
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    //! Expected volumes are closed forms derived in the comments.
    use super::*;

    fn assert_close(what: &str, got: f64, want: f64) {
        assert!(
            (got - want).abs() <= 1e-6 * want.abs().max(1.0),
            "{what}: got {got}, want {want}"
        );
    }

    fn prism(points: &[(f64, f64)], h: f64) -> Body {
        let pts: Vec<Point2> = points.iter().map(|(x, y)| Point2::new(*x, *y)).collect();
        extrude(&Frame::XY, &Profile::polygon(&pts), h).unwrap()
    }

    /// An L-bracket 40 x 40 with 5 mm legs, 20 deep: (40·5 + 35·5)·20 = 7500 mm³.
    fn bracket() -> Body {
        prism(
            &[
                (0.0, 0.0),
                (40.0, 0.0),
                (40.0, 5.0),
                (5.0, 5.0),
                (5.0, 40.0),
                (0.0, 40.0),
            ],
            20.0,
        )
    }

    /// An H 40 x 40: two 5 mm posts and a cross bar 30 x 5 at y = 15..20, 20 deep:
    /// (2·5·40 + 30·5)·20 = 11000 mm³.
    fn h_shape() -> Body {
        prism(
            &[
                (0.0, 0.0),
                (5.0, 0.0),
                (5.0, 15.0),
                (35.0, 15.0),
                (35.0, 0.0),
                (40.0, 0.0),
                (40.0, 40.0),
                (35.0, 40.0),
                (35.0, 20.0),
                (5.0, 20.0),
                (5.0, 40.0),
                (0.0, 40.0),
            ],
            20.0,
        )
    }

    /// The plane z = 10, halfway up both shapes.
    fn mid_plane() -> Frame {
        Frame::XY.with_origin(Point3::new(0.0, 0.0, 10.0)).unwrap()
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> ProfileEdge {
        ProfileEdge::line(Point2::new(a.0, a.1), Point2::new(b.0, b.1))
    }

    fn volume(b: &Body) -> f64 {
        b.validate().unwrap();
        b.volume().unwrap()
    }

    #[test]
    fn a_rib_across_the_inside_corner_of_an_l_bracket_fills_the_triangle() {
        // The line (5,25) → (25,5) and the inner faces x = 5, y = 5 bound the right
        // triangle with legs 20 and 20: area 200, 4 thick: 800 mm³ more.
        let chain = [line((5.0, 25.0), (25.0, 5.0))];
        for extent in [RibExtent::Through, RibExtent::ToBody] {
            let out = rib(&bracket(), &mid_plane(), &chain, 4.0, extent, false).unwrap();
            assert_close("bracket + rib", volume(&out), 7500.0 + 800.0);
            assert_eq!(out.shells().len(), 1);
            // The rib's faces carry the extrude's names: the wall along the line is
            // `side 0`, the faces across the thickness `bottom` and `top`.
            let names: Vec<String> = out
                .faces()
                .into_iter()
                .filter_map(|f| out.face_name(f).ok().flatten().map(str::to_owned))
                .collect();
            assert!(names.iter().any(|n| n == "side 0"), "{names:?}");
            assert!(names.iter().any(|n| n == "top"), "{names:?}");
        }
    }

    #[test]
    fn a_rib_stops_at_the_cross_bar_and_a_web_goes_through_it() {
        // The line (5,35) → (35,35) between the posts: the rib fills down to the bar,
        // 30 x 15 x 4 = 1800; the web also fills under it, another 30 x 15 x 4.
        let chain = [line((5.0, 35.0), (35.0, 35.0))];
        let h = h_shape();
        let r = rib(&h, &mid_plane(), &chain, 4.0, RibExtent::ToBody, false).unwrap();
        assert_close("rib", volume(&r), 11000.0 + 1800.0);
        let w = rib(&h, &mid_plane(), &chain, 4.0, RibExtent::Through, false).unwrap();
        assert_close("web", volume(&w), 11000.0 + 3600.0);
        // Flipped, the rib grows up to the top of the posts: 30 x 5 x 4 = 600.
        let up = rib(&h, &mid_plane(), &chain, 4.0, RibExtent::ToBody, true).unwrap();
        assert_close("flipped", volume(&up), 11000.0 + 600.0);
    }

    #[test]
    fn hostile_ribs_are_refused() {
        let b = bracket();
        let p = mid_plane();
        let ok = [line((5.0, 25.0), (25.0, 5.0))];
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300] {
            assert!(rib(&b, &p, &ok, t, RibExtent::ToBody, false).is_err());
        }
        assert!(rib(&b, &p, &[], 4.0, RibExtent::ToBody, false).is_err());
        // Closed: its ends meet.
        let closed = [
            line((10.0, 10.0), (20.0, 10.0)),
            line((20.0, 10.0), (10.0, 10.0)),
        ];
        assert!(rib(&b, &p, &closed, 4.0, RibExtent::ToBody, false).is_err());
        // Not joined end to end.
        let gap = [
            line((5.0, 25.0), (10.0, 20.0)),
            line((11.0, 19.0), (25.0, 5.0)),
        ];
        assert!(rib(&b, &p, &gap, 4.0, RibExtent::ToBody, false).is_err());
        // Far to the right and turned away from the bracket: nothing to meet.
        let away = [line((100.0, 10.0), (100.0, 30.0))];
        let e = rib(&b, &p, &away, 4.0, RibExtent::ToBody, true).unwrap_err();
        assert!(e.to_string().contains("flip"), "{e}");
        // Inside the bracket's leg: nothing to add.
        let inside = [line((1.0, 10.0), (1.0, 30.0))];
        assert!(rib(&b, &p, &inside, 1.0, RibExtent::ToBody, false).is_err());
    }
}

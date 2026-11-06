//! Machining geometry read from the exact model: face boundaries, pocket regions, edge
//! chains and vertical holes, as loops of lines and arcs in XY.
//!
//! Only what 2.5D milling from above can cut is accepted: horizontal boundaries, pocket
//! floors facing up with vertical walls (planes or cylinders about vertical axes) and no
//! material overhanging them, holes along vertical axes. Anything else is refused with a
//! reason rather than machined wrongly.

use crate::CamError;
use crate::ops::{Hole, Region};
use crate::path2::{JOIN_TOLERANCE, Loop, Seg, distance_to, is_closed, signed_area, winding};
use chamfera_geom::{Point2, Point3, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId, FaceId, HalfEdgeId, Surface};
use std::f64::consts::TAU;

/// A direction is vertical (or horizontal) when its unit vector's other components are
/// below this: about 2e-7 degrees, far finer than any machine, coarse enough for
/// directions computed from rounded points.
const DIRECTION_TOLERANCE: f64 = 1e-9;

/// Heights closer than this (mm) are the same height.
const HEIGHT_TOLERANCE: f64 = 1e-6;

fn refused(s: String) -> CamError {
    CamError::Refused(s)
}

fn kernel(e: impl std::fmt::Display) -> CamError {
    CamError::Model(e.to_string())
}

fn face_label(body: &Body, f: FaceId) -> String {
    body.face_name(f).ok().flatten().map_or_else(
        || "an unnamed face".to_owned(),
        |n| format!("the face {n:?}"),
    )
}

/// The outward normal of a planar face, if it is planar.
fn plane_normal(body: &Body, f: FaceId) -> Result<Option<(Vec3, Point3)>, CamError> {
    match body.face_surface(f).map_err(kernel)? {
        Some((Surface::Plane(frame), same)) => {
            let n = if same { frame.z() } else { -frame.z() };
            Ok(Some((n, frame.origin())))
        }
        _ => Ok(None),
    }
}

/// The face's height when it is a flat horizontal face (facing up or down).
pub fn horizontal_face_z(body: &Body, f: FaceId) -> Result<(f64, bool), CamError> {
    let Some((n, o)) = plane_normal(body, f)? else {
        return Err(refused(format!("{} is not flat", face_label(body, f))));
    };
    if n.x.abs() > DIRECTION_TOLERANCE || n.y.abs() > DIRECTION_TOLERANCE {
        return Err(refused(format!(
            "{} is not horizontal",
            face_label(body, f)
        )));
    }
    Ok((o.z, n.z > 0.0))
}

/// The XY segment one half-edge runs along, and its height. Only horizontal lines and
/// arcs of circles about vertical axes.
pub fn half_edge_seg(body: &Body, h: HalfEdgeId) -> Result<(Seg, f64), CamError> {
    let e = body
        .half_edge_edge(h)
        .map_err(kernel)?
        .ok_or_else(|| refused("a face has a loop with no edge".into()))?;
    let a = body
        .vertex_point(body.half_edge_origin(h).map_err(kernel)?)
        .map_err(kernel)?;
    let b = body
        .vertex_point(body.half_edge_end(h).map_err(kernel)?)
        .map_err(kernel)?;
    let forward = body.edge_half_edges(e).map_err(kernel)?[0] == h;
    let (curve, t0, t1) = body
        .edge_curve(e)
        .map_err(kernel)?
        .ok_or_else(|| refused("an edge has no curve".into()))?;
    if (a.z - b.z).abs() > HEIGHT_TOLERANCE {
        return Err(refused("an edge of the boundary is not horizontal".into()));
    }
    let a2 = Point2::new(a.x, a.y);
    let b2 = Point2::new(b.x, b.y);
    match curve {
        Curve::Line(_) | Curve::Segment(_) => Ok((Seg::line(a2, b2), a.z)),
        Curve::Circle(_) | Curve::Arc(_) => {
            let (centre, normal) = match curve {
                Curve::Circle(c) => (c.center(), c.normal()),
                Curve::Arc(arc) => (arc.circle().center(), arc.circle().normal()),
                _ => return Err(refused("unexpected curve".into())),
            };
            if normal.x.abs() > DIRECTION_TOLERANCE || normal.y.abs() > DIRECTION_TOLERANCE {
                return Err(refused("an arc of the boundary is not horizontal".into()));
            }
            let c2 = Point2::new(centre.x, centre.y);
            let tangent = if forward { curve.d1(t0) } else { -curve.d1(t1) };
            let ccw = (a2 - c2).cross(chamfera_geom::Vec2::new(tangent.x, tangent.y)) > 0.0;
            Ok((Seg::arc_between(c2, a2, b2, ccw), a.z))
        }
        _ => Err(refused(
            "the boundary has an elliptical edge: only lines and arcs can be machined".into(),
        )),
    }
}

/// The loop's segments in loop order and their common height.
fn loop_segs(body: &Body, lp: chamfera_kernel::LoopId) -> Result<(Loop, f64), CamError> {
    let mut out = Vec::new();
    let mut z: Option<f64> = None;
    for h in body.loop_half_edges(lp).map_err(kernel)? {
        let (s, hz) = half_edge_seg(body, h)?;
        if z.is_some_and(|z| (z - hz).abs() > HEIGHT_TOLERANCE) {
            return Err(refused("the boundary is not at one height".into()));
        }
        z = Some(hz);
        out.push(s);
    }
    let z = z.ok_or_else(|| refused("an empty boundary".into()))?;
    if !is_closed(&out) {
        return Err(refused("the boundary does not close".into()));
    }
    Ok((out, z))
}

fn ccw(l: Loop) -> Loop {
    if signed_area(&l) < 0.0 {
        crate::path2::reverse_loop(&l)
    } else {
        l
    }
}

/// The outer boundary of a flat horizontal face, counter-clockwise from above, and its
/// height.
pub fn face_outline(body: &Body, f: FaceId) -> Result<(Loop, f64), CamError> {
    horizontal_face_z(body, f)?;
    let (l, z) = loop_segs(body, body.face_outer_loop(f).map_err(kernel)?)?;
    Ok((ccw(l), z))
}

/// Chains edges (in any order and direction) into one closed horizontal loop,
/// counter-clockwise from above, and its height.
pub fn edge_chain(body: &Body, edges: &[EdgeId]) -> Result<(Loop, f64), CamError> {
    let mut segs = Vec::new();
    let mut z: Option<f64> = None;
    for e in edges {
        let h = body.edge_half_edges(*e).map_err(kernel)?[0];
        let (s, hz) = half_edge_seg(body, h)?;
        if z.is_some_and(|z| (z - hz).abs() > HEIGHT_TOLERANCE) {
            return Err(refused("the edges are not at one height".into()));
        }
        z = Some(hz);
        segs.push(s);
    }
    let z = z.ok_or_else(|| refused("no edges were given".into()))?;
    let mut left = segs;
    let mut chain = vec![left.remove(0)];
    while !left.is_empty() {
        let end = chain.last().map_or(Point2::ORIGIN, Seg::end);
        let i = left.iter().position(|s| {
            s.start().distance(end) <= JOIN_TOLERANCE * 10.0
                || s.end().distance(end) <= JOIN_TOLERANCE * 10.0
        });
        let Some(i) = i else {
            return Err(refused(
                "the edges do not form one chain: pick edges that meet end to end".into(),
            ));
        };
        let s = left.remove(i);
        chain.push(if s.start().distance(end) <= JOIN_TOLERANCE * 10.0 {
            s
        } else {
            s.reversed()
        });
    }
    if !is_closed(&chain) {
        return Err(refused(
            "the edges do not close: a contour needs a closed chain".into(),
        ));
    }
    Ok((ccw(chain), z))
}

/// True when the face is a vertical wall: a vertical plane or a cylinder about a vertical
/// axis.
fn is_vertical(body: &Body, f: FaceId) -> Result<bool, CamError> {
    Ok(match body.face_surface(f).map_err(kernel)? {
        Some((Surface::Plane(frame), _)) => frame.z().z.abs() <= DIRECTION_TOLERANCE,
        Some((Surface::Cylinder(c), _)) => {
            let a = c.frame().z();
            a.x.abs() <= DIRECTION_TOLERANCE && a.y.abs() <= DIRECTION_TOLERANCE
        }
        _ => false,
    })
}

/// The highest and lowest points of a face (its vertices and edge midpoints).
fn face_points(body: &Body, f: FaceId) -> Result<Vec<Point3>, CamError> {
    let mut pts = Vec::new();
    for e in body.face_edges(f).map_err(kernel)? {
        for v in body.edge_vertices(e).map_err(kernel)? {
            pts.push(body.vertex_point(v).map_err(kernel)?);
        }
        if let Some((c, t0, t1)) = body.edge_curve(e).map_err(kernel)? {
            pts.push(c.point_at((t0 + t1) / 2.0));
        }
    }
    Ok(pts)
}

fn z_range(pts: &[Point3]) -> (f64, f64) {
    pts.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.z), hi.max(p.z))
        })
}

/// A pocket: its floor face and the region of it, with the height of its walls' tops.
#[derive(Debug, Clone, PartialEq)]
pub struct Pocket {
    pub region: Region,
    pub floor_z: f64,
    pub top_z: f64,
}

/// The pocket whose floor is `floor`: a flat face facing up, its outer edges each with a
/// vertical wall rising from it, nothing of the body hanging over it. Inner loops of the
/// floor are islands the tool keeps clear of.
pub fn pocket_region(body: &Body, floor: FaceId) -> Result<Pocket, CamError> {
    let (floor_z, up) = horizontal_face_z(body, floor)?;
    if !up {
        return Err(refused(format!(
            "{} faces down: a pocket floor must face up",
            face_label(body, floor)
        )));
    }
    let outer_lp = body.face_outer_loop(floor).map_err(kernel)?;
    let (outer, _) = loop_segs(body, outer_lp)?;
    let mut walls = vec![floor];
    let mut top_z = floor_z;
    for h in body.loop_half_edges(outer_lp).map_err(kernel)? {
        let Some(mate) = body.half_edge_mate(h).map_err(kernel)? else {
            continue;
        };
        let wall = body
            .loop_face(body.half_edge_loop(mate).map_err(kernel)?)
            .map_err(kernel)?;
        if !is_vertical(body, wall)? {
            return Err(refused(format!(
                "{} is not a vertical wall: only vertical walls can be pocketed for now",
                face_label(body, wall)
            )));
        }
        let (_, hi) = z_range(&face_points(body, wall)?);
        if hi <= floor_z + HEIGHT_TOLERANCE {
            return Err(refused(format!(
                "the pocket is open where {} goes down from the floor: use a contour",
                face_label(body, wall)
            )));
        }
        top_z = top_z.max(hi);
        if !walls.contains(&wall) {
            walls.push(wall);
        }
    }
    let mut islands = Vec::new();
    for lp in body.face_inner_loops(floor).map_err(kernel)? {
        let (l, _) = loop_segs(body, lp)?;
        for h in body.loop_half_edges(lp).map_err(kernel)? {
            if let Some(mate) = body.half_edge_mate(h).map_err(kernel)? {
                let wall = body
                    .loop_face(body.half_edge_loop(mate).map_err(kernel)?)
                    .map_err(kernel)?;
                if !is_vertical(body, wall)? {
                    return Err(refused(format!(
                        "{} round an island is not vertical",
                        face_label(body, wall)
                    )));
                }
                if !walls.contains(&wall) {
                    walls.push(wall);
                }
            }
        }
        islands.push(ccw(l));
    }
    let outer = ccw(outer);
    // Nothing may hang over the floor: no point of any other face strictly inside the
    // region (away from its boundary) and above the floor.
    let mut boundary = vec![outer.clone()];
    boundary.extend(islands.iter().cloned());
    for f in body.faces() {
        if walls.contains(&f) {
            continue;
        }
        for p in face_points(body, f)? {
            let q = Point2::new(p.x, p.y);
            let inside = winding(&outer, q) != 0
                && islands.iter().all(|i| winding(i, q) == 0)
                && distance_to(&boundary, q) > HEIGHT_TOLERANCE;
            if inside && p.z > floor_z + HEIGHT_TOLERANCE {
                return Err(refused(format!(
                    "{} overhangs the pocket (an undercut): it cannot be cut from above",
                    face_label(body, f)
                )));
            }
        }
    }
    Ok(Pocket {
        region: Region { outer, islands },
        floor_z,
        top_z,
    })
}

/// The vertical holes of a body: concave cylindrical faces about vertical axes, grouped
/// by axis and radius (a hole may be split into several faces), that go all the way
/// round. With `only`, just the holes those faces belong to.
pub fn find_holes(
    body: &Body,
    only: Option<&[FaceId]>,
) -> Result<Vec<(Hole, Vec<FaceId>)>, CamError> {
    // (centre, radius, lowest, highest, swept angle at the top, faces)
    type Group = (Point2, f64, f64, f64, f64, Vec<FaceId>);
    let mut groups: Vec<Group> = Vec::new();
    for f in body.faces() {
        let Some((Surface::Cylinder(c), same)) = body.face_surface(f).map_err(kernel)? else {
            continue;
        };
        let axis = c.frame().z();
        // A hole's surface faces its axis: the face is against the cylinder's outward
        // normal.
        if same || axis.x.abs() > DIRECTION_TOLERANCE || axis.y.abs() > DIRECTION_TOLERANCE {
            continue;
        }
        let o = c.frame().origin();
        let centre = Point2::new(o.x, o.y);
        let r = c.radius();
        let (lo, hi) = z_range(&face_points(body, f)?);
        // How far round the face goes: its circular edges at its top.
        let mut swept = 0.0;
        for e in body.face_edges(f).map_err(kernel)? {
            if let Some((Curve::Circle(_) | Curve::Arc(_), t0, t1)) =
                body.edge_curve(e).map_err(kernel)?
                && let Some((curve, _, _)) = body.edge_curve(e).map_err(kernel)?
                && (curve.point_at(t0).z - hi).abs() <= HEIGHT_TOLERANCE
            {
                swept += (t1 - t0).abs();
            }
        }
        match groups
            .iter_mut()
            .find(|g| g.0.distance(centre) <= JOIN_TOLERANCE && (g.1 - r).abs() <= JOIN_TOLERANCE)
        {
            Some(g) => {
                g.2 = g.2.min(lo);
                g.3 = g.3.max(hi);
                g.4 += swept;
                g.5.push(f);
            }
            None => groups.push((centre, r, lo, hi, swept, vec![f])),
        }
    }
    Ok(groups
        .into_iter()
        .filter(|g| g.4 >= TAU - 1e-6)
        .filter(|g| only.is_none_or(|o| g.5.iter().any(|f| o.contains(f))))
        .map(|(centre, r, lo, hi, _, faces)| {
            (
                Hole {
                    centre,
                    diameter: 2.0 * r,
                    top: hi,
                    bottom: lo,
                },
                faces,
            )
        })
        .collect())
}

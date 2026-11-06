//! Sweeps (rung K4): [`extrude`] and [`revolve`] turn a planar [`Profile`] into a valid
//! solid [`Body`].
//!
//! # Construction
//! Both follow Mantyla, *An Introduction to Solid Modeling* (1988), section 9.4
//! (translational and rotational sweeping with Euler operators), as `Body::prism` does:
//! MVFS and a chain of MEVs draw the outer loop as a strut tree and one MEF closes it
//! into a lamina of two faces (the far cap and the near cap). The far cap's loop is then
//! swept: an MEV per vertex raises a strut to the moved vertex and an MEF per edge cuts a
//! side face off. Each hole is drawn into the far cap (bridge MEV, chain, MEF making a
//! cap face, KEMR leaving a ring), its cap swept back to the near end and turned into a
//! ring of the near cap by KFMRH (genus + 1). A one-edge loop (a full circle) has one
//! vertex: its strut is the seam of a cylindrical or toroidal side face.
//!
//! A revolve by less than a full turn is the same with rotated copies and arc struts.
//! A vertex on the axis does not move, so it gets no strut and its edges' side faces are
//! triangles meeting at it (a cone's apex, a disc's centre, a sphere's pole); an edge
//! lying on the axis sweeps nothing and stays as the hinge between the two caps. A full
//! revolve has no caps: the lamina's two faces become one by KFMRH (around the axis) or
//! KEF (across an edge on the axis, with KEV for further edges on it), then each
//! off-axis vertex gets its circle by MEKR or MEF, splitting one side face off per
//! profile edge; the profile edges stay as the side faces' seams, except on a disc or
//! annulus, whose seam (a slit in a plane) is removed again by KEV (with the disc's
//! centre) or KEMR (leaving the annulus's inner circle as a ring).
//!
//! Geometry is attached after the topology: each edge gets its curve oriented along its
//! first half-edge, each face its surface with the orientation flag from the outward
//! normal `T × S` (profile tangent `T` in the loop's direction, sweep direction `S` at
//! the profile, which is the frame's normal for both sweeps). The result is checked with
//! [`Body::validate`]; any failure returns a [`KernelError`] and no body.
//!
//! # Surfaces of the side faces
//! Extrude: a line sweeps a plane, an arc or circle a cylinder. Revolve, by the profile
//! edge's position relative to the axis: a line parallel to it a cylinder, perpendicular
//! to it a plane (a disc or annulus), oblique a cone; an arc centred on the axis a sphere,
//! any other arc a torus (spindle or lemon-shaped when the arc's centre is near or beyond
//! the axis).
//!
//! # Names (the start of persistent naming, K8)
//! Every face, edge and vertex is named from the profile element that made it and its
//! role. `i` is the index of the edge in the input loop (or, for a vertex and the edge it
//! sweeps, the index of the input edge starting at that vertex); names from hole `j` start
//! with `hole j `:
//!
//! | Entity | Extrude | Revolve (partial) | Revolve (full turn) |
//! |---|---|---|---|
//! | caps | `bottom` (at the sketch plane), `top` | `start cap` (at the sketch plane), `end cap` | none |
//! | face swept by edge `i` | `side i` | `side i` | `side i` |
//! | edge `i` at the near end | `bottom i` | `start i` | `seam i` (none on a disc or annulus) |
//! | edge `i` at the far end | `top i` | `end i` | none |
//! | edge swept by vertex `i` | `sweep i` | `sweep i` (an arc) | `sweep i` (a circle) |
//! | edge `i` lying on the axis | — | `axis i` (shared by both caps) | removed |
//! | vertex `i` at the near end | `bottom i` | `start i` | `vertex i` |
//! | vertex `i` at the far end | `top i` | `end i` | none |
//! | vertex `i` on the axis | — | `axis i` | `axis i` (none at a disc's centre) |
//!
//! The engine prefixes these with the feature's id.

#[cfg(test)]
mod tests;

use crate::profile::{CheckedLoop, CheckedProfile, ProfileEdge};
use crate::topology::{Body, Curve, EdgeId, FaceId, HalfEdgeId, Surface, VertexId};
use crate::{KernelError, KernelResult, Line2, Profile, Segment3};
use chamfera_geom::{
    ANGULAR_TOLERANCE, Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Point3, Vec2, Vec3,
};
use std::f64::consts::TAU;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

fn internal(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

/// `profile` (in `frame`'s plane coordinates) extruded by `distance` along the frame's
/// normal; a negative distance extrudes the other way. Faces: `bottom` on the sketch
/// plane, `top`, and `side i` (planes for lines, cylinders for arcs and circles); see the
/// module documentation for every name.
pub fn extrude(frame: &Frame, profile: &Profile, distance: f64) -> KernelResult<Body> {
    if !(distance.is_finite()
        && distance.abs() > LINEAR_TOLERANCE
        && distance.abs() <= 2.0 * MAX_COORDINATE)
    {
        return Err(invalid(format!(
            "`distance` must be a finite length with {LINEAR_TOLERANCE} < |distance| <= {} mm",
            2.0 * MAX_COORDINATE
        )));
    }
    let (frame, profile) = if distance > 0.0 {
        (*frame, profile.check()?)
    } else {
        (frame.flipped(), profile.mirrored().check()?)
    };
    let motion = Motion::Translate(frame.z() * distance.abs());
    build_swept(&frame, &profile, &motion, &Names::extrude())
}

/// `profile` extruded by `distance` (its absolute value) centred on the sketch plane:
/// half on each side. Names as for [`extrude`], `bottom` being the face at
/// `-distance / 2` along the frame's normal.
pub fn extrude_symmetric(frame: &Frame, profile: &Profile, distance: f64) -> KernelResult<Body> {
    if !(distance.is_finite() && distance.abs() > LINEAR_TOLERANCE) {
        return Err(invalid(format!(
            "`distance` must be a finite length longer than {LINEAR_TOLERANCE} mm"
        )));
    }
    let half = distance.abs() * 0.5;
    let moved = frame
        .with_origin(frame.origin() + frame.z() * -half)
        .ok_or_else(|| invalid("the extrusion must lie within the working range"))?;
    extrude(&moved, profile, distance.abs())
}

/// `profile` (in `frame`'s plane coordinates) revolved about `axis` (a line in the same
/// plane coordinates) by `angle` radians, counter-clockwise about the axis direction
/// (right-hand rule); a negative angle turns the other way. `0 < |angle| <= 2π`; an
/// angle within [`ANGULAR_TOLERANCE`] of a full turn is a full revolve, which has no caps.
///
/// The profile must lie on one side of the axis. It may touch the axis at vertices and
/// along whole edges (such vertices become apexes or poles, such edges collapse), but no
/// edge may cross it or touch it elsewhere. A full revolve needs the profile to touch the
/// axis along at most one run of edges and nowhere else, and to have no holes (both would
/// enclose voids); otherwise it is refused.
pub fn revolve(frame: &Frame, profile: &Profile, axis: &Line2, angle: f64) -> KernelResult<Body> {
    if !(angle.is_finite()
        && angle.abs() > ANGULAR_TOLERANCE
        && angle.abs() <= TAU + ANGULAR_TOLERANCE)
    {
        return Err(invalid(
            "`angle` must be a finite angle with 0 < |angle| <= 2π radians",
        ));
    }
    let full = angle.abs() >= TAU - ANGULAR_TOLERANCE;
    let turn = if full { TAU } else { angle.abs() };
    // A negative angle is a positive one about the reversed axis.
    let axis = if angle < 0.0 { axis.reversed() } else { *axis };
    let checked = profile.check()?;
    let side = profile_side(&checked, &axis)?;
    // The sweep must move the profile toward +normal at the start (`a × e = n`): if it
    // moves it the other way, work in the flipped frame with the mirrored profile.
    let a3 = frame.vector_from_plane(axis.direction());
    let e3 = frame.vector_from_plane(axis.direction().perp() * side);
    let (frame, checked, axis) = if a3.cross(e3).dot(frame.z()) > 0.0 {
        (*frame, checked, axis)
    } else {
        let m = |p: Point2| Point2::new(p.x, -p.y);
        let d = axis.direction();
        let mirrored_axis = Line2::new(m(axis.origin()), Vec2::new(d.x, -d.y))?;
        (frame.flipped(), profile.mirrored().check()?, mirrored_axis)
    };
    let side = profile_side(&checked, &axis)?;
    let rev = Revolution::new(&frame, &axis, side)?;
    let checked = rev.snap_to_axis(checked)?;
    rev.check_profile(&checked, full)?;
    let motion = Motion::Rotate { rev, angle: turn };
    if full {
        build_full_revolve(&frame, &checked, &rev)
    } else {
        build_swept(&frame, &checked, &motion, &Names::revolve())
    }
}

/// Which side of the axis the profile lies on: `+1` left of its direction, `-1` right.
fn profile_side(p: &CheckedProfile, axis: &Line2) -> KernelResult<f64> {
    let normal = axis.direction().perp();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for l in p.loops() {
        for e in &l.edges {
            let (a, b) = e.edge.extent_along(axis.origin(), normal);
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    let tol = LINEAR_TOLERANCE * 0.5;
    if lo >= -tol && hi > tol {
        Ok(1.0)
    } else if hi <= tol && lo < -tol {
        Ok(-1.0)
    } else {
        Err(invalid(
            "the profile must lie on one side of the revolve axis (it may touch the axis but not cross it)",
        ))
    }
}

/// The axis of a revolve in the canonical frame: `n = a × e` is the frame's normal.
#[derive(Debug, Clone, Copy)]
struct Revolution {
    /// Axis origin and direction in plane coordinates, and the radial direction.
    q2: Point2,
    d2: Vec2,
    e2: Vec2,
    q3: Point3,
    a: Vec3,
    e: Vec3,
    n: Vec3,
}

impl Revolution {
    fn new(frame: &Frame, axis: &Line2, side: f64) -> KernelResult<Revolution> {
        let d2 = axis.direction();
        let e2 = d2.perp() * side;
        let a = frame.vector_from_plane(d2);
        let e = frame.vector_from_plane(e2);
        let n = a.cross(e);
        if n.dot(frame.z()) <= 0.5 {
            return Err(internal("the revolve frame is not canonical"));
        }
        Ok(Revolution {
            q2: axis.origin(),
            d2,
            e2,
            q3: frame.from_plane(axis.origin()),
            a,
            e,
            n,
        })
    }

    /// `(ρ, z)`: distance from the axis (signed, positive on the profile's side) and
    /// position along it.
    fn meridian(&self, p: Point2) -> (f64, f64) {
        let v = p - self.q2;
        (v.dot(self.e2), v.dot(self.d2))
    }

    /// The point with meridian coordinates `(ρ, z)` turned by `theta`.
    fn point(&self, rho: f64, z: f64, theta: f64) -> Point3 {
        let (s, c) = theta.sin_cos();
        self.q3 + self.a * z + (self.e * c + self.n * s) * rho
    }

    fn rotate_vector(&self, w: Vec3, theta: f64) -> Vec3 {
        let (s, c) = theta.sin_cos();
        let (wa, we, wn) = (w.dot(self.a), w.dot(self.e), w.dot(self.n));
        self.a * wa + self.e * (we * c - wn * s) + self.n * (we * s + wn * c)
    }

    /// The sketch frame turned by `theta` about the axis.
    fn rotated_frame(&self, frame: &Frame, theta: f64) -> KernelResult<Frame> {
        let (rho, z) = {
            let o = frame.to_plane(frame.origin());
            self.meridian(o)
        };
        let origin = self.point(rho, z, theta);
        Frame::from_normal_and_x(
            origin,
            self.rotate_vector(frame.z(), theta),
            self.rotate_vector(frame.x(), theta),
        )
        .ok_or_else(|| invalid("the revolved profile must lie within the working range"))
    }

    /// The frame of the circle a point at `(ρ, z)` sweeps: centre on the axis, normal
    /// along it, x toward the point.
    fn circle_frame(&self, z: f64) -> KernelResult<Frame> {
        Frame::from_normal_and_x(self.q3 + self.a * z, self.a, self.e)
            .ok_or_else(|| invalid("the revolve must lie within the working range"))
    }

    fn on_axis(&self, p: Point2) -> bool {
        self.meridian(p).0.abs() <= LINEAR_TOLERANCE * 0.5
    }

    /// The profile with every vertex within half the tolerance of the axis moved onto it
    /// exactly (so that turning it leaves it in place).
    fn snap_to_axis(&self, mut p: CheckedProfile) -> KernelResult<CheckedProfile> {
        let snap = |q: Point2| self.q2 + self.d2 * (q - self.q2).dot(self.d2);
        for l in std::iter::once(&mut p.outer).chain(p.holes.iter_mut()) {
            let n = l.edges.len();
            for k in 0..n {
                let start = l.edges[k].edge.start();
                if self.on_axis(start) {
                    let s = snap(start);
                    if let ProfileEdge::Line { a, .. } = &mut l.edges[k].edge {
                        *a = s;
                    }
                    let prev = (k + n - 1) % n;
                    if let ProfileEdge::Line { b, .. } = &mut l.edges[prev].edge {
                        *b = s;
                    }
                }
            }
        }
        Ok(p)
    }

    /// Refuses profiles that touch the axis other than at vertices or along whole edges,
    /// and full revolves that would enclose a void or pinch at a vertex.
    fn check_profile(&self, p: &CheckedProfile, full: bool) -> KernelResult<()> {
        if full && !p.holes.is_empty() {
            return Err(KernelError::Unsupported(
                "a full revolve of a profile with holes would enclose a void".into(),
            ));
        }
        // A line touches the axis only at its ends or lies on it (its distance from the
        // axis is linear and never negative). An arc's nearest point to the axis is an
        // end or the circle's point toward the axis; the arc may reach the axis only at
        // an end, and not tangentially (which is when that point is the end itself).
        for l in p.loops() {
            for e in &l.edges {
                let ProfileEdge::Arc {
                    centre,
                    radius,
                    start_angle,
                    sweep,
                } = e.edge
                else {
                    continue;
                };
                let (lo, _) = e.edge.extent_along(self.q2, self.e2);
                if lo > LINEAR_TOLERANCE * 0.5 {
                    continue;
                }
                let toward = (-self.e2).angle();
                let delta = if sweep >= 0.0 {
                    (toward - start_angle).rem_euclid(TAU)
                } else {
                    (start_angle - toward).rem_euclid(TAU)
                };
                let f = delta / sweep.abs();
                let nearest_on_arc = f <= 1.0 || f >= TAU / sweep.abs() - 1e-12;
                let deep = self.meridian(centre).0 - radius <= LINEAR_TOLERANCE * 0.5;
                if nearest_on_arc && deep {
                    return Err(invalid(format!(
                        "edge {} of {} touches the revolve axis inside or tangentially; only vertices and whole edges may lie on it",
                        e.source,
                        loop_label(l)
                    )));
                }
            }
        }
        if full {
            let l = &p.outer;
            let n = l.len();
            let on_v: Vec<bool> = (0..n).map(|k| self.on_axis(l.vertex(k))).collect();
            let on_e: Vec<bool> = (0..n).map(|k| self.edge_on_axis(l, k)).collect();
            let runs = (0..n)
                .filter(|k| on_e[*k] && !on_e[(k + n - 1) % n])
                .count();
            let all_axis = on_e.iter().all(|x| *x);
            if runs > 1 || all_axis {
                return Err(KernelError::Unsupported(
                    "a full revolve of a profile touching the axis along separate edges would enclose a void".into(),
                ));
            }
            for k in 0..n {
                if on_v[k] && !on_e[k] && !on_e[(k + n - 1) % n] {
                    return Err(KernelError::Unsupported(format!(
                        "a full revolve of a profile touching the axis at vertex {} alone would pinch the solid there",
                        l.edges[k].vertex_source
                    )));
                }
            }
        }
        Ok(())
    }

    fn edge_on_axis(&self, l: &CheckedLoop, k: usize) -> bool {
        l.edges.get(k).is_some_and(|e| {
            matches!(e.edge, ProfileEdge::Line { .. })
                && self.on_axis(e.edge.start())
                && self.on_axis(e.edge.end())
        })
    }
}

fn loop_label(l: &CheckedLoop) -> String {
    match l.hole {
        None => "the outer loop".into(),
        Some(j) => format!("hole {j}"),
    }
}

/// How a partial sweep moves the profile.
#[derive(Debug, Clone, Copy)]
enum Motion {
    Translate(Vec3),
    Rotate { rev: Revolution, angle: f64 },
}

impl Motion {
    /// The sketch frame at the far end.
    fn far_frame(&self, frame: &Frame) -> KernelResult<Frame> {
        match self {
            Motion::Translate(d) => frame
                .with_origin(frame.origin() + *d)
                .ok_or_else(|| invalid("the extrusion must lie within the working range")),
            Motion::Rotate { rev, angle } => rev.rotated_frame(frame, *angle),
        }
    }

    /// Whether a profile point stays where it is (on the revolve axis).
    fn fixed(&self, p: Point2) -> bool {
        match self {
            Motion::Translate(_) => false,
            Motion::Rotate { rev, .. } => rev.on_axis(p),
        }
    }

    fn edge_fixed(&self, l: &CheckedLoop, k: usize) -> bool {
        match self {
            Motion::Translate(_) => false,
            Motion::Rotate { rev, .. } => rev.edge_on_axis(l, k),
        }
    }

    /// The far position of a profile point.
    fn moved(&self, frame: &Frame, p: Point2) -> Point3 {
        match self {
            Motion::Translate(d) => frame.from_plane(p) + *d,
            Motion::Rotate { rev, angle } => {
                let (rho, z) = rev.meridian(p);
                rev.point(rho, z, *angle)
            }
        }
    }

    /// The curve a profile point sweeps, from its near to its far position.
    fn strut(&self, frame: &Frame, p: Point2) -> KernelResult<(Curve, f64, f64)> {
        match self {
            Motion::Translate(d) => {
                let a = frame.from_plane(p);
                Ok((Curve::Segment(Segment3::new(a, a + *d)?), 0.0, 1.0))
            }
            Motion::Rotate { rev, angle } => {
                let (rho, z) = rev.meridian(p);
                let arc = crate::Arc3::new(rev.circle_frame(z)?, rho, 0.0, *angle)?;
                Ok((Curve::Arc(arc), 0.0, *angle))
            }
        }
    }

    /// The sweep direction at the profile (the canonical frame's normal for both).
    fn direction(&self, frame: &Frame) -> Vec3 {
        frame.z()
    }
}

/// The names one kind of sweep gives.
struct Names {
    near_cap: &'static str,
    far_cap: &'static str,
    near: &'static str,
    far: &'static str,
}

impl Names {
    fn extrude() -> Names {
        Names {
            near_cap: "bottom",
            far_cap: "top",
            near: "bottom",
            far: "top",
        }
    }
    fn revolve() -> Names {
        Names {
            near_cap: "start cap",
            far_cap: "end cap",
            near: "start",
            far: "end",
        }
    }
}

/// `role i`, prefixed with `hole j ` for a hole loop.
fn label(l: &CheckedLoop, role: &str, i: usize) -> String {
    match l.hole {
        None => format!("{role} {i}"),
        Some(j) => format!("hole {j} {role} {i}"),
    }
}

/// The topology made for one profile loop, by checked-loop position `k` (edge `k` runs
/// from vertex `k` to vertex `k + 1`).
#[derive(Debug, Clone, Default)]
struct LoopTopology {
    near_v: Vec<Option<VertexId>>,
    far_v: Vec<Option<VertexId>>,
    near_e: Vec<Option<EdgeId>>,
    far_e: Vec<Option<EdgeId>>,
    strut: Vec<Option<EdgeId>>,
    side: Vec<Option<FaceId>>,
}

impl LoopTopology {
    fn new(n: usize) -> LoopTopology {
        LoopTopology {
            near_v: vec![None; n],
            far_v: vec![None; n],
            near_e: vec![None; n],
            far_e: vec![None; n],
            strut: vec![None; n],
            side: vec![None; n],
        }
    }
}

fn need<T: Copy>(x: Option<T>, what: &str) -> KernelResult<T> {
    x.ok_or_else(|| internal(format!("the sweep lost track of {what}")))
}

/// What sweeping one loop made, by position in the swept loop.
struct Swept {
    far_v: Vec<VertexId>,
    far_e: Vec<Option<EdgeId>>,
    strut: Vec<Option<EdgeId>>,
    side: Vec<Option<FaceId>>,
    /// The face that holds the moved loop.
    far_face: FaceId,
}

/// Sweeps the outer loop of `face`, whose vertices in loop order are `verts` (the
/// loop's half-edge leaving `verts[k]` runs to `verts[k + 1]`): an MEV per vertex not
/// `fixed` to `targets[k]`, then an MEF per edge not `skip`ped.
fn sweep_loop(
    body: &mut Body,
    face: FaceId,
    verts: &[VertexId],
    fixed: &[bool],
    skip: &[bool],
    targets: &[Point3],
) -> KernelResult<Swept> {
    let lp = body.face_outer_loop(face)?;
    let n = verts.len();
    let mut far_v = Vec::with_capacity(n);
    let mut strut = Vec::with_capacity(n);
    for (k, &v) in verts.iter().enumerate() {
        if fixed.get(k).copied().unwrap_or(false) {
            far_v.push(v);
            strut.push(None);
        } else {
            let h = need(body.loop_half_edge_from(lp, v)?, "a loop vertex")?;
            let p = *targets
                .get(k)
                .ok_or_else(|| internal("missing sweep target"))?;
            let (e, t) = body.mev(h, p)?;
            far_v.push(t);
            strut.push(Some(e));
        }
    }
    let mut far_e = vec![None; n];
    let mut side = vec![None; n];
    let mut far_face = face;
    for k in 0..n {
        if skip.get(k).copied().unwrap_or(false) {
            continue;
        }
        let h1 = need(body.loop_half_edge_from(lp, far_v[k])?, "a moved vertex")?;
        let h2 = need(
            body.loop_half_edge_from(lp, far_v[(k + 1) % n])?,
            "a moved vertex",
        )?;
        let (e, f) = body.mef(h1, h2)?;
        far_e[k] = Some(e);
        if h1 == h2 {
            // One vertex: the old face keeps the strut and becomes the side; the new face,
            // bounded by the closed edge alone, holds the moved loop.
            side[k] = Some(face);
            far_face = f;
        } else {
            side[k] = Some(f);
        }
    }
    Ok(Swept {
        far_v,
        far_e,
        strut,
        side,
        far_face,
    })
}

/// The lamina of the outer loop: MVFS, MEVs, closing MEF. Returns the far-cap face
/// (MVFS, counter-clockwise loop), the near-cap face, vertices and edges by position.
fn lamina(
    body: &mut Body,
    points: &[Point3],
) -> KernelResult<(FaceId, FaceId, Vec<VertexId>, Vec<EdgeId>)> {
    let (p0, rest) = points
        .split_first()
        .ok_or_else(|| invalid("the profile needs vertices"))?;
    let m = body.mvfs(*p0)?;
    let mut verts = vec![m.vertex];
    let mut edges = Vec::new();
    let mut at = m.half_edge;
    for p in rest {
        let (e, v) = body.mev(at, *p)?;
        verts.push(v);
        edges.push(e);
        at = body.edge_half_edges(e)?[1];
    }
    let first = match edges.first() {
        Some(e) => body.edge_half_edges(*e)?[0],
        None => m.half_edge,
    };
    let (close, near) = body.mef(at, first)?;
    edges.push(close);
    Ok((m.face, near, verts, edges))
}

/// The half-edge of `e` whose loop belongs to `face`.
fn half_in(body: &Body, e: EdgeId, face: FaceId) -> KernelResult<Option<HalfEdgeId>> {
    for h in body.edge_half_edges(e)? {
        if body.loop_face(body.half_edge_loop(h)?)? == face {
            return Ok(Some(h));
        }
    }
    Ok(None)
}

/// Which way the curve of an edge runs.
enum Along {
    /// From this vertex (the edge's ends differ).
    From(VertexId),
    /// Along this half-edge.
    Half(HalfEdgeId),
}

/// Puts `e` on `curve` over `[t0, t1]` running as `along` says.
fn place(
    body: &mut Body,
    e: EdgeId,
    curve: Curve,
    t0: f64,
    t1: f64,
    along: Along,
) -> KernelResult<()> {
    let [h0, _] = body.edge_half_edges(e)?;
    let forward = match along {
        Along::From(v) => {
            let [a, b] = body.edge_vertices(e)?;
            if a == b {
                return Err(internal(format!(
                    "{e} is closed; its direction needs a half-edge"
                )));
            }
            a == v
        }
        Along::Half(h) => h == h0,
    };
    if forward {
        body.set_edge_curve(e, curve, t0, t1)
    } else {
        let (c, a, b) = curve.reversed_on(t0, t1)?;
        body.set_edge_curve(e, c, a, b)
    }
}

/// A profile edge as a curve on the sketch frame (or a moved copy of it), running in the
/// edge's direction.
fn profile_curve(frame: &Frame, e: &ProfileEdge) -> KernelResult<(Curve, f64, f64)> {
    match *e {
        ProfileEdge::Line { a, b } => Ok((
            Curve::Segment(Segment3::new(frame.from_plane(a), frame.from_plane(b))?),
            0.0,
            1.0,
        )),
        ProfileEdge::Arc {
            centre,
            radius,
            start_angle,
            sweep,
        } => {
            let arc = crate::Arc2::new(centre, radius, start_angle, sweep)?.on(frame)?;
            let (lo, hi) = arc.range();
            Ok((Curve::Arc(arc), lo, hi))
        }
    }
}

/// The surface swept by profile edge `e`, and whether its normal is the outward one.
/// `frame` is the canonical sketch frame (the sweep moves the profile along its normal).
fn side_surface(frame: &Frame, motion: &Motion, e: &ProfileEdge) -> KernelResult<(Surface, bool)> {
    let s = match (motion, *e) {
        (Motion::Translate(d), ProfileEdge::Line { a, b }) => {
            let t = frame.vector_from_plane(b - a);
            let n = t.cross(*d);
            let f = Frame::from_normal_and_x(frame.from_plane(a), n, t)
                .ok_or_else(|| invalid("a side face has no plane"))?;
            return Ok((Surface::Plane(f), true));
        }
        (Motion::Translate(_), ProfileEdge::Arc { centre, radius, .. }) => {
            let f = Frame::from_normal_and_x(frame.from_plane(centre), frame.z(), frame.x())
                .ok_or_else(|| invalid("a side face has no cylinder"))?;
            Surface::cylinder(f, radius)?
        }
        (Motion::Rotate { rev, .. }, ProfileEdge::Line { a, b }) => {
            let ((ra, za), (rb, zb)) = (rev.meridian(a), rev.meridian(b));
            let (dr, dz) = (rb - ra, zb - za);
            let len = dr.hypot(dz);
            // Ends within the tolerance of one radius (or height): parallel (or
            // perpendicular) to the axis, so each end is within half the tolerance of the
            // cylinder (or plane) at the mean.
            if dr.abs() <= LINEAR_TOLERANCE {
                Surface::cylinder(rev.circle_frame(0.0)?, 0.5 * (ra + rb))?
            } else if dz.abs() <= LINEAR_TOLERANCE {
                Surface::Plane(rev.circle_frame(0.5 * (za + zb))?)
            } else {
                // A cone through the line, opening along +z of its frame.
                let up = if dz > 0.0 { rev.a } else { -rev.a };
                let f = Frame::from_normal_and_x(rev.q3 + rev.a * za, up, rev.e)
                    .ok_or_else(|| invalid("a side face has no cone"))?;
                Surface::cone(f, ra.max(0.0), (dr / len).asin())?
            }
        }
        (Motion::Rotate { rev, .. }, ProfileEdge::Arc { centre, radius, .. }) => {
            let (rc, zc) = rev.meridian(centre);
            let f = rev.circle_frame(zc)?;
            if rc.abs() <= LINEAR_TOLERANCE {
                Surface::sphere(f, radius)?
            } else {
                Surface::torus(f, rc, radius)?
            }
        }
    };
    Ok((s, same_sense(frame, motion, e, &s)))
}

/// Whether the surface normal at the middle of profile edge `e` is the outward normal
/// `T × S` there.
fn same_sense(frame: &Frame, motion: &Motion, e: &ProfileEdge, s: &Surface) -> bool {
    let p = frame.from_plane(e.point_at_fraction(0.5));
    let t = frame.vector_from_plane(e.tangent_at_fraction(0.5));
    let outward = t.cross(motion.direction(frame));
    let q = s.parameters(p);
    s.normal_at(q.u, q.v).dot(outward) > 0.0
}

/// Builds an extrusion or a partial revolve.
fn build_swept(
    frame: &Frame,
    profile: &CheckedProfile,
    motion: &Motion,
    names: &Names,
) -> KernelResult<Body> {
    let far_frame = motion.far_frame(frame)?;
    let mut body = Body::new();

    // The outer loop: lamina, then sweep the far cap.
    let outer = &profile.outer;
    let n = outer.len();
    let near_pts: Vec<Point3> = (0..n).map(|k| frame.from_plane(outer.vertex(k))).collect();
    let far_pts: Vec<Point3> = (0..n)
        .map(|k| motion.moved(frame, outer.vertex(k)))
        .collect();
    if !far_pts.iter().chain(near_pts.iter()).all(|p| p.is_valid()) {
        return Err(invalid(format!(
            "the swept solid must lie within ±{MAX_COORDINATE} mm"
        )));
    }
    let (far_cap0, near_cap, verts, edges) = lamina(&mut body, &near_pts)?;
    let fixed: Vec<bool> = (0..n).map(|k| motion.fixed(outer.vertex(k))).collect();
    let skip: Vec<bool> = (0..n).map(|k| motion.edge_fixed(outer, k)).collect();
    let swept = sweep_loop(&mut body, far_cap0, &verts, &fixed, &skip, &far_pts)?;
    let far_cap = swept.far_face;
    let mut topo = vec![LoopTopology::new(n)];
    {
        let t = &mut topo[0];
        for k in 0..n {
            t.near_v[k] = Some(verts[k]);
            t.near_e[k] = Some(edges[k]);
            t.far_v[k] = Some(swept.far_v[k]);
            t.far_e[k] = swept.far_e[k];
            t.strut[k] = swept.strut[k];
            t.side[k] = swept.side[k];
        }
    }

    // Holes: drawn into the far cap, swept back to the near end, made rings there.
    for hole in &profile.holes {
        let m = hole.len();
        let mut t = LoopTopology::new(m);
        // Drawing order: counter-clockwise, i.e. hole vertices 0, m-1, m-2, ..., 1. In
        // the cap's loop the half-edge from drawn vertex i runs to drawn vertex i + 1,
        // along hole edge (m - 1 - i) backwards.
        let order: Vec<usize> = (0..m).map(|i| (m - i) % m).collect();
        let far: Vec<Point3> = order
            .iter()
            .map(|k| motion.moved(frame, hole.vertex(*k)))
            .collect();
        let near: Vec<Point3> = order
            .iter()
            .map(|k| frame.from_plane(hole.vertex(*k)))
            .collect();
        if !far.iter().chain(near.iter()).all(|p| p.is_valid()) {
            return Err(invalid(format!(
                "the swept solid must lie within ±{MAX_COORDINATE} mm"
            )));
        }
        let cap_loop = body.face_outer_loop(far_cap)?;
        let start = need(
            body.loop_half_edges(cap_loop)?.first().copied(),
            "the far cap's loop",
        )?;
        let Some((w0, wrest)) = far.split_first() else {
            return Err(invalid("a hole needs vertices"));
        };
        let (bridge, v0) = body.mev(start, *w0)?;
        let [bridge_out, bridge_back] = body.edge_half_edges(bridge)?;
        let mut drawn_v = vec![v0];
        let mut chain = Vec::new();
        let mut at = bridge_back;
        for p in wrest {
            let (e, v) = body.mev(at, *p)?;
            drawn_v.push(v);
            chain.push(e);
            at = body.edge_half_edges(e)?[1];
        }
        let chain_first = match chain.first() {
            Some(e) => body.edge_half_edges(*e)?[0],
            None => bridge_back,
        };
        let (close, cap) = body.mef(chain_first, at)?;
        body.kemr(bridge_out)?;
        // Cap-loop edge i: chain edge i for i < m - 1, the closing edge last.
        let mut cap_edges = chain.clone();
        cap_edges.push(close);
        let back = sweep_loop(&mut body, cap, &drawn_v, &[], &[], &near)?;
        body.kfmrh(near_cap, back.far_face)?;
        for i in 0..m {
            let vk = order[i];
            let ek = (m - 1 - i) % m;
            t.far_v[vk] = Some(drawn_v[i]);
            t.near_v[vk] = Some(back.far_v[i]);
            t.strut[vk] = back.strut[i];
            t.far_e[ek] = cap_edges.get(i).copied();
            t.near_e[ek] = back.far_e[i];
            t.side[ek] = back.side[i];
        }
        topo.push(t);
    }

    // Geometry and names.
    let near_normal = -motion.direction(frame);
    let near_plane = Frame::from_normal_and_x(frame.origin(), near_normal, frame.x())
        .ok_or_else(|| internal("the near cap has no plane"))?;
    body.set_face_surface(near_cap, Surface::Plane(near_plane), true)?;
    body.set_face_name(near_cap, names.near_cap)?;
    body.set_face_surface(far_cap, Surface::Plane(far_frame), true)?;
    body.set_face_name(far_cap, names.far_cap)?;

    for (l, t) in profile.loops().zip(topo.iter()) {
        let n = l.len();
        for k in 0..n {
            let e = &l.edges[k];
            let pe = e.edge;
            let vs = e.vertex_source;
            let near_v = need(t.near_v[k], "a near vertex")?;
            let near_e = need(t.near_e[k], "a near edge")?;
            let fixed = motion.fixed(l.vertex(k));
            // Vertex names.
            if fixed {
                body.set_vertex_name(near_v, &label(l, "axis", vs))?;
            } else {
                body.set_vertex_name(near_v, &label(l, names.near, vs))?;
                body.set_vertex_name(need(t.far_v[k], "a far vertex")?, &label(l, names.far, vs))?;
            }
            // The strut swept by vertex k.
            if let Some(s) = t.strut[k] {
                let (c, t0, t1) = motion.strut(frame, l.vertex(k))?;
                place(&mut body, s, c, t0, t1, Along::From(near_v))?;
                body.set_edge_name(s, &label(l, "sweep", vs))?;
            }
            // Near and far copies of edge k.
            let (c, t0, t1) = profile_curve(frame, &pe)?;
            let side = t.side[k];
            let near_along = if n == 1 {
                let s = need(side, "a side face")?;
                Along::Half(need(half_in(&body, near_e, s)?, "a side half-edge")?)
            } else {
                Along::From(near_v)
            };
            place(&mut body, near_e, c, t0, t1, near_along)?;
            match (t.far_e[k], side) {
                (Some(fe), Some(s)) => {
                    let (c, t0, t1) = profile_curve(&far_frame, &pe)?;
                    let along = if n == 1 {
                        let [h0, h1] = body.edge_half_edges(fe)?;
                        let in_side = half_in(&body, fe, s)?;
                        Along::Half(if in_side == Some(h0) { h1 } else { h0 })
                    } else {
                        Along::From(need(t.far_v[k], "a far vertex")?)
                    };
                    place(&mut body, fe, c, t0, t1, along)?;
                    body.set_edge_name(near_e, &label(l, names.near, e.source))?;
                    body.set_edge_name(fe, &label(l, names.far, e.source))?;
                    let (surface, sense) = side_surface(frame, motion, &pe)?;
                    body.set_face_surface(s, surface, sense)?;
                    body.set_face_name(s, &label(l, "side", e.source))?;
                }
                (None, None) => {
                    body.set_edge_name(near_e, &label(l, "axis", e.source))?;
                }
                _ => return Err(internal("a swept edge lost its side face")),
            }
        }
    }
    body.validate()?;
    Ok(body)
}

/// Builds a full revolve (no caps; see the module documentation).
fn build_full_revolve(
    frame: &Frame,
    profile: &CheckedProfile,
    rev: &Revolution,
) -> KernelResult<Body> {
    let l = &profile.outer;
    let n = l.len();
    let pts: Vec<Point3> = (0..n).map(|k| frame.from_plane(l.vertex(k))).collect();
    let mut body = Body::new();
    let (far, near, verts, edges) = lamina(&mut body, &pts)?;
    let on_v: Vec<bool> = (0..n).map(|k| rev.on_axis(l.vertex(k))).collect();
    let on_e: Vec<bool> = (0..n).map(|k| rev.edge_on_axis(l, k)).collect();
    let mut circles: Vec<Option<EdgeId>> = vec![None; n];
    let forward =
        |b: &Body, k: usize| -> KernelResult<HalfEdgeId> { Ok(b.edge_half_edges(edges[k])?[0]) };
    let backward =
        |b: &Body, k: usize| -> KernelResult<HalfEdgeId> { Ok(b.edge_half_edges(edges[k])?[1]) };
    match (0..n).find(|k| on_e[*k] && !on_e[(k + n - 1) % n]) {
        None => {
            // Around the axis: one face with the lamina's two loops, joined at vertex 0
            // by its circle, then one circle per further vertex.
            body.kfmrh(far, near)?;
            let c0 = body.mekr(forward(&body, 0)?, backward(&body, (n - 1) % n)?)?;
            circles[0] = Some(c0);
            for (k, slot) in circles.iter_mut().enumerate().skip(1) {
                let (c, _) = body.mef(forward(&body, k)?, backward(&body, k - 1)?)?;
                *slot = Some(c);
            }
        }
        Some(first) => {
            // Across the run of edges on the axis: KEF on its first edge, KEV on the rest.
            body.kef(edges[first])?;
            let mut k = (first + 1) % n;
            while on_e[k] && k != first {
                body.kev(edges[k], verts[k])?;
                k = (k + 1) % n;
            }
            // k is now the first edge after the run; its start vertex is a pole. Each
            // following off-axis vertex gets its circle.
            let mut j = (k + 1) % n;
            while !on_v[j] {
                let (c, _) = body.mef(forward(&body, j)?, backward(&body, (j + n - 1) % n)?)?;
                circles[j] = Some(c);
                j = (j + 1) % n;
            }
        }
    }
    // Geometry: seams are the profile edges, circles the vertices' paths.
    for k in 0..n {
        let e = &l.edges[k];
        if on_e[k] {
            continue;
        }
        let (c, t0, t1) = profile_curve(frame, &e.edge)?;
        let along = if n == 1 {
            Along::Half(forward(&body, k)?)
        } else {
            Along::From(verts[k])
        };
        place(&mut body, edges[k], c, t0, t1, along)?;
        body.set_edge_name(edges[k], &label(l, "seam", e.source))?;
        let face = body.edge_faces(edges[k])?[0];
        let (surface, sense) = side_surface(
            frame,
            &Motion::Rotate {
                rev: *rev,
                angle: TAU,
            },
            &e.edge,
        )?;
        body.set_face_surface(face, surface, sense)?;
        body.set_face_name(face, &label(l, "side", e.source))?;
    }
    for k in 0..n {
        let vs = l.edges[k].vertex_source;
        if on_v[k] {
            body.set_vertex_name(verts[k], &label(l, "axis", vs))?;
            continue;
        }
        body.set_vertex_name(verts[k], &label(l, "vertex", vs))?;
        let c = need(circles[k], "a vertex circle")?;
        let (rho, z) = rev.meridian(l.vertex(k));
        let circle = Curve::Circle(crate::Circle3::new(rev.circle_frame(z)?, rho)?);
        // The circle runs counter-clockwise about the axis along its half-edge in the side
        // face of the edge ending at vertex k. With one edge both halves are in that face:
        // the counter-clockwise one is followed by the seam run backwards.
        let prev = (k + n - 1) % n;
        let prev_face = body.edge_faces(edges[prev])?[0];
        let [h0, h1] = body.edge_half_edges(c)?;
        let ccw = if n == 1 {
            let seam_forward = forward(&body, 0)?;
            if body.half_edge_next(h0)? != seam_forward {
                h0
            } else {
                h1
            }
        } else {
            need(half_in(&body, c, prev_face)?, "a circle's half-edge")?
        };
        place(&mut body, c, circle, 0.0, TAU, Along::Half(ccw))?;
        body.set_edge_name(c, &label(l, "sweep", vs))?;
    }
    // A disc or annulus keeps no seam (on a plane it would be a slit): KEV removes a
    // disc's seam with its centre, KEMR an annulus's, leaving the inner circle a ring.
    for k in 0..n {
        if on_e[k] {
            continue;
        }
        let e = edges[k];
        let face = body.edge_faces(e)?[0];
        if !matches!(body.face_surface(face)?, Some((Surface::Plane(_), _))) {
            continue;
        }
        let next = (k + 1) % n;
        if on_v[k] {
            body.kev(e, verts[k])?;
        } else if on_v[next] {
            body.kev(e, verts[next])?;
        } else {
            // KEMR(h) makes the circle after h (at h's end) the ring.
            let [h0, h1] = body.edge_half_edges(e)?;
            let (rho_k, _) = rev.meridian(l.vertex(k));
            let (rho_next, _) = rev.meridian(l.vertex(next));
            let h0_ends_inner = body.half_edge_end(h0)? == verts[next] && rho_next < rho_k
                || body.half_edge_end(h0)? == verts[k] && rho_k < rho_next;
            body.kemr(if h0_ends_inner { h0 } else { h1 })?;
        }
    }
    body.validate()?;
    Ok(body)
}

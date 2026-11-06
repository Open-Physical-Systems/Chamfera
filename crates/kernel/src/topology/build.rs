//! Solids built from Euler operators: cuboid, prism (with holes) and a cylinder. These are the shapes K4's extrude will produce.
//!
//! A prism is built as in Mantyla (1988), section 9.4 ("sweeping"): MVFS and a chain of
//! MEVs draw the profile as a tree of struts, one MEF closes it into a bottom face and a
//! top face, then the top face is swept: one MEV per profile vertex raises a vertical
//! strut inside it and one MEF per profile edge cuts a side face off it. Each hole is cut
//! into the top face as a bridge MEV, a chain of MEVs and an MEF (making a cap face),
//! the bridge is removed with KEMR (leaving a ring in the top face), the cap is swept
//! down through the solid in the same way, and KFMRH turns it into a ring of the bottom
//! face (genus + 1).

use super::{Body, Curve, FaceId, HalfEdgeId, Surface, VertexId};
use crate::{Circle3, KernelError, KernelResult};
use chamfera_geom::{
    Frame, LINEAR_TOLERANCE, MAX_COORDINATE, Point2, Point3, Sign, Vec3, orient2d,
};
use std::f64::consts::TAU;

/// Most vertices one profile polygon may have (the simplicity check is quadratic).
pub const MAX_PROFILE_VERTICES: usize = 10_000;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

fn orient(a: Point2, b: Point2, c: Point2) -> KernelResult<Sign> {
    orient2d(a, b, c).ok_or_else(|| {
        invalid("profile coordinates must be zero or of magnitude at least 1e-60 for the exact predicates")
    })
}

/// Twice the signed area of a polygon (shoelace), positive counter-clockwise.
fn twice_area(p: &[Point2]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

/// `c` lies within the box of `a` and `b` (used once `a, b, c` are known collinear).
fn within(a: Point2, b: Point2, c: Point2) -> bool {
    c.x >= a.x.min(b.x) && c.x <= a.x.max(b.x) && c.y >= a.y.min(b.y) && c.y <= a.y.max(b.y)
}

/// Whether closed segments `ab` and `cd` share a point, decided exactly (Cormen et al.,
/// *Introduction to Algorithms*, section 33.1, with exact orientation signs).
fn segments_touch(a: Point2, b: Point2, c: Point2, d: Point2) -> KernelResult<bool> {
    let (o1, o2) = (orient(a, b, c)?, orient(a, b, d)?);
    let (o3, o4) = (orient(c, d, a)?, orient(c, d, b)?);
    let opposite = |x: Sign, y: Sign| {
        matches!(
            (x, y),
            (Sign::Positive, Sign::Negative) | (Sign::Negative, Sign::Positive)
        )
    };
    Ok((opposite(o1, o2) && opposite(o3, o4))
        || (o1 == Sign::Zero && within(a, b, c))
        || (o2 == Sign::Zero && within(a, b, d))
        || (o3 == Sign::Zero && within(c, d, a))
        || (o4 == Sign::Zero && within(c, d, b)))
}

/// Strictly inside a simple polygon (the point is known not to be on its boundary):
/// winding number with exact orientation signs.
fn inside(poly: &[Point2], p: Point2) -> KernelResult<bool> {
    let n = poly.len();
    let mut winding = 0i64;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if a.y <= p.y {
            if b.y > p.y && orient(a, b, p)? == Sign::Positive {
                winding += 1;
            }
        } else if b.y <= p.y && orient(a, b, p)? == Sign::Negative {
            winding -= 1;
        }
    }
    Ok(winding != 0)
}

/// Checks a profile polygon and returns it counter-clockwise: 3 to
/// [`MAX_PROFILE_VERTICES`] valid points, consecutive points more than
/// [`LINEAR_TOLERANCE`] apart, simple (no edge touches a non-adjacent edge, no spike),
/// enclosing area.
fn check_polygon(name: &str, pts: &[Point2]) -> KernelResult<Vec<Point2>> {
    let n = pts.len();
    if !(3..=MAX_PROFILE_VERTICES).contains(&n) {
        return Err(invalid(format!(
            "`{name}` must have between 3 and {MAX_PROFILE_VERTICES} points"
        )));
    }
    if !pts.iter().all(|p| p.is_valid()) {
        return Err(invalid(format!(
            "`{name}` points must be finite and within ±{MAX_COORDINATE} mm"
        )));
    }
    for i in 0..n {
        if pts[i].distance(pts[(i + 1) % n]) <= LINEAR_TOLERANCE {
            return Err(invalid(format!(
                "consecutive points of `{name}` must be more than {LINEAR_TOLERANCE} mm apart"
            )));
        }
    }
    for i in 0..n {
        let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        if orient(a, b, c)? == Sign::Zero && (a - b).dot(c - b) > 0.0 {
            return Err(invalid(format!(
                "`{name}` folds back on itself at point {}",
                (i + 1) % n
            )));
        }
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue; // adjacent through the closing edge
            }
            if segments_touch(a, b, pts[j], pts[(j + 1) % n])? {
                return Err(invalid(format!("`{name}` must not cross or touch itself")));
            }
        }
    }
    let area = twice_area(pts) * 0.5;
    if area.is_nan() || area.abs() <= LINEAR_TOLERANCE * LINEAR_TOLERANCE {
        return Err(invalid(format!("`{name}` must enclose an area")));
    }
    let mut out = pts.to_vec();
    if area < 0.0 {
        out.reverse();
    }
    Ok(out)
}

/// Checks that holes lie strictly inside the outer polygon, outside each other, and that
/// no two polygons touch.
fn check_holes(outer: &[Point2], holes: &[Vec<Point2>]) -> KernelResult<()> {
    let mut polys: Vec<&[Point2]> = vec![outer];
    polys.extend(holes.iter().map(Vec::as_slice));
    for (i, p) in polys.iter().enumerate() {
        for q in polys.iter().skip(i + 1) {
            for a in 0..p.len() {
                for b in 0..q.len() {
                    let (p0, p1) = (p[a], p[(a + 1) % p.len()]);
                    let (q0, q1) = (q[b], q[(b + 1) % q.len()]);
                    if segments_touch(p0, p1, q0, q1)? {
                        return Err(invalid("holes must not touch the outline or each other"));
                    }
                }
            }
        }
    }
    for (i, h) in holes.iter().enumerate() {
        let Some(first) = h.first() else {
            return Err(invalid("a hole needs points"));
        };
        if !inside(outer, *first)? {
            return Err(invalid(format!("hole {i} must lie inside the outline")));
        }
        for (j, other) in holes.iter().enumerate() {
            if i != j && inside(other, *first)? {
                return Err(invalid(format!("hole {i} must not lie inside hole {j}")));
            }
        }
    }
    Ok(())
}

impl Body {
    /// Sweeps the loop of `start` by `d`: one MEV per vertex (a strut along `d`, inside
    /// the loop's face), then one MEF per edge, each cutting a side face off. The loop ends
    /// up as the moved copy of itself, in the same face. Returns the side faces in loop
    /// order from `start` (side `i` is swept from the `i`-th half-edge).
    fn sweep_loop(&mut self, start: HalfEdgeId, d: Vec3) -> KernelResult<Vec<FaceId>> {
        let lp = self.half_edge_loop(start)?;
        let hs = self.walk(start)?;
        let mut tops: Vec<VertexId> = Vec::with_capacity(hs.len());
        for h in hs {
            let p = self.vertex_point(self.half_edge_origin(h)?)? + d;
            tops.push(self.mev(h, p)?.1);
        }
        let n = tops.len();
        let mut sides = Vec::with_capacity(n);
        for i in 0..n {
            let from = |b: &Body, v: VertexId| {
                b.loop_half_edge_from(lp, v)?
                    .ok_or_else(|| KernelError::InvalidResult("sweep lost a strut".into()))
            };
            let h1 = from(self, tops[i])?;
            let h2 = from(self, tops[(i + 1) % n])?;
            sides.push(self.mef(h1, h2)?.1);
        }
        Ok(sides)
    }

    /// Draws a polygon as a chain of MEVs from the origin of `at` (which must be at
    /// `pts[0]`'s vertex) through `pts[1..]`. Returns the first half-edge of the first
    /// chain edge (from the start vertex) and the half-edge leaving the last vertex.
    fn mev_chain(
        &mut self,
        at: HalfEdgeId,
        pts: &[Point3],
    ) -> KernelResult<(HalfEdgeId, HalfEdgeId)> {
        let mut at = at;
        let mut first = None;
        for p in pts {
            let (e, _) = self.mev(at, *p)?;
            let [a, b] = self.edge_half_edges(e)?;
            first.get_or_insert(a);
            at = b;
        }
        let first = first.ok_or_else(|| invalid("a chain needs at least one point"))?;
        Ok((first, at))
    }

    /// A rectangular box with corner `min` and edge lengths `size` (all positive) along
    /// X, Y, Z: V = 8, E = 12, F = 6.
    pub fn cuboid(min: Point3, size: Vec3) -> KernelResult<Body> {
        if ![size.x, size.y, size.z]
            .iter()
            .all(|s| s.is_finite() && *s > LINEAR_TOLERANCE)
        {
            return Err(invalid(format!(
                "`size` must be finite and longer than {LINEAR_TOLERANCE} mm along each axis"
            )));
        }
        let frame = Frame::XY
            .with_origin(min)
            .ok_or_else(|| invalid("`min` must be a valid point"))?;
        let rect = [
            Point2::new(0.0, 0.0),
            Point2::new(size.x, 0.0),
            Point2::new(size.x, size.y),
            Point2::new(0.0, size.y),
        ];
        Body::prism(&frame, &rect, size.z)
    }

    /// A prism: the simple polygon `profile` (in `frame`'s plane coordinates, either
    /// orientation) extruded by `height > 0` along the frame's z axis. For n profile
    /// points: V = 2n, E = 3n, F = n + 2.
    pub fn prism(frame: &Frame, profile: &[Point2], height: f64) -> KernelResult<Body> {
        Body::prism_with_holes(frame, profile, &[], height)
    }

    /// A prism with through holes: the region inside `outer` and outside every polygon of
    /// `holes` (all simple, holes strictly inside `outer` and apart from each other),
    /// extruded by `height > 0` along `frame`'s z axis. Faces are named `bottom`, `top`,
    /// `side i` (from outer edge i) and `hole j side i`. For an n-gon with holes of
    /// m_j points: V = 2(n + Σm), E = 3(n + Σm), F = n + 2 + Σm, rings 2h, genus h.
    pub fn prism_with_holes(
        frame: &Frame,
        outer: &[Point2],
        holes: &[Vec<Point2>],
        height: f64,
    ) -> KernelResult<Body> {
        if !(height.is_finite() && height > LINEAR_TOLERANCE && height <= 2.0 * MAX_COORDINATE) {
            return Err(invalid(format!(
                "`height` must be finite and longer than {LINEAR_TOLERANCE} mm (flip the frame to extrude the other way)"
            )));
        }
        let outer = check_polygon("outer", outer)?;
        let holes = holes
            .iter()
            .enumerate()
            .map(|(i, h)| check_polygon(&format!("hole {i}"), h))
            .collect::<KernelResult<Vec<_>>>()?;
        let total: usize = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
        if total > MAX_PROFILE_VERTICES {
            return Err(invalid(format!(
                "a profile may have at most {MAX_PROFILE_VERTICES} points in all"
            )));
        }
        check_holes(&outer, &holes)?;
        let d = frame.z() * height;
        let lift = |p: &Point2| frame.from_plane(*p);
        let base: Vec<Point3> = outer.iter().map(lift).collect();
        let in_range = |p: Point3| p.is_valid() && (p + d).is_valid();
        if !(base.iter().all(|p| in_range(*p)) && holes.iter().flatten().all(|p| in_range(lift(p))))
        {
            return Err(invalid(format!(
                "the prism must lie within ±{MAX_COORDINATE} mm"
            )));
        }

        let mut body = Body::new();
        // The profile as a strut tree, closed by MEF: the MVFS face keeps the
        // counter-clockwise loop (it becomes the top), the new face is the bottom.
        let Some((p0, rest)) = base.split_first() else {
            return Err(invalid("`outer` needs points"));
        };
        let m = body.mvfs(*p0)?;
        let (first, last) = body.mev_chain(m.half_edge, rest)?;
        let (_, bottom) = body.mef(last, first)?;
        let top = m.face;
        let top_loop = body.face_outer_loop(top)?;
        let sides = body.sweep_loop(first, d)?;

        let mut hole_sides = Vec::new();
        for hole in &holes {
            let pts: Vec<Point3> = hole.iter().map(|p| lift(p) + d).collect();
            let Some((w0, wrest)) = pts.split_first() else {
                return Err(invalid("a hole needs points"));
            };
            // Bridge from the top face's first vertex to the hole's first vertex.
            let start = body
                .loop_half_edges(top_loop)?
                .first()
                .copied()
                .ok_or_else(|| KernelError::InvalidResult("the top face lost its loop".into()))?;
            let (bridge, _) = body.mev(start, *w0)?;
            let bridge_out = body.edge_half_edges(bridge)?[0];
            let bridge_back = body.edge_half_edges(bridge)?[1];
            let (chain_first, chain_last) = body.mev_chain(bridge_back, wrest)?;
            // Closing the hole polygon cuts off a cap face (counter-clockwise).
            let (_, cap) = body.mef(chain_first, chain_last)?;
            // Removing the bridge leaves the clockwise hole polygon as a ring of the top.
            body.kemr(bridge_out)?;
            hole_sides.push(body.sweep_loop(chain_first, -d)?);
            body.kfmrh(bottom, cap)?;
        }

        for e in body.edges() {
            body.set_edge_line(e)?;
        }
        for f in body.faces() {
            body.set_face_plane_from_loop(f)?;
        }
        body.set_face_name(bottom, "bottom")?;
        body.set_face_name(top, "top")?;
        for (i, f) in sides.iter().enumerate() {
            body.set_face_name(*f, &format!("side {i}"))?;
        }
        for (j, walls) in hole_sides.iter().enumerate() {
            for (i, f) in walls.iter().enumerate() {
                body.set_face_name(*f, &format!("hole {j} side {i}"))?;
            }
        }
        body.validate()?;
        Ok(body)
    }

    /// The topology of a closed cylinder of `radius` and `height` standing on `frame`'s
    /// plane (axis along its z axis, seam at its x axis): two circular edges, one seam
    /// edge, two planar caps and a lateral face. V = 2, E = 3, F = 3:
    /// `2 - 3 + 3 - 0 = 2`. Built as MVFS, MEF (closed bottom circle), MEV (seam), MEF
    /// (closed top circle).
    ///
    /// The lateral face lies on a [`Surface::Cylinder`] (outward normal away from the
    /// axis), so the body passes [`Body::validate`]. K4's extrude of a circle builds the
    /// same solid with the general naming scheme.
    pub fn cylinder_topology(frame: &Frame, radius: f64, height: f64) -> KernelResult<Body> {
        if !(height.is_finite() && height > LINEAR_TOLERANCE && height <= 2.0 * MAX_COORDINATE) {
            return Err(invalid(format!(
                "`height` must be finite and longer than {LINEAR_TOLERANCE} mm"
            )));
        }
        let bottom_circle = Circle3::new(*frame, radius)?;
        let up = frame.z() * height;
        let top_center = frame.origin() + up;
        let top_frame = Frame::from_normal_and_x(top_center, -frame.z(), frame.x())
            .ok_or_else(|| invalid("the cylinder must lie within the working range"))?;
        let top_circle = Circle3::new(top_frame, radius)?;
        let p0 = bottom_circle.point_at(0.0);
        let p1 = p0 + up;

        let mut body = Body::new();
        let m = body.mvfs(p0)?;
        let lateral = m.face;
        // Closed bottom edge: the lateral face's half-edge runs counter-clockwise about
        // the axis; the bottom face is bounded by its mate (clockwise, normal -z).
        let (bottom_edge, bottom) = body.mef(m.half_edge, m.half_edge)?;
        let along = body.edge_half_edges(bottom_edge)?[0];
        let (seam, _) = body.mev(along, p1)?;
        let down = body.edge_half_edges(seam)?[1];
        // Closed top edge: the lateral half-edge runs clockwise about the axis.
        let (top_edge, top) = body.mef(down, down)?;

        body.set_edge_curve(bottom_edge, Curve::Circle(bottom_circle), 0.0, TAU)?;
        body.set_edge_curve(top_edge, Curve::Circle(top_circle), 0.0, TAU)?;
        body.set_edge_line(seam)?;
        body.set_face_plane_from_loop(bottom)?;
        body.set_face_plane_from_loop(top)?;
        body.set_face_surface(lateral, Surface::cylinder(*frame, radius)?, true)?;
        body.set_face_name(lateral, "lateral")?;
        body.set_face_name(bottom, "bottom")?;
        body.set_face_name(top, "top")?;
        body.validate()?;
        Ok(body)
    }
}

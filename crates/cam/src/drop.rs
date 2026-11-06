//! The drop-cutter: lower a tool of revolution along -Z at a point (x, y) until it first
//! touches the exact model, and return the height of its tip.
//!
//! Clean-room, from Choi and Jerard, "Sculptured Surface Machining" (1998), ch. 5
//! (cutter-location from the offset surface), Hwang, "Interference-free tool-path
//! generation in the NC machining of parametric compound surfaces" (CAD 24(12), 1992),
//! and the vertex / edge / facet contact split of Chuang, Yang et al. Here the facets are
//! the exact B-rep faces, not triangles.
//!
//! # The tool
//! A flat, ball or bull-nose end mill of radius `R` with corner radius `r` (0, `R`, or
//! between) is the set of points within `r` of a horizontal disk of radius `e = R - r`
//! held `r` above the tip (the Minkowski sum of a disk and a ball). The height of its
//! cutting surface above the tip at a distance `ρ` from its axis is
//! `h(ρ) = 0` for `ρ <= e` and `r - sqrt(r² - (ρ - e)²)` up to `ρ = R` (a cylinder, the
//! shank, above). A model point `p` within `R` of the axis therefore holds the tip at
//! `p.z - h(ρ)` or higher; the drop-cutter height is the largest such value over the
//! model, and points outside every tool reach do not matter.
//!
//! # Contacts
//! The maximum is found in three kinds of place (each exact):
//! - **vertex**: `v.z - h(ρ)` in closed form;
//! - **edge**: the largest `p.z - h(ρ)` along the edge's exact curve: in closed form for a
//!   line and a ball or flat tool (`p.z - h` is concave along a line, its maximum has a
//!   closed form for these two), by golden-section search along a line for a bull-nose,
//!   and by sampling then golden-section refinement along circles, ellipses and splines;
//! - **facet**: the disk of radius `e` is dropped onto the face's offset surface (the
//!   face moved out by `r`), which is again a plane, sphere, cylinder, cone or torus
//!   (Choi and Jerard, §5.2). For a ball (`e = 0`) that is one vertical line against the
//!   offset surface: a quadratic for the quadrics, the torus's quartic by bracketed
//!   bisection. For `e > 0` the highest point of the offset surface over the disk is a
//!   critical point (the top of a sphere, of a torus's tube) or lies on the disk's rim,
//!   found by sampling the rim and golden-section refinement. Each candidate's contact
//!   point (the offset point moved back by `r` along the normal) must lie on the face
//!   (`Body::classify_point_on_face`); when it does not, the face's edges hold the tool.
//!
//! Vertical faces (walls) and faces facing down never hold a tool dropped from above on
//! their own: their top edges do. A spline extrusion that is not vertical is refused.
//!
//! A bounding-volume hierarchy over the faces, edges and vertices (in XY) keeps each drop
//! to what lies under the tool, and items are tried highest first so most are skipped.

use crate::CamError;
use chamfera_geom::{Aabb, Point3, Vec3};
use chamfera_kernel::{Body, Curve, FaceId, PointOnFace, Surface};
use std::f64::consts::{PI, TAU};

/// Most faces, edges or vertices a model may have for 3D machining (hostile-input limit).
pub const MAX_MODEL_ITEMS: usize = 200_000;
/// A normal's Z component must be above this for a point to face up (holds a tool from
/// above). Below it a surface is treated as a vertical wall, whose edges hold the tool.
const UP: f64 = 1e-9;
/// Samples round the disk's rim, before refinement: a rim maximum narrower than
/// `2π / RIM_SAMPLES` of the rim could be missed only on a surface that curves that
/// tightly within one tool radius, which none of the analytic offset surfaces does.
const RIM_SAMPLES: usize = 48;
/// How far past its radius a point still counts as under the tool (mm): rounding of
/// coordinates up to the working range moves a computed rim point by about 1e-10, which
/// is large next to a pin-point probe's radius.
const REACH_SLACK: f64 = 1e-9;
/// Golden-section steps: each shrinks the bracket by 0.618, so 60 take it to 3e-13 of
/// its width.
const GOLDEN_STEPS: usize = 60;

/// A cutter's shape: radius `radius`, corner radius `corner` (0 flat, `radius` ball).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolShape {
    pub radius: f64,
    pub corner: f64,
}

impl ToolShape {
    /// A checked shape: finite, `radius > 0`, `0 <= corner <= radius`.
    pub fn new(radius: f64, corner: f64) -> Result<ToolShape, CamError> {
        if !(radius.is_finite() && radius > 0.0 && radius <= crate::ops3d::MAX_TOOL_RADIUS) {
            return Err(CamError::Invalid(
                "the tool radius must be a positive number".into(),
            ));
        }
        if !(corner.is_finite() && corner >= 0.0 && corner <= radius * (1.0 + 1e-12)) {
            return Err(CamError::Invalid(
                "the corner radius must be from 0 to the tool radius".into(),
            ));
        }
        Ok(ToolShape {
            radius,
            corner: corner.min(radius),
        })
    }

    /// The radius of the flat disk, `R - r`.
    pub fn flat(&self) -> f64 {
        (self.radius - self.corner).max(0.0)
    }

    /// The cutting surface's height above the tip at `rho` from the axis, or `None`
    /// beyond the tool's radius.
    pub fn height(&self, rho: f64) -> Option<f64> {
        if !(rho.is_finite() && rho >= 0.0) || rho > self.radius + REACH_SLACK {
            return None;
        }
        let e = self.flat();
        if rho <= e {
            return Some(0.0);
        }
        let d = (rho - e).min(self.corner);
        let r = self.corner;
        Some(r - (r * r - d * d).max(0.0).sqrt())
    }
}

/// A plain 2D bounding-volume hierarchy over boxes in XY (median split on the longer
/// side; Ericson, *Real-Time Collision Detection*, §6.2).
#[derive(Debug, Clone, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    lo: [f64; 2],
    hi: [f64; 2],
    /// Leaf: `start..start + count` in `order`; inner: children `start` and `count`.
    start: usize,
    count: usize,
    leaf: bool,
}

const LEAF: usize = 4;

impl Bvh {
    pub fn build(boxes: &[([f64; 2], [f64; 2])]) -> Bvh {
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        let mut nodes = Vec::new();
        if boxes.is_empty() {
            return Bvh { nodes, order };
        }
        // (node index, start, end), built depth first with an explicit stack.
        nodes.push(Node {
            lo: [0.0; 2],
            hi: [0.0; 2],
            start: 0,
            count: 0,
            leaf: true,
        });
        let mut stack = vec![(0usize, 0usize, boxes.len())];
        while let Some((ni, s, e)) = stack.pop() {
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for &i in &order[s..e] {
                if let Some(b) = boxes.get(i) {
                    for k in 0..2 {
                        lo[k] = lo[k].min(b.0[k]);
                        hi[k] = hi[k].max(b.1[k]);
                    }
                }
            }
            let leaf = e - s <= LEAF;
            if leaf {
                if let Some(n) = nodes.get_mut(ni) {
                    *n = Node {
                        lo,
                        hi,
                        start: s,
                        count: e - s,
                        leaf: true,
                    };
                }
                continue;
            }
            let axis = usize::from(hi[1] - lo[1] > hi[0] - lo[0]);
            let centre = |i: usize| boxes.get(i).map_or(0.0, |b| b.0[axis] + b.1[axis]);
            order[s..e].sort_by(|a, b| centre(*a).total_cmp(&centre(*b)));
            let m = s + (e - s) / 2;
            let l = nodes.len();
            for _ in 0..2 {
                nodes.push(Node {
                    lo: [0.0; 2],
                    hi: [0.0; 2],
                    start: 0,
                    count: 0,
                    leaf: true,
                });
            }
            if let Some(n) = nodes.get_mut(ni) {
                *n = Node {
                    lo,
                    hi,
                    start: l,
                    count: l + 1,
                    leaf: false,
                };
            }
            stack.push((l, s, m));
            stack.push((l + 1, m, e));
        }
        Bvh { nodes, order }
    }

    /// Every item whose box meets the box `lo..hi`, appended to `out`.
    pub fn query(&self, lo: [f64; 2], hi: [f64; 2], out: &mut Vec<usize>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(ni) = stack.pop() {
            let Some(n) = self.nodes.get(ni) else {
                continue;
            };
            if n.lo[0] > hi[0] || n.hi[0] < lo[0] || n.lo[1] > hi[1] || n.hi[1] < lo[1] {
                continue;
            }
            if n.leaf {
                out.extend(self.order.iter().skip(n.start).take(n.count));
            } else {
                stack.push(n.start);
                stack.push(n.count);
            }
        }
    }
}

/// How a face holds a tool, worked out once.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Never holds a tool on its own (a wall, a face facing down).
    Skip,
    /// `n` is the outward unit normal (facing up), `o` a point of it.
    Plane {
        n: Vec3,
        o: Point3,
    },
    Sphere {
        c: Point3,
        s: f64,
        convex: bool,
    },
    Cylinder {
        o: Point3,
        a: Vec3,
        rc: f64,
        convex: bool,
    },
    Cone {
        o: Point3,
        a: Vec3,
        rad: f64,
        tan: f64,
        cos: f64,
        convex: bool,
    },
    Torus {
        frame: chamfera_geom::Frame,
        big: f64,
        small: f64,
        convex: bool,
    },
}

#[derive(Debug, Clone)]
struct FaceItem {
    body: usize,
    face: FaceId,
    shape: Shape,
    top: f64,
}

#[derive(Debug, Clone)]
struct EdgeItem {
    curve: Curve,
    t0: f64,
    t1: f64,
    top: f64,
}

/// A point of an offset surface over (x, y): the offset point `q`, the contact point
/// `foot` on the face's surface, and the outward unit normal there.
#[derive(Debug, Clone, Copy)]
struct Hit {
    q: Point3,
    foot: Point3,
}

/// The model as the drop-cutter sees it: every face, edge and vertex of the bodies, with
/// hierarchies over their XY boxes.
#[derive(Debug, Clone)]
pub struct DropModel {
    bodies: Vec<Body>,
    faces: Vec<FaceItem>,
    edges: Vec<EdgeItem>,
    vertices: Vec<Point3>,
    face_bvh: Bvh,
    edge_bvh: Bvh,
    vertex_bvh: Bvh,
    bounds: Aabb,
}

fn kernel(e: impl std::fmt::Display) -> CamError {
    CamError::Model(e.to_string())
}

fn xy_box(b: &Aabb) -> ([f64; 2], [f64; 2]) {
    ([b.min.x, b.min.y], [b.max.x, b.max.y])
}

fn grow(b: Aabb, d: f64) -> Aabb {
    let v = Vec3::new(d, d, d);
    Aabb {
        min: b.min + v * -1.0,
        max: b.max + v,
    }
}

fn meet(a: Aabb, b: Aabb) -> Aabb {
    Aabb {
        min: Point3::new(
            a.min.x.max(b.min.x),
            a.min.y.max(b.min.y),
            a.min.z.max(b.min.z),
        ),
        max: Point3::new(
            a.max.x.min(b.max.x),
            a.max.y.min(b.max.y),
            a.max.z.min(b.max.z),
        ),
    }
}

/// The component of `w` across the unit axis `a`.
fn across(w: Vec3, a: Vec3) -> Vec3 {
    w - a * w.dot(a)
}

/// Real roots of `a t² + 2 b t + c = 0` (a linear equation when `a` is ~0).
fn quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() <= 1e-14 * (b.abs() + c.abs()).max(1.0) {
        if b.abs() <= 1e-300 {
            return Vec::new();
        }
        return vec![-c / (2.0 * b)];
    }
    let d = b * b - a * c;
    if d < 0.0 {
        return Vec::new();
    }
    let s = d.sqrt();
    // The stable form (no cancellation).
    let q = -(b + if b >= 0.0 { s } else { -s });
    if q.abs() <= 1e-300 {
        return vec![0.0];
    }
    vec![q / a, c / q]
}

/// The largest value of `f` on `[a, b]` near a bracketed maximum, by golden-section
/// search: `(argument, value)`.
pub(crate) fn golden(mut a: f64, mut b: f64, f: &dyn Fn(f64) -> f64) -> (f64, f64) {
    let g = (5f64.sqrt() - 1.0) / 2.0;
    let mut x1 = b - g * (b - a);
    let mut x2 = a + g * (b - a);
    let (mut f1, mut f2) = (f(x1), f(x2));
    for _ in 0..GOLDEN_STEPS {
        if f1 < f2 {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = a + g * (b - a);
            f2 = f(x2);
        } else {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = b - g * (b - a);
            f1 = f(x1);
        }
    }
    if f1 >= f2 { (x1, f1) } else { (x2, f2) }
}

/// How a face holds a tool and a box that holds every point of it: its edges' box, grown
/// by the surface's size (every point of a face on a sphere, cylinder, cone or torus is
/// within that of its boundary) and cut to the surface's own box where it has one. A
/// cone's face may bulge past its edges only toward its widest edge, which is an edge:
/// its box is its edges'. `None` for a face with no edges and no bounded surface.
fn face_shape_and_box(body: &Body, f: FaceId) -> Result<Option<(Shape, Aabb)>, CamError> {
    let Some((surface, same)) = body.face_surface(f).map_err(kernel)? else {
        return Err(CamError::Refused(
            "a face of the model has no surface".into(),
        ));
    };
    let sign = if same { 1.0 } else { -1.0 };
    let mut eb: Option<Aabb> = None;
    for e in body.face_edges(f).map_err(kernel)? {
        if let Some((c, t0, t1)) = body.edge_curve(e).map_err(kernel)? {
            let b = c.bounding_box_on(t0, t1).map_err(kernel)?;
            eb = Some(eb.map_or(b, |o| o.union(b)));
        }
    }
    let shape = shape_of(&surface, sign)?;
    let (size, own): (f64, Option<Aabb>) = match shape {
        Shape::Skip | Shape::Plane { .. } => (0.0, None),
        Shape::Sphere { c, s, .. } => (2.0 * s, Some(grow(Aabb { min: c, max: c }, s))),
        Shape::Cylinder { rc, .. } => (2.0 * rc, None),
        Shape::Cone { .. } => (0.0, None),
        Shape::Torus {
            frame, big, small, ..
        } => {
            let o = frame.origin();
            let reach = big.abs() + small;
            (2.0 * reach, Some(grow(Aabb { min: o, max: o }, reach)))
        }
    };
    Ok(match (eb, own) {
        (Some(e), Some(o)) => Some((shape, meet(grow(e, size), o))),
        (Some(e), None) => Some((shape, grow(e, size))),
        (None, Some(o)) => Some((shape, o)),
        (None, None) => None,
    })
}

/// A box holding every point of the face (see [`DropModel`]), or `None` when it has no
/// edges and no bounded surface.
pub fn face_box(body: &Body, f: FaceId) -> Result<Option<Aabb>, CamError> {
    Ok(face_shape_and_box(body, f)?.map(|x| x.1))
}

impl DropModel {
    /// Reads every face, edge and vertex of `bodies`. Refuses what the drop-cutter cannot
    /// cut exactly: a face without a surface, a sloping spline extrusion.
    pub fn new(bodies: &[Body]) -> Result<DropModel, CamError> {
        let mut faces = Vec::new();
        let mut edges = Vec::new();
        let mut vertices = Vec::new();
        let mut boxes_f = Vec::new();
        let mut boxes_e = Vec::new();
        let mut boxes_v = Vec::new();
        let mut bounds: Option<Aabb> = None;
        for (bi, body) in bodies.iter().enumerate() {
            if let Some(b) = body.bounding_box() {
                bounds = Some(bounds.map_or(b, |o| o.union(b)));
            }
            let n = body.faces().len() + body.edges().len() + body.vertices().len();
            if faces.len() + edges.len() + vertices.len() + n > MAX_MODEL_ITEMS {
                return Err(CamError::TooComplex(format!(
                    "the model has more than {MAX_MODEL_ITEMS} faces, edges and vertices"
                )));
            }
            for v in body.vertices() {
                let p = body.vertex_point(v).map_err(kernel)?;
                boxes_v.push(([p.x, p.y], [p.x, p.y]));
                vertices.push(p);
            }
            for e in body.edges() {
                let Some((curve, t0, t1)) = body.edge_curve(e).map_err(kernel)? else {
                    return Err(CamError::Refused(
                        "an edge of the model has no curve".into(),
                    ));
                };
                let b = curve.bounding_box_on(t0, t1).map_err(kernel)?;
                boxes_e.push(xy_box(&b));
                edges.push(EdgeItem {
                    curve,
                    t0,
                    t1,
                    top: b.max.z,
                });
            }
            for f in body.faces() {
                let Some((shape, b)) = face_shape_and_box(body, f)? else {
                    continue;
                };
                boxes_f.push(xy_box(&b));
                faces.push(FaceItem {
                    body: bi,
                    face: f,
                    shape,
                    top: b.max.z,
                });
            }
        }
        let bounds =
            bounds.ok_or_else(|| CamError::Refused("there is no model to machine".into()))?;
        Ok(DropModel {
            bodies: bodies.to_vec(),
            faces,
            edges,
            vertices,
            face_bvh: Bvh::build(&boxes_f),
            edge_bvh: Bvh::build(&boxes_e),
            vertex_bvh: Bvh::build(&boxes_v),
            bounds,
        })
    }

    /// The model's bounding box.
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// The tip height of `tool` dropped at `(x, y)`, or `None` when it misses the model.
    pub fn drop(&self, tool: &ToolShape, x: f64, y: f64) -> Option<f64> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let rr = tool.radius;
        let lo = [x - rr, y - rr];
        let hi = [x + rr, y + rr];
        let mut best = f64::NEG_INFINITY;
        let mut found = Vec::new();
        self.vertex_bvh.query(lo, hi, &mut found);
        for i in found.drain(..) {
            if let Some(v) = self.vertices.get(i)
                && let Some(h) = tool.height((v.x - x).hypot(v.y - y))
            {
                best = best.max(v.z - h);
            }
        }
        self.edge_bvh.query(lo, hi, &mut found);
        found.sort_by(|a, b| {
            let t = |i: usize| self.edges.get(i).map_or(f64::NEG_INFINITY, |e| e.top);
            t(*b).total_cmp(&t(*a))
        });
        for i in found.drain(..) {
            let Some(e) = self.edges.get(i) else { continue };
            if e.top <= best {
                break;
            }
            if let Some(z) = edge_contact(tool, e, x, y) {
                best = best.max(z);
            }
        }
        self.face_bvh.query(lo, hi, &mut found);
        found.sort_by(|a, b| {
            let t = |i: usize| self.faces.get(i).map_or(f64::NEG_INFINITY, |f| f.top);
            t(*b).total_cmp(&t(*a))
        });
        for i in found {
            let Some(f) = self.faces.get(i) else { continue };
            if f.top <= best {
                break;
            }
            if let Some(z) = self.facet_contact(tool, f, x, y, best) {
                best = best.max(z);
            }
        }
        best.is_finite().then_some(best)
    }

    /// True when `p` is on the face (inside or on its boundary). A point the kernel cannot
    /// classify (on a curved face's axis) counts as on it: that can only hold the tool
    /// higher, never cut into the part.
    fn on_face(&self, f: &FaceItem, p: Point3) -> bool {
        let Some(body) = self.bodies.get(f.body) else {
            return false;
        };
        !matches!(
            body.classify_point_on_face(f.face, p),
            Ok(PointOnFace::Outside)
        )
    }

    fn facet_contact(
        &self,
        tool: &ToolShape,
        f: &FaceItem,
        x: f64,
        y: f64,
        best: f64,
    ) -> Option<f64> {
        let r = tool.corner;
        let e = tool.flat();
        let mut out = f64::NEG_INFINITY;
        let consider = |hits: &mut Vec<Hit>, out: &mut f64| {
            hits.sort_by(|a, b| b.q.z.total_cmp(&a.q.z));
            for h in hits.drain(..) {
                let z = h.q.z - r;
                if z <= best.max(*out) {
                    break;
                }
                let rho = (h.foot.x - x).hypot(h.foot.y - y);
                if rho <= tool.radius + REACH_SLACK && self.on_face(f, h.foot) {
                    *out = z;
                    break;
                }
            }
        };
        let mut hits = Vec::new();
        if let Shape::Plane { n, o } = f.shape {
            // The highest point of the offset plane over the disk is on its uphill side.
            let nh = n.x.hypot(n.y);
            let (qx, qy) = if nh > 1e-15 {
                (x - e * n.x / nh, y - e * n.y / nh)
            } else {
                (x, y)
            };
            let o2 = o + n * r;
            let qz = o2.z - ((qx - o2.x) * n.x + (qy - o2.y) * n.y) / n.z;
            let q = Point3::new(qx, qy, qz);
            hits.push(Hit {
                q,
                foot: q + n * -r,
            });
            consider(&mut hits, &mut out);
            return out.is_finite().then_some(out);
        }
        if matches!(f.shape, Shape::Skip) {
            return None;
        }
        // A disk this small (a pin-point probe) is its centre: the rim adds nothing a
        // micrometre could show, and on a seam between faces its points fall off both.
        if e <= 1e-6 {
            vertical_hits(&f.shape, r, x, y, &mut hits);
            consider(&mut hits, &mut out);
            return out.is_finite().then_some(out);
        }
        // Critical points: the tops of a sphere and of a torus's tube.
        critical_hits(&f.shape, r, &mut hits);
        hits.retain(|h| (h.q.x - x).hypot(h.q.y - y) <= e + REACH_SLACK);
        consider(&mut hits, &mut out);
        // The rim: each branch of upward hits separately (a torus has two).
        let branches = if matches!(f.shape, Shape::Torus { .. }) {
            2
        } else {
            1
        };
        for k in 0..branches {
            let height = |phi: f64| -> f64 {
                let mut hs = Vec::new();
                vertical_hits(&f.shape, r, x + e * phi.cos(), y + e * phi.sin(), &mut hs);
                hs.sort_by(|a, b| b.q.z.total_cmp(&a.q.z));
                hs.get(k).map_or(f64::NEG_INFINITY, |h| h.q.z)
            };
            let step = TAU / RIM_SAMPLES as f64;
            let samples: Vec<f64> = (0..RIM_SAMPLES).map(|i| height(i as f64 * step)).collect();
            for i in 0..RIM_SAMPLES {
                let (Some(&a), Some(&m), Some(&b)) = (
                    samples.get((i + RIM_SAMPLES - 1) % RIM_SAMPLES),
                    samples.get(i),
                    samples.get((i + 1) % RIM_SAMPLES),
                ) else {
                    continue;
                };
                // Refine each sampled local maximum (a sample next to one where the rim
                // leaves the surface counts: its neighbour is -inf).
                if !(m.is_finite() && m >= a && m >= b) {
                    continue;
                }
                let phi0 = i as f64 * step;
                let (phi, _) = golden(phi0 - step, phi0 + step, &height);
                let mut hs = Vec::new();
                vertical_hits(&f.shape, r, x + e * phi.cos(), y + e * phi.sin(), &mut hs);
                hs.sort_by(|a, b| b.q.z.total_cmp(&a.q.z));
                let mut chosen: Vec<Hit> = hs.into_iter().skip(k).take(1).collect();
                consider(&mut chosen, &mut out);
            }
        }
        out.is_finite().then_some(out)
    }
}

fn shape_of(surface: &Surface, sign: f64) -> Result<Shape, CamError> {
    let k = Vec3::new(0.0, 0.0, 1.0);
    Ok(match surface {
        Surface::Plane(fr) => {
            let n = fr.z() * sign;
            if n.z <= UP {
                Shape::Skip
            } else {
                Shape::Plane { n, o: fr.origin() }
            }
        }
        Surface::Extrusion(ex) => {
            let d = ex.frame().z();
            if d.x.abs() <= 1e-9 && d.y.abs() <= 1e-9 {
                Shape::Skip
            } else {
                return Err(CamError::Refused(
                    "a sloping spline face cannot be machined in 3D yet: only vertical \
                     extruded splines"
                        .into(),
                ));
            }
        }
        _ => {
            // Convex when the outward normal points away from the axis (the centre, the
            // tube's centre circle) at a sample point.
            let (u, v) = match surface.v_range() {
                (lo, hi) if lo.is_finite() && hi.is_finite() => (0.3, (lo + hi) / 2.0),
                (lo, _) if lo.is_finite() => (0.3, lo + 1.0),
                (_, hi) if hi.is_finite() => (0.3, hi - 1.0),
                _ => (0.3, 0.0),
            };
            let p = surface.point_at(u, v);
            let n = surface.normal_at(u, v) * sign;
            let fr = *surface.frame();
            let a = fr.z();
            let o = fr.origin();
            match surface {
                Surface::Sphere(s) => Shape::Sphere {
                    c: o,
                    s: s.radius(),
                    convex: (p - o).dot(n) > 0.0,
                },
                Surface::Cylinder(c) => {
                    if a.z.abs() >= 1.0 - 1e-12 {
                        Shape::Skip
                    } else {
                        Shape::Cylinder {
                            o,
                            a,
                            rc: c.radius(),
                            convex: across(p - o, a).dot(n) > 0.0,
                        }
                    }
                }
                Surface::Cone(c) => {
                    let (s, co) = c.half_angle().sin_cos();
                    // A cone about a vertical axis with a vertical generator is a wall;
                    // others hold the tool.
                    let _ = k;
                    Shape::Cone {
                        o,
                        a,
                        rad: c.radius(),
                        tan: s / co,
                        cos: co,
                        convex: across(p - o, a).dot(n) > 0.0,
                    }
                }
                Surface::Torus(t) => {
                    let l = fr.to_local(p);
                    let rho = l.x.hypot(l.y);
                    let centre = if rho > 1e-12 {
                        fr.to_world(Point3::new(
                            t.major_radius() * l.x / rho,
                            t.major_radius() * l.y / rho,
                            0.0,
                        ))
                    } else {
                        o
                    };
                    Shape::Torus {
                        frame: fr,
                        big: t.major_radius(),
                        small: t.minor_radius(),
                        convex: (p - centre).dot(n) > 0.0,
                    }
                }
                _ => Shape::Skip,
            }
        }
    })
}

/// The tops of the offset surface (points where its normal is +Z that are local
/// maxima of height): a convex sphere's top, a convex torus's tube tops.
fn critical_hits(shape: &Shape, r: f64, out: &mut Vec<Hit>) {
    let k = Vec3::new(0.0, 0.0, 1.0);
    match *shape {
        Shape::Sphere { c, s, convex: true } => {
            let q = c + k * (s + r);
            out.push(Hit { q, foot: c + k * s });
        }
        Shape::Torus {
            frame,
            big,
            small,
            convex: true,
        } => {
            let (xz, yz) = (frame.x().z, frame.y().z);
            if xz.hypot(yz) <= 1e-12 {
                return;
            }
            let t = yz.atan2(xz);
            for th in [t, t + PI] {
                let c = frame.origin() + (frame.x() * th.cos() + frame.y() * th.sin()) * big;
                out.push(Hit {
                    q: c + k * (small + r),
                    foot: c + k * small,
                });
            }
        }
        _ => {}
    }
}

/// Where the vertical line through `(x, y)` meets the face's surface offset outward by
/// `r`, at points whose outward normal faces up; each with its contact point.
fn vertical_hits(shape: &Shape, r: f64, x: f64, y: f64, out: &mut Vec<Hit>) {
    let k = Vec3::new(0.0, 0.0, 1.0);
    let at = |z: f64| Point3::new(x, y, z);
    match *shape {
        Shape::Skip => {}
        Shape::Plane { n, o } => {
            let o2 = o + n * r;
            let qz = o2.z - ((x - o2.x) * n.x + (y - o2.y) * n.y) / n.z;
            let q = at(qz);
            out.push(Hit {
                q,
                foot: q + n * -r,
            });
        }
        Shape::Sphere { c, s, convex } => {
            let s2 = if convex { s + r } else { s - r };
            if s2 <= 1e-12 {
                return;
            }
            let d2 = (x - c.x).powi(2) + (y - c.y).powi(2);
            if d2 > s2 * s2 {
                return;
            }
            let h = (s2 * s2 - d2).sqrt();
            // Convex: the upper half faces up; concave (a bowl): the lower half.
            let qz = if convex { c.z + h } else { c.z - h };
            if h <= UP * s2 {
                return;
            }
            let q = at(qz);
            out.push(Hit {
                q,
                foot: c + (q - c) * (s / s2),
            });
        }
        Shape::Cylinder { o, a, rc, convex } => {
            let r2 = if convex { rc + r } else { rc - r };
            if r2 <= 1e-12 {
                return;
            }
            let w0 = Vec3::new(x - o.x, y - o.y, -o.z);
            let p0 = across(w0, a);
            let d = across(k, a);
            for z in quadratic(d.dot(d), p0.dot(d), p0.dot(p0) - r2 * r2) {
                let perp = p0 + d * z;
                let len = perp.length();
                let Some(u) = perp.normalized() else { continue };
                let n = if convex { u } else { -u };
                if n.z <= UP || len <= 0.0 {
                    continue;
                }
                let q = at(z);
                out.push(Hit {
                    q,
                    foot: q + n * -r,
                });
            }
        }
        Shape::Cone {
            o,
            a,
            rad,
            tan,
            cos,
            convex,
        } => {
            if cos <= 1e-12 {
                return;
            }
            let rad2 = if convex { rad + r / cos } else { rad - r / cos };
            let w0 = Vec3::new(x - o.x, y - o.y, -o.z);
            let p0 = across(w0, a);
            let d = across(k, a);
            let s0 = w0.dot(a);
            let c0 = rad2 + tan * s0;
            let c1 = tan * a.z;
            for z in quadratic(
                d.dot(d) - c1 * c1,
                p0.dot(d) - c0 * c1,
                p0.dot(p0) - c0 * c0,
            ) {
                let s = s0 + a.z * z;
                if rad2 + tan * s < 0.0 {
                    continue;
                }
                let perp = p0 + d * z;
                let Some(u) = perp.normalized() else { continue };
                let sin = tan * cos;
                let n0 = u * cos - a * sin;
                let n = if convex { n0 } else { -n0 };
                if n.z <= UP {
                    continue;
                }
                let q = at(z);
                let foot = q + n * -r;
                // The contact must be on the nappe (not past the apex).
                if rad + tan * (foot - o).dot(a) < -1e-9 {
                    continue;
                }
                out.push(Hit { q, foot });
            }
        }
        Shape::Torus {
            frame,
            big,
            small,
            convex,
        } => {
            let a2 = if convex { small + r } else { small - r };
            if a2 <= 1e-12 {
                return;
            }
            let reach = big.abs() + a2;
            let l0 = frame.to_local(at(0.0));
            let dl = Vec3::new(frame.x().z, frame.y().z, frame.z().z);
            let l0v = Vec3::new(l0.x, l0.y, l0.z);
            // The part of the line inside the bounding sphere |l| <= reach.
            let roots = quadratic(1.0, l0v.dot(dl), l0v.dot(l0v) - reach * reach);
            if roots.len() < 2 {
                return;
            }
            let (t0, t1) = (roots[0].min(roots[1]), roots[0].max(roots[1]));
            let f = |t: f64| {
                let p = l0v + dl * t;
                (p.x.hypot(p.y) - big).powi(2) + p.z * p.z - a2 * a2
            };
            const N: usize = 96;
            let mut prev = (t0, f(t0));
            for i in 1..=N {
                let t = t0 + (t1 - t0) * i as f64 / N as f64;
                let ft = f(t);
                if prev.1 == 0.0 || prev.1.signum() != ft.signum() {
                    let (mut lo, mut hi) = (prev.0, t);
                    let flo = prev.1;
                    for _ in 0..80 {
                        let m = (lo + hi) / 2.0;
                        if f(m).signum() == flo.signum() && flo != 0.0 {
                            lo = m;
                        } else {
                            hi = m;
                        }
                    }
                    let tz = (lo + hi) / 2.0;
                    let pl = l0v + dl * tz;
                    let rho = pl.x.hypot(pl.y);
                    if rho > 1e-12 {
                        let c = Vec3::new(big * pl.x / rho, big * pl.y / rho, 0.0);
                        if let Some(u) = (pl - c).normalized() {
                            let ul = if convex { u } else { -u };
                            let n = frame.x() * ul.x + frame.y() * ul.y + frame.z() * ul.z;
                            if n.z > UP {
                                // World z of the hit: the line is vertical through (x, y).
                                let q = at(tz);
                                out.push(Hit {
                                    q,
                                    foot: q + n * -r,
                                });
                            }
                        }
                    }
                }
                prev = (t, ft);
            }
        }
    }
}

/// The highest the tool's tip can be held by one edge, or `None` if it is out of reach.
fn edge_contact(tool: &ToolShape, e: &EdgeItem, x: f64, y: f64) -> Option<f64> {
    let g = |p: Point3| tool.height((p.x - x).hypot(p.y - y)).map(|h| p.z - h);
    if let Curve::Line(_) | Curve::Segment(_) = e.curve {
        let a = e.curve.point_at(e.t0);
        let b = e.curve.point_at(e.t1);
        return line_contact(tool, a, b, x, y);
    }
    // A general curve: samples (by its length over a quarter tool radius), the best
    // bracketed by golden-section search, and the points where it enters the reach.
    let mut len = 0.0;
    let mut prev = e.curve.point_at(e.t0);
    for i in 1..=16 {
        let p = e.curve.point_at(e.t0 + (e.t1 - e.t0) * f64::from(i) / 16.0);
        len += p.distance(prev);
        prev = p;
    }
    let n = ((len / (tool.radius / 4.0)).ceil() as usize).clamp(16, 512);
    let t_at = |i: usize| e.t0 + (e.t1 - e.t0) * i as f64 / n as f64;
    let val = |t: f64| g(e.curve.point_at(t)).unwrap_or(f64::NEG_INFINITY);
    let vals: Vec<f64> = (0..=n).map(|i| val(t_at(i))).collect();
    let mut best = f64::NEG_INFINITY;
    for i in 0..=n {
        let Some(&m) = vals.get(i) else { continue };
        let l = if i > 0 {
            vals.get(i - 1).copied()
        } else {
            None
        };
        let r = vals.get(i + 1).copied();
        if m.is_finite() {
            best = best.max(m);
            if l.is_none_or(|l| m >= l) && r.is_none_or(|r| m >= r) {
                let (_, v) = golden(t_at(i.saturating_sub(1)), t_at((i + 1).min(n)), &val);
                best = best.max(v);
            }
        }
        // Entering or leaving the reach between samples: the boundary point.
        if let Some(r) = r
            && m.is_finite() != r.is_finite()
        {
            let (mut lo, mut hi) = (t_at(i), t_at(i + 1));
            let inside_lo = m.is_finite();
            for _ in 0..60 {
                let mid = (lo + hi) / 2.0;
                if val(mid).is_finite() == inside_lo {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let t = if inside_lo { lo } else { hi };
            best = best.max(val(t));
        }
    }
    best.is_finite().then_some(best)
}

/// A straight edge `a`–`b`: the part within the tool's reach, then the maximum of the
/// concave `z - h(ρ)` on it (closed form for a flat or ball tool).
fn line_contact(tool: &ToolShape, a: Point3, b: Point3, x: f64, y: f64) -> Option<f64> {
    let rr = tool.radius;
    let (dx, dy, dz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (ax, ay) = (a.x - x, a.y - y);
    let dd = dx * dx + dy * dy;
    let point = |s: f64| Point3::new(a.x + dx * s, a.y + dy * s, a.z + dz * s);
    let g = |s: f64| {
        let p = point(s);
        tool.height((p.x - x).hypot(p.y - y))
            .map_or(f64::NEG_INFINITY, |h| p.z - h)
    };
    if dd <= 1e-24 {
        // Vertical: its top end.
        let top = if a.z >= b.z { a } else { b };
        return tool.height((top.x - x).hypot(top.y - y)).map(|h| top.z - h);
    }
    // |a + s d - A|² <= R²: s in [s0, s1].
    let roots = quadratic(dd, ax * dx + ay * dy, ax * ax + ay * ay - rr * rr);
    if roots.len() < 2 {
        return None;
    }
    let s0 = roots[0].min(roots[1]).max(0.0);
    let s1 = roots[0].max(roots[1]).min(1.0);
    if s0 > s1 {
        return None;
    }
    let e = tool.flat();
    let r = tool.corner;
    let best = if r <= 1e-12 {
        // Flat: z is linear along the line, so the highest end of the part in reach.
        g(s0).max(g(s1))
    } else if e <= 1e-12 {
        // Ball: the centre c = (x, y, zc) is at distance r from the line. With the line's
        // closest point to the axis at parameter s*, the distance² from the axis point
        // (x, y, zc) to the line is a quadratic in zc; the largest root is the contact,
        // valid when its foot lies on the segment, else an end holds the ball.
        let len2 = dd + dz * dz;
        // Distance² from (x, y, zc) to the infinite line, as A zc² + 2 B zc + C.
        // w = P - a, |w × d|² / |d|² = |w|² - (w·d)² / |d|².
        // w = (-ax, -ay, zc - a.z).
        let wd0 = -ax * dx - ay * dy - a.z * dz;
        let w2_0 = ax * ax + ay * ay + a.z * a.z;
        let qa = 1.0 - dz * dz / len2;
        let qb = -a.z - wd0 * dz / len2;
        let qc = w2_0 - wd0 * wd0 / len2 - r * r;
        let mut v = g(s0).max(g(s1));
        for zc in quadratic(qa, qb, qc) {
            let s = ((wd0 + zc * dz) / len2).clamp(0.0, 1.0);
            let p = point(s);
            let dist = ((p.x - x).powi(2) + (p.y - y).powi(2) + (p.z - zc).powi(2)).sqrt();
            if (dist - r).abs() <= 1e-7 * r.max(1.0) && zc >= p.z - 1e-12 {
                v = v.max(zc - r);
            }
        }
        v
    } else {
        // Bull-nose: concave along the line, so golden-section search is exact to
        // rounding.
        golden(s0, s1, &g).1.max(g(s0)).max(g(s1))
    };
    best.is_finite().then_some(best)
}

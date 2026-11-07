//! Hidden-line removal for orthographic views, from the exact B-rep.
//!
//! Method (the classical edge-sampling approach, see Foley, van Dam, Feiner and Hughes,
//! *Computer Graphics: Principles and Practice*, 2nd ed., section 15.3 on visible-line
//! determination, and Appel's 1967 quantitative-invisibility idea reduced to a visible /
//! hidden test):
//!
//! 1. Every edge of every body is sampled along its exact curve. Seams (an edge with the
//!    same face on both sides) and smooth joins between two faces on one surface are not
//!    drawn: they are not edges on paper.
//! 2. A sample is hidden when the ray from it towards the viewer meets the inside of
//!    any face: an exact line-surface intersection (`intersect_curve_surface_on`), then
//!    the exact point-on-face test (`classify_point_on_face`). Faces the ray only grazes
//!    or runs along (a face seen edge-on), and hits on the face boundary, do not hide.
//!    Each face's bounding box (from the display tessellation, padded by its chord
//!    tolerance) only skips faces the ray cannot reach.
//! 3. Where two neighbouring samples disagree, bisection on the curve parameter finds the
//!    change to well below a micrometre.
//! 4. Curved faces get their silhouettes (contour generators): the curves on the surface
//!    where its normal is perpendicular to the direction of sight `d`, `n · d = 0`
//!    (Koenderink, "What does the occluding contour tell us about solid shape?",
//!    Perception 13, 1984; the same condition drives Hertzmann and Zorin, "Illustrating
//!    smooth surfaces", SIGGRAPH 2000). For the surfaces of revolution of `surface.rs`,
//!    `S = o + ρ(v) e(u) + h(v) z` with normal along `h' e(u) - ρ' z`, the condition is
//!    `h'(v) (e(u) · d) = ρ'(v) (z · d)`, solved here in closed form where it can be:
//!    - cylinder (`ρ' = 0`): the two generators with `e(u) · d = 0`;
//!    - cone (`ρ' = sin α`, `h' = cos α`): `cos(u - φ) = tan α (z · d) / |d_xy|`, two
//!      straight generators through the apex (none when the view looks into the cone's
//!      opening);
//!    - sphere: the great circle in the plane through the centre perpendicular to `d`;
//!    - torus (`ρ' = -a sin v`, `h' = a cos v`): for each `u`, `tan v = -(e(u) · d) /
//!      (z · d)`, two branches sampled densely in `u` (the curve is of degree 4, not a
//!      conic); seen along the axis these are the exact circles of radius `R ± a`, and
//!      seen side-on the top and bottom circles plus the two tube circles seen end-on.
//!
//!    Each silhouette is clipped to its face (the exact point-on-face test, with
//!    bisection where it enters or leaves), then tested for visibility like an edge.
//! 5. Circles seen end-on stay exact circles and arcs; other curves become polylines at
//!    the tessellator's resolution (`edge_polylines`). Overlaps are removed: of two
//!    collinear lines or concentric equal arcs only one is kept, and a hidden piece under
//!    a visible one is dropped.

use crate::{DrawingError, DrawingResult, ViewDirection};
use chamfera_geom::{Point2, Point3, Vec3};
use chamfera_kernel::{
    Body, Circle3, Cone, Curve, CurveSurfaceIntersection, EdgeId, FaceId, Line3, PointOnFace,
    Segment3, Surface, TessOptions, Torus, intersect_curve_surface_on, tessellate,
};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// Ray hits closer than this to the sample (mm) are the sample's own surface.
const T_MIN: f64 = 1e-6;
/// Hits on the edge's own faces closer than this (mm) are rounding on a grazing ray.
const T_OWN: f64 = 1e-3;
/// Two 2D points or lines closer than this (mm) are the same.
const TOL_2D: f64 = 1e-5;
/// Bisection steps when a visibility change is found between two samples.
const BISECTIONS: u32 = 30;
/// Samples along one edge: at least this many ...
const MIN_SAMPLES: usize = 8;
/// ... and at most this many.
const MAX_SAMPLES: usize = 400;
/// The sample spacing is the view's size divided by this.
const SAMPLES_ACROSS: f64 = 400.0;
/// Most edges and faces one view may hold.
pub const MAX_VIEW_ENTITIES: usize = 20_000;
/// Most 2D lines one view keeps overlap removal for (it is quadratic).
const MAX_OVERLAP_LINES: usize = 20_000;
/// Samples along one branch of a torus silhouette (one per degree).
const TORUS_SAMPLES: usize = 360;

/// A view's axes in model space: `right` and `up` on paper, `toward` the viewer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Basis {
    pub right: Vec3,
    pub up: Vec3,
    pub toward: Vec3,
}

fn vec_of(p: Point3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

impl Basis {
    /// The axes of a standard view. Z is up; the front view is seen from -Y.
    pub fn of(d: ViewDirection) -> Basis {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        match d {
            ViewDirection::Front => Basis {
                right: x,
                up: z,
                toward: -y,
            },
            ViewDirection::Back => Basis {
                right: -x,
                up: z,
                toward: y,
            },
            ViewDirection::Right => Basis {
                right: y,
                up: z,
                toward: x,
            },
            ViewDirection::Left => Basis {
                right: -y,
                up: z,
                toward: -x,
            },
            ViewDirection::Top => Basis {
                right: x,
                up: y,
                toward: z,
            },
            ViewDirection::Bottom => Basis {
                right: x,
                up: -y,
                toward: -z,
            },
            ViewDirection::Iso => {
                // From the front-right-top corner: toward (1, -1, 1)/√3, right in the XY
                // plane, up = toward × right (Z appears vertical).
                let s3 = 1.0 / 3f64.sqrt();
                let s2 = 1.0 / 2f64.sqrt();
                let toward = Vec3::new(s3, -s3, s3);
                let right = Vec3::new(s2, s2, 0.0);
                Basis {
                    right,
                    up: toward.cross(right),
                    toward,
                }
            }
        }
    }

    /// The point on paper (model mm along `right` and `up`).
    pub fn project(&self, p: Point3) -> Point2 {
        let v = vec_of(p);
        Point2::new(v.dot(self.right), v.dot(self.up))
    }
}

/// A 2D curve in a view's coordinates (model mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve2 {
    Line {
        a: Point2,
        b: Point2,
    },
    Circle {
        centre: Point2,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` (radians, `start < end < start + 2π`).
    Arc {
        centre: Point2,
        radius: f64,
        start: f64,
        end: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewCurve {
    pub curve: Curve2,
    pub hidden: bool,
}

/// One projected view, in its own coordinates (model mm along the basis).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViewGeometry {
    pub curves: Vec<ViewCurve>,
    /// Circles seen end-on: centre and (largest) radius, for centre marks.
    pub centre_marks: Vec<(Point2, f64)>,
    /// Axes of whole cylinders and cones seen side-on, end to end.
    pub centre_lines: Vec<[Point2; 2]>,
    /// Cut faces of a section view, to hatch: each region is its boundary loops, closed
    /// polygons (the last point joins the first), filled by the even-odd rule.
    pub regions: Vec<Vec<Vec<Point2>>>,
    /// Which body each region cuts, by index (for the hatch angle).
    pub region_parts: Vec<usize>,
}

impl ViewGeometry {
    /// The points a dimension can snap to: line ends, arc ends, circle centres.
    pub fn vertices(&self) -> Vec<Point2> {
        let mut out: Vec<Point2> = Vec::new();
        let mut push = |p: Point2| {
            if !out.iter().any(|q| q.distance(p) < TOL_2D) {
                out.push(p);
            }
        };
        for c in &self.curves {
            match c.curve {
                Curve2::Line { a, b } => {
                    push(a);
                    push(b);
                }
                Curve2::Circle { centre, .. } => push(centre),
                Curve2::Arc {
                    centre,
                    radius,
                    start,
                    end,
                } => {
                    push(centre);
                    push(polar(centre, radius, start));
                    push(polar(centre, radius, end));
                }
            }
        }
        out
    }

    /// The smallest box around every curve, or `None` for an empty view.
    pub fn bounds(&self) -> Option<(Point2, Point2)> {
        let mut b: Option<(Point2, Point2)> = None;
        let mut add = |p: Point2| {
            b = Some(match b {
                None => (p, p),
                Some((lo, hi)) => (
                    Point2::new(lo.x.min(p.x), lo.y.min(p.y)),
                    Point2::new(hi.x.max(p.x), hi.y.max(p.y)),
                ),
            });
        };
        for c in &self.curves {
            match c.curve {
                Curve2::Line { a, b } => {
                    add(a);
                    add(b);
                }
                Curve2::Circle { centre, radius } | Curve2::Arc { centre, radius, .. } => {
                    add(Point2::new(centre.x - radius, centre.y - radius));
                    add(Point2::new(centre.x + radius, centre.y + radius));
                }
            }
        }
        b
    }
}

pub(crate) fn polar(c: Point2, r: f64, a: f64) -> Point2 {
    Point2::new(c.x + r * a.cos(), c.y + r * a.sin())
}

/// A face that may hide things, with its padded box.
struct Occluder<'a> {
    body: usize,
    face: FaceId,
    owner: &'a Body,
    surface: Surface,
    min: Point3,
    max: Point3,
}

/// Everything the visibility test needs.
struct Scene<'a> {
    basis: Basis,
    occluders: Vec<Occluder<'a>>,
    /// The sample spacing (mm).
    step: f64,
}

impl Scene<'_> {
    /// True when the ray from `p` towards the viewer enters the inside of a face. Hits
    /// on `own` faces (the edge's or silhouette's faces) only count from `T_OWN` on.
    fn hidden(&self, p: Point3, own: &[(usize, FaceId)]) -> bool {
        let Ok(line) = Line3::new(p, self.basis.toward) else {
            return false;
        };
        let ray = Curve::Line(line);
        let d = self.basis.toward;
        for o in &self.occluders {
            if !ray_meets_box(p, d, o.min, o.max) {
                continue;
            }
            let Ok(CurveSurfaceIntersection::Points(hits)) =
                intersect_curve_surface_on(&ray, T_MIN, f64::INFINITY, &o.surface)
            else {
                continue;
            };
            let near = if own.contains(&(o.body, o.face)) {
                T_OWN
            } else {
                T_MIN
            };
            for h in hits {
                if h.t <= near || !in_box(h.point, o.min, o.max) {
                    continue;
                }
                if matches!(
                    o.owner.classify_point_on_face(o.face, h.point),
                    Ok(PointOnFace::Inside)
                ) {
                    return true;
                }
            }
        }
        false
    }
}

fn in_box(p: Point3, min: Point3, max: Point3) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y && p.z >= min.z && p.z <= max.z
}

/// Slab test: does the ray `p + t d`, `t >= 0`, meet the box?
fn ray_meets_box(p: Point3, d: Vec3, min: Point3, max: Point3) -> bool {
    let mut lo = 0.0f64;
    let mut hi = f64::INFINITY;
    for (o, dir, a, b) in [
        (p.x, d.x, min.x, max.x),
        (p.y, d.y, min.y, max.y),
        (p.z, d.z, min.z, max.z),
    ] {
        if dir.abs() < 1e-15 {
            if o < a || o > b {
                return false;
            }
        } else {
            let (t0, t1) = ((a - o) / dir, (b - o) / dir);
            let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
            lo = lo.max(t0);
            hi = hi.min(t1);
            if lo > hi {
                return false;
            }
        }
    }
    true
}

/// Raw projected pieces before overlap removal.
#[derive(Default)]
struct Raw {
    lines: Vec<([Point2; 2], bool)>,
    /// (centre, radius, start, end, hidden), angles counter-clockwise.
    arcs: Vec<(Point2, f64, f64, f64, bool)>,
    centre_marks: Vec<(Point2, f64)>,
    centre_lines: Vec<[Point2; 2]>,
}

/// Projects `bodies` as seen from `direction`, with hidden lines.
pub fn project(bodies: &[&Body], direction: ViewDirection) -> DrawingResult<ViewGeometry> {
    project_basis(bodies, Basis::of(direction))
}

/// Projects `bodies` along any orthonormal `basis` (a section view's), with hidden lines.
pub fn project_basis(bodies: &[&Body], basis: Basis) -> DrawingResult<ViewGeometry> {
    let options = TessOptions::default();
    let mut occluders = Vec::new();
    let mut polylines: Vec<HashMap<EdgeId, usize>> = Vec::new();
    let mut extent: Option<(Point3, Point3)> = None;
    let mut entities = 0usize;
    for (bi, body) in bodies.iter().enumerate() {
        let s = body.summary();
        entities = entities.saturating_add(s.edges).saturating_add(s.faces);
        if entities > MAX_VIEW_ENTITIES {
            return Err(DrawingError::BadInput(format!(
                "a view can show at most {MAX_VIEW_ENTITIES} edges and faces"
            )));
        }
        let mesh = tessellate(body, &options)?;
        let mut boxes: HashMap<FaceId, (Point3, Point3)> = HashMap::new();
        for (i, f) in mesh.face_of_triangle.iter().enumerate() {
            let Some(tri) = mesh.mesh.triangle(i) else {
                continue;
            };
            for p in tri {
                let e = boxes.entry(*f).or_insert((p, p));
                e.0 = Point3::new(e.0.x.min(p.x), e.0.y.min(p.y), e.0.z.min(p.z));
                e.1 = Point3::new(e.1.x.max(p.x), e.1.y.max(p.y), e.1.z.max(p.z));
            }
        }
        let pad = options.chord_tolerance * 2.0 + 1e-6;
        for (f, (lo, hi)) in boxes {
            let Some((surface, _)) = body.face_surface(f)? else {
                continue;
            };
            let min = Point3::new(lo.x - pad, lo.y - pad, lo.z - pad);
            let max = Point3::new(hi.x + pad, hi.y + pad, hi.z + pad);
            extent = Some(match extent {
                None => (min, max),
                Some((a, b)) => (
                    Point3::new(a.x.min(min.x), a.y.min(min.y), a.z.min(min.z)),
                    Point3::new(b.x.max(max.x), b.y.max(max.y), b.z.max(max.z)),
                ),
            });
            occluders.push(Occluder {
                body: bi,
                face: f,
                owner: body,
                surface,
                min,
                max,
            });
        }
        polylines.push(
            mesh.edge_polylines
                .iter()
                .map(|(e, pts)| (*e, pts.len().saturating_sub(1).max(1)))
                .collect(),
        );
    }
    // A stable order, so the same model always gives the same drawing.
    occluders.sort_by_key(|o| (o.body, o.face.index()));
    let Some((lo, hi)) = extent else {
        return Ok(ViewGeometry::default());
    };
    let size = (hi - lo).length();
    let step = (size / SAMPLES_ACROSS).max(1e-4);
    let scene = Scene {
        basis,
        occluders,
        step,
    };
    let mut raw = Raw::default();
    for (bi, body) in bodies.iter().enumerate() {
        for e in body.edges() {
            let [f1, f2] = body.edge_faces(e)?;
            if f1 == f2 {
                continue; // a seam
            }
            let s1 = body.face_surface(f1)?;
            let s2 = body.face_surface(f2)?;
            if let (Some((a, _)), Some((b, _))) = (s1, s2)
                && a == b
            {
                continue; // a smooth join of two faces on one surface
            }
            let Some((curve, t0, t1)) = body.edge_curve(e)? else {
                continue;
            };
            let segments = polylines
                .get(bi)
                .and_then(|m| m.get(&e))
                .copied()
                .unwrap_or(1);
            edge_pieces(
                &scene,
                &curve,
                t0,
                t1,
                segments,
                &[(bi, f1), (bi, f2)],
                &mut raw,
            );
        }
        for f in body.faces() {
            match body.face_surface(f)? {
                Some((Surface::Cylinder(c), _)) => {
                    silhouettes(&scene, body, bi, f, c.frame(), c.radius(), &mut raw)?;
                }
                Some((Surface::Cone(c), _)) => {
                    cone_silhouettes(&scene, body, bi, f, &c, &mut raw)?;
                }
                Some((Surface::Sphere(sp), _)) => {
                    let (o, r) = (sp.frame().origin(), sp.radius());
                    if let Some(fr) = chamfera_geom::Frame::from_normal(o, scene.basis.toward)
                        && let Ok(c) = Circle3::new(fr, r)
                    {
                        circle_silhouette(&scene, body, bi, f, &c, &mut raw);
                    }
                }
                Some((Surface::Torus(t), _)) => {
                    torus_silhouettes(&scene, body, bi, f, &t, &mut raw);
                }
                _ => {}
            }
        }
    }
    Ok(finish(raw))
}

/// Visible and hidden runs of a curve over `[t0, t1]`: `(ta, tb, hidden)`.
fn runs(
    scene: &Scene,
    curve: &Curve,
    t0: f64,
    t1: f64,
    own: &[(usize, FaceId)],
) -> Vec<(f64, f64, bool)> {
    let length = approx_length(curve, t0, t1);
    let n = ((length / scene.step).ceil() as usize).clamp(MIN_SAMPLES, MAX_SAMPLES);
    let inset = (t1 - t0) * 1e-4;
    let t_at = |i: usize| {
        if i == 0 {
            t0 + inset
        } else if i == n {
            t1 - inset
        } else {
            t0 + (t1 - t0) * i as f64 / n as f64
        }
    };
    let hidden_at = |t: f64| scene.hidden(curve.point_at(t), own);
    let mut out = Vec::new();
    let mut start = t0;
    let mut prev_t = t_at(0);
    let mut prev = hidden_at(prev_t);
    for i in 1..=n {
        let t = t_at(i);
        let h = hidden_at(t);
        if h != prev {
            let (mut a, mut b) = (prev_t, t);
            for _ in 0..BISECTIONS {
                let m = 0.5 * (a + b);
                if hidden_at(m) == prev {
                    a = m;
                } else {
                    b = m;
                }
            }
            let cut = 0.5 * (a + b);
            out.push((start, cut, prev));
            start = cut;
            prev = h;
        }
        prev_t = t;
    }
    out.push((start, t1, prev));
    out
}

fn approx_length(curve: &Curve, t0: f64, t1: f64) -> f64 {
    let k = 32;
    let mut len = 0.0;
    let mut p = curve.point_at(t0);
    for i in 1..=k {
        let q = curve.point_at(t0 + (t1 - t0) * i as f64 / k as f64);
        len += (q - p).length();
        p = q;
    }
    len
}

/// Projects one edge's (or silhouette's) visible and hidden pieces into `raw`.
fn edge_pieces(
    scene: &Scene,
    curve: &Curve,
    t0: f64,
    t1: f64,
    segments: usize,
    own: &[(usize, FaceId)],
    raw: &mut Raw,
) {
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return;
    }
    let b = &scene.basis;
    // A circle or arc seen end-on stays exact.
    let end_on = match curve {
        Curve::Circle(c) => Some((c.center(), c.normal(), c.radius(), *c.frame())),
        Curve::Arc(a) => {
            let c = a.circle();
            Some((c.center(), c.normal(), c.radius(), *c.frame()))
        }
        _ => None,
    }
    .filter(|(_, n, _, _)| n.dot(b.toward).abs() > 1.0 - 1e-9);
    if let Some((centre, normal, radius, _)) = end_on {
        let c2 = b.project(centre);
        if is_full_circle(curve, t0, t1) {
            raw.centre_marks.push((c2, radius));
        }
        for (ta, tb, hidden) in runs(scene, curve, t0, t1, own) {
            let pa = b.project(curve.point_at(ta));
            let pb = b.project(curve.point_at(tb));
            let ang = |p: Point2| (p.y - c2.y).atan2(p.x - c2.x);
            let sweep = tb - ta;
            // Counter-clockwise on paper when the circle's normal faces the viewer.
            let start = if normal.dot(b.toward) > 0.0 {
                ang(pa)
            } else {
                ang(pb)
            };
            raw.arcs.push((c2, radius, start, start + sweep, hidden));
        }
        return;
    }
    let straight = matches!(curve, Curve::Line(_) | Curve::Segment(_));
    for (ta, tb, hidden) in runs(scene, curve, t0, t1, own) {
        let m = if straight {
            1
        } else {
            ((segments as f64 * (tb - ta) / (t1 - t0)).ceil() as usize).clamp(1, MAX_SAMPLES)
        };
        let mut ts: Vec<f64> = (0..=m)
            .map(|k| ta + (tb - ta) * k as f64 / m as f64)
            .collect();
        // A circle's extreme points along the paper's axes are exact samples, so a circle
        // seen edge-on (a line on paper) or obliquely (an ellipse) keeps its true extent.
        if let Some(frame) = match curve {
            Curve::Circle(c) => Some(*c.frame()),
            Curve::Arc(a) => Some(*a.circle().frame()),
            _ => None,
        } {
            for s in [b.right, b.up] {
                let t0 = frame.y().dot(s).atan2(frame.x().dot(s));
                for k in -3..=3 {
                    let t = t0 + std::f64::consts::PI * f64::from(k);
                    if t > ta && t < tb {
                        ts.push(t);
                    }
                }
            }
            ts.sort_by(f64::total_cmp);
        }
        let points: Vec<Point2> = ts.iter().map(|t| b.project(curve.point_at(*t))).collect();
        for seg in simplify(&points) {
            raw.lines.push((seg, hidden));
        }
    }
}

/// A circle, or an arc over a whole turn.
fn is_full_circle(curve: &Curve, t0: f64, t1: f64) -> bool {
    matches!(curve, Curve::Circle(_) | Curve::Arc(_)) && t1 - t0 >= TAU - 1e-9
}

/// Joins consecutive collinear pieces running the same way; drops zero-length ones.
fn simplify(points: &[Point2]) -> Vec<[Point2; 2]> {
    let mut out: Vec<[Point2; 2]> = Vec::new();
    for w in points.windows(2) {
        let [a, b] = [w[0], w[1]];
        if a.distance(b) < TOL_2D {
            continue;
        }
        if let Some(last) = out.last_mut() {
            let u = last[1] - last[0];
            let v = b - a;
            let (lu, lv) = (u.length(), v.length());
            if last[1].distance(a) < TOL_2D
                && lu > 0.0
                && lv > 0.0
                && (u.cross(v) / (lu * lv)).abs() < 1e-9
                && u.dot(v) > 0.0
            {
                last[1] = b;
                continue;
            }
        }
        out.push([a, b]);
    }
    out
}

/// The outline lines of a cylinder face seen side-on, and its centre line when the face
/// goes all the way round.
fn silhouettes(
    scene: &Scene,
    body: &Body,
    bi: usize,
    face: FaceId,
    frame: &chamfera_geom::Frame,
    radius: f64,
    raw: &mut Raw,
) -> DrawingResult<()> {
    let axis = frame.z();
    let origin = frame.origin();
    let side = axis.cross(scene.basis.toward);
    let Some(n) = side.normalized().filter(|_| side.length() > 1e-6) else {
        return Ok(()); // seen end-on: its circles are the outline
    };
    let (smin, smax, whole) = axial_extent(body, face, origin, axis)?;
    if !(smin.is_finite() && smax.is_finite() && smax - smin > TOL_2D) {
        return Ok(());
    }
    if whole {
        let a = scene.basis.project(origin + axis * smin);
        let b = scene.basis.project(origin + axis * smax);
        if a.distance(b) > TOL_2D {
            raw.centre_lines.push([a, b]);
        }
    }
    let own = [(bi, face)];
    for sign in [1.0, -1.0] {
        let base = origin + n * (radius * sign);
        segment_on_face(scene, body, face, (base, axis), (smin, smax), &own, raw);
    }
    Ok(())
}

/// The parameter ranges of `[t0, t1]` where `on(t)` holds, found by sampling `n` steps
/// and bisecting each change. For a `closed` curve a range ending at `t1` and one
/// starting at `t0` are joined across the seam (the joined range then runs past `t1`).
fn spans(n: usize, t0: f64, t1: f64, closed: bool, on: impl Fn(f64) -> bool) -> Vec<(f64, f64)> {
    let n = n.max(1);
    let t_at = |i: usize| t0 + (t1 - t0) * i as f64 / n as f64;
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut prev_t = t_at(0);
    let mut prev = on(prev_t);
    let mut start = prev.then_some(prev_t);
    for i in 1..=n {
        let t = t_at(i);
        let now = on(t);
        if now != prev {
            let (mut a, mut b) = (prev_t, t);
            for _ in 0..BISECTIONS {
                let m = 0.5 * (a + b);
                if on(m) == prev {
                    a = m;
                } else {
                    b = m;
                }
            }
            let cut = 0.5 * (a + b);
            if now {
                start = Some(cut);
            } else if let Some(s0) = start.take() {
                out.push((s0, cut));
            }
            prev = now;
        }
        prev_t = t;
    }
    if let Some(s0) = start {
        out.push((s0, t1));
    }
    if closed
        && out.len() >= 2
        && out.first().is_some_and(|f| f.0 <= t0)
        && out.last().is_some_and(|l| l.1 >= t1)
        && let Some(last) = out.pop()
        && let Some(first) = out.first_mut()
    {
        *first = (last.0, first.1 + (t1 - t0));
    }
    out
}

/// True when `p` is on the face (inside or on its boundary).
fn on_face(body: &Body, face: FaceId, p: Point3) -> bool {
    !matches!(
        body.classify_point_on_face(face, p),
        Ok(PointOnFace::Outside) | Err(_)
    )
}

/// The parts of the line `base + dir s`, `s` in `range`, that lie on the face, projected
/// with their visibility.
fn segment_on_face(
    scene: &Scene,
    body: &Body,
    face: FaceId,
    (base, dir): (Point3, Vec3),
    (smin, smax): (f64, f64),
    own: &[(usize, FaceId)],
    raw: &mut Raw,
) {
    if !(smin.is_finite() && smax.is_finite() && smax - smin > TOL_2D) {
        return;
    }
    let k = (((smax - smin) / scene.step).ceil() as usize).clamp(MIN_SAMPLES, MAX_SAMPLES);
    for (sa, sb) in spans(k, smin, smax, false, |s| {
        on_face(body, face, base + dir * s)
    }) {
        if sb - sa <= TOL_2D {
            continue;
        }
        let Ok(seg) = Segment3::new(base + dir * sa, base + dir * sb) else {
            continue;
        };
        let curve = Curve::Segment(seg);
        let (t0, t1) = curve.range();
        edge_pieces(scene, &curve, t0, t1, 1, own, raw);
    }
}

/// The face's extent along `axis` from `origin`, from points on its edges, and whether
/// the face goes all the way round (it has a seam or a whole circle).
fn axial_extent(
    body: &Body,
    face: FaceId,
    origin: Point3,
    axis: Vec3,
) -> DrawingResult<(f64, f64, bool)> {
    let mut smin = f64::INFINITY;
    let mut smax = f64::NEG_INFINITY;
    let mut whole = false;
    for e in body.face_edges(face)? {
        let [f1, f2] = body.edge_faces(e)?;
        let Some((curve, t0, t1)) = body.edge_curve(e)? else {
            continue;
        };
        if f1 == f2 || is_full_circle(&curve, t0, t1) {
            whole = true;
        }
        for k in 0..=32 {
            let p = curve.point_at(t0 + (t1 - t0) * f64::from(k) / 32.0);
            let s = (p - origin).dot(axis);
            smin = smin.min(s);
            smax = smax.max(s);
        }
    }
    Ok((smin, smax, whole))
}

/// The two outline generators of a cone face seen side-on, through the apex, and its
/// axis as a centre line when the face goes all the way round.
fn cone_silhouettes(
    scene: &Scene,
    body: &Body,
    bi: usize,
    face: FaceId,
    cone: &Cone,
    raw: &mut Raw,
) -> DrawingResult<()> {
    let f = cone.frame();
    let (x, y, z) = (f.x(), f.y(), f.z());
    let d = scene.basis.toward;
    let (sa, ca) = cone.half_angle().sin_cos();
    let apex = cone.apex();
    // The face's extent along the axis from the apex. The point A + s g(u) of the
    // generator g(u) = sin α e(u) + cos α z is at height s cos α above the apex.
    let (hmin, hmax, whole) = axial_extent(body, face, apex, z)?;
    if !(hmin.is_finite() && hmax.is_finite()) || ca.abs() < 1e-12 {
        return Ok(());
    }
    let (s1, s2) = (hmin / ca, hmax / ca);
    let (smin, smax) = (s1.min(s2), s1.max(s2));
    if whole {
        let a = scene.basis.project(apex + z * hmin);
        let b = scene.basis.project(apex + z * hmax);
        if a.distance(b) > TOL_2D {
            raw.centre_lines.push([a, b]);
        }
    }
    let (p, q) = (x.dot(d), y.dot(d));
    let rho = p.hypot(q);
    if rho < 1e-9 {
        return Ok(()); // seen along the axis: its circles are the outline
    }
    // n · d = cos α (e(u) · d) - sin α (z · d) = 0, with e(u) · d = ρ cos(u - φ).
    let k = (sa / ca) * z.dot(d) / rho;
    if k.is_nan() || k.abs() > 1.0 {
        return Ok(()); // looking into the cone's opening: no silhouette
    }
    let phi = q.atan2(p);
    let delta = k.acos();
    let own = [(bi, face)];
    for u in [phi + delta, phi - delta] {
        let (su, cu) = u.sin_cos();
        let g = (x * cu + y * su) * sa + z * ca;
        segment_on_face(scene, body, face, (apex, g), (smin, smax), &own, raw);
    }
    Ok(())
}

/// The parts of a circle that lie on the face, projected with their visibility (exact
/// arcs when the circle is seen end-on).
fn circle_silhouette(
    scene: &Scene,
    body: &Body,
    bi: usize,
    face: FaceId,
    circle: &Circle3,
    raw: &mut Raw,
) {
    let curve = Curve::Circle(*circle);
    let n = ((TAU * circle.radius() / scene.step).ceil() as usize).clamp(64, MAX_SAMPLES);
    let own = [(bi, face)];
    let pieces = spans(n, 0.0, TAU, true, |t| {
        on_face(body, face, curve.point_at(t))
    });
    for (ta, tb) in pieces {
        if (tb - ta) * circle.radius() > TOL_2D {
            edge_pieces(scene, &curve, ta, tb, 64, &own, raw);
        }
    }
}

/// The outline of a torus face (see the module notes).
fn torus_silhouettes(
    scene: &Scene,
    body: &Body,
    bi: usize,
    face: FaceId,
    torus: &Torus,
    raw: &mut Raw,
) {
    let f = torus.frame();
    let (o, x, y, z) = (f.origin(), f.x(), f.y(), f.z());
    let (big, a) = (torus.major_radius(), torus.minor_radius());
    let d = scene.basis.toward;
    let w = z.dot(d);
    let (p, q) = (x.dot(d), y.dot(d));
    let circle = |centre: Point3, normal: Vec3, r: f64| {
        chamfera_geom::Frame::from_normal(centre, normal).and_then(|fr| Circle3::new(fr, r).ok())
    };
    if 1.0 - w.abs() < 1e-12 {
        // Along the axis: the equator's outer and inner circles.
        for r in [big + a, (big - a).abs()] {
            if r > TOL_2D
                && let Some(c) = circle(o, z, r)
            {
                circle_silhouette(scene, body, bi, face, &c, raw);
            }
        }
        return;
    }
    if w.abs() < 1e-12 {
        // Side-on: the top and bottom circles (v = ±π/2), and the tube circles seen
        // end-on where e(u) is perpendicular to d.
        for h in [a, -a] {
            if big.abs() > TOL_2D
                && let Some(c) = circle(o + z * h, z, big.abs())
            {
                circle_silhouette(scene, body, bi, face, &c, raw);
            }
        }
        let phi = q.atan2(p);
        for u in [
            phi + std::f64::consts::FRAC_PI_2,
            phi - std::f64::consts::FRAC_PI_2,
        ] {
            let (su, cu) = u.sin_cos();
            let e = x * cu + y * su;
            let t = y * cu - x * su;
            if let Some(c) = circle(o + e * big, t, a) {
                circle_silhouette(scene, body, bi, face, &c, raw);
            }
        }
        return;
    }
    // In general: for each u, the v with cos v (e(u) · d) + sin v (z · d) = 0, on two
    // branches half a turn of v apart.
    let point = |u: f64, branch: f64| {
        let (su, cu) = u.sin_cos();
        let e = x * cu + y * su;
        let m = p * cu + q * su;
        let v = (-m).atan2(w) + branch;
        let (sv, cv) = v.sin_cos();
        o + e * (big + a * cv) + z * (a * sv)
    };
    let own = [(bi, face)];
    let n = TORUS_SAMPLES;
    for branch in [0.0, std::f64::consts::PI] {
        for (ua, ub) in spans(n, 0.0, TAU, true, |u| on_face(body, face, point(u, branch))) {
            let m = (((ub - ua) / TAU * n as f64).ceil() as usize).clamp(1, n);
            for i in 0..m {
                let t0 = ua + (ub - ua) * i as f64 / m as f64;
                let t1 = ua + (ub - ua) * (i + 1) as f64 / m as f64;
                let Ok(seg) = Segment3::new(point(t0, branch), point(t1, branch)) else {
                    continue;
                };
                let curve = Curve::Segment(seg);
                let (s0, s1) = curve.range();
                edge_pieces(scene, &curve, s0, s1, 1, &own, raw);
            }
        }
    }
}

/// Removes overlaps and builds the view.
fn finish(raw: Raw) -> ViewGeometry {
    let mut out = ViewGeometry::default();
    // Lines: visible first, each keeping only what earlier lines do not cover; then
    // hidden ones, minus everything visible and earlier hidden lines.
    let dedup = raw.lines.len() <= MAX_OVERLAP_LINES;
    let mut visible: Vec<[Point2; 2]> = Vec::new();
    for (seg, hidden) in &raw.lines {
        if !hidden {
            let pieces = if dedup {
                uncovered(*seg, &visible)
            } else {
                vec![*seg]
            };
            visible.extend(pieces);
        }
    }
    let mut all = visible.clone();
    let mut hidden_lines = Vec::new();
    for (seg, hidden) in &raw.lines {
        if *hidden {
            let pieces = if dedup {
                uncovered(*seg, &all)
            } else {
                vec![*seg]
            };
            all.extend(pieces.iter().copied());
            hidden_lines.extend(pieces);
        }
    }
    for [a, b] in visible {
        out.curves.push(ViewCurve {
            curve: Curve2::Line { a, b },
            hidden: false,
        });
    }
    for [a, b] in hidden_lines {
        out.curves.push(ViewCurve {
            curve: Curve2::Line { a, b },
            hidden: true,
        });
    }
    // Arcs: group equal circles, join visible intervals, take them from the hidden ones.
    let mut groups: Vec<(Point2, f64, Intervals, Intervals)> = Vec::new();
    for (c, r, s, e, hidden) in raw.arcs {
        let i = match groups
            .iter()
            .position(|g| g.0.distance(c) < TOL_2D && (g.1 - r).abs() < TOL_2D)
        {
            Some(i) => i,
            None => {
                groups.push((c, r, Intervals::default(), Intervals::default()));
                groups.len() - 1
            }
        };
        if let Some(g) = groups.get_mut(i) {
            if hidden {
                g.3.add_arc(s, e);
            } else {
                g.2.add_arc(s, e);
            }
        }
    }
    for (c, r, vis, hid) in groups {
        let hid = hid.minus(&vis);
        for (set, hidden) in [(vis, false), (hid, true)] {
            for curve in set.curves(c, r) {
                out.curves.push(ViewCurve { curve, hidden });
            }
        }
    }
    for (c, r) in raw.centre_marks {
        match out
            .centre_marks
            .iter_mut()
            .find(|m| m.0.distance(c) < TOL_2D)
        {
            Some(m) => m.1 = m.1.max(r),
            None => out.centre_marks.push((c, r)),
        }
    }
    let mut lines: Vec<[Point2; 2]> = Vec::new();
    for l in raw.centre_lines {
        let pieces = uncovered(l, &lines);
        lines.extend(pieces);
    }
    // Collinear pieces of one axis (a stepped shaft) become one centre line.
    for l in lines {
        if let Some(m) = out.centre_lines.iter_mut().find(|m| collinear(**m, l)) {
            *m = span(*m, l);
        } else {
            out.centre_lines.push(l);
        }
    }
    out
}

fn collinear(a: [Point2; 2], b: [Point2; 2]) -> bool {
    let d = a[1] - a[0];
    let len = d.length();
    len > 0.0
        && (d.cross(b[0] - a[0]) / len).abs() < TOL_2D
        && (d.cross(b[1] - a[0]) / len).abs() < TOL_2D
}

/// The longest segment along `a` that holds both `a` and `b` (they are collinear).
fn span(a: [Point2; 2], b: [Point2; 2]) -> [Point2; 2] {
    let d = a[1] - a[0];
    let len2 = d.dot(d);
    if len2 <= 0.0 {
        return a;
    }
    let t = |p: Point2| (p - a[0]).dot(d) / len2;
    let ts = [0.0, 1.0, t(b[0]), t(b[1])];
    let lo = ts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    [a[0] + d * lo, a[0] + d * hi]
}

/// The parts of `seg` that no collinear segment of `covers` overlaps.
fn uncovered(seg: [Point2; 2], covers: &[[Point2; 2]]) -> Vec<[Point2; 2]> {
    let [a, b] = seg;
    let d = b - a;
    let len = d.length();
    if len < TOL_2D {
        return Vec::new();
    }
    let u = d * (1.0 / len);
    let mut taken: Vec<(f64, f64)> = Vec::new();
    for c in covers {
        if u.cross(c[0] - a).abs() > TOL_2D || u.cross(c[1] - a).abs() > TOL_2D {
            continue;
        }
        let (s0, s1) = ((c[0] - a).dot(u), (c[1] - a).dot(u));
        let (s0, s1) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
        let (s0, s1) = (s0.max(0.0), s1.min(len));
        if s1 - s0 > 0.0 {
            taken.push((s0, s1));
        }
    }
    taken.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut out = Vec::new();
    let mut at = 0.0;
    for (s0, s1) in taken {
        if s0 - at > TOL_2D {
            out.push([a + u * at, a + u * s0]);
        }
        at = f64::max(at, s1);
    }
    if len - at > TOL_2D {
        out.push([a + u * at, b]);
    }
    out
}

/// A union of closed angle intervals within `[0, 2π]`.
#[derive(Debug, Clone, Default)]
struct Intervals(Vec<(f64, f64)>);

impl Intervals {
    fn add_arc(&mut self, start: f64, end: f64) {
        let sweep = end - start;
        if !(sweep.is_finite() && start.is_finite()) || sweep <= 0.0 {
            return;
        }
        if sweep >= TAU - 1e-9 {
            self.add(0.0, TAU);
            return;
        }
        let s = start.rem_euclid(TAU);
        let e = s + sweep;
        if e <= TAU {
            self.add(s, e);
        } else {
            self.add(s, TAU);
            self.add(0.0, e - TAU);
        }
    }

    fn add(&mut self, s: f64, e: f64) {
        self.0.push((s, e));
        self.0.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (s, e) in self.0.drain(..) {
            match merged.last_mut() {
                Some(last) if s <= last.1 + 1e-9 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        self.0 = merged;
    }

    fn minus(&self, o: &Intervals) -> Intervals {
        let mut out = Vec::new();
        for &(s, e) in &self.0 {
            let mut pieces = vec![(s, e)];
            for &(os, oe) in &o.0 {
                let mut next = Vec::new();
                for (ps, pe) in pieces {
                    if oe <= ps || os >= pe {
                        next.push((ps, pe));
                        continue;
                    }
                    if os > ps {
                        next.push((ps, os));
                    }
                    if oe < pe {
                        next.push((oe, pe));
                    }
                }
                pieces = next;
            }
            out.extend(pieces.into_iter().filter(|(a, b)| b - a > 1e-9));
        }
        Intervals(out)
    }

    /// Circles and arcs, joining a piece ending at 2π with one starting at 0.
    fn curves(&self, centre: Point2, radius: f64) -> Vec<Curve2> {
        let mut v = self.0.clone();
        if v.len() == 1
            && v.first()
                .is_some_and(|&(s, e)| s <= 1e-9 && e >= TAU - 1e-9)
        {
            return vec![Curve2::Circle { centre, radius }];
        }
        if v.len() >= 2
            && v.first().is_some_and(|f| f.0 <= 1e-9)
            && v.last().is_some_and(|l| l.1 >= TAU - 1e-9)
            && let (Some(first), Some(last)) = (v.first().copied(), v.pop())
            && let Some(f) = v.first_mut()
        {
            *f = (last.0, first.1 + TAU);
        }
        v.into_iter()
            .filter(|(s, e)| (e - s) * radius > TOL_2D)
            .map(|(start, end)| Curve2::Arc {
                centre,
                radius,
                start,
                end,
            })
            .collect()
    }
}

//! Lofts (rung K12): [`loft`] joins two or more planar closed profiles on parallel
//! planes into a valid solid [`Body`], where the result can be exact.
//!
//! # Surfaces
//! Consecutive sections are joined edge to edge by ruled surfaces (Farin, *Curves and
//! Surfaces for CAGD*, 5th ed., 2002, section 20.1: the ruled surface between two curves
//! is `(1 - v) a(u) + v b(u)`). With both curves on parallel planes:
//! - two parallel lines span a planar trapezoid;
//! - two coaxial circles, or coaxial arcs over the same angles, span a circular cone (a
//!   cylinder when the radii are equal): the ruling joins points at the same angle, which
//!   is a generator of that cone.
//!
//! Anything else is refused with [`KernelError::Unsupported`] until the kernel has the
//! surfaces it needs: skew lines (a hyperbolic paraboloid), non-coaxial or rotated arcs
//! (an oblique cone, not circular), a line against an arc and general curves (B-spline
//! surfaces, K10).
//!
//! # Matching edges
//! Every profile needs the same number of edges and no holes. Each loop is oriented
//! counter-clockwise about the loft's direction (from the first section toward the
//! last); the edges of each section are then matched to the previous section's by the
//! cyclic shift that puts each vertex nearest (in the plane) to the one it is joined to,
//! among the shifts that match lines with lines and arcs with arcs.
//!
//! # Checks
//! Sections must lie on distinct parallel planes in order along their common normal.
//! Each band's cross-section at a quarter, a half and three quarters of the way is the
//! linear interpolation of its two profiles, which is exact for these surfaces; each must
//! be a simple loop of the same orientation, or the loft would fold through itself.
//!
//! # Names
//! `start` and `end` (the caps at the first and last section), `band i side k` (the face
//! from section `i` to `i + 1` through edge `k`, numbered as in the first profile's
//! input), `section i k` (edge and vertex `k` of section `i`), `band i rail k` (the
//! straight edge from vertex `k` of section `i` to section `i + 1`).

#[cfg(test)]
mod tests;

use crate::profile::CheckedLoop;
use crate::{
    Arc3, Body, Circle3, Curve, KernelError, KernelResult, ListEdge, ListFace, ListVertex, Profile,
    ProfileEdge, Segment3, Surface,
};
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point3, Vec3};
use std::f64::consts::TAU;

/// Most sections one loft may join.
pub const MAX_LOFT_SECTIONS: usize = 1_000;

/// How far from parallel (radians, as the sine of the angle between normals) two section
/// planes may be. Planes made by offsetting a sketch plane are parallel to rounding,
/// about 1e-15; this leaves room for planes found from faces of a rebuilt solid.
pub const LOFT_PARALLEL_TOLERANCE: f64 = 1e-9;

fn invalid(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidInput(msg.into())
}

fn unsupported(msg: impl Into<String>) -> KernelError {
    KernelError::Unsupported(msg.into())
}

fn internal(msg: impl Into<String>) -> KernelError {
    KernelError::InvalidResult(msg.into())
}

/// One edge of a section in space, running counter-clockwise about the loft direction
/// as part of its loop.
#[derive(Debug, Clone, Copy)]
enum Edge3 {
    Line {
        a: Point3,
        b: Point3,
    },
    /// From `a`, `sweep` radians (positive, at most 2π) about `axis` (the loft direction
    /// or its opposite) around `centre`.
    Arc {
        centre: Point3,
        radius: f64,
        axis: Vec3,
        a: Point3,
        sweep: f64,
    },
}

impl Edge3 {
    fn start(&self) -> Point3 {
        match *self {
            Edge3::Line { a, .. } | Edge3::Arc { a, .. } => a,
        }
    }
    fn is_line(&self) -> bool {
        matches!(self, Edge3::Line { .. })
    }
    /// The curve and range, the edge running from `curve(t0)` to `curve(t1)`.
    fn curve(&self) -> KernelResult<(Curve, f64, f64)> {
        match *self {
            Edge3::Line { a, b } => Ok((Curve::Segment(Segment3::new(a, b)?), 0.0, 1.0)),
            Edge3::Arc {
                centre,
                radius,
                axis,
                a,
                sweep,
            } => {
                let frame = Frame::from_normal_and_x(centre, axis, a - centre)
                    .ok_or_else(|| invalid("an arc of a section has no plane"))?;
                if sweep >= TAU - chamfera_geom::ANGULAR_TOLERANCE {
                    Ok((Curve::Circle(Circle3::new(frame, radius)?), 0.0, TAU))
                } else {
                    Ok((
                        Curve::Arc(Arc3::new(frame, radius, 0.0, sweep)?),
                        0.0,
                        sweep,
                    ))
                }
            }
        }
    }
    /// The point a fraction `f` along, and the unit tangent there.
    fn at(&self, f: f64) -> (Point3, Vec3) {
        match *self {
            Edge3::Line { a, b } => (a + (b - a) * f, (b - a).normalized().unwrap_or(Vec3::X)),
            Edge3::Arc {
                centre,
                axis,
                a,
                sweep,
                ..
            } => {
                let p = chamfera_geom::Transform::rotation_about(centre, axis, sweep * f)
                    .map_or(a, |t| t.apply_point(a));
                let t = axis.cross(p - centre).normalized().unwrap_or(Vec3::X);
                (p, t)
            }
        }
    }
}

/// A section: its plane's height along the loft direction and its loop in space.
struct Section {
    edges: Vec<Edge3>,
    /// The input index of each edge (for names).
    source: Vec<usize>,
}

/// The loop of `l` (checked, counter-clockwise in `frame`) in space, counter-clockwise
/// about `n`.
fn section(frame: &Frame, l: &CheckedLoop, n: Vec3) -> Section {
    let mut edges: Vec<Edge3> = Vec::with_capacity(l.edges.len());
    let mut source = Vec::with_capacity(l.edges.len());
    for e in &l.edges {
        edges.push(match e.edge {
            ProfileEdge::Line { a, b } => Edge3::Line {
                a: frame.from_plane(a),
                b: frame.from_plane(b),
            },
            ProfileEdge::Arc {
                centre,
                radius,
                sweep,
                ..
            } => Edge3::Arc {
                centre: frame.from_plane(centre),
                radius,
                axis: frame.z() * sweep.signum(),
                a: frame.from_plane(e.edge.start()),
                sweep: sweep.abs(),
            },
        });
        source.push(e.source);
    }
    if frame.z().dot(n) < 0.0 {
        // Clockwise about `n`: run the loop backwards.
        let ends: Vec<Point3> = edges.iter().map(|e| e.at(1.0).0).collect();
        let mut rev: Vec<Edge3> = edges
            .iter()
            .zip(ends)
            .map(|(e, end)| match *e {
                Edge3::Line { a, b } => Edge3::Line { a: b, b: a },
                Edge3::Arc {
                    centre,
                    radius,
                    axis,
                    sweep,
                    ..
                } => Edge3::Arc {
                    centre,
                    radius,
                    axis: -axis,
                    a: end,
                    sweep,
                },
            })
            .collect();
        rev.reverse();
        let mut src = source;
        src.reverse();
        return Section {
            edges: rev,
            source: src,
        };
    }
    Section { edges, source }
}

/// The surface joining edge `a` of one section with the matching edge `b` of the next,
/// and whether its normal points out of the loft; `None` when no exact surface joins them.
fn ruled_surface(a: &Edge3, b: &Edge3, n: Vec3) -> KernelResult<Option<(Surface, bool)>> {
    let surface = match (*a, *b) {
        (Edge3::Line { a: a0, b: a1 }, Edge3::Line { a: b0, b: b1 }) => {
            let (Some(ta), Some(tb)) = ((a1 - a0).normalized(), (b1 - b0).normalized()) else {
                return Ok(None);
            };
            if ta.cross(tb).length() > LOFT_PARALLEL_TOLERANCE || ta.dot(tb) <= 0.0 {
                return Ok(None);
            }
            let normal = ta.cross(b0 - a0);
            let frame = Frame::from_normal_and_x(a0, normal, ta)
                .ok_or_else(|| internal("a loft side has no plane"))?;
            Surface::Plane(frame)
        }
        (
            Edge3::Arc {
                centre: ca,
                radius: ra,
                axis: xa,
                a: sa,
                sweep: wa,
            },
            Edge3::Arc {
                centre: cb,
                radius: rb,
                axis: xb,
                a: sb,
                sweep: wb,
            },
        ) => {
            let off = cb - ca;
            let h = off.dot(n);
            let lateral = off - n * h;
            let (Some(da), Some(db)) = ((sa - ca).normalized(), (sb - cb).normalized()) else {
                return Ok(None);
            };
            let same_start = (da - db).length() * ra.max(rb) <= LINEAR_TOLERANCE;
            let same_turn = xa.dot(xb) > 0.0 && (wa - wb).abs() * ra.max(rb) <= LINEAR_TOLERANCE;
            if lateral.length() > LINEAR_TOLERANCE || !same_start || !same_turn {
                return Ok(None);
            }
            let frame = Frame::from_normal_and_x(ca, n, da)
                .ok_or_else(|| internal("a loft side has no axis"))?;
            if (rb - ra).abs() <= LINEAR_TOLERANCE {
                Surface::cylinder(frame, ra)?
            } else {
                Surface::cone(frame, ra, (rb - ra).atan2(h))?
            }
        }
        _ => return Ok(None),
    };
    // Outward: the edge's tangent crossed with the ruling (loops run counter-clockwise
    // about the loft direction, rulings go along it).
    let (p, t) = a.at(0.5);
    let (q, _) = b.at(0.5);
    let out = t.cross(q - p);
    let uv = surface.parameters(p);
    let sense = surface.normal_at(uv.u, uv.v).dot(out) > 0.0;
    Ok(Some((surface, sense)))
}

/// `profiles` (each in its frame's plane coordinates) lofted in order. See the module
/// documentation for what is joined exactly, what is refused and the names.
pub fn loft(sections: &[(Frame, Profile)]) -> KernelResult<Body> {
    if sections.len() < 2 {
        return Err(invalid("a loft needs at least two profiles"));
    }
    if sections.len() > MAX_LOFT_SECTIONS {
        return Err(invalid(format!(
            "a loft may join at most {MAX_LOFT_SECTIONS} profiles"
        )));
    }
    let Some((f0, _)) = sections.first() else {
        return Err(invalid("a loft needs at least two profiles"));
    };
    let mut n = f0.z();
    for (f, _) in sections {
        if f.z().cross(n).length() > LOFT_PARALLEL_TOLERANCE {
            return Err(invalid(
                "the profiles of a loft must lie on parallel planes",
            ));
        }
    }
    // Heights along the common normal: strictly monotonic.
    let heights: Vec<f64> = sections
        .iter()
        .map(|(f, _)| (f.origin() - Point3::ORIGIN).dot(n))
        .collect();
    let rising = heights.windows(2).all(|w| w[1] - w[0] > LINEAR_TOLERANCE);
    let falling = heights.windows(2).all(|w| w[0] - w[1] > LINEAR_TOLERANCE);
    if falling {
        n = -n;
    } else if !rising {
        return Err(invalid(
            "the profiles of a loft must lie on distinct planes, in order along their normal",
        ));
    }
    let mut loops: Vec<Section> = Vec::with_capacity(sections.len());
    for (i, (f, p)) in sections.iter().enumerate() {
        let c = p.check()?;
        if !c.holes.is_empty() {
            return Err(unsupported(format!(
                "lofting a profile with holes (profile {i})"
            )));
        }
        loops.push(section(f, &c.outer, n));
    }
    let m = loops.first().map_or(0, |s| s.edges.len());
    if loops.iter().any(|s| s.edges.len() != m) {
        return Err(invalid(
            "every profile of a loft must have the same number of edges",
        ));
    }
    // Match each section's edges to the previous one's.
    for i in 1..loops.len() {
        let (prev, rest) = loops.split_at_mut(i);
        let (Some(a), Some(b)) = (prev.last(), rest.first_mut()) else {
            continue;
        };
        let shift = best_shift(a, b, n).ok_or_else(|| {
            unsupported(
                "lofting profiles whose edges do not correspond (lines must meet lines and arcs meet arcs)",
            )
        })?;
        b.edges.rotate_left(shift);
        b.source.rotate_left(shift);
    }
    check_bands(&loops, n)?;
    build(&loops, n)
}

/// The cyclic shift of `b` that matches `a` edge kind for edge kind with the smallest sum
/// of squared in-plane distances between matched vertices.
fn best_shift(a: &Section, b: &Section, n: Vec3) -> Option<usize> {
    let m = a.edges.len();
    let flat = |p: Point3| {
        let v = p - Point3::ORIGIN;
        v - n * v.dot(n)
    };
    let mut best: Option<(f64, usize)> = None;
    for s in 0..m {
        let mut cost = 0.0;
        let mut ok = true;
        for k in 0..m {
            let (Some(ea), Some(eb)) = (a.edges.get(k), b.edges.get((k + s) % m)) else {
                ok = false;
                break;
            };
            if ea.is_line() != eb.is_line() {
                ok = false;
                break;
            }
            let d = flat(ea.start()) - flat(eb.start());
            cost += d.dot(d);
        }
        if ok && best.is_none_or(|(c, _)| cost < c) {
            best = Some((cost, s));
        }
    }
    best.map(|(_, s)| s)
}

/// Each band's interpolated cross-sections must be simple loops of the same orientation.
fn check_bands(loops: &[Section], n: Vec3) -> KernelResult<()> {
    let frame =
        Frame::from_normal(Point3::ORIGIN, n).ok_or_else(|| invalid("no loft direction"))?;
    for w in loops.windows(2) {
        let [a, b] = w else { continue };
        for f in [0.25, 0.5, 0.75] {
            let mut edges = Vec::with_capacity(a.edges.len());
            for (ea, eb) in a.edges.iter().zip(&b.edges) {
                let pa = frame.to_plane(ea.start());
                let pb = frame.to_plane(eb.start());
                let qa = frame.to_plane(ea.at(1.0).0);
                let qb = frame.to_plane(eb.at(1.0).0);
                let s = pa.lerp(pb, f);
                let e = qa.lerp(qb, f);
                edges.push(match (*ea, *eb) {
                    (
                        Edge3::Arc {
                            centre: ca,
                            axis,
                            sweep,
                            ..
                        },
                        Edge3::Arc { centre: cb, .. },
                    ) => {
                        let c = frame.to_plane(ca).lerp(frame.to_plane(cb), f);
                        if sweep >= TAU - chamfera_geom::ANGULAR_TOLERANCE {
                            ProfileEdge::circle(c, s.distance(c))
                        } else {
                            let start = (s - c).angle();
                            let sign = if axis.dot(n) > 0.0 { 1.0 } else { -1.0 };
                            ProfileEdge::Arc {
                                centre: c,
                                radius: s.distance(c),
                                start_angle: start,
                                sweep: sweep * sign,
                            }
                        }
                    }
                    _ => ProfileEdge::line(s, e),
                });
            }
            // Still counter-clockwise (positive area) and simple.
            let area: f64 = edges.iter().map(ProfileEdge::area_term).sum();
            let ok = area > 0.0 && Profile::new(edges, Vec::new()).check().is_ok();
            if !ok {
                return Err(invalid(
                    "the loft would twist through itself between two profiles",
                ));
            }
        }
    }
    Ok(())
}

/// The face list of the loft.
fn build(loops: &[Section], n: Vec3) -> KernelResult<Body> {
    let m = loops.first().map_or(0, |s| s.edges.len());
    let names0: Vec<usize> = loops.first().map(|s| s.source.clone()).unwrap_or_default();
    let name_k = |k: usize| names0.get(k).copied().unwrap_or(k);
    let mut vertices = Vec::new();
    let mut edges: Vec<ListEdge> = Vec::new();
    let mut faces = Vec::new();
    // Vertex k of section i is i * m + k; its section edge has the same index.
    for (i, s) in loops.iter().enumerate() {
        for (k, e) in s.edges.iter().enumerate() {
            vertices.push(ListVertex {
                point: e.start(),
                name: Some(format!("section {i} {}", name_k(k))),
            });
        }
    }
    for (i, s) in loops.iter().enumerate() {
        for (k, e) in s.edges.iter().enumerate() {
            let (curve, t0, t1) = e.curve()?;
            edges.push(ListEdge {
                vertices: [i * m + k, i * m + (k + 1) % m],
                curve,
                t0,
                t1,
                name: Some(format!("section {i} {}", name_k(k))),
            });
        }
    }
    // Rail k of band i: edge loops.len() * m + i * m + k.
    let rail0 = loops.len() * m;
    for i in 0..loops.len().saturating_sub(1) {
        for k in 0..m {
            let (a, b) = (
                vertices.get(i * m + k).map(|v: &ListVertex| v.point),
                vertices.get((i + 1) * m + k).map(|v: &ListVertex| v.point),
            );
            let (Some(a), Some(b)) = (a, b) else {
                return Err(internal("a loft vertex is missing"));
            };
            edges.push(ListEdge {
                vertices: [i * m + k, (i + 1) * m + k],
                curve: Curve::Segment(Segment3::new(a, b)?),
                t0: 0.0,
                t1: 1.0,
                name: Some(format!("band {i} rail {}", name_k(k))),
            });
        }
    }
    let (Some(first), Some(last)) = (loops.first(), loops.last()) else {
        return Err(invalid("a loft needs at least two profiles"));
    };
    let p0 = first.edges.first().map_or(Point3::ORIGIN, Edge3::start);
    let p1 = last.edges.first().map_or(Point3::ORIGIN, Edge3::start);
    let cap = |o: Point3, normal: Vec3| {
        Frame::from_normal(o, normal)
            .map(Surface::Plane)
            .ok_or_else(|| internal("a loft cap has no plane"))
    };
    faces.push(ListFace {
        surface: cap(p0, -n)?,
        same_sense: true,
        name: Some("start".into()),
        loops: vec![(0..m).rev().map(|k| (k, false)).collect()],
    });
    let li = loops.len() - 1;
    faces.push(ListFace {
        surface: cap(p1, n)?,
        same_sense: true,
        name: Some("end".into()),
        loops: vec![(0..m).map(|k| (li * m + k, true)).collect()],
    });
    for (i, w) in loops.windows(2).enumerate() {
        let [a, b] = w else { continue };
        for k in 0..m {
            let (Some(ea), Some(eb)) = (a.edges.get(k), b.edges.get(k)) else {
                return Err(internal("a loft edge is missing"));
            };
            let (surface, same_sense) = ruled_surface(ea, eb, n)?.ok_or_else(|| {
                unsupported(format!(
                    "lofting edge {} between profiles {i} and {}: only parallel lines (planes) and coaxial arcs over the same angles (cones) are exact so far",
                    name_k(k),
                    i + 1
                ))
            })?;
            let r = |kk: usize| rail0 + i * m + kk % m;
            faces.push(ListFace {
                surface,
                same_sense,
                name: Some(format!("band {i} side {}", name_k(k))),
                loops: vec![vec![
                    (i * m + k, true),
                    (r(k + 1), true),
                    ((i + 1) * m + k, false),
                    (r(k), false),
                ]],
            });
        }
    }
    let body = Body::from_face_list(&vertices, &edges, &faces)?;
    body.validate()?;
    Ok(body)
}

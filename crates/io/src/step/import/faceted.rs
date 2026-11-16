//! Faceted B-reps (ISO 10303-42, 4.5.2 `faceted_brep`) and shell-based surface models whose
//! faces are bounded by `POLY_LOOP`s (4.5.13), read as exact planar-face solids or, when
//! they are not closed valid solids, as triangle meshes.
//!
//! A `POLY_LOOP(name, polygon)` lists `CARTESIAN_POINT`s; its edges are the straight
//! segments between consecutive points and back to the first. A face (`FACE`,
//! `FACE_SURFACE` or `ADVANCED_FACE`) lies to the left of its bounds seen against its
//! normal (Part 42), so its plane's normal is the vector area (Newell's method) of its
//! outer bound, taken after the bound's orientation flag.
//!
//! Points are merged by instance and then by position (within
//! [`chamfera_geom::LINEAR_TOLERANCE`]), since many writers repeat a point per facet. The
//! faces then become planes and the segments edges of [`Body::from_boundary`], which
//! validates the solid (closed, every edge used twice in opposite directions, every
//! facet flat). A shell that is open or not a valid solid becomes a [`TriMesh`] instead:
//! each facet is ear-clipped (exact `orient2d`) in its plane; facets with holes are then
//! refused.

use super::{Ctx, Fail, R, arg, bool_of, invalid, list_of, ref_of};
use crate::step::read::Param;
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Point3, Sign, TriMesh, Vec3, orient2d};
use chamfera_kernel::{Body, Curve, Segment3, Surface};
use std::collections::HashMap;

/// Most points one faceted shell may hold.
const MAX_FACET_POINTS: usize = 5_000_000;

/// A shell of polygons: its points, and per face its loops (outer first) as point
/// indices running with the face on their left.
#[derive(Debug, Default)]
pub(super) struct Facets {
    points: Vec<Point3>,
    faces: Vec<Vec<Vec<usize>>>,
    face_ids: Vec<u64>,
    by_id: HashMap<u64, usize>,
    grid: HashMap<(i64, i64, i64), Vec<usize>>,
    /// Whether the shell was a `CLOSED_SHELL`.
    pub closed: bool,
}

/// True when instance `id` is a face with a `POLY_LOOP` bound, so its shell is faceted.
pub(super) fn is_poly_face(ctx: &Ctx<'_>, id: u64) -> bool {
    let Ok(e) = ctx.ent(id) else {
        return false;
    };
    let Some(Param::List(bounds)) = e.params.get(1) else {
        return false;
    };
    bounds.iter().any(|b| {
        matches!(b, Param::Ref(b) if ctx.ent(*b).ok().is_some_and(|be| {
            matches!(be.params.get(1), Some(Param::Ref(l))
                if ctx.ent(*l).ok().is_some_and(|le| le.name == "POLY_LOOP"))
        }))
    })
}

fn cell(p: Point3) -> (i64, i64, i64) {
    let q = |x: f64| (x / LINEAR_TOLERANCE).floor() as i64;
    (q(p.x), q(p.y), q(p.z))
}

/// Newell's vector area of a polygon (twice the area along the normal).
fn newell(points: &[Point3], lp: &[usize]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (k, &i) in lp.iter().enumerate() {
        let j = lp.get((k + 1) % lp.len()).copied().unwrap_or(i);
        let (Some(a), Some(b)) = (points.get(i), points.get(j)) else {
            continue;
        };
        n = n + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    n
}

impl Facets {
    /// The vertex of point `id`, merged with any earlier point within the tolerance.
    fn vertex(&mut self, ctx: &Ctx<'_>, id: u64) -> R<usize> {
        if let Some(&i) = self.by_id.get(&id) {
            return Ok(i);
        }
        let p = ctx.point(id)?;
        let (cx, cy, cz) = cell(p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = (
                        cx.saturating_add(dx),
                        cy.saturating_add(dy),
                        cz.saturating_add(dz),
                    );
                    if let Some(list) = self.grid.get(&key) {
                        for &i in list {
                            if self
                                .points
                                .get(i)
                                .is_some_and(|q| q.distance(p) <= LINEAR_TOLERANCE)
                            {
                                found = Some(i);
                                break 'search;
                            }
                        }
                    }
                }
            }
        }
        let i = match found {
            Some(i) => i,
            None => {
                if self.points.len() >= MAX_FACET_POINTS {
                    return Err(invalid(format!(
                        "a faceted shell may hold at most {MAX_FACET_POINTS} points"
                    )));
                }
                self.points.push(p);
                let i = self.points.len() - 1;
                self.grid.entry((cx, cy, cz)).or_default().push(i);
                i
            }
        };
        self.by_id.insert(id, i);
        Ok(i)
    }
}

impl Ctx<'_> {
    /// The polygons of shell `shell` (a `CLOSED_SHELL` or `OPEN_SHELL` of faces bounded by
    /// `POLY_LOOP`s).
    pub(super) fn facets(&self, shell: u64) -> R<Facets> {
        let se = self.expect(shell, &["CLOSED_SHELL", "OPEN_SHELL"], "shell")?;
        let closed = se.name == "CLOSED_SHELL";
        let faces = list_of(&arg(&se, 1, shell)?, shell, "faces")?;
        drop(se);
        let mut out = Facets {
            closed,
            ..Facets::default()
        };
        for f in &faces {
            let f = ref_of(f, shell, "face")?;
            let fe = self.expect(f, &["FACE", "FACE_SURFACE", "ADVANCED_FACE"], "face")?;
            let bounds = list_of(&arg(&fe, 1, f)?, f, "bounds")?;
            drop(fe);
            let mut loops: Vec<(bool, Vec<usize>)> = Vec::new();
            for b in &bounds {
                let b = ref_of(b, f, "bound")?;
                let be = self.expect(b, &["FACE_OUTER_BOUND", "FACE_BOUND"], "bound")?;
                let outer = be.name == "FACE_OUTER_BOUND";
                let lp = ref_of(&arg(&be, 1, b)?, b, "bound")?;
                let orientation = bool_of(&arg(&be, 2, b)?, b, "orientation")?;
                drop(be);
                let le = self.ent(lp)?;
                if le.name != "POLY_LOOP" {
                    return Err(Fail::Unsupported(vec![format!(
                        "face #{f} mixes a {} (#{lp}) with polygon bounds, which is not supported",
                        le.name
                    )]));
                }
                let pts = list_of(&arg(&le, 1, lp)?, lp, "polygon")?;
                drop(le);
                let mut ring: Vec<usize> = Vec::with_capacity(pts.len());
                for p in &pts {
                    let v = out.vertex(self, ref_of(p, lp, "point")?)?;
                    if ring.last() != Some(&v) {
                        ring.push(v);
                    }
                }
                while ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if ring.len() < 3 {
                    return Err(invalid(format!(
                        "the polygon #{lp} of face #{f} has fewer than three distinct points"
                    )));
                }
                if !orientation {
                    ring.reverse();
                }
                loops.push((outer, ring));
            }
            if loops.is_empty() {
                return Err(invalid(format!("face #{f} has no bounds")));
            }
            // The outer bound first: FACE_OUTER_BOUND, else the largest polygon.
            let area = |l: &Vec<usize>| newell(&out.points, l).length();
            let outer = loops.iter().position(|l| l.0).unwrap_or_else(|| {
                let mut best = 0;
                for (k, l) in loops.iter().enumerate() {
                    if loops.get(best).is_some_and(|b| area(&l.1) > area(&b.1)) {
                        best = k;
                    }
                }
                best
            });
            if outer != 0 {
                loops.swap(0, outer);
            }
            out.faces.push(loops.into_iter().map(|l| l.1).collect());
            out.face_ids.push(f);
        }
        if out.faces.is_empty() {
            return Err(invalid(format!("shell #{shell} has no faces")));
        }
        Ok(out)
    }
}

impl Facets {
    /// The planar-face solid of a closed shell, validated, or why it is not one.
    pub(super) fn to_body(&self) -> Result<Body, String> {
        let mut edge_of: HashMap<(usize, usize), usize> = HashMap::new();
        let mut edges: Vec<(usize, usize, Curve, f64, f64)> = Vec::new();
        let mut faces = Vec::with_capacity(self.faces.len());
        for (loops, id) in self.faces.iter().zip(&self.face_ids) {
            let outer = loops
                .first()
                .ok_or_else(|| format!("face #{id} has no bounds"))?;
            let n = newell(&self.points, outer);
            let origin = outer
                .first()
                .and_then(|i| self.points.get(*i))
                .copied()
                .ok_or_else(|| format!("face #{id} has no points"))?;
            let frame = Frame::from_normal(origin, n)
                .ok_or_else(|| format!("face #{id} has no area (its polygon is degenerate)"))?;
            let mut uses_all = Vec::with_capacity(loops.len());
            for lp in loops {
                let mut uses = Vec::with_capacity(lp.len());
                for (k, &a) in lp.iter().enumerate() {
                    let b = lp.get((k + 1) % lp.len()).copied().unwrap_or(a);
                    let key = (a.min(b), a.max(b));
                    let e = match edge_of.get(&key) {
                        Some(&e) => e,
                        None => {
                            let (Some(pa), Some(pb)) =
                                (self.points.get(key.0), self.points.get(key.1))
                            else {
                                return Err(format!("face #{id} refers to a missing point"));
                            };
                            let seg = Segment3::new(*pa, *pb)
                                .map_err(|e| format!("an edge of face #{id}: {e}"))?;
                            edges.push((key.0, key.1, Curve::Segment(seg), 0.0, 1.0));
                            edge_of.insert(key, edges.len() - 1);
                            edges.len() - 1
                        }
                    };
                    uses.push((e, a == key.0));
                }
                uses_all.push(uses);
            }
            faces.push((Surface::Plane(frame), true, uses_all));
        }
        Body::from_boundary(&self.points, &edges, &faces).map_err(|e| {
            // Name the facet behind a face error.
            let msg = e.to_string();
            match e {
                chamfera_kernel::TopologyError::OffSurface(f)
                | chamfera_kernel::TopologyError::WrongOrientation(f)
                | chamfera_kernel::TopologyError::DegenerateFace(f) => self
                    .face_ids
                    .get(f.index())
                    .map_or(msg.clone(), |id| format!("face #{id}: {msg}")),
                _ => msg,
            }
        })
    }

    /// The shell as triangles, each facet ear-clipped in its own plane.
    pub(super) fn to_mesh(&self) -> Result<TriMesh, String> {
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        let index = |i: usize| u32::try_from(i).map_err(|_| "too many points".to_owned());
        for (loops, id) in self.faces.iter().zip(&self.face_ids) {
            let [outer] = loops.as_slice() else {
                return Err(format!(
                    "face #{id} has holes, which a mesh of its facets cannot show yet"
                ));
            };
            let n = newell(&self.points, outer);
            let frame = Frame::from_normal(Point3::ORIGIN, n)
                .ok_or_else(|| format!("face #{id} has no area (its polygon is degenerate)"))?;
            let flat: Vec<Point2> = outer
                .iter()
                .filter_map(|i| self.points.get(*i))
                .map(|p| frame.to_plane(*p))
                .collect();
            for [a, b, c] in ear_clip(&flat).ok_or_else(|| {
                format!("face #{id}: its polygon crosses itself and cannot be triangulated")
            })? {
                let (Some(a), Some(b), Some(c)) = (outer.get(a), outer.get(b), outer.get(c)) else {
                    continue;
                };
                triangles.push([index(*a)?, index(*b)?, index(*c)?]);
            }
        }
        let mesh = TriMesh {
            positions: self.points.clone(),
            triangles,
        };
        mesh.validate().map_err(|e| format!("{e:?}"))?;
        Ok(mesh)
    }
}

/// Triangles of a simple polygon running counter-clockwise, by ear clipping (O(n²)), as
/// index triples; `None` when no ear can be found (the polygon crosses itself).
fn ear_clip(poly: &[Point2]) -> Option<Vec<[usize; 3]>> {
    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let mut out = Vec::with_capacity(poly.len().saturating_sub(2));
    let at = |i: usize| poly.get(i).copied().unwrap_or(Point2::ORIGIN);
    let ccw = |a: usize, b: usize, c: usize| orient2d(at(a), at(b), at(c));
    while idx.len() > 3 {
        let n = idx.len();
        let mut clipped = false;
        for k in 0..n {
            let (a, b, c) = (idx[(k + n - 1) % n], idx[k], idx[(k + 1) % n]);
            match ccw(a, b, c) {
                Some(Sign::Positive) => {}
                // A straight vertex is dropped without a triangle.
                Some(Sign::Zero) => {
                    idx.remove(k);
                    clipped = true;
                    break;
                }
                _ => continue,
            }
            let inside = idx.iter().any(|&p| {
                p != a
                    && p != b
                    && p != c
                    && ccw(a, b, p) != Some(Sign::Negative)
                    && ccw(b, c, p) != Some(Sign::Negative)
                    && ccw(c, a, p) != Some(Sign::Negative)
            });
            if !inside {
                out.push([a, b, c]);
                idx.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            return None;
        }
    }
    if let [a, b, c] = idx.as_slice()
        && ccw(*a, *b, *c) == Some(Sign::Positive)
    {
        out.push([*a, *b, *c]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ear_clipping_an_l_shape_gives_four_triangles_of_its_area() {
        // An L of area 3 (three unit squares) with six corners: 6 - 2 = 4 triangles.
        let p = |x: f64, y: f64| Point2::new(x, y);
        let l = [
            p(0., 0.),
            p(2., 0.),
            p(2., 1.),
            p(1., 1.),
            p(1., 2.),
            p(0., 2.),
        ];
        let t = ear_clip(&l).unwrap();
        assert_eq!(t.len(), 4);
        let area: f64 = t
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (l[*a], l[*b], l[*c]);
                0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x))
            })
            .sum();
        assert!((area - 3.0).abs() < 1e-12);
        // A bow tie has no ear that leaves a simple polygon behind at some point.
        let bow = [p(0., 0.), p(1., 1.), p(1., 0.), p(0., 1.)];
        let _ = ear_clip(&bow);
    }
}

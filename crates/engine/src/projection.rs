//! Projecting model edges into a sketch (`sketch.project`) and keeping them linked.
//!
//! An edge's curve is projected straight onto the sketch plane (along its normal), exactly:
//! - a segment stays a segment (its ends projected);
//! - a circle, arc, ellipse or elliptical arc `p(t) = o + rx cos t X + ry sin t Y` projects
//!   to `c + cos t U + sin t V` with `c`, `U = rx X`, `V = ry Y` projected. That is an
//!   ellipse with conjugate semi-diameters `U` and `V`. `|p(t) - c|²` is
//!   `(|U|² + |V|²)/2 + (|U|² - |V|²)/2 cos 2t + (U·V) sin 2t`, largest at
//!   `2t* = atan2(2 U·V, |U|² - |V|²)`: the major semi-axis is `p(t*) - c`, the minor one
//!   `p(t* + π/2) - c` (they are square to each other). Equal axes give a circle; a
//!   zero minor axis (the curve seen edge-on) a segment between the extremes along the
//!   major axis. Ends are the projected ends; when the projection mirrors the curve
//!   (`U × V < 0`) its counter-clockwise direction turns clockwise, so the ends swap.
//!
//! The curve is added fixed in place (`Sketch::add_fixed_curve`) and remembered with the
//! edge's reference name (`edge_ref`). When the design rebuilds, every linked projection
//! is moved onto its edge as built so far and the sketch is solved, so geometry
//! constrained to it follows. An edge that is not there (yet) leaves the curve where it
//! was.

use crate::Session;
use crate::solids::edge_ref;
use chamfera_doc::Projection;
use chamfera_geom::{Frame, LINEAR_TOLERANCE, Point2, Vec2, Vec3};
use chamfera_kernel::{Body, Curve, EdgeId};
use chamfera_sketch::{CurveId, FixedCurve};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A vector in the plane's coordinates (its part along the normal dropped).
fn in_plane(plane: &Frame, v: Vec3) -> Vec2 {
    let l = plane.vector_to_local(v);
    Vec2::new(l.x, l.y)
}

/// The edge's curve projected onto `plane`, in plane coordinates.
pub fn project_edge(body: &Body, e: EdgeId, plane: &Frame) -> Result<FixedCurve, String> {
    let (curve, t0, t1) = body
        .edge_curve(e)
        .map_err(|x| x.to_string())?
        .ok_or("the edge has no geometry")?;
    let (lo, hi) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
    let at = |t: f64| plane.to_plane(curve.point_at(t));
    let conic = match &curve {
        Curve::Line(_) | Curve::Segment(_) => None,
        Curve::Circle(c) => Some((*c.frame(), c.radius(), c.radius())),
        Curve::Arc(a) => Some((
            *a.circle().frame(),
            a.circle().radius(),
            a.circle().radius(),
        )),
        Curve::Ellipse(x) => Some((*x.frame(), x.rx(), x.ry())),
        Curve::EllipticalArc(a) => Some((*a.ellipse().frame(), a.ellipse().rx(), a.ellipse().ry())),
    };
    let Some((frame, rx, ry)) = conic else {
        let (a, b) = (at(lo), at(hi));
        if a.coincident(b) {
            return Err(
                "the edge points straight at the sketch plane: it projects to a point".into(),
            );
        }
        return Ok(FixedCurve::Line { a, b });
    };
    let full = hi - lo >= TAU - chamfera_geom::ANGULAR_TOLERANCE;
    let c = plane.to_plane(frame.origin());
    let u = in_plane(plane, frame.x() * rx);
    let v = in_plane(plane, frame.y() * ry);
    project_conic(c, u, v, (lo, hi), full)
}

/// The projection of `c + cos t U + sin t V` for `t` in `range` (the whole curve when
/// `full`), as sketch geometry (see the module notes).
fn project_conic(
    c: Point2,
    u: Vec2,
    v: Vec2,
    range: (f64, f64),
    full: bool,
) -> Result<FixedCurve, String> {
    let (lo, hi) = range;
    let p = |t: f64| u * t.cos() + v * t.sin();
    let at = |t: f64| c + p(t);
    let ts = 0.5 * (2.0 * u.dot(v)).atan2(u.dot(u) - v.dot(v));
    let (major, minor) = (p(ts), p(ts + FRAC_PI_2));
    let (a, b) = (major.length(), minor.length());
    if !(a.is_finite() && a > LINEAR_TOLERANCE) {
        return Err("the edge projects to a point".into());
    }
    let axis = major * (1.0 / a);
    if b <= LINEAR_TOLERANCE {
        // Seen edge-on: the segment between the extremes of `axis · p(t)` over the range,
        // at the ends or where `t = ts + kπ`.
        let mut ext: Vec<f64> = vec![lo, hi];
        let k0 = ((lo - ts) / PI).ceil();
        let mut k = k0;
        while ts + k * PI <= hi && k - k0 < 4.0 {
            ext.push(ts + k * PI);
            k += 1.0;
        }
        let along = |t: f64| axis.dot(p(t));
        let min = ext
            .iter()
            .copied()
            .fold(f64::INFINITY, |m, t| m.min(along(t)));
        let max = ext
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, |m, t| m.max(along(t)));
        let (a, b) = (c + axis * min, c + axis * max);
        if a.coincident(b) {
            return Err("the edge projects to a point".into());
        }
        return Ok(FixedCurve::Line { a, b });
    }
    // Counter-clockwise in the sketch: swap the ends when the projection mirrors.
    let (s, e) = if u.cross(v) >= 0.0 {
        (at(lo), at(hi))
    } else {
        (at(hi), at(lo))
    };
    if (a - b).abs() <= LINEAR_TOLERANCE {
        let r = 0.5 * (a + b);
        return Ok(if full {
            FixedCurve::Circle { c, r }
        } else {
            FixedCurve::Arc { c, s, e }
        });
    }
    let (m, n) = (c + axis * a, c + axis.perp() * b);
    Ok(if full {
        FixedCurve::Ellipse {
            c,
            major: m,
            minor: n,
        }
    } else {
        FixedCurve::EllipticalArc {
            c,
            major: m,
            minor: n,
            s,
            e,
        }
    })
}

/// The edge named `name` among `bodies`.
fn find_edge<'a>(bodies: impl Iterator<Item = &'a Body>, name: &str) -> Option<(&'a Body, EdgeId)> {
    for body in bodies {
        for e in body.edges() {
            if edge_ref(body, e).as_deref() == Some(name) {
                return Some((body, e));
            }
        }
    }
    None
}

impl Session {
    /// The reference names of the edges bounding the face `face` of `feature` (outer loop
    /// first), in the solids built now.
    pub fn face_edge_refs(&self, feature: u32, face: &str) -> Result<Vec<String>, String> {
        let key = crate::solids::face_key(feature, face);
        for s in self.solids() {
            for f in s.body.faces() {
                if s.body.face_name(f).ok().flatten() == Some(key.as_str()) {
                    let edges = s.body.face_edges(f).map_err(|e| e.to_string())?;
                    return Ok(edges
                        .into_iter()
                        .filter_map(|e| edge_ref(&s.body, e))
                        .collect());
                }
            }
        }
        Err(format!("feature {feature} has no face named {face:?}"))
    }

    /// Projects the named edges of the built solids into sketch `sketch`: each becomes a
    /// fixed curve, linked to its edge if `linked`. Returns (edge, curve) pairs. Edges
    /// already projected into this sketch are skipped.
    pub fn project_edges(
        &mut self,
        sketch: u32,
        edges: &[String],
        linked: bool,
    ) -> Result<Vec<(String, u32)>, String> {
        let plane = self.doc.sketch(sketch).map_err(|e| e.to_string())?.plane;
        let mut shapes = Vec::new();
        for name in edges {
            let (body, e) = find_edge(self.solids().iter().map(|s| &s.body), name)
                .ok_or_else(|| format!("there is no edge named {name:?}"))?;
            let g = project_edge(body, e, &plane).map_err(|x| format!("{name}: {x}"))?;
            shapes.push((name.clone(), g));
        }
        let f = self.doc.sketch_mut(sketch).map_err(|e| e.to_string())?;
        if f.projections.len() + shapes.len() > chamfera_doc::MAX_PROJECTIONS {
            return Err("the sketch holds too many projected edges".into());
        }
        let mut out = Vec::new();
        for (name, g) in shapes {
            if f.projections.iter().any(|p| p.edge == name) {
                continue;
            }
            let id = f.sketch.add_fixed_curve(&g).map_err(|e| e.to_string())?;
            f.projections.push(Projection {
                edge: name.clone(),
                curve: id.0,
                linked,
            });
            out.push((name, id.0));
        }
        f.sketch.solve().map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Moves the linked projections of `sketch` onto their edges in `bodies` (the solids
    /// built so far in a rebuild) and solves. Edges not found leave their curves alone.
    /// On failure the sketch is unchanged.
    pub(crate) fn follow_projections<'a>(
        &mut self,
        sketch: u32,
        bodies: impl Iterator<Item = &'a Body> + Clone,
    ) -> Result<(), String> {
        let Ok(f) = self.doc.sketch(sketch) else {
            return Ok(());
        };
        if f.projections.iter().all(|p| !p.linked) {
            return Ok(());
        }
        let plane = f.plane;
        let mut next = f.sketch.clone();
        let mut links = f.projections.clone();
        let mut moved = false;
        for link in links.iter_mut().filter(|p| p.linked) {
            let Some((body, e)) = find_edge(bodies.clone(), &link.edge) else {
                continue;
            };
            let Ok(g) = project_edge(body, e, &plane) else {
                continue;
            };
            let id = CurveId(link.curve);
            if next.move_fixed_curve(id, &g).is_err() {
                // The edge projects to another kind of curve now (a circle seen at a
                // slant is an ellipse): replace the curve.
                let _ = next.remove_curve(id);
                let new = next
                    .add_fixed_curve(&g)
                    .map_err(|x| format!("{}: {x}", link.edge))?;
                link.curve = new.0;
            }
            moved = true;
        }
        if !moved {
            return Ok(());
        }
        next.solve()
            .map_err(|e| format!("the sketch cannot follow its projected edges: {e}"))?;
        if let Ok(f) = self.doc.sketch_mut(sketch) {
            f.sketch = next;
            f.projections = links;
            let sketch = &f.sketch;
            f.expressions.retain(|c, _| {
                sketch
                    .constraint(chamfera_sketch::ConstraintId(*c))
                    .is_some()
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Point3;
    use chamfera_sketch::Sketch;

    /// A circle of radius `r` in `frame`, over `range`, seen in the XY plane.
    fn seen_from_above(frame: &Frame, r: f64, range: (f64, f64), full: bool) -> FixedCurve {
        let plane = Frame::XY;
        project_conic(
            plane.to_plane(frame.origin()),
            in_plane(&plane, frame.x() * r),
            in_plane(&plane, frame.y() * r),
            range,
            full,
        )
        .unwrap()
    }

    /// A frame at (1, 2, 3) whose normal is turned `angle` from +Z about X, x along +X.
    fn tilted(angle: f64) -> Frame {
        let n = Vec3::new(0.0, -angle.sin(), angle.cos());
        Frame::from_normal_and_x(Point3::new(1.0, 2.0, 3.0), n, Vec3::X).unwrap()
    }

    #[test]
    fn a_circle_tilted_60_degrees_projects_to_an_ellipse_10_by_5() {
        // Seen from above, a circle of radius 10 tilted 60° keeps its width along X and
        // is foreshortened to 10 cos 60° = 5 across: area 50π.
        let g = seen_from_above(&tilted(PI / 3.0), 10.0, (0.0, TAU), true);
        let FixedCurve::Ellipse { c, major, minor } = g else {
            panic!("{g:?}");
        };
        assert!(c.distance(Point2::new(1.0, 2.0)) < 1e-12);
        assert!((major.distance(c) - 10.0).abs() < 1e-12);
        assert!((minor.distance(c) - 5.0).abs() < 1e-12);
        let mut s = Sketch::new();
        s.add_fixed_curve(&g).unwrap();
        s.solve().unwrap();
        assert!((s.profiles()[0].area - 50.0 * PI).abs() < 1e-9);
    }

    #[test]
    fn circles_project_round_mirrored_or_edge_on() {
        // Parallel to the plane: the same circle.
        let g = seen_from_above(&tilted(0.0), 4.0, (0.0, TAU), true);
        assert!(matches!(g, FixedCurve::Circle { r, .. } if (r - 4.0).abs() < 1e-12));
        // Upside down (normal -Z) the frame's x stays +X and its y turns to -Y, so the
        // half from t = 0 at (5, 2) to t = π at (-3, 2) runs over the bottom; in the
        // sketch, counter-clockwise, that arc starts at (-3, 2).
        let g = seen_from_above(&tilted(PI), 4.0, (0.0, PI), false);
        let FixedCurve::Arc { c, s, e } = g else {
            panic!("{g:?}");
        };
        assert!(c.distance(Point2::new(1.0, 2.0)) < 1e-9);
        assert!(s.distance(Point2::new(-3.0, 2.0)) < 1e-9, "{s:?}");
        assert!(e.distance(Point2::new(5.0, 2.0)) < 1e-9, "{e:?}");
        // Edge-on (normal along -Y): the diameter along X, 20 long, at y = 2.
        let g = seen_from_above(&tilted(FRAC_PI_2), 10.0, (0.0, TAU), true);
        let FixedCurve::Line { a, b } = g else {
            panic!("{g:?}");
        };
        assert!((a.distance(b) - 20.0).abs() < 1e-9);
        assert!((a.y - 2.0).abs() < 1e-9 && (b.y - 2.0).abs() < 1e-9);
    }
}

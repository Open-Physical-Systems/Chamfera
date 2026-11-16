//! Intersection curves (`sketch.intersect`): where the sketch plane cuts a body, as sketch
//! geometry that follows the body when the design rebuilds.
//!
//! The body is cut by its own plane with the kernel's split (`chamfera_kernel::split_body`,
//! two exact booleans with a box standing on the plane); the faces the cut makes lie in
//! the plane, and their edges are exactly where the plane meets the body's faces: lines
//! where it meets planes, circles and arcs where it meets cylinders square to it or
//! spheres, ellipses where it meets slanted cylinders. Each edge comes into the sketch
//! through the projection of `projection.rs` (a curve already in the plane projects to
//! itself), fixed in place. Linked, the curves move with the body when it changes; when
//! the cut gives a different set of curves they are replaced.

use crate::Session;
use crate::projection::project_edge;
use chamfera_doc::Intersection;
use chamfera_geom::{Frame, Point3};
use chamfera_kernel::Body;
use chamfera_sketch::{CurveId, FixedCurve};

/// The name the kernel gives the faces of the cut (never seen outside this module).
const CUT_FACE: &str = "sketch intersection";

/// The curves where `plane` cuts `body`, in plane coordinates.
pub fn section_curves(body: &Body, plane: &Frame) -> Result<Vec<FixedCurve>, String> {
    let bb = body.bounding_box().ok_or("the body is empty")?;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..8 {
        let p = Point3::new(
            if i & 1 == 0 { bb.min.x } else { bb.max.x },
            if i & 2 == 0 { bb.min.y } else { bb.max.y },
            if i & 4 == 0 { bb.min.z } else { bb.max.z },
        );
        let d = (p - plane.origin()).dot(plane.z());
        lo = lo.min(d);
        hi = hi.max(d);
    }
    let tol = chamfera_geom::LINEAR_TOLERANCE;
    if !(lo < -tol && hi > tol) {
        return Err("the sketch plane does not cut it".into());
    }
    let (_, front) = chamfera_kernel::split_body(body, plane, CUT_FACE)
        .map_err(|e| format!("the sketch plane does not cut it cleanly: {e}"))?;
    let mut out = Vec::new();
    for f in front.faces() {
        if front.face_name(f).ok().flatten() != Some(CUT_FACE) {
            continue;
        }
        for e in front.face_edges(f).map_err(|e| e.to_string())? {
            out.push(project_edge(&front, e, plane)?);
        }
    }
    if out.is_empty() {
        return Err("the sketch plane does not cut it".into());
    }
    Ok(out)
}

impl Session {
    /// Adds the curves where the plane of sketch `sketch` cuts each named solid, fixed in
    /// place and linked to the solid when `linked`. Returns (body, curve ids) per solid.
    pub fn intersect_bodies(
        &mut self,
        sketch: u32,
        bodies: &[String],
        linked: bool,
    ) -> Result<Vec<(String, Vec<u32>)>, String> {
        let plane = self.doc.sketch(sketch).map_err(|e| e.to_string())?.plane;
        let mut cuts = Vec::new();
        for name in bodies {
            let solid = self
                .solids()
                .iter()
                .find(|s| &s.name == name)
                .ok_or_else(|| format!("there is no body named {name:?}"))?;
            let curves = section_curves(&solid.body, &plane).map_err(|e| format!("{name}: {e}"))?;
            cuts.push((name.clone(), solid.feature, curves));
        }
        let f = self.doc.sketch_mut(sketch).map_err(|e| e.to_string())?;
        if f.intersections.len() + cuts.len() > chamfera_doc::MAX_INTERSECTIONS {
            return Err("the sketch holds too many intersection curves".into());
        }
        let mut out = Vec::new();
        for (name, feature, curves) in cuts {
            if curves.len() > chamfera_doc::MAX_INTERSECTION_CURVES {
                return Err(format!("{name}: the cut has too many curves"));
            }
            let mut ids = Vec::new();
            for g in &curves {
                ids.push(f.sketch.add_fixed_curve(g).map_err(|e| e.to_string())?.0);
            }
            f.intersections.push(Intersection {
                body: name.clone(),
                feature,
                curves: ids.clone(),
                linked,
            });
            out.push((name, ids));
        }
        f.sketch.solve().map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Moves the linked intersection curves of `sketch` onto the cut of their bodies as
    /// built so far (`made`: feature, name and body), and solves. A body that is not
    /// there, or that the plane no longer cuts, leaves its curves alone. On failure the
    /// sketch is unchanged.
    pub(crate) fn follow_intersections(
        &mut self,
        sketch: u32,
        made: &[(u32, String, Body)],
    ) -> Result<(), String> {
        let Ok(f) = self.doc.sketch(sketch) else {
            return Ok(());
        };
        if f.intersections.iter().all(|i| !i.linked) {
            return Ok(());
        }
        let plane = f.plane;
        let mut next = f.sketch.clone();
        let mut links = f.intersections.clone();
        let mut moved = false;
        for link in links.iter_mut().filter(|i| i.linked) {
            // The latest body of that name: later features replace earlier states.
            let Some((_, _, body)) = made.iter().rev().find(|m| m.1 == link.body) else {
                continue;
            };
            let Ok(curves) = section_curves(body, &plane) else {
                continue;
            };
            let same = curves.len() == link.curves.len()
                && curves
                    .iter()
                    .zip(&link.curves)
                    .all(|(g, c)| next.move_fixed_curve(CurveId(*c), g).is_ok());
            if !same {
                for c in &link.curves {
                    let _ = next.remove_curve(CurveId(*c));
                }
                let mut ids = Vec::new();
                for g in &curves {
                    ids.push(
                        next.add_fixed_curve(g)
                            .map_err(|x| format!("{}: {x}", link.body))?
                            .0,
                    );
                }
                link.curves = ids;
            }
            moved = true;
        }
        if !moved {
            return Ok(());
        }
        next.solve()
            .map_err(|e| format!("the sketch cannot follow its intersection curves: {e}"))?;
        if let Ok(f) = self.doc.sketch_mut(sketch) {
            f.sketch = next;
            f.intersections = links;
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

//! Building surface features on the timeline: swept surfaces, patches, stitches and
//! trims (the geometry is the kernel's `surfacing`). Each works on the bodies built so
//! far and returns the bodies after it, or an error and leaves them as they were.

use crate::construct::Constructed;
use crate::solids::{face_plane, feature_profiles, name_edges, name_faces};
use chamfera_doc::{
    Document, Feature, SketchFeature, SurfaceFeature, SurfacePatch, SurfaceShapes, SurfaceStitch,
    SurfaceTrim, TrimKeep, TrimTool,
};
use chamfera_kernel::{Body, ProfileEdge, SurfaceProfile};
use chamfera_sketch::Curve;
use std::collections::BTreeMap;

type Made = Vec<(u32, String, Body)>;

/// The bodies after surface feature `feature`, from the bodies built so far.
pub(crate) fn build(
    doc: &Document,
    feature: &Feature,
    sf: &SurfaceFeature,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<Made, String> {
    let new_bodies = |bodies: Vec<Body>| -> Made {
        let n = bodies.len();
        let mut next = made.to_vec();
        for (k, mut b) in bodies.into_iter().enumerate() {
            name_faces(&mut b, feature.id);
            name_edges(&mut b, feature.id);
            let label = if n == 1 {
                feature.name.clone()
            } else {
                format!("{} ({})", feature.name, k + 1)
            };
            next.push((feature.id, label, b));
        }
        next
    };
    match sf {
        SurfaceFeature::Extrude(e) => {
            let f = doc.sketch(e.shapes.sketch).map_err(|x| x.to_string())?;
            let d = doc
                .evaluate_feature_value(&e.distance, chamfera_doc::expr::Dim::LENGTH)
                .map_err(|x| x.to_string())?;
            let (plane, d) = if e.symmetric {
                let start = f
                    .plane
                    .with_origin(f.plane.origin() + f.plane.z() * (-0.5 * d.abs()))
                    .ok_or("surface extrude: the extrude leaves the working range")?;
                (start, d.abs())
            } else {
                (f.plane, d)
            };
            let bodies = shapes(f, &e.shapes)?
                .iter()
                .map(|p| {
                    chamfera_kernel::surface_extrude(&plane, p, d)
                        .map_err(|x| format!("surface extrude: {x}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(new_bodies(bodies))
        }
        SurfaceFeature::Revolve(r) => {
            let f = doc.sketch(r.shapes.sketch).map_err(|x| x.to_string())?;
            let angle = doc
                .evaluate_feature_value(&r.angle, chamfera_doc::expr::Dim::ANGLE)
                .map_err(|x| x.to_string())?;
            let axis = crate::solids::revolve_axis(f, r.axis, r.axis_feature, constructs)?;
            let bodies = shapes(f, &r.shapes)?
                .iter()
                .map(|p| {
                    chamfera_kernel::surface_revolve(&f.plane, p, &axis, angle)
                        .map_err(|x| format!("surface revolve: {x}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(new_bodies(bodies))
        }
        SurfaceFeature::Sweep(w) => {
            let f = doc.sketch(w.shapes.sketch).map_err(|x| x.to_string())?;
            let p = doc.sketch(w.path_sketch).map_err(|x| x.to_string())?;
            let path = crate::solids::sweep_path_segments(p, &w.path)?;
            let bodies = shapes(f, &w.shapes)?
                .iter()
                .map(|s| {
                    chamfera_kernel::surface_sweep(&f.plane, s, &path)
                        .map_err(|x| format!("surface sweep: {x}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(new_bodies(bodies))
        }
        SurfaceFeature::Patch(p) => Ok(new_bodies(patch(doc, p, made)?)),
        SurfaceFeature::Stitch(s) => stitch(s, feature, made),
        SurfaceFeature::Trim(t) => trim(t, feature, made, constructs),
    }
}

/// The closed profiles and the open chain of a swept surface.
fn shapes(f: &SketchFeature, s: &SurfaceShapes) -> Result<Vec<SurfaceProfile>, String> {
    let mut out: Vec<SurfaceProfile> = feature_profiles(f, &s.profiles)?
        .into_iter()
        .map(SurfaceProfile::Closed)
        .collect();
    if !s.curves.is_empty() {
        out.push(SurfaceProfile::Open(chain(f, &s.curves)?));
    }
    if out.is_empty() {
        return Err("pick at least one profile or curve".into());
    }
    Ok(out)
}

/// One sketch curve as profile edges from its start to its end.
fn curve_edges(f: &SketchFeature, id: u32) -> Result<Vec<ProfileEdge>, String> {
    let gone = || format!("curve {id} of {} is gone", f.name);
    let curve = f
        .sketch
        .curve(chamfera_sketch::CurveId(id))
        .ok_or_else(gone)?;
    let pt = |p: chamfera_sketch::PointId| f.sketch.point(p).ok_or_else(gone);
    Ok(match curve {
        Curve::Line { start, end, .. } => vec![ProfileEdge::line(pt(*start)?, pt(*end)?)],
        Curve::Arc {
            centre, start, end, ..
        } => vec![
            ProfileEdge::arc_from_points(pt(*centre)?, pt(*start)?, pt(*end)?)
                .map_err(|e| format!("arc {id}: {e}"))?,
        ],
        Curve::Circle { centre, radius, .. } => vec![ProfileEdge::circle(pt(*centre)?, *radius)],
        Curve::Spline { .. } => {
            let c = f
                .sketch
                .spline_of(curve)
                .ok_or_else(|| format!("spline {id} has no curve"))?;
            if c.is_closed() {
                let (lo, hi) = c.domain();
                let (a, b) = c
                    .split_at(0.5 * (lo + hi))
                    .map_err(|e| format!("spline {id}: {e}"))?;
                vec![ProfileEdge::Spline(a), ProfileEdge::Spline(b)]
            } else {
                vec![ProfileEdge::Spline(c)]
            }
        }
        Curve::Ellipse { .. } | Curve::EllipticalArc { .. } => {
            return Err("ellipses cannot be swept into surfaces yet".into());
        }
    })
}

fn reversed(edges: Vec<ProfileEdge>) -> Vec<ProfileEdge> {
    edges.iter().rev().map(ProfileEdge::reversed).collect()
}

/// The curves joined end to end in the order given, each turned to follow the last.
fn chain(f: &SketchFeature, curves: &[u32]) -> Result<Vec<ProfileEdge>, String> {
    let tol = chamfera_geom::LINEAR_TOLERANCE;
    let mut pieces = Vec::with_capacity(curves.len());
    for id in curves {
        pieces.push(curve_edges(f, *id)?);
    }
    let ends = |p: &[ProfileEdge]| match (p.first(), p.last()) {
        (Some(a), Some(b)) => Some((a.start(), b.end())),
        _ => None,
    };
    let mut out: Vec<ProfileEdge> = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        let (s, e) = ends(piece).ok_or("a curve has no edges")?;
        let piece = if k == 0 {
            // Turn the first so it ends where the second meets it.
            match pieces.get(1).and_then(|n| ends(n)) {
                Some((ns, ne)) if s.distance(ns) <= tol || s.distance(ne) <= tol => {
                    reversed(piece.clone())
                }
                _ => piece.clone(),
            }
        } else {
            let at = out.last().map(ProfileEdge::end).ok_or("the chain is empty")?;
            if s.distance(at) <= tol {
                piece.clone()
            } else if e.distance(at) <= tol {
                reversed(piece.clone())
            } else {
                return Err(format!(
                    "curve {} does not join the curve before it; pick curves that join end to end, in order",
                    curves.get(k).copied().unwrap_or(0)
                ));
            }
        };
        out.extend(piece);
    }
    Ok(out)
}

fn index_of(made: &[(u32, String, Body)], name: &str) -> Result<usize, String> {
    made.iter()
        .position(|m| m.1 == name)
        .ok_or_else(|| format!("there is no body named {name:?} at this point"))
}

fn patch(doc: &Document, p: &SurfacePatch, made: &[(u32, String, Body)]) -> Result<Vec<Body>, String> {
    if let Some(sketch) = p.sketch {
        let f = doc.sketch(sketch).map_err(|x| x.to_string())?;
        return feature_profiles(f, &p.profiles)?
            .iter()
            .map(|pr| {
                chamfera_kernel::patch_profile(&f.plane, pr).map_err(|x| format!("patch: {x}"))
            })
            .collect();
    }
    // Every edge of one body.
    for (_, _, body) in made {
        let mut found = Vec::new();
        for name in &p.edges {
            if let Some(e) = body
                .edges()
                .into_iter()
                .find(|e| body.edge_name(*e).ok().flatten() == Some(name.as_str()))
            {
                found.push(e);
            }
        }
        if found.is_empty() {
            continue;
        }
        if found.len() != p.edges.len() {
            return Err("patch: the edges must all belong to one body".into());
        }
        return chamfera_kernel::patch_edges(body, &found)
            .map(|b| vec![b])
            .map_err(|x| format!("patch: {x}"));
    }
    Err(format!(
        "patch: the edge {:?} is no longer there",
        p.edges.first().map_or("", String::as_str)
    ))
}

fn stitch(s: &SurfaceStitch, feature: &Feature, made: &[(u32, String, Body)]) -> Result<Made, String> {
    let mut picked = Vec::new();
    for name in &s.bodies {
        let i = index_of(made, name)?;
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    let bodies: Vec<&Body> = picked
        .iter()
        .filter_map(|i| made.get(*i).map(|m| &m.2))
        .collect();
    let joined = chamfera_kernel::stitch(&bodies).map_err(|e| format!("stitch: {e}"))?;
    let first = picked.iter().copied().min().unwrap_or(0);
    let mut next = Vec::with_capacity(made.len());
    for (k, m) in made.iter().enumerate() {
        if k == first {
            next.push((feature.id, feature.name.clone(), joined.clone()));
        } else if !picked.contains(&k) {
            next.push(m.clone());
        }
    }
    Ok(next)
}

fn trim(
    t: &SurfaceTrim,
    feature: &Feature,
    made: &[(u32, String, Body)],
    constructs: &BTreeMap<u32, Constructed>,
) -> Result<Made, String> {
    let i = index_of(made, &t.body)?;
    let surface = &made.get(i).ok_or("the surface is missing")?.2;
    if !surface.is_open() {
        return Err(format!(
            "trim: {} is a closed solid; trim works on surface bodies (use Split Body for solids)",
            t.body
        ));
    }
    let k = |e: chamfera_kernel::KernelError| match e {
        chamfera_kernel::KernelError::EmptyResult => {
            format!("trim: nothing of {} is on the side to keep", t.body)
        }
        e => format!("trim: {e}"),
    };
    let mut out = match &t.tool {
        TrimTool::Plane { plane } => {
            let (frame, _) = crate::construct::plane_of(&made.to_vec(), constructs, plane)?;
            chamfera_kernel::trim_by_plane(surface, &frame, t.keep == TrimKeep::Below)
                .map_err(k)?
        }
        TrimTool::Body { body } => {
            let j = index_of(made, body)?;
            if j == i {
                return Err("trim: a surface cannot trim itself".into());
            }
            let tool = &made.get(j).ok_or("the tool is missing")?.2;
            if tool.is_open() {
                // A flat sheet of one face trims by its plane, toward its normal.
                let faces = tool.faces();
                let frame = match faces.as_slice() {
                    [f] => face_plane(tool, *f),
                    _ => None,
                }
                .ok_or_else(|| {
                    "trim: a surface tool must be one flat face for now (or use a closed solid)"
                        .to_owned()
                })?;
                chamfera_kernel::trim_by_plane(surface, &frame, t.keep == TrimKeep::Below)
                    .map_err(k)?
            } else {
                chamfera_kernel::trim_by_solid(surface, tool, t.keep == TrimKeep::Above)
                    .map_err(k)?
            }
        }
    };
    name_faces(&mut out, feature.id);
    name_edges(&mut out, feature.id);
    let mut next = made.to_vec();
    if let Some(slot) = next.get_mut(i) {
        slot.2 = out;
    }
    Ok(next)
}

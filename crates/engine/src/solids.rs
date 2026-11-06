//! Building the timeline: each feature becomes kernel bodies, tessellated for the viewport.
//!
//! The design stores only what the user asked for (sketches, features, parameters); the
//! solids are worked out again from it after every change, so a parameter change rebuilds
//! everything downstream. A feature that cannot be built keeps its error (shown in the
//! timeline) and the rest of the timeline still builds, as CAD users expect.

use crate::Session;
use chamfera_doc::{Extrude, ExtrudeDirection, FeatureKind, Revolve};
use chamfera_geom::{Point2, Point3};
use chamfera_kernel::{Body, Line2, Profile, ProfileEdge, TessOptions, tessellate};
use chamfera_sketch::{Curve, ProfileLoop, Region, Sketch};
use std::collections::BTreeMap;

/// A solid built by a feature.
#[derive(Debug, Clone)]
pub struct Solid {
    pub feature: u32,
    pub name: String,
    pub body: Body,
    /// The scene body that shows it, if it could be displayed.
    pub scene_id: Option<u32>,
}

/// What the last rebuild produced.
#[derive(Debug, Default, Clone)]
pub struct Built {
    pub solids: Vec<Solid>,
    /// Features that could not be built, and why.
    pub errors: BTreeMap<u32, String>,
    /// Features whose solids were built but cannot be shown yet, and why.
    pub display_errors: BTreeMap<u32, String>,
}

/// A sketch loop as exact kernel profile edges: lines stay lines, arcs and circles stay
/// arcs and circles, in the loop's direction.
fn loop_edges(sketch: &Sketch, l: &ProfileLoop) -> Result<Vec<ProfileEdge>, String> {
    let gone = || "a curve of the profile is gone".to_owned();
    l.segments
        .iter()
        .map(|seg| {
            let curve = sketch.curve(seg.curve).ok_or_else(gone)?;
            let edge = match curve {
                Curve::Line { start, end, .. } => {
                    let (a, b) = (
                        sketch.point(*start).ok_or_else(gone)?,
                        sketch.point(*end).ok_or_else(gone)?,
                    );
                    ProfileEdge::line(a, b)
                }
                Curve::Circle { centre, radius, .. } => {
                    ProfileEdge::circle(sketch.point(*centre).ok_or_else(gone)?, *radius)
                }
                Curve::Arc {
                    centre, start, end, ..
                } => ProfileEdge::arc_from_points(
                    sketch.point(*centre).ok_or_else(gone)?,
                    sketch.point(*start).ok_or_else(gone)?,
                    sketch.point(*end).ok_or_else(gone)?,
                )
                .map_err(|e| format!("an arc of the profile: {e}"))?,
            };
            Ok(if seg.reversed { edge.reversed() } else { edge })
        })
        .collect()
}

/// A sketch region as an exact kernel profile (outer loop and holes).
fn region_profile(sketch: &Sketch, r: &Region) -> Result<Profile, String> {
    let outer = loop_edges(sketch, &r.outer)?;
    let holes = r
        .holes
        .iter()
        .map(|h| loop_edges(sketch, h))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Profile::new(outer, holes))
}

/// The profiles a feature uses, found from its seed points.
fn feature_profiles(
    f: &chamfera_doc::SketchFeature,
    seeds: &[Point2],
) -> Result<Vec<Profile>, String> {
    seeds
        .iter()
        .map(|seed| {
            let region = f.sketch.region_at(*seed).ok_or_else(|| {
                format!(
                    "the profile at ({:.2}, {:.2}) of {} is no longer closed",
                    seed.x, seed.y, f.name
                )
            })?;
            region_profile(&f.sketch, &region)
        })
        .collect()
}

/// The bodies one extrude makes: one per profile.
fn build_extrude(doc: &chamfera_doc::Document, e: &Extrude) -> Result<Vec<Body>, String> {
    let f = doc.sketch(e.sketch).map_err(|x| x.to_string())?;
    let distance = doc
        .evaluate_feature_value(&e.distance, chamfera_doc::expr::Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    feature_profiles(f, &e.profiles)?
        .iter()
        .map(|profile| {
            match e.direction {
                ExtrudeDirection::Normal => chamfera_kernel::extrude(&f.plane, profile, distance),
                ExtrudeDirection::Reversed => {
                    chamfera_kernel::extrude(&f.plane, profile, -distance)
                }
                ExtrudeDirection::Symmetric => {
                    chamfera_kernel::extrude_symmetric(&f.plane, profile, distance)
                }
            }
            .map_err(|x| format!("extrude: {x}"))
        })
        .collect()
}

/// The bodies one revolve makes: one per profile, about a line of the same sketch.
fn build_revolve(doc: &chamfera_doc::Document, r: &Revolve) -> Result<Vec<Body>, String> {
    let f = doc.sketch(r.sketch).map_err(|x| x.to_string())?;
    let angle = doc
        .evaluate_feature_value(&r.angle, chamfera_doc::expr::Dim::ANGLE)
        .map_err(|x| x.to_string())?;
    let axis = match f.sketch.curve(chamfera_sketch::CurveId(r.axis)) {
        Some(Curve::Line { start, end, .. }) => {
            let (Some(a), Some(b)) = (f.sketch.point(*start), f.sketch.point(*end)) else {
                return Err("the axis line has lost a point".into());
            };
            Line2::new(a, b - a).map_err(|e| format!("the axis: {e}"))?
        }
        _ => return Err(format!("the axis {} is not a line of {}", r.axis, f.name)),
    };
    feature_profiles(f, &r.profiles)?
        .iter()
        .map(|profile| {
            chamfera_kernel::revolve(&f.plane, profile, &axis, angle)
                .map_err(|x| format!("revolve: {x}"))
        })
        .collect()
}

impl Session {
    /// Rebuilds every feature, in timeline order, and replaces the solid bodies in the
    /// scene. Meshes and other bodies (imported STL) are left alone.
    pub fn rebuild(&mut self) {
        let mut built = Built::default();
        let mut made: Vec<(u32, String, Body)> = Vec::new();
        for feature in &self.doc.features {
            if feature.suppressed {
                continue;
            }
            let result = match &feature.kind {
                FeatureKind::Extrude(e) => build_extrude(&self.doc, e),
                FeatureKind::Revolve(r) => build_revolve(&self.doc, r),
            };
            match result {
                Ok(bodies) => {
                    let n = bodies.len();
                    for (k, body) in bodies.into_iter().enumerate() {
                        let name = if n == 1 {
                            feature.name.clone()
                        } else {
                            format!("{} ({})", feature.name, k + 1)
                        };
                        made.push((feature.id, name, body));
                    }
                }
                Err(e) => {
                    built.errors.insert(feature.id, e);
                }
            }
        }
        // Replace the scene's solid bodies.
        let old: Vec<u32> = self
            .built
            .solids
            .iter()
            .filter_map(|s| s.scene_id)
            .collect();
        self.scene.bodies.retain(|b| !old.contains(&b.id));
        if self.scene.selection.is_some_and(|h| old.contains(&h.body)) {
            self.scene.selection = None;
        }
        let options = TessOptions::default();
        for (feature, name, body) in made {
            // A solid that cannot be shown is still built: its measures and export work.
            let scene_id = match tessellate(&body, &options) {
                Ok(m) => {
                    let id = self.scene.add(&name, m.mesh);
                    if id.is_none() {
                        built
                            .display_errors
                            .insert(feature, "the scene holds no more bodies".into());
                    }
                    id
                }
                Err(e) => {
                    built
                        .display_errors
                        .insert(feature, format!("the solid cannot be shown yet: {e}"));
                    None
                }
            };
            built.solids.push(Solid {
                feature,
                name,
                body,
                scene_id,
            });
        }
        self.built = built;
    }

    /// The solids of the last rebuild.
    pub fn solids(&self) -> &[Solid] {
        &self.built.solids
    }

    /// Features that failed in the last rebuild.
    pub fn feature_errors(&self) -> &BTreeMap<u32, String> {
        &self.built.errors
    }

    /// Features built but not displayable (for example curved faces before their
    /// tessellation exists).
    pub fn display_errors(&self) -> &BTreeMap<u32, String> {
        &self.built.display_errors
    }

    /// The profile under pixel `(x, y)`: the nearest sketch plane the ray meets where a
    /// closed region lies. Returns (sketch id, seed point inside the region, its area).
    pub fn profile_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<(u32, Point2, f64)> {
        let bounds = self.scene.bounds();
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let mut best: Option<(f64, u32, Point2, f64)> = None;
        for f in &self.doc.sketches {
            let n = f.plane.z();
            let denom = n.dot(ray.direction);
            if denom.abs() < 1e-6 {
                continue;
            }
            let t = n.dot(f.plane.origin() - ray.origin) / denom;
            if t < 0.0 {
                continue;
            }
            let p = f.plane.to_plane(ray.at(t));
            if let Some(region) = f.sketch.region_at(p)
                && best.is_none_or(|b| t < b.0)
            {
                // Remember the click point itself: it is inside this region and is what
                // the user meant.
                best = Some((t, f.id, p, region.area));
            }
        }
        best.map(|(_, s, p, a)| (s, p, a))
    }

    /// The outline (outer loop and holes) of the region at `seed`, as world line segments.
    pub fn profile_outline(&self, sketch: u32, seed: Point2) -> Vec<[Point3; 2]> {
        let Ok(f) = self.doc.sketch(sketch) else {
            return Vec::new();
        };
        let Some(region) = f.sketch.region_at(seed) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for l in std::iter::once(&region.outer).chain(region.holes.iter()) {
            let poly = f.sketch.loop_polyline(l);
            for i in 0..poly.len() {
                if let (Some(a), Some(b)) = (poly.get(i), poly.get((i + 1) % poly.len())) {
                    out.push([f.plane.from_plane(*a), f.plane.from_plane(*b)]);
                }
            }
        }
        out
    }
}

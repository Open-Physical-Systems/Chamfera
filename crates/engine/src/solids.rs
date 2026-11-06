//! Building the timeline: each feature becomes kernel bodies, tessellated for the viewport.
//!
//! The design stores only what the user asked for (sketches, features, parameters); the
//! solids are worked out again from it after every change, so a parameter change rebuilds
//! everything downstream. A feature that cannot be built keeps its error (shown in the
//! timeline) and the rest of the timeline still builds, as CAD users expect.

use crate::Session;
use chamfera_doc::{Extrude, ExtrudeDirection, FeatureKind};
use chamfera_geom::{Frame, Point2, Point3};
use chamfera_kernel::{Body, TessOptions, tessellate};
use chamfera_sketch::{Region, Sketch};
use std::collections::BTreeMap;

/// A solid built by a feature.
#[derive(Debug, Clone)]
pub struct Solid {
    pub feature: u32,
    pub name: String,
    pub body: Body,
    /// The scene body that shows it.
    pub scene_id: u32,
}

/// What the last rebuild produced.
#[derive(Debug, Default, Clone)]
pub struct Built {
    pub solids: Vec<Solid>,
    /// Features that could not be built, and why.
    pub errors: BTreeMap<u32, String>,
}

/// A region as polygons in sketch coordinates: the outer loop and its holes.
/// Arcs are subdivided until the kernel's extrude takes arcs exactly (K4).
fn region_polygons(sketch: &Sketch, r: &Region) -> (Vec<Point2>, Vec<Vec<Point2>>) {
    let outer = sketch.loop_polyline(&r.outer);
    let holes = r.holes.iter().map(|h| sketch.loop_polyline(h)).collect();
    (outer, holes)
}

/// The bodies one extrude makes: one per profile.
fn build_extrude(doc: &chamfera_doc::Document, e: &Extrude) -> Result<Vec<Body>, String> {
    let f = doc.sketch(e.sketch).map_err(|x| x.to_string())?;
    let distance = doc
        .evaluate_feature_value(&e.distance, chamfera_doc::expr::Dim::LENGTH)
        .map_err(|x| x.to_string())?;
    let (start, height) = match e.direction {
        ExtrudeDirection::Normal => (0.0, distance),
        ExtrudeDirection::Reversed => (-distance, distance),
        ExtrudeDirection::Symmetric => (-distance / 2.0, distance),
    };
    // A negative distance goes the other way.
    let (start, height) = if height < 0.0 {
        (start + height, -height)
    } else {
        (start, height)
    };
    let origin: Point3 = f.plane.origin() + f.plane.z() * start;
    let frame: Frame = f
        .plane
        .with_origin(origin)
        .ok_or("the extrude start is outside the working range")?;
    let mut bodies = Vec::new();
    for seed in &e.profiles {
        let region = f.sketch.region_at(*seed).ok_or_else(|| {
            format!(
                "the profile at ({:.2}, {:.2}) of {} is no longer closed",
                seed.x, seed.y, f.name
            )
        })?;
        let (outer, holes) = region_polygons(&f.sketch, &region);
        let body = Body::prism_with_holes(&frame, &outer, &holes, height)
            .map_err(|x| format!("extrude: {x}"))?;
        bodies.push(body);
    }
    Ok(bodies)
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
                FeatureKind::Revolve(_) => {
                    Err("revolve is not available yet (it comes with kernel rung K4)".into())
                }
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
        let old: Vec<u32> = self.built.solids.iter().map(|s| s.scene_id).collect();
        self.scene.bodies.retain(|b| !old.contains(&b.id));
        if self.scene.selection.is_some_and(|h| old.contains(&h.body)) {
            self.scene.selection = None;
        }
        let options = TessOptions::default();
        for (feature, name, body) in made {
            let mesh = match tessellate(&body, &options) {
                Ok(m) => m.mesh,
                Err(e) => {
                    built
                        .errors
                        .insert(feature, format!("could not display the solid: {e}"));
                    continue;
                }
            };
            let Some(scene_id) = self.scene.add(&name, mesh) else {
                built
                    .errors
                    .insert(feature, "the scene holds no more bodies".into());
                continue;
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

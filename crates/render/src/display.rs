//! How the model is displayed: visual styles, per-body appearance, the section plane and
//! the order bodies are drawn in. Plain data and CPU math only; `chamfera-gpu` draws it.
//!
//! All of this is display state: it never changes the design's geometry, and model
//! decisions (picking aside) never read it.

use crate::environment::EnvironmentPreset;
use crate::material::{LIBRARY, LibraryMaterial, Material};
use crate::{Camera, Projection, Rgba, Scene};
use chamfera_geom::{Point3, Ray, TriMesh, Vec3};
use serde::{Deserialize, Serialize};

/// How bodies are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualStyle {
    /// Lit faces only.
    Shaded,
    /// Lit faces and the edges you can see.
    #[default]
    ShadedEdges,
    /// Lit faces, visible edges, and the edges behind faces dashed and faint.
    ShadedHiddenEdges,
    /// Every edge, no faces.
    Wireframe,
    /// The edges you can see, no faces: faces only hide what is behind them.
    WireframeHidden,
    /// Physically based materials in the lighting environment, with a floor and its
    /// shadows (`chamfera_render::environment`); no edges.
    Realistic,
}

/// How a style draws faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacePass {
    /// Lit, in the body's colour.
    Shaded,
    /// Physically based materials, the environment's lights, the floor and shadows.
    Realistic,
    /// Into the depth buffer only, so they hide edges behind them but are not seen.
    DepthOnly,
    /// Not at all.
    None,
}

/// One pass over the edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePass {
    /// Edges in front of every face (solid lines).
    Visible,
    /// Edges behind a face (dashed, `theme.edge_hidden`).
    Hidden,
}

/// What a visual style draws, in order: faces first, then the edge passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StylePlan {
    pub faces: FacePass,
    pub edges: &'static [EdgePass],
}

impl VisualStyle {
    pub const ALL: [VisualStyle; 6] = [
        VisualStyle::Shaded,
        VisualStyle::ShadedEdges,
        VisualStyle::ShadedHiddenEdges,
        VisualStyle::Wireframe,
        VisualStyle::WireframeHidden,
        VisualStyle::Realistic,
    ];

    /// The command parameter value (`view.style`).
    pub fn id(self) -> &'static str {
        match self {
            VisualStyle::Shaded => "shaded",
            VisualStyle::ShadedEdges => "shaded_edges",
            VisualStyle::ShadedHiddenEdges => "shaded_hidden_edges",
            VisualStyle::Wireframe => "wireframe",
            VisualStyle::WireframeHidden => "wireframe_hidden",
            VisualStyle::Realistic => "realistic",
        }
    }

    /// What menus show.
    pub fn label(self) -> &'static str {
        match self {
            VisualStyle::Shaded => "Shaded",
            VisualStyle::ShadedEdges => "Shaded with Visible Edges",
            VisualStyle::ShadedHiddenEdges => "Shaded with Hidden Edges",
            VisualStyle::Wireframe => "Wireframe",
            VisualStyle::WireframeHidden => "Wireframe, Hidden Lines Removed",
            VisualStyle::Realistic => "Realistic",
        }
    }

    pub fn parse(s: &str) -> Option<VisualStyle> {
        VisualStyle::ALL.into_iter().find(|v| v.id() == s)
    }

    pub fn plan(self) -> StylePlan {
        use EdgePass::{Hidden, Visible};
        match self {
            VisualStyle::Shaded => StylePlan {
                faces: FacePass::Shaded,
                edges: &[],
            },
            VisualStyle::ShadedEdges => StylePlan {
                faces: FacePass::Shaded,
                edges: &[Visible],
            },
            VisualStyle::ShadedHiddenEdges => StylePlan {
                faces: FacePass::Shaded,
                edges: &[Hidden, Visible],
            },
            // Nothing is in the depth buffer, so every edge passes the visible test.
            VisualStyle::Wireframe => StylePlan {
                faces: FacePass::None,
                edges: &[Visible],
            },
            VisualStyle::WireframeHidden => StylePlan {
                faces: FacePass::DepthOnly,
                edges: &[Visible],
            },
            VisualStyle::Realistic => StylePlan {
                faces: FacePass::Realistic,
                edges: &[],
            },
        }
    }
}

/// Lowest opacity a body may have: below it, a body would vanish and could not be found.
pub const MIN_OPACITY: f32 = 0.05;
/// Below this opacity a body counts as transparent (drawn blended, after opaque ones).
pub const OPAQUE: f32 = 0.999;

/// A body's colour, opacity and physically based material.
///
/// `colour` is the user's choice and is data, not a theme colour: it is what the body is
/// made of, the same in either theme. Without one, the body takes `theme.body`. The
/// material numbers are read by the realistic style and the path tracer only.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub colour: Option<Rgba>,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub metallic: f32,
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    #[serde(default)]
    pub transmission: f32,
    #[serde(default = "default_ior")]
    pub ior: f32,
}

fn one() -> f32 {
    1.0
}

fn default_roughness() -> f32 {
    crate::material::DEFAULT_ROUGHNESS
}

fn default_ior() -> f32 {
    crate::material::DEFAULT_IOR
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            colour: None,
            opacity: 1.0,
            metallic: 0.0,
            roughness: default_roughness(),
            transmission: 0.0,
            ior: default_ior(),
        }
    }
}

impl Appearance {
    /// The opacity, repaired into `MIN_OPACITY..=1` (hostile or old data).
    pub fn opacity(&self) -> f32 {
        if self.opacity.is_finite() {
            self.opacity.clamp(MIN_OPACITY, 1.0)
        } else {
            1.0
        }
    }

    pub fn is_transparent(&self) -> bool {
        self.opacity() < OPAQUE
    }

    /// The material for shading, with `fallback` as the colour when it has none.
    pub fn material(&self, fallback: Rgba) -> Material {
        Material::new(
            self.colour.unwrap_or(fallback),
            self.metallic,
            self.roughness,
            self.transmission,
            self.ior,
        )
    }
}

/// A named appearance the UI offers: an entry of the material library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub appearance: Appearance,
}

/// The appearance library: the material library (`crate::material::LIBRARY`) with each
/// entry's appearance.
pub fn presets() -> impl Iterator<Item = Preset> {
    LIBRARY.iter().map(preset_of)
}

fn preset_of(x: &LibraryMaterial) -> Preset {
    Preset {
        id: x.id,
        label: x.label,
        appearance: Appearance {
            colour: Some(x.colour),
            opacity: x.opacity,
            metallic: x.metallic,
            roughness: x.roughness,
            transmission: x.transmission,
            ior: x.ior,
        },
    }
}

pub fn preset_by_id(id: &str) -> Option<Preset> {
    crate::material::by_id(id).map(preset_of)
}

/// A section plane: the side its normal points to is cut away, so you look into the part.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub origin: Point3,
    /// Unit length, pointing to the removed side.
    pub normal: Vec3,
}

/// Points this close to the plane count as on it (mm).
const ON_PLANE: f64 = 1e-9;

impl Section {
    /// `None` when the origin is not finite or the normal has no length.
    pub fn new(origin: Point3, normal: Vec3) -> Option<Section> {
        if !origin.is_valid() {
            return None;
        }
        Some(Section {
            origin,
            normal: normal.normalized()?,
        })
    }

    /// Distance of `p` from the plane, positive on the removed side.
    pub fn signed_distance(&self, p: Point3) -> f64 {
        (p - self.origin).dot(self.normal)
    }

    /// Whether `p` is on the kept side (or on the plane).
    pub fn keeps(&self, p: Point3) -> bool {
        self.signed_distance(p) <= ON_PLANE
    }

    /// The part of `mesh` on the kept side: whole triangles kept as they are, triangles
    /// across the plane cut at it. Also returns, for every output triangle, the input
    /// triangle it came from, so picks still name the right face.
    pub fn clip_mesh(&self, mesh: &TriMesh) -> (TriMesh, Vec<usize>) {
        let mut out = TriMesh::default();
        let mut source = Vec::new();
        for i in 0..mesh.triangles.len() {
            let Some(tri) = mesh.triangle(i) else {
                continue;
            };
            let d = tri.map(|p| self.signed_distance(p));
            if d.iter().all(|v| *v <= ON_PLANE) {
                push_polygon(&mut out, &mut source, i, &tri);
                continue;
            }
            if d.iter().all(|v| *v > ON_PLANE) {
                continue;
            }
            // Sutherland-Hodgman against one plane: walk the edges, keep inside corners and
            // add the crossing of every edge that changes side.
            let mut poly: Vec<Point3> = Vec::with_capacity(4);
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let (da, db) = (d[k], d[(k + 1) % 3]);
                if da <= ON_PLANE {
                    poly.push(a);
                }
                if (da <= ON_PLANE) != (db <= ON_PLANE) {
                    let t = da / (da - db);
                    poly.push(a + (b - a) * t);
                }
            }
            push_polygon(&mut out, &mut source, i, &poly);
        }
        (out, source)
    }
}

/// Adds a convex polygon as a triangle fan (degenerate fans are skipped).
fn push_polygon(out: &mut TriMesh, source: &mut Vec<usize>, from: usize, poly: &[Point3]) {
    let Some((first, rest)) = poly.split_first() else {
        return;
    };
    for pair in rest.windows(2) {
        let [b, c] = pair else { continue };
        if (*b - *first).cross(*c - *first).length() <= 1e-18 {
            continue;
        }
        let Ok(base) = u32::try_from(out.positions.len()) else {
            return;
        };
        out.positions.extend([*first, *b, *c]);
        out.triangles.push([base, base + 1, base + 2]);
        source.push(from);
    }
}

/// Display state that is not per body.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Display {
    #[serde(default)]
    pub style: VisualStyle,
    #[serde(default)]
    pub section: Option<Section>,
    /// The lighting of the realistic style and of renders (`render.environment`).
    #[serde(default)]
    pub environment: EnvironmentPreset,
}

/// The order bodies are drawn in: opaque bodies (in scene order), then transparent bodies
/// from the farthest to the nearest, so each blends over what is behind it. Hidden bodies
/// are left out.
pub fn draw_order(scene: &Scene, camera: &Camera) -> (Vec<u32>, Vec<u32>) {
    let eye = camera.eye();
    let forward = camera.basis().forward;
    let mut opaque = Vec::new();
    let mut transparent: Vec<(f64, u32)> = Vec::new();
    for b in scene.bodies.iter().filter(|b| b.visible) {
        if b.appearance.is_transparent() {
            // Depth along the view direction (the same for every point of a plane facing
            // the viewer, in both projections).
            let depth = b
                .mesh
                .bounds()
                .map_or(0.0, |bb| (bb.center() - eye).dot(forward));
            transparent.push((depth, b.id));
        } else {
            opaque.push(b.id);
        }
    }
    transparent.sort_by(|a, b| b.0.total_cmp(&a.0));
    (opaque, transparent.into_iter().map(|(_, id)| id).collect())
}

/// Whether the point `p` (on a body's surface or edge) is hidden behind a face of a
/// visible body, seen with `camera`: the ray from the eye to it hits a face clearly
/// nearer. Respects the section plane. This is the CPU reference for what the GPU's depth
/// test hides in the hidden-edge style.
pub fn point_hidden(scene: &Scene, camera: &Camera, p: Point3) -> bool {
    let bounds = scene.bounds();
    let (origin, direction) = match camera.projection {
        Projection::Perspective => (camera.eye(), p - camera.eye()),
        Projection::Orthographic => {
            let (near, _) = camera.clip_range(bounds.as_ref());
            let f = camera.basis().forward;
            // Start in front of everything, on the line through p along the view.
            let back = (p - camera.eye()).dot(f) - near;
            (p + f * -back, f)
        }
    };
    let Some(ray) = Ray::new(origin, direction) else {
        return false;
    };
    let reach = (p - origin).dot(ray.direction);
    let tol = 1e-6 * reach.abs().max(1.0);
    scene.pick(&ray).is_some_and(|h| h.distance < reach - tol)
}

/// Each segment of the bodies' exact edges with whether it is hidden (judged at its
/// midpoint), for the visible bodies.
pub fn edge_segments(scene: &Scene, camera: &Camera) -> Vec<([Point3; 2], bool)> {
    let mut out = Vec::new();
    for b in scene.bodies.iter().filter(|b| b.visible) {
        for line in &b.edges {
            for pair in line.windows(2) {
                let [a, c] = pair else { continue };
                let mid = *a + (*c - *a) * 0.5;
                out.push(([*a, *c], point_hidden(scene, camera, mid)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StandardView;

    fn unit_box_edges() -> Vec<Vec<Point3>> {
        // The 12 edges of the box 0..10 on every axis, each as a two-point polyline.
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let mut e = Vec::new();
        for a in [0.0, 10.0] {
            for b in [0.0, 10.0] {
                e.push(vec![p(0.0, a, b), p(10.0, a, b)]);
                e.push(vec![p(a, 0.0, b), p(a, 10.0, b)]);
                e.push(vec![p(a, b, 0.0), p(a, b, 10.0)]);
            }
        }
        e
    }

    fn box_scene() -> (Scene, u32) {
        let mut s = Scene::default();
        let id = s
            .add(
                "box",
                TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0)),
            )
            .unwrap();
        s.body_mut(id).unwrap().edges = unit_box_edges();
        (s, id)
    }

    #[test]
    fn wireframe_draws_only_lines_and_shaded_draws_no_lines() {
        let w = VisualStyle::Wireframe.plan();
        assert_eq!(w.faces, FacePass::None);
        assert_eq!(w.edges, &[EdgePass::Visible]);
        let h = VisualStyle::WireframeHidden.plan();
        assert_eq!(h.faces, FacePass::DepthOnly);
        assert!(!h.edges.contains(&EdgePass::Hidden));
        assert!(VisualStyle::Shaded.plan().edges.is_empty());
        assert_eq!(
            VisualStyle::ShadedHiddenEdges.plan().edges,
            &[EdgePass::Hidden, EdgePass::Visible]
        );
        for v in VisualStyle::ALL {
            assert_eq!(VisualStyle::parse(v.id()), Some(v));
        }
        assert_eq!(VisualStyle::parse("Shaded"), None);
    }

    #[test]
    fn the_three_edges_at_the_far_back_corner_of_a_box_are_hidden_from_the_front() {
        // Front view turned 20 degrees right and 20 up: the eye is at -Y (front), +X and
        // +Z, so of the box's 8 corners only (0, 10, 0) (back, left, bottom) is out of
        // sight, and exactly the 3 edges meeting there are hidden: by hand, each of the
        // other 9 edges lies on a face turned towards the eye (front y=0, right x=10 or
        // top z=10).
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let (scene, _) = box_scene();
            let mut cam = Camera {
                target: Point3::new(5.0, 5.0, 5.0),
                distance: 60.0,
                projection,
                ..Camera::default()
            };
            cam.set_view(StandardView::Front);
            cam.orbit(20f64.to_radians(), 20f64.to_radians());
            let segs = edge_segments(&scene, &cam);
            assert_eq!(segs.len(), 12);
            let far = Point3::new(0.0, 10.0, 0.0);
            for ([a, b], hidden) in segs {
                let at_far = (a - far).length() < 1e-9 || (b - far).length() < 1e-9;
                assert_eq!(hidden, at_far, "{projection:?}: {a:?}-{b:?}");
            }
        }
    }

    #[test]
    fn straight_from_the_front_the_back_edges_are_hidden() {
        // Looking along +Y in orthographic: the back face (y = 10) is behind the front
        // face. Its top and bottom edges (along X at y = 10) are hidden behind it; the
        // front face's own edges are visible.
        let (scene, _) = box_scene();
        let mut cam = Camera {
            target: Point3::new(5.0, 5.0, 5.0),
            distance: 60.0,
            projection: Projection::Orthographic,
            ..Camera::default()
        };
        cam.set_view(StandardView::Front);
        for z in [2.0, 8.0] {
            assert!(point_hidden(&scene, &cam, Point3::new(5.0, 10.0, z)));
            assert!(!point_hidden(&scene, &cam, Point3::new(5.0, 0.0, z)));
        }
    }

    fn area(m: &TriMesh) -> f64 {
        m.area()
    }

    #[test]
    fn section_clipping_keeps_exactly_the_part_on_the_kept_side() {
        // Box 0..10, plane x = 4 removing x > 4. Kept: the x = 0 face (10 x 10 = 100) and
        // 4 bands of the side faces, each 4 x 10 = 40, so 100 + 160 = 260 mm^2. The x = 10
        // face is gone.
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0));
        let s = Section::new(Point3::new(4.0, 0.0, 0.0), Vec3::X).unwrap();
        let (kept, from) = s.clip_mesh(&m);
        assert!((area(&kept) - 260.0).abs() < 1e-9, "{}", area(&kept));
        assert_eq!(kept.triangles.len(), from.len());
        for p in &kept.positions {
            assert!(p.x <= 4.0 + 1e-9, "{p:?}");
        }
        // Every kept triangle lies in the plane of the triangle it came from.
        for (i, src) in from.iter().enumerate() {
            let n = m.triangle_normal(*src).unwrap();
            let k = kept.triangle_normal(i).unwrap();
            assert!((n - k).length() < 1e-9);
        }
        // Triangles wholly on the removed side never appear.
        for (i, _) in m.triangles.iter().enumerate() {
            let t = m.triangle(i).unwrap();
            if t.iter().all(|p| p.x > 4.0) {
                assert!(!from.contains(&i));
            }
        }
        // A plane that keeps everything returns the whole mesh, one for one.
        let all = Section::new(Point3::new(20.0, 0.0, 0.0), Vec3::X).unwrap();
        let (whole, from) = all.clip_mesh(&m);
        assert_eq!(whole.triangles.len(), m.triangles.len());
        assert_eq!(from, (0..m.triangles.len()).collect::<Vec<_>>());
        assert!(Section::new(Point3::ORIGIN, Vec3::new(0.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn transparent_bodies_are_drawn_back_to_front_after_opaque_ones() {
        // Eye on the -Y side (front view): the body at y = 100 is farther than y = 0.
        let mut s = Scene::default();
        let cube =
            |y: f64| TriMesh::cuboid(Point3::new(0.0, y, 0.0), Point3::new(10.0, y + 10.0, 10.0));
        let near = s.add("near", cube(0.0)).unwrap();
        let solid = s.add("solid", cube(50.0)).unwrap();
        let far = s.add("far", cube(100.0)).unwrap();
        for id in [near, far] {
            s.body_mut(id).unwrap().appearance.opacity = 0.3;
        }
        let mut cam = Camera::default();
        cam.set_view(StandardView::Front);
        let (opaque, transparent) = draw_order(&s, &cam);
        assert_eq!(opaque, vec![solid]);
        assert_eq!(transparent, vec![far, near]);
        // From the back the order flips.
        cam.set_view(StandardView::Back);
        assert_eq!(draw_order(&s, &cam).1, vec![near, far]);
        // Opacity 1 is opaque; hostile opacities are repaired.
        let a = Appearance {
            opacity: f32::NAN,
            ..Appearance::default()
        };
        assert!(!a.is_transparent());
        let a = Appearance {
            opacity: -3.0,
            ..Appearance::default()
        };
        assert_eq!(a.opacity(), MIN_OPACITY);
    }

    #[test]
    fn the_glass_preset_is_thirty_percent_opaque() {
        let g = preset_by_id("glass").unwrap();
        assert!((g.appearance.opacity - 0.3).abs() < 1e-6);
        assert!(g.appearance.is_transparent());
        assert!(preset_by_id("unobtainium").is_none());
        // Glass is fully transmissive in the realistic style.
        assert!(g.appearance.material(Rgba(0, 0, 0, 255)).is_glass());
        assert!(presets().count() >= 20);
    }
}

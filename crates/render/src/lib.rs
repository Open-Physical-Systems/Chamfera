//! Layer 1: camera math and the headless scene description.
//!
//! No GPU code lives here. The scene is plain data that the UI draws, and that the snapshot
//! renderer draws without a window. Z is up; lengths are mm, angles radians.

pub use chamfera_geom as geom;

pub mod display;
mod theme;
pub use display::{Appearance, Display, Section, VisualStyle};
pub use theme::{Rgba, Theme, ThemeMode};

use chamfera_geom::{Aabb, Point3, Ray, TriMesh, Vec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

/// A column-major 4x4 matrix in the layout WGSL `mat4x4<f32>` expects.
pub type Mat4 = [[f32; 4]; 4];

/// The camera never looks exactly straight up or down, so "right" stays defined.
pub const PITCH_LIMIT: f64 = FRAC_PI_2 - 1e-5;
pub const MIN_DISTANCE: f64 = 1e-3;
pub const MAX_DISTANCE: f64 = 1e7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    Perspective,
    Orthographic,
}

/// The standard views. Front looks along +Y, so +X is to the right and +Z up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Iso,
}

impl StandardView {
    pub fn parse(s: &str) -> Option<StandardView> {
        Some(match s {
            "front" => StandardView::Front,
            "back" => StandardView::Back,
            "left" => StandardView::Left,
            "right" => StandardView::Right,
            "top" => StandardView::Top,
            "bottom" => StandardView::Bottom,
            "iso" => StandardView::Iso,
            _ => return None,
        })
    }

    /// `(yaw, pitch)` of the eye seen from the target.
    pub fn angles(self) -> (f64, f64) {
        match self {
            StandardView::Front => (-FRAC_PI_2, 0.0),
            StandardView::Back => (FRAC_PI_2, 0.0),
            StandardView::Right => (0.0, 0.0),
            StandardView::Left => (std::f64::consts::PI, 0.0),
            StandardView::Top => (-FRAC_PI_2, PITCH_LIMIT),
            StandardView::Bottom => (-FRAC_PI_2, -PITCH_LIMIT),
            // Eye towards +X, -Y, +Z: the classic isometric direction (1, -1, 1).
            StandardView::Iso => (-FRAC_PI_4, (1.0 / 2f64.sqrt()).atan()),
        }
    }
}

/// An orbit camera: it looks at `target` from `distance` away, in the direction set by
/// `yaw` (around +Z, from +X) and `pitch` (up from the XY plane).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub target: Point3,
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    /// Vertical field of view (perspective) in radians.
    pub fov_y: f64,
    pub projection: Projection,
}

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = StandardView::Iso.angles();
        Camera {
            target: Point3::ORIGIN,
            distance: 250.0,
            yaw,
            pitch,
            fov_y: 35f64.to_radians(),
            projection: Projection::Perspective,
        }
    }
}

/// Camera axes: where it looks, and which way is right and up on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Basis {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

impl Camera {
    /// Unit vector from the target to the eye.
    fn eye_direction(&self) -> Vec3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        Vec3::new(cp * self.yaw.cos(), cp * self.yaw.sin(), sp)
    }

    pub fn eye(&self) -> Point3 {
        self.target + self.eye_direction() * self.distance
    }

    pub fn basis(&self) -> Basis {
        let forward = -self.eye_direction();
        // The pitch limit keeps forward off the Z axis, so this cross product has length.
        let right = forward.cross(Vec3::Z).normalized().unwrap_or(Vec3::new(
            -self.yaw.sin(),
            self.yaw.cos(),
            0.0,
        ));
        let up = right.cross(forward);
        Basis { forward, right, up }
    }

    /// Half the visible height at the target (mm); also the orthographic half height.
    pub fn half_height(&self) -> f64 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// Keeps the camera in its valid range after any change.
    pub fn sanitize(&mut self) {
        if !self.target.is_valid() {
            self.target = Point3::ORIGIN;
        }
        if !self.distance.is_finite() {
            self.distance = Camera::default().distance;
        }
        self.distance = self.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        if !self.yaw.is_finite() {
            self.yaw = 0.0;
        }
        self.yaw = self.yaw.rem_euclid(std::f64::consts::TAU);
        if !self.pitch.is_finite() {
            self.pitch = 0.0;
        }
        self.pitch = self.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        if !self.fov_y.is_finite() {
            self.fov_y = Camera::default().fov_y;
        }
        self.fov_y = self.fov_y.clamp(5f64.to_radians(), 120f64.to_radians());
    }

    pub fn orbit(&mut self, d_yaw: f64, d_pitch: f64) {
        self.yaw += d_yaw;
        self.pitch += d_pitch;
        self.sanitize();
    }

    /// Moves the view by a screen offset in pixels (`dx` right, `dy` down) for a viewport
    /// `viewport_height` pixels tall, so the model follows the cursor.
    pub fn pan_pixels(&mut self, dx: f64, dy: f64, viewport_height: f64) {
        if !(viewport_height.is_finite() && viewport_height >= 1.0) {
            return;
        }
        let per_px = 2.0 * self.half_height() / viewport_height;
        let b = self.basis();
        self.target = self.target + b.right * (-dx * per_px) + b.up * (dy * per_px);
        self.sanitize();
    }

    /// Scales the distance by `factor` (< 1 zooms in). With `toward`, that point stays
    /// where it is on screen.
    pub fn zoom(&mut self, factor: f64, toward: Option<Point3>) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let new_distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let f = new_distance / self.distance;
        if let Some(p) = toward.filter(|p| p.is_valid()) {
            self.target = p + (self.target - p) * f;
        }
        self.distance = new_distance;
        self.sanitize();
    }

    pub fn set_view(&mut self, view: StandardView) {
        let (yaw, pitch) = view.angles();
        self.yaw = yaw;
        self.pitch = pitch;
        self.sanitize();
    }

    /// Centres the box and sets the distance so all of it is visible at `aspect` (w / h).
    pub fn fit(&mut self, bounds: &Aabb, aspect: f64) {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let radius = bounds.radius().max(1.0);
        let half_v = self.fov_y * 0.5;
        let half_h = (half_v.tan() * aspect).atan();
        let half = half_v.min(half_h);
        self.target = bounds.center();
        self.distance = match self.projection {
            Projection::Perspective => radius / half.sin(),
            Projection::Orthographic => radius / half.tan(),
        } * 1.05;
        self.sanitize();
    }

    /// Near and far clip distances that hold the scene.
    pub fn clip_range(&self, scene: Option<&Aabb>) -> (f64, f64) {
        let reach = scene
            .map(|b| (b.center() - self.eye()).length() + b.radius())
            .unwrap_or(0.0)
            .max(self.distance * 2.0)
            * 1.5;
        match self.projection {
            Projection::Perspective => ((self.distance * 1e-3).max(reach * 1e-6), reach),
            Projection::Orthographic => (-reach, reach),
        }
    }

    pub fn view_matrix(&self) -> Mat4 {
        let b = self.basis();
        let e = self.eye() - Point3::ORIGIN;
        let (r, u, f) = (b.right, b.up, b.forward);
        to_f32([
            [r.x, u.x, -f.x, 0.0],
            [r.y, u.y, -f.y, 0.0],
            [r.z, u.z, -f.z, 0.0],
            [-r.dot(e), -u.dot(e), f.dot(e), 1.0],
        ])
    }

    /// Projection to WebGPU clip space (depth 0..1), right-handed.
    pub fn projection_matrix(&self, aspect: f64, scene: Option<&Aabb>) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let (near, far) = self.clip_range(scene);
        match self.projection {
            Projection::Perspective => {
                let f = 1.0 / (self.fov_y * 0.5).tan();
                to_f32([
                    [f / aspect, 0.0, 0.0, 0.0],
                    [0.0, f, 0.0, 0.0],
                    [0.0, 0.0, far / (near - far), -1.0],
                    [0.0, 0.0, near * far / (near - far), 0.0],
                ])
            }
            Projection::Orthographic => {
                let hh = self.half_height();
                let hw = hh * aspect;
                to_f32([
                    [1.0 / hw, 0.0, 0.0, 0.0],
                    [0.0, 1.0 / hh, 0.0, 0.0],
                    [0.0, 0.0, 1.0 / (near - far), 0.0],
                    [0.0, 0.0, near / (near - far), 1.0],
                ])
            }
        }
    }

    /// The ray through pixel `(px, py)` (origin top-left) of a `width` x `height` viewport.
    pub fn ray_through(
        &self,
        px: f64,
        py: f64,
        width: f64,
        height: f64,
        scene: Option<&Aabb>,
    ) -> Option<Ray> {
        if !(width >= 1.0 && height >= 1.0 && px.is_finite() && py.is_finite()) {
            return None;
        }
        let x = 2.0 * px / width - 1.0;
        let y = 1.0 - 2.0 * py / height;
        let b = self.basis();
        let hh = self.half_height();
        let hw = hh * width / height;
        match self.projection {
            Projection::Perspective => {
                let d = b.forward * self.distance + b.right * (x * hw) + b.up * (y * hh);
                Ray::new(self.eye(), d)
            }
            Projection::Orthographic => {
                let (near, _) = self.clip_range(scene);
                let origin = self.eye() + b.right * (x * hw) + b.up * (y * hh) + b.forward * near;
                Ray::new(origin, b.forward)
            }
        }
    }
}

fn to_f32(m: [[f64; 4]; 4]) -> Mat4 {
    m.map(|c| c.map(|v| v as f32))
}

/// Multiplies two column-major matrices: `a * b`.
pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0f32; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, cell) in col.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// Applies a column-major matrix to a point (w = 1) and divides by w.
pub fn transform_point(m: &Mat4, p: Point3) -> Option<[f64; 3]> {
    let v = [p.x as f32, p.y as f32, p.z as f32, 1.0];
    let mut o = [0f32; 4];
    for (r, cell) in o.iter_mut().enumerate() {
        *cell = (0..4).map(|k| m[k][r] * v[k]).sum();
    }
    if o[3].abs() < f32::EPSILON {
        return None;
    }
    Some([
        f64::from(o[0] / o[3]),
        f64::from(o[1] / o[3]),
        f64::from(o[2] / o[3]),
    ])
}

/// A body shown in the viewport. In M1 every body is a mesh body (imported or a sample).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneBody {
    pub id: u32,
    pub name: String,
    pub mesh: TriMesh,
    pub visible: bool,
    /// The exact B-rep edges as polylines (from the tessellator), drawn as the body's
    /// edges. Empty for mesh bodies, which show the mesh's sharp edges instead.
    #[serde(default)]
    pub edges: Vec<Vec<Point3>>,
    #[serde(default)]
    pub appearance: Appearance,
    /// A component tint: an index into the theme's `component_tints`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<u8>,
}

/// What a click picked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub body: u32,
    pub triangle: usize,
    pub point: Point3,
    pub distance: f64,
}

/// How overlay geometry is coloured (from the theme's sketch tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayStyle {
    Sketch,
    Constrained,
    Construction,
    /// Model edges projected into the sketch.
    Projected,
    Preview,
    Highlight,
    /// A measure result: the line between two closest points.
    Measure,
    /// Construction planes' outlines, axes and points.
    Construct,
    /// What the pointer is over, before a click (`theme.hover_highlight`).
    Hover,
    /// A toolpath's cutting moves.
    ToolpathFeed,
    /// A toolpath's rapid moves (the engine splits them into dashes).
    ToolpathRapid,
}

impl OverlayStyle {
    pub fn color(self, theme: &Theme) -> Rgba {
        match self {
            OverlayStyle::Sketch => theme.sketch,
            OverlayStyle::Constrained => theme.sketch_constrained,
            OverlayStyle::Construction => theme.sketch_construction,
            OverlayStyle::Projected => theme.sketch_projected,
            OverlayStyle::Preview => theme.sketch_preview,
            OverlayStyle::Highlight => theme.highlight,
            OverlayStyle::Measure => theme.measure,
            OverlayStyle::Construct => theme.construct,
            OverlayStyle::Hover => theme.hover_highlight,
            OverlayStyle::ToolpathFeed => theme.toolpath_feed,
            OverlayStyle::ToolpathRapid => theme.toolpath_rapid,
        }
    }
}

/// How a fill is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStyle {
    /// Construction planes, see-through.
    Construct,
}

impl FillStyle {
    pub fn color(self, theme: &Theme) -> Rgba {
        match self {
            FillStyle::Construct => theme.construct_fill,
        }
    }
}

/// See-through triangles drawn over the model, hidden by bodies in front and hiding
/// nothing (construction planes). Display only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub style: FillStyle,
    pub triangles: Vec<[Point3; 3]>,
}

/// Lines and points drawn over the model: sketches, previews and highlights. Display
/// only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub style: OverlayStyle,
    pub lines: Vec<[Point3; 2]>,
    pub points: Vec<Point3>,
    /// Drawn over everything (the sketch being edited, previews). When false, bodies in
    /// front hide it (finished sketches).
    #[serde(default = "yes")]
    pub on_top: bool,
}

fn yes() -> bool {
    true
}

/// Most overlay lines plus points one scene holds (hostile-input limit).
pub const MAX_OVERLAY_ITEMS: usize = 2_000_000;

/// Most bodies one scene holds (hostile-input limit).
pub const MAX_BODIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    pub bodies: Vec<SceneBody>,
    /// The last pick: where the pointer hit a selected body.
    pub selection: Option<Hit>,
    /// Every selected body, each once, in the order they were selected. The picked body
    /// (`selection`) is one of them.
    #[serde(default)]
    pub selected: Vec<u32>,
    /// Drawn over everything, in order.
    #[serde(default)]
    pub overlays: Vec<Overlay>,
    /// Shapes a tool would make (the extrude preview), drawn translucent in the theme's
    /// `preview_body` colour over the bodies. Not picked; display only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<TriMesh>,
    /// See-through fills (construction planes), drawn before the overlays.
    #[serde(default)]
    pub fills: Vec<Fill>,
    /// Visual style and section plane.
    #[serde(default)]
    pub display: Display,
    next_id: u32,
}

impl Scene {
    /// Adds a body and returns its id, or `None` when the scene is full.
    pub fn add(&mut self, name: &str, mesh: TriMesh) -> Option<u32> {
        if self.bodies.len() >= MAX_BODIES {
            return None;
        }
        self.next_id = self.next_id.checked_add(1)?;
        let id = self.next_id;
        self.bodies.push(SceneBody {
            id,
            name: name.to_owned(),
            mesh,
            visible: true,
            edges: Vec::new(),
            appearance: Appearance::default(),
            tint: None,
        });
        Some(id)
    }

    pub fn body(&self, id: u32) -> Option<&SceneBody> {
        self.bodies.iter().find(|b| b.id == id)
    }

    pub fn body_mut(&mut self, id: u32) -> Option<&mut SceneBody> {
        self.bodies.iter_mut().find(|b| b.id == id)
    }

    /// Bounds of the visible bodies, the previews and the overlays.
    pub fn bounds(&self) -> Option<Aabb> {
        let overlay_points = self
            .overlays
            .iter()
            .flat_map(|o| o.lines.iter().flatten().chain(o.points.iter()))
            .copied()
            .filter(|p| p.is_valid());
        let overlays = Aabb::from_points(overlay_points);
        self.bodies
            .iter()
            .filter(|b| b.visible)
            .filter_map(|b| b.mesh.bounds())
            .chain(self.previews.iter().filter_map(TriMesh::bounds))
            .chain(overlays)
            .reduce(Aabb::union)
    }

    /// Whether a body is selected (in the set, or the last pick).
    pub fn is_selected(&self, id: u32) -> bool {
        self.selected.contains(&id) || self.selection.is_some_and(|h| h.body == id)
    }

    /// Every selected body: the set, plus the picked body if it is missing from it.
    pub fn selected_bodies(&self) -> Vec<u32> {
        let mut ids = self.selected.clone();
        if let Some(h) = self.selection
            && !ids.contains(&h.body)
        {
            ids.push(h.body);
        }
        ids
    }

    /// Drops the bodies for which `gone` is true from the selection.
    pub fn deselect(&mut self, gone: impl Fn(u32) -> bool) {
        self.selected.retain(|id| !gone(*id));
        if self.selection.is_some_and(|h| gone(h.body)) {
            self.selection = None;
        }
    }

    /// Selects nothing.
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.selection = None;
    }

    /// The nearest visible body the ray hits. With a section plane, only the part on its
    /// kept side can be hit (what is cut away is not there to click).
    pub fn pick(&self, ray: &Ray) -> Option<Hit> {
        let section = self.display.section;
        self.bodies
            .iter()
            .filter(|b| b.visible)
            .filter_map(|b| {
                let (triangle, distance) = match section {
                    None => b.mesh.raycast(ray)?,
                    Some(s) => {
                        let (kept, from) = s.clip_mesh(&b.mesh);
                        let (t, d) = kept.raycast(ray)?;
                        (*from.get(t)?, d)
                    }
                };
                Some(Hit {
                    body: b.id,
                    triangle,
                    point: ray.at(distance),
                    distance,
                })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn front_view_looks_along_plus_y_with_z_up() {
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        let b = c.basis();
        assert!(close(b.forward, Vec3::Y));
        assert!(close(b.right, Vec3::X));
        assert!(close(b.up, Vec3::Z));
    }

    #[test]
    fn top_view_has_plus_y_up_on_screen() {
        let mut c = Camera::default();
        c.set_view(StandardView::Top);
        let b = c.basis();
        assert!((b.forward.z + 1.0).abs() < 1e-9);
        assert!((b.up.y - 1.0).abs() < 1e-4);
        assert!((b.right.x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn iso_eye_lies_on_the_one_minus_one_one_diagonal() {
        let c = Camera::default();
        let d = (c.eye() - c.target).normalized().unwrap();
        let s = 1.0 / 3f64.sqrt();
        assert!(close(d, Vec3::new(s, -s, s)));
    }

    #[test]
    fn the_target_projects_to_the_centre_of_the_screen() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let mut c = Camera {
                target: Point3::new(10.0, 20.0, 5.0),
                projection,
                ..Camera::default()
            };
            c.orbit(0.3, 0.2);
            let vp = mat_mul(&c.projection_matrix(1.5, None), &c.view_matrix());
            let ndc = transform_point(&vp, c.target).unwrap();
            assert!(
                ndc[0].abs() < 1e-5 && ndc[1].abs() < 1e-5,
                "{projection:?} {ndc:?}"
            );
            assert!((0.0..=1.0).contains(&ndc[2]), "depth {}", ndc[2]);
        }
    }

    #[test]
    fn the_centre_pixel_ray_goes_through_the_target() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let c = Camera {
                projection,
                ..Camera::default()
            };
            let ray = c.ray_through(400.0, 300.0, 800.0, 600.0, None).unwrap();
            let to_target = c.target - ray.origin;
            let off_axis = to_target.cross(ray.direction).length();
            assert!(off_axis < 1e-6, "{projection:?}: {off_axis}");
        }
    }

    #[test]
    fn a_corner_pixel_ray_reaches_the_projected_corner() {
        // The ray through a pixel and the projection of a point on it must agree.
        let c = Camera::default();
        let (w, h) = (800.0, 600.0);
        let ray = c.ray_through(0.0, 0.0, w, h, None).unwrap();
        let p = ray.at(c.distance);
        let vp = mat_mul(&c.projection_matrix(w / h, None), &c.view_matrix());
        let ndc = transform_point(&vp, p).unwrap();
        assert!(
            (ndc[0] + 1.0).abs() < 1e-4 && (ndc[1] - 1.0).abs() < 1e-4,
            "{ndc:?}"
        );
    }

    #[test]
    fn pitch_is_clamped_and_hostile_values_are_repaired() {
        let mut c = Camera::default();
        c.orbit(0.0, 10.0);
        assert!(c.pitch <= PITCH_LIMIT);
        c.orbit(f64::NAN, f64::INFINITY);
        c.zoom(f64::NAN, None);
        c.zoom(-1.0, None);
        c.pan_pixels(f64::NAN, 1.0, 0.0);
        assert!(c.yaw.is_finite() && c.pitch.is_finite() && c.distance > 0.0);
        c.zoom(1e30, None);
        assert_eq!(c.distance, MAX_DISTANCE);
    }

    #[test]
    fn zooming_toward_a_point_keeps_it_on_the_same_ray() {
        let mut c = Camera::default();
        let p = Point3::new(30.0, -10.0, 5.0);
        let before = (p - c.eye()).normalized().unwrap();
        c.zoom(0.5, Some(p));
        let after = (p - c.eye()).normalized().unwrap();
        assert!(close(before, after));
    }

    #[test]
    fn panning_moves_the_target_by_the_pixel_size() {
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        let per_px = 2.0 * c.half_height() / 600.0;
        c.pan_pixels(10.0, 0.0, 600.0);
        // Dragging right moves the model right, so the target moves left (-X).
        assert!((c.target.x + 10.0 * per_px).abs() < 1e-9);
    }

    #[test]
    fn fit_makes_the_whole_box_visible() {
        let bounds = Aabb {
            min: Point3::new(-100.0, -20.0, 0.0),
            max: Point3::new(300.0, 40.0, 80.0),
        };
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let mut c = Camera {
                projection,
                ..Camera::default()
            };
            c.fit(&bounds, 0.5);
            let vp = mat_mul(&c.projection_matrix(0.5, Some(&bounds)), &c.view_matrix());
            for x in [bounds.min.x, bounds.max.x] {
                for y in [bounds.min.y, bounds.max.y] {
                    for z in [bounds.min.z, bounds.max.z] {
                        let n = transform_point(&vp, Point3::new(x, y, z)).unwrap();
                        assert!(
                            n[0].abs() <= 1.0 && n[1].abs() <= 1.0,
                            "{projection:?} {n:?}"
                        );
                        assert!((0.0..=1.0).contains(&n[2]), "{projection:?} depth {n:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn picking_finds_the_nearest_visible_body() {
        let mut s = Scene::default();
        let near = s
            .add(
                "near",
                TriMesh::cuboid(Point3::new(0.0, 0.0, 10.0), Point3::new(10.0, 10.0, 20.0)),
            )
            .unwrap();
        let far = s
            .add(
                "far",
                TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 5.0)),
            )
            .unwrap();
        let ray = Ray::new(Point3::new(5.0, 5.0, 100.0), -Vec3::Z).unwrap();
        let hit = s.pick(&ray).unwrap();
        assert_eq!(hit.body, near);
        assert!((hit.point.z - 20.0).abs() < 1e-9);
        s.body_mut(near).unwrap().visible = false;
        assert_eq!(s.pick(&ray).unwrap().body, far);
    }

    #[test]
    fn a_section_lets_picks_through_the_cut_away_part() {
        // Box 0..10 cut at z = 5 (the top half removed): a ray down from above passes the
        // removed top face and hits the far side of the kept half, the bottom face z = 0
        // (seen from inside). Without the section it hits the top at z = 10.
        let mut s = Scene::default();
        let id = s
            .add(
                "box",
                TriMesh::cuboid(Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0)),
            )
            .unwrap();
        let ray = Ray::new(Point3::new(3.0, 4.0, 100.0), -Vec3::Z).unwrap();
        assert!((s.pick(&ray).unwrap().point.z - 10.0).abs() < 1e-9);
        s.display.section = Section::new(Point3::new(0.0, 0.0, 5.0), Vec3::Z);
        let hit = s.pick(&ray).unwrap();
        assert_eq!(hit.body, id);
        assert!(hit.point.z.abs() < 1e-9, "{hit:?}");
        // The triangle named is one of the mesh's own bottom triangles.
        let n = s
            .body(id)
            .unwrap()
            .mesh
            .triangle_normal(hit.triangle)
            .unwrap();
        assert!((n.z + 1.0).abs() < 1e-9);
    }
}

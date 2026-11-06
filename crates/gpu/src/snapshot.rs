//! Rendering a frame without a window: for `chamfera-cli snapshot`, tests and agents.

use crate::{Frame, MAX_SIZE, ViewRenderer};
use std::time::Duration;

/// How long to wait for the GPU before giving up.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum SnapshotError {
    NoAdapter(String),
    Device(String),
    Render(String),
    Size,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::NoAdapter(e) => write!(f, "no graphics adapter: {e}"),
            SnapshotError::Device(e) => write!(f, "could not open the graphics device: {e}"),
            SnapshotError::Render(e) => write!(f, "rendering failed: {e}"),
            SnapshotError::Size => write!(f, "width and height must be 1 to {MAX_SIZE} pixels"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Opens a GPU without a window: the system's best adapter, else a software one
/// (lavapipe/llvmpipe), so this also works on CI machines without a GPU.
fn open() -> Result<(wgpu::Device, wgpu::Queue), SnapshotError> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .or_else(|_| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: true,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
    })
    .map_err(|e| SnapshotError::NoAdapter(e.to_string()))?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("chamfera snapshot"),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| SnapshotError::Device(e.to_string()))
}

/// Renders a frame and returns its pixels as tightly packed sRGB RGBA rows, top row first.
pub fn snapshot_rgba(frame: &Frame) -> Result<Vec<u8>, SnapshotError> {
    if !(1..=MAX_SIZE).contains(&frame.width) || !(1..=MAX_SIZE).contains(&frame.height) {
        return Err(SnapshotError::Size);
    }
    let (device, queue) = open()?;
    let mut renderer = ViewRenderer::new(&device);
    let draw = renderer.render(&device, &queue, frame);
    let (w, h) = (frame.width, frame.height);
    let row = w * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot read-back"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let (texture, _) = renderer
        .output()
        .ok_or_else(|| SnapshotError::Render("nothing was drawn".into()))?;
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([draw, encoder.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(TIMEOUT),
        })
        .map_err(|e| SnapshotError::Render(format!("waiting for the picture: {e}")))?;
    rx.recv_timeout(TIMEOUT)
        .map_err(|e| SnapshotError::Render(e.to_string()))?
        .map_err(|e| SnapshotError::Render(e.to_string()))?;
    let data = slice
        .get_mapped_range()
        .map_err(|e| SnapshotError::Render(format!("reading the picture back: {e}")))?;
    let mut rgba = Vec::with_capacity((row * h) as usize);
    for r in data.chunks(padded as usize).take(h as usize) {
        rgba.extend_from_slice(r.get(..row as usize).unwrap_or_default());
    }
    if rgba.len() != (row * h) as usize {
        return Err(SnapshotError::Render("the picture read back short".into()));
    }
    Ok(rgba)
}

/// Renders a frame as PNG file bytes.
pub fn snapshot_png(frame: &Frame) -> Result<Vec<u8>, SnapshotError> {
    let rgba = snapshot_rgba(frame)?;
    encode_png(frame.width, frame.height, &rgba)
}

pub(crate) fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = enc
            .write_header()
            .map_err(|e| SnapshotError::Render(format!("PNG: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| SnapshotError::Render(format!("PNG: {e}")))?;
    }
    Ok(out)
}

/// GPU tests skip on machines without any adapter, unless `CHAMFERA_REQUIRE_GPU=1`
/// (set in CI, which installs a software adapter), where a missing GPU is a failure.
#[cfg(test)]
pub(crate) fn skip_without_gpu(reason: &str) {
    if std::env::var("CHAMFERA_REQUIRE_GPU").is_ok_and(|v| v == "1") {
        panic!("CHAMFERA_REQUIRE_GPU=1 but no GPU: {reason}");
    }
    eprintln!("skipped (no GPU): {reason}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::{Point3, TriMesh};
    use chamfera_render::{Camera, Scene, StandardView, Theme};

    /// Renders a box from the front and checks pixels: the centre is the body colour
    /// (shaded, so close to but not the background) and a corner is the background.
    /// Skipped when the machine has no GPU and no software adapter.
    #[test]
    fn a_front_view_of_a_box_draws_the_box_in_the_middle() {
        let mut scene = Scene::default();
        scene.add(
            "box",
            TriMesh::cuboid(
                Point3::new(-10.0, -10.0, -10.0),
                Point3::new(10.0, 10.0, 10.0),
            ),
        );
        let mut camera = Camera::default();
        camera.set_view(StandardView::Front);
        camera.distance = 100.0;
        let theme = Theme::light();
        let frame = Frame {
            scene: &scene,
            camera: &camera,
            theme: &theme,
            width: 64,
            height: 48,
            scale: 1.0,
        };
        let rgba = match snapshot_rgba(&frame) {
            Ok(p) => p,
            Err(SnapshotError::NoAdapter(e)) | Err(SnapshotError::Device(e)) => {
                skip_without_gpu(&e);
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let px = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        let centre = px(32, 24);
        let corner = px(1, 1);
        // The background's top colour is light (above 220); the box face is grey and lit
        // straight on, darker than that.
        assert!(corner.iter().all(|&c| c > 220), "corner {corner:?}");
        assert!(centre.iter().all(|&c| c < 220), "centre {centre:?}");
        let png = encode_png(64, 48, &rgba).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use chamfera_geom::{Point3, TriMesh};
    use chamfera_render::{Camera, Overlay, OverlayStyle, Scene, StandardView, Theme};

    /// A sketch line and point placed behind a box stay visible (overlays are never hidden
    /// by bodies) and take the theme's sketch colour.
    ///
    /// Front view, distance 100, fov 35 degrees: the view is 2 * 100 * tan(17.5) = 63.1 mm
    /// tall over 48 px, so 0.761 px per mm, centre at (32, 24). The box (x -10..10) covers
    /// columns 24.4..39.6. The line at z = 0.7 mm sits on row 24 - 0.53 = 23.47, inside
    /// row 23 rather than on a row boundary. The point at x = -6, z = 6 lands at column
    /// 27.4 and row 19.4, a 7 px square.
    #[test]
    fn overlays_draw_over_bodies_in_the_sketch_colour() {
        let mut scene = Scene::default();
        scene.add(
            "box",
            TriMesh::cuboid(
                Point3::new(-10.0, -10.0, -10.0),
                Point3::new(10.0, 10.0, 10.0),
            ),
        );
        scene.overlays.push(Overlay {
            style: OverlayStyle::Sketch,
            lines: vec![[Point3::new(-30.0, 20.0, 0.7), Point3::new(30.0, 20.0, 0.7)]],
            points: vec![Point3::new(-6.0, 20.0, 6.0)],
            on_top: true,
        });
        let mut camera = Camera::default();
        camera.set_view(StandardView::Front);
        camera.distance = 100.0;
        let theme = Theme::light();
        let (w, h) = (64u32, 48u32);
        let frame = Frame {
            scene: &scene,
            camera: &camera,
            theme: &theme,
            width: w,
            height: h,
            scale: 1.0,
        };
        let rgba = match snapshot_rgba(&frame) {
            Ok(p) => p,
            Err(SnapshotError::NoAdapter(e)) | Err(SnapshotError::Device(e)) => {
                skip_without_gpu(&e);
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        let s = theme.sketch;
        let is_sketch = |c: [u8; 3]| {
            (i32::from(c[0]) - i32::from(s.0)).abs() <= 8
                && (i32::from(c[1]) - i32::from(s.1)).abs() <= 8
                && (i32::from(c[2]) - i32::from(s.2)).abs() <= 8
        };
        // Column 36 is on the box face, away from the point.
        assert!(is_sketch(px(36, 23)), "line pixel {:?}", px(36, 23));
        assert!(
            !is_sketch(px(36, 28)),
            "the box face is not sketch coloured"
        );
        assert!(is_sketch(px(27, 19)), "point pixel {:?}", px(27, 19));
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    use chamfera_geom::{Point3, TriMesh, Vec3};
    use chamfera_render::{Camera, Rgba, Scene, Section, StandardView, Theme, VisualStyle};

    const W: u32 = 64;
    const H: u32 = 48;

    /// The box -10..10 seen from the front (eye on -Y), distance 100: 0.761 px per mm
    /// (see `overlays_draw_over_bodies_in_the_sketch_colour`), so it covers columns
    /// 24.4..39.6 and rows 16.4..31.6 around the centre (32, 24). Row 24 holds the X axis
    /// line, so pixels are checked on row 20.
    fn render(scene: &Scene) -> Option<Vec<u8>> {
        let mut camera = Camera::default();
        camera.set_view(StandardView::Front);
        camera.distance = 100.0;
        let theme = Theme::light();
        let frame = Frame {
            scene,
            camera: &camera,
            theme: &theme,
            width: W,
            height: H,
            scale: 1.0,
        };
        match snapshot_rgba(&frame) {
            Ok(p) => Some(p),
            Err(SnapshotError::NoAdapter(e)) | Err(SnapshotError::Device(e)) => {
                skip_without_gpu(&e);
                None
            }
            Err(e) => panic!("{e}"),
        }
    }

    fn box_scene() -> (Scene, u32) {
        let mut scene = Scene::default();
        let id = scene
            .add(
                "box",
                TriMesh::cuboid(
                    Point3::new(-10.0, -10.0, -10.0),
                    Point3::new(10.0, 10.0, 10.0),
                ),
            )
            .unwrap();
        (scene, id)
    }

    fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 3] {
        let i = ((y * W + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    fn near(c: [u8; 3], t: Rgba, tol: i32) -> bool {
        (i32::from(c[0]) - i32::from(t.0)).abs() <= tol
            && (i32::from(c[1]) - i32::from(t.1)).abs() <= tol
            && (i32::from(c[2]) - i32::from(t.2)).abs() <= tol
    }

    /// Cut at y = 0 removing the front half (normal -Y, towards the eye): the view looks
    /// into the cut and sees the inside of the back half, drawn as the flat cap colour.
    /// Row 24 holds the X axis (y = z = 0), so the test looks at row 20. Pixel (35, 20):
    /// 35 + 20 = 55, 55 mod 10 = 5, off the hatch stripes (0 and 1), so it is exactly
    /// `section_cap`; (30, 20): 50 mod 10 = 0, a hatch stripe.
    #[test]
    fn a_sectioned_box_shows_its_cut_face_in_the_cap_colour() {
        let theme = Theme::light();
        let (mut scene, _) = box_scene();
        let Some(plain) = render(&scene) else {
            return;
        };
        assert!(!near(px(&plain, 35, 20), theme.section_cap, 24));
        scene.display.section = Section::new(Point3::ORIGIN, -Vec3::Y);
        let Some(cut) = render(&scene) else {
            return;
        };
        assert!(
            near(px(&cut, 35, 20), theme.section_cap, 2),
            "cap pixel {:?}",
            px(&cut, 35, 20)
        );
        assert!(
            near(px(&cut, 30, 20), theme.section_hatch, 2),
            "hatch pixel {:?}",
            px(&cut, 30, 20)
        );
        // Outside the box the background is untouched.
        assert!(px(&cut, 1, 1).iter().all(|&c| c > 220));
    }

    /// Wireframe draws no faces: inside the box outline the background shows (its light
    /// colour, above 220), where the shaded style shows the grey body.
    #[test]
    fn wireframe_shows_the_background_through_the_faces() {
        let (mut scene, _) = box_scene();
        scene.display.style = VisualStyle::Wireframe;
        let Some(rgba) = render(&scene) else {
            return;
        };
        let c = px(&rgba, 35, 20);
        assert!(c.iter().all(|&v| v > 220), "{c:?}");
    }

    /// A black body at opacity 0.3 over the light background blends to 70% of the
    /// background in linear light, so it is darker than the background but far lighter
    /// than the opaque black body.
    #[test]
    fn a_glass_body_lets_the_background_through() {
        let (mut scene, id) = box_scene();
        if let Some(b) = scene.body_mut(id) {
            b.appearance.colour = Some(Rgba(0, 0, 0, 255));
        }
        let Some(opaque) = render(&scene) else {
            return;
        };
        if let Some(b) = scene.body_mut(id) {
            b.appearance.opacity = 0.3;
        }
        let Some(glass) = render(&scene) else {
            return;
        };
        let (o, g, bg) = (px(&opaque, 35, 20), px(&glass, 35, 20), px(&glass, 1, 20));
        // The opaque black face only has its specular highlight on it (0.18 of white at
        // most, about 117 in sRGB).
        assert!(o.iter().all(|&v| v < 120), "opaque {o:?}");
        assert!(g.iter().all(|&v| v > 170), "glass {g:?}");
        assert!(g[0] < bg[0], "glass {g:?} background {bg:?}");
    }
}

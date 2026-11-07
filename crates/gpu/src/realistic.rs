//! The realistic visual style: physically based materials in the procedural environment
//! (`chamfera_render::environment`), a floor that fades into the sky and a shadow map from
//! the key light. Shaders in `realistic.wgsl`.

use crate::{COLOR_FORMAT, DEPTH_FORMAT, SAMPLES};
use chamfera_geom::{Aabb, Point3, Vec3};
use chamfera_render::{Camera, Environment, Mat4, Projection, Scene, mat_mul};

/// Side of the shadow map in texels.
pub const SHADOW_SIZE: u32 = 2048;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The realistic style's part of the globals uniform (see `Globals` in `lib.rs`).
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RealisticGlobals {
    pub cam_right: [f32; 4],
    pub cam_up: [f32; 4],
    pub sky_zenith: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub sky_nadir: [f32; 4],
    pub floor_albedo: [f32; 4],
    pub floor_geom: [f32; 4],
    pub lights: [[f32; 4]; 6],
    pub shadow_view_proj: [[f32; 4]; 4],
    pub shadow: [f32; 4],
}

/// The bounds of the visible bodies (what the floor and the shadow map are sized to).
pub fn body_bounds(scene: &Scene) -> Option<Aabb> {
    scene
        .bodies
        .iter()
        .filter(|b| b.visible)
        .filter_map(|b| b.mesh.bounds())
        .reduce(Aabb::union)
}

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x as f32, v.y as f32, v.z as f32, w]
}

/// A view matrix looking along `forward` from `eye` (as `Camera::view_matrix`).
fn look(eye: Point3, forward: Vec3) -> Option<Mat4> {
    let f = forward.normalized()?;
    let r = f
        .cross(Vec3::Z)
        .normalized()
        .or_else(|| f.cross(Vec3::X).normalized())?;
    let u = r.cross(f);
    let e = eye - Point3::ORIGIN;
    let m = [
        [r.x, u.x, -f.x, 0.0],
        [r.y, u.y, -f.y, 0.0],
        [r.z, u.z, -f.z, 0.0],
        [-r.dot(e), -u.dot(e), f.dot(e), 1.0],
    ];
    Some(m.map(|c| c.map(|v| v as f32)))
}

/// An orthographic projection to WebGPU clip space (as `Camera::projection_matrix`).
fn ortho(half: f64, near: f64, far: f64) -> Mat4 {
    let m = [
        [1.0 / half, 0.0, 0.0, 0.0],
        [0.0, 1.0 / half, 0.0, 0.0],
        [0.0, 0.0, 1.0 / (near - far), 0.0],
        [0.0, 0.0, near / (near - far), 1.0],
    ];
    m.map(|c| c.map(|v| v as f32))
}

/// The world-to-shadow-map transform of a light towards `towards` over the box.
pub fn shadow_matrix(bounds: &Aabb, towards: Vec3) -> Option<Mat4> {
    let r = bounds.radius().max(1e-3);
    let c = bounds.center();
    let l = towards.normalized()?;
    let eye = c + l * (r * 2.0);
    // Wider than the part, so long shadows on the floor stay on the map.
    Some(mat_mul(&ortho(r * 1.6, r * 0.5, r * 3.5), &look(eye, -l)?))
}

/// The realistic globals for a frame.
pub fn globals(scene: &Scene, camera: &Camera, aspect: f64) -> RealisticGlobals {
    let env = Environment::preset(scene.display.environment, camera);
    let b = camera.basis();
    let tan = (camera.fov_y * 0.5).tan();
    let rgb = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
    let bounds = body_bounds(scene);
    let (centre, radius, floor_z) = match &bounds {
        Some(bb) => {
            let r = bb.radius().max(1e-3);
            (bb.center(), r, bb.min.z - r * 1e-4)
        }
        None => (Point3::ORIGIN, 50.0, 0.0),
    };
    let mut lights = [[0.0; 4]; 6];
    for (i, l) in env.lights.iter().take(3).enumerate() {
        let sin = (1.0 - l.cos_max * l.cos_max).max(0.0).sqrt() as f32;
        lights[2 * i] = v4(l.direction, sin);
        lights[2 * i + 1] = rgb(l.irradiance(), 1.0);
    }
    let shadow = bounds
        .as_ref()
        .zip(env.lights.first())
        .and_then(|(bb, key)| shadow_matrix(bb, key.direction));
    let ortho_view = camera.projection == Projection::Orthographic;
    RealisticGlobals {
        cam_right: v4(
            b.right,
            if ortho_view {
                0.0
            } else {
                (tan * aspect) as f32
            },
        ),
        cam_up: v4(b.up, if ortho_view { 0.0 } else { tan as f32 }),
        sky_zenith: rgb(env.zenith, env.exposure),
        sky_horizon: rgb(env.horizon, 0.0),
        sky_nadir: rgb(env.nadir, 0.0),
        floor_albedo: match env.floor {
            Some(f) => rgb(f, 1.0),
            None => [0.0; 4],
        },
        floor_geom: [
            centre.x as f32,
            centre.y as f32,
            floor_z as f32,
            radius as f32,
        ],
        lights,
        shadow_view_proj: shadow.unwrap_or([[0.0; 4]; 4]),
        shadow: [
            SHADOW_SIZE as f32,
            0.002,
            if shadow.is_some() { 1.0 } else { 0.0 },
            0.0,
        ],
    }
}

pub struct Realistic {
    pub sky: wgpu::RenderPipeline,
    pub floor: wgpu::RenderPipeline,
    pub mesh: wgpu::RenderPipeline,
    pub glass: wgpu::RenderPipeline,
    pub shadow: wgpu::RenderPipeline,
    pub shadow_view: wgpu::TextureView,
    pub shadow_bind: wgpu::BindGroup,
    /// Bound as the body group for the floor, which reads no body.
    pub no_body: wgpu::BindGroup,
}

impl Realistic {
    pub fn new(
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        globals_layout: &wgpu::BindGroupLayout,
        body_layout: &wgpu::BindGroupLayout,
        mesh_stride: u64,
    ) -> Realistic {
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow map"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow map"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let no_body_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("no body"),
            size: crate::BODY_UNIFORM,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let no_body = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("no body"),
            layout: body_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: no_body_buf.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("realistic"),
            bind_group_layouts: &[
                Some(globals_layout),
                Some(body_layout),
                Some(&shadow_layout),
            ],
            immediate_size: 0,
        });
        let globals_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow pass"),
            bind_group_layouts: &[Some(globals_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: mesh_stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attrs,
        })];
        let position = wgpu::vertex_attr_array![0 => Float32x3];
        let position_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: mesh_stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &position,
        })];
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    depth_write: bool,
                    compare: wgpu::CompareFunction,
                    blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    buffers,
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, Less, LessEqual};
        let sky = make("sky", "sky_vs", "sky_fs", &[], false, Always, None);
        let floor = make("floor", "floor_vs", "floor_fs", &[], true, Less, None);
        let mesh = make(
            "realistic mesh",
            "realistic_vs",
            "realistic_fs",
            &mesh_buffers,
            true,
            Less,
            None,
        );
        let glass = make(
            "realistic glass",
            "realistic_vs",
            "realistic_fs",
            &mesh_buffers,
            false,
            LessEqual,
            // Premultiplied: the reflection adds in full, what is behind is dimmed by
            // alpha (the light the glass does not pass).
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pass"),
            layout: Some(&globals_only),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("shadow_vs"),
                buffers: &position_buffers,
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        Realistic {
            sky,
            floor,
            mesh,
            glass,
            shadow,
            shadow_view,
            shadow_bind,
            no_body,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_render::transform_point;

    #[test]
    fn the_shadow_map_holds_the_whole_box_and_sees_it_from_the_light() {
        // A box 0..10 on every axis, light from straight above: the box's corners land
        // inside clip space (x, y in -1..1, depth in 0..1), and the top is nearer the
        // light (smaller depth) than the bottom.
        let bb = Aabb::from_points([Point3::ORIGIN, Point3::new(10.0, 10.0, 10.0)]).unwrap();
        let m = shadow_matrix(&bb, Vec3::Z).unwrap();
        for x in [0.0, 10.0] {
            for y in [0.0, 10.0] {
                for z in [0.0, 10.0] {
                    let p = transform_point(&m, Point3::new(x, y, z)).unwrap();
                    assert!(p[0].abs() <= 1.0 && p[1].abs() <= 1.0, "{p:?}");
                    assert!((0.0..=1.0).contains(&p[2]), "{p:?}");
                }
            }
        }
        let top = transform_point(&m, Point3::new(5.0, 5.0, 10.0)).unwrap();
        let bottom = transform_point(&m, Point3::new(5.0, 5.0, 0.0)).unwrap();
        assert!(top[2] < bottom[2]);
        assert!(shadow_matrix(&bb, Vec3::ZERO).is_none());
    }
}

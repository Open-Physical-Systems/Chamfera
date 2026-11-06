//! Layer 5: the GPU viewport renderer (wgpu).
//!
//! It draws a [`Frame`] (scene, camera, theme, size) into its own texture. The desktop and
//! browser UI show that texture; [`snapshot`] draws the same frame without a window, so the
//! CLI and tests see exactly what users see.

use chamfera_geom::{Point3, TriMesh};
use chamfera_render::display::{EdgePass, FacePass, draw_order};
use chamfera_render::{Camera, MAX_OVERLAY_ITEMS, Projection, Scene, SceneBody, Theme, mat_mul};
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
mod snapshot;
#[cfg(not(target_arch = "wasm32"))]
pub use snapshot::{SnapshotError, snapshot_png, snapshot_rgba};

pub use wgpu;

/// The colour format of the rendered view (sRGB, so the GPU converts from linear light).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Edges are drawn where faces meet at more than this angle (radians, 30 degrees).
pub const EDGE_ANGLE: f64 = std::f64::consts::PI / 6.0;
/// Largest view size (pixels per side).
pub const MAX_SIZE: u32 = 8192;
/// Overlay point size in logical pixels (multiplied by [`Frame::scale`]).
pub const POINT_SIZE: f32 = 7.0;
/// Dash period of hidden edges, in logical pixels (half dash, half gap).
pub const DASH_PIXELS: f32 = 8.0;
/// Hatch period on section caps, in physical pixels; the stripe is
/// [`HATCH_STRIPE_PIXELS`] of it. The stripes run at 45 degrees: a pixel `(x, y)` is
/// striped when `(x + y) mod HATCH_PIXELS < HATCH_STRIPE_PIXELS`.
pub const HATCH_PIXELS: u32 = 10;
pub const HATCH_STRIPE_PIXELS: u32 = 2;

/// What to draw.
pub struct Frame<'a> {
    pub scene: &'a Scene,
    pub camera: &'a Camera,
    pub theme: &'a Theme,
    pub width: u32,
    pub height: u32,
    /// Physical pixels per logical pixel (the UI's scale; 1 for snapshots).
    pub scale: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    forward: [f32; 4],
    background_top: [f32; 4],
    background_bottom: [f32; 4],
    viewport: [f32; 4],
    /// The section plane: xyz its normal (towards the removed side), w minus its distance
    /// from the origin, so `dot(normal, p) + w > 0` is cut away.
    section: [f32; 4],
    /// x: 1 with a section; y: pixels per mm at the target (hidden-edge dashes); z: the
    /// dash period in pixels; w: the hatch period in pixels.
    display: [f32; 4],
    section_cap: [f32; 4],
    section_hatch: [f32; 4],
    edge_hidden: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
    /// Length along the edge from its start (mm), for dashes; 0 for other lines.
    arc: f32,
}

struct GpuBody {
    /// Triangle, vertex and edge counts of the body the buffers were built from.
    key: (usize, usize, usize),
    triangles: wgpu::Buffer,
    triangle_vertices: u32,
    edges: wgpu::Buffer,
    edge_vertices: u32,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same pixels seen as plain `Rgba8Unorm`, for UIs that composite in sRGB space
    /// and would otherwise decode the colours twice.
    raw_view: wgpu::TextureView,
    /// Where the device cannot view a texture in another format (wgpu's GL backend, so
    /// WebGL 2), `raw_view` is a separate `Rgba8Unorm` texture that [`RawBlit`] fills from
    /// the sRGB picture after each frame. GL cannot copy between the two formats.
    raw_blit: Option<wgpu::BindGroup>,
}

/// Re-encodes the sRGB picture into a plain `Rgba8Unorm` texture (GL backends only).
struct RawBlit {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

const RAW_BLIT_WGSL: &str = r"
@group(0) @binding(0) var picture: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // Reading the sRGB texture decodes to linear; encode again for the plain target.
    let c = textureLoad(picture, vec2<i32>(p.xy), 0);
    let lo = c.rgb * 12.92;
    let hi = 1.055 * pow(c.rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(hi, lo, c.rgb <= vec3<f32>(0.0031308)), c.a);
}
";

impl RawBlit {
    fn new(device: &wgpu::Device) -> RawBlit {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raw blit"),
            source: wgpu::ShaderSource::Wgsl(RAW_BLIT_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raw blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raw blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raw blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        RawBlit { layout, pipeline }
    }
}

pub struct ViewRenderer {
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    body_layout: wgpu::BindGroupLayout,
    background: wgpu::RenderPipeline,
    mesh: wgpu::RenderPipeline,
    /// Blended, without depth writes, for bodies that are not opaque.
    mesh_transparent: wgpu::RenderPipeline,
    /// Faces into the depth buffer only (wireframe with hidden lines removed).
    mesh_depth_only: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    /// Edges behind faces, dashed (the hidden-edge style).
    hidden_edges: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    /// See-through fills (construction planes): hidden by bodies, hiding nothing.
    fills: wgpu::RenderPipeline,
    overlay_lines: wgpu::RenderPipeline,
    overlay_points: wgpu::RenderPipeline,
    /// The same, hidden by bodies in front (finished sketches).
    overlay_lines_depth: wgpu::RenderPipeline,
    overlay_points_depth: wgpu::RenderPipeline,
    /// Translucent tool previews (`Scene::previews`).
    preview: wgpu::RenderPipeline,
    targets: Option<Targets>,
    bodies: HashMap<u32, GpuBody>,
    /// Made on first use, only on backends without texture view formats.
    raw_blit: Option<RawBlit>,
}

fn f3(p: Point3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

impl ViewRenderer {
    pub fn new(device: &wgpu::Device) -> ViewRenderer {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chamfera viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let uniform_entry = |vis| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: vis,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let body_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("body"),
            entries: &[uniform_entry(wgpu::ShaderStages::FRAGMENT)],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let base_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globals only"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&globals_layout), Some(&body_layout)],
            immediate_size: 0,
        });

        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attrs,
        })];
        let point_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attrs,
        })];

        struct Spec<'a> {
            label: &'a str,
            layout: &'a wgpu::PipelineLayout,
            vs: &'a str,
            fs: &'a str,
            buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
            topology: wgpu::PrimitiveTopology,
            depth_write: bool,
            compare: wgpu::CompareFunction,
            bias: wgpu::DepthBiasState,
            blend: Option<wgpu::BlendState>,
            write: wgpu::ColorWrites,
        }
        let make = |s: Spec| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(s.label),
                layout: Some(s.layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(s.vs),
                    buffers: s.buffers,
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology: s.topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(s.depth_write),
                    depth_compare: Some(s.compare),
                    stencil: Default::default(),
                    bias: s.bias,
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(s.fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: s.blend,
                        write_mask: s.write,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let no_bias = wgpu::DepthBiasState::default();
        let all = wgpu::ColorWrites::ALL;
        // Faces are pushed slightly back so their edges draw cleanly on top.
        let face_bias = wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 1.5,
            clamp: 0.0,
        };
        let background = make(Spec {
            label: "background",
            layout: &base_layout,
            vs: "bg_vs",
            fs: "bg_fs",
            buffers: &[],
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::Always,
            bias: no_bias,
            blend: None,
            write: all,
        });
        let mesh = make(Spec {
            label: "mesh",
            layout: &mesh_layout,
            vs: "mesh_vs",
            fs: "mesh_fs",
            buffers: &mesh_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: true,
            compare: wgpu::CompareFunction::Less,
            bias: face_bias,
            blend: None,
            write: all,
        });
        let mesh_transparent = make(Spec {
            label: "mesh (transparent)",
            layout: &mesh_layout,
            vs: "mesh_vs",
            fs: "mesh_fs",
            buffers: &mesh_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::Less,
            bias: face_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let mesh_depth_only = make(Spec {
            label: "mesh (depth only)",
            layout: &mesh_layout,
            vs: "mesh_vs",
            fs: "mesh_fs",
            buffers: &mesh_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: true,
            compare: wgpu::CompareFunction::Less,
            bias: face_bias,
            blend: None,
            write: wgpu::ColorWrites::empty(),
        });
        let edges = make(Spec {
            label: "edges",
            layout: &base_layout,
            vs: "edge_vs",
            fs: "edge_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::LineList,
            depth_write: true,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: None,
            write: all,
        });
        // Only where a face is in front: the faces' depth bias keeps visible edges out.
        let hidden_edges = make(Spec {
            label: "hidden edges",
            layout: &base_layout,
            vs: "edge_vs",
            fs: "hidden_edge_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::LineList,
            depth_write: false,
            compare: wgpu::CompareFunction::Greater,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let grid = make(Spec {
            label: "grid",
            layout: &base_layout,
            vs: "line_vs",
            fs: "line_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::LineList,
            depth_write: false,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let fills = make(Spec {
            label: "fills",
            layout: &base_layout,
            vs: "line_vs",
            fs: "line_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        // Overlays draw over everything, in order, blended.
        let overlay_lines = make(Spec {
            label: "overlay lines",
            layout: &base_layout,
            vs: "line_vs",
            fs: "line_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::LineList,
            depth_write: false,
            compare: wgpu::CompareFunction::Always,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let overlay_points = make(Spec {
            label: "overlay points",
            layout: &base_layout,
            vs: "point_vs",
            fs: "line_fs",
            buffers: &point_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::Always,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        // Previews blend over the bodies and hide behind them, without hiding anything.
        let preview = make(Spec {
            label: "preview",
            layout: &mesh_layout,
            vs: "mesh_vs",
            fs: "preview_fs",
            buffers: &mesh_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let overlay_lines_depth = make(Spec {
            label: "overlay lines (depth tested)",
            layout: &base_layout,
            vs: "line_vs",
            fs: "line_fs",
            buffers: &line_buffers,
            topology: wgpu::PrimitiveTopology::LineList,
            depth_write: false,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        let overlay_points_depth = make(Spec {
            label: "overlay points (depth tested)",
            layout: &base_layout,
            vs: "point_vs",
            fs: "line_fs",
            buffers: &point_buffers,
            topology: wgpu::PrimitiveTopology::TriangleList,
            depth_write: false,
            compare: wgpu::CompareFunction::LessEqual,
            bias: no_bias,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write: all,
        });
        ViewRenderer {
            globals,
            globals_bind,
            body_layout,
            background,
            mesh,
            mesh_transparent,
            mesh_depth_only,
            edges,
            hidden_edges,
            grid,
            fills,
            overlay_lines,
            overlay_points,
            overlay_lines_depth,
            overlay_points_depth,
            preview,
            targets: None,
            bodies: HashMap::new(),
            raw_blit: None,
        }
    }

    /// The last rendered picture (single-sampled, sRGB).
    pub fn output(&self) -> Option<(&wgpu::Texture, &wgpu::TextureView)> {
        self.targets.as_ref().map(|t| (&t.resolve, &t.resolve_view))
    }

    /// The last picture for display by a UI. A UI that draws to an sRGB surface samples
    /// the sRGB view (decoded to linear); one that blends in sRGB space, like egui on a
    /// non-sRGB surface, samples the raw bytes.
    pub fn display_view(&self, surface_is_srgb: bool) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| {
            if surface_is_srgb {
                &t.resolve_view
            } else {
                &t.raw_view
            }
        })
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let tex = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = tex("msaa", COLOR_FORMAT, SAMPLES, attach, &[]);
        let depth = tex("depth", DEPTH_FORMAT, SAMPLES, attach, &[]);
        let raw = wgpu::TextureFormat::Rgba8Unorm;
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        // wgpu's GL backend (WebGL 2 in browsers) lacks `DownlevelFlags::VIEW_FORMATS`.
        let views = device.adapter_info().backend != wgpu::Backend::Gl;
        let raw_formats = [raw];
        let resolve = tex(
            "view",
            COLOR_FORMAT,
            1,
            attach | sampled | wgpu::TextureUsages::COPY_SRC,
            if views { &raw_formats } else { &[] },
        );
        let resolve_view = resolve.create_view(&Default::default());
        let (raw_view, raw_blit) = if views {
            let view = resolve.create_view(&wgpu::TextureViewDescriptor {
                format: Some(raw),
                ..Default::default()
            });
            (view, None)
        } else {
            let blit = self.raw_blit.get_or_insert_with(|| RawBlit::new(device));
            let copy = tex("view (raw)", raw, 1, attach | sampled, &[]);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("raw blit"),
                layout: &blit.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&resolve_view),
                }],
            });
            (copy.create_view(&Default::default()), Some(bind))
        };
        self.targets = Some(Targets {
            size: (w, h),
            msaa: msaa.create_view(&Default::default()),
            depth: depth.create_view(&Default::default()),
            resolve_view,
            raw_view,
            raw_blit,
            resolve,
        });
    }

    fn sync_bodies(&mut self, device: &wgpu::Device, theme: &Theme, scene: &Scene) {
        use wgpu::util::DeviceExt;
        self.bodies.retain(|id, _| scene.body(*id).is_some());
        for b in &scene.bodies {
            let key = (
                b.mesh.triangles.len(),
                b.mesh.positions.len(),
                b.edges.len(),
            );
            if self.bodies.get(&b.id).is_some_and(|g| g.key == key) {
                continue;
            }
            let tris = mesh_vertices(&b.mesh);
            let edges = edge_vertices(b, theme.edge.linear());
            let buffer = |label, contents: &[u8]| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                })
            };
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("body colour"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("body"),
                layout: &self.body_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            self.bodies.insert(
                b.id,
                GpuBody {
                    key,
                    triangles: buffer("body triangles", bytemuck::cast_slice(&tris)),
                    triangle_vertices: tris.len() as u32,
                    edges: buffer("body edges", bytemuck::cast_slice(&edges)),
                    edge_vertices: edges.len() as u32,
                    uniform,
                    bind,
                },
            );
        }
    }

    /// Records the commands that draw `frame`; the caller submits them. Sizes are clamped
    /// to 1..=[`MAX_SIZE`].
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
    ) -> wgpu::CommandBuffer {
        use wgpu::util::DeviceExt;
        let w = frame.width.clamp(1, MAX_SIZE);
        let h = frame.height.clamp(1, MAX_SIZE);
        self.ensure_targets(device, w, h);
        self.sync_bodies(device, frame.theme, frame.scene);

        let bounds = frame.scene.bounds();
        let cam = frame.camera;
        let aspect = f64::from(w) / f64::from(h);
        let view_proj = mat_mul(
            &cam.projection_matrix(aspect, bounds.as_ref()),
            &cam.view_matrix(),
        );
        let basis = cam.basis();
        // Key light from above, behind the viewer's right shoulder.
        let key = basis.up * 0.8 + basis.right * 0.5 - basis.forward * 0.6;
        let ortho = if cam.projection == Projection::Orthographic {
            1.0
        } else {
            0.0
        };
        let e = cam.eye();
        let section = match frame.scene.display.section {
            Some(s) => {
                let n = s.normal;
                let w = -(s.origin - Point3::ORIGIN).dot(n);
                [n.x as f32, n.y as f32, n.z as f32, w as f32]
            }
            None => [0.0; 4],
        };
        let scale = frame.scale.clamp(0.25, 8.0);
        let px_per_mm = f64::from(h) / (2.0 * cam.half_height()).max(1e-9);
        let globals = Globals {
            view_proj,
            eye: [e.x as f32, e.y as f32, e.z as f32, 1.0],
            light: [key.x as f32, key.y as f32, key.z as f32, ortho],
            forward: [
                basis.forward.x as f32,
                basis.forward.y as f32,
                basis.forward.z as f32,
                0.0,
            ],
            background_top: frame.theme.background_top.linear(),
            background_bottom: frame.theme.background_bottom.linear(),
            viewport: [w as f32, h as f32, POINT_SIZE * scale, 0.0],
            section,
            display: [
                if frame.scene.display.section.is_some() {
                    1.0
                } else {
                    0.0
                },
                px_per_mm as f32,
                DASH_PIXELS * scale,
                HATCH_PIXELS as f32,
            ],
            section_cap: frame.theme.section_cap.linear(),
            section_hatch: frame.theme.section_hatch.linear(),
            edge_hidden: frame.theme.edge_hidden.linear(),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        for b in &frame.scene.bodies {
            if let Some(g) = self.bodies.get(&b.id) {
                let selected = frame.scene.is_selected(b.id);
                // A chosen colour is the body's own data; without one the theme decides.
                let c = if selected {
                    frame.theme.body_selected
                } else {
                    b.appearance
                        .colour
                        .unwrap_or_else(|| frame.theme.body_colour(b.tint))
                };
                let mut c = c.linear();
                c[3] = b.appearance.opacity();
                queue.write_buffer(&g.uniform, 0, bytemuck::cast_slice(&c));
            }
        }
        let grid = grid_vertices(cam, frame.theme);
        let grid_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid"),
            contents: bytemuck::cast_slice(&grid),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let fills = fill_vertices(frame.scene, frame.theme);
        let (depth_lines, depth_points) = overlay_vertices(frame.scene, frame.theme, false);
        let (overlay_lines, overlay_points) = overlay_vertices(frame.scene, frame.theme, true);
        let vertex_buffer = |label, data: &[LineVertex]| {
            (!data.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        let fill_buf = vertex_buffer("fills", &fills);
        let overlay_line_buf = vertex_buffer("overlay lines", &overlay_lines);
        let overlay_point_buf = vertex_buffer("overlay points", &overlay_points);
        let depth_line_buf = vertex_buffer("overlay lines (depth)", &depth_lines);
        let depth_point_buf = vertex_buffer("overlay points (depth)", &depth_points);
        // Previews change while a dialog's values change, so they are uploaded each frame.
        let preview_tris: Vec<MeshVertex> = frame
            .scene
            .previews
            .iter()
            .flat_map(mesh_vertices)
            .collect();
        let preview_lines: Vec<LineVertex> = frame
            .scene
            .previews
            .iter()
            .flat_map(|m| mesh_edge_vertices(m, frame.theme.sketch_preview.linear()))
            .collect();
        let preview = (!preview_tris.is_empty()).then(|| {
            let tris = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview triangles"),
                contents: bytemuck::cast_slice(&preview_tris),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let colour = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview colour"),
                contents: bytemuck::cast_slice(&frame.theme.preview_body.linear()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("preview"),
                layout: &self.body_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: colour.as_entire_binding(),
                }],
            });
            (tris, preview_tris.len() as u32, bind)
        });
        let preview_edges =
            vertex_buffer("preview edges", &preview_lines).map(|b| (b, preview_lines.len() as u32));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        if let Some(t) = &self.targets {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&t.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);

            let plan = frame.scene.display.style.plan();
            let (opaque, transparent) = draw_order(frame.scene, cam);
            let gpu = |ids: &[u32]| -> Vec<&GpuBody> {
                ids.iter().filter_map(|id| self.bodies.get(id)).collect()
            };
            let (opaque, transparent) = (gpu(&opaque), gpu(&transparent));
            let draw_faces = |pass: &mut wgpu::RenderPass, bodies: &[&GpuBody]| {
                for g in bodies {
                    pass.set_bind_group(1, &g.bind, &[]);
                    pass.set_vertex_buffer(0, g.triangles.slice(..));
                    pass.draw(0..g.triangle_vertices, 0..1);
                }
            };
            match plan.faces {
                FacePass::Shaded => {
                    pass.set_pipeline(&self.mesh);
                    draw_faces(&mut pass, &opaque);
                }
                FacePass::DepthOnly => {
                    pass.set_pipeline(&self.mesh_depth_only);
                    draw_faces(&mut pass, &opaque);
                    draw_faces(&mut pass, &transparent);
                }
                FacePass::None => {}
            }
            for edge_pass in plan.edges {
                pass.set_pipeline(match edge_pass {
                    EdgePass::Visible => &self.edges,
                    EdgePass::Hidden => &self.hidden_edges,
                });
                for g in opaque.iter().chain(&transparent) {
                    if g.edge_vertices > 0 {
                        pass.set_vertex_buffer(0, g.edges.slice(..));
                        pass.draw(0..g.edge_vertices, 0..1);
                    }
                }
            }
            // Transparent faces last, back to front, over the edges seen through them.
            if plan.faces == FacePass::Shaded && !transparent.is_empty() {
                pass.set_pipeline(&self.mesh_transparent);
                draw_faces(&mut pass, &transparent);
            }
            if !grid.is_empty() {
                pass.set_pipeline(&self.grid);
                pass.set_vertex_buffer(0, grid_buf.slice(..));
                pass.draw(0..grid.len() as u32, 0..1);
            }
            if let Some((tris, n, bind)) = &preview {
                pass.set_pipeline(&self.preview);
                pass.set_bind_group(1, bind, &[]);
                pass.set_vertex_buffer(0, tris.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some((buf, n)) = &preview_edges {
                pass.set_pipeline(&self.overlay_lines_depth);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some(buf) = &fill_buf {
                pass.set_pipeline(&self.fills);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..fills.len() as u32, 0..1);
            }
            if let Some(buf) = &depth_line_buf {
                pass.set_pipeline(&self.overlay_lines_depth);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..depth_lines.len() as u32, 0..1);
            }
            if let Some(buf) = &depth_point_buf {
                pass.set_pipeline(&self.overlay_points_depth);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..depth_points.len() as u32);
            }
            if let Some(buf) = &overlay_line_buf {
                pass.set_pipeline(&self.overlay_lines);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..overlay_lines.len() as u32, 0..1);
            }
            if let Some(buf) = &overlay_point_buf {
                pass.set_pipeline(&self.overlay_points);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..overlay_points.len() as u32);
            }
        }
        if let (Some(t), Some(blit)) = (&self.targets, &self.raw_blit)
            && let Some(bind) = &t.raw_blit
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raw blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.raw_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&blit.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.finish()
    }
}

/// Overlay line vertices (pairs) and point instances of the overlays drawn on top (or
/// not), in drawing order, capped at [`MAX_OVERLAY_ITEMS`]. Points with invalid
/// coordinates are skipped.
fn overlay_vertices(
    scene: &Scene,
    theme: &Theme,
    on_top: bool,
) -> (Vec<LineVertex>, Vec<LineVertex>) {
    let mut lines = Vec::new();
    let mut points = Vec::new();
    let mut budget = MAX_OVERLAY_ITEMS;
    for o in scene.overlays.iter().filter(|o| o.on_top == on_top) {
        let color = o.style.color(theme).linear();
        for [a, b] in o.lines.iter().take(budget) {
            if a.is_valid() && b.is_valid() {
                lines.push(LineVertex {
                    position: f3(*a),
                    color,
                    arc: 0.0,
                });
                lines.push(LineVertex {
                    position: f3(*b),
                    color,
                    arc: 0.0,
                });
            }
        }
        budget = budget.saturating_sub(o.lines.len());
        for p in o.points.iter().take(budget) {
            if p.is_valid() {
                points.push(LineVertex {
                    position: f3(*p),
                    color,
                    arc: 0.0,
                });
            }
        }
        budget = budget.saturating_sub(o.points.len());
    }
    (lines, points)
}

/// The fills' triangle corners, in drawing order, capped at [`MAX_OVERLAY_ITEMS`]
/// triangles. Triangles with an invalid corner are skipped.
fn fill_vertices(scene: &Scene, theme: &Theme) -> Vec<LineVertex> {
    let mut out = Vec::new();
    let mut budget = MAX_OVERLAY_ITEMS;
    for f in &scene.fills {
        let color = f.style.color(theme).linear();
        for t in f.triangles.iter().take(budget) {
            if t.iter().all(|p| p.is_valid()) {
                out.extend(t.iter().map(|p| LineVertex {
                    position: f3(*p),
                    color,
                    arc: 0.0,
                }));
            }
        }
        budget = budget.saturating_sub(f.triangles.len());
    }
    out
}

/// Flat-shaded triangles: each corner carries its face's normal.
fn mesh_vertices(mesh: &TriMesh) -> Vec<MeshVertex> {
    let mut out = Vec::with_capacity(mesh.triangles.len() * 3);
    for i in 0..mesh.triangles.len() {
        let (Some(corners), Some(n)) = (mesh.triangle(i), mesh.triangle_normal(i)) else {
            continue;
        };
        let normal = [n.x as f32, n.y as f32, n.z as f32];
        for p in corners {
            out.push(MeshVertex {
                position: f3(p),
                normal,
            });
        }
    }
    out
}

/// A mesh's sharp edges as vertex pairs, each carrying its length along the edge.
fn mesh_edge_vertices(mesh: &TriMesh, color: [f32; 4]) -> Vec<LineVertex> {
    let mut out = Vec::new();
    for [a, b] in mesh.feature_edges(EDGE_ANGLE) {
        out.push(LineVertex {
            position: f3(a),
            color,
            arc: 0.0,
        });
        out.push(LineVertex {
            position: f3(b),
            color,
            arc: (b - a).length() as f32,
        });
    }
    out
}

/// A body's edge lines as vertex pairs: its exact B-rep edges when it has them, else the
/// mesh's sharp edges. Each vertex carries its length along the edge, for dashes.
fn edge_vertices(body: &SceneBody, color: [f32; 4]) -> Vec<LineVertex> {
    if body.edges.is_empty() {
        return mesh_edge_vertices(&body.mesh, color);
    }
    let vertex = |p: Point3, arc: f64| LineVertex {
        position: f3(p),
        color,
        arc: arc as f32,
    };
    let mut out = Vec::new();
    for line in &body.edges {
        let mut arc = 0.0;
        for pair in line.windows(2) {
            let [a, b] = pair else { continue };
            if !(a.is_valid() && b.is_valid()) {
                continue;
            }
            let next = arc + (*b - *a).length();
            out.push(vertex(*a, arc));
            out.push(vertex(*b, next));
            arc = next;
        }
    }
    out
}

/// Grid spacing (mm) for a camera distance: a power of ten, about 1/10 of the distance.
pub fn grid_step(distance: f64) -> f64 {
    let d = if distance.is_finite() {
        distance.clamp(1e-3, 1e7)
    } else {
        250.0
    };
    10f64.powf((d / 10.0).log10().floor())
}

/// The ground grid on the XY plane around the target, fading out at its rim, plus the
/// X (red), Y (green) and Z (blue) axes through the origin.
fn grid_vertices(cam: &Camera, theme: &Theme) -> Vec<LineVertex> {
    const SEGMENTS: usize = 8;
    let minor = grid_step(cam.distance);
    let major = minor * 10.0;
    let extent = major * (cam.distance * 2.0 / major).ceil().max(1.0);
    let cx = (cam.target.x / major).round() * major;
    let cy = (cam.target.y / major).round() * major;
    let lines = (extent / minor).round() as i64;
    let mut out = Vec::with_capacity((4 * lines as usize + 6) * SEGMENTS * 2 + 6);
    let fade = |x: f64, y: f64, c: [f32; 4]| {
        let r = (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / extent).min(1.0);
        [c[0], c[1], c[2], c[3] * (1.0 - r * r) as f32]
    };
    let minor_c = theme.grid_minor.linear();
    let major_c = theme.grid_major.linear();
    let segment = |out: &mut Vec<LineVertex>, a: (f64, f64), b: (f64, f64), c: [f32; 4]| {
        for s in 0..SEGMENTS {
            let t0 = s as f64 / SEGMENTS as f64;
            let t1 = (s + 1) as f64 / SEGMENTS as f64;
            for t in [t0, t1] {
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                out.push(LineVertex {
                    position: [x as f32, y as f32, 0.0],
                    color: fade(x, y, c),
                    arc: 0.0,
                });
            }
        }
    };
    for k in -lines..=lines {
        let off = k as f64 * minor;
        let is_major = k.rem_euclid(10) == ((cx / minor).round() as i64).rem_euclid(10);
        let c = if is_major { major_c } else { minor_c };
        let x = cx + off;
        let y = cy + off;
        // Skip the lines the axes draw over.
        if x.abs() > minor * 1e-6 {
            segment(&mut out, (x, cy - extent), (x, cy + extent), c);
        }
        if y.abs() > minor * 1e-6 {
            segment(&mut out, (cx - extent, y), (cx + extent, y), c);
        }
    }
    segment(
        &mut out,
        (cx - extent, 0.0),
        (cx + extent, 0.0),
        theme.axis_x.linear(),
    );
    segment(
        &mut out,
        (0.0, cy - extent),
        (0.0, cy + extent),
        theme.axis_y.linear(),
    );
    let z = theme.axis_z.linear();
    for p in [[0.0, 0.0, 0.0], [0.0, 0.0, (major * 0.5) as f32]] {
        out.push(LineVertex {
            position: p,
            color: z,
            arc: 0.0,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_steps_are_powers_of_ten_near_a_tenth_of_the_distance() {
        assert_eq!(grid_step(250.0), 10.0);
        assert_eq!(grid_step(99.0), 1.0);
        assert_eq!(grid_step(100.0), 10.0);
        assert_eq!(grid_step(5000.0), 100.0);
        assert_eq!(grid_step(f64::NAN), 10.0);
    }

    #[test]
    fn mesh_vertices_carry_face_normals() {
        let m = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        let v = mesh_vertices(&m);
        assert_eq!(v.len(), 36);
        for tri in v.chunks(3) {
            let n = tri[0].normal;
            assert!((n[0].abs() + n[1].abs() + n[2].abs() - 1.0).abs() < 1e-6);
        }
    }
}

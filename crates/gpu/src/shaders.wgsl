// Chamfera viewport shaders: shaded bodies, lines (grid, axes, edges) and the background.

struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xyz: key light direction (towards the light); w: 1 for orthographic views.
    light: vec4<f32>,
    // Camera forward direction, used as the view direction in orthographic views.
    forward: vec4<f32>,
    background_top: vec4<f32>,
    background_bottom: vec4<f32>,
    // x, y: view size in pixels; z: overlay point size in pixels.
    viewport: vec4<f32>,
    // Section plane: xyz normal (towards the removed side), w = -dot(normal, origin).
    section: vec4<f32>,
    // x: 1 with a section; y: pixels per mm; z: dash period (px); w: hatch period (px).
    display: vec4<f32>,
    section_cap: vec4<f32>,
    section_hatch: vec4<f32>,
    edge_hidden: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Body {
    color: vec4<f32>,
};

@group(1) @binding(0) var<uniform> body: Body;

// ---- bodies ----

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn mesh_vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> MeshOut {
    var out: MeshOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.world = position;
    return out;
}

// True where the section plane cuts the point away.
fn cut_away(world: vec3<f32>) -> bool {
    return g.display.x > 0.5 && dot(g.section.xyz, world) + g.section.w > 0.0;
}

@fragment
fn mesh_fs(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    // Looking into a cut solid shows the inside of its far faces: draw them as the cut
    // face, flat, with 45-degree hatch stripes in screen space.
    if (g.display.x > 0.5 && !front) {
        let period = max(g.display.w, 2.0);
        let px = floor(in.clip.xy);
        let k = (px.x + px.y) - floor((px.x + px.y) / period) * period;
        if (k < 2.0) {
            return vec4<f32>(g.section_hatch.rgb, 1.0);
        }
        return vec4<f32>(g.section_cap.rgb, 1.0);
    }
    var n = normalize(in.normal);
    // Meshes from files may be inside out; light the side we see.
    if (!front) {
        n = -n;
    }
    var v = normalize(g.eye.xyz - in.world);
    if (g.light.w > 0.5) {
        v = -g.forward.xyz;
    }
    let key = normalize(g.light.xyz);
    let head = max(dot(n, v), 0.0);
    let side = max(dot(n, key), 0.0);
    let h = normalize(v + key);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.18;
    let shade = 0.22 + 0.55 * head + 0.3 * side;
    return vec4<f32>(body.color.rgb * shade + vec3<f32>(spec), body.color.a);
}

// ---- lines ----

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn line_vs(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var out: LineOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}

@fragment
fn line_fs(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

// ---- body edges: clipped by the section; hidden ones dashed ----

struct EdgeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
    @location(2) arc: f32,
};

@vertex
fn edge_vs(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) arc: f32,
) -> EdgeOut {
    var out: EdgeOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.color = color;
    out.world = position;
    out.arc = arc;
    return out;
}

@fragment
fn edge_fs(in: EdgeOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    return in.color;
}

@fragment
fn hidden_edge_fs(in: EdgeOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    let period = max(g.display.z, 2.0);
    let along = in.arc * g.display.y;
    if (along - floor(along / period) * period > period * 0.5) {
        discard;
    }
    return g.edge_hidden;
}

// ---- background: a vertical gradient from one full-screen triangle ----

struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var out: BgOut;
    out.clip = vec4<f32>(x, y, 1.0, 1.0);
    out.t = y * 0.5 + 0.5;
    return out;
}

@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    return mix(g.background_bottom, g.background_top, clamp(in.t, 0.0, 1.0));
}

// ---- overlay points: a square of constant screen size per point (instanced) ----

@vertex
fn point_vs(
    @builtin(vertex_index) i: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
) -> LineOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i % 6u];
    var clip = g.view_proj * vec4<f32>(position, 1.0);
    let half_px = g.viewport.z * 0.5;
    clip.x = clip.x + c.x * half_px * 2.0 / max(g.viewport.x, 1.0) * clip.w;
    clip.y = clip.y + c.y * half_px * 2.0 / max(g.viewport.y, 1.0) * clip.w;
    var out: LineOut;
    out.clip = clip;
    out.color = color;
    return out;
}

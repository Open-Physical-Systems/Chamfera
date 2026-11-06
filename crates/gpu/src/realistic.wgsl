// ---- the realistic style: physically based shading in the procedural environment ----
//
// Appended to shaders.wgsl (it uses `g`, `body` and `cut_away` from there).
//
// Cook-Torrance with the GGX distribution (Walter et al. 2007, "Microfacet Models for
// Refraction through Rough Surfaces"), roughness squared as alpha, the Schlick-GGX
// masking k for analytic lights and Schlick's Fresnel (Karis 2013, "Real Shading in
// Unreal Engine 4"), and the analytic fit of the split-sum environment term (Karis 2014,
// "Physically Based Shading on Mobile"). The sky is the same gradient the path tracer
// uses (crates/render/src/environment.rs).

@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

const PI: f32 = 3.14159265;

struct LitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn realistic_vs(
    @location(0) position: vec3<f32>,
    @location(1) flat_normal: vec3<f32>,
    @location(2) smooth_normal: vec3<f32>,
) -> LitOut {
    var out: LitOut;
    out.clip = g.view_proj * vec4<f32>(position, 1.0);
    out.normal = smooth_normal;
    out.world = position;
    return out;
}

@vertex
fn shadow_vs(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return g.shadow_view_proj * vec4<f32>(position, 1.0);
}

fn sky(d: vec3<f32>) -> vec3<f32> {
    let z = clamp(d.z, -1.0, 1.0);
    if (z >= 0.0) {
        return mix(g.sky_horizon.rgb, g.sky_zenith.rgb, sqrt(z));
    }
    return mix(g.sky_horizon.rgb, g.sky_nadir.rgb, sqrt(-z));
}

// Narkowicz's fit of the ACES curve, as `chamfera_render::environment::aces`.
fn aces(x: vec3<f32>) -> vec3<f32> {
    let c = max(x, vec3<f32>(0.0));
    return clamp(c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn exposed(c: vec3<f32>) -> vec3<f32> {
    return aces(c * g.sky_zenith.w);
}

// The radiance a Lambert floor facing up sends back from the sky alone, per unit albedo:
// with L(z) = mix(h, zen, sqrt z), irradiance = 2 pi int_0^1 L z dz = pi (0.2 h + 0.8 zen),
// and radiance = albedo / pi * irradiance.
fn sky_on_floor() -> vec3<f32> {
    return 0.2 * g.sky_horizon.rgb + 0.8 * g.sky_zenith.rgb;
}

fn shadow_at(world: vec3<f32>) -> f32 {
    if (g.shadow.z < 0.5) {
        return 1.0;
    }
    let p = g.shadow_view_proj * vec4<f32>(world, 1.0);
    let ndc = p.xyz / p.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let texel = 1.0 / max(g.shadow.x, 1.0);
    var sum = 0.0;
    // A 4 x 4 percentage-closer filter: a soft edge, like the key light's disc.
    for (var i = -2; i < 2; i = i + 1) {
        for (var j = -2; j < 2; j = j + 1) {
            let o = (vec2<f32>(f32(i), f32(j)) + 0.5) * texel * 1.5;
            sum = sum + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o, ndc.z - g.shadow.y);
        }
    }
    return sum / 16.0;
}

fn ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let k = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * k * k);
}

fn schlick(c: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(c, 0.0, 1.0), 5.0);
}

// One light's reflected radiance: f * E * cos.
fn light_term(
    n: vec3<f32>, v: vec3<f32>, dir: vec4<f32>, e: vec4<f32>,
    diffuse: vec3<f32>, f0: vec3<f32>, roughness: f32,
) -> vec3<f32> {
    if (e.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let l = normalize(dir.xyz);
    let nl = dot(n, l);
    let nv = max(dot(n, v), 1e-4);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(v + l);
    // A disc light widens the highlight: alpha grows by about the disc's angular radius
    // (Karis 2013, sphere lights).
    let a = clamp(roughness * roughness + 0.5 * dir.w, 0.002, 1.0);
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let gv = nv / (nv * (1.0 - k) + k);
    let gl = nl / (nl * (1.0 - k) + k);
    let f = schlick(dot(v, h), f0);
    let spec = f * ggx(max(dot(n, h), 0.0), a) * gv * gl / (4.0 * nv * nl);
    // Light reaching the diffuse layer crosses the coat in and out, as in the path tracer.
    let through = (vec3<f32>(1.0) - schlick(nv, f0)) * (vec3<f32>(1.0) - schlick(nl, f0));
    return (diffuse / PI * through + spec) * e.rgb * nl;
}

// Karis 2014: the split-sum environment BRDF as a fit, a scale and bias of F0.
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// The lit floor's radiance far from shadows (sky and lights), or the nadir without one.
fn below() -> vec3<f32> {
    var e = vec3<f32>(0.0);
    e = e + g.light0_e.rgb * max(normalize(g.light0_dir.xyz).z, 0.0) * g.light0_e.w;
    e = e + g.light1_e.rgb * max(normalize(g.light1_dir.xyz).z, 0.0) * g.light1_e.w;
    e = e + g.light2_e.rgb * max(normalize(g.light2_dir.xyz).z, 0.0) * g.light2_e.w;
    let lit = g.floor_albedo.rgb * (sky_on_floor() + e / PI);
    return select(g.sky_nadir.rgb, lit, g.floor_albedo.w > 0.5);
}

// What a direction sees of the environment, blurred by roughness: the sky above, the lit
// floor below.
fn environment(d: vec3<f32>, roughness: f32) -> vec3<f32> {
    let seen = select(sky(d), below(), d.z < 0.0);
    let average = 0.5 * (sky_on_floor() + below());
    return mix(seen, average, roughness * roughness);
}

fn view_dir(world: vec3<f32>) -> vec3<f32> {
    if (g.light.w > 0.5) {
        return -g.forward.xyz;
    }
    return normalize(g.eye.xyz - world);
}

@fragment
fn realistic_fs(in: LitOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = view_dir(in.world);
    // Shading normals can turn away from the viewer on silhouettes.
    if (dot(n, v) < 0.0) {
        n = normalize(n - v * dot(n, v) * 1.01);
    }
    let base = body.color.rgb;
    let metallic = clamp(body.material.x, 0.0, 1.0);
    let roughness = clamp(body.material.y, 0.03, 1.0);
    let transmission = clamp(body.material.z, 0.0, 1.0);
    let ior = clamp(body.material.w, 1.0, 3.0);
    let r0 = (ior - 1.0) / (ior + 1.0);
    let f0 = mix(vec3<f32>(r0 * r0), base, metallic);
    let diffuse = base * (1.0 - metallic) * (1.0 - transmission);
    let nv = max(dot(n, v), 1e-4);

    var c = vec3<f32>(0.0);
    let s = shadow_at(in.world);
    c = c + light_term(n, v, g.light0_dir, g.light0_e, diffuse, f0, roughness) * s;
    c = c + light_term(n, v, g.light1_dir, g.light1_e, diffuse, f0, roughness);
    c = c + light_term(n, v, g.light2_dir, g.light2_e, diffuse, f0, roughness);
    // Ambient: diffuse light from the sky above and the floor below, and the environment
    // seen in the reflection. Faces turned down and points in the key light's shadow see
    // less of the sky (a cheap stand-in for ambient occlusion).
    let ambient = mix(below(), sky_on_floor(), n.z * 0.5 + 0.5);
    let occlusion = mix(0.55, 1.0, s) * mix(0.75, 1.0, n.z * 0.5 + 0.5);
    let fr = env_brdf(f0, roughness, nv);
    c = c + diffuse * ambient * (vec3<f32>(1.0) - fr) * occlusion;
    c = c + environment(reflect(-v, n), roughness) * fr * mix(0.6, 1.0, s);
    if (transmission > 0.5) {
        // Glass, blended premultiplied: its reflections add, and what is behind passes by
        // what the surface does not reflect times the glass's colour (on average).
        let f = schlick(nv, vec3<f32>(r0 * r0)).x;
        let passed = (1.0 - f) * dot(base, vec3<f32>(1.0 / 3.0));
        return vec4<f32>(exposed(c + base * ambient * 0.04), clamp(1.0 - passed, 0.0, 1.0));
    }
    // Other see-through bodies (opacity below 1) blend by their opacity.
    let a = body.color.a;
    return vec4<f32>(exposed(c) * a, a);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn sky_vs(@builtin(vertex_index) i: u32) -> SkyOut {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

fn sky_ray(ndc: vec2<f32>) -> vec3<f32> {
    if (g.light.w > 0.5) {
        return normalize(g.forward.xyz);
    }
    return normalize(g.forward.xyz + g.cam_right.xyz * ndc.x * g.cam_right.w + g.cam_up.xyz * ndc.y * g.cam_up.w);
}

@fragment
fn sky_fs(in: SkyOut) -> @location(0) vec4<f32> {
    return vec4<f32>(exposed(sky(sky_ray(in.ndc))), 1.0);
}

// ---- the floor: a square under the scene that fades into the sky ----

struct FloorOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn floor_vs(@builtin(vertex_index) i: u32) -> FloorOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i % 6u];
    // FLOOR_FADE_END radii out: beyond it the floor has faded away.
    let half = g.floor_geom.w * 10.0;
    let p = vec3<f32>(g.floor_geom.x + c.x * half, g.floor_geom.y + c.y * half, g.floor_geom.z);
    var out: FloorOut;
    out.clip = g.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    return out;
}

@fragment
fn floor_fs(in: FloorOut) -> @location(0) vec4<f32> {
    let s = shadow_at(in.world);
    var e = vec3<f32>(0.0);
    e = e + g.light0_e.rgb * max(normalize(g.light0_dir.xyz).z, 0.0) * g.light0_e.w * s;
    e = e + g.light1_e.rgb * max(normalize(g.light1_dir.xyz).z, 0.0) * g.light1_e.w;
    e = e + g.light2_e.rgb * max(normalize(g.light2_dir.xyz).z, 0.0) * g.light2_e.w;
    // Under the part the sky is partly hidden too.
    let c = g.floor_albedo.rgb * (sky_on_floor() * mix(0.6, 1.0, s) + e / PI);
    // Fade into the sky between 3 and 10 radii from the centre (FLOOR_FADE_START, END).
    let r = max(g.floor_geom.w, 1e-6);
    let d = length(in.world.xy - g.floor_geom.xy) / r;
    let t = clamp((d - 3.0) / 7.0, 0.0, 1.0);
    let w = 1.0 - t * t * (3.0 - 2.0 * t);
    let bg = sky(-view_dir(in.world));
    return vec4<f32>(exposed(mix(bg, c, w)), 1.0);
}

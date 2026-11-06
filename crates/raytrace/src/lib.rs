//! Layer 3: a CPU path tracer for presentation images.
//!
//! It traces the display meshes (the same tessellation the viewport draws) through a
//! bounding volume hierarchy, with the materials of `chamfera_render::material` and the
//! procedural environment of `chamfera_render::environment`, over a floor that catches
//! shadows. Rendering is progressive: [`Render::step`] adds one sample to every pixel, on
//! as many threads as the machine has (std threads; one thread in the browser). Pure Rust,
//! no GPU, builds for wasm.
//!
//! Light transport: unidirectional path tracing with next-event estimation towards the
//! environment's disc lights, combined with BSDF sampling by multiple importance sampling
//! (Veach and Guibas 1995, the power heuristic), and Russian roulette after a few bounces.

mod bsdf;
mod bvh;
mod v3;

pub use bsdf::{fresnel_dielectric, ggx_d, schlick, smith_g, smith_g1};
pub use bvh::{Bvh, Hit, Tri};
pub use v3::V3;

use bsdf::Opaque;
use chamfera_geom::TriMesh;
use chamfera_render::normals::corner_normals;
use chamfera_render::{Camera, Environment, Material, Projection};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use thiserror::Error;
use v3::{to_world, v3};

/// Largest image side (pixels).
pub const MAX_SIZE: u32 = 4096;
/// Most samples per pixel.
pub const MAX_SAMPLES: u32 = 4096;
/// Most triangles a render takes.
pub const MAX_TRIANGLES: usize = 20_000_000;
/// Most threads one render uses.
pub const MAX_THREADS: usize = 16;
/// Longest path.
const MAX_BOUNCES: u32 = 10;
/// Normals within this angle shade smoothly (the viewport's edge angle).
const CREASE: f64 = std::f64::consts::PI / 6.0;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderError {
    #[error("`{name}` must be 1 to {max}, not {value}")]
    OutOfRange {
        name: &'static str,
        value: u64,
        max: u64,
    },
    #[error("the scene has {0} triangles; a render takes at most {MAX_TRIANGLES}")]
    TooManyTriangles(usize),
    #[error("could not write the PNG: {0}")]
    Png(String),
}

/// What to render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel to stop at.
    pub samples: u32,
    /// Seeds the random numbers: the same settings give the same picture.
    pub seed: u64,
    /// Threads; 0 picks the machine's count (capped at [`MAX_THREADS`]).
    pub threads: usize,
}

impl Settings {
    pub fn new(width: u32, height: u32, samples: u32) -> Settings {
        Settings {
            width,
            height,
            samples,
            seed: 1,
            threads: 0,
        }
    }

    /// Refuses sizes and sample counts out of range.
    pub fn check(&self) -> Result<(), RenderError> {
        for (name, value, max) in [
            ("width", self.width, MAX_SIZE),
            ("height", self.height, MAX_SIZE),
            ("samples", self.samples, MAX_SAMPLES),
        ] {
            if value == 0 || value > max {
                return Err(RenderError::OutOfRange {
                    name,
                    value: u64::from(value),
                    max: u64::from(max),
                });
            }
        }
        Ok(())
    }
}

/// One body to render.
pub struct Body<'a> {
    pub mesh: &'a TriMesh,
    pub material: Material,
}

/// A small, fast random number generator: PCG32 (O'Neill 2014), one stream per pixel and
/// sample, so the picture does not depend on how pixels are shared between threads.
struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    fn new(seed: u64, stream: u64) -> Rng {
        let mut r = Rng {
            state: 0,
            inc: (stream << 1) | 1,
        };
        r.next();
        r.state = r.state.wrapping_add(seed);
        r.next();
        r
    }

    fn next(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in [0, 1).
    fn f(&mut self) -> f64 {
        f64::from(self.next()) * (1.0 / 4_294_967_296.0)
    }
}

/// The camera as rays.
#[derive(Debug, Clone, Copy)]
struct View {
    eye: V3,
    forward: V3,
    right: V3,
    up: V3,
    half_w: f64,
    half_h: f64,
    distance: f64,
    ortho: bool,
    /// Orthographic rays start this far behind the eye plane.
    back: f64,
}

impl View {
    fn new(camera: &Camera, width: u32, height: u32, radius: f64) -> View {
        let mut cam = *camera;
        cam.sanitize();
        let b = cam.basis();
        let hh = cam.half_height();
        View {
            eye: cam.eye().into(),
            forward: b.forward.into(),
            right: b.right.into(),
            up: b.up.into(),
            half_w: hh * f64::from(width) / f64::from(height),
            half_h: hh,
            distance: cam.distance,
            ortho: cam.projection == Projection::Orthographic,
            back: radius * 4.0 + cam.distance,
        }
    }

    /// The ray through image point `(x, y)` in -1..1 (y up).
    fn ray(&self, x: f64, y: f64) -> (V3, V3) {
        let off = self.right * (x * self.half_w) + self.up * (y * self.half_h);
        if self.ortho {
            (self.eye + off - self.forward * self.back, self.forward)
        } else {
            let d = self.forward * self.distance + off;
            (self.eye, d.normalized().unwrap_or(self.forward))
        }
    }
}

/// The scene ready for tracing.
struct World {
    bvh: Bvh,
    opaque: Vec<Opaque>,
    materials: Vec<Material>,
    env: Environment,
    floor_z: f64,
    centre: V3,
    radius: f64,
    /// Offset of new rays from a surface, against self-hits.
    eps: f64,
}

/// What a ray met.
enum Met {
    Triangle(Hit),
    Floor(f64),
}

impl World {
    fn intersect(&self, o: V3, d: V3) -> Option<Met> {
        let floor = match self.env.floor {
            Some(_) if d.z < 0.0 && o.z > self.floor_z => Some((self.floor_z - o.z) / d.z),
            _ => None,
        };
        let reach = floor.unwrap_or(f64::INFINITY);
        match self.bvh.intersect(o, d, 0.0, reach) {
            Some(h) => Some(Met::Triangle(h)),
            None => floor.map(Met::Floor),
        }
    }

    /// Power heuristic with exponent 2.
    fn mis(a: f64, b: f64) -> f64 {
        let (a2, b2) = (a * a, b * b);
        if a2 + b2 <= 0.0 { 0.0 } else { a2 / (a2 + b2) }
    }

    /// Light from the lights reaching `p` with shading normal `n`, by sampling each
    /// light's disc once and weighting against BSDF sampling.
    fn direct(&self, p: V3, n: V3, ng: V3, v: V3, b: &Opaque, rng: &mut Rng) -> V3 {
        let mut out = V3::ZERO;
        for light in &self.env.lights {
            let (u1, u2) = (rng.f(), rng.f());
            // Uniform in the cone: cos t = 1 - u (1 - cos_max).
            let cos_t = 1.0 - u1 * (1.0 - light.cos_max);
            let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
            let phi = std::f64::consts::TAU * u2;
            let axis: V3 = light.direction.into();
            let l = to_world(axis, v3(sin_t * phi.cos(), sin_t * phi.sin(), cos_t));
            let Some(l) = l.normalized() else { continue };
            let nl = n.dot(l);
            if nl <= 0.0 || ng.dot(l) <= 0.0 {
                continue;
            }
            let through = self.transmittance(p + ng * self.eps, l);
            if through.max_component() <= 0.0 {
                continue;
            }
            let pdf_light = 1.0 / light.solid_angle();
            let w = Self::mis(pdf_light, b.pdf(n, v, l));
            out += b.eval(n, v, l) * V3::rgb(light.radiance) * through * (nl / pdf_light * w);
        }
        out
    }

    /// How much light gets from `o` along `d` to the sky: 0 behind an opaque body; glass
    /// passes its tint and what its surfaces do not reflect (a shadow that ignores the
    /// bending of the light, so no caustics, but not black).
    fn transmittance(&self, mut o: V3, d: V3) -> V3 {
        let mut t = V3::ONE;
        for _ in 0..16 {
            let Some(h) = self.bvh.intersect(o, d, 0.0, f64::INFINITY) else {
                return t;
            };
            let Some(tri) = self.bvh.tris.get(h.tri) else {
                return V3::ZERO;
            };
            let Some(m) = self
                .materials
                .get(tri.material as usize)
                .filter(|m| m.is_glass())
            else {
                return V3::ZERO;
            };
            let cos = tri.ng.dot(d).abs();
            let f = bsdf::fresnel_dielectric(cos, 1.0, f64::from(m.ior));
            let tint = V3::rgb(m.base);
            t = t * v3(tint.x.sqrt(), tint.y.sqrt(), tint.z.sqrt()) * (1.0 - f);
            o = o + d * h.t + d * self.eps;
        }
        V3::ZERO
    }

    /// The lights' radiance towards `d`, each weighted against its light sampling.
    fn lights_seen(&self, d: V3, pdf_bsdf: Option<f64>) -> V3 {
        let mut out = V3::ZERO;
        for light in &self.env.lights {
            if d.dot(light.direction.into()) < light.cos_max {
                continue;
            }
            let w = match pdf_bsdf {
                Some(p) => Self::mis(p, 1.0 / light.solid_angle()),
                None => 1.0,
            };
            out += V3::rgb(light.radiance) * w;
        }
        out
    }

    /// Radiance along one camera ray.
    fn trace(&self, mut o: V3, mut d: V3, rng: &mut Rng) -> V3 {
        let mut radiance = V3::ZERO;
        let mut throughput = V3::ONE;
        // The pdf of the BSDF sample that made this ray, or `None` for the camera ray and
        // after a mirror-like (glass) bounce, which light sampling cannot reach.
        let mut pdf_bsdf: Option<f64> = None;
        // Light that reached an opaque surface through glass is already counted by its
        // light sampling (`transmittance`), so such a path seeing a light adds nothing:
        // no double counting, and none of the fireflies of caustics found by chance.
        let mut through_glass_from_opaque = false;
        let mut had_opaque = false;
        for depth in 0..MAX_BOUNCES {
            let Some(met) = self.intersect(o, d) else {
                let sky = V3::rgb(self.env.sky(d.into_vec()));
                let lights = if through_glass_from_opaque {
                    V3::ZERO
                } else {
                    self.lights_seen(d, pdf_bsdf)
                };
                radiance += clamp_indirect(throughput * (sky + lights), depth);
                break;
            };
            let v = -d;
            let (p, n, ng, opaque, glass) = match met {
                Met::Floor(t) => {
                    let p = o + d * t;
                    if depth == 0 {
                        // The floor fades into the sky far from the part.
                        let off = v3(p.x - self.centre.x, p.y - self.centre.y, 0.0).length();
                        let w = f64::from(Environment::floor_weight(off, self.radius));
                        let bg = V3::rgb(self.env.sky(d.into_vec()));
                        radiance += throughput * bg * (1.0 - w);
                        throughput = throughput * w;
                        if w <= 0.0 {
                            break;
                        }
                    }
                    let albedo = V3::rgb(self.env.floor.unwrap_or([0.5; 3]));
                    let up = v3(0.0, 0.0, 1.0);
                    (p, up, up, Opaque::lambert(albedo), None)
                }
                Met::Triangle(h) => {
                    let Some(tri) = self.bvh.tris.get(h.tri) else {
                        break;
                    };
                    let p = o + d * h.t;
                    let mut ng = tri.ng;
                    let mut n = tri.shading_normal(h.u, h.v);
                    let entering = ng.dot(d) < 0.0;
                    if !entering {
                        ng = -ng;
                    }
                    if n.dot(ng) < 0.0 {
                        n = -n;
                    }
                    let i = tri.material as usize;
                    let m = self.materials.get(i).copied();
                    match (m, self.opaque.get(i)) {
                        (Some(m), _) if m.is_glass() => {
                            (p, n, ng, Opaque::lambert(V3::ZERO), Some((m, entering)))
                        }
                        (_, Some(b)) => (p, n, ng, *b, None),
                        _ => break,
                    }
                }
            };
            if let Some((m, entering)) = glass {
                let (dir, through) = bsdf::dielectric(n, v, entering, f64::from(m.ior), rng.f());
                if through {
                    // Tinted once per crossing of the surface: twice through a slab.
                    let tint = V3::rgb(m.base);
                    throughput = throughput * v3(tint.x.sqrt(), tint.y.sqrt(), tint.z.sqrt());
                    o = p - ng * self.eps;
                } else {
                    o = p + ng * self.eps;
                }
                d = dir;
                pdf_bsdf = None;
                through_glass_from_opaque = had_opaque;
            } else {
                had_opaque = true;
                through_glass_from_opaque = false;
                // Shading normals can face away from the viewer on silhouettes.
                let n = if n.dot(v) <= 0.0 { ng } else { n };
                radiance +=
                    clamp_indirect(throughput * self.direct(p, n, ng, v, &opaque, rng), depth);
                let Some(s) = opaque.sample(n, v, rng.f(), rng.f(), rng.f()) else {
                    break;
                };
                if s.dir.dot(ng) <= 0.0 {
                    break;
                }
                throughput = throughput * s.weight;
                o = p + ng * self.eps;
                d = s.dir;
                pdf_bsdf = Some(s.pdf);
            }
            if depth >= 3 {
                let keep = throughput.max_component().clamp(0.05, 0.95);
                if rng.f() >= keep {
                    break;
                }
                throughput = throughput / keep;
            }
            if !throughput.is_finite() {
                break;
            }
        }
        if radiance.is_finite() {
            radiance
        } else {
            V3::ZERO
        }
    }
}

/// Light found after the first bounce is capped at this radiance: rare paths that find
/// a small bright light through a glossy bounce (a caustic) would otherwise leave single
/// white pixels for thousands of samples. Slightly biased, like every production clamp.
const INDIRECT_CLAMP: f64 = 3.0;

fn clamp_indirect(c: V3, depth: u32) -> V3 {
    let m = c.max_component();
    if depth == 0 || m <= INDIRECT_CLAMP {
        c
    } else {
        c * (INDIRECT_CLAMP / m)
    }
}

impl V3 {
    fn into_vec(self) -> chamfera_geom::Vec3 {
        chamfera_geom::Vec3::new(self.x, self.y, self.z)
    }
}

struct Accum {
    sum: Vec<[f32; 3]>,
    samples: u32,
}

/// A progressive render: [`Render::step`] adds a sample per pixel until the target.
pub struct Render {
    world: World,
    view: View,
    settings: Settings,
    accum: Mutex<Accum>,
    /// One pass at a time.
    pass: Mutex<()>,
    cancelled: AtomicBool,
}

impl std::fmt::Debug for Render {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Render")
            .field("settings", &self.settings)
            .field("samples", &self.samples())
            .finish()
    }
}

impl Render {
    /// Prepares a render of `bodies` seen by `camera` in `env`. The floor (if the
    /// environment has one) is under the lowest body.
    pub fn new(
        bodies: &[Body],
        camera: &Camera,
        env: Environment,
        settings: Settings,
    ) -> Result<Render, RenderError> {
        settings.check()?;
        let count: usize = bodies.iter().map(|b| b.mesh.triangles.len()).sum();
        if count > MAX_TRIANGLES {
            return Err(RenderError::TooManyTriangles(count));
        }
        let mut tris = Vec::with_capacity(count);
        let mut materials = Vec::with_capacity(bodies.len());
        let inf = V3::splat(f64::INFINITY);
        let (mut lo, mut hi) = (inf, -inf);
        for (i, b) in bodies.iter().enumerate() {
            materials.push(b.material);
            let normals = corner_normals(b.mesh, CREASE);
            for (t, n) in normals.iter().enumerate() {
                let Some([a, bb, c]) = b.mesh.triangle(t) else {
                    continue;
                };
                let (a, bb, c) = (V3::from(a), V3::from(bb), V3::from(c));
                if !(a.is_finite() && bb.is_finite() && c.is_finite()) {
                    continue;
                }
                if let Some(tri) = Tri::new(a, bb, c, n.map(V3::from), i as u32) {
                    lo = lo.min(a).min(bb).min(c);
                    hi = hi.max(a).max(bb).max(c);
                    tris.push(tri);
                }
            }
        }
        let (centre, radius, floor_z) = if tris.is_empty() {
            (V3::ZERO, 50.0, 0.0)
        } else {
            let c = (lo + hi) * 0.5;
            ((c), ((hi - lo) * 0.5).length().max(1e-3), lo.z)
        };
        let scale = radius + centre.length();
        let opaque = materials.iter().map(Opaque::new).collect();
        let world = World {
            bvh: Bvh::build(tris),
            opaque,
            materials,
            env,
            // Just under the lowest point, so a part standing on it does not z-fight.
            floor_z: floor_z - radius * 1e-4,
            centre,
            radius,
            eps: scale * 1e-7 + 1e-9,
        };
        let view = View::new(camera, settings.width, settings.height, radius);
        let pixels = settings.width as usize * settings.height as usize;
        Ok(Render {
            world,
            view,
            settings,
            accum: Mutex::new(Accum {
                sum: vec![[0.0; 3]; pixels],
                samples: 0,
            }),
            pass: Mutex::new(()),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Samples per pixel so far.
    pub fn samples(&self) -> u32 {
        self.accum.lock().map_or(0, |a| a.samples)
    }

    pub fn done(&self) -> bool {
        self.samples() >= self.settings.samples || self.is_cancelled()
    }

    /// Stops the render after the pass under way.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn threads(&self) -> usize {
        if cfg!(target_arch = "wasm32") {
            return 1;
        }
        let n = match self.settings.threads {
            0 => std::thread::available_parallelism().map_or(1, usize::from),
            n => n,
        };
        n.clamp(1, MAX_THREADS)
    }

    /// One row of one pass.
    fn row(&self, y: usize, sample: u32, out: &mut [[f32; 3]]) {
        let (w, h) = (self.settings.width as usize, self.settings.height);
        for (x, px) in out.iter_mut().enumerate() {
            let index = (y * w + x) as u64;
            let mut rng = Rng::new(
                self.settings.seed ^ u64::from(sample).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                index,
            );
            // A random point in the pixel (box filter).
            let fx = (x as f64 + rng.f()) / w as f64 * 2.0 - 1.0;
            let fy = 1.0 - (y as f64 + rng.f()) / f64::from(h) * 2.0;
            let (o, d) = self.view.ray(fx, fy);
            *px = self.world.trace(o, d, &mut rng).to_f32();
        }
    }

    /// Adds one sample to every pixel (unless done) and returns the samples so far.
    pub fn step(&self) -> u32 {
        let Ok(_pass) = self.pass.lock() else {
            return self.samples();
        };
        if self.done() {
            return self.samples();
        }
        let sample = self.samples();
        let (w, h) = (self.settings.width as usize, self.settings.height as usize);
        let mut pass = vec![[0f32; 3]; w * h];
        let threads = self.threads();
        if threads <= 1 {
            for (y, row) in pass.chunks_mut(w).enumerate() {
                self.row(y, sample, row);
            }
        } else {
            // Rows are handed out one at a time, so threads that finish early take more.
            let next = AtomicUsize::new(0);
            let rows: Vec<Mutex<&mut [[f32; 3]]>> = pass.chunks_mut(w).map(Mutex::new).collect();
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    let (next, rows) = (&next, &rows);
                    let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                        loop {
                            let y = next.fetch_add(1, Ordering::Relaxed);
                            let Some(row) = rows.get(y) else { break };
                            if let Ok(mut row) = row.lock() {
                                self.row(y, sample, &mut row);
                            }
                        }
                    });
                    if spawned.is_err() {
                        break;
                    }
                }
            });
            // Rows no thread took (if none could start) are done here.
            loop {
                let y = next.fetch_add(1, Ordering::Relaxed);
                let Some(row) = rows.get(y) else { break };
                if let Ok(mut row) = row.lock() {
                    self.row(y, sample, &mut row);
                }
            }
        }
        if let Ok(mut a) = self.accum.lock() {
            for (s, p) in a.sum.iter_mut().zip(&pass) {
                for k in 0..3 {
                    s[k] += p[k];
                }
            }
            a.samples += 1;
            a.samples
        } else {
            sample
        }
    }

    /// The mean radiance of every pixel so far (linear, before exposure), row by row from
    /// the top.
    pub fn radiance(&self) -> Vec<[f32; 3]> {
        let Ok(a) = self.accum.lock() else {
            return Vec::new();
        };
        let k = 1.0 / a.samples.max(1) as f32;
        a.sum.iter().map(|p| p.map(|v| v * k)).collect()
    }

    /// The picture so far: tone-mapped sRGB, RGBA, row by row from the top.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.radiance().len() * 4);
        let samples = self.samples();
        for p in self.radiance() {
            let c = if samples == 0 {
                self.world.env.display([0.0; 3])
            } else {
                self.world.env.display(p)
            };
            out.extend([c.0, c.1, c.2, 255]);
        }
        out
    }

    /// The picture so far as a PNG file.
    pub fn png(&self) -> Result<Vec<u8>, RenderError> {
        encode_png(self.settings.width, self.settings.height, &self.rgba())
    }
}

/// RGBA pixels to a PNG file.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, RenderError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| RenderError::Png(e.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| RenderError::Png(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

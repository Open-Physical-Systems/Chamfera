//! The procedural lighting environment of the realistic style and the path tracer: a sky
//! gradient, up to three soft disc lights (key, fill, rim) and a floor that catches
//! shadows. No image files: everything is a few numbers, the same for both renderers.
//!
//! The sky's colours and the lights' strengths are rendering data (like materials), not
//! theme colours. Radiance is in linear RGB, in arbitrary units scaled by `exposure`
//! before tone mapping.

use crate::{Camera, Rgba};
use chamfera_geom::Vec3;
use serde::{Deserialize, Serialize};

/// The environment presets (`render.environment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPreset {
    /// A light grey studio: soft key, fill and rim lights over a pale floor.
    #[default]
    Studio,
    /// A blue sky over a pale horizon, with one small warm sun.
    Outdoor,
    /// A dark room with a strong key and rim: for bright and metal parts.
    Dark,
}

impl EnvironmentPreset {
    pub const ALL: [EnvironmentPreset; 3] = [
        EnvironmentPreset::Studio,
        EnvironmentPreset::Outdoor,
        EnvironmentPreset::Dark,
    ];

    pub fn id(self) -> &'static str {
        match self {
            EnvironmentPreset::Studio => "studio",
            EnvironmentPreset::Outdoor => "outdoor",
            EnvironmentPreset::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EnvironmentPreset::Studio => "Studio",
            EnvironmentPreset::Outdoor => "Outdoor",
            EnvironmentPreset::Dark => "Dark Studio",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            EnvironmentPreset::Studio => "Soft key, fill and rim lights over a pale floor",
            EnvironmentPreset::Outdoor => "Blue sky and a small warm sun",
            EnvironmentPreset::Dark => "A dark room with a strong key and rim light",
        }
    }

    pub fn parse(s: &str) -> Option<EnvironmentPreset> {
        EnvironmentPreset::ALL.into_iter().find(|p| p.id() == s)
    }
}

/// A light far away: a disc of constant radiance in the sky.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Unit vector towards the light.
    pub direction: Vec3,
    /// Radiance of the disc (linear RGB).
    pub radiance: [f32; 3],
    /// Cosine of the disc's angular radius.
    pub cos_max: f64,
}

impl Light {
    /// A light that gives `irradiance` to a surface facing it, from a disc of angular
    /// radius `radius` (radians). A cone of half angle t seen head on gives
    /// E = L * pi * sin^2(t) (integrate L cos(theta) over the cone), so L = E / (pi sin^2 t).
    pub fn from_irradiance(direction: Vec3, irradiance: [f32; 3], radius: f64) -> Option<Light> {
        let direction = direction.normalized()?;
        let r = radius.clamp(1e-3, 0.5);
        let s = r.sin();
        let k = (1.0 / (std::f64::consts::PI * s * s)) as f32;
        Some(Light {
            direction,
            radiance: irradiance.map(|e| e * k),
            cos_max: r.cos(),
        })
    }

    /// The solid angle of the disc: 2 pi (1 - cos_max).
    pub fn solid_angle(&self) -> f64 {
        std::f64::consts::TAU * (1.0 - self.cos_max)
    }

    /// Whether the direction `d` (unit) points into the disc.
    pub fn contains(&self, d: Vec3) -> bool {
        d.dot(self.direction) >= self.cos_max
    }

    /// Irradiance on a surface facing the light: L * pi * (1 - cos_max^2).
    pub fn irradiance(&self) -> [f32; 3] {
        let k = (std::f64::consts::PI * (1.0 - self.cos_max * self.cos_max)) as f32;
        self.radiance.map(|l| l * k)
    }
}

/// Most lights an environment has (the GPU shader has room for this many).
pub const MAX_LIGHTS: usize = 3;

/// Everything the renderers need to light a scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    /// Sky radiance straight up, at the horizon and straight down.
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub nadir: [f32; 3],
    /// The floor's albedo (linear), or `None` for no floor.
    pub floor: Option<[f32; 3]>,
    pub lights: Vec<Light>,
    /// Multiplies the radiance before tone mapping.
    pub exposure: f32,
}

/// The floor fades into the sky between these multiples of the scene's radius from its
/// centre, so it has no visible far edge.
pub const FLOOR_FADE_START: f64 = 3.0;
pub const FLOOR_FADE_END: f64 = 10.0;

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    c.map(|v| v * k)
}

/// A direction from angles in the camera's frame: `azimuth` from the view direction back
/// towards the eye (positive to the camera's right), `elevation` up from the horizon.
fn around_camera(camera: &Camera, azimuth_deg: f64, elevation_deg: f64) -> Vec3 {
    let (a, e) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    let back = Vec3::new(camera.yaw.cos(), camera.yaw.sin(), 0.0);
    let right = Vec3::new(-camera.yaw.sin(), camera.yaw.cos(), 0.0);
    (back * a.cos() + right * a.sin()) * e.cos() + Vec3::Z * e.sin()
}

impl Environment {
    /// A preset's environment. Its lights turn with the camera around Z, so a part is
    /// lit from the same side whichever way it is seen.
    pub fn preset(preset: EnvironmentPreset, camera: &Camera) -> Environment {
        let light = |az: f64, el: f64, e: [f32; 3], radius_deg: f64| {
            Light::from_irradiance(around_camera(camera, az, el), e, radius_deg.to_radians())
        };
        match preset {
            EnvironmentPreset::Studio => Environment {
                zenith: [0.26, 0.27, 0.29],
                horizon: [0.82, 0.82, 0.82],
                nadir: [0.22, 0.22, 0.22],
                floor: Some([0.55, 0.55, 0.55]),
                lights: [
                    light(-45.0, 50.0, [3.0, 2.94, 2.82], 9.0),
                    light(65.0, 20.0, [0.7, 0.72, 0.78], 14.0),
                    light(165.0, 35.0, [1.6, 1.6, 1.6], 7.0),
                ]
                .into_iter()
                .flatten()
                .collect(),
                exposure: 0.85,
            },
            EnvironmentPreset::Outdoor => Environment {
                zenith: [0.22, 0.42, 0.85],
                horizon: [0.80, 0.86, 0.95],
                nadir: [0.25, 0.24, 0.22],
                floor: Some([0.42, 0.42, 0.38]),
                lights: [light(-35.0, 55.0, [4.2, 3.9, 3.4], 1.5)]
                    .into_iter()
                    .flatten()
                    .collect(),
                exposure: 0.7,
            },
            EnvironmentPreset::Dark => Environment {
                zenith: [0.025, 0.027, 0.03],
                horizon: [0.05, 0.05, 0.055],
                nadir: [0.01, 0.01, 0.01],
                floor: Some([0.18, 0.18, 0.18]),
                lights: [
                    light(-50.0, 45.0, [3.2, 3.1, 3.0], 8.0),
                    light(160.0, 30.0, [2.4, 2.45, 2.6], 6.0),
                ]
                .into_iter()
                .flatten()
                .collect(),
                exposure: 1.0,
            },
        }
    }

    /// The same radiance from every direction, no lights, no floor (furnace tests).
    pub fn uniform(radiance: f32) -> Environment {
        let c = [radiance; 3];
        Environment {
            zenith: c,
            horizon: c,
            nadir: c,
            floor: None,
            lights: Vec::new(),
            exposure: 1.0,
        }
    }

    /// The sky's radiance towards the unit direction `d`, without the lights.
    pub fn sky(&self, d: Vec3) -> [f32; 3] {
        let z = d.z.clamp(-1.0, 1.0) as f32;
        if z >= 0.0 {
            mix3(self.horizon, self.zenith, z.sqrt())
        } else {
            mix3(self.horizon, self.nadir, (-z).sqrt())
        }
    }

    /// The lights' radiance towards `d` (the discs it points into).
    pub fn lights_towards(&self, d: Vec3) -> [f32; 3] {
        let mut out = [0.0; 3];
        for l in self.lights.iter().filter(|l| l.contains(d)) {
            for (o, r) in out.iter_mut().zip(l.radiance) {
                *o += r;
            }
        }
        out
    }

    /// Everything seen towards `d`: the sky and the lights.
    pub fn radiance(&self, d: Vec3) -> [f32; 3] {
        let s = self.sky(d);
        let l = self.lights_towards(d);
        [s[0] + l[0], s[1] + l[1], s[2] + l[2]]
    }

    /// How much of the floor shows at `distance` from the scene's centre (1 near, 0 far).
    pub fn floor_weight(distance: f64, radius: f64) -> f32 {
        let r = radius.max(1e-6);
        let t = ((distance / r - FLOOR_FADE_START) / (FLOOR_FADE_END - FLOOR_FADE_START))
            .clamp(0.0, 1.0);
        (1.0 - t * t * (3.0 - 2.0 * t)) as f32
    }

    /// The tone-mapped sRGB colour of a linear radiance.
    pub fn display(&self, rgb: [f32; 3]) -> Rgba {
        let [r, g, b] = scale(rgb, self.exposure).map(|v| encode_srgb(aces(v)));
        Rgba(r, g, b, 255)
    }
}

/// A filmic curve that rolls highlights off smoothly: the fit of the ACES reference
/// transform by Narkowicz (2015), x (2.51 x + 0.03) / (x (2.43 x + 0.59) + 0.14).
pub fn aces(x: f32) -> f32 {
    if !x.is_finite() || x <= 0.0 {
        return 0.0;
    }
    (x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// Linear 0..1 to an sRGB byte (IEC 61966-2-1).
pub fn encode_srgb(v: f32) -> u8 {
    let v = if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_delivers_the_irradiance_it_was_made_with() {
        // From L = E / (pi sin^2 r) back to E = L pi (1 - cos^2 r) = L pi sin^2 r.
        let l = Light::from_irradiance(Vec3::Z, [2.0, 1.0, 0.5], 0.1).unwrap();
        let e = l.irradiance();
        for (a, b) in e.iter().zip([2.0, 1.0, 0.5]) {
            assert!((a - b).abs() < 1e-4, "{a} {b}");
        }
        // Solid angle of a cap: 2 pi (1 - cos 0.1).
        assert!((l.solid_angle() - std::f64::consts::TAU * (1.0 - 0.1f64.cos())).abs() < 1e-12);
        assert!(l.contains(Vec3::Z));
        assert!(!l.contains(Vec3::X));
        assert!(Light::from_irradiance(Vec3::ZERO, [1.0; 3], 0.1).is_none());
    }

    #[test]
    fn the_sky_runs_from_horizon_to_zenith_and_presets_parse() {
        let cam = Camera::default();
        for p in EnvironmentPreset::ALL {
            assert_eq!(EnvironmentPreset::parse(p.id()), Some(p));
            let e = Environment::preset(p, &cam);
            let near =
                |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
            assert!(near(e.sky(Vec3::Z), e.zenith));
            assert!(near(e.sky(Vec3::X), e.horizon));
            assert!(near(e.sky(-Vec3::Z), e.nadir));
            assert!(!e.lights.is_empty() && e.lights.len() <= MAX_LIGHTS);
            for l in &e.lights {
                // Every light is above the horizon.
                assert!(l.direction.z > 0.0);
            }
        }
        assert_eq!(EnvironmentPreset::parse("Studio"), None);
        let u = Environment::uniform(1.0);
        assert_eq!(
            u.radiance(Vec3::new(0.3, -0.2, 0.9).normalized().unwrap()),
            [1.0; 3]
        );
    }

    #[test]
    fn tone_mapping_is_monotonic_and_ends_at_black_and_white() {
        assert_eq!(encode_srgb(0.0), 0);
        assert_eq!(encode_srgb(1.0), 255);
        assert_eq!(encode_srgb(f32::NAN), 0);
        // 0.5 linear is 0.7354 in sRGB: 1.055 * 0.5^(1/2.4) - 0.055 = 0.73536, * 255 = 187.5.
        assert!((i32::from(encode_srgb(0.5)) - 188).abs() <= 1);
        let mut last = 0.0;
        for i in 1..100 {
            let v = aces(i as f32 * 0.1);
            assert!(v >= last);
            last = v;
        }
        assert_eq!(aces(-1.0), 0.0);
        assert!(aces(1e6) <= 1.0);
        assert_eq!(Environment::floor_weight(0.0, 10.0), 1.0);
        assert_eq!(Environment::floor_weight(1000.0, 10.0), 0.0);
    }
}

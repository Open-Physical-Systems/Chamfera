//! Scattering: a Lambert diffuse layer under a GGX microfacet specular layer for metals,
//! plastics and paints, and a smooth dielectric for glass.
//!
//! - GGX distribution, Smith masking and the sampling of normals: Walter, Marschner, Li
//!   and Torrance (2007), "Microfacet Models for Refraction through Rough Surfaces".
//! - Roughness as perceptual roughness squared, and Schlick's Fresnel in the specular
//!   lobe: Karis (2013), "Real Shading in Unreal Engine 4"; Schlick (1994).
//! - The dielectric's exact Fresnel equations for unpolarised light (any optics text,
//!   for example Hecht, "Optics").
//!
//! Every direction is a unit vector pointing away from the surface; `n` is the shading
//! normal on the side the ray came from.

use crate::v3::{V3, to_world, v3};
use chamfera_render::Material;
use std::f64::consts::PI;

/// The GGX (Trowbridge-Reitz) normal distribution: D(h) = a^2 / (pi ((n.h)^2 (a^2 - 1) + 1)^2).
pub fn ggx_d(n_dot_h: f64, alpha: f64) -> f64 {
    if n_dot_h <= 0.0 {
        return 0.0;
    }
    let a2 = alpha * alpha;
    let k = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (PI * k * k)
}

/// Smith's masking for GGX, one direction: G1(v) = 2 (n.v) / ((n.v) + sqrt(a^2 + (1 - a^2) (n.v)^2)).
pub fn smith_g1(n_dot_v: f64, alpha: f64) -> f64 {
    if n_dot_v <= 0.0 {
        return 0.0;
    }
    let a2 = alpha * alpha;
    2.0 * n_dot_v / (n_dot_v + (a2 + (1.0 - a2) * n_dot_v * n_dot_v).sqrt())
}

/// Separable Smith shadowing-masking: G = G1(v) G1(l).
pub fn smith_g(n_dot_v: f64, n_dot_l: f64, alpha: f64) -> f64 {
    smith_g1(n_dot_v, alpha) * smith_g1(n_dot_l, alpha)
}

/// Schlick's Fresnel: F = F0 + (1 - F0) (1 - cos)^5.
pub fn schlick(cos: f64, f0: V3) -> V3 {
    let k = (1.0 - cos.clamp(0.0, 1.0)).powi(5);
    f0 + (V3::ONE - f0) * k
}

/// Fresnel reflectance of a smooth dielectric for unpolarised light, going from a medium
/// of index `eta_i` into one of `eta_t`, at incidence cosine `cos_i`. 1 under total
/// internal reflection.
pub fn fresnel_dielectric(cos_i: f64, eta_i: f64, eta_t: f64) -> f64 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin_t = eta_i / eta_t * (1.0 - cos_i * cos_i).max(0.0).sqrt();
    if sin_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin_t * sin_t).max(0.0).sqrt();
    let rs = (eta_i * cos_i - eta_t * cos_t) / (eta_i * cos_i + eta_t * cos_t);
    let rp = (eta_t * cos_i - eta_i * cos_t) / (eta_t * cos_i + eta_i * cos_t);
    0.5 * (rs * rs + rp * rp)
}

/// Smallest GGX alpha used (the lobe of a mirror is a spike light sampling cannot hit).
const MIN_ALPHA: f64 = 1e-3;

/// An opaque surface: Lambert under GGX.
#[derive(Debug, Clone, Copy)]
pub struct Opaque {
    pub diffuse: V3,
    pub f0: V3,
    pub alpha: f64,
    /// The chance of sampling the specular lobe rather than the diffuse one.
    pub p_spec: f64,
    /// No specular layer (F0 is zero: an index of refraction of 1).
    pub no_spec: bool,
}

/// A sampled direction with its weight f cos / pdf, and the pdf (solid angle).
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub dir: V3,
    pub weight: V3,
    pub pdf: f64,
}

impl Opaque {
    pub fn new(m: &Material) -> Opaque {
        let metallic = f64::from(m.metallic);
        let base = V3::rgb(m.base);
        let f0d = f64::from(m.dielectric_f0());
        let f0 = V3::splat(f0d) * (1.0 - metallic) + base * metallic;
        let no_spec = f0.max_component() < 1e-6;
        let p_spec = if no_spec {
            0.0
        } else {
            (metallic + (1.0 - metallic) * 0.3).clamp(0.0, 1.0)
        };
        Opaque {
            diffuse: base * (1.0 - metallic),
            f0,
            alpha: f64::from(m.alpha()).max(MIN_ALPHA),
            p_spec,
            no_spec,
        }
    }

    /// Lambert only (furnace tests).
    pub fn lambert(albedo: V3) -> Opaque {
        Opaque {
            diffuse: albedo,
            f0: V3::ZERO,
            alpha: 1.0,
            p_spec: 0.0,
            no_spec: true,
        }
    }

    /// The BRDF value f(v, l).
    pub fn eval(&self, n: V3, v: V3, l: V3) -> V3 {
        let (nv, nl) = (n.dot(v), n.dot(l));
        if nv <= 0.0 || nl <= 0.0 {
            return V3::ZERO;
        }
        let diffuse = self.diffuse * (1.0 / PI);
        if self.no_spec {
            return diffuse;
        }
        let Some(h) = (v + l).normalized() else {
            return diffuse;
        };
        let f = schlick(v.dot(h), self.f0);
        let spec =
            f * (ggx_d(n.dot(h), self.alpha) * smith_g(nv, nl, self.alpha) / (4.0 * nv * nl));
        // Light that enters the diffuse layer has passed the coat twice: in towards l
        // and out towards v (symmetric, so the BRDF stays reciprocal).
        let through = (V3::ONE - schlick(nv, self.f0)) * (V3::ONE - schlick(nl, self.f0));
        diffuse * through + spec
    }

    /// The pdf of sampling `l` from `v`.
    pub fn pdf(&self, n: V3, v: V3, l: V3) -> f64 {
        let (nv, nl) = (n.dot(v), n.dot(l));
        if nv <= 0.0 || nl <= 0.0 {
            return 0.0;
        }
        let diffuse = nl / PI;
        if self.p_spec <= 0.0 {
            return diffuse;
        }
        let spec = match (v + l).normalized() {
            Some(h) => {
                let vh = v.dot(h);
                if vh <= 0.0 {
                    0.0
                } else {
                    ggx_d(n.dot(h), self.alpha) * n.dot(h) / (4.0 * vh)
                }
            }
            None => 0.0,
        };
        self.p_spec * spec + (1.0 - self.p_spec) * diffuse
    }

    /// Samples a direction: the specular lobe by the GGX distribution of normals (Walter
    /// et al. 2007, eqs. 35 and 36), the diffuse one by the cosine.
    pub fn sample(&self, n: V3, v: V3, u0: f64, u1: f64, u2: f64) -> Option<Sample> {
        let l = if u0 < self.p_spec {
            let a2 = self.alpha * self.alpha;
            // tan^2 theta_h = a^2 u / (1 - u).
            let cos_h = ((1.0 - u1) / (1.0 + (a2 - 1.0) * u1)).max(0.0).sqrt();
            let sin_h = (1.0 - cos_h * cos_h).max(0.0).sqrt();
            let phi = 2.0 * PI * u2;
            let h = to_world(n, v3(sin_h * phi.cos(), sin_h * phi.sin(), cos_h));
            let vh = v.dot(h);
            if vh <= 0.0 {
                return None;
            }
            h * (2.0 * vh) - v
        } else {
            let r = u1.sqrt();
            let phi = 2.0 * PI * u2;
            to_world(
                n,
                v3(r * phi.cos(), r * phi.sin(), (1.0 - u1).max(0.0).sqrt()),
            )
        };
        let l = l.normalized()?;
        let pdf = self.pdf(n, v, l);
        if pdf <= 0.0 {
            return None;
        }
        let weight = self.eval(n, v, l) * (n.dot(l) / pdf);
        weight.is_finite().then_some(Sample {
            dir: l,
            weight,
            pdf,
        })
    }
}

/// What a smooth dielectric does to a ray: reflect or refract, chosen by the Fresnel
/// reflectance. Returns the new direction and whether it went through.
pub fn dielectric(n: V3, v: V3, entering: bool, ior: f64, u: f64) -> (V3, bool) {
    let (eta_i, eta_t) = if entering { (1.0, ior) } else { (ior, 1.0) };
    let cos_i = n.dot(v).clamp(0.0, 1.0);
    let f = fresnel_dielectric(cos_i, eta_i, eta_t);
    let reflect = n * (2.0 * cos_i) - v;
    if u < f {
        return (reflect, false);
    }
    let eta = eta_i / eta_t;
    let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
    if k < 0.0 {
        return (reflect, false);
    }
    let t = (-v) * eta + n * (eta * cos_i - k.sqrt());
    (t.normalized().unwrap_or(reflect), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn ggx_d_matches_hand_values() {
        // At n.h = 1: D = a^2 / (pi a^4) = 1 / (pi a^2). a = 0.5: 1 / (0.25 pi) = 1.273240.
        assert!(close(ggx_d(1.0, 0.5), 1.273_239_5, 1e-6));
        // a = 0.5, n.h = cos 60 = 0.5: k = 0.25 (0.25 - 1) + 1 = 0.8125;
        // D = 0.25 / (pi 0.66015625) = 0.120543.
        assert!(close(ggx_d(0.5, 0.5), 0.120_543, 1e-6));
        // a = 1 is uniform over the hemisphere of normals: D = 1 / pi everywhere.
        for c in [0.1, 0.5, 0.9] {
            assert!(close(ggx_d(c, 1.0), 1.0 / PI, 1e-12));
        }
        assert_eq!(ggx_d(-0.2, 0.5), 0.0);
    }

    #[test]
    fn ggx_d_is_normalised_over_projected_normals() {
        // The integral of D(h) (n.h) over the hemisphere is 1 (Walter et al. eq. 4). In
        // polar form: 2 pi int_0^{pi/2} D(cos t) cos t sin t dt, by the midpoint rule.
        for alpha in [0.1, 0.3, 0.7, 1.0] {
            let steps = 200_000;
            let dt = std::f64::consts::FRAC_PI_2 / steps as f64;
            let mut sum = 0.0;
            for i in 0..steps {
                let t = (i as f64 + 0.5) * dt;
                sum += ggx_d(t.cos(), alpha) * t.cos() * t.sin() * dt;
            }
            assert!(
                close(2.0 * PI * sum, 1.0, 1e-3),
                "{alpha}: {}",
                2.0 * PI * sum
            );
        }
    }

    #[test]
    fn smith_g_matches_hand_values() {
        // Head on there is no masking.
        assert!(close(smith_g1(1.0, 0.5), 1.0, 1e-12));
        // a = 0.5, n.v = 0.5: 1 / (0.5 + sqrt(0.25 + 0.75 * 0.25)) = 1 / (0.5 + 0.661438)
        // = 1 / 1.161438 = 0.861002.
        assert!(close(smith_g1(0.5, 0.5), 0.861_002, 1e-6));
        // G = G1(v) G1(l): 0.861002^2 = 0.741324.
        assert!(close(smith_g(0.5, 0.5, 0.5), 0.741_324, 1e-6));
        // A perfectly smooth surface (a -> 0) masks nothing above the horizon:
        // 2c / (c + c) = 1.
        assert!(close(smith_g1(0.3, 1e-9), 1.0, 1e-9));
        assert_eq!(smith_g1(0.0, 0.5), 0.0);
    }

    #[test]
    fn schlick_and_exact_fresnel_match_hand_values() {
        let f0 = V3::splat(0.04);
        assert!(close(schlick(1.0, f0).x, 0.04, 1e-12));
        assert!(close(schlick(0.0, f0).x, 1.0, 1e-12));
        // cos = 0.5: 0.04 + 0.96 / 32 = 0.07.
        assert!(close(schlick(0.5, f0).x, 0.07, 1e-12));
        // Glass head on: ((1.5 - 1) / (1.5 + 1))^2 = 0.04.
        assert!(close(fresnel_dielectric(1.0, 1.0, 1.5), 0.04, 1e-12));
        // At grazing, everything reflects.
        assert!(close(fresnel_dielectric(0.0, 1.0, 1.5), 1.0, 1e-12));
        // Brewster's angle, tan i = 1.5: rp = 0, so R = 0.5 rs^2 with
        // rs = (cos i - 1.5 cos t) / (cos i + 1.5 cos t): i = 56.31 deg, cos i = 0.5547, sin i = 0.8321, sin t = 0.5547, cos t = 0.8321;
        // rs = (0.5547 - 1.24815) / (0.5547 + 1.24815) = -0.38462; R = 0.5 * 0.14793 = 0.07396.
        let cos_b = 1.0 / (1.0 + 1.5f64 * 1.5).sqrt();
        assert!(close(fresnel_dielectric(cos_b, 1.0, 1.5), 0.073_96, 1e-4));
        // From inside past the critical angle (sin = 1 / 1.5): total internal reflection.
        assert_eq!(fresnel_dielectric(0.5, 1.5, 1.0), 1.0);
    }

    /// Directional albedo int f cos dw, estimated with the sampler on a fixed stratified
    /// grid: each lobe's mean weight f cos / pdf, times the chance of picking that lobe.
    fn albedo(b: &Opaque, theta: f64) -> V3 {
        let n = v3(0.0, 0.0, 1.0);
        let v = v3(theta.sin(), 0.0, theta.cos());
        let k = 200;
        let mut est = V3::ZERO;
        for (u0, chance) in [(0.0, b.p_spec), (0.999_999, 1.0 - b.p_spec)] {
            if chance <= 0.0 {
                continue;
            }
            let mut sum = V3::ZERO;
            for i in 0..k {
                for j in 0..k {
                    let (u1, u2) = ((i as f64 + 0.5) / k as f64, (j as f64 + 0.5) / k as f64);
                    if let Some(s) = b.sample(n, v, u0, u1, u2) {
                        sum += s.weight;
                    }
                }
            }
            est += sum * (chance / (k * k) as f64);
        }
        est
    }

    #[test]
    fn metals_never_reflect_more_energy_than_they_receive() {
        // A white metal (F0 = 1): no light is absorbed, so the directional albedo is at
        // most 1; single-scattering GGX loses some energy at high roughness, so it is
        // below 1. At roughness 1 (a = 1) seen head on it is known exactly: D = 1 / pi,
        // G1(v) = 1, G1(l) = 2c / (c + 1) with c = n.l, so
        // int f cos dw = int (1 / (4 pi)) 2c / (c + 1) dw = (1 / 2) int_0^1 2c / (c + 1) dc
        // = 1 - ln 2 = 0.306853.
        for roughness in [0.1f32, 0.3, 0.6, 1.0] {
            let m = Material::new(
                chamfera_render::Rgba(255, 255, 255, 255),
                1.0,
                roughness,
                0.0,
                1.5,
            );
            let b = Opaque::new(&m);
            for deg in [0.0f64, 30.0, 60.0, 80.0] {
                let a = albedo(&b, deg.to_radians());
                assert!(
                    a.max_component() <= 1.0 + 1e-2,
                    "r {roughness} at {deg}: {a:?}"
                );
                assert!(a.x >= 0.3, "r {roughness} at {deg}: {a:?}");
            }
        }
        let white = chamfera_render::Rgba(255, 255, 255, 255);
        let rough = Opaque::new(&Material::new(white, 1.0, 1.0, 0.0, 1.5));
        let a = albedo(&rough, 0.0).x;
        assert!(close(a, 1.0 - 2f64.ln(), 2e-3), "{a}");
        // A smooth metal reflects nearly everything: by numerical integration of the same
        // terms, a = 0.09 (roughness 0.3) head on gives 0.9907.
        let smooth = Opaque::new(&Material::new(white, 1.0, 0.3, 0.0, 1.5));
        assert!(close(albedo(&smooth, 0.0).x, 0.9907, 1e-2));
        // A plastic: diffuse plus coat stays at most 1 too.
        let m = Material::new(
            chamfera_render::Rgba(255, 255, 255, 255),
            0.0,
            0.4,
            0.0,
            1.5,
        );
        let b = Opaque::new(&m);
        for deg in [0.0f64, 45.0, 75.0] {
            assert!(albedo(&b, deg.to_radians()).max_component() <= 1.0 + 1e-2);
        }
    }

    #[test]
    fn a_lambert_surface_reflects_its_albedo() {
        // int (rho / pi) cos dw = rho.
        let b = Opaque::lambert(V3::splat(0.6));
        assert!(close(albedo(&b, 0.4).x, 0.6, 1e-9));
    }

    #[test]
    fn glass_refracts_by_snell_and_reflects_head_on_a_little() {
        // 30 degrees in, n = 1.5: sin t = 0.5 / 1.5 = 1/3.
        let n = v3(0.0, 0.0, 1.0);
        let v = v3(0.5, 0.0, 3f64.sqrt() / 2.0);
        let (t, through) = dielectric(n, v, true, 1.5, 0.99);
        assert!(through);
        assert!(close(t.x, -1.0 / 3.0, 1e-12), "{t:?}");
        assert!(t.z < 0.0);
        let (r, through) = dielectric(n, v, true, 1.5, 0.0);
        assert!(!through);
        assert!(close(r.x, -0.5, 1e-12) && close(r.z, v.z, 1e-12));
    }
}

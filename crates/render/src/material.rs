//! Physically based materials: base colour, metallic, roughness, and transmission with an
//! index of refraction for glass. The viewport's realistic style (`chamfera-gpu`) and the
//! offline path tracer (`chamfera-raytrace`) read the same values, so the two pictures
//! agree.
//!
//! The library's colours and numbers are material data (what a part is made of), not
//! theme colours, so they live here and not in `theme.rs`. They are our own choices,
//! picked by eye against the studio environment; metals take their base colour as the
//! reflectance at normal incidence (F0).

use crate::Rgba;

/// Index of refraction of common plastics, paints and glass (F0 = 4%).
pub const DEFAULT_IOR: f32 = 1.5;
/// Roughness of a body that has no material: a satin plastic.
pub const DEFAULT_ROUGHNESS: f32 = 0.5;
/// Smallest roughness used in shading: below it the GGX lobe is a spike that a
/// rasteriser aliases and a path tracer cannot find with light sampling.
pub const MIN_ROUGHNESS: f32 = 0.03;
/// Range of the index of refraction.
pub const MIN_IOR: f32 = 1.0;
pub const MAX_IOR: f32 = 3.0;

/// A material ready for shading: colours in linear light, every number in range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Linear RGB, 0..1: the diffuse albedo, or F0 for metals.
    pub base: [f32; 3],
    /// 0 dielectric, 1 metal.
    pub metallic: f32,
    /// Perceptual roughness, `MIN_ROUGHNESS..=1`; the GGX alpha is its square (Burley
    /// 2012, also Karis 2013).
    pub roughness: f32,
    /// 0 opaque, 1 fully transmissive (glass).
    pub transmission: f32,
    pub ior: f32,
}

impl Material {
    /// A material from loose numbers: non-finite values take their defaults, the rest are
    /// clamped into range.
    pub fn new(
        colour: Rgba,
        metallic: f32,
        roughness: f32,
        transmission: f32,
        ior: f32,
    ) -> Material {
        let unit = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        let l = colour.linear();
        Material {
            base: [l[0], l[1], l[2]],
            metallic: unit(metallic, 0.0),
            roughness: unit(roughness, DEFAULT_ROUGHNESS).max(MIN_ROUGHNESS),
            transmission: unit(transmission, 0.0),
            ior: if ior.is_finite() {
                ior.clamp(MIN_IOR, MAX_IOR)
            } else {
                DEFAULT_IOR
            },
        }
    }

    /// The GGX alpha (roughness squared).
    pub fn alpha(&self) -> f32 {
        self.roughness * self.roughness
    }

    /// Reflectance at normal incidence of the dielectric part, from the IOR (Fresnel at
    /// zero angle): ((n - 1) / (n + 1))^2.
    pub fn dielectric_f0(&self) -> f32 {
        let r = (self.ior - 1.0) / (self.ior + 1.0);
        r * r
    }

    /// Whether it is drawn and traced as glass.
    pub fn is_glass(&self) -> bool {
        self.transmission >= 0.5
    }
}

/// A group of the material library, for the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Metal,
    Plastic,
    Rubber,
    Glass,
    Wood,
    Paint,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Metal,
        Category::Plastic,
        Category::Rubber,
        Category::Glass,
        Category::Wood,
        Category::Paint,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Metal => "Metals",
            Category::Plastic => "Plastics",
            Category::Rubber => "Rubber",
            Category::Glass => "Glass",
            Category::Wood => "Wood",
            Category::Paint => "Paint",
        }
    }
}

/// A named material of the library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LibraryMaterial {
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
    /// sRGB.
    pub colour: Rgba,
    pub metallic: f32,
    pub roughness: f32,
    pub transmission: f32,
    pub ior: f32,
    /// How opaque the plain (not realistic) styles draw it: glass is see-through there.
    pub opacity: f32,
}

#[allow(clippy::too_many_arguments)]
const fn m(
    id: &'static str,
    label: &'static str,
    category: Category,
    colour: Rgba,
    metallic: f32,
    roughness: f32,
    transmission: f32,
    opacity: f32,
) -> LibraryMaterial {
    LibraryMaterial {
        id,
        label,
        category,
        colour,
        metallic,
        roughness,
        transmission,
        ior: DEFAULT_IOR,
        opacity,
    }
}

const fn with_ior(mut x: LibraryMaterial, ior: f32) -> LibraryMaterial {
    x.ior = ior;
    x
}

use Category::{Glass as G, Metal as M, Paint as P, Plastic as L, Rubber as R, Wood as W};

/// The material library. The first ten ids are the appearance presets of design format
/// 21, kept so older designs keep their names.
pub const LIBRARY: &[LibraryMaterial] = &[
    m(
        "steel",
        "Steel",
        M,
        Rgba(168, 172, 178, 255),
        1.0,
        0.35,
        0.0,
        1.0,
    ),
    m(
        "aluminium",
        "Aluminium",
        M,
        Rgba(206, 210, 214, 255),
        1.0,
        0.3,
        0.0,
        1.0,
    ),
    m(
        "brass",
        "Brass",
        M,
        Rgba(214, 176, 92, 255),
        1.0,
        0.22,
        0.0,
        1.0,
    ),
    m(
        "copper",
        "Copper",
        M,
        Rgba(204, 128, 88, 255),
        1.0,
        0.28,
        0.0,
        1.0,
    ),
    m(
        "paint_red",
        "Painted Red",
        P,
        Rgba(186, 40, 36, 255),
        0.0,
        0.32,
        0.0,
        1.0,
    ),
    m(
        "paint_blue",
        "Painted Blue",
        P,
        Rgba(36, 84, 168, 255),
        0.0,
        0.32,
        0.0,
        1.0,
    ),
    m(
        "paint_yellow",
        "Painted Yellow",
        P,
        Rgba(226, 178, 30, 255),
        0.0,
        0.32,
        0.0,
        1.0,
    ),
    m(
        "paint_black",
        "Painted Black",
        P,
        Rgba(38, 40, 44, 255),
        0.0,
        0.3,
        0.0,
        1.0,
    ),
    with_ior(
        m(
            "glass",
            "Glass",
            G,
            Rgba(214, 236, 240, 255),
            0.0,
            0.03,
            1.0,
            0.3,
        ),
        1.5,
    ),
    m(
        "rubber",
        "Rubber",
        R,
        Rgba(34, 34, 36, 255),
        0.0,
        0.85,
        0.0,
        1.0,
    ),
    m(
        "stainless",
        "Brushed Stainless",
        M,
        Rgba(190, 192, 194, 255),
        1.0,
        0.42,
        0.0,
        1.0,
    ),
    m(
        "aluminium_polished",
        "Polished Aluminium",
        M,
        Rgba(228, 230, 232, 255),
        1.0,
        0.1,
        0.0,
        1.0,
    ),
    m(
        "chrome",
        "Chrome",
        M,
        Rgba(232, 234, 238, 255),
        1.0,
        0.05,
        0.0,
        1.0,
    ),
    m(
        "gold",
        "Gold",
        M,
        Rgba(236, 190, 92, 255),
        1.0,
        0.18,
        0.0,
        1.0,
    ),
    m(
        "titanium",
        "Titanium",
        M,
        Rgba(162, 154, 144, 255),
        1.0,
        0.4,
        0.0,
        1.0,
    ),
    m(
        "cast_iron",
        "Cast Iron",
        M,
        Rgba(98, 100, 102, 255),
        1.0,
        0.65,
        0.0,
        1.0,
    ),
    m(
        "plastic_white",
        "White Plastic",
        L,
        Rgba(232, 232, 228, 255),
        0.0,
        0.4,
        0.0,
        1.0,
    ),
    m(
        "plastic_grey",
        "Grey Plastic",
        L,
        Rgba(124, 128, 134, 255),
        0.0,
        0.5,
        0.0,
        1.0,
    ),
    m(
        "plastic_black",
        "Matte Black Plastic",
        L,
        Rgba(30, 30, 33, 255),
        0.0,
        0.7,
        0.0,
        1.0,
    ),
    m(
        "plastic_red_gloss",
        "Glossy Red Plastic",
        L,
        Rgba(200, 34, 30, 255),
        0.0,
        0.12,
        0.0,
        1.0,
    ),
    m(
        "rubber_grey",
        "Grey Rubber",
        R,
        Rgba(92, 94, 98, 255),
        0.0,
        0.9,
        0.0,
        1.0,
    ),
    with_ior(
        m(
            "glass_tinted",
            "Tinted Glass",
            G,
            Rgba(150, 204, 180, 255),
            0.0,
            0.03,
            1.0,
            0.4,
        ),
        1.5,
    ),
    with_ior(
        m(
            "acrylic_smoke",
            "Smoked Acrylic",
            G,
            Rgba(120, 120, 128, 255),
            0.0,
            0.03,
            1.0,
            0.5,
        ),
        1.49,
    ),
    m(
        "wood_oak",
        "Oak",
        W,
        Rgba(170, 128, 84, 255),
        0.0,
        0.6,
        0.0,
        1.0,
    ),
    m(
        "wood_walnut",
        "Walnut",
        W,
        Rgba(98, 66, 44, 255),
        0.0,
        0.55,
        0.0,
        1.0,
    ),
    m(
        "paint_white",
        "Painted White",
        P,
        Rgba(240, 240, 236, 255),
        0.0,
        0.3,
        0.0,
        1.0,
    ),
];

pub fn by_id(id: &str) -> Option<&'static LibraryMaterial> {
    LIBRARY.iter().find(|x| x.id == id)
}

impl LibraryMaterial {
    pub fn material(&self) -> Material {
        Material::new(
            self.colour,
            self.metallic,
            self.roughness,
            self.transmission,
            self.ior,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_has_unique_ids_and_every_value_in_range() {
        assert!(LIBRARY.len() >= 20, "{}", LIBRARY.len());
        for (i, a) in LIBRARY.iter().enumerate() {
            assert!(
                a.id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}",
                a.id
            );
            for b in LIBRARY.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
            }
            assert_eq!(a.material(), a.material());
            let x = a.material();
            assert!((0.0..=1.0).contains(&x.metallic));
            assert!((MIN_ROUGHNESS..=1.0).contains(&x.roughness));
            assert!((MIN_IOR..=MAX_IOR).contains(&x.ior));
            assert!((0.05..=1.0).contains(&a.opacity));
        }
        for c in Category::ALL {
            assert!(LIBRARY.iter().any(|x| x.category == c), "{c:?} is empty");
        }
        // The format-21 preset ids are all still there.
        for id in [
            "steel",
            "aluminium",
            "brass",
            "copper",
            "paint_red",
            "paint_blue",
            "paint_yellow",
            "paint_black",
            "glass",
            "rubber",
        ] {
            assert!(by_id(id).is_some(), "{id}");
        }
    }

    #[test]
    fn hostile_numbers_are_repaired_and_the_ior_gives_f0() {
        let x = Material::new(Rgba(255, 255, 255, 255), f32::NAN, -1.0, 7.0, f32::INFINITY);
        assert_eq!(x.metallic, 0.0);
        assert_eq!(x.roughness, MIN_ROUGHNESS);
        assert_eq!(x.transmission, 1.0);
        assert_eq!(x.ior, DEFAULT_IOR);
        // n = 1.5: ((0.5) / (2.5))^2 = 0.04.
        assert!((x.dielectric_f0() - 0.04).abs() < 1e-6);
        assert!(x.is_glass());
    }
}

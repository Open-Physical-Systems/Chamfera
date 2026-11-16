//! Physical materials: what the bodies of a feature are made of, for their mass (format
//! 43). Stored by feature id like appearances; a body without one has no density and
//! mass properties take the density they are given.
//!
//! Densities are in g/cm³. The library's values are rounded handbook figures for common
//! engineering materials (room temperature), good for estimates, not for certification.

use crate::DocError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most physical materials one design may hold.
pub const MAX_MATERIALS: usize = 10_000;
/// Range of a density, g/cm³ (aerogel to osmium, with room).
pub const MIN_DENSITY: f64 = 1e-4;
pub const MAX_DENSITY: f64 = 30.0;

/// What the bodies of one feature are made of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicalMaterial {
    /// The library material it came from (`steel`), or a user's name.
    pub name: String,
    /// Mass per volume in g/cm³.
    pub density: f64,
}

impl PhysicalMaterial {
    /// Density as grams per mm³ (1 cm³ = 1000 mm³), which a volume in mm³ multiplies.
    pub fn grams_per_mm3(&self) -> f64 {
        self.density / 1000.0
    }
}

/// The library: (key, what the user reads, density in g/cm³).
pub const LIBRARY: &[(&str, &str, f64)] = &[
    ("steel", "Steel", 7.85),
    ("stainless_steel", "Stainless steel", 8.0),
    ("cast_iron", "Cast iron", 7.2),
    ("aluminium", "Aluminium", 2.70),
    ("brass", "Brass", 8.5),
    ("copper", "Copper", 8.96),
    ("titanium", "Titanium", 4.43),
    ("abs", "ABS plastic", 1.04),
    ("pla", "PLA plastic", 1.24),
    ("nylon", "Nylon", 1.14),
    ("acrylic", "Acrylic", 1.18),
    ("polycarbonate", "Polycarbonate", 1.20),
    ("rubber", "Rubber", 1.10),
    ("glass", "Glass", 2.50),
    ("wood_oak", "Oak", 0.75),
    ("water", "Water", 1.0),
];

/// The library material with this key.
pub fn library(key: &str) -> Option<PhysicalMaterial> {
    LIBRARY
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(k, _, d)| PhysicalMaterial {
            name: (*k).to_owned(),
            density: *d,
        })
}

fn bad(reason: String) -> DocError {
    DocError::BadFeature { reason }
}

/// Checks one material's name and density.
pub fn check_one(m: &PhysicalMaterial) -> Result<(), DocError> {
    if m.name.trim().is_empty() || m.name.chars().count() > 64 {
        return Err(bad("a material name must be 1 to 64 characters".into()));
    }
    if !(m.density.is_finite() && (MIN_DENSITY..=MAX_DENSITY).contains(&m.density)) {
        return Err(bad(format!(
            "a density must be between {MIN_DENSITY} and {MAX_DENSITY} g/cm³"
        )));
    }
    Ok(())
}

/// Checks the physical materials of a design read from a file.
pub(crate) fn check(map: &BTreeMap<u32, PhysicalMaterial>, last_id: u32) -> Result<(), DocError> {
    if map.len() > MAX_MATERIALS {
        return Err(bad("too many physical materials".into()));
    }
    for (id, m) in map {
        if *id == 0 || *id > last_id {
            return Err(bad(format!("physical material for unknown feature {id}")));
        }
        check_one(m)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn materials_survive_a_round_trip_and_bad_ones_are_refused() {
        let mut d = Document {
            last_feature_id: 3,
            ..Document::default()
        };
        let steel = library("steel").unwrap();
        // 7.85 g/cm³ = 0.00785 g/mm³.
        assert!((steel.grams_per_mm3() - 0.00785).abs() < 1e-15);
        d.materials.insert(2, steel);
        let back = Document::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(back.materials, d.materials);
        for (id, density, name) in [
            (9, 1.0, "x"),
            (2, 0.0, "x"),
            (2, 100.0, "x"),
            (2, f64::NAN, "x"),
            (2, 1.0, ""),
        ] {
            let mut b = d.clone();
            b.materials.insert(
                id,
                PhysicalMaterial {
                    name: name.into(),
                    density,
                },
            );
            assert!(
                Document::from_json(&b.to_json().unwrap_or_default()).is_err(),
                "{id} {density} {name:?}"
            );
        }
        assert!(library("unobtainium").is_none());
    }
}

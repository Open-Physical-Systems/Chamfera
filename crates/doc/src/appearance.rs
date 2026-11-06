//! Body appearances: the colour and opacity a user gave the bodies of a feature.
//!
//! A colour here is the user's data (what the part is made of), not a theme colour, so it
//! is stored as plain sRGB bytes. Without an appearance a body takes the theme's colour.

use crate::DocError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Lowest opacity a body may have (fainter bodies could not be found again).
pub const MIN_OPACITY: f64 = 0.05;
/// Most appearances one design may hold.
pub const MAX_APPEARANCES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    /// sRGB red, green, blue; `None` keeps the theme's body colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<[u8; 3]>,
    /// `MIN_OPACITY..=1`; 1 is opaque.
    #[serde(default = "one")]
    pub opacity: f64,
    /// The named appearance it came from (`glass`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

fn one() -> f64 {
    1.0
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            colour: None,
            opacity: 1.0,
            preset: None,
        }
    }
}

impl Appearance {
    /// Whether it changes nothing (then it need not be stored).
    pub fn is_default(&self) -> bool {
        self.colour.is_none() && self.opacity >= 1.0 && self.preset.is_none()
    }
}

fn bad(reason: String) -> DocError {
    DocError::BadFeature { reason }
}

/// Checks the appearances of a design read from a file.
pub(crate) fn check(map: &BTreeMap<u32, Appearance>, last_id: u32) -> Result<(), DocError> {
    if map.len() > MAX_APPEARANCES {
        return Err(bad("too many appearances".into()));
    }
    for (id, a) in map {
        if *id == 0 || *id > last_id {
            return Err(bad(format!("appearance for unknown feature {id}")));
        }
        if !(a.opacity.is_finite() && (MIN_OPACITY..=1.0).contains(&a.opacity)) {
            return Err(bad(format!(
                "appearance of feature {id}: opacity must be {MIN_OPACITY} to 1"
            )));
        }
        if a.preset.as_ref().is_some_and(|p| {
            p.is_empty() || p.len() > 40 || !p.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        }) {
            return Err(bad(format!("appearance of feature {id}: bad preset name")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn appearances_survive_a_round_trip_and_bad_ones_are_refused() {
        let mut d = Document {
            last_feature_id: 3,
            ..Document::default()
        };
        d.appearances.insert(
            2,
            super::Appearance {
                colour: Some([170, 210, 230]),
                opacity: 0.3,
                preset: Some("glass".into()),
            },
        );
        let back = Document::from_json(&d.to_json().unwrap()).unwrap();
        assert_eq!(back.appearances, d.appearances);
        for (id, opacity, preset) in [
            (9, 1.0, None),
            (2, 0.0, None),
            (2, 1.5, None),
            (2, 1.0, Some("Glass!")),
        ] {
            let mut bad = d.clone();
            bad.appearances.insert(
                id,
                super::Appearance {
                    colour: None,
                    opacity,
                    preset: preset.map(str::to_owned),
                },
            );
            assert!(
                Document::from_json(&bad.to_json().unwrap()).is_err(),
                "{id} {opacity}"
            );
        }
        // NaN is not JSON: serde writes null, which is refused as a number.
        let text = d.to_json().unwrap().replace("0.3", "null");
        assert!(Document::from_json(&text).is_err());
    }
}

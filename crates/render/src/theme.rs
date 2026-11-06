//! Theme tokens: every colour the viewport and the UI use. Nothing else picks colours.

use serde::{Deserialize, Serialize};

/// An sRGB colour with alpha, 0-255 per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    /// The colour in linear light (0..1), as the GPU blends it.
    pub fn linear(self) -> [f32; 4] {
        let f = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [f(self.0), f(self.1), f(self.2), f32::from(self.3) / 255.0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub mode: ThemeMode,
    // viewport
    pub background_top: Rgba,
    pub background_bottom: Rgba,
    pub grid_minor: Rgba,
    pub grid_major: Rgba,
    pub axis_x: Rgba,
    pub axis_y: Rgba,
    pub axis_z: Rgba,
    pub body: Rgba,
    pub body_selected: Rgba,
    pub edge: Rgba,
    // sketches
    /// Sketch geometry that can still move (under-constrained).
    pub sketch: Rgba,
    /// Sketch geometry fixed by its constraints.
    pub sketch_constrained: Rgba,
    pub sketch_construction: Rgba,
    /// Geometry being drawn, before it is created.
    pub sketch_preview: Rgba,
    /// What the pointer is over, or what is picked.
    pub highlight: Rgba,
    // interface
    pub panel: Rgba,
    pub panel_alt: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub error: Rgba,
}

impl Theme {
    pub fn light() -> Theme {
        Theme {
            mode: ThemeMode::Light,
            background_top: Rgba(232, 236, 242, 255),
            background_bottom: Rgba(250, 251, 253, 255),
            grid_minor: Rgba(120, 130, 145, 40),
            grid_major: Rgba(90, 100, 118, 90),
            axis_x: Rgba(214, 64, 64, 255),
            axis_y: Rgba(52, 160, 82, 255),
            axis_z: Rgba(56, 108, 214, 255),
            body: Rgba(176, 184, 196, 255),
            body_selected: Rgba(64, 140, 230, 255),
            edge: Rgba(40, 46, 56, 255),
            sketch: Rgba(30, 100, 220, 255),
            sketch_constrained: Rgba(20, 24, 30, 255),
            sketch_construction: Rgba(214, 120, 30, 255),
            sketch_preview: Rgba(30, 100, 220, 140),
            highlight: Rgba(240, 160, 20, 255),
            panel: Rgba(246, 247, 249, 255),
            panel_alt: Rgba(234, 237, 241, 255),
            text: Rgba(28, 32, 38, 255),
            text_muted: Rgba(98, 106, 118, 255),
            accent: Rgba(30, 112, 214, 255),
            error: Rgba(196, 44, 44, 255),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            mode: ThemeMode::Dark,
            background_top: Rgba(30, 33, 38, 255),
            background_bottom: Rgba(48, 52, 59, 255),
            grid_minor: Rgba(160, 168, 180, 28),
            grid_major: Rgba(170, 178, 190, 70),
            axis_x: Rgba(230, 86, 86, 255),
            axis_y: Rgba(86, 190, 110, 255),
            axis_z: Rgba(90, 140, 240, 255),
            body: Rgba(150, 158, 170, 255),
            body_selected: Rgba(84, 160, 255, 255),
            edge: Rgba(18, 20, 24, 255),
            sketch: Rgba(96, 170, 255, 255),
            sketch_constrained: Rgba(236, 240, 246, 255),
            sketch_construction: Rgba(240, 150, 60, 255),
            sketch_preview: Rgba(96, 170, 255, 140),
            highlight: Rgba(255, 190, 60, 255),
            panel: Rgba(36, 39, 44, 255),
            panel_alt: Rgba(44, 48, 54, 255),
            text: Rgba(226, 230, 236, 255),
            text_muted: Rgba(150, 158, 170, 255),
            accent: Rgba(84, 160, 255, 255),
            error: Rgba(240, 100, 100, 255),
        }
    }

    pub fn for_mode(mode: ThemeMode) -> Theme {
        match mode {
            ThemeMode::Light => Theme::light(),
            ThemeMode::Dark => Theme::dark(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_to_linear_matches_the_standard_curve() {
        assert_eq!(Rgba(0, 0, 0, 255).linear(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(Rgba(255, 255, 255, 0).linear(), [1.0, 1.0, 1.0, 0.0]);
        // sRGB 0.5 (128) is about 0.2158 linear.
        assert!((Rgba(128, 0, 0, 0).linear()[0] - 0.2158).abs() < 1e-3);
    }

    #[test]
    fn text_stands_out_from_panels_in_both_themes() {
        // WCAG relative luminance contrast of at least 4.5:1.
        let lum = |c: Rgba| {
            let l = c.linear();
            0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
        };
        for t in [Theme::light(), Theme::dark()] {
            for bg in [t.panel, t.panel_alt] {
                let (a, b) = (lum(t.text), lum(bg));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 4.5, "{:?}: {ratio}", t.mode);
            }
        }
    }
}

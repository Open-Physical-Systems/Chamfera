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
    /// A surface body (an open shell), drawn from both sides: the body colour with a
    /// light teal tint, so sheets read apart from solids.
    pub surface_body: Rgba,
    pub body_selected: Rgba,
    pub edge: Rgba,
    /// The shape a tool would make (the extrude preview), drawn translucent over the
    /// model: its alpha is how much of the model shows through.
    pub preview_body: Rgba,
    /// Edges behind faces in the "hidden edges" visual style (drawn dashed and faint).
    pub edge_hidden: Rgba,
    /// The cut faces of a section view.
    pub section_cap: Rgba,
    /// The hatch stripes over `section_cap`.
    pub section_hatch: Rgba,
    // sketches
    /// Sketch geometry that can still move (under-constrained).
    pub sketch: Rgba,
    /// Sketch geometry fixed by its constraints.
    pub sketch_constrained: Rgba,
    pub sketch_construction: Rgba,
    /// Model edges projected into a sketch (fixed, linked to their source).
    pub sketch_projected: Rgba,
    /// Geometry being drawn, before it is created.
    pub sketch_preview: Rgba,
    /// What the pointer is over, or what is picked.
    pub highlight: Rgba,
    /// Measure results in the viewport: the dimension line between closest points.
    pub measure: Rgba,
    // construction geometry
    /// Construction plane outlines, axes and points.
    pub construct: Rgba,
    /// The see-through fill of construction planes.
    pub construct_fill: Rgba,
    /// The face or edge under the pointer in the viewport, before a click picks it.
    pub hover_highlight: Rgba,
    /// Window box selection (dragged left to right: what is fully inside): solid outline.
    pub select_window: Rgba,
    pub select_window_fill: Rgba,
    /// Crossing box selection (dragged right to left: what it touches): dashed outline.
    pub select_crossing: Rgba,
    pub select_crossing_fill: Rgba,
    // toolpaths (Manufacture)
    /// Cutting moves (feeds), drawn solid.
    pub toolpath_feed: Rgba,
    /// Rapid moves, drawn dashed.
    pub toolpath_rapid: Rgba,
    /// The simulated stock (`cam.simulate`).
    pub stock: Rgba,
    /// Simulated stock cut below the part: a gouge.
    pub gouge: Rgba,
    // assemblies
    /// Where bodies overlap (`inspect.interference`): the outline, drawn over everything.
    pub interference: Rgba,
    /// The see-through fill of an overlap.
    pub interference_fill: Rgba,
    /// Exploded view trails: lines from where a component sits to where it is shown.
    pub explode_trail: Rgba,
    // interface surfaces (see docs/design/ui.md)
    /// The app bar, ribbon, timeline and status bar.
    pub chrome: Rgba,
    /// The Browser and dialogs.
    pub panel: Rgba,
    /// Text fields, tiles and pressed buttons.
    pub panel_alt: Rgba,
    /// Hairlines between areas and around fields.
    pub border: Rgba,
    /// What the pointer is over: rows, buttons, tabs.
    pub hover: Rgba,
    /// The active or selected row, tool or tab.
    pub selected: Rgba,
    /// Drop shadow under popups and dialogs.
    pub shadow: Rgba,
    // interface content
    pub text: Rgba,
    pub text_muted: Rgba,
    /// Interactive accent: focus rings, the primary button, selection.
    pub accent: Rgba,
    /// Text and icons on an `accent` fill.
    pub on_accent: Rgba,
    /// The base strokes of icons.
    pub icon: Rgba,
    /// The accent part of icons (the brand's chamfer orange).
    pub icon_accent: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    // the view cube
    pub cube_face: Rgba,
    pub cube_edge: Rgba,
    // assemblies
    /// Component tints, mixed lightly into the body colour (`body_colour`).
    pub component_tints: [Rgba; 6],
    /// The drawing workspace: the area around the sheet, the paper, and its lines.
    pub sheet_backdrop: Rgba,
    pub sheet_paper: Rgba,
    pub sheet_line: Rgba,
    pub sheet_hidden: Rgba,
    pub sheet_centre: Rgba,
    pub sheet_dimension: Rgba,
    pub sheet_border: Rgba,
    pub sheet_pick: Rgba,
    /// Section hatching: narrow lines, quieter than outlines.
    pub sheet_hatch: Rgba,
    /// A dimension or annotation whose model reference no longer resolves (warning).
    pub sheet_dangling: Rgba,
}

impl Theme {
    /// The colour of a body: the plain body colour, or with a component tint mixed in
    /// (a quarter of the tint, so shading and edges still read as one material).
    pub fn body_colour(&self, tint: Option<u8>) -> Rgba {
        self.tinted(self.body, tint)
    }

    /// The colour of a surface body, with a component tint mixed in like [`Theme::body_colour`].
    pub fn surface_colour(&self, tint: Option<u8>) -> Rgba {
        self.tinted(self.surface_body, tint)
    }

    fn tinted(&self, base: Rgba, tint: Option<u8>) -> Rgba {
        let Some(i) = tint else {
            return base;
        };
        let t = self.component_tints[usize::from(i) % self.component_tints.len()];
        let mix = |a: u8, b: u8| ((u16::from(a) * 3 + u16::from(b)) / 4) as u8;
        Rgba(mix(base.0, t.0), mix(base.1, t.1), mix(base.2, t.2), base.3)
    }

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
            surface_body: Rgba(150, 196, 192, 255),
            body_selected: Rgba(64, 140, 230, 255),
            edge: Rgba(40, 46, 56, 255),
            preview_body: Rgba(30, 112, 214, 110),
            edge_hidden: Rgba(40, 46, 56, 110),
            section_cap: Rgba(214, 120, 64, 255),
            section_hatch: Rgba(120, 58, 24, 255),
            sketch: Rgba(30, 100, 220, 255),
            sketch_constrained: Rgba(20, 24, 30, 255),
            sketch_construction: Rgba(214, 120, 30, 255),
            sketch_projected: Rgba(150, 60, 200, 255),
            sketch_preview: Rgba(30, 100, 220, 140),
            highlight: Rgba(240, 160, 20, 255),
            measure: Rgba(0, 140, 112, 255),
            construct: Rgba(176, 112, 16, 255),
            construct_fill: Rgba(232, 168, 40, 46),
            hover_highlight: Rgba(0, 150, 180, 255),
            select_window: Rgba(30, 112, 214, 255),
            select_window_fill: Rgba(30, 112, 214, 36),
            select_crossing: Rgba(30, 135, 70, 255),
            select_crossing_fill: Rgba(30, 135, 70, 36),
            toolpath_feed: Rgba(96, 72, 204, 255),
            toolpath_rapid: Rgba(200, 50, 60, 220),
            stock: Rgba(196, 170, 120, 255),
            gouge: Rgba(222, 32, 48, 255),
            interference: Rgba(214, 30, 60, 255),
            interference_fill: Rgba(230, 40, 70, 120),
            explode_trail: Rgba(52, 58, 72, 255),
            chrome: Rgba(252, 252, 253, 255),
            panel: Rgba(246, 247, 249, 255),
            panel_alt: Rgba(236, 239, 243, 255),
            border: Rgba(219, 223, 229, 255),
            hover: Rgba(231, 235, 241, 255),
            selected: Rgba(219, 232, 250, 255),
            shadow: Rgba(16, 24, 40, 40),
            text: Rgba(28, 32, 38, 255),
            text_muted: Rgba(92, 100, 112, 255),
            accent: Rgba(30, 112, 214, 255),
            on_accent: Rgba(255, 255, 255, 255),
            icon: Rgba(52, 58, 68, 255),
            icon_accent: Rgba(196, 98, 8, 255),
            success: Rgba(30, 135, 70, 255),
            warning: Rgba(166, 102, 0, 255),
            error: Rgba(196, 44, 44, 255),
            cube_face: Rgba(236, 240, 245, 240),
            cube_edge: Rgba(128, 137, 150, 255),
            component_tints: [
                Rgba(70, 130, 220, 255),
                Rgba(226, 132, 40, 255),
                Rgba(60, 170, 110, 255),
                Rgba(196, 80, 160, 255),
                Rgba(200, 176, 50, 255),
                Rgba(60, 170, 190, 255),
            ],
            sheet_backdrop: Rgba(214, 219, 226, 255),
            sheet_paper: Rgba(255, 255, 255, 255),
            sheet_line: Rgba(20, 24, 30, 255),
            sheet_hidden: Rgba(70, 78, 92, 255),
            sheet_centre: Rgba(176, 64, 44, 255),
            sheet_dimension: Rgba(28, 92, 190, 255),
            sheet_border: Rgba(20, 24, 30, 255),
            sheet_pick: Rgba(240, 140, 20, 255),
            sheet_hatch: Rgba(90, 98, 112, 255),
            sheet_dangling: Rgba(196, 60, 120, 255),
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
            surface_body: Rgba(118, 168, 164, 255),
            body_selected: Rgba(84, 160, 255, 255),
            edge: Rgba(18, 20, 24, 255),
            preview_body: Rgba(84, 160, 255, 110),
            edge_hidden: Rgba(200, 206, 216, 90),
            section_cap: Rgba(200, 110, 60, 255),
            section_hatch: Rgba(110, 52, 22, 255),
            sketch: Rgba(96, 170, 255, 255),
            sketch_constrained: Rgba(236, 240, 246, 255),
            sketch_construction: Rgba(240, 150, 60, 255),
            sketch_projected: Rgba(200, 140, 255, 255),
            sketch_preview: Rgba(96, 170, 255, 140),
            highlight: Rgba(255, 190, 60, 255),
            measure: Rgba(70, 216, 178, 255),
            construct: Rgba(240, 180, 80, 255),
            construct_fill: Rgba(240, 180, 80, 38),
            hover_highlight: Rgba(90, 215, 235, 255),
            select_window: Rgba(84, 160, 255, 255),
            select_window_fill: Rgba(84, 160, 255, 40),
            select_crossing: Rgba(88, 196, 124, 255),
            select_crossing_fill: Rgba(88, 196, 124, 40),
            toolpath_feed: Rgba(170, 150, 255, 255),
            toolpath_rapid: Rgba(255, 110, 110, 220),
            stock: Rgba(178, 152, 104, 255),
            gouge: Rgba(255, 64, 80, 255),
            interference: Rgba(255, 80, 110, 255),
            interference_fill: Rgba(255, 70, 100, 120),
            explode_trail: Rgba(200, 206, 218, 255),
            chrome: Rgba(29, 31, 35, 255),
            panel: Rgba(35, 38, 43, 255),
            panel_alt: Rgba(45, 49, 56, 255),
            border: Rgba(55, 60, 68, 255),
            hover: Rgba(50, 55, 63, 255),
            selected: Rgba(31, 58, 97, 255),
            shadow: Rgba(0, 0, 0, 120),
            text: Rgba(226, 230, 236, 255),
            text_muted: Rgba(156, 164, 176, 255),
            accent: Rgba(84, 160, 255, 255),
            on_accent: Rgba(10, 18, 30, 255),
            icon: Rgba(206, 211, 219, 255),
            icon_accent: Rgba(255, 158, 62, 255),
            success: Rgba(88, 196, 124, 255),
            warning: Rgba(236, 178, 64, 255),
            error: Rgba(240, 100, 100, 255),
            cube_face: Rgba(54, 58, 66, 235),
            cube_edge: Rgba(112, 120, 132, 255),
            component_tints: [
                Rgba(90, 150, 240, 255),
                Rgba(240, 150, 60, 255),
                Rgba(80, 190, 130, 255),
                Rgba(210, 100, 180, 255),
                Rgba(220, 196, 70, 255),
                Rgba(80, 190, 210, 255),
            ],
            sheet_backdrop: Rgba(24, 26, 30, 255),
            sheet_paper: Rgba(38, 42, 48, 255),
            sheet_line: Rgba(232, 236, 242, 255),
            sheet_hidden: Rgba(160, 170, 184, 255),
            sheet_centre: Rgba(240, 130, 100, 255),
            sheet_dimension: Rgba(110, 172, 255, 255),
            sheet_border: Rgba(200, 206, 214, 255),
            sheet_pick: Rgba(255, 180, 60, 255),
            sheet_hatch: Rgba(150, 160, 176, 255),
            sheet_dangling: Rgba(255, 110, 180, 255),
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

    /// WCAG contrast ratio between two opaque colours.
    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let lum = |c: Rgba| {
            let l = c.linear();
            0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]
        };
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_stands_out_from_panels_in_both_themes() {
        // WCAG AA for text: at least 4.5:1 on every surface text is drawn on.
        for t in [Theme::light(), Theme::dark()] {
            let surfaces = [
                ("chrome", t.chrome),
                ("panel", t.panel),
                ("panel_alt", t.panel_alt),
                ("hover", t.hover),
                ("selected", t.selected),
            ];
            for (name, bg) in surfaces {
                for (what, fg) in [("text", t.text), ("text_muted", t.text_muted)] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 4.5, "{:?}: {what} on {name}: {ratio}", t.mode);
                }
            }
            for bg in [t.chrome, t.panel] {
                let ratio = contrast(t.error, bg);
                assert!(ratio >= 4.5, "{:?}: error text: {ratio}", t.mode);
            }
            let ratio = contrast(t.on_accent, t.accent);
            assert!(ratio >= 4.5, "{:?}: on_accent: {ratio}", t.mode);
        }
    }

    #[test]
    fn box_selection_outlines_stand_out_from_the_viewport_and_differ() {
        // Graphics contrast (WCAG 3:1) against both ends of the background gradient, and
        // window and crossing boxes in different colours, not only different dashes.
        for t in [Theme::light(), Theme::dark()] {
            for bg in [t.background_top, t.background_bottom] {
                for (what, fg) in [("window", t.select_window), ("crossing", t.select_crossing)] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 3.0, "{:?}: {what} box: {ratio}", t.mode);
                }
            }
            assert_ne!(t.select_window, t.select_crossing);
            assert!(t.select_window_fill.3 < 64 && t.select_crossing_fill.3 < 64);
        }
    }

    #[test]
    fn drawing_lines_stand_out_from_the_paper_in_both_themes() {
        for t in [Theme::light(), Theme::dark()] {
            for (what, fg) in [
                ("sheet_line", t.sheet_line),
                ("sheet_hidden", t.sheet_hidden),
                ("sheet_centre", t.sheet_centre),
                ("sheet_dimension", t.sheet_dimension),
                ("sheet_border", t.sheet_border),
                ("sheet_hatch", t.sheet_hatch),
                ("sheet_dangling", t.sheet_dangling),
            ] {
                let ratio = contrast(fg, t.sheet_paper);
                assert!(ratio >= 3.0, "{:?}: {what}: {ratio}", t.mode);
            }
        }
    }

    #[test]
    fn icons_and_status_marks_stand_out_in_both_themes() {
        // WCAG AA for graphics (icons, the sketch status dot, focus rings): at least 3:1.
        for t in [Theme::light(), Theme::dark()] {
            for bg in [t.chrome, t.panel, t.hover, t.selected] {
                for (what, fg) in [
                    ("icon", t.icon),
                    ("icon_accent", t.icon_accent),
                    ("accent", t.accent),
                    ("success", t.success),
                    ("warning", t.warning),
                    ("error", t.error),
                ] {
                    let ratio = contrast(fg, bg);
                    assert!(ratio >= 3.0, "{:?}: {what}: {ratio}", t.mode);
                }
            }
        }
    }
}

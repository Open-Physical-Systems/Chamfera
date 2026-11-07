//! Mouse navigation presets: which button and modifier orbit, pan or draw a selection
//! box, plus zoom direction, zoom to the cursor, the orbit centre and sensitivity.
//!
//! `navigate` (in `lib.rs`) asks [`drag_for`] what a drag does once, when it starts, and
//! keeps that for the whole drag. Everything still goes through `view.*` commands.

use crate::ChamferaApp;
use egui::{Modifiers, PointerButton, Rect, Response, Vec2};
use serde_json::json;

/// A mouse layout. The names describe a familiar style; the mappings are our own table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Right-drag orbits, middle-drag pans (Shift+middle orbits), left-drag draws a box.
    #[default]
    Chamfera,
    /// Shift+middle orbits, middle pans, left-drag draws a box.
    Fusion,
    /// Hold F4 and left-drag, or Shift+middle, to orbit; middle pans.
    Inventor,
    /// Middle-drag orbits, Ctrl+middle pans.
    SolidWorks,
    /// Left-drag orbits (for touchpads); Shift+left-drag draws a box, Ctrl+left-drag,
    /// right-drag or middle-drag pans.
    LeftOrbit,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Chamfera,
        Preset::Fusion,
        Preset::Inventor,
        Preset::SolidWorks,
        Preset::LeftOrbit,
    ];

    /// The key stored in the preferences file.
    pub fn key(self) -> &'static str {
        match self {
            Preset::Chamfera => "chamfera",
            Preset::Fusion => "fusion",
            Preset::Inventor => "inventor",
            Preset::SolidWorks => "solidworks",
            Preset::LeftOrbit => "left_orbit",
        }
    }

    pub fn parse(s: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.key() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Chamfera => "Chamfera default",
            Preset::Fusion => "Fusion-style",
            Preset::Inventor => "Inventor-style",
            Preset::SolidWorks => "SolidWorks-style",
            Preset::LeftOrbit => "Left-drag orbit (touchpad)",
        }
    }

    /// The status bar's hint while nothing else is going on.
    pub fn status_hint(self) -> String {
        let moves = self
            .summary()
            .iter()
            .filter(|(_, does)| *does != "Box select" && *does != "Zoom")
            .map(|(gesture, does)| format!("{gesture}: {}", does.to_lowercase()))
            .collect::<Vec<_>>()
            .join(" · ");
        let boxed = self
            .summary()
            .iter()
            .find(|(_, does)| *does == "Box select")
            .map_or("Left-drag", |(gesture, _)| gesture);
        format!(
            "Click: select · Shift/Ctrl+click: add or remove · {boxed}: box (right: \
             window, left: crossing) · {moves} · Wheel: zoom · F: fit"
        )
    }

    /// (gesture, what it does) rows for the Preferences window.
    pub fn summary(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Preset::Chamfera => &[
                ("Right-drag", "Orbit"),
                ("Shift + middle-drag", "Orbit"),
                ("Middle-drag", "Pan"),
                ("Left-drag", "Box select"),
                ("Wheel", "Zoom"),
            ],
            Preset::Fusion => &[
                ("Shift + middle-drag", "Orbit"),
                ("Middle-drag", "Pan"),
                ("Left-drag", "Box select"),
                ("Wheel", "Zoom"),
            ],
            Preset::Inventor => &[
                ("F4 + left-drag", "Orbit"),
                ("Shift + middle-drag", "Orbit"),
                ("Middle-drag", "Pan"),
                ("Left-drag", "Box select"),
                ("Wheel", "Zoom"),
            ],
            Preset::SolidWorks => &[
                ("Middle-drag", "Orbit"),
                ("Ctrl + middle-drag", "Pan"),
                ("Left-drag", "Box select"),
                ("Wheel", "Zoom"),
            ],
            Preset::LeftOrbit => &[
                ("Left-drag", "Orbit"),
                ("Shift + left-drag", "Box select"),
                ("Ctrl + left-drag", "Pan"),
                ("Right- or middle-drag", "Pan"),
                ("Wheel or pinch", "Zoom"),
            ],
        }
    }
}

/// What a drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Orbit,
    Pan,
    /// A selection box (`sketch_select`).
    Select,
    /// Nothing here (a right drag in some presets, or a drag a tool uses).
    Nothing,
}

/// The preset's action for a drag with `button` and `mods` held; `orbit_key` is true
/// while F4 is held (the Inventor-style orbit key).
pub fn drag_for(preset: Preset, button: PointerButton, mods: Modifiers, orbit_key: bool) -> Drag {
    use PointerButton::{Middle, Primary, Secondary};
    let ctrl = mods.ctrl || mods.command;
    let shift = mods.shift;
    match (preset, button) {
        (Preset::Chamfera, Primary) => Drag::Select,
        (Preset::Chamfera, Secondary) => Drag::Orbit,
        (Preset::Chamfera | Preset::Fusion | Preset::Inventor, Middle) if shift => Drag::Orbit,
        (Preset::Chamfera | Preset::Fusion | Preset::Inventor, Middle) => Drag::Pan,
        (Preset::Fusion, Primary) => Drag::Select,
        (Preset::Inventor, Primary) if orbit_key => Drag::Orbit,
        (Preset::Inventor, Primary) => Drag::Select,
        (Preset::SolidWorks, Primary) => Drag::Select,
        (Preset::SolidWorks, Middle) if ctrl => Drag::Pan,
        (Preset::SolidWorks, Middle) => Drag::Orbit,
        (Preset::LeftOrbit, Primary) if shift => Drag::Select,
        (Preset::LeftOrbit, Primary) if ctrl => Drag::Pan,
        (Preset::LeftOrbit, Primary) => Drag::Orbit,
        (Preset::LeftOrbit, Middle | Secondary) => Drag::Pan,
        (Preset::Fusion | Preset::Inventor | Preset::SolidWorks, Secondary) => Drag::Nothing,
        (_, PointerButton::Extra1 | PointerButton::Extra2) => Drag::Nothing,
    }
}

/// Navigation preferences.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Navigation {
    pub preset: Preset,
    /// Wheel up zooms out instead of in.
    pub invert_zoom: bool,
    /// Zoom toward the point under the cursor (else toward the view's centre).
    pub zoom_to_cursor: bool,
    /// Orbit about the model point under the cursor where the drag starts (else about the
    /// view's target, which Home and Fit put at the model's centre).
    pub orbit_about_cursor: bool,
    /// Multiplies orbit, pan and wheel speed (0.25 to 4).
    pub sensitivity: f64,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            preset: Preset::Chamfera,
            invert_zoom: false,
            zoom_to_cursor: true,
            orbit_about_cursor: false,
            sensitivity: 1.0,
        }
    }
}

pub const MIN_SENSITIVITY: f64 = 0.25;
pub const MAX_SENSITIVITY: f64 = 4.0;

/// Radians of orbit per pixel of drag at sensitivity 1.
const ORBIT_PER_PX: f64 = 0.008;
/// Wheel zoom: the exponent per point of scroll at sensitivity 1.
const ZOOM_PER_SCROLL: f64 = 0.0025;

/// The button a drag is held with, if any.
fn drag_button(response: &Response) -> Option<PointerButton> {
    [
        PointerButton::Primary,
        PointerButton::Secondary,
        PointerButton::Middle,
    ]
    .into_iter()
    .find(|b| response.dragged_by(*b))
}

/// A drag in progress: what it does, and the orbit centre if it orbits about a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavDrag {
    pub drag: Drag,
    pub button: PointerButton,
    pivot: Option<[f64; 3]>,
}

impl ChamferaApp {
    /// Orbit and pan from a drag, by the active preset. Returns what the drag does, so the
    /// caller starts a selection box only for `Drag::Select`.
    pub(crate) fn navigate_drag(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        rect: Rect,
    ) -> Option<Drag> {
        let h = f64::from(rect.height());
        let nav = self.prefs.navigation;
        if response.drag_started()
            && let Some(button) = drag_button(response)
        {
            let (mods, f4) = ui.input(|i| (i.modifiers, i.key_down(egui::Key::F4)));
            let mut drag = drag_for(nav.preset, button, mods, f4);
            // In a sketch the left button belongs to the sketch tools.
            if self.session.active_sketch.is_some() && button == PointerButton::Primary {
                drag = Drag::Nothing;
            }
            let pivot = if drag == Drag::Orbit && nav.orbit_about_cursor {
                self.pivot_at(ui, rect)
            } else {
                None
            };
            self.nav_drag = Some(NavDrag {
                drag,
                button,
                pivot,
            });
        }
        let current = self.nav_drag?;
        if !response.dragged_by(current.button) {
            self.nav_drag = None;
            return Some(current.drag);
        }
        let delta = response.drag_delta();
        if delta != Vec2::ZERO {
            let k = nav.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
            let (dx, dy) = (f64::from(delta.x) * k, f64::from(delta.y) * k);
            match current.drag {
                Drag::Orbit => {
                    let mut p = json!({ "yaw": (-dx * ORBIT_PER_PX).clamp(-6.0, 6.0),
                                        "pitch": (dy * ORBIT_PER_PX).clamp(-6.0, 6.0) });
                    if let Some([x, y, z]) = current.pivot {
                        p["pivot"] = json!({ "x": x, "y": y, "z": z });
                    }
                    self.run("view.orbit", p);
                }
                Drag::Pan => {
                    self.run("view.pan", json!({ "dx": dx, "dy": dy, "height": h }));
                }
                Drag::Select | Drag::Nothing => {}
            }
        }
        if response.drag_stopped() {
            self.nav_drag = None;
        }
        Some(current.drag)
    }

    /// The model point under where the drag was pressed.
    fn pivot_at(&mut self, ui: &egui::Ui, rect: Rect) -> Option<[f64; 3]> {
        let start = ui.input(|i| i.pointer.press_origin())?;
        let r = self.session.execute(
            "view.point_at",
            &json!({ "x": f64::from(start.x - rect.min.x), "y": f64::from(start.y - rect.min.y),
                     "width": f64::from(rect.width()).max(1.0),
                     "height": f64::from(rect.height()).max(1.0) }),
        );
        let p = r.ok()?;
        let p = &p["point"];
        Some([p["x"].as_f64()?, p["y"].as_f64()?, p["z"].as_f64()?])
    }

    /// Wheel and pinch zoom, by the zoom preferences.
    pub(crate) fn navigate_zoom(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        if !response.hovered() {
            return;
        }
        let nav = self.prefs.navigation;
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let k = nav.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
        let mut factor = if (pinch - 1.0).abs() > f32::EPSILON {
            1.0 / f64::from(pinch)
        } else if scroll.abs() > 0.0 {
            (-f64::from(scroll) * ZOOM_PER_SCROLL * k).exp()
        } else {
            return;
        };
        if nav.invert_zoom && scroll.abs() > 0.0 && (pinch - 1.0).abs() <= f32::EPSILON {
            factor = 1.0 / factor;
        }
        if (factor - 1.0).abs() <= 1e-9 || !factor.is_finite() {
            return;
        }
        let factor = factor.clamp(0.01, 100.0);
        let mut p = json!({ "factor": factor });
        if nav.zoom_to_cursor
            && let Some(pos) = response.hover_pos()
        {
            p = json!({ "factor": factor,
                        "x": f64::from(pos.x - rect.min.x), "y": f64::from(pos.y - rect.min.y),
                        "width": f64::from(rect.width()), "height": f64::from(rect.height()) });
        }
        self.run("view.zoom", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PointerButton::{Middle, Primary, Secondary};

    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CTRL: Modifiers = Modifiers::CTRL;

    #[test]
    fn each_preset_maps_buttons_and_modifiers_to_its_actions() {
        use Drag::{Nothing, Orbit, Pan, Select};
        // (preset, button, modifiers, F4 held, expected): the table in each preset's doc.
        let cases = [
            (Preset::Chamfera, Secondary, NONE, false, Orbit),
            (Preset::Chamfera, Middle, NONE, false, Pan),
            (Preset::Chamfera, Middle, SHIFT, false, Orbit),
            (Preset::Chamfera, Primary, NONE, false, Select),
            (Preset::Fusion, Middle, SHIFT, false, Orbit),
            (Preset::Fusion, Middle, NONE, false, Pan),
            (Preset::Fusion, Primary, NONE, false, Select),
            (Preset::Fusion, Secondary, NONE, false, Nothing),
            (Preset::Inventor, Primary, NONE, true, Orbit),
            (Preset::Inventor, Primary, NONE, false, Select),
            (Preset::Inventor, Middle, SHIFT, false, Orbit),
            (Preset::Inventor, Middle, NONE, false, Pan),
            (Preset::SolidWorks, Middle, NONE, false, Orbit),
            (Preset::SolidWorks, Middle, CTRL, false, Pan),
            (Preset::SolidWorks, Primary, NONE, false, Select),
            (Preset::LeftOrbit, Primary, NONE, false, Orbit),
            (Preset::LeftOrbit, Primary, SHIFT, false, Select),
            (Preset::LeftOrbit, Primary, CTRL, false, Pan),
            (Preset::LeftOrbit, Middle, NONE, false, Pan),
            (Preset::LeftOrbit, Secondary, NONE, false, Pan),
        ];
        for (preset, button, mods, f4, expected) in cases {
            assert_eq!(
                drag_for(preset, button, mods, f4),
                expected,
                "{preset:?} {button:?} {mods:?} f4={f4}"
            );
        }
    }

    #[test]
    fn every_preset_has_a_way_to_orbit_pan_and_box_select_and_round_trips_its_key() {
        let buttons = [Primary, Secondary, Middle];
        let mods = [NONE, SHIFT, CTRL];
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.key()), Some(preset));
            let mut found = Vec::new();
            for b in buttons {
                for m in mods {
                    for f4 in [false, true] {
                        found.push(drag_for(preset, b, m, f4));
                    }
                }
            }
            for want in [Drag::Orbit, Drag::Pan, Drag::Select] {
                assert!(found.contains(&want), "{preset:?} has no {want:?}");
            }
        }
        assert_eq!(Preset::parse("Fusion"), None);
    }
}

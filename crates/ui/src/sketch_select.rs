//! Box selection by left-dragging in the viewport, for bodies and for the active sketch's
//! points and curves (Select tool, drag started on empty space).
//!
//! Dragged left to right the box is a window (solid outline: what is fully inside);
//! right to left it is a crossing (dashed outline: what it touches). Shift or Ctrl adds,
//! Ctrl+Shift removes. The release runs `select.box`; the same modifiers on a click make
//! `select.pick` toggle or remove (`pick_mode`).

use crate::{ChamferaApp, color};
use egui::{Modifiers, Pos2, Rect, Response, Stroke};
use serde_json::json;

/// A box being dragged: where the press was and where the pointer is, in screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxDrag {
    pub start: Pos2,
    pub now: Pos2,
}

impl BoxDrag {
    /// Dragged to the right (or straight up or down): a window.
    pub fn is_window(&self) -> bool {
        self.now.x >= self.start.x
    }

    pub fn mode(&self) -> &'static str {
        if self.is_window() {
            "window"
        } else {
            "crossing"
        }
    }
}

/// What a click with these modifiers does to the selection (`select.pick`'s `mode`).
pub(crate) fn pick_mode(m: Modifiers) -> &'static str {
    let ctrl = m.ctrl || m.command;
    match (ctrl, m.shift) {
        (true, true) => "remove",
        (true, false) | (false, true) => "toggle",
        (false, false) => "replace",
    }
}

impl ChamferaApp {
    /// Starts a box at the press position once a left drag has begun. In a sketch the drag
    /// must start on empty space (a drag on a point moves it instead).
    pub(crate) fn box_begin(&mut self, ui: &egui::Ui, rect: Rect, in_sketch: bool) {
        let Some(start) = ui.input(|i| i.pointer.press_origin()) else {
            return;
        };
        if !rect.contains(start) {
            return;
        }
        if in_sketch {
            let (x, y) = (
                f64::from(start.x - rect.min.x),
                f64::from(start.y - rect.min.y),
            );
            let item = self.session.execute(
                "sketch.pick",
                &json!({ "x": x, "y": y, "width": f64::from(rect.width()), "height": f64::from(rect.height()) }),
            );
            if item.is_ok_and(|r| !r["item"].is_null()) {
                return;
            }
        }
        let now = ui.input(|i| i.pointer.latest_pos()).unwrap_or(start);
        self.select_box = Some(BoxDrag { start, now });
        self.box_preview = None;
    }

    /// Follows a box drag and runs `select.box` when it ends; Esc drops it. True while a
    /// box is being dragged, so the drag does nothing else.
    pub(crate) fn box_input(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) -> bool {
        let Some(mut drag) = self.select_box else {
            return false;
        };
        let (esc, modifiers, pos) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.modifiers,
                i.pointer.latest_pos(),
            )
        });
        if esc {
            self.select_box = None;
            return true;
        }
        if let Some(p) = response.interact_pointer_pos().or(pos) {
            drag.now = p;
        }
        self.select_box = Some(drag);
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            self.select_box = None;
            self.run_box(drag, modifiers, rect);
        }
        true
    }

    /// Runs `select.box` for a finished drag, its corners clamped to the viewport.
    fn run_box(&mut self, drag: BoxDrag, m: Modifiers, rect: Rect) {
        let local = |p: Pos2| {
            let p = rect.clamp(p);
            (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y))
        };
        let (a, b) = (local(drag.start), local(drag.now));
        let mut p = json!({
            "x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1,
            "width": f64::from(rect.width()).max(1.0),
            "height": f64::from(rect.height()).max(1.0),
            "mode": drag.mode(),
        });
        match pick_mode(m) {
            "remove" => p["remove"] = json!(true),
            "toggle" => p["add"] = json!(true),
            _ => {}
        }
        if self.session.active_sketch.is_some() {
            self.run_sketch("select.box", p);
        } else {
            self.run("select.box", p);
        }
    }

    /// The box being dragged, or the `--box` one shown for a screenshot.
    fn shown_box(&self) -> Option<BoxDrag> {
        self.select_box.or_else(|| {
            let [x1, y1, x2, y2] = self.box_preview?;
            let min = self.viewport_rect.min;
            Some(BoxDrag {
                start: min + egui::vec2(x1, y1),
                now: min + egui::vec2(x2, y2),
            })
        })
    }

    /// Draws the box being dragged: a window with a solid outline, a crossing dashed.
    pub(crate) fn paint_box(&self, ui: &egui::Ui, rect: Rect) {
        let Some(drag) = self.shown_box() else {
            return;
        };
        let r = Rect::from_two_pos(rect.clamp(drag.start), rect.clamp(drag.now));
        let t = &self.theme;
        let painter = ui.painter().with_clip_rect(rect);
        if drag.is_window() {
            painter.rect(
                r,
                0.0,
                color(t.select_window_fill),
                Stroke::new(1.5, color(t.select_window)),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect_filled(r, 0.0, color(t.select_crossing_fill));
            let ring = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(
                &ring,
                Stroke::new(1.5, color(t.select_crossing)),
                6.0,
                4.0,
            ));
        }
    }

    /// The status bar hint while a box is dragged.
    pub(crate) fn box_hint(&self) -> Option<&'static str> {
        let drag = self.shown_box()?;
        Some(
            match (drag.is_window(), self.session.active_sketch.is_some()) {
                (true, false) => {
                    "Window: selects bodies fully inside · drag left for a crossing · Shift or Ctrl adds · Ctrl+Shift removes"
                }
                (false, false) => {
                    "Crossing: selects bodies the box touches · drag right for a window · Shift or Ctrl adds · Ctrl+Shift removes"
                }
                (true, true) => {
                    "Window: selects points and curves fully inside · drag left for a crossing · Shift or Ctrl adds"
                }
                (false, true) => {
                    "Crossing: selects points and curves the box touches · drag right for a window · Shift or Ctrl adds"
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_choose_how_a_click_changes_the_selection() {
        assert_eq!(pick_mode(Modifiers::NONE), "replace");
        assert_eq!(pick_mode(Modifiers::SHIFT), "toggle");
        assert_eq!(pick_mode(Modifiers::CTRL), "toggle");
        assert_eq!(pick_mode(Modifiers::CTRL | Modifiers::SHIFT), "remove");
    }

    #[test]
    fn the_drag_direction_picks_window_or_crossing() {
        let d = |x0: f32, x1: f32| BoxDrag {
            start: Pos2::new(x0, 10.0),
            now: Pos2::new(x1, 80.0),
        };
        assert_eq!(d(10.0, 200.0).mode(), "window");
        assert_eq!(d(200.0, 10.0).mode(), "crossing");
    }
}

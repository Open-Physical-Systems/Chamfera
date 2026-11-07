//! Typed points and Ortho mode while drawing.
//!
//! **Typed points.** With a drawing tool waiting for a point, typing a point and pressing
//! Enter places it, as a click there would (with no snap):
//! - `x,y`: the point at those sketch coordinates (`#x,y` says the same, when the
//!   heads-up length and angle fields would otherwise take the digits);
//! - `@dx,dy`: that far from the last point placed;
//! - `@length<angle`: that far from the last point, at that angle from +X
//!   (counter-clockwise, in degrees unless the design's angle unit says otherwise).
//!
//! Lengths are in the design's unit. The comma separates the two values, so decimals use
//! a point. A typed point starts with `@` or `#` at any time, or with a digit, `-` or `.`
//! when no heads-up field is waiting; Backspace deletes, Esc drops it.
//!
//! **Ortho mode** (F8, or the Ortho button): lines and arcs go horizontal or vertical
//! from their last point, as if Shift were held; holding Shift frees them. Point snaps
//! (ends, midpoints, centres, crossings) still win over Ortho.

use crate::sketch_ui::{Click, Tool, parse_value};
use crate::{ChamferaApp, color};
use chamfera_geom::{MAX_COORDINATE, Point2, Vec2};
use chamfera_sketch::exact_direction;
use egui::Rect;

/// Most characters in a typed point.
const MAX_COORD_CHARS: usize = 48;

/// Characters a typed point may hold.
fn coord_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '.' | ',' | '-' | '+' | '<' | '@' | '#' | ' ')
}

/// True for the tools that place points (and so take a typed one).
pub(crate) fn takes_coordinates(tool: Tool) -> bool {
    !matches!(
        tool,
        Tool::Select
            | Tool::TangentArc
            | Tool::Transform
            | Tool::Project
            | Tool::TangentCircle
            | Tool::Offset
            | Tool::Mirror
            | Tool::RectangularPattern
            | Tool::CircularPattern
            | Tool::Fillet
            | Tool::Chamfer
            | Tool::Trim
            | Tool::Extend
            | Tool::Split
    )
}

/// The point a typed entry names. `last` is the last point placed (the origin when there
/// is none). Errors say what was wrong and what is allowed.
pub(crate) fn parse_coordinate(
    units: chamfera_doc::DisplayUnits,
    text: &str,
    last: Option<Point2>,
) -> Result<Point2, String> {
    let text = text.trim();
    let (relative, rest) = match text.strip_prefix('@') {
        Some(r) => (true, r),
        None => (false, text.strip_prefix('#').unwrap_or(text)),
    };
    let base = if relative {
        last.unwrap_or(Point2::ORIGIN)
    } else {
        Point2::ORIGIN
    };
    let length = |s: &str, what: &str| {
        parse_value(units, "length", s.trim())
            .ok_or_else(|| format!("{what} {:?} is not a number", s.trim()))
    };
    let at = if let Some((len, angle)) = rest.split_once('<') {
        let len = length(len, "the length")?;
        let angle = parse_value(units, "angle", angle.trim())
            .ok_or_else(|| format!("the angle {:?} is not a number", angle.trim()))?;
        base + exact_direction(angle) * len
    } else {
        let (Some((x, y)), 1) = (rest.split_once(','), rest.matches(',').count()) else {
            return Err(
                "type x,y for a point, @dx,dy from the last point, or @length<angle".into(),
            );
        };
        let (x, y) = (length(x, "x")?, length(y, "y")?);
        base + Vec2::new(x, y)
    };
    if !(at.x.is_finite() && at.y.is_finite())
        || at.x.abs() > MAX_COORDINATE
        || at.y.abs() > MAX_COORDINATE
    {
        return Err(format!(
            "the point must be within {MAX_COORDINATE} mm of the origin"
        ));
    }
    Ok(at)
}

impl ChamferaApp {
    /// The keys for a typed point. Returns whether this frame's keys went to it, and
    /// whether Esc and Backspace were used.
    pub(crate) fn coordinate_keys(&mut self, ui: &egui::Ui) -> (bool, bool, bool) {
        let tool = self.sketch.tool();
        if !takes_coordinates(tool) {
            self.sketch.coord = None;
            return (false, false, false);
        }
        let fields = self.sketch.fields_shown();
        let events = ui.input(|i| i.events.clone());
        let mut used = self.sketch.coord.is_some();
        let (mut esc, mut back, mut enter) = (false, false, false);
        for e in events {
            match e {
                egui::Event::Text(t) => {
                    for ch in t.chars() {
                        match self.sketch.coord.as_mut() {
                            Some(s) => {
                                if coord_char(ch) && s.chars().count() < MAX_COORD_CHARS {
                                    s.push(ch);
                                }
                            }
                            None => {
                                let starts = matches!(ch, '@' | '#')
                                    || (!fields
                                        && (ch.is_ascii_digit() || matches!(ch, '-' | '.')));
                                if starts {
                                    self.sketch.coord = Some(ch.to_string());
                                    used = true;
                                }
                            }
                        }
                    }
                }
                egui::Event::Key {
                    key, pressed: true, ..
                } if self.sketch.coord.is_some() => match key {
                    egui::Key::Backspace => {
                        if let Some(s) = self.sketch.coord.as_mut() {
                            s.pop();
                            if s.is_empty() {
                                self.sketch.coord = None;
                            }
                        }
                        back = true;
                    }
                    egui::Key::Enter => enter = true,
                    egui::Key::Escape => {
                        self.sketch.coord = None;
                        esc = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if enter && let Some(text) = self.sketch.coord.take() {
            self.place_typed_coordinate(&text);
        }
        (used, esc, back)
    }

    /// Places the current tool's next point at a typed entry.
    pub(crate) fn place_typed_coordinate(&mut self, text: &str) {
        let last = self
            .sketch
            .clicks
            .last()
            .map(|c| c.at)
            .or(self.sketch.last_at);
        match parse_coordinate(self.units(), text, last) {
            Ok(at) => self.place_point_at(at),
            Err(e) => self.message = Some((format!("Typed point: {e}"), true)),
        }
    }

    /// Places the current tool's next point exactly at `at`, as a click with no snap.
    pub(crate) fn place_point_at(&mut self, at: Point2) {
        let tool = self.sketch.tool();
        if !takes_coordinates(tool) {
            return;
        }
        self.sketch.last_at = Some(at);
        let click = Click {
            at,
            point: None,
            snap: None,
            track: None,
            free: true,
        };
        if tool == Tool::Spline {
            if self
                .sketch
                .clicks
                .last()
                .is_some_and(|c| c.at.coincident(at))
            {
                self.spline_finish(false);
            } else {
                self.sketch.clicks.push(click);
            }
            return;
        }
        self.sketch.clicks.push(click);
        self.place_clicks(tool);
    }

    /// Turns Ortho mode on or off.
    pub(crate) fn toggle_ortho(&mut self) {
        self.sketch.ortho = !self.sketch.ortho;
        let state = if self.sketch.ortho {
            "on: lines go horizontal or vertical, Shift frees them"
        } else {
            "off"
        };
        self.message = Some((format!("Ortho {state}"), false));
    }

    /// The typed point near the cursor, with where it lands.
    pub(crate) fn coordinate_box(&self, ui: &egui::Ui, rect: Rect) {
        let Some(text) = self.sketch.coord.as_deref() else {
            return;
        };
        let last = self
            .sketch
            .clicks
            .last()
            .map(|c| c.at)
            .or(self.sketch.last_at);
        let parsed = parse_coordinate(self.units(), text, last);
        let u = self.units();
        let shown = match &parsed {
            Ok(p) => format!(
                "{text}   ({}, {})",
                u.length_number(p.x),
                u.length_number(p.y)
            ),
            Err(_) => format!("{text}   x,y · @dx,dy · @length<angle"),
        };
        let anchor = self
            .sketch
            .hover_px
            .map(|(x, y, _, _)| rect.min + egui::Vec2::new(x as f32, y as f32))
            .unwrap_or(rect.center());
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(12.5);
        let colour = if parsed.is_ok() {
            color(self.theme.text)
        } else {
            color(self.theme.text_muted)
        };
        let galley = painter.layout_no_wrap(shown, font, colour);
        let at = anchor + egui::Vec2::new(16.0, 18.0);
        let r = Rect::from_min_size(at, galley.size() + egui::Vec2::new(12.0, 6.0));
        let radius = egui::CornerRadius::same(crate::widgets::R_SMALL);
        painter.rect_filled(r, radius, color(self.theme.panel));
        painter.rect_stroke(
            r,
            radius,
            egui::Stroke::new(1.5, color(self.theme.accent)),
            egui::StrokeKind::Inside,
        );
        painter.galley(r.min + egui::Vec2::new(6.0, 3.0), galley, colour);
        // Where it lands.
        if let (Ok(p), Some(f)) = (parsed, self.session.active_sketch())
            && let Some(q) = self.project(rect, f.plane.from_plane(p))
        {
            let c = color(self.theme.accent);
            painter.circle_stroke(q, 5.0, egui::Stroke::new(1.6, c));
            painter.line_segment(
                [q - egui::Vec2::new(8.0, 0.0), q + egui::Vec2::new(8.0, 0.0)],
                egui::Stroke::new(1.0, c),
            );
            painter.line_segment(
                [q - egui::Vec2::new(0.0, 8.0), q + egui::Vec2::new(0.0, 8.0)],
                egui::Stroke::new(1.0, c),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_doc::DisplayUnits;

    fn close(a: Point2, b: Point2) -> bool {
        a.distance(b) < 1e-12
    }

    #[test]
    fn typed_points_are_absolute_relative_or_polar() {
        let u = DisplayUnits::default();
        let last = Some(Point2::new(10.0, 5.0));
        assert!(close(
            parse_coordinate(u, "30,40", last).unwrap(),
            Point2::new(30.0, 40.0)
        ));
        assert!(close(
            parse_coordinate(u, "#-2.5, 4", last).unwrap(),
            Point2::new(-2.5, 4.0)
        ));
        assert!(close(
            parse_coordinate(u, "@5,-3", last).unwrap(),
            Point2::new(15.0, 2.0)
        ));
        // @20<90: straight up from (10, 5), exactly (cos 90° is 0, not 6e-17).
        let up = parse_coordinate(u, "@20<90", last).unwrap();
        assert_eq!(up, Point2::new(10.0, 25.0));
        // @10<30 from the origin when nothing was placed: (10 cos 30°, 10 sin 30°)
        // = (5√3, 5).
        let p = parse_coordinate(u, "@10<30", None).unwrap();
        assert!(close(p, Point2::new(5.0 * 3f64.sqrt(), 5.0)), "{p:?}");
        for bad in ["", "@", "12", "a,b", "1,2,3", "@5<", "1e300,0", "@<45"] {
            assert!(parse_coordinate(u, bad, last).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn typed_lengths_are_in_the_design_unit() {
        let u = DisplayUnits {
            length: chamfera_doc::LengthUnit::In,
            ..DisplayUnits::default()
        };
        let p = parse_coordinate(u, "1,2", None).unwrap();
        assert!(close(p, Point2::new(25.4, 50.8)), "{p:?}");
    }
}

//! Drawing input driven by real pointer and key events through the whole UI: Shift
//! straightens lines, clicks snap, and typed values place and dimension the geometry.

use super::*;
use chamfera_sketch::Curve;
use eframe::App;
use egui::{Event, Key, Modifiers, PointerButton};

struct Driver {
    app: ChamferaApp,
    ctx: egui::Context,
    t: f64,
}

impl Driver {
    /// A headless app showing a new sketch on the top plane.
    fn new() -> Driver {
        let mut d = Driver {
            app: ChamferaApp::headless(),
            ctx: egui::Context::default(),
            t: 0.0,
        };
        for _ in 0..3 {
            d.frame(vec![]);
        }
        d.app.create_sketch("xy", "top");
        d.frame(vec![]);
        d.frame(vec![]);
        d
    }

    /// Runs one frame of the real UI with these input events, through the raw input hook
    /// as eframe does.
    fn frame(&mut self, events: Vec<Event>) {
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut f = eframe::Frame::_new_kittest();
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
        self.t += 0.05;
    }

    /// The screen position of sketch point (x, y).
    fn screen(&self, x: f64, y: f64) -> Pos2 {
        let f = self.app.session.active_sketch().unwrap();
        self.app
            .project(
                self.app.viewport_rect,
                f.plane.from_plane(Point2::new(x, y)),
            )
            .unwrap()
    }

    fn hover(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
    }

    fn click(&mut self, at: Pos2, modifiers: Modifiers) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        self.frame(vec![
            Event::ModifiersChanged(modifiers),
            Event::PointerMoved(at),
        ]);
        self.frame(vec![button(true)]);
        self.t += 0.03;
        self.frame(vec![button(false)]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn key(&mut self, key: Key) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }

    fn text(&mut self, s: &str) {
        self.frame(vec![Event::Text(s.into())]);
    }

    fn sketch(&self) -> &chamfera_sketch::Sketch {
        &self.app.session.active_sketch().unwrap().sketch
    }

    /// The ends of the only line drawn by the UI (not `skip`).
    fn new_line(&self, skip: &[CurveId]) -> (CurveId, Point2, Point2) {
        let s = self.sketch();
        let (id, start, end) = s
            .curves()
            .find_map(|(id, c)| match c {
                Curve::Line { start, end, .. } if !skip.contains(&id) => Some((id, *start, *end)),
                _ => None,
            })
            .expect("no line drawn");
        (id, s.point(start).unwrap(), s.point(end).unwrap())
    }

    fn has(&self, c: &Constraint) -> bool {
        self.sketch().constraints().any(|(_, k)| k == c)
    }
}

#[test]
fn shift_drawing_gives_an_exactly_horizontal_line_with_a_horizontal_constraint() {
    let mut d = Driver::new();
    let a = d.screen(10.0, 10.0);
    d.click(a, Modifiers::NONE);
    // 25 px up for 120 across is 11.8 degrees: far from level without Shift.
    let b = a + Vec2::new(120.0, -25.0);
    d.frame(vec![
        Event::ModifiersChanged(Modifiers::SHIFT),
        Event::PointerMoved(b),
    ]);
    // The preview is locked too: its end is level with the start.
    let hover = d.app.sketch.hover.unwrap();
    assert_eq!(hover.y, d.app.sketch.clicks[0].at.y);
    d.click(b, Modifiers::SHIFT);
    let (line, p, q) = d.new_line(&[]);
    assert_eq!(p.y, q.y, "{p:?} {q:?}");
    assert!(q.x - p.x > 1.0);
    assert!(d.has(&Constraint::Horizontal { line }));
}

#[test]
fn without_shift_the_same_clicks_give_a_sloping_line() {
    let mut d = Driver::new();
    let a = d.screen(10.0, 10.0);
    d.click(a, Modifiers::NONE);
    d.click(a + Vec2::new(120.0, -25.0), Modifiers::NONE);
    let (line, p, q) = d.new_line(&[]);
    // Screen y grows downward, so 25 px up is +Y: atan(25 / 120) = 11.77 degrees.
    let angle = (q - p).angle().to_degrees();
    assert!((angle - 11.77).abs() < 0.1, "{angle}");
    assert!(!d.has(&Constraint::Horizontal { line }));
}

#[test]
fn a_click_near_a_midpoint_snaps_there_and_adds_a_midpoint_constraint() {
    let mut d = Driver::new();
    // The midpoint of (10, 0)-(50, 20) is (30, 10).
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 10, "y1": 0, "x2": 50, "y2": 20 }),
        )
        .unwrap();
    let base = CurveId(as_id(&r["curve"]).unwrap());
    d.app.sketch.tool = Some(Tool::Line);
    d.click(d.screen(60.0, 40.0), Modifiers::NONE);
    let near = d.screen(30.0, 10.0) + Vec2::new(3.0, 2.0);
    d.hover(near);
    let snap = d.app.sketch.snap.expect("no snap shown");
    assert_eq!(snap.kind, SnapKind::Midpoint);
    d.click(near, Modifiers::NONE);
    let (_, _, end) = d.new_line(&[base]);
    assert!(end.distance(Point2::new(30.0, 10.0)) < 1e-9, "{end:?}");
    let mid = d
        .sketch()
        .points()
        .find(|(_, p)| p.distance(Point2::new(30.0, 10.0)) < 1e-9)
        .map(|(id, _)| id)
        .unwrap();
    assert!(d.has(&Constraint::Midpoint {
        point: mid,
        line: base
    }));
}

#[test]
fn typing_a_length_and_enter_places_the_line_with_a_length_dimension() {
    let mut d = Driver::new();
    let a = d.screen(10.0, 10.0);
    d.click(a, Modifiers::NONE);
    d.hover(d.screen(40.0, 30.0));
    assert!(d.app.sketch.fields_shown());
    d.text("25");
    assert_eq!(d.app.sketch.typed.fields[0], "25");
    d.key(Key::Enter);
    let (line, p, q) = d.new_line(&[]);
    assert!((p.distance(q) - 25.0).abs() < 1e-9, "{}", p.distance(q));
    // Toward the cursor: (30, 20) / |(30, 20)|, to within the pixel the cursor is on.
    let dir = (q - p) * (1.0 / 25.0);
    let want = chamfera_geom::Vec2::new(30.0, 20.0) * (1.0 / 1300f64.sqrt());
    assert!((dir - want).length() < 0.01, "{dir:?}");
    assert!(d.has(&Constraint::Length { line, value: 25.0 }));
    // The chain goes on from the new end, with the fields empty again.
    assert_eq!(d.app.sketch.clicks.len(), 1);
    assert!(!d.app.sketch.typed.any());
}

#[test]
fn tab_moves_to_the_angle_field_so_30_tab_90_enter_draws_up_30() {
    let mut d = Driver::new();
    d.click(d.screen(10.0, 10.0), Modifiers::NONE);
    d.hover(d.screen(40.0, 12.0));
    d.text("30");
    d.key(Key::Tab);
    assert_eq!(d.app.sketch.typed.active, 1);
    d.text("90");
    d.key(Key::Enter);
    let (line, p, q) = d.new_line(&[]);
    // 90 degrees is exactly up: the same x, 30 higher.
    assert_eq!(p.x, q.x);
    assert!((q.y - p.y - 30.0).abs() < 1e-9);
    assert!(d.has(&Constraint::Vertical { line }));
    assert!(d.has(&Constraint::Length { line, value: 30.0 }));
}

#[test]
fn a_typed_diameter_and_a_typed_rectangle_get_their_dimensions() {
    let mut d = Driver::new();
    d.app.sketch.tool = Some(Tool::Circle);
    d.click(d.screen(20.0, 20.0), Modifiers::NONE);
    d.hover(d.screen(25.0, 20.0));
    d.text("16");
    d.key(Key::Enter);
    let s = d.sketch();
    let (circle, r) = s
        .curves()
        .find_map(|(id, c)| matches!(c, Curve::Circle { .. }).then(|| (id, s.radius(id))))
        .unwrap();
    assert!((r.unwrap() - 8.0).abs() < 1e-9);
    assert!(d.has(&Constraint::Diameter {
        curve: circle,
        value: 16.0
    }));

    d.app.sketch.tool = Some(Tool::Rectangle);
    d.app.sketch.reset_tool();
    d.click(d.screen(40.0, 10.0), Modifiers::NONE);
    d.hover(d.screen(50.0, 15.0));
    d.text("30");
    d.key(Key::Tab);
    d.text("12");
    d.key(Key::Enter);
    let lengths: Vec<f64> = d
        .sketch()
        .constraints()
        .filter_map(|(_, c)| match c {
            Constraint::Length { value, .. } => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(lengths, vec![30.0, 12.0]);
    // 30 x 12 = 360 next to the disc of area 64 pi.
    let mut areas: Vec<f64> = d.sketch().profiles().iter().map(|r| r.area).collect();
    areas.sort_by(f64::total_cmp);
    let disc = 64.0 * std::f64::consts::PI;
    assert!(
        (areas[0] - disc).abs() < 1e-6 && (areas[1] - 360.0).abs() < 1e-6,
        "{areas:?}"
    );
}

#[test]
fn esc_clears_typed_values_before_it_ends_the_tool() {
    let mut d = Driver::new();
    d.click(d.screen(10.0, 10.0), Modifiers::NONE);
    d.hover(d.screen(20.0, 10.0));
    d.text("12");
    d.key(Key::Escape);
    assert!(!d.app.sketch.typed.any());
    assert_eq!(
        d.app.sketch.clicks.len(),
        1,
        "the line is still being drawn"
    );
    d.key(Key::Escape);
    assert!(d.app.sketch.clicks.is_empty());
}

#[test]
fn sketch_input_steps_parse_and_refuse_nonsense() {
    let steps = parse_demo("tool line; click 0,0; hover 30, 12; shift; type 25; tab").unwrap();
    assert_eq!(
        steps,
        vec![
            DemoStep::Tool(Tool::Line),
            DemoStep::Click(Point2::new(0.0, 0.0)),
            DemoStep::Hover(Point2::new(30.0, 12.0)),
            DemoStep::Shift,
            DemoStep::Type("25".into()),
            DemoStep::Tab,
        ]
    );
    assert!(parse_demo("tool teleport").is_err());
    assert!(parse_demo("click 1").is_err());
    assert!(parse_demo("click nan,1e400").is_err());
    assert!(parse_demo("dance").is_err());
}

/// Ctrl, as egui reports it on Linux and Windows (and Cmd on a Mac).
const CTRL: Modifiers = Modifiers {
    ctrl: true,
    command: true,
    ..Modifiers::NONE
};

#[test]
fn ctrl_places_the_click_freely_with_no_snap_and_no_inference() {
    let mut d = Driver::new();
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 10, "y1": 10, "x2": 50, "y2": 10 }),
        )
        .unwrap();
    let base = CurveId(as_id(&r["curve"]).unwrap());
    let points_before = d.sketch().points().count();
    d.app.sketch.tool = Some(Tool::Line);
    // 2 px right of the end (50, 10): without Ctrl this would reuse that point.
    let start = d.screen(50.0, 10.0) + Vec2::new(2.0, 0.0);
    d.click(start, CTRL);
    assert!(d.app.sketch.clicks[0].point.is_none(), "no point reused");
    assert!(d.app.sketch.clicks[0].free);
    // 1 px up over 200 across is 0.29 degrees: within the 2 degree H/V inference, which
    // Ctrl suspends.
    let end = start + Vec2::new(200.0, -1.0);
    d.frame(vec![
        Event::ModifiersChanged(CTRL),
        Event::PointerMoved(end),
    ]);
    assert!(d.app.sketch.snap.is_none() && d.app.sketch.track.is_none());
    d.click(end, CTRL);
    let (line, p, q) = d.new_line(&[base]);
    assert!(p.distance(Point2::new(50.0, 10.0)) > 1e-3, "{p:?}");
    assert!(q.y > p.y, "the line still slopes: {p:?} {q:?}");
    assert!(!d.has(&Constraint::Horizontal { line }));
    assert_eq!(d.sketch().points().count(), points_before + 2);
}

#[test]
fn a_line_from_the_end_of_a_line_is_held_at_right_angles_and_constrained() {
    let mut d = Driver::new();
    // The first line slopes (3, 4), so its perpendicular at (30, 40) is (-4, 3): 25 mm
    // along it is (10, 55). Neither is horizontal nor vertical, so only the
    // perpendicular guide can explain where the end lands.
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 0, "y1": 0, "x2": 30, "y2": 40 }),
        )
        .unwrap();
    let base = CurveId(as_id(&r["curve"]).unwrap());
    d.app.sketch.tool = Some(Tool::Line);
    d.click(d.screen(30.0, 40.0), Modifiers::NONE);
    assert!(d.app.sketch.clicks[0].point.is_some(), "starts on the end");
    // 3 px off the guide.
    let near = d.screen(10.0, 55.0) + Vec2::new(2.0, 2.0);
    d.hover(near);
    let track = d.app.sketch.track.expect("no guide shown");
    assert_eq!(
        track.direction().map(|g| g.kind),
        Some(GuideKind::Perpendicular)
    );
    d.click(near, Modifiers::NONE);
    let (line, p, q) = d.new_line(&[base]);
    assert!(p.distance(Point2::new(30.0, 40.0)) < 1e-9);
    // On the guide: (q - p) . (3, 4) = 0, to rounding.
    assert!(
        (q - p).dot(chamfera_geom::Vec2::new(3.0, 4.0)).abs() < 1e-6,
        "{q:?}"
    );
    assert!(d.has(&Constraint::Perpendicular { a: line, b: base }));
}

#[test]
fn a_typed_length_along_the_perpendicular_guide_keeps_the_constraint() {
    let mut d = Driver::new();
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 0, "y1": 0, "x2": 30, "y2": 40 }),
        )
        .unwrap();
    let base = CurveId(as_id(&r["curve"]).unwrap());
    d.app.sketch.tool = Some(Tool::Line);
    d.click(d.screen(30.0, 40.0), Modifiers::NONE);
    d.hover(d.screen(10.0, 55.0) + Vec2::new(2.0, 2.0));
    d.text("20");
    d.key(Key::Enter);
    let (line, p, q) = d.new_line(&[base]);
    // 20 along (-4, 3) / 5 from (30, 40): (14, 52).
    assert!(q.distance(Point2::new(14.0, 52.0)) < 1e-6, "{p:?} {q:?}");
    assert!(d.has(&Constraint::Perpendicular { a: line, b: base }));
    assert!(d.has(&Constraint::Length { line, value: 20.0 }));
}

#[test]
fn the_cursor_lines_up_with_a_point_on_an_alignment_guide() {
    let mut d = Driver::new();
    d.app
        .run_sketch("sketch.point", json!({ "x": 40, "y": 30 }))
        .unwrap();
    d.app.sketch.tool = Some(Tool::Circle);
    // 4 px right of x = 40, far below the point: the centre lands on x = 40.
    let near = d.screen(40.0, 5.0) + Vec2::new(4.0, 0.0);
    d.hover(near);
    let track = d.app.sketch.track.expect("no guide shown");
    assert_eq!(
        track.guides[0].map(|g| g.kind),
        Some(GuideKind::AlignVertical)
    );
    d.click(near, Modifiers::NONE);
    assert_eq!(d.app.sketch.clicks[0].at.x, 40.0);
    // With alignment off in the preferences the same spot stays where it is.
    d.app.sketch.reset_tool();
    d.app.prefs.snaps.alignment = false;
    d.hover(near);
    assert!(d.app.sketch.track.is_none());
    assert!((d.app.sketch.hover.unwrap().x - 40.0).abs() > 0.1);
}

#[test]
fn snap_settings_turn_a_snap_off_and_widen_the_radius() {
    let mut d = Driver::new();
    // The midpoint of (10, 0)-(50, 20) is (30, 10), as in the snap test above.
    d.app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 10, "y1": 0, "x2": 50, "y2": 20 }),
        )
        .unwrap();
    d.app.prefs.snaps.alignment = false;
    d.app.sketch.tool = Some(Tool::Line);
    d.click(d.screen(60.0, 40.0), Modifiers::NONE);
    // 14 px from the midpoint, across the line: beyond the 9 px default radius.
    let far = d.screen(30.0, 10.0) + Vec2::new(-6.0, -13.0);
    let midpoint = |d: &Driver| d.app.sketch.snap.map(|s| s.kind) == Some(SnapKind::Midpoint);
    d.hover(far);
    assert!(!midpoint(&d));
    d.app.prefs.snaps.radius_px = 20.0;
    d.hover(far);
    assert!(midpoint(&d));
    d.app.prefs.snaps.kinds = d.app.prefs.snaps.kinds.with(SnapKind::Midpoint, false);
    d.hover(far);
    assert!(!midpoint(&d));
}

#[test]
fn polar_guides_hold_a_line_on_whole_steps() {
    let mut d = Driver::new();
    d.app.prefs.snaps.polar = true;
    d.app.prefs.snaps.polar_step_deg = 15.0;
    d.app.prefs.snaps.alignment = false;
    d.click(d.screen(10.0, 10.0), Modifiers::NONE);
    // 44 degrees from (10, 10), 40 mm out: pulled onto 45 degrees.
    let a = 44f64.to_radians();
    d.click(
        d.screen(10.0 + 40.0 * a.cos(), 10.0 + 40.0 * a.sin()),
        Modifiers::NONE,
    );
    let (_, p, q) = d.new_line(&[]);
    let angle = (q - p).angle().to_degrees();
    assert!((angle - 45.0).abs() < 1e-9, "{angle}");
}

/// The standalone points of the sketch (points no curve uses).
fn loose_points(d: &Driver) -> Vec<Point2> {
    let s = d.sketch();
    s.points()
        .filter(|(id, _)| !s.curves().any(|(_, c)| c.points().contains(id)))
        .map(|(_, p)| p)
        .collect()
}

#[test]
fn the_point_tool_places_typed_and_snapped_points() {
    let mut d = Driver::new();
    d.key(Key::P);
    assert_eq!(d.app.sketch.tool(), Tool::Point);
    // Typed: exactly (15, 10), then @5,-3 from it: (20, 7).
    d.text("15,10");
    d.key(Key::Enter);
    d.text("@5,-3");
    d.key(Key::Enter);
    let pts = loose_points(&d);
    assert_eq!(pts, vec![Point2::new(15.0, 10.0), Point2::new(20.0, 7.0)]);
    // A click on the midpoint of a line places a point held at its middle.
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 40, "y1": 0, "x2": 80, "y2": 20 }),
        )
        .unwrap();
    let line = CurveId(r["curve"].as_u64().unwrap() as u32);
    d.click(d.screen(60.0, 10.0), Modifiers::NONE);
    let held = d
        .sketch()
        .constraints()
        .find_map(|(_, c)| match c {
            Constraint::Midpoint { point, line: l } if *l == line => Some(*point),
            _ => None,
        })
        .expect("no midpoint constraint");
    assert_eq!(d.sketch().point(held), Some(Point2::new(60.0, 10.0)));
}

#[test]
fn typed_points_draw_a_line_by_relative_and_polar_entries() {
    let mut d = Driver::new();
    d.key(Key::L);
    // Start at (10, 10), then @30<0: (40, 10), then @0,20: (40, 30).
    d.text("10,10");
    d.key(Key::Enter);
    d.text("@30<0");
    d.key(Key::Enter);
    d.text("@0,20");
    d.key(Key::Enter);
    let s = d.sketch();
    let mut ends: Vec<(Point2, Point2)> = s
        .curves()
        .filter_map(|(_, c)| match c {
            Curve::Line { start, end, .. } => Some((s.point(*start)?, s.point(*end)?)),
            _ => None,
        })
        .collect();
    ends.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    assert_eq!(
        ends,
        vec![
            (Point2::new(10.0, 10.0), Point2::new(40.0, 10.0)),
            (Point2::new(40.0, 10.0), Point2::new(40.0, 30.0)),
        ]
    );
    // A bad entry places nothing and says why.
    d.text("@abc");
    d.key(Key::Enter);
    d.text("@1");
    d.key(Key::Enter);
    assert_eq!(d.sketch().curves().count(), 2);
    assert!(
        d.app
            .message
            .as_ref()
            .is_some_and(|m| m.0.contains("Typed point"))
    );
}

#[test]
fn ortho_mode_keeps_lines_straight_until_shift_frees_them() {
    let mut d = Driver::new();
    d.app.prefs.snaps.alignment = false;
    d.app.prefs.snaps.polar = false;
    d.key(Key::F8);
    assert!(d.app.sketch.ortho);
    d.key(Key::L);
    d.click(d.screen(10.0, 10.0), Modifiers::NONE);
    // 40 right and 7 up: Ortho makes it level, (50, 10).
    d.click(d.screen(50.0, 17.0), Modifiers::NONE);
    let (first, p, q) = d.new_line(&[]);
    assert!(p.distance(Point2::new(10.0, 10.0)) < 0.01, "{p:?}");
    assert!(
        (q.y - p.y).abs() < 1e-9 && (q.x - 50.0).abs() < 0.5,
        "{q:?}"
    );
    // Shift frees the next one: it goes where clicked, up and to the right.
    d.click(d.screen(70.0, 37.0), Modifiers::SHIFT);
    let (_, a, b) = d.new_line(&[first]);
    assert!(b.y - a.y > 20.0 && b.x - a.x > 15.0, "{a:?} {b:?}");
    d.key(Key::F8);
    assert!(!d.app.sketch.ortho);
}

#[test]
fn a_constraint_marker_selects_and_del_deletes_its_constraint() {
    let mut d = Driver::new();
    let r = d
        .app
        .run_sketch(
            "sketch.line",
            json!({ "x1": 0, "y1": 0, "x2": 40, "y2": 0 }),
        )
        .unwrap();
    let line = r["curve"].as_u64().unwrap();
    d.app
        .run_sketch(
            "sketch.constrain",
            json!({ "constraint": {"kind": "horizontal", "line": line} }),
        )
        .unwrap();
    d.app.sketch.tool = Some(Tool::Select);
    d.frame(vec![]);
    // The marker sits 13 px right of and above the middle of the line, (20, 0).
    let marker = d.screen(20.0, 0.0) + Vec2::new(13.0, -13.0);
    d.click(marker, Modifiers::NONE);
    let horizontal = d
        .sketch()
        .constraints()
        .find(|(_, c)| matches!(c, Constraint::Horizontal { .. }))
        .map(|(id, _)| id.0)
        .unwrap();
    assert_eq!(d.app.sketch.glyph_selected, Some(horizontal));
    d.key(Key::Delete);
    assert!(
        !d.sketch()
            .constraints()
            .any(|(_, c)| matches!(c, Constraint::Horizontal { .. }))
    );
    // The line itself stays.
    assert_eq!(d.sketch().curves().count(), 1);
}

#[test]
fn a_conflicting_constraint_opens_the_dialog_and_replace_swaps_it() {
    let mut d = Driver::new();
    // p fixed at (0, 0), q level with it 10 away; fixing q at (6, 8) conflicts with p's
    // fix and with "level" (see the sketch crate's conflict test).
    let p = d
        .app
        .run_sketch("sketch.point", json!({"x": 0, "y": 0}))
        .unwrap()["point"]
        .clone();
    let q = d
        .app
        .run_sketch("sketch.point", json!({"x": 10, "y": 0}))
        .unwrap()["point"]
        .clone();
    for c in [
        json!({"kind": "fix", "point": p, "at": {"x": 0, "y": 0}}),
        json!({"kind": "horizontal_points", "a": p, "b": q}),
        json!({"kind": "distance", "a": p, "b": q, "value": 10}),
    ] {
        d.app
            .run_sketch("sketch.constrain", json!({ "constraint": c }))
            .unwrap();
    }
    let fix_q = json!({"kind": "fix", "point": q, "at": {"x": 6, "y": 8}});
    d.app.open_conflict_for(&fix_q).unwrap();
    let dialog = d.app.sketch.conflict.clone().unwrap();
    assert_eq!(dialog.involved.len(), 2, "{dialog:?}");
    assert!(dialog.involved[1].1.starts_with("Horizontal (points)"));
    d.frame(vec![]);
    let level = dialog.involved[1].0;
    d.app.conflict_replace(level);
    assert!(d.app.sketch.conflict.is_none());
    let q = PointId(q.as_u64().unwrap() as u32);
    let at = d.sketch().point(q).unwrap();
    assert!(at.distance(Point2::new(6.0, 8.0)) < 1e-9, "{at:?}");
    // A constraint that fits opens nothing.
    assert!(
        d.app
            .open_conflict_for(&json!({"kind": "fix", "point": p, "at": {"x": 0, "y": 0}}))
            .is_err()
    );
}

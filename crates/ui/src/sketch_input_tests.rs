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

//! In-app guided tours: a card over the viewport says what to do next, the ribbon tool to
//! use is outlined in the `accent` token, and the step is done when the app runs the
//! command the step expects (`tutorial_saw`, called from `ChamferaApp::run`).
//!
//! The tours are data: one JSON file per tour in `assets/tutorials/`, built into the
//! binary. Each step names a ribbon tool by its id (`tool_id`, from `ribbon::TOOLS`) and
//! the commands that finish it. The files are read as hostile input: a broken one is left
//! out, never a panic.

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::ribbon::{self, RibbonTool, Workspace};
use crate::shortcuts::Action;
use crate::widgets::{self, S2, S3, S5, T_BODY, T_CAPTION};
use egui::{Rect, Stroke};
use serde_json::Value;

/// The tours, as built into the binary: (id, JSON).
pub const SOURCES: [(&str, &str); 3] = [
    (
        "first-part",
        include_str!("../../../assets/tutorials/first-part.json"),
    ),
    (
        "parametric-plate",
        include_str!("../../../assets/tutorials/parametric-plate.json"),
    ),
    (
        "first-drawing",
        include_str!("../../../assets/tutorials/first-drawing.json"),
    ),
];

/// Most steps a tour may have, and the longest text kept.
const MAX_STEPS: usize = 64;
const MAX_TEXT: usize = 600;

/// One step: what to do, the tool to outline and the commands that finish it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub title: String,
    pub text: String,
    /// A ribbon tool's id (`tool_id`).
    pub tool: String,
    /// Any of these commands, run successfully, finishes the step.
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tutorial {
    pub id: String,
    pub title: String,
    pub summary: String,
    /// The workspace the tour starts in.
    pub workspace: Workspace,
    /// A sample (`welcome_ui::SAMPLES` key) opened first when the design has no solids.
    pub sample: Option<String>,
    pub steps: Vec<Step>,
    /// Shown when every step is done.
    pub finish: String,
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" must be text"))?;
    if s.trim().is_empty() || s.len() > MAX_TEXT {
        return Err(format!("\"{key}\" must be 1 to {MAX_TEXT} characters long"));
    }
    Ok(s.to_owned())
}

fn workspace(name: &str) -> Option<Workspace> {
    Workspace::ALL
        .into_iter()
        .find(|w| w.name().eq_ignore_ascii_case(name))
}

/// Reads one tour. Every field is checked; nothing here panics on any input.
pub fn parse(source: &str) -> Result<Tutorial, String> {
    let v: Value = serde_json::from_str(source).map_err(|e| e.to_string())?;
    let steps = v
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("\"steps\" must be a list")?;
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("a tour has 1 to {MAX_STEPS} steps"));
    }
    let steps = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let commands: Vec<String> = s
                .get("commands")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            if commands.is_empty() {
                return Err(format!(
                    "step {}: \"commands\" must list command ids",
                    i + 1
                ));
            }
            Ok(Step {
                title: text(s, "title").map_err(|e| format!("step {}: {e}", i + 1))?,
                text: text(s, "text").map_err(|e| format!("step {}: {e}", i + 1))?,
                tool: text(s, "tool").map_err(|e| format!("step {}: {e}", i + 1))?,
                commands,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ws = v
        .get("workspace")
        .and_then(Value::as_str)
        .unwrap_or("design");
    Ok(Tutorial {
        id: text(&v, "id")?,
        title: text(&v, "title")?,
        summary: text(&v, "summary")?,
        workspace: workspace(ws).ok_or_else(|| format!("no workspace \"{ws}\""))?,
        sample: v.get("sample").and_then(Value::as_str).map(str::to_owned),
        steps,
        finish: text(&v, "finish")?,
    })
}

/// Every built-in tour that reads.
pub fn all() -> Vec<Tutorial> {
    SOURCES.iter().filter_map(|(_, s)| parse(s).ok()).collect()
}

/// A built-in tour by id.
pub fn find(id: &str) -> Option<Tutorial> {
    all().into_iter().find(|t| t.id == id)
}

/// `CentreRectangle` -> `centre_rectangle`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A ribbon action's id, as tours name tools: the command id for `Action::Tool`, the
/// command a button runs for the solid tools with dialogs of their own (`solid.extrude`),
/// `sketch.tool.<name>` for drawing tools, else the action's name (`parameters`).
pub fn action_id(action: Action) -> String {
    match action {
        Action::Tool(id) | Action::Command(id) => id.to_owned(),
        Action::SketchTool(t) => format!("sketch.tool.{}", snake(&format!("{t:?}"))),
        Action::Extrude => "solid.extrude".into(),
        Action::Revolve => "solid.revolve".into(),
        Action::PressPull => "solid.press_pull".into(),
        Action::Sweep => "solid.sweep".into(),
        Action::Loft => "solid.loft".into(),
        Action::Fillet => "solid.fillet".into(),
        Action::Chamfer => "solid.chamfer".into(),
        Action::FinishSketch => "sketch.finish".into(),
        Action::Dimension => "sketch.dimension".into(),
        Action::Construction => "sketch.construction".into(),
        other => snake(&format!("{other:?}")),
    }
}

/// The id of a ribbon tool.
pub fn tool_id(t: &RibbonTool) -> String {
    action_id(t.action)
}

/// The ribbon tool with this id.
pub fn tool_by_id(id: &str) -> Option<&'static RibbonTool> {
    ribbon::TOOLS.iter().find(|t| tool_id(t) == id)
}

fn rect_key(id: &str) -> egui::Id {
    egui::Id::unique(("chamfera ribbon tool", id))
}

/// Where the ribbon drew a tool button in this frame (the hook in `ribbon::draw_group`),
/// so a tour can outline it.
pub(crate) fn note_tool_rect(ctx: &egui::Context, t: &RibbonTool, rect: Rect) {
    let pass = ctx.cumulative_pass_nr();
    let key = rect_key(&tool_id(t));
    ctx.data_mut(|d| d.insert_temp(key, (pass, rect)));
}

/// The tool's button as drawn in this frame or the one before, if it was drawn.
pub(crate) fn tool_rect(ctx: &egui::Context, id: &str) -> Option<Rect> {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(u64, Rect)>(rect_key(id)))
        .filter(|(pass, _)| pass + 1 >= now)
        .map(|(_, r)| r)
}

/// The tour in progress.
#[derive(Debug, Clone)]
pub struct Running {
    pub tutorial: Tutorial,
    /// The step to do; `steps.len()` once every step is done.
    pub step: usize,
}

impl Running {
    pub fn finished(&self) -> bool {
        self.step >= self.tutorial.steps.len()
    }
}

#[derive(Debug, Default)]
pub struct TutorialUi {
    pub running: Option<Running>,
}

impl ChamferaApp {
    /// Starts a tour: opens its sample when the design has no solids, switches to its
    /// workspace and closes the welcome panel. False when there is no such tour.
    pub(crate) fn start_tutorial(&mut self, id: &str) -> bool {
        let Some(t) = find(id) else {
            self.message = Some((format!("No tutorial called \"{id}\""), true));
            return false;
        };
        if let Some(key) = &t.sample
            && self.session.solids().is_empty()
            && let Some(i) = crate::welcome_ui::SAMPLES.iter().position(|s| s.key == key)
        {
            self.open_sample(i);
        }
        if self.session.active_sketch.is_some() && t.workspace != Workspace::Design {
            self.run("sketch.finish", serde_json::json!({}));
        }
        self.set_workspace(t.workspace);
        self.welcome.open = false;
        self.tutorial.running = Some(Running {
            tutorial: t,
            step: 0,
        });
        true
    }

    /// A command ran: if it is what the current step waits for, the step is done.
    pub(crate) fn tutorial_saw(&mut self, command: &str) {
        let Some(r) = self.tutorial.running.as_mut() else {
            return;
        };
        if r.tutorial
            .steps
            .get(r.step)
            .is_some_and(|s| s.commands.iter().any(|c| c == command))
        {
            r.step += 1;
        }
    }

    /// The tour's card over the viewport's bottom-left corner, and the outline around the
    /// tool to use.
    pub(crate) fn tutorial_overlay(&mut self, ctx: &egui::Context) {
        let Some(run) = self.tutorial.running.clone() else {
            return;
        };
        let look = self.look();
        let step = if run.finished() {
            None
        } else {
            run.tutorial.steps.get(run.step)
        };
        let tool = step.and_then(|s| tool_by_id(&s.tool));
        let tab = self.ribbon_tab();
        // The button if it is shown, else its group's caption (the tool is in its list).
        let target = step.and_then(|s| tool_rect(ctx, &s.tool)).or_else(|| {
            let t = tool.filter(|t| t.tab == tab)?;
            self.ribbon
                .laid_out
                .iter()
                .find(|(caption, _, _)| *caption == t.group)
                .map(|(_, r, _)| *r)
        });
        if let Some(r) = target {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("tutorial outline"),
            ));
            let accent = look.c(|t| t.accent);
            painter.rect_filled(
                r.expand(2.0),
                egui::CornerRadius::same(widgets::R_MEDIUM),
                accent.gamma_multiply(0.12),
            );
            painter.rect_stroke(
                r.expand(2.0),
                egui::CornerRadius::same(widgets::R_MEDIUM),
                Stroke::new(2.0, accent),
                egui::StrokeKind::Outside,
            );
        }
        let where_hint = match tool {
            Some(t) if t.tab != tab && target.is_none() => {
                Some(format!("The tool is on the {} tab.", t.tab.title()))
            }
            _ => None,
        };
        let total = run.tutorial.steps.len();
        // The sheet takes the viewport's place in the Drawing workspace.
        let vr = match self.drawing.rect {
            Some(r) if self.workspace() == Workspace::Drawing => r,
            _ => self.viewport_rect,
        };
        let mut act = None;
        egui::Area::new(egui::Id::unique("tutorial card"))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .fixed_pos(vr.left_bottom() + egui::vec2(S5, -S5 - 40.0))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ctx, |ui| {
                widgets::dialog_frame(look).show(ui, |ui| {
                    ui.set_width(330.0);
                    if widgets::dialog_title(ui, look, Icon::Select, &run.tutorial.title) {
                        act = Some(Act::End);
                    }
                    widgets::dialog_body(ui, |ui| {
                        let caption = match step {
                            Some(_) => format!("STEP {} OF {total}", run.step + 1),
                            None => "TUTORIAL COMPLETE".to_owned(),
                        };
                        widgets::caption(ui, look, &caption);
                        ui.add_space(S2);
                        let (title, body) = match step {
                            Some(s) => (s.title.as_str(), s.text.as_str()),
                            None => ("Well done", run.tutorial.finish.as_str()),
                        };
                        ui.label(
                            egui::RichText::new(title)
                                .size(widgets::T_TITLE)
                                .color(look.c(|t| t.text)),
                        );
                        ui.add_space(S2);
                        ui.label(
                            egui::RichText::new(body)
                                .size(T_BODY)
                                .color(look.c(|t| t.text)),
                        );
                        if let Some(h) = &where_hint {
                            ui.add_space(S2);
                            ui.label(
                                egui::RichText::new(h)
                                    .size(T_CAPTION + 1.0)
                                    .color(look.c(|t| t.text_muted)),
                            );
                        }
                        ui.add_space(S3);
                        // Progress: one dot per step, done ones filled.
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = S2;
                            for i in 0..total {
                                let c = if i < run.step {
                                    look.c(|t| t.accent)
                                } else {
                                    look.c(|t| t.border)
                                };
                                widgets::dot(ui, c, 7.0);
                            }
                        });
                    });
                    widgets::dialog_footer(ui, look, |ui| {
                        if step.is_none() {
                            if widgets::primary_button(ui, look, "Close", true).clicked() {
                                act = Some(Act::End);
                            }
                            return;
                        }
                        if widgets::secondary_button(ui, look, "Skip Step").clicked() {
                            act = Some(Act::Skip);
                        }
                        if run.step > 0 && widgets::secondary_button(ui, look, "Back").clicked() {
                            act = Some(Act::Back);
                        }
                        if widgets::secondary_button(ui, look, "End Tour").clicked() {
                            act = Some(Act::End);
                        }
                    });
                });
            });
        match act {
            Some(Act::End) => self.tutorial.running = None,
            Some(Act::Skip) => {
                if let Some(r) = self.tutorial.running.as_mut() {
                    r.step = (r.step + 1).min(total);
                }
            }
            Some(Act::Back) => {
                if let Some(r) = self.tutorial.running.as_mut() {
                    r.step = r.step.saturating_sub(1);
                }
            }
            None => {}
        }
    }
}

enum Act {
    End,
    Skip,
    Back,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_tour_reads_and_has_three_to_twelve_steps() {
        for (id, source) in SOURCES {
            let t = parse(source).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(t.id, id, "the file's id matches its entry");
            assert!(
                (3..=12).contains(&t.steps.len()),
                "{id}: {} steps",
                t.steps.len()
            );
        }
        assert_eq!(all().len(), SOURCES.len());
    }

    #[test]
    fn every_tutorial_step_names_a_ribbon_tool_and_registry_commands() {
        let commands: Vec<&str> = chamfera_engine::registry().iter().map(|c| c.id).collect();
        for t in all() {
            for (i, s) in t.steps.iter().enumerate() {
                assert!(
                    tool_by_id(&s.tool).is_some(),
                    "{} step {}: no ribbon tool \"{}\"",
                    t.id,
                    i + 1,
                    s.tool
                );
                for c in &s.commands {
                    assert!(
                        commands.contains(&c.as_str()),
                        "{} step {}: no command \"{c}\"",
                        t.id,
                        i + 1
                    );
                }
            }
            if let Some(key) = &t.sample {
                assert!(
                    crate::welcome_ui::SAMPLES.iter().any(|s| s.key == key),
                    "{}: no sample \"{key}\"",
                    t.id
                );
            }
        }
    }

    #[test]
    fn tool_ids_are_the_command_or_the_tool_name() {
        assert!(tool_by_id("solid.extrude").is_some());
        assert!(tool_by_id("sketch.tool.rectangle").is_some());
        assert!(tool_by_id("sketch.tool.centre_rectangle").is_some());
        assert!(tool_by_id("drawing.view.front").is_some());
        assert!(tool_by_id("parameters").is_some());
        assert!(tool_by_id("no.such.tool").is_none());
    }

    #[test]
    fn broken_tours_are_refused_with_a_reason_and_never_panic() {
        let cases = [
            "",
            "null",
            "[]",
            "{\"steps\": 3}",
            "{\"id\": \"x\", \"steps\": []}",
            "{\"id\": \"x\", \"title\": \"t\", \"summary\": \"s\", \"finish\": \"f\", \
             \"steps\": [{\"title\": \"a\", \"text\": \"b\", \"tool\": \"c\"}]}",
            "{\"id\": \"x\", \"title\": \"t\", \"summary\": \"s\", \"finish\": \"f\", \
             \"workspace\": \"kitchen\", \"steps\": [{\"title\": \"a\", \"text\": \"b\", \
             \"tool\": \"c\", \"commands\": [\"d\"]}]}",
        ];
        for c in cases {
            assert!(parse(c).is_err(), "{c}");
        }
        let long = format!(
            "{{\"id\": \"{}\", \"steps\": []}}",
            "x".repeat(MAX_TEXT + 1)
        );
        assert!(parse(&long).is_err());
    }

    #[test]
    fn a_step_is_done_by_its_command_and_not_by_others() {
        let mut app = ChamferaApp::headless();
        assert!(app.start_tutorial("first-part"));
        app.tutorial_saw("view.fit");
        assert_eq!(app.tutorial.running.as_ref().map(|r| r.step), Some(0));
        app.tutorial_saw("sketch.create");
        assert_eq!(app.tutorial.running.as_ref().map(|r| r.step), Some(1));
        // A later step's command does not skip ahead.
        app.tutorial_saw("solid.hole");
        assert_eq!(app.tutorial.running.as_ref().map(|r| r.step), Some(1));
        assert!(!app.start_tutorial("no-such-tour"));
    }

    /// One frame of the real UI with these events; every text drawn and where.
    fn frame(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        events: Vec<egui::Event>,
    ) -> Vec<(String, Rect)> {
        use eframe::App as _;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            time: Some(*t),
            events,
            ..Default::default()
        };
        *t += 0.05;
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(s) => Some((
                    s.galley.text().to_owned(),
                    s.galley.rect.translate(s.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    /// A left click: move, press, release, and a frame after.
    fn click(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        at: egui::Pos2,
    ) -> Vec<(String, Rect)> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(app, ctx, t, vec![egui::Event::PointerMoved(at)]);
        frame(app, ctx, t, vec![button(true)]);
        frame(app, ctx, t, vec![button(false)]);
        frame(app, ctx, t, vec![]);
        frame(app, ctx, t, vec![])
    }

    /// Clicks the text drawn last frame (a menu item or a dialog button).
    fn click_text(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        shown: &[(String, Rect)],
        text: &str,
    ) -> Vec<(String, Rect)> {
        let at = shown
            .iter()
            .rev()
            .find(|(s, _)| s == text)
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("\"{text}\" is not drawn"));
        click(app, ctx, t, at)
    }

    /// Clicks the ribbon button of a tool id, where the ribbon last drew it.
    fn click_tool(
        app: &mut ChamferaApp,
        ctx: &egui::Context,
        t: &mut f64,
        id: &str,
    ) -> Vec<(String, Rect)> {
        let at = ctx
            .data(|d| d.get_temp::<(u64, Rect)>(rect_key(id)))
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("the ribbon drew no {id}"));
        click(app, ctx, t, at)
    }

    fn step(app: &ChamferaApp) -> usize {
        app.tutorial.running.as_ref().map_or(usize::MAX, |r| r.step)
    }

    #[test]
    fn tutorial_one_completes_with_real_pointer_and_key_events() {
        let ctx = egui::Context::default();
        let mut app = ChamferaApp::headless();
        let mut t = 0.0;
        for _ in 0..3 {
            frame(&mut app, &ctx, &mut t, vec![]);
        }
        assert!(app.start_tutorial("first-part"));
        frame(&mut app, &ctx, &mut t, vec![]);
        let shown = frame(&mut app, &ctx, &mut t, vec![]);
        assert!(shown.iter().any(|(s, _)| s == "STEP 1 OF 6"), "{shown:?}");
        // 1. Create Sketch, then the top plane in its list.
        let shown = click_tool(&mut app, &ctx, &mut t, "sketch.create");
        click_text(&mut app, &ctx, &mut t, &shown, "Top plane (XY)");
        assert_eq!(step(&app), 1, "the sketch step is done");
        // 2. The Rectangle tool by its key, R, then two corners in the viewport.
        let key = |pressed| egui::Event::Key {
            key: egui::Key::R,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, &ctx, &mut t, vec![key(true)]);
        frame(&mut app, &ctx, &mut t, vec![key(false)]);
        assert_eq!(app.sketch.tool(), crate::sketch_ui::Tool::Rectangle);
        let c = app.viewport_rect.center();
        click(&mut app, &ctx, &mut t, c + egui::vec2(-150.0, -100.0));
        click(&mut app, &ctx, &mut t, c + egui::vec2(150.0, 100.0));
        assert_eq!(step(&app), 2, "the rectangle step is done");
        // 3. Finish Sketch.
        click_tool(&mut app, &ctx, &mut t, "sketch.finish");
        assert_eq!(step(&app), 3);
        // 4. Extrude: the profile under the centre, then OK.
        click_tool(&mut app, &ctx, &mut t, "solid.extrude");
        let shown = click(&mut app, &ctx, &mut t, c);
        click_text(&mut app, &ctx, &mut t, &shown, "OK");
        assert_eq!(step(&app), 4, "extruded: {:?}", app.message);
        // 5. Fillet: the block's top edge along the rectangle's upper side, then OK.
        click_tool(&mut app, &ctx, &mut t, "solid.fillet");
        let shown = click(&mut app, &ctx, &mut t, c + egui::vec2(0.0, -100.0));
        click_text(&mut app, &ctx, &mut t, &shown, "OK");
        assert_eq!(step(&app), 5, "filleted: {:?}", app.message);
        // 6. Hole: a point on the top face, then OK.
        click_tool(&mut app, &ctx, &mut t, "solid.hole");
        let shown = click(&mut app, &ctx, &mut t, c + egui::vec2(40.0, 20.0));
        let shown = click_text(&mut app, &ctx, &mut t, &shown, "OK");
        assert_eq!(step(&app), 6, "drilled: {:?}", app.message);
        assert!(app.tutorial.running.as_ref().is_some_and(Running::finished));
        assert!(shown.iter().any(|(s, _)| s == "TUTORIAL COMPLETE"));
        // The timeline holds what the tour made: sketch, extrude, fillet and hole.
        let features = app.session.doc.features.len();
        assert_eq!(features, 3, "extrude, fillet and hole");
        assert_eq!(app.session.solids().len(), 1);
    }

    #[test]
    fn the_drawing_tour_opens_its_sample_and_the_drawing_workspace() {
        let mut app = ChamferaApp::headless();
        assert!(app.start_tutorial("first-drawing"));
        assert!(!app.session.solids().is_empty(), "the sample was opened");
        assert_eq!(app.workspace(), Workspace::Drawing);
    }
}

//! 2D drafting on the sheet canvas: drafted entities painted by layer, selection, grips,
//! and the command line at the canvas's foot.
//!
//! Entities are painted in their layer's colour, weight and line type (dashes worked out
//! in sheet mm by `chamfera_drawing::linetype`, then mapped to the screen), entities on a
//! locked layer faded. A click with no tool selects an entity (Shift adds); the selected
//! ones show grips. Dragging a grip runs `drawing.entity.grip` once at release; dragging
//! the entity itself runs `drawing.entity.move`. Delete runs `drawing.entity.delete`.
//! The command line (`command_line.rs`) takes the keyboard when you start typing on the
//! canvas, and its points can be clicked too.

use crate::command_line::{Act, CommandLine, Verb};
use crate::drawing_ui::Mapping;
use crate::widgets::{Look, T_SMALL};
use crate::{ChamferaApp, color};
use chamfera_drawing::drafting::{self as dr, DraftStyle};
use chamfera_drawing::linetype;
use chamfera_drawing::{Shape, SheetGeometry};
use chamfera_geom::Point2;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use serde_json::{Value, json};

/// How near (screen points) the pointer must be to a grip to take it.
const GRIP_PX: f32 = 7.0;
/// The grip squares' half size, screen points.
const GRIP_HALF: f32 = 4.5;
/// How near (sheet mm) a click must be to an entity to select it.
const PICK_MM: f64 = 1.5;
/// The command line's id, for focus.
const LINE_ID: &str = "drafting command line";

/// The drafting state of the DRAWING workspace.
#[derive(Default)]
pub(crate) struct DraftingUi {
    /// Selected entities, by id.
    pub selected: Vec<u32>,
    /// A grip being dragged: (entity, grip index, where the pointer is in sheet mm).
    pub grip_drag: Option<(u32, usize, Point2)>,
    /// Selected entities being dragged: from, to (sheet mm).
    pub move_drag: Option<(Point2, Point2)>,
    pub cmd: CommandLine,
    /// The pointer on the sheet, for previews.
    pub hover: Option<Point2>,
    /// Shift was held on the last frame (Shift+click adds to the selection).
    pub shift: bool,
    /// The Layers panel (`layer_ui.rs`).
    pub layers: crate::layer_ui::LayerPanel,
}

impl DraftingUi {
    /// Esc: drops the pending command, the drags and the selection.
    pub fn escape(&mut self) {
        self.cmd.cancel();
        self.grip_drag = None;
        self.move_drag = None;
        self.selected.clear();
    }
}

/// The colour a layer draws in on screen: its own, or the sheet's ink for none.
fn style_colour(app: &ChamferaApp, style: &DraftStyle) -> Color32 {
    let c = match style.colour {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => color(app.theme.sheet_line),
    };
    if style.locked {
        c.gamma_multiply(0.45)
    } else {
        c
    }
}

/// A shape's outline as sheet polylines (circles and arcs sampled at about 1.5 screen
/// points).
fn outline(shape: &Shape, zoom: f32) -> Vec<Point2> {
    let step = 1.5 / f64::from(zoom.max(0.01));
    match shape {
        Shape::Line { a, b } => vec![*a, *b],
        Shape::Circle { centre, radius } => {
            linetype::arc_points(*centre, *radius, 0.0, std::f64::consts::TAU, step)
        }
        Shape::Arc {
            centre,
            radius,
            start,
            end,
        } => linetype::arc_points(*centre, *radius, *start, *end, step),
        _ => Vec::new(),
    }
}

/// Paints `shape` with `pattern` (sheet mm; dashes too small to see are drawn whole).
fn paint_shape(
    painter: &egui::Painter,
    m: &Mapping,
    shape: &Shape,
    pattern: &[f64],
    stroke: Stroke,
) {
    let pts = outline(shape, m.zoom);
    let period: f64 = pattern.iter().sum();
    let visible = period * f64::from(m.zoom) >= 4.0;
    let pieces = if visible {
        linetype::dash_polyline(&pts, pattern)
    } else {
        linetype::dash_polyline(&pts, &[])
    };
    for [a, b] in pieces {
        painter.line_segment([m.to_screen(a), m.to_screen(b)], stroke);
    }
}

fn grip_rect(at: Pos2) -> Rect {
    Rect::from_center_size(at, Vec2::splat(GRIP_HALF * 2.0))
}

impl ChamferaApp {
    fn drafting_geometry(&mut self) -> Option<SheetGeometry> {
        self.drawing_geometry().cloned()
    }

    /// The entity `id` of the shown sheet, with its layer's lock.
    fn drafting_entity(&self, id: u32) -> Option<(chamfera_doc::drafting::EntityKind, bool)> {
        let sheet = self.drawing_sheet()?;
        let d = &self.session.doc.drawing.sheet(sheet).ok()?.drafting;
        let e = d.entity(id)?;
        let locked = d.layer(&e.layer).is_some_and(|l| l.locked);
        Some((e.kind.clone(), locked))
    }

    /// Runs what the command line asked for.
    pub(crate) fn drafting_acts(&mut self, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Run(id, mut params) => {
                    let takes_sheet = id.starts_with("drawing.draw.")
                        || id.starts_with("drawing.layer.")
                        || id == "drawing.linetype_scale";
                    if takes_sheet
                        && params.get("sheet").is_none()
                        && let (Some(s), Value::Object(map)) = (self.drawing_sheet(), &mut params)
                    {
                        map.insert("sheet".into(), json!(s));
                    }
                    if self.run(&id, params).is_none() {
                        let why = self
                            .message
                            .as_ref()
                            .map(|m| m.0.clone())
                            .unwrap_or_default();
                        self.drawing.drafting.cmd.history.push(format!("! {why}"));
                        // A refused step ends the verb, so the next input starts afresh.
                        self.drawing.drafting.cmd.pending = None;
                    }
                }
                Act::MoveSelection { dx, dy, copy } => {
                    let ids = self.drawing.drafting.selected.clone();
                    if let Some(r) = self.run(
                        "drawing.entity.move",
                        json!({ "entities": ids, "dx": dx, "dy": dy, "copy": copy }),
                    ) && copy
                    {
                        self.drawing.drafting.selected = r["entities"]
                            .as_array()
                            .map(|v| {
                                v.iter()
                                    .filter_map(|x| x.as_u64().and_then(|x| u32::try_from(x).ok()))
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                }
                Act::EraseSelection => {
                    self.drafting_delete_selected();
                }
                Act::SelectAll => {
                    let all: Vec<u32> = self
                        .drawing_sheet()
                        .and_then(|s| self.session.doc.drawing.sheet(s).ok())
                        .map(|s| {
                            s.drafting
                                .entities
                                .iter()
                                .filter(|e| {
                                    s.drafting
                                        .layer(&e.layer)
                                        .is_some_and(|l| l.visible() && !l.locked)
                                })
                                .map(|e| e.id)
                                .collect()
                        })
                        .unwrap_or_default();
                    self.drawing.selected = None;
                    self.drawing.drafting.selected = all;
                }
                Act::SelectNone => self.drawing.drafting.selected.clear(),
                Act::Select(ids) => {
                    self.drawing.selected = None;
                    self.drawing.drafting.selected = ids
                        .into_iter()
                        .filter(|id| self.drafting_entity(*id).is_some_and(|(_, l)| !l))
                        .collect();
                }
                Act::ToggleLayers => self.toggle_layer_panel(),
                Act::Note(_) => {}
                Act::Fail(e) => self.message = Some((e, true)),
            }
        }
    }

    /// Types a whole line into the command line and runs it (tests and `--command-line`).
    pub(crate) fn drafting_submit(&mut self, text: &str) {
        let has = !self.drawing.drafting.selected.is_empty();
        let acts = self.drawing.drafting.cmd.submit(text, has);
        self.drafting_acts(acts);
    }

    /// A ribbon drawing tool (`drawing.draw.<what>`): starts that verb, or ends it when it
    /// is the one running.
    pub(crate) fn drafting_tool(&mut self, what: &str) {
        let Some(verb) = Verb::of_tool(what) else {
            return;
        };
        if self.drafting_tool_active(what) {
            self.drawing.drafting.cmd.cancel();
            return;
        }
        self.drawing.tool = None;
        let has = !self.drawing.drafting.selected.is_empty();
        let acts = self.drawing.drafting.cmd.start(verb, has);
        self.drafting_acts(acts);
    }

    /// True while the verb of ribbon tool `drawing.draw.<what>` is pending.
    pub(crate) fn drafting_tool_active(&self, what: &str) -> bool {
        let pending = self.drawing.drafting.cmd.pending.as_ref().map(|p| p.verb);
        pending.is_some() && pending == Verb::of_tool(what)
    }

    /// A click on the sheet while the command line waits for a point.
    pub(crate) fn drafting_click(&mut self, p: Point2) -> bool {
        if !self.drawing.drafting.cmd.wants_point() {
            return false;
        }
        let acts = self.drawing.drafting.cmd.click(p);
        self.drafting_acts(acts);
        true
    }

    /// A click with no tool: selects the entity under `p` (Shift toggles it in the
    /// selection). False when there is none there, so the sheet's own items are tried.
    pub(crate) fn drafting_select_at(&mut self, p: Point2) -> bool {
        let mut params = json!({ "x": p.x, "y": p.y, "tolerance": PICK_MM });
        if let Some(s) = self.drawing_sheet() {
            params["sheet"] = json!(s);
        }
        let hit = self
            .session
            .execute("drawing.entity.pick", &params)
            .ok()
            .and_then(|r| r["entity"].as_u64())
            .and_then(|v| u32::try_from(v).ok());
        let d = &mut self.drawing.drafting;
        match hit {
            Some(id) => {
                if d.shift {
                    if let Some(i) = d.selected.iter().position(|x| *x == id) {
                        d.selected.remove(i);
                    } else {
                        d.selected.push(id);
                    }
                } else {
                    d.selected = vec![id];
                }
                self.drawing.selected = None;
                true
            }
            None => {
                if !d.shift {
                    d.selected.clear();
                }
                false
            }
        }
    }

    /// Delete with entities selected: removes them. False when none are selected.
    pub(crate) fn drafting_delete_selected(&mut self) -> bool {
        let ids = std::mem::take(&mut self.drawing.drafting.selected);
        if ids.is_empty() {
            return false;
        }
        self.run("drawing.entity.delete", json!({ "entities": ids }));
        true
    }

    /// The selected entity and grip under screen point `at`, if any.
    fn grip_under(&self, m: &Mapping, at: Pos2) -> Option<(u32, usize)> {
        let mut best: Option<(f32, u32, usize)> = None;
        for id in &self.drawing.drafting.selected {
            let Some((kind, false)) = self.drafting_entity(*id) else {
                continue;
            };
            for (i, g) in dr::grips(&kind).iter().enumerate() {
                let d = m.to_screen(g.at).distance(at);
                if d <= GRIP_PX && best.is_none_or(|b| d < b.0) {
                    best = Some((d, *id, i));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// Drags with no tool on drafted entities: a grip of a selected entity, or the
    /// entities themselves. True when the drag is drafting's (the sheet's own selection
    /// and drag then stay out of it).
    pub(crate) fn drafting_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        m: &Mapping,
    ) -> bool {
        if self.drawing.drafting.cmd.wants_point() {
            return true;
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin())
        {
            let p = m.to_sheet(o);
            if let Some((id, grip)) = self.grip_under(m, o) {
                self.drawing.drafting.grip_drag = Some((id, grip, p));
            } else {
                let already = self.drawing.drafting.selected.clone();
                let shift = self.drawing.drafting.shift;
                self.drawing.drafting.shift = false;
                let hit = self.drafting_select_at(p);
                self.drawing.drafting.shift = shift;
                if hit {
                    // Dragging an entity that was in a larger selection moves them all.
                    if let Some(id) = self.drawing.drafting.selected.first().copied()
                        && already.contains(&id)
                    {
                        self.drawing.drafting.selected = already;
                    }
                    self.drawing.drafting.move_drag = Some((p, p));
                }
            }
        }
        let busy =
            self.drawing.drafting.grip_drag.is_some() || self.drawing.drafting.move_drag.is_some();
        if !busy {
            return false;
        }
        if let Some(h) = response.interact_pointer_pos() {
            let q = m.to_sheet(h);
            if let Some(g) = self.drawing.drafting.grip_drag.as_mut() {
                g.2 = q;
            }
            if let Some(d) = self.drawing.drafting.move_drag.as_mut() {
                d.1 = q;
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            || !ui.input(|i| i.pointer.primary_down())
        {
            if let Some((id, grip, to)) = self.drawing.drafting.grip_drag.take() {
                self.run(
                    "drawing.entity.grip",
                    json!({ "entity": id, "grip": grip, "x": to.x, "y": to.y }),
                );
            }
            if let Some((from, to)) = self.drawing.drafting.move_drag.take() {
                let (dx, dy) = (to.x - from.x, to.y - from.y);
                if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
                    let ids = self.drawing.drafting.selected.clone();
                    self.run(
                        "drawing.entity.move",
                        json!({ "entities": ids, "dx": dx, "dy": dy }),
                    );
                }
            }
        }
        true
    }

    /// Paints the drafted entities, the selection with its grips, a drag being made and
    /// the pending command's preview.
    pub(crate) fn drafting_paint(&mut self, painter: &egui::Painter, m: &Mapping) {
        let Some(g) = self.drafting_geometry() else {
            return;
        };
        let pick = color(self.theme.sheet_pick);
        let zoom = m.zoom;
        let selected = self.drawing.drafting.selected.clone();
        let moving = self.drawing.drafting.move_drag.map(|(a, b)| b - a);
        for s in &g.drafted {
            let Some(style) = g.draft_layers.get(s.layer) else {
                continue;
            };
            let width = (style.weight as f32 * zoom).max(1.0);
            let is_selected = selected.contains(&s.entity);
            if is_selected {
                // A halo under the selected entity, in the pick colour.
                paint_shape(
                    painter,
                    m,
                    &s.shape,
                    &[],
                    Stroke::new(width + 3.0, pick.gamma_multiply(0.45)),
                );
            }
            let stroke = Stroke::new(width, style_colour(self, style));
            paint_shape(painter, m, &s.shape, &style.pattern, stroke);
            if is_selected && let Some(d) = moving {
                let shifted = match &s.shape {
                    Shape::Line { a, b } => Shape::Line {
                        a: *a + d,
                        b: *b + d,
                    },
                    Shape::Circle { centre, radius } => Shape::Circle {
                        centre: *centre + d,
                        radius: *radius,
                    },
                    Shape::Arc {
                        centre,
                        radius,
                        start,
                        end,
                    } => Shape::Arc {
                        centre: *centre + d,
                        radius: *radius,
                        start: *start,
                        end: *end,
                    },
                    other => other.clone(),
                };
                paint_shape(painter, m, &shifted, &[], Stroke::new(1.5, pick));
            }
        }
        // Grips of the selected entities, and a grip being dragged with its result.
        let accent = color(self.theme.accent);
        let edge = color(self.theme.sheet_paper);
        let hot = self.drawing.drafting.grip_drag;
        let hover_grip = self
            .drawing
            .drafting
            .hover
            .map(|h| m.to_screen(h))
            .and_then(|h| self.grip_under(m, h));
        for id in &selected {
            let Some((kind, locked)) = self.drafting_entity(*id) else {
                continue;
            };
            if locked {
                continue;
            }
            for (i, grip) in dr::grips(&kind).iter().enumerate() {
                let is_hot =
                    hot.is_some_and(|h| h.0 == *id && h.1 == i) || hover_grip == Some((*id, i));
                let r = grip_rect(m.to_screen(grip.at));
                painter.rect_filled(r, 1.0, if is_hot { pick } else { accent });
                painter.rect_stroke(r, 1.0, Stroke::new(1.0, edge), egui::StrokeKind::Outside);
            }
            if let Some((hid, grip, to)) = hot
                && hid == *id
                && let Ok(next) = dr::drag_grip(&kind, grip, to)
            {
                for shape in dr::shapes_of(&next) {
                    paint_shape(painter, m, &shape, &[], Stroke::new(1.5, pick));
                }
            }
        }
        // The pending command's rubber band and the pointer's cross.
        if let Some(h) = self.drawing.drafting.hover
            && self.drawing.drafting.cmd.wants_point()
        {
            let stroke = Stroke::new(1.2, pick);
            for shape in self.drawing.drafting.cmd.preview(h) {
                paint_shape(
                    painter,
                    m,
                    &shape,
                    &[
                        2.0 / f64::from(zoom.max(0.01)) * 2.0,
                        2.0 / f64::from(zoom.max(0.01)),
                    ],
                    stroke,
                );
            }
            let s = m.to_screen(h);
            let cross = Stroke::new(1.0, pick);
            painter.line_segment([s - Vec2::new(12.0, 0.0), s + Vec2::new(12.0, 0.0)], cross);
            painter.line_segment([s - Vec2::new(0.0, 12.0), s + Vec2::new(0.0, 12.0)], cross);
            painter.rect_stroke(grip_rect(s), 0.0, cross, egui::StrokeKind::Middle);
            painter.text(
                s + Vec2::new(14.0, 14.0),
                egui::Align2::LEFT_TOP,
                format!("{:.2}, {:.2}", h.x, h.y),
                egui::FontId::proportional(T_SMALL),
                pick,
            );
        }
    }

    /// Starting to type on the canvas (no text box has the keyboard) goes to the
    /// command line. Call before the command line is shown.
    pub(crate) fn drafting_type_anywhere(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || self.search.open {
            return;
        }
        let mut typed = String::new();
        ctx.input_mut(|i| {
            if i.modifiers.ctrl || i.modifiers.command || i.modifiers.alt {
                return;
            }
            i.events.retain(|e| match e {
                egui::Event::Text(t) if !t.trim().is_empty() || !typed.is_empty() => {
                    typed.push_str(t);
                    false
                }
                _ => true,
            });
        });
        if !typed.is_empty() {
            let line = &mut self.drawing.drafting.cmd;
            for c in typed.chars() {
                if line.input.chars().count() < crate::command_line::MAX_INPUT {
                    line.input.push(c);
                }
            }
            line.focus = true;
        }
    }

    /// The command line at the canvas's foot: recent history, the prompt and the input.
    pub(crate) fn drafting_command_line(&mut self, ctx: &egui::Context, rect: Rect) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let width = (rect.width() - 24.0).clamp(200.0, 760.0);
        let id = egui::Id::unique(LINE_ID);
        let mut submit: Option<String> = None;
        let mut cancel = false;
        let prompt = self.drawing.drafting.cmd.prompt();
        let history: Vec<String> = self
            .drawing
            .drafting
            .cmd
            .history
            .iter()
            .rev()
            .take(3)
            .rev()
            .cloned()
            .collect();
        let focus = std::mem::take(&mut self.drawing.drafting.cmd.focus);
        let mut input = std::mem::take(&mut self.drawing.drafting.cmd.input);
        egui::Area::new(egui::Id::unique("drafting command line area"))
            .fixed_pos(egui::pos2(rect.min.x + 12.0, rect.max.y - 12.0))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(look.c(|t| t.panel))
                    .stroke(Stroke::new(1.0, look.c(|t| t.border)))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        for h in &history {
                            let c = if h.starts_with('!') {
                                look.c(|t| t.error)
                            } else {
                                look.c(|t| t.text_muted)
                            };
                            ui.label(egui::RichText::new(h).size(T_SMALL).color(c));
                        }
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{prompt}:"))
                                    .color(look.c(|t| t.accent))
                                    .strong(),
                            );
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut input)
                                    .id(id)
                                    .char_limit(crate::command_line::MAX_INPUT)
                                    .desired_width(ui.available_width())
                                    .hint_text("Type a command: L, C, A, PL, REC, M, CP, E, LA, LN, LTS, ? for all"),
                            );
                            if focus {
                                r.request_focus();
                                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                                    let end = egui::text::CCursor::new(input.chars().count());
                                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                                    state.store(ui.ctx(), id);
                                }
                            }
                            let (enter, escape) = ui.input(|i| {
                                (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
                            });
                            if r.lost_focus() && enter {
                                submit = Some(std::mem::take(&mut input));
                            } else if (r.lost_focus() || r.has_focus()) && escape {
                                cancel = true;
                                input.clear();
                            }
                        });
                    });
            });
        self.drawing.drafting.cmd.input = input;
        if cancel {
            self.drawing.drafting.escape();
        }
        if let Some(text) = submit {
            self.drafting_submit(&text);
            self.drawing.drafting.cmd.focus = self.drawing.drafting.cmd.pending.is_some();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_doc::drafting::EntityKind;

    fn app_with_sheet() -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        app.session
            .execute("drawing.new_sheet", &json!({}))
            .unwrap();
        app.drawing.active = true;
        app
    }

    fn entities(app: &ChamferaApp) -> Vec<chamfera_doc::drafting::Entity> {
        app.session.doc.drawing.sheets[0].drafting.entities.clone()
    }

    #[test]
    fn typed_commands_and_clicks_draw_on_the_current_layer() {
        let mut app = app_with_sheet();
        app.drafting_submit("LN Axes center #ff0000");
        app.drafting_submit("L 10,10 100,10");
        let e = entities(&app);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].layer, "Axes");
        assert_eq!(
            e[0].kind,
            EntityKind::Line {
                a: Point2::new(10.0, 10.0),
                b: Point2::new(100.0, 10.0)
            }
        );
        // The ribbon's Rectangle tool, then two clicks on the sheet.
        app.drafting_tool("rectangle");
        assert!(app.drafting_tool_active("rectangle"));
        app.drawing_click(Point2::new(20.0, 20.0));
        app.drawing_click(Point2::new(60.0, 50.0));
        assert!(!app.drafting_tool_active("rectangle"));
        let e = entities(&app);
        assert_eq!(e.len(), 2);
        assert!(
            matches!(&e[1].kind, EntityKind::Polyline { points, closed: true } if points.len() == 4)
        );
        // A refused step (a radius off the sheet) shows the reason and draws nothing.
        app.drafting_submit("C 50,50 99999");
        assert_eq!(entities(&app).len(), 2);
        assert!(app.message.as_ref().is_some_and(|m| m.1));
    }

    #[test]
    fn a_click_selects_an_entity_and_delete_removes_it() {
        let mut app = app_with_sheet();
        app.drafting_submit("L 10,10 100,10");
        app.drafting_submit(""); // Enter ends the line
        app.drafting_submit("C 200,150 20");
        let ids: Vec<u32> = entities(&app).iter().map(|e| e.id).collect();
        // 0.4 mm above the line: picked; Shift adds the circle (its right quadrant).
        app.drawing_click(Point2::new(50.0, 10.4));
        assert_eq!(app.drawing.drafting.selected, vec![ids[0]]);
        app.drawing.drafting.shift = true;
        app.drawing_click(Point2::new(220.3, 150.0));
        app.drawing.drafting.shift = false;
        assert_eq!(app.drawing.drafting.selected, ids);
        // Move by the command line: base point then second point.
        app.drafting_submit("M 0,0 @5,-2");
        let e = entities(&app);
        assert_eq!(
            e[0].kind,
            EntityKind::Line {
                a: Point2::new(15.0, 8.0),
                b: Point2::new(105.0, 8.0)
            }
        );
        app.drawing_delete_selected();
        assert!(entities(&app).is_empty());
        // Locked layers cannot be picked; SEL ALL skips them too.
        app.drafting_submit("L 10,10 100,10");
        app.drafting_submit("");
        app.session
            .execute("drawing.layer.edit", &json!({"layer": "0", "locked": true}))
            .unwrap();
        app.drawing_click(Point2::new(50.0, 10.0));
        assert!(app.drawing.drafting.selected.is_empty());
        app.drafting_submit("SEL ALL");
        assert!(app.drawing.drafting.selected.is_empty());
        // LA opens the Layers panel; Esc clears everything pending.
        app.drafting_submit("LA");
        assert!(app.drawing.drafting.layers.open);
        app.drafting_submit("PL 0,0 10,0");
        app.drawing.drafting.escape();
        assert!(app.drawing.drafting.cmd.pending.is_none());
    }
}

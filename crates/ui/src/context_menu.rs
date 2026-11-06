//! Right-click menus, the selection filter and hover pre-highlight in the viewport.
//!
//! A right-button click (no drag: a right drag still orbits) opens a menu for what is
//! under the cursor: a body, a face, an edge, empty space, or the sketch being edited.
//! Every item runs a command or the same UI action as its ribbon button (`run_action`);
//! items that cannot run now are shown disabled. The Browser reuses the body items.
//!
//! The selection filter (bottom-left of the viewport) says what clicks may pick; it is
//! passed to `select.pick` and `sketch.select` as their `filter` parameter. Hover asks
//! `Session::pick_filtered` only when the pointer or the view moved.

use crate::icons::Icon;
use crate::shortcuts::{Action, action_label};
use crate::sketch_ui::Tool;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_engine::pick_filter::{PickFilter, Picked};
use chamfera_engine::sketching::SketchItem;
use chamfera_geom::Point3;
use chamfera_render::{Camera, Overlay, OverlayStyle};
use egui::{Pos2, Rect, vec2};
use serde_json::json;

/// A viewport pixel and the viewport's size: `(x, y, width, height)`.
pub type Px = (f64, f64, f64, f64);

/// What a menu was opened on.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// The sketch being edited.
    Sketch,
    /// Nothing under the cursor.
    Empty,
    /// A true edge of a solid, by name.
    Edge(String),
    /// A body; `face` is the solid's face under the cursor when faces pass the filter, and
    /// `body_items` is false when bodies do not.
    Body {
        body: u32,
        face: Option<String>,
        body_items: bool,
    },
}

/// What renaming changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameTarget {
    Feature(u32),
    Sketch(u32),
    Body(u32),
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// The same action as a ribbon button, key or palette row.
    Run(Action),
    Visibility(u32, bool),
    Isolate(u32),
    ShowAll,
    /// Selects the body under this pixel.
    SelectAt(Px),
    EditFeature(u32),
    Suppress(u32, bool),
    DeleteFeature(u32),
    RemoveBody(u32),
    Properties(u32),
    SketchOnFace(Px),
    /// Opens Press Pull with what is under this pixel picked.
    PressPullAt(Px),
    /// Opens Measure, with what is under the pixel (when there is one) as its first pick.
    Measure(Option<Px>),
    /// Opens Fillet (or Chamfer) with this edge picked.
    Blend {
        chamfer: bool,
        edge: String,
    },
    Constrain(&'static str),
    Rename(RenameTarget),
    EditSketch(u32),
    DeleteSketch(u32),
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Item {
        text: String,
        icon: Icon,
        /// Why it cannot run now; `None` when it can.
        why_not: Option<String>,
        act: Act,
    },
    Caption(String),
    Separator,
}

fn item(text: impl Into<String>, icon: Icon, why_not: Option<String>, act: Act) -> Entry {
    Entry::Item {
        text: text.into(),
        icon,
        why_not,
        act,
    }
}

/// An open viewport menu.
pub struct Menu {
    /// Screen position of its top-left corner.
    pub pos: Pos2,
    pub px: Px,
    pub target: Target,
    /// Opened this frame: the click that opened it does not close it.
    fresh: bool,
    /// Each item's text and screen rectangle as last drawn (tests click them).
    pub shown: Vec<(String, Rect)>,
}

/// The Properties window's contents, measured once when it opens.
pub struct Properties {
    title: String,
    rows: Vec<(String, String)>,
}

/// What the hover pre-highlight was worked out for, so it is redone only on a change.
#[derive(PartialEq)]
struct HoverKey {
    px: Px,
    camera: Camera,
    bodies: Vec<u32>,
    filter: PickFilter,
}

#[derive(Default)]
pub struct ContextMenuUi {
    /// What clicks may pick.
    pub filter: PickFilter,
    pub menu: Option<Menu>,
    /// `--context-menu X,Y`: open the menu at this viewport point once the layout settled.
    pub open_at: Option<[f32; 2]>,
    hover_key: Option<HoverKey>,
    /// What the pointer is over, and its lines (drawn in `hover_highlight`).
    pub hover: Option<(Picked, Vec<[Point3; 2]>)>,
    /// The last solid tool opened, for Repeat.
    last_action: Option<Action>,
    /// The last sketch drawing tool, for Repeat in a sketch.
    last_tool: Option<Tool>,
    rename: Option<(RenameTarget, String, bool)>,
    properties: Option<Properties>,
}

/// Whether the sketch selection suits a constraint, by how `sketch.constrain_selection`
/// reads it: `points`, `lines`, and `others` (arcs and circles).
fn constraint_fits(kind: &str, points: usize, lines: usize, others: usize) -> bool {
    let curves = lines + others;
    match kind {
        "horizontal" | "vertical" => points == 0 && lines >= 1 && others == 0,
        "coincident" => points == 2 && curves == 0,
        "point_on" => points == 1 && curves == 1,
        "midpoint" => points == 1 && lines == 1 && others == 0,
        "parallel" | "perpendicular" => points == 0 && lines == 2 && others == 0,
        "tangent" => points == 0 && curves == 2 && others >= 1,
        "equal" => points == 0 && curves == 2,
        "fix" => points >= 1 && curves == 0,
        _ => false,
    }
}

/// Draws menu rows; returns the clicked item's act and records where each item went.
pub fn show_entries(
    ui: &mut egui::Ui,
    look: Look,
    entries: &[Entry],
    shown: &mut Vec<(String, Rect)>,
) -> Option<Act> {
    let mut chosen = None;
    shown.clear();
    for e in entries {
        match e {
            Entry::Separator => {
                ui.separator();
            }
            Entry::Caption(text) => {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    widgets::caption(ui, look, text);
                });
            }
            Entry::Item {
                text,
                icon,
                why_not,
                act,
            } => {
                let r = widgets::menu_item(ui, look, *icon, text, why_not.is_none());
                shown.push((text.clone(), r.rect));
                let r = match why_not {
                    Some(why) => r.on_disabled_hover_text(why.as_str()),
                    None => r,
                };
                if r.clicked() && why_not.is_none() {
                    chosen = Some(act.clone());
                }
            }
        }
    }
    chosen
}

impl ChamferaApp {
    /// Remembers the last solid tool and sketch tool opened (from any button, key or
    /// menu), for the Repeat items. Called once a frame.
    pub(crate) fn track_last_used(&mut self) {
        if let Some(t) = &self.solid.extrude {
            self.ctx_menu.last_action = Some(if t.revolve {
                Action::Revolve
            } else {
                Action::Extrude
            });
        }
        if let Some(t) = &self.solid.blend {
            self.ctx_menu.last_action = Some(if t.chamfer {
                Action::Chamfer
            } else {
                Action::Fillet
            });
        }
        if self.solid.pick_face {
            self.ctx_menu.last_action = Some(Action::SketchOnFace);
        }
        if let Some(t) = self.sketch.tool
            && t != Tool::Select
        {
            self.ctx_menu.last_tool = Some(t);
        }
    }

    /// The selection filter's kinds as `select.pick` and `sketch.select` take them.
    pub(crate) fn filter_param(&self) -> serde_json::Value {
        json!(self.ctx_menu.filter.names())
    }

    /// What a menu opened at this pixel is for.
    pub(crate) fn target_at(&self, (x, y, w, h): Px) -> Target {
        if self.session.active_sketch.is_some() {
            return Target::Sketch;
        }
        let filter = self.ctx_menu.filter;
        match self.session.pick_filtered(x, y, w, h, &filter) {
            Picked::Edge { name, .. } => Target::Edge(name),
            Picked::Face { hit, name } => Target::Body {
                body: hit.body,
                face: Some(name),
                body_items: filter.bodies,
            },
            Picked::Body(hit) => Target::Body {
                body: hit.body,
                face: None,
                body_items: true,
            },
            Picked::Nothing => Target::Empty,
        }
    }

    /// Opens the viewport menu for what is under screen point `pos`.
    pub(crate) fn open_context_menu(&mut self, pos: Pos2, rect: Rect) {
        let px = (
            f64::from(pos.x - rect.min.x),
            f64::from(pos.y - rect.min.y),
            f64::from(rect.width()),
            f64::from(rect.height()),
        );
        let target = self.target_at(px);
        self.ctx_menu.menu = Some(Menu {
            pos,
            px,
            target,
            fresh: true,
            shown: Vec::new(),
        });
    }

    /// A right click (not a drag) in the viewport opens the menu. Called from `navigate`.
    pub(crate) fn context_click(&mut self, response: &egui::Response, rect: Rect) {
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.open_context_menu(pos, rect);
        }
    }

    fn why(&self, action: Action) -> Option<String> {
        self.unavailable(action)
    }

    fn run_item(&self, action: Action) -> Entry {
        item(
            action_label(action),
            crate::search::icon_of(action),
            self.why(action),
            Act::Run(action),
        )
    }

    /// The solid that a scene body shows, if it is one.
    fn solid_of(&self, body: u32) -> Option<&chamfera_engine::solids::Solid> {
        self.session
            .solids()
            .iter()
            .find(|s| s.scene_id == Some(body))
    }

    /// A body's items: in the viewport menu and the Browser.
    pub(crate) fn body_entries(&self, body: u32, at: Option<Px>, measure: bool) -> Vec<Entry> {
        let Some(b) = self.session.scene.body(body) else {
            return Vec::new();
        };
        let mut out = vec![Entry::Caption(b.name.to_uppercase())];
        let visible = b.visible;
        out.push(item(
            if visible { "Hide" } else { "Show" },
            if visible { Icon::EyeOff } else { Icon::Eye },
            None,
            Act::Visibility(body, !visible),
        ));
        let others = self.session.scene.bodies.iter().any(|o| o.id != body);
        out.push(item(
            "Isolate",
            Icon::Eye,
            (!others).then(|| "It is the only body".to_owned()),
            Act::Isolate(body),
        ));
        if let Some(px) = at {
            out.push(item("Select Body", Icon::Select, None, Act::SelectAt(px)));
        }
        let sketching = self.session.active_sketch.is_some();
        let busy = sketching.then(|| "Finish the sketch first".to_owned());
        out.push(Entry::Separator);
        match self.solid_of(body).map(|s| s.feature) {
            Some(feature) => {
                let suppressed = self
                    .session
                    .doc
                    .feature(feature)
                    .is_ok_and(|f| self.session.doc.is_suppressed(f));
                out.push(item(
                    label("feature.edit"),
                    Icon::Edit,
                    busy.clone(),
                    Act::EditFeature(feature),
                ));
                out.push(item(
                    label("feature.rename"),
                    Icon::Edit,
                    None,
                    Act::Rename(RenameTarget::Feature(feature)),
                ));
                out.push(item(
                    if suppressed { "Unsuppress" } else { "Suppress" },
                    Icon::EyeOff,
                    None,
                    Act::Suppress(feature, !suppressed),
                ));
                out.push(item(
                    label("feature.delete"),
                    Icon::Delete,
                    None,
                    Act::DeleteFeature(feature),
                ));
            }
            None => {
                out.push(item(
                    label("body.rename"),
                    Icon::Edit,
                    None,
                    Act::Rename(RenameTarget::Body(body)),
                ));
                out.push(item(
                    label("body.remove"),
                    Icon::Delete,
                    None,
                    Act::RemoveBody(body),
                ));
            }
        }
        out.push(Entry::Separator);
        if measure {
            out.push(measure_item(at));
        }
        out.push(item(
            "Properties",
            Icon::Parameters,
            None,
            Act::Properties(body),
        ));
        out
    }

    /// The items of a menu opened on `target` at pixel `px`.
    pub(crate) fn menu_entries(&self, target: &Target, px: Px) -> Vec<Entry> {
        match target {
            Target::Sketch => self.sketch_entries(),
            Target::Empty => self.empty_entries(),
            Target::Edge(edge) => {
                let mut out = vec![Entry::Caption("EDGE".into())];
                for (chamfer, action) in [(false, Action::Fillet), (true, Action::Chamfer)] {
                    out.push(item(
                        action_label(action),
                        crate::search::icon_of(action),
                        self.why(action),
                        Act::Blend {
                            chamfer,
                            edge: edge.clone(),
                        },
                    ));
                }
                out.push(Entry::Separator);
                out.push(measure_item(Some(px)));
                out
            }
            Target::Body {
                body,
                face,
                body_items,
            } => {
                let mut out = Vec::new();
                if let Some(face) = face {
                    // Face names are `feature:role`; the role says which face it is.
                    let role = chamfera_engine::solids::split_face_key(face)
                        .map_or(face.as_str(), |(_, r)| r);
                    out.push(Entry::Caption(format!("{} FACE", role.to_uppercase())));
                    out.push(item(
                        label("sketch.create_on_face"),
                        Icon::SketchCreate,
                        self.why(Action::SketchOnFace),
                        Act::SketchOnFace(px),
                    ));
                    out.push(self.run_item(Action::Extrude));
                    out.push(item(
                        action_label(Action::PressPull),
                        Icon::PressPull,
                        self.why(Action::PressPull),
                        Act::PressPullAt(px),
                    ));
                    out.push(measure_item(Some(px)));
                    if *body_items {
                        out.push(Entry::Separator);
                    }
                }
                if *body_items {
                    out.extend(self.body_entries(*body, Some(px), face.is_none()));
                }
                out
            }
        }
    }

    fn empty_entries(&self) -> Vec<Entry> {
        let mut out = vec![
            self.run_item(Action::Home),
            self.run_item(Action::Fit),
            Entry::Separator,
        ];
        if let Some(a) = self.ctx_menu.last_action {
            out.push(item(
                format!("Repeat {}", action_label(a)),
                crate::search::icon_of(a),
                self.why(a),
                Act::Run(a),
            ));
        }
        out.push(self.run_item(Action::Undo));
        out.push(self.run_item(Action::Redo));
        out.push(Entry::Separator);
        let hidden = self.session.scene.bodies.iter().any(|b| !b.visible);
        out.push(item(
            "Show All Bodies",
            Icon::Eye,
            (!hidden).then(|| "No body is hidden".to_owned()),
            Act::ShowAll,
        ));
        out
    }

    fn sketch_entries(&self) -> Vec<Entry> {
        let mut out = vec![self.run_item(Action::FinishSketch)];
        if let Some(t) = self.ctx_menu.last_tool {
            let a = Action::SketchTool(t);
            out.push(item(
                format!("Repeat {}", action_label(a)),
                crate::search::icon_of(a),
                self.why(a),
                Act::Run(a),
            ));
        }
        out.push(Entry::Separator);
        out.push(self.run_item(Action::Dimension));
        out.push(self.run_item(Action::Construction));
        out.push(self.run_item(Action::Delete));
        // Constraints that suit the selection, as the CONSTRAINTS buttons apply them.
        let (mut points, mut lines, mut others) = (0, 0, 0);
        let sketch = self.session.active_sketch();
        for i in &self.session.sketch_selection {
            match i {
                SketchItem::Point(_) => points += 1,
                SketchItem::Curve(id) => {
                    let line = sketch
                        .and_then(|f| f.sketch.curve(chamfera_sketch::CurveId(*id)))
                        .is_some_and(chamfera_sketch::Curve::is_line);
                    if line {
                        lines += 1;
                    } else {
                        others += 1;
                    }
                }
            }
        }
        let fits: Vec<Entry> = crate::sketch_ui::CONSTRAINTS
            .iter()
            .filter(|(kind, ..)| constraint_fits(kind, points, lines, others))
            .map(|(kind, text, icon, _)| item(*text, *icon, None, Act::Constrain(kind)))
            .collect();
        out.push(Entry::Separator);
        if fits.is_empty() {
            out.push(Entry::Caption("SELECT GEOMETRY TO CONSTRAIN".into()));
        } else {
            out.push(Entry::Caption("CONSTRAIN".into()));
            out.extend(fits);
        }
        out
    }

    /// Does what a menu item says.
    pub(crate) fn menu_act(&mut self, act: Act) {
        match act {
            Act::Run(Action::Cancel) => {}
            Act::Run(a) => self.run_action(a),
            Act::Visibility(body, visible) => {
                self.run(
                    "body.visibility",
                    json!({ "body": body, "visible": visible }),
                );
            }
            Act::Isolate(body) => {
                let all: Vec<(u32, bool)> = self
                    .session
                    .scene
                    .bodies
                    .iter()
                    .map(|b| (b.id, b.visible))
                    .collect();
                for (id, visible) in all {
                    if visible != (id == body) {
                        self.run(
                            "body.visibility",
                            json!({ "body": id, "visible": id == body }),
                        );
                    }
                }
            }
            Act::ShowAll => {
                let hidden: Vec<u32> = self
                    .session
                    .scene
                    .bodies
                    .iter()
                    .filter(|b| !b.visible)
                    .map(|b| b.id)
                    .collect();
                for id in hidden {
                    self.run("body.visibility", json!({ "body": id, "visible": true }));
                }
            }
            Act::SelectAt((x, y, w, h)) => {
                self.run(
                    "select.pick",
                    json!({ "x": x, "y": y, "width": w, "height": h }),
                );
            }
            Act::EditFeature(f) => self.edit_feature(f),
            Act::Suppress(f, suppressed) => {
                self.run(
                    "feature.edit",
                    json!({ "feature": f, "suppressed": suppressed }),
                );
            }
            Act::DeleteFeature(f) => {
                self.run("feature.delete", json!({ "feature": f }));
            }
            Act::RemoveBody(b) => {
                self.run("body.remove", json!({ "body": b }));
            }
            Act::Properties(b) => self.open_properties(b),
            Act::SketchOnFace((x, y, w, h)) => self.face_click(x, y, w, h),
            Act::PressPullAt((x, y, w, h)) => {
                self.run_action(Action::PressPull);
                if self.solid.press_pull.is_some() {
                    self.press_pull_click(x, y, w, h);
                }
            }
            Act::Measure(at) => {
                // Open it fresh (`open_inspect` toggles an open Measure closed).
                self.solid.inspect = None;
                self.open_inspect(crate::inspect_ui::InspectMode::Measure);
                if let Some((x, y, w, h)) = at {
                    self.inspect_click(x, y, w, h);
                }
            }
            Act::Blend { chamfer, edge } => {
                let action = if chamfer {
                    Action::Chamfer
                } else {
                    Action::Fillet
                };
                self.run_action(action);
                if let Some(tool) = self.solid.blend.as_mut()
                    && !tool.edges.contains(&edge)
                {
                    tool.edges.push(edge);
                }
            }
            Act::Constrain(kind) => {
                self.run_sketch("sketch.constrain_selection", json!({ "kind": kind }));
            }
            Act::Rename(target) => {
                let current = match target {
                    RenameTarget::Feature(id) => {
                        self.session.doc.feature(id).map(|f| f.name.clone()).ok()
                    }
                    RenameTarget::Sketch(id) => {
                        self.session.doc.sketch(id).map(|s| s.name.clone()).ok()
                    }
                    RenameTarget::Body(id) => self.session.scene.body(id).map(|b| b.name.clone()),
                };
                if let Some(name) = current {
                    self.ctx_menu.rename = Some((target, name, true));
                }
            }
            Act::EditSketch(id) => self.edit_sketch(id),
            Act::DeleteSketch(id) => {
                self.run("sketch.delete", json!({ "sketch": id }));
            }
        }
    }

    /// The open viewport menu. Esc, a click elsewhere or an item closes it.
    pub(crate) fn context_menu_window(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.ctx_menu.menu.as_ref() else {
            return;
        };
        let entries = self.menu_entries(&menu.target, menu.px);
        let pos = menu.pos;
        let fresh = menu.fresh;
        let look = self.look();
        let mut shown = Vec::new();
        let area = egui::Area::new(egui::Id::unique("viewport context menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .constrain(true)
            .fade_in(false)
            .show(ctx, |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::same(widgets::S2 as i8))
                    .show(ui, |ui| {
                        ui.set_width(220.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                        show_entries(ui, look, &entries, &mut shown)
                    })
                    .inner
            });
        let chosen = area.inner;
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let elsewhere = !fresh && area.response.clicked_elsewhere();
        if let Some(m) = self.ctx_menu.menu.as_mut() {
            m.fresh = false;
            m.shown = shown;
        }
        if chosen.is_some() || esc || elsewhere {
            self.ctx_menu.menu = None;
        }
        if let Some(act) = chosen {
            self.menu_act(act);
        }
    }

    /// Works out what the pointer is over, only when it or the view moved. `at` is the
    /// pointer's viewport pixel, `None` when it is not over the viewport.
    pub(crate) fn hover_update(&mut self, at: Option<Px>) {
        let quiet = self.session.active_sketch.is_none()
            && self.solid.extrude.is_none()
            && self.solid.blend.is_none()
            && self.ctx_menu.menu.is_none();
        let Some(px) = at.filter(|_| quiet) else {
            self.ctx_menu.hover_key = None;
            self.ctx_menu.hover = None;
            return;
        };
        let key = HoverKey {
            px,
            camera: self.session.camera,
            bodies: self.session.scene.bodies.iter().map(|b| b.id).collect(),
            filter: self.ctx_menu.filter,
        };
        if self.ctx_menu.hover_key.as_ref() == Some(&key) {
            return;
        }
        let (x, y, w, h) = px;
        let picked = self.session.pick_filtered(x, y, w, h, &key.filter);
        let lines = self.picked_lines(&picked);
        self.ctx_menu.hover = (!lines.is_empty()).then_some((picked, lines));
        self.ctx_menu.hover_key = Some(key);
    }

    /// The lines that show a picked edge or face (its boundary edges).
    fn picked_lines(&self, picked: &Picked) -> Vec<[Point3; 2]> {
        match picked {
            Picked::Edge { name, .. } => self.edge_lines(std::iter::once(name)),
            Picked::Face { hit, .. } => {
                let Some(solid) = self.solid_of(hit.body) else {
                    return Vec::new();
                };
                let Some(face) = solid.faces.get(hit.triangle) else {
                    return Vec::new();
                };
                let Ok(edges) = solid.body.face_edges(*face) else {
                    return Vec::new();
                };
                solid
                    .edges
                    .iter()
                    .filter(|(e, _)| edges.contains(e))
                    .flat_map(|(_, line)| {
                        line.windows(2)
                            .filter_map(|p| match p {
                                [a, b] => Some([*a, *b]),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect()
            }
            Picked::Body(_) | Picked::Nothing => Vec::new(),
        }
    }

    /// The selected edges (highlighted) and the hover pre-highlight, drawn over the model.
    pub(crate) fn pick_overlays(&self) -> Vec<Overlay> {
        let mut out = Vec::new();
        if !self.session.selected_edges.is_empty() {
            out.push(Overlay {
                style: OverlayStyle::Highlight,
                lines: self.edge_lines(self.session.selected_edges.iter()),
                points: Vec::new(),
                on_top: true,
            });
        }
        if let Some((_, lines)) = self.ctx_menu.hover.as_ref() {
            out.push(Overlay {
                style: OverlayStyle::Hover,
                lines: lines.clone(),
                points: Vec::new(),
                on_top: true,
            });
        }
        out
    }

    /// The selection filter: a row of toggles at the bottom-left of the viewport.
    pub(crate) fn selection_filter_bar(&mut self, ctx: &egui::Context, rect: Rect) {
        let look = self.look();
        let sketching = self.session.active_sketch.is_some();
        let kinds: [(&str, &str, &str); 3] = if sketching {
            [
                ("vertices", "Points", "Clicks may pick sketch points"),
                ("sketch", "Curves", "Clicks may pick sketch curves"),
                (
                    "construction",
                    "Construction",
                    "Clicks may pick construction curves",
                ),
            ]
        } else {
            [
                ("bodies", "Bodies", "Clicks may pick whole bodies"),
                ("faces", "Faces", "Clicks may pick faces of solids"),
                ("edges", "Edges", "Clicks may pick edges of solids"),
            ]
        };
        let mut filter = self.ctx_menu.filter;
        egui::Area::new(egui::Id::unique("selection filter"))
            .order(egui::Order::Middle)
            .fade_in(false)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(rect.left_bottom() + vec2(widgets::S3, -widgets::S3))
            .show(ctx, |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::symmetric(widgets::S2 as i8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            widgets::caption(ui, look, "SELECT");
                            ui.add_space(2.0);
                            for (kind, text, tip) in kinds {
                                let Some(on) = filter.kind_mut(kind) else {
                                    continue;
                                };
                                let colour = if *on {
                                    look.c(|t| t.text)
                                } else {
                                    look.c(|t| t.text_muted)
                                };
                                let r = ui
                                    .add(
                                        egui::Button::selectable(
                                            *on,
                                            egui::RichText::new(text)
                                                .size(widgets::T_SMALL)
                                                .color(colour),
                                        )
                                        .min_size(vec2(0.0, 22.0)),
                                    )
                                    .on_hover_text(tip);
                                if r.clicked() {
                                    *on = !*on;
                                }
                            }
                        });
                    });
            });
        if filter != self.ctx_menu.filter {
            self.ctx_menu.filter = filter;
            self.ctx_menu.hover_key = None;
        }
    }

    fn open_properties(&mut self, body: u32) {
        let Some(b) = self.session.scene.body(body) else {
            return;
        };
        let mut rows = Vec::new();
        let mut surface = false;
        let (volume, area) = match self.solid_of(body) {
            Some(s) => {
                if let Ok(f) = self.session.doc.feature(s.feature) {
                    rows.push(("Feature".into(), f.name.clone()));
                }
                surface = s.body.is_open();
                let kind = if surface {
                    "Surface (exact)"
                } else {
                    "Solid (exact)"
                };
                rows.push(("Kind".into(), kind.into()));
                (s.body.volume().ok(), s.body.area().ok())
            }
            None => {
                rows.push(("Kind".into(), "Mesh".into()));
                (Some(b.mesh.signed_volume().abs()), Some(b.mesh.area()))
            }
        };
        let u = self.session.doc.units;
        let text = |v: Option<f64>, show: &dyn Fn(f64) -> String| match v {
            Some(v) => show(v),
            None => "could not be measured".into(),
        };
        // An open surface encloses nothing: it has no volume to measure.
        let volume_text = if surface {
            "none (an open surface)".into()
        } else {
            text(volume, &|v| u.volume(v))
        };
        rows.push(("Volume".into(), volume_text));
        rows.push(("Area".into(), text(area, &|v| u.area(v))));
        if let Some(bb) = b.mesh.bounds() {
            let d = bb.max - bb.min;
            rows.push((
                "Size".into(),
                format!(
                    "{} × {} × {} {}",
                    u.length_number(d.x),
                    u.length_number(d.y),
                    u.length_number(d.z),
                    u.length.symbol()
                ),
            ));
        }
        self.ctx_menu.properties = Some(Properties {
            title: b.name.clone(),
            rows,
        });
    }

    /// The Properties window (volume and area of a body).
    pub(crate) fn properties_window(&mut self, ctx: &egui::Context) {
        let Some(p) = self.ctx_menu.properties.as_ref() else {
            return;
        };
        let look = self.look();
        let mut close = false;
        egui::Window::new("Properties")
            .id(egui::Id::unique("properties window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(self.tool_dialog_pos(280.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(280.0);
                if widgets::dialog_title(ui, look, Icon::Parameters, &p.title) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("properties grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S2])
                        .show(ui, |ui| {
                            for (k, v) in &p.rows {
                                widgets::field_label(ui, look, k);
                                ui.label(v);
                                ui.end_row();
                            }
                        });
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ctx_menu.properties = None;
        }
    }

    /// The Rename dialog, for features, sketches and bodies.
    pub(crate) fn rename_dialog(&mut self, ctx: &egui::Context) {
        let pos = self.tool_dialog_pos(300.0);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some((target, text, focus)) = self.ctx_menu.rename.as_mut() else {
            return;
        };
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Rename")
            .id(egui::Id::unique("rename dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(300.0);
                if widgets::dialog_title(ui, look, Icon::Edit, "Rename") {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    let edit = ui.add_sized([268.0, 26.0], egui::TextEdit::singleline(text));
                    if *focus {
                        edit.request_focus();
                        *focus = false;
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "OK", !text.trim().is_empty()).clicked() {
                        ok = true;
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        let (target, name) = (*target, text.trim().to_owned());
        if ok && !name.is_empty() {
            let done = match target {
                RenameTarget::Feature(id) => {
                    self.run("feature.rename", json!({ "feature": id, "name": name }))
                }
                RenameTarget::Sketch(id) => {
                    self.run("sketch.rename", json!({ "sketch": id, "name": name }))
                }
                RenameTarget::Body(id) => {
                    self.run("body.rename", json!({ "body": id, "name": name }))
                }
            };
            if done.is_some() {
                self.ctx_menu.rename = None;
            }
        } else if cancel {
            self.ctx_menu.rename = None;
        }
    }
}

/// Measure: opens the Measure tool, with what is under `at` as its first pick when the
/// menu was opened in the viewport.
fn measure_item(at: Option<Px>) -> Entry {
    item(
        label("inspect.measure"),
        Icon::Measure,
        None,
        Act::Measure(at),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraints_offered_match_what_the_command_accepts() {
        // One line: horizontal and vertical; two lines: parallel, perpendicular, equal.
        assert!(constraint_fits("horizontal", 0, 1, 0));
        assert!(!constraint_fits("parallel", 0, 1, 0));
        for k in ["parallel", "perpendicular", "equal"] {
            assert!(constraint_fits(k, 0, 2, 0), "{k}");
        }
        assert!(!constraint_fits("tangent", 0, 2, 0));
        // A line and a circle: tangent and equal, not parallel.
        assert!(constraint_fits("tangent", 0, 1, 1));
        assert!(!constraint_fits("parallel", 0, 1, 1));
        // Two points: coincident; a point and a line: point on, midpoint.
        assert!(constraint_fits("coincident", 2, 0, 0));
        assert!(constraint_fits("point_on", 1, 1, 0));
        assert!(constraint_fits("midpoint", 1, 1, 0));
        assert!(!constraint_fits("midpoint", 1, 0, 1));
        assert!(constraint_fits("fix", 3, 0, 0));
        assert!(!constraint_fits("fix", 0, 0, 0));
    }

    #[test]
    fn the_empty_space_menu_has_the_view_and_history_items() {
        let app = ChamferaApp::headless();
        let entries = app.menu_entries(&Target::Empty, (0.0, 0.0, 100.0, 100.0));
        let texts: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item { text, why_not, .. } => Some(format!("{text}:{}", why_not.is_none())),
                _ => None,
            })
            .collect();
        // Nothing done yet: Undo and Redo are shown but disabled; nothing is hidden.
        assert!(
            texts
                .iter()
                .any(|t| t.ends_with(":true") && t.contains("Home"))
        );
        assert!(
            texts
                .iter()
                .any(|t| t.ends_with(":true") && t.contains("Fit"))
        );
        assert!(texts.iter().any(|t| t == "Undo:false"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Redo:false"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Show All Bodies:false"));
    }

    // ----- real pointer events through the whole UI (as `input_tests` in lib.rs) -----

    use eframe::App;
    use egui::{Event, PointerButton, Vec2};

    fn frame(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64, events: Vec<Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
            time: Some(*t),
            events,
            ..Default::default()
        };
        let mut f = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
        out.textures_delta.clear();
        *t += 0.05;
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A press and release at `at` with no movement between.
    fn click(app: &mut ChamferaApp, ctx: &egui::Context, t: &mut f64, at: Pos2, b: PointerButton) {
        frame(app, ctx, t, vec![Event::PointerMoved(at)]);
        frame(app, ctx, t, vec![button(at, b, true)]);
        frame(app, ctx, t, vec![button(at, b, false)]);
        frame(app, ctx, t, vec![]);
    }

    /// The sample bracket, fitted in the viewport after the layout has settled.
    fn sample_app(ctx: &egui::Context, t: &mut f64) -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        app.run("body.sample", json!({}));
        app.fit_pending = true;
        for _ in 0..4 {
            frame(&mut app, ctx, t, vec![]);
        }
        app
    }

    /// A 50 x 30 x 10 plate extruded from the XY plane, seen from the top and fitted, so
    /// the viewport's centre looks down on the middle of the top face, far from its edges.
    fn plate_app(ctx: &egui::Context, t: &mut f64) -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        for _ in 0..3 {
            frame(&mut app, ctx, t, vec![]);
        }
        let sketch = app.run("sketch.create", json!({ "plane": "xy" })).unwrap()["sketch"].clone();
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": sketch, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        )
        .unwrap();
        app.run("view.standard", json!({ "view": "top" }));
        app.run("view.fit", app.viewport_params());
        frame(&mut app, ctx, t, vec![]);
        app
    }

    #[test]
    fn a_right_click_opens_the_menu_and_a_right_drag_orbits_without_one() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = sample_app(&ctx, &mut t);
        let c = app.viewport_rect.center();
        click(&mut app, &ctx, &mut t, c, PointerButton::Secondary);
        let menu = app
            .ctx_menu
            .menu
            .as_ref()
            .expect("no menu after a right click");
        assert!(
            matches!(menu.target, Target::Body { .. }),
            "{:?}",
            menu.target
        );
        assert!(!menu.shown.is_empty(), "the menu drew no items");
        // Esc closes it.
        frame(
            &mut app,
            &ctx,
            &mut t,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.ctx_menu.menu.is_none());
        // A right drag of 120 px orbits and opens no menu.
        let yaw = app.session.camera.yaw;
        frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c)]);
        frame(
            &mut app,
            &ctx,
            &mut t,
            vec![button(c, PointerButton::Secondary, true)],
        );
        for k in 1..=8 {
            let p = c + Vec2::new(15.0 * k as f32, 0.0);
            frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(p)]);
        }
        let end = c + Vec2::new(120.0, 0.0);
        frame(
            &mut app,
            &ctx,
            &mut t,
            vec![button(end, PointerButton::Secondary, false)],
        );
        frame(&mut app, &ctx, &mut t, vec![]);
        assert!(app.ctx_menu.menu.is_none(), "a right drag opened the menu");
        // view.orbit turns by -dx * 0.008 rad per point; egui holds back the first few
        // points until the drag is decided, so at least half of 120 points arrive.
        let turned = yaw - app.session.camera.yaw;
        assert!(turned > 60.0 * 0.008 - 1e-9, "orbited by {turned}");
    }

    #[test]
    fn the_body_menus_hide_item_hides_the_body() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = sample_app(&ctx, &mut t);
        let body = app.session.scene.bodies[0].id;
        let c = app.viewport_rect.center();
        click(&mut app, &ctx, &mut t, c, PointerButton::Secondary);
        let hide = app
            .ctx_menu
            .menu
            .as_ref()
            .and_then(|m| m.shown.iter().find(|(text, _)| text == "Hide"))
            .map(|(_, r)| r.center())
            .expect("no Hide item");
        click(&mut app, &ctx, &mut t, hide, PointerButton::Primary);
        let visible = app.session.scene.body(body).map(|b| b.visible);
        assert_eq!(visible, Some(false));
        assert!(app.ctx_menu.menu.is_none(), "the menu stayed open");
        // On empty space now: Show All Bodies brings it back.
        click(&mut app, &ctx, &mut t, c, PointerButton::Secondary);
        let menu = app.ctx_menu.menu.as_ref().expect("no menu on empty space");
        assert_eq!(menu.target, Target::Empty);
        let show = menu
            .shown
            .iter()
            .find(|(text, _)| text == "Show All Bodies")
            .map(|(_, r)| r.center())
            .expect("no Show All Bodies item");
        click(&mut app, &ctx, &mut t, show, PointerButton::Primary);
        assert_eq!(app.session.scene.body(body).map(|b| b.visible), Some(true));
    }

    #[test]
    fn a_filter_that_passes_only_edges_makes_a_click_on_a_face_pick_nothing() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = plate_app(&ctx, &mut t);
        let c = app.viewport_rect.center();
        app.ctx_menu.filter = PickFilter {
            edges: true,
            ..PickFilter::NONE
        };
        click(&mut app, &ctx, &mut t, c, PointerButton::Primary);
        assert!(app.session.scene.selection.is_none(), "the face was picked");
        assert!(app.session.selected_edges.is_empty());
        // With every kind passing, the same click picks the top face.
        app.ctx_menu.filter = PickFilter::ALL;
        click(&mut app, &ctx, &mut t, c, PointerButton::Primary);
        assert!(app.session.scene.selection.is_some(), "nothing picked");
        assert!(
            app.session
                .selected_face()
                .is_some_and(|f| f.ends_with(":top"))
        );
    }

    #[test]
    fn hovering_a_face_lights_its_outline_and_a_right_click_offers_a_sketch_on_it() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = plate_app(&ctx, &mut t);
        let c = app.viewport_rect.center();
        frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c)]);
        let (picked, lines) = app.ctx_menu.hover.clone().expect("no hover");
        assert!(matches!(picked, Picked::Face { .. }), "{picked:?}");
        // The top face is bounded by its four edges, each drawn as one segment.
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().flatten().all(|p| (p.z - 10.0).abs() < 1e-9));
        click(&mut app, &ctx, &mut t, c, PointerButton::Secondary);
        let menu = app.ctx_menu.menu.as_ref().expect("no menu");
        let texts: Vec<&str> = menu.shown.iter().map(|(s, _)| s.as_str()).collect();
        assert!(texts.contains(&label("sketch.create_on_face")), "{texts:?}");
        assert!(texts.contains(&label("feature.delete")), "{texts:?}");
    }

    /// Properties of a surface body say so, and give it no volume (QA user guide pass:
    /// "Solid (exact)" and "could not be measured"). The 50 x 30 rectangle's sides
    /// extruded 10 as a surface: area 2 (50 + 30) 10 = 1600.
    #[test]
    fn properties_of_a_surface_say_surface_and_no_volume() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "surface.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        )
        .unwrap();
        let body = app.session.scene.bodies[0].id;
        app.open_properties(body);
        let rows = app.ctx_menu.properties.as_ref().unwrap().rows.clone();
        let get = |k: &str| rows.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("Kind").as_deref(), Some("Surface (exact)"));
        assert_eq!(get("Volume").as_deref(), Some("none (an open surface)"));
        assert_eq!(get("Area").as_deref(), Some("1600.00 mm²"));
    }

    /// The face menu's Press Pull and Measure items run their tools on the face under
    /// the cursor: Press Pull opens with the plate's top face as its target, and Measure
    /// opens with the top face as its first pick (the plate's top is at z = 10).
    #[test]
    fn the_face_menus_press_pull_and_measure_open_their_tools_on_that_face() {
        let ctx = egui::Context::default();
        let mut t = 0.0;
        let mut app = plate_app(&ctx, &mut t);
        let c = app.viewport_rect.center();
        let menu_item = |app: &mut ChamferaApp, t: &mut f64, text: &str| {
            click(app, &ctx, t, c, PointerButton::Secondary);
            let menu = app.ctx_menu.menu.as_ref().expect("no menu");
            let entries = app.menu_entries(&menu.target, menu.px);
            let enabled = entries
                .iter()
                .any(|e| matches!(e, Entry::Item { text: s, why_not: None, .. } if s == text));
            assert!(enabled, "{text} is not offered enabled: {entries:?}");
            let at = menu
                .shown
                .iter()
                .find(|(s, _)| s == text)
                .map(|(_, r)| r.center())
                .expect("item not drawn");
            click(app, &ctx, t, at, PointerButton::Primary);
        };
        menu_item(&mut app, &mut t, "Press Pull");
        let face = app
            .solid
            .press_pull
            .as_ref()
            .and_then(|p| p.target.as_ref())
            .and_then(|t| {
                t.params
                    .get("face")
                    .and_then(|f| f.as_str())
                    .map(str::to_owned)
            });
        assert!(
            face.is_some_and(|f| f.ends_with(":top")),
            "no top face target"
        );
        app.solid.press_pull = None;
        menu_item(&mut app, &mut t, "Measure");
        let tool = app.solid.inspect.as_ref().expect("Measure did not open");
        assert_eq!(tool.mode, crate::inspect_ui::InspectMode::Measure);
        assert_eq!(tool.picks.len(), 1, "{:?}", tool.picks);
        assert!(
            tool.picks[0].to_string().contains("top"),
            "{:?}",
            tool.picks
        );
    }
}

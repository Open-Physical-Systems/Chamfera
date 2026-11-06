//! The timeline at the bottom: one tile per sketch and feature in build order, and the
//! marker after the last one that is built. Drag the marker to roll the design back (the
//! items after it are greyed and not built); drag a tile to move it; click, Ctrl- or
//! Shift-click to select tiles and group them from the right-click menu. Suppressed
//! features are faded, failed ones marked; hover for details, double-click to edit, F2 to
//! rename. Acts through `timeline.*`, `sketch.edit`, `feature.edit`, `feature.delete` and
//! the rename commands.

use crate::icons::Icon;
use crate::solid_ui::{ExtrudeForm, extrude_summary};
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{FeatureKind, ItemKind};
use egui::{Align2, CornerRadius, FontId, Id, Rect, Sense, Stroke, StrokeKind, Vec2, vec2};
use serde_json::json;
use std::collections::BTreeSet;

/// A feature being edited in the Edit Feature dialog.
pub struct FeatureEdit {
    feature: u32,
    /// The value a revolve, pattern, fillet or chamfer shows or edits.
    distance: String,
    /// An extrude's options.
    pub(crate) extrude: Option<ExtrudeForm>,
    suppressed: bool,
}

/// A sketch, feature or group being renamed in a small text box at `at`.
pub(crate) struct Rename {
    pub(crate) item: u32,
    pub(crate) text: String,
    /// The tile or row being renamed (the box goes above a tile, over a row).
    pub(crate) at: Rect,
    pub(crate) above: bool,
    focused: bool,
}

impl Rename {
    pub(crate) fn new(item: u32, text: String, at: Rect, above: bool) -> Rename {
        Rename {
            item,
            text,
            at,
            above,
            focused: false,
        }
    }
}

/// What is being dragged along the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Marker,
    /// A sketch, feature or group (by id).
    Item(u32),
}

/// Where a tile was drawn last frame, and the timeline indices it covers (`start..end`; a
/// collapsed group covers all its items), so a pointer x becomes a gap index.
#[derive(Debug, Clone, Copy)]
struct Slot {
    rect: Rect,
    start: usize,
    end: usize,
}

#[derive(Default)]
pub struct TimelineUi {
    pub editing: Option<FeatureEdit>,
    /// Selected tiles (sketch and feature ids), for grouping.
    selected: Vec<u32>,
    /// Folded groups. How the timeline looks, not part of the design.
    collapsed: BTreeSet<u32>,
    pub(crate) renaming: Option<Rename>,
    drag: Option<Drag>,
    slots: Vec<Slot>,
    marker_rect: Option<Rect>,
    /// The feature whose error is shown in the error window.
    error_shown: Option<u32>,
    /// The tile under the pointer (F2 renames it).
    hovered: Option<(u32, Rect)>,
}

/// One tile: a sketch or a feature.
#[derive(Clone)]
struct Item {
    id: u32,
    name: String,
    icon: Icon,
    sketch: bool,
    suppressed: bool,
    active: bool,
    /// Before the marker: built.
    built: bool,
    group: Option<u32>,
    error: Option<String>,
    /// The upstream reference whose change made this feature fail.
    broken: Option<String>,
    warning: Option<String>,
    details: String,
}

const TILE: f32 = 30.0;
/// Width of the marker's grab area.
const MARKER_WIDTH: f32 = 12.0;
/// How faded the icons of items after the marker are.
const ROLLED_BACK_FADE: f32 = 0.3;

impl ChamferaApp {
    fn timeline_items(&self) -> Vec<Item> {
        let doc = &self.session.doc;
        let sketch_name = |id: u32| {
            doc.sketch(id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|_| format!("sketch {id}"))
        };
        let built = doc.built_items();
        let mut items = Vec::new();
        for id in doc.timeline_order() {
            let group = doc.group_of(id).map(|g| g.id);
            if let Ok(s) = doc.sketch(id) {
                let profiles = s.sketch.profiles().len();
                let place = if s.attached.is_some() {
                    "on a face"
                } else {
                    "on an origin plane"
                };
                items.push(Item {
                    id: s.id,
                    name: s.name.clone(),
                    icon: Icon::Sketch,
                    sketch: true,
                    suppressed: false,
                    active: self.session.active_sketch == Some(s.id),
                    built: built.contains(&id),
                    group,
                    error: self.session.sketch_errors().get(&id).cloned(),
                    broken: None,
                    warning: None,
                    details: format!(
                        "Sketch {place} · {profiles} closed profile{}",
                        if profiles == 1 { "" } else { "s" }
                    ),
                });
                continue;
            }
            let Ok(f) = doc.feature(id) else {
                continue;
            };
            let (icon, details) = match &f.kind {
                FeatureKind::Extrude(e) => (
                    Icon::Extrude,
                    format!(
                        "Extrude {} · from {}",
                        extrude_summary(e),
                        sketch_name(e.sketch)
                    ),
                ),
                FeatureKind::Revolve(r) => (
                    Icon::Revolve,
                    format!("Revolve {} · from {}", r.angle, sketch_name(r.sketch)),
                ),
                FeatureKind::Pattern(p) => (
                    Icon::PatternRectangular,
                    format!(
                        "Pattern × {} of {}",
                        p.count,
                        doc.item_name(p.source)
                            .unwrap_or_else(|| format!("feature {}", p.source))
                    ),
                ),
                FeatureKind::Hole(h) => (Icon::Hole, crate::hole_ui::hole_details(h)),
                FeatureKind::Construct(c) => (crate::construct_ui::feature_icon(c), c.describe()),
                FeatureKind::Blend(b) => (
                    match b.blend {
                        chamfera_doc::BlendKind::Fillet => Icon::Fillet,
                        chamfera_doc::BlendKind::Chamfer => Icon::Chamfer,
                    },
                    format!(
                        "{} {} · {} edge{}",
                        match b.blend {
                            chamfera_doc::BlendKind::Fillet => "Fillet",
                            chamfera_doc::BlendKind::Chamfer => "Chamfer",
                        },
                        b.size,
                        b.edges.len(),
                        if b.edges.len() == 1 { "" } else { "s" }
                    ),
                ),
                FeatureKind::Mirror(m) => {
                    (Icon::MirrorBody, crate::transform_ui::mirror_details(m))
                }
                FeatureKind::Move(m) => (Icon::Move, crate::transform_ui::move_details(m)),
                FeatureKind::OffsetFace(_)
                | FeatureKind::DeleteFace(_)
                | FeatureKind::MoveFace(_) => crate::press_pull_ui::timeline_entry(&f.kind),
                FeatureKind::Shell(sh) => (
                    Icon::Shell,
                    format!(
                        "Shell {} · {} open face{}",
                        sh.thickness,
                        sh.faces.len(),
                        if sh.faces.len() == 1 { "" } else { "s" }
                    ),
                ),
                FeatureKind::Sweep(w) => (
                    Icon::Sweep,
                    format!(
                        "Sweep · from {} along {} curve{} of {}",
                        sketch_name(w.sketch),
                        w.path.len(),
                        if w.path.len() == 1 { "" } else { "s" },
                        sketch_name(w.path_sketch)
                    ),
                ),
                FeatureKind::Loft(l) => (
                    Icon::Loft,
                    format!("Loft · through {} profiles", l.sections.len()),
                ),
            };
            items.push(Item {
                id: f.id,
                name: f.name.clone(),
                icon,
                sketch: false,
                suppressed: f.suppressed,
                active: false,
                built: built.contains(&id),
                group,
                error: self.session.feature_errors().get(&f.id).cloned(),
                broken: self.session.broken_references().get(&f.id).cloned(),
                warning: self.session.display_errors().get(&f.id).cloned(),
                details,
            });
        }
        items
    }

    /// The gap (a timeline index from 0 to the number of items) nearest to pointer x,
    /// from where the tiles were drawn.
    fn timeline_gap_at(&self, x: f32) -> usize {
        let slots = &self.timeline.slots;
        for s in slots {
            if x < s.rect.center().x {
                return s.start;
            }
        }
        slots.last().map_or(0, |s| s.end)
    }

    /// Where on screen gap `g` is: halfway between the tiles beside it.
    fn timeline_gap_x(&self, g: usize) -> Option<f32> {
        let slots = &self.timeline.slots;
        let after = slots.iter().find(|s| s.start >= g);
        let before = slots.iter().rev().find(|s| s.end <= g);
        match (before, after) {
            (Some(b), Some(a)) => Some((b.rect.right() + a.rect.left()) * 0.5),
            (None, Some(a)) => Some(a.rect.left() - 5.0),
            (Some(b), None) => Some(b.rect.right() + 5.0),
            (None, None) => None,
        }
    }

    /// The timeline bar.
    pub(crate) fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let items = self.timeline_items();
        let border = self.look().c(|t| t.border);
        let muted = self.look().c(|t| t.text_muted);
        let ids: BTreeSet<u32> = items.iter().map(|i| i.id).collect();
        self.timeline.selected.retain(|i| ids.contains(i));
        self.timeline.hovered = None;
        let n = items.len();
        let marker = self.session.doc.marker().min(n);
        let pointer = ui.ctx().pointer_latest_pos();
        // While the marker is dragged, the tiles show where it would go.
        let shown = match (self.timeline.drag, pointer) {
            (Some(Drag::Marker), Some(p)) => self.timeline_gap_at(p.x),
            _ => marker,
        };
        let mut slots = Vec::new();
        let mut marker_rect = None;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = widgets::S2;
            widgets::caption(ui, self.look(), "TIMELINE");
            ui.add_space(widgets::S3);
            if items.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Sketches and features appear here in the order they build",
                    )
                    .size(widgets::T_SMALL)
                    .color(muted),
                );
                return;
            }
            egui::ScrollArea::horizontal()
                .id_salt("timeline")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .stick_to_right(true)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let mut k = 0;
                        while k < n {
                            let Some(item) = items.get(k) else { break };
                            if k == marker {
                                marker_rect = Some(self.timeline_marker(ui));
                            } else if k > 0 {
                                connector(ui, border);
                            }
                            // A group starts with its header tile.
                            let group = item
                                .group
                                .and_then(|g| self.session.doc.group(g).cloned())
                                .filter(|g| g.items.first() == Some(&item.id));
                            if let Some(g) = group {
                                let len = g.items.len().max(1);
                                let folded = self.timeline.collapsed.contains(&g.id);
                                let faded = k >= shown;
                                let rect = self.timeline_group_tile(ui, &g, folded, faded, &items);
                                if folded {
                                    slots.push(Slot {
                                        rect,
                                        start: k,
                                        end: k + len,
                                    });
                                    // A marker inside a folded group shows after it.
                                    if marker > k && marker < k + len {
                                        marker_rect = Some(self.timeline_marker(ui));
                                    }
                                    k += len;
                                    continue;
                                }
                                connector(ui, border);
                            }
                            let rect = self.timeline_tile(ui, item, k >= shown);
                            slots.push(Slot {
                                rect,
                                start: k,
                                end: k + 1,
                            });
                            k += 1;
                        }
                        if marker >= n {
                            ui.add_space(widgets::S3);
                            marker_rect = Some(self.timeline_marker(ui));
                        }
                    });
                });
        });
        self.timeline.slots = slots;
        self.timeline.marker_rect = marker_rect;
        self.timeline_drag(ui, &items);
        self.timeline_brackets(ui, &items, border);
        // F2 renames the tile under the pointer, or the one selected tile.
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing && self.timeline.renaming.is_none() && ui.input(|i| i.key_pressed(egui::Key::F2))
        {
            let target = self.timeline.hovered.or_else(|| {
                let [one] = self.timeline.selected.as_slice() else {
                    return None;
                };
                self.timeline
                    .slots
                    .iter()
                    .zip(items.iter().map(|i| i.id))
                    .find(|(_, id)| id == one)
                    .map(|(s, id)| (id, s.rect))
            });
            if let Some((id, rect)) = target {
                self.start_rename(id, rect, true);
            }
        }
    }

    /// Opens the rename box for a sketch, feature or group.
    pub(crate) fn start_rename(&mut self, id: u32, at: Rect, above: bool) {
        if let Some(name) = self.session.doc.item_name(id) {
            self.timeline.renaming = Some(Rename::new(id, name, at, above));
        }
    }

    /// Renames through the item's own command.
    pub(crate) fn rename_item(&mut self, id: u32, name: &str) -> bool {
        let (cmd, params) = match self.session.doc.item_kind(id) {
            Some(ItemKind::Sketch) => ("sketch.rename", json!({ "sketch": id, "name": name })),
            Some(ItemKind::Feature) => ("feature.rename", json!({ "feature": id, "name": name })),
            Some(ItemKind::Group) => ("timeline.rename", json!({ "item": id, "name": name })),
            None => return false,
        };
        self.run(cmd, params).is_some()
    }

    /// The rename box, over everything (opened from the timeline or the browser).
    pub(crate) fn rename_box(&mut self, ctx: &egui::Context) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(r) = self.timeline.renaming.as_mut() else {
            return;
        };
        let width = r.at.width().max(180.0);
        let pos = if r.above {
            r.at.left_top() - vec2(0.0, 40.0)
        } else {
            r.at.left_top()
        };
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(Id::unique("rename box"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::same(widgets::S2 as i8))
                    .show(ui, |ui| {
                        let edit = ui.add_sized(
                            [width, 24.0],
                            egui::TextEdit::singleline(&mut r.text)
                                .id(Id::unique("rename text"))
                                .hint_text("Name"),
                        );
                        if !r.focused {
                            edit.request_focus();
                            r.focused = true;
                        }
                        if edit.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                commit = true;
                            } else {
                                cancel = true;
                            }
                        }
                    });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if commit {
            let (item, text) = (r.item, r.text.clone());
            if self.rename_item(item, &text) {
                self.timeline.renaming = None;
            }
        } else if cancel {
            self.timeline.renaming = None;
        }
    }

    /// The marker: where the build stops and new items go. Drag it to roll back.
    fn timeline_marker(&mut self, ui: &mut egui::Ui) -> Rect {
        let look = self.look();
        let (rect, _) = ui.allocate_exact_size(vec2(MARKER_WIDTH, TILE + 6.0), Sense::hover());
        let response = ui.interact(rect, Id::unique("timeline marker"), Sense::drag());
        let dragging = self.timeline.drag == Some(Drag::Marker);
        let colour = if dragging {
            look.c(|t| t.text_muted)
        } else {
            look.c(|t| t.icon_accent)
        };
        let bar = Rect::from_center_size(rect.center(), vec2(3.0, TILE + 2.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), colour);
        // A grip on top, so it reads as something to drag.
        ui.painter()
            .circle_filled(egui::pos2(rect.center().x, bar.top() + 2.0), 4.0, colour);
        if response.drag_started() {
            self.timeline.drag = Some(Drag::Marker);
        }
        let tip = if self.session.doc.rolled_back() {
            "Timeline marker: the items after it are not built.\nDrag it to roll the \
             design back or forward; new features go in here"
        } else {
            "Timeline marker: new features go in here.\nDrag it left to roll the design back"
        };
        response.on_hover_text(tip);
        rect
    }

    /// Drop handling: the marker rolls the design to the gap under the pointer, a tile
    /// moves there. A ghost line shows the gap while dragging.
    fn timeline_drag(&mut self, ui: &mut egui::Ui, items: &[Item]) {
        let Some(drag) = self.timeline.drag else {
            return;
        };
        let (pointer, released) = ui.input(|i| (i.pointer.latest_pos(), !i.pointer.any_down()));
        let Some(p) = pointer else {
            self.timeline.drag = None;
            return;
        };
        let gap = self.timeline_gap_at(p.x);
        if let Some(x) = self.timeline_gap_x(gap)
            && let Some(m) = self
                .timeline
                .marker_rect
                .or_else(|| self.timeline.slots.first().map(|s| s.rect))
        {
            let accent = self.look().c(|t| t.accent);
            let layer = egui::LayerId::new(egui::Order::Foreground, Id::unique("timeline ghost"));
            ui.ctx().layer_painter(layer).line_segment(
                [egui::pos2(x, m.top()), egui::pos2(x, m.bottom())],
                Stroke::new(2.0, accent),
            );
        }
        if !released {
            ui.ctx().request_repaint();
            return;
        }
        self.timeline.drag = None;
        match drag {
            Drag::Marker => {
                if gap != self.session.doc.marker() {
                    self.run("timeline.roll_to", json!({ "position": gap }));
                }
            }
            Drag::Item(id) => {
                // The dragged block: one item, or all of a group's.
                let block: Vec<u32> = match self.session.doc.group(id) {
                    Some(g) => g.items.clone(),
                    None => vec![id],
                };
                let Some(start) = items.iter().position(|i| block.first() == Some(&i.id)) else {
                    return;
                };
                let end = start + block.len();
                if (start..=end).contains(&gap) {
                    return;
                }
                let to = if gap > end { gap - block.len() } else { gap };
                self.run("timeline.move", json!({ "item": id, "to": to }));
            }
        }
    }

    /// A line under the tiles of each open group, so its members read as one.
    fn timeline_brackets(&self, ui: &egui::Ui, items: &[Item], colour: egui::Color32) {
        for g in &self.session.doc.timeline.groups {
            if self.timeline.collapsed.contains(&g.id) {
                continue;
            }
            let rects: Vec<Rect> = self
                .timeline
                .slots
                .iter()
                .filter(|s| items.get(s.start).is_some_and(|i| g.items.contains(&i.id)))
                .map(|s| s.rect)
                .collect();
            let (Some(a), Some(b)) = (rects.first(), rects.last()) else {
                continue;
            };
            let y = a.bottom() + 3.5;
            ui.painter().line_segment(
                [egui::pos2(a.left() - 30.0, y), egui::pos2(b.right(), y)],
                Stroke::new(1.5, colour),
            );
        }
    }

    /// A group's header tile: its name and a fold arrow. Click folds, double-click renames,
    /// drag moves the whole group.
    fn timeline_group_tile(
        &mut self,
        ui: &mut egui::Ui,
        g: &chamfera_doc::Group,
        folded: bool,
        faded: bool,
        items: &[Item],
    ) -> Rect {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let short: String = if g.name.chars().count() > 14 {
            g.name.chars().take(13).chain(['…']).collect()
        } else {
            g.name.clone()
        };
        let galley_w = ui
            .painter()
            .layout_no_wrap(
                short.clone(),
                FontId::proportional(widgets::T_SMALL),
                look.c(|t| t.text),
            )
            .size()
            .x;
        let (rect, _) = ui.allocate_exact_size(vec2(galley_w + 30.0, TILE), Sense::hover());
        let response = ui.interact(
            rect,
            Id::unique(("timeline group", g.id)),
            Sense::click_and_drag(),
        );
        let fill = if response.hovered() {
            look.c(|t| t.hover)
        } else {
            look.c(|t| t.panel)
        };
        let error = g
            .items
            .iter()
            .any(|i| items.iter().any(|x| x.id == *i && x.error.is_some()));
        let stroke = if error {
            Stroke::new(1.5, look.c(|t| t.error))
        } else {
            Stroke::new(1.0, look.c(|t| t.border))
        };
        let painter = ui.painter();
        // Two stacked cards: a group of items.
        let back = rect.translate(vec2(2.5, -2.5)).shrink2(vec2(1.5, 1.5));
        painter.rect(
            back,
            CornerRadius::same(widgets::R_SMALL),
            look.c(|t| t.panel_alt),
            Stroke::new(1.0, look.c(|t| t.border)),
            StrokeKind::Inside,
        );
        painter.rect(
            rect,
            CornerRadius::same(widgets::R_SMALL + 1),
            fill,
            stroke,
            StrokeKind::Inside,
        );
        let (mut b, mut a) = look.icon_colours(true);
        let mut text = look.c(|t| t.text);
        if faded {
            b = b.gamma_multiply(ROLLED_BACK_FADE);
            a = a.gamma_multiply(ROLLED_BACK_FADE);
            text = text.gamma_multiply(ROLLED_BACK_FADE + 0.2);
        }
        look.icons.paint(
            painter,
            if folded {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            },
            Rect::from_center_size(
                egui::pos2(rect.left() + 12.0, rect.center().y),
                Vec2::splat(14.0),
            ),
            b,
            a,
        );
        painter.text(
            egui::pos2(rect.left() + 22.0, rect.center().y),
            Align2::LEFT_CENTER,
            short,
            FontId::proportional(widgets::T_SMALL),
            text,
        );
        if response.hovered() {
            self.timeline.hovered = Some((g.id, rect));
        }
        if response.drag_started() {
            self.timeline.drag = Some(Drag::Item(g.id));
        }
        let response = response.on_hover_text(format!(
            "{}\nGroup of {} item{} · click to {} · double-click or F2 to rename",
            g.name,
            g.items.len(),
            if g.items.len() == 1 { "" } else { "s" },
            if folded { "open" } else { "fold" }
        ));
        let mut action: Option<&str> = if response.double_clicked() {
            Some("rename")
        } else if response.clicked() {
            Some("fold")
        } else {
            None
        };
        response.context_menu(|ui| {
            if widgets::menu_item(ui, look, Icon::Edit, "Rename…", true).clicked() {
                ui.close();
                action = Some("rename");
            }
            if widgets::menu_item(ui, look, Icon::Delete, label("timeline.ungroup"), true).clicked()
            {
                ui.close();
                action = Some("ungroup");
            }
        });
        match action {
            Some("rename") => self.start_rename(g.id, rect, true),
            Some("fold") => {
                if folded {
                    self.timeline.collapsed.remove(&g.id);
                } else {
                    self.timeline.collapsed.insert(g.id);
                }
            }
            Some("ungroup") => {
                self.run("timeline.ungroup", json!({ "group": g.id }));
            }
            _ => {}
        }
        rect
    }

    fn timeline_tile(&mut self, ui: &mut egui::Ui, item: &Item, faded: bool) -> Rect {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::hover());
        let response = ui.interact(
            rect,
            Id::unique(("timeline tile", item.id)),
            Sense::click_and_drag(),
        );
        let radius = CornerRadius::same(widgets::R_SMALL + 1);
        let selected = self.timeline.selected.contains(&item.id);
        let fill = if item.active || selected {
            look.c(|t| t.selected)
        } else if response.hovered() {
            look.c(|t| t.hover)
        } else {
            look.c(|t| t.panel_alt)
        };
        let stroke = if item.error.is_some() {
            Stroke::new(1.5, look.c(|t| t.error))
        } else if item.active || selected {
            Stroke::new(1.0, look.c(|t| t.accent))
        } else {
            Stroke::new(1.0, look.c(|t| t.border))
        };
        let painter = ui.painter();
        let (fill, stroke) = if faded {
            (
                fill.gamma_multiply(0.6),
                Stroke::new(stroke.width, stroke.color.gamma_multiply(0.6)),
            )
        } else {
            (fill, stroke)
        };
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        let (mut b, mut a) = look.icon_colours(true);
        if item.suppressed {
            b = b.gamma_multiply(0.35);
            a = a.gamma_multiply(0.35);
        }
        if faded {
            b = b.gamma_multiply(ROLLED_BACK_FADE);
            a = a.gamma_multiply(ROLLED_BACK_FADE);
        }
        look.icons.paint(
            painter,
            item.icon,
            Rect::from_center_size(rect.center(), Vec2::splat(20.0)),
            b,
            a,
        );
        if item.suppressed {
            // A slash across a suppressed feature.
            painter.line_segment(
                [
                    rect.left_bottom() + vec2(6.0, -6.0),
                    rect.right_top() + vec2(-6.0, 6.0),
                ],
                Stroke::new(1.5, look.c(|t| t.text_muted)),
            );
        }
        let badge = if item.error.is_some() {
            Some(look.c(|t| t.error))
        } else if item.warning.is_some() {
            Some(look.c(|t| t.warning))
        } else {
            None
        };
        if let Some(c) = badge {
            let centre = rect.right_top() + vec2(-3.0, 3.0);
            painter.circle_filled(centre, 4.5, look.c(|t| t.chrome));
            painter.circle_filled(centre, 3.5, c);
        }
        widgets::focus_ring(ui, look, &response, widgets::R_SMALL + 1);
        if response.hovered() {
            self.timeline.hovered = Some((item.id, rect));
        }
        if response.drag_started() {
            self.timeline.drag = Some(Drag::Item(item.id));
        }

        let mut tip = format!("{}\n{}", item.name, item.details);
        if !item.built {
            tip.push_str("\nAfter the timeline marker: not built");
        }
        if item.suppressed {
            tip.push_str("\nSuppressed: kept, but not built");
        }
        if let Some(e) = &item.error {
            tip.push_str(&format!("\nError: {e}"));
        }
        if let Some(r) = &item.broken {
            tip.push_str(&format!("\nBroken reference: {r}"));
        }
        if let Some(w) = &item.warning {
            tip.push_str(&format!("\nNot shown: {w}"));
        }
        let sketching = self.session.active_sketch.is_some();
        tip.push_str(if item.sketch {
            if item.active {
                "\nBeing edited"
            } else if sketching {
                "\nFinish the active sketch to edit this one"
            } else {
                "\nDouble-click to edit · F2 to rename · drag to move"
            }
        } else {
            "\nDouble-click to edit · F2 to rename · drag to move · right-click for more"
        });
        let response = response.on_hover_text(tip);
        if response.double_clicked() {
            self.edit_item(item);
        } else if response.clicked() {
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            let sel = &mut self.timeline.selected;
            if ctrl || shift {
                if let Some(i) = sel.iter().position(|x| *x == item.id) {
                    sel.remove(i);
                } else {
                    sel.push(item.id);
                }
            } else {
                *sel = vec![item.id];
            }
        }
        let mut action: Option<&str> = None;
        let look = self.look();
        let group_pick: Vec<u32> = if selected && self.timeline.selected.len() > 1 {
            self.timeline.selected.clone()
        } else {
            vec![item.id]
        };
        let grouped = item.group.is_some();
        response.context_menu(|ui| {
            let can_edit = !item.sketch || (!sketching && !item.active);
            if widgets::menu_item(ui, look, Icon::Edit, "Edit…", can_edit).clicked() {
                ui.close();
                action = Some("edit");
            }
            let rename = if item.sketch {
                label("sketch.rename")
            } else {
                label("feature.rename")
            };
            if widgets::menu_item(ui, look, Icon::Edit, rename, true).clicked() {
                ui.close();
                action = Some("rename");
            }
            if item.error.is_some()
                && widgets::menu_item(ui, look, Icon::Error, "Show Error", true).clicked()
            {
                ui.close();
                action = Some("error");
            }
            ui.separator();
            if widgets::menu_item(ui, look, Icon::Select, "Move Marker After This", !sketching)
                .clicked()
            {
                ui.close();
                action = Some("roll");
            }
            let text = if group_pick.len() > 1 {
                format!("{} {} Items", label("timeline.group"), group_pick.len())
            } else {
                label("timeline.group").to_owned()
            };
            if widgets::menu_item(ui, look, Icon::PatternRectangular, &text, !grouped).clicked() {
                ui.close();
                action = Some("group");
            }
            if item.sketch {
                ui.separator();
                if widgets::menu_item(ui, look, Icon::Delete, label("sketch.delete"), !item.active)
                    .clicked()
                {
                    ui.close();
                    action = Some("delete sketch");
                }
            }
            if !item.sketch {
                let text = if item.suppressed {
                    "Unsuppress"
                } else {
                    "Suppress"
                };
                let icon = if item.suppressed {
                    Icon::Eye
                } else {
                    Icon::EyeOff
                };
                if widgets::menu_item(ui, look, icon, text, true).clicked() {
                    ui.close();
                    action = Some("suppress");
                }
                ui.separator();
                if widgets::menu_item(ui, look, Icon::Delete, label("feature.delete"), true)
                    .clicked()
                {
                    ui.close();
                    action = Some("delete");
                }
            }
        });
        match action {
            Some("edit") => self.edit_item(item),
            Some("rename") => self.start_rename(item.id, rect, true),
            Some("error") => self.timeline.error_shown = Some(item.id),
            Some("roll") => {
                let at = self
                    .session
                    .doc
                    .timeline_order()
                    .iter()
                    .position(|x| *x == item.id);
                if let Some(k) = at {
                    self.run("timeline.roll_to", json!({ "position": k + 1 }));
                }
            }
            Some("group") => {
                if self
                    .run("timeline.group", json!({ "items": group_pick }))
                    .is_some()
                {
                    self.timeline.selected.clear();
                }
            }
            Some("suppress") => {
                self.run(
                    "feature.edit",
                    json!({ "feature": item.id, "suppressed": !item.suppressed }),
                );
            }
            Some("delete") => {
                self.run("feature.delete", json!({ "feature": item.id }));
            }
            Some("delete sketch") => {
                self.menu_act(crate::context_menu::Act::DeleteSketch(item.id));
            }
            _ => {}
        }
        rect
    }

    /// The error window of a failed feature ("Show Error" in its menu).
    pub(crate) fn timeline_error_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.timeline.error_shown else {
            return;
        };
        let error = self
            .session
            .feature_errors()
            .get(&id)
            .or_else(|| self.session.sketch_errors().get(&id))
            .cloned();
        let (Some(error), Some(name)) = (error, self.session.doc.item_name(id)) else {
            // Fixed, or gone.
            self.timeline.error_shown = None;
            return;
        };
        let broken = self.session.broken_references().get(&id).cloned();
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        const WIDTH: f32 = 340.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let mut close = false;
        egui::Window::new("Feature Error")
            .id(Id::unique("timeline error"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Error, &format!("{name} failed")) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(egui::RichText::new(&error).color(look.c(|t| t.text)));
                    if let Some(r) = &broken {
                        ui.add_space(widgets::S3);
                        widgets::field_label(ui, look, "Broken reference");
                        ui.label(egui::RichText::new(r).color(look.c(|t| t.error)));
                    }
                    ui.add_space(widgets::S3);
                    ui.label(
                        egui::RichText::new(
                            "Edit this feature, or the one it refers to, to fix it.",
                        )
                        .size(widgets::T_SMALL)
                        .color(look.c(|t| t.text_muted)),
                    );
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.timeline.error_shown = None;
        }
    }

    fn edit_item(&mut self, item: &Item) {
        if item.sketch {
            if self.session.active_sketch.is_none() {
                self.edit_sketch(item.id);
            }
            return;
        }
        self.edit_feature(item.id);
    }

    /// Opens the Edit Feature dialog for a feature.
    pub(crate) fn edit_feature(&mut self, id: u32) {
        let Ok(f) = self.session.doc.feature(id) else {
            return;
        };
        if let FeatureKind::Hole(h) = &f.kind {
            self.solid.hole = Some(crate::hole_ui::HoleTool::editing(id, h));
            return;
        }
        let (distance, extrude) = match &f.kind {
            FeatureKind::Extrude(e) => (e.distance.clone(), Some(ExtrudeForm::from_extrude(e))),
            FeatureKind::Revolve(r) => (r.angle.clone(), None),
            FeatureKind::Pattern(p) => (p.count.clone(), None),
            FeatureKind::Construct(_) => return self.edit_construct(id),
            FeatureKind::Blend(b) => (b.size.clone(), None),
            FeatureKind::Hole(h) => (h.diameter.clone(), None),
            FeatureKind::Mirror(m) => (crate::transform_ui::mirror_details(m), None),
            FeatureKind::Move(m) => (crate::transform_ui::move_details(m), None),
            FeatureKind::Shell(sh) => (sh.thickness.clone(), None),
            FeatureKind::OffsetFace(o) => (o.distance.clone(), None),
            FeatureKind::DeleteFace(_) => (crate::press_pull_ui::timeline_entry(&f.kind).1, None),
            FeatureKind::MoveFace(m) => (m.distance.clone(), None),
            FeatureKind::Sweep(w) => (format!("{} curves", w.path.len()), None),
            FeatureKind::Loft(l) => (format!("{} profiles", l.sections.len()), None),
        };
        self.timeline.editing = Some(FeatureEdit {
            feature: id,
            distance,
            extrude,
            suppressed: f.suppressed,
        });
    }

    /// The Edit Feature dialog, opened from the timeline.
    pub(crate) fn feature_dialog(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.timeline.editing.as_ref() else {
            return;
        };
        let Ok(feature) = self.session.doc.feature(edit.feature) else {
            self.timeline.editing = None;
            return;
        };
        let name = feature.name.clone();
        // Extrudes edit distance and direction, fillets and chamfers their size; the
        // others show their value only.
        let extrude = matches!(feature.kind, FeatureKind::Extrude(_));
        let shell = matches!(feature.kind, FeatureKind::Shell(_));
        // Offset and move face edit their distance in the same field.
        let face_edit = matches!(
            feature.kind,
            FeatureKind::OffsetFace(_) | FeatureKind::MoveFace(_)
        );
        let blend = shell || face_edit || matches!(feature.kind, FeatureKind::Blend(_));
        let revolve = !extrude && !blend;
        let value_label = match &feature.kind {
            FeatureKind::Revolve(_) => "Angle",
            FeatureKind::Pattern(_) => "Count",
            FeatureKind::Mirror(_) | FeatureKind::Move(_) => "Definition",
            FeatureKind::Shell(_) => "Thickness",
            FeatureKind::OffsetFace(_) | FeatureKind::MoveFace(_) => "Distance",
            FeatureKind::DeleteFace(_) => "Faces",
            FeatureKind::Sweep(_) => "Path",
            FeatureKind::Loft(_) => "Profiles",
            _ => "Size",
        };
        const WIDTH: f32 = 300.0;
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let Some(edit) = self.timeline.editing.as_mut() else {
            return;
        };
        let mut ok = false;
        let mut cancel = false;
        let icon = match feature.kind {
            FeatureKind::Sweep(_) => Icon::Sweep,
            FeatureKind::Loft(_) => Icon::Loft,
            _ if revolve => Icon::Revolve,
            _ => Icon::Extrude,
        };
        let title = format!("{}: {name}", label("feature.edit"));
        egui::Window::new("Edit Feature")
            .id(egui::Id::unique("feature dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .default_height(200.0)
            .fade_in(false)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, icon, &title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("feature grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            if revolve {
                                widgets::field_label(ui, look, value_label);
                                ui.label(&edit.distance);
                                ui.end_row();
                            } else if blend {
                                widgets::field_label(ui, look, value_label);
                                let r = ui.add_sized(
                                    [180.0, 26.0],
                                    egui::TextEdit::singleline(&mut edit.distance)
                                        .hint_text("2 mm")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    ok = true;
                                }
                                ui.end_row();
                            } else if let Some(form) = edit.extrude.as_mut()
                                && form.rows(ui, look)
                            {
                                ok = true;
                            }
                            widgets::field_label(ui, look, "Build");
                            let mut on = !edit.suppressed;
                            if ui
                                .checkbox(&mut on, "Build this feature")
                                .on_hover_text(
                                    "Off keeps the feature in the timeline without building it",
                                )
                                .changed()
                            {
                                edit.suppressed = !on;
                            }
                            ui.end_row();
                        });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "OK", true).clicked() {
                        ok = true;
                    }
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if ok {
            let mut params = json!({ "feature": edit.feature, "suppressed": edit.suppressed });
            if shell {
                params["thickness"] = json!(edit.distance.trim());
            } else if face_edit {
                params["distance"] = json!(edit.distance.trim());
            } else if blend {
                params["size"] = json!(edit.distance.trim());
            } else if let Some(form) = &edit.extrude
                && let (Some(p), serde_json::Value::Object(options)) =
                    (params.as_object_mut(), form.params())
            {
                p.extend(options);
            }
            if self.run("feature.edit", params).is_some() {
                self.timeline.editing = None;
                self.message = Some((format!("{name} updated"), false));
            }
        } else if cancel {
            self.timeline.editing = None;
        }
    }
}

/// The short line that joins neighbouring tiles.
fn connector(ui: &mut egui::Ui, colour: egui::Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(10.0, TILE), Sense::hover());
    ui.painter().hline(
        r.x_range(),
        r.center().y.round() + 0.5,
        Stroke::new(1.0, colour),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plate (sketch 1, extrude 2) with a boss on its top face (sketch 3, extrude 4).
    fn plate_with_boss() -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        let steps = [
            ("sketch.create", json!({ "plane": "xy" })),
            (
                "sketch.rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 80, "y2": 50 }),
            ),
            ("sketch.finish", json!({})),
            (
                "solid.extrude",
                json!({ "sketch": 1, "profiles": [{ "x": 5, "y": 5 }], "distance": "10 mm" }),
            ),
            (
                "sketch.create_on_face",
                json!({ "feature": 2, "face": "top" }),
            ),
            (
                "sketch.rectangle",
                json!({ "x1": 25, "y1": 15, "x2": 55, "y2": 35 }),
            ),
            ("sketch.finish", json!({})),
            (
                "solid.extrude",
                json!({ "sketch": 3, "profiles": [{ "x": 40, "y": 25 }], "distance": "15 mm" }),
            ),
        ];
        for (id, p) in steps {
            assert!(app.run(id, p).is_some(), "{id}");
        }
        app
    }

    #[test]
    fn the_timeline_lists_sketches_and_features_in_build_order() {
        let app = plate_with_boss();
        let items = app.timeline_items();
        let ids: Vec<u32> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        let icons: Vec<Icon> = items.iter().map(|i| i.icon).collect();
        assert_eq!(
            icons,
            [Icon::Sketch, Icon::Extrude, Icon::Sketch, Icon::Extrude]
        );
        assert!(items.iter().all(|i| i.error.is_none() && !i.suppressed));
        assert!(items[1].details.contains("10 mm"), "{}", items[1].details);
    }

    #[test]
    fn editing_from_the_timeline_goes_through_the_feature_and_sketch_commands() {
        let mut app = plate_with_boss();
        let items = app.timeline_items();
        // Double-click on the boss extrude opens its dialog with its current values.
        app.edit_item(&items[3]);
        let edit = app.timeline.editing.as_ref().unwrap();
        assert_eq!(
            (
                edit.feature,
                edit.extrude.as_ref().unwrap().distance.as_str()
            ),
            (4, "15 mm")
        );
        // Suppressing it (the context menu's command) shows it faded, with one solid left.
        app.run("feature.edit", json!({ "feature": 4, "suppressed": true }));
        assert!(app.timeline_items()[3].suppressed);
        assert_eq!(app.session.solids().len(), 1);
        // Double-click on a sketch edits it.
        app.timeline.editing = None;
        app.edit_item(&items[0]);
        assert_eq!(app.session.active_sketch, Some(1));
        assert!(app.timeline_items()[0].active);
    }
}

/// The timeline driven by real pointer and key events through the whole UI.
#[cfg(test)]
mod input_tests {
    use super::*;
    use eframe::App;
    use egui::{Event, Modifiers, PointerButton, Pos2};

    struct Driver {
        app: ChamferaApp,
        ctx: egui::Context,
        t: f64,
    }

    impl Driver {
        fn new(app: ChamferaApp) -> Driver {
            let mut d = Driver {
                app,
                ctx: egui::Context::default(),
                t: 0.0,
            };
            for _ in 0..4 {
                d.frame(vec![]);
            }
            d
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
                time: Some(self.t),
                events,
                ..Default::default()
            };
            let mut f = eframe::Frame::_new_kittest();
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(raw, |ui| app.ui(ui, &mut f));
            out.textures_delta.clear();
            self.t += 0.05;
        }

        fn button(&mut self, at: Pos2, pressed: bool) {
            self.frame(vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }

        /// Press at `from`, move in steps to `to`, release there.
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.button(from, true);
            for k in 1..=6 {
                let p = from + (to - from) * (k as f32 / 6.0);
                self.frame(vec![Event::PointerMoved(p)]);
            }
            self.button(to, false);
            self.frame(vec![]);
            self.frame(vec![]);
        }

        fn key(&mut self, key: egui::Key) {
            for pressed in [true, false] {
                self.frame(vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]);
            }
        }

        /// The centre of the gap between timeline items `g - 1` and `g`.
        fn gap(&self, g: usize) -> Pos2 {
            let x = self.app.timeline_gap_x(g).unwrap();
            let y = self.app.timeline.slots[0].rect.center().y;
            egui::pos2(x, y)
        }
    }

    fn volume(app: &ChamferaApp) -> f64 {
        app.session
            .solids()
            .iter()
            .map(|s| s.body.volume().unwrap())
            .sum()
    }

    /// Plate 50 x 30 x 10 = 15000 (sketch 1, extrude 2) with a hole r = 3 cut through it
    /// (sketch 3 on the XY plane, extrude 4): 15000 - 90π.
    fn plate_with_hole() -> ChamferaApp {
        let mut app = ChamferaApp::headless();
        let steps = [
            ("sketch.create", json!({ "plane": "xy" })),
            (
                "sketch.rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
            ),
            ("sketch.finish", json!({})),
            (
                "solid.extrude",
                json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
            ),
            ("sketch.create", json!({ "plane": "xy" })),
            ("sketch.circle", json!({ "cx": 25, "cy": 15, "radius": 3 })),
            ("sketch.finish", json!({})),
            (
                "solid.extrude",
                json!({ "sketch": 3, "profiles": [{ "x": 25, "y": 15 }], "distance": 10,
                        "operation": "cut" }),
            ),
        ];
        for (id, p) in steps {
            assert!(app.run(id, p).is_some(), "{id}");
        }
        app
    }

    #[test]
    fn dragging_the_marker_rolls_the_design_back_and_forward() {
        let mut d = Driver::new(plate_with_hole());
        let expected = 15_000.0 - 90.0 * std::f64::consts::PI;
        assert!((volume(&d.app) - expected).abs() < 1e-6);
        let marker = d.app.timeline.marker_rect.unwrap().center();
        // Drag the marker from the end to between the plate (item 2) and the hole sketch.
        let to = d.gap(2);
        d.drag(marker, to);
        assert_eq!(d.app.session.doc.marker(), 2);
        assert!(
            (volume(&d.app) - 15_000.0).abs() < 1e-6,
            "{}",
            volume(&d.app)
        );
        let built: Vec<bool> = d.app.timeline_items().iter().map(|i| i.built).collect();
        assert_eq!(built, [true, true, false, false]);
        // The marker is now drawn in that gap; drag it back to the end.
        let marker = d.app.timeline.marker_rect.unwrap().center();
        assert!((marker.x - to.x).abs() < TILE, "{marker:?} vs {to:?}");
        let end = d.app.timeline.slots.last().unwrap().rect.right_center() + vec2(20.0, 0.0);
        d.drag(marker, end);
        assert!(!d.app.session.doc.rolled_back());
        assert!((volume(&d.app) - expected).abs() < 1e-6);
    }

    #[test]
    fn dragging_a_tile_moves_it_and_a_refused_move_changes_nothing() {
        let mut d = Driver::new(plate_with_hole());
        // The hole sketch (item 3, uses nothing) dragged before the plate's extrude.
        let tile = d.app.timeline.slots[2].rect.center();
        let to = d.gap(1);
        d.drag(tile, to);
        assert_eq!(d.app.session.doc.timeline_order(), [1, 3, 2, 4]);
        // The cut dragged before the plate it cuts is refused: nothing to cut there.
        let tile = d.app.timeline.slots[3].rect.center();
        let to = d.gap(2);
        d.drag(tile, to);
        assert_eq!(d.app.session.doc.timeline_order(), [1, 3, 2, 4]);
        let (msg, error) = d.app.message.clone().unwrap();
        assert!(error && msg.contains("nothing to cut"), "{msg}");
    }

    #[test]
    fn f2_on_a_tile_renames_it() {
        let mut d = Driver::new(plate_with_hole());
        let tile = d.app.timeline.slots[1].rect.center();
        d.frame(vec![Event::PointerMoved(tile)]);
        d.key(egui::Key::F2);
        assert_eq!(d.app.timeline.renaming.as_ref().map(|r| r.item), Some(2));
        // Select all of the old name, type the new one, Enter.
        if let Some(r) = d.app.timeline.renaming.as_mut() {
            r.text.clear();
        }
        d.frame(vec![Event::Text("Plate".into())]);
        d.key(egui::Key::Enter);
        assert!(d.app.timeline.renaming.is_none());
        assert_eq!(d.app.session.doc.feature(2).unwrap().name, "Plate");
    }
}

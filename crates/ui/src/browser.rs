//! The Browser: the design as a tree with collapsible sections (Origin, Sketches, Bodies),
//! eye toggles for bodies and the active sketch highlighted. Acts through `sketch.*` and
//! `body.*` commands.

use crate::context_menu::{Act, Entry, RenameTarget, show_entries};
use crate::icons::Icon;
use crate::widgets::{self, Look, ROW_HEIGHT, T_BODY};
use crate::{ChamferaApp, label};
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Ui, Vec2, vec2};
use serde_json::json;
use std::collections::BTreeSet;

/// The Browser's own state: which sections are folded.
pub struct BrowserUi {
    folded: BTreeSet<&'static str>,
}

impl Default for BrowserUi {
    fn default() -> Self {
        BrowserUi {
            // The origin planes are rarely needed: folded at first.
            folded: ["origin"].into_iter().collect(),
        }
    }
}

/// How a tree row looks.
#[derive(Default)]
struct Row<'a> {
    depth: usize,
    /// `Some(open)` for a section that folds.
    fold: Option<bool>,
    icon: Option<Icon>,
    brand: bool,
    text: &'a str,
    /// Small muted text after the name (a count, a state).
    note: Option<&'a str>,
    active: bool,
    selected: bool,
    dimmed: bool,
    error: bool,
    /// An eye toggle on the right: `Some(visible)`.
    eye: Option<bool>,
}

struct RowResponse {
    row: egui::Response,
    eye: Option<egui::Response>,
}

fn tree_row(ui: &mut Ui, look: Look, id: egui::Id, row: &Row) -> RowResponse {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(widgets::R_SMALL);
    if row.active || row.selected {
        painter.rect_filled(rect, radius, look.c(|t| t.selected));
    } else if response.hovered() {
        painter.rect_filled(rect, radius, look.c(|t| t.hover));
    }
    if row.active {
        let bar = Rect::from_min_size(rect.min + vec2(0.0, 5.0), vec2(2.5, rect.height() - 10.0));
        painter.rect_filled(bar, CornerRadius::same(1), look.c(|t| t.accent));
    }
    widgets::focus_ring(ui, look, &response, widgets::R_SMALL);
    let mut x = rect.min.x + 4.0 + row.depth as f32 * 14.0;
    let cy = rect.center().y;
    if let Some(open) = row.fold {
        let chevron = Rect::from_center_size(egui::pos2(x + 7.0, cy), Vec2::splat(12.0));
        let muted = look.c(|t| t.text_muted);
        let icon = if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        look.icons.paint(painter, icon, chevron, muted, muted);
    }
    x += 16.0;
    if row.brand {
        look.icons.paint_brand(
            painter,
            Rect::from_center_size(egui::pos2(x + 8.0, cy), Vec2::splat(16.0)),
        );
        x += 22.0;
    } else if let Some(icon) = row.icon {
        let r = Rect::from_center_size(egui::pos2(x + 8.0, cy), Vec2::splat(16.0));
        let (mut b, mut a) = look.icon_colours(!row.dimmed);
        if row.error {
            b = look.c(|t| t.error);
            a = b;
        }
        look.icons.paint(painter, icon, r, b, a);
        x += 22.0;
    }
    let text_colour = if row.error {
        look.c(|t| t.error)
    } else if row.dimmed {
        look.c(|t| t.text_muted)
    } else {
        look.c(|t| t.text)
    };
    let right = rect.max.x - if row.eye.is_some() { 30.0 } else { 6.0 };
    let clip = Rect::from_min_max(egui::pos2(x, rect.min.y), egui::pos2(right, rect.max.y));
    let font = FontId::proportional(T_BODY);
    let galley = painter.layout_no_wrap(row.text.to_owned(), font, text_colour);
    let text_end = x + galley.size().x;
    painter.with_clip_rect(clip).galley(
        egui::pos2(x, cy - galley.size().y * 0.5),
        galley,
        text_colour,
    );
    if let Some(note) = row.note {
        painter.with_clip_rect(clip).text(
            egui::pos2(text_end + 6.0, cy),
            Align2::LEFT_CENTER,
            note,
            FontId::proportional(widgets::T_SMALL),
            look.c(|t| t.text_muted),
        );
    }
    let eye = row.eye.map(|visible| {
        let r = Rect::from_center_size(egui::pos2(rect.max.x - 15.0, cy), Vec2::splat(22.0));
        let eye = ui.interact(r, id.with("eye"), Sense::click());
        if eye.hovered() {
            ui.painter().rect_filled(r, radius, look.c(|t| t.hover));
        }
        widgets::focus_ring(ui, look, &eye, widgets::R_SMALL);
        let icon = if visible { Icon::Eye } else { Icon::EyeOff };
        let (b, a) = if visible {
            look.icon_colours(true)
        } else {
            let m = look.c(|t| t.text_muted);
            (m, m)
        };
        look.icons.paint(
            ui.painter(),
            icon,
            Rect::from_center_size(r.center(), Vec2::splat(16.0)),
            b,
            a,
        );
        eye
    });
    RowResponse { row: response, eye }
}

impl ChamferaApp {
    fn section(
        &mut self,
        ui: &mut Ui,
        key: &'static str,
        icon: Icon,
        text: &str,
        count: usize,
    ) -> bool {
        let open = !self.browser.folded.contains(key);
        let note = format!("{count}");
        let r = tree_row(
            ui,
            self.look(),
            ui.scope_id().with(("section", key)),
            &Row {
                depth: 1,
                fold: Some(open),
                icon: Some(icon),
                text,
                note: (count > 0).then_some(note.as_str()),
                ..Row::default()
            },
        );
        if r.row.clicked() {
            if open {
                self.browser.folded.insert(key);
            } else {
                self.browser.folded.remove(key);
            }
        }
        open
    }

    /// The Browser panel.
    pub(crate) fn browser(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            widgets::caption(ui, self.look(), "BROWSER");
        });
        ui.add_space(widgets::S2);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.browser_tree(ui));
    }

    fn browser_tree(&mut self, ui: &mut Ui) {
        let sketching = self.session.active_sketch.is_some();
        let name = self.design_name();
        let root_open = !self.browser.folded.contains("root");
        let root = tree_row(
            ui,
            self.look(),
            ui.scope_id().with("root"),
            &Row {
                fold: Some(root_open),
                brand: true,
                text: &name,
                note: self.session.has_unsaved_changes().then_some("unsaved"),
                ..Row::default()
            },
        );
        if root.row.clicked() {
            if root_open {
                self.browser.folded.insert("root");
            } else {
                self.browser.folded.remove("root");
            }
        }
        if !root_open {
            return;
        }

        // Origin: the three planes; double-click one to sketch on it.
        if self.section(ui, "origin", Icon::Origin, "Origin", 0) {
            for (plane, text, view) in crate::sketch_ui::PLANES {
                let r = tree_row(
                    ui,
                    self.look(),
                    ui.scope_id().with(("plane", plane)),
                    &Row {
                        depth: 2,
                        icon: Some(Icon::Plane),
                        text,
                        ..Row::default()
                    },
                );
                let r = r.row.on_hover_text(if sketching {
                    "Finish the sketch to start another"
                } else {
                    "Double-click to sketch on this plane"
                });
                if r.double_clicked() && !sketching {
                    self.create_sketch(plane, view);
                }
            }
        }

        let sketches: Vec<(u32, String, bool)> = self
            .session
            .doc
            .sketches
            .iter()
            .map(|s| (s.id, s.name.clone(), s.attached.is_some()))
            .collect();
        if self.section(ui, "sketches", Icon::Sketch, "Sketches", sketches.len()) {
            if sketches.is_empty() {
                self.empty_note(ui, "Create Sketch adds one here");
            }
            for (id, name, on_face) in sketches {
                let active = self.session.active_sketch == Some(id);
                let r = tree_row(
                    ui,
                    self.look(),
                    ui.scope_id().with(("sketch", id)),
                    &Row {
                        depth: 2,
                        icon: Some(Icon::Sketch),
                        text: &name,
                        note: if active {
                            Some("editing")
                        } else if on_face {
                            Some("on a face")
                        } else {
                            None
                        },
                        active,
                        ..Row::default()
                    },
                );
                let can_edit = !active && !sketching;
                let row = r.row.on_hover_text(if can_edit {
                    "Double-click to edit this sketch"
                } else if active {
                    "Being edited: Finish Sketch in the ribbon when done"
                } else {
                    "Finish the active sketch to edit this one"
                });
                if can_edit && row.double_clicked() {
                    self.edit_sketch(id);
                }
                let entries = vec![
                    Entry::Item {
                        text: label("sketch.edit").into(),
                        icon: Icon::Edit,
                        why_not: (!can_edit).then(|| "Finish the active sketch first".into()),
                        act: Act::EditSketch(id),
                    },
                    Entry::Item {
                        text: label("sketch.rename").into(),
                        icon: Icon::Edit,
                        why_not: None,
                        act: Act::Rename(RenameTarget::Sketch(id)),
                    },
                    Entry::Separator,
                    Entry::Item {
                        text: label("sketch.delete").into(),
                        icon: Icon::Delete,
                        why_not: active.then(|| "Finish the sketch first".into()),
                        act: Act::DeleteSketch(id),
                    },
                ];
                self.row_menu(&row, &entries);
            }
        }

        let bodies: Vec<(u32, String, bool)> = self
            .session
            .scene
            .bodies
            .iter()
            .map(|b| (b.id, b.name.clone(), b.visible))
            .collect();
        let selected = self.session.scene.selection.map(|h| h.body);
        if self.section(ui, "bodies", Icon::Body, "Bodies", bodies.len()) {
            if bodies.is_empty() {
                self.empty_note(ui, "Extrude a closed profile to make one");
            }
            for (id, name, visible) in bodies {
                let r = tree_row(
                    ui,
                    self.look(),
                    ui.scope_id().with(("body", id)),
                    &Row {
                        depth: 2,
                        icon: Some(Icon::Body),
                        text: &name,
                        selected: selected == Some(id),
                        dimmed: !visible,
                        eye: Some(visible),
                        ..Row::default()
                    },
                );
                let entries = self.body_entries(id, None, true);
                self.row_menu(&r.row, &entries);
                if let Some(eye) = r.eye {
                    let tip = if visible { "Hide" } else { "Show" };
                    if eye
                        .on_hover_text(format!("{tip}  ({})", label("body.visibility")))
                        .clicked()
                    {
                        self.run(
                            "body.visibility",
                            json!({ "body": id, "visible": !visible }),
                        );
                    }
                }
            }
        }
    }

    /// A right-click menu on a tree row, with the same items as the viewport's.
    fn row_menu(&mut self, row: &egui::Response, entries: &[Entry]) {
        let look = self.look();
        let mut chosen = None;
        row.context_menu(|ui| {
            chosen = show_entries(ui, look, entries, &mut Vec::new());
            if chosen.is_some() {
                ui.close();
            }
        });
        if let Some(act) = chosen {
            self.menu_act(act);
        }
    }

    fn empty_note(&self, ui: &mut Ui, text: &str) {
        ui.horizontal(|ui| {
            ui.add_space(4.0 + 2.0 * 14.0 + 16.0);
            ui.label(
                egui::RichText::new(text)
                    .size(widgets::T_SMALL)
                    .color(self.look().c(|t| t.text_muted)),
            );
        });
        ui.add_space(widgets::S2);
    }

    /// Opens a finished sketch for editing again.
    pub(crate) fn edit_sketch(&mut self, id: u32) {
        if self
            .run_sketch("sketch.edit", json!({ "sketch": id }))
            .is_some()
        {
            self.sketch.tool = Some(crate::sketch_ui::Tool::Select);
        }
    }
}

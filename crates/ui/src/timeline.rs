//! The timeline at the bottom: one tile per sketch and feature in build order. Suppressed
//! features are faded, failed ones marked; hover for details, double-click to edit, right-
//! click for more. Acts through `sketch.edit`, `feature.edit` and `feature.delete`.

use crate::icons::Icon;
use crate::solid_ui::{direction_key, direction_name};
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::{ExtrudeDirection, FeatureKind};
use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, Vec2, vec2};
use serde_json::json;

/// A feature being edited in the Edit Feature dialog.
pub struct FeatureEdit {
    feature: u32,
    distance: String,
    direction: ExtrudeDirection,
    suppressed: bool,
}

#[derive(Default)]
pub struct TimelineUi {
    pub editing: Option<FeatureEdit>,
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
    error: Option<String>,
    warning: Option<String>,
    details: String,
}

const TILE: f32 = 30.0;

impl ChamferaApp {
    fn timeline_items(&self) -> Vec<Item> {
        let doc = &self.session.doc;
        let sketch_name = |id: u32| {
            doc.sketch(id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|_| format!("sketch {id}"))
        };
        let mut items: Vec<Item> = doc
            .sketches
            .iter()
            .map(|s| {
                let profiles = s.sketch.profiles().len();
                let place = if s.attached.is_some() {
                    "on a face"
                } else {
                    "on an origin plane"
                };
                Item {
                    id: s.id,
                    name: s.name.clone(),
                    icon: Icon::Sketch,
                    sketch: true,
                    suppressed: false,
                    active: self.session.active_sketch == Some(s.id),
                    error: None,
                    warning: None,
                    details: format!(
                        "Sketch {place} · {profiles} closed profile{}",
                        if profiles == 1 { "" } else { "s" }
                    ),
                }
            })
            .collect();
        items.extend(doc.features.iter().map(|f| {
            let (icon, details) = match &f.kind {
                FeatureKind::Extrude(e) => (
                    Icon::Extrude,
                    format!(
                        "Extrude {} · {} · from {}",
                        e.distance,
                        direction_name(e.direction).to_lowercase(),
                        sketch_name(e.sketch)
                    ),
                ),
                FeatureKind::Revolve(r) => (
                    Icon::Revolve,
                    format!("Revolve {} · from {}", r.angle, sketch_name(r.sketch)),
                ),
                FeatureKind::Pattern(p) => (
                    Icon::PatternRectangular,
                    format!("Pattern × {} of feature {}", p.count, p.source),
                ),
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
            };
            Item {
                id: f.id,
                name: f.name.clone(),
                icon,
                sketch: false,
                suppressed: f.suppressed,
                active: false,
                error: self.session.feature_errors().get(&f.id).cloned(),
                warning: self.session.display_errors().get(&f.id).cloned(),
                details,
            }
        }));
        // Sketches and features share one id counter, so ids are the build order.
        items.sort_by_key(|i| i.id);
        items
    }

    /// The timeline bar.
    pub(crate) fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let items = self.timeline_items();
        let border = self.look().c(|t| t.border);
        let muted = self.look().c(|t| t.text_muted);
        let marker = self.look().c(|t| t.icon_accent);
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
                        for (k, item) in items.iter().enumerate() {
                            if k > 0 {
                                connector(ui, border);
                            }
                            self.timeline_tile(ui, item);
                        }
                        // The end marker: where the next feature goes.
                        ui.add_space(widgets::S3);
                        let (r, _) = ui.allocate_exact_size(vec2(4.0, TILE), Sense::hover());
                        let bar = Rect::from_center_size(r.center(), vec2(3.0, TILE - 4.0));
                        ui.painter().rect_filled(bar, CornerRadius::same(2), marker);
                    });
                });
        });
    }

    fn timeline_tile(&mut self, ui: &mut egui::Ui, item: &Item) {
        let look = self.look();
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
        let radius = CornerRadius::same(widgets::R_SMALL + 1);
        let fill = if item.active {
            look.c(|t| t.selected)
        } else if response.hovered() {
            look.c(|t| t.hover)
        } else {
            look.c(|t| t.panel_alt)
        };
        let stroke = if item.error.is_some() {
            Stroke::new(1.5, look.c(|t| t.error))
        } else if item.active {
            Stroke::new(1.0, look.c(|t| t.accent))
        } else {
            Stroke::new(1.0, look.c(|t| t.border))
        };
        let painter = ui.painter();
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        let (mut b, mut a) = look.icon_colours(true);
        if item.suppressed {
            b = b.gamma_multiply(0.35);
            a = a.gamma_multiply(0.35);
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

        let mut tip = format!("{}\n{}", item.name, item.details);
        if item.suppressed {
            tip.push_str("\nSuppressed: kept, but not built");
        }
        if let Some(e) = &item.error {
            tip.push_str(&format!("\nError: {e}"));
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
                "\nDouble-click to edit"
            }
        } else {
            "\nDouble-click to edit · right-click for more"
        });
        let response = response.on_hover_text(tip);
        if response.double_clicked() {
            self.edit_item(item);
        }
        let mut action: Option<&str> = None;
        let look = self.look();
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
            Some("rename") => {
                let target = if item.sketch {
                    crate::context_menu::RenameTarget::Sketch(item.id)
                } else {
                    crate::context_menu::RenameTarget::Feature(item.id)
                };
                self.menu_act(crate::context_menu::Act::Rename(target));
            }
            _ => {}
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
        let (distance, direction) = match &f.kind {
            FeatureKind::Extrude(e) => (e.distance.clone(), e.direction),
            FeatureKind::Revolve(r) => (r.angle.clone(), ExtrudeDirection::Normal),
            FeatureKind::Pattern(p) => (p.count.clone(), ExtrudeDirection::Normal),
            FeatureKind::Blend(b) => (b.size.clone(), ExtrudeDirection::Normal),
        };
        self.timeline.editing = Some(FeatureEdit {
            feature: id,
            distance,
            direction,
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
        let blend = matches!(feature.kind, FeatureKind::Blend(_));
        let revolve = !extrude && !blend;
        let value_label = match &feature.kind {
            FeatureKind::Revolve(_) => "Angle",
            FeatureKind::Pattern(_) => "Count",
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
        let icon = if revolve {
            Icon::Revolve
        } else {
            Icon::Extrude
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
                            } else {
                                widgets::field_label(ui, look, "Distance");
                                let r = ui.add_sized(
                                    [180.0, 26.0],
                                    egui::TextEdit::singleline(&mut edit.distance)
                                        .hint_text("10 mm or thickness * 2")
                                        .align(egui::Align2::LEFT_CENTER),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    ok = true;
                                }
                                ui.end_row();
                                widgets::field_label(ui, look, "Direction");
                                egui::ComboBox::from_id_salt("feature direction")
                                    .selected_text(direction_name(edit.direction))
                                    .width(180.0)
                                    .show_ui(ui, |ui| {
                                        for d in [
                                            ExtrudeDirection::Normal,
                                            ExtrudeDirection::Reversed,
                                            ExtrudeDirection::Symmetric,
                                        ] {
                                            ui.selectable_value(
                                                &mut edit.direction,
                                                d,
                                                direction_name(d),
                                            );
                                        }
                                    });
                                ui.end_row();
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
            if blend {
                params["size"] = json!(edit.distance.trim());
            } else if extrude {
                params["distance"] = json!(edit.distance.trim());
                params["direction"] = json!(direction_key(edit.direction));
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
        assert_eq!((edit.feature, edit.distance.as_str()), (4, "15 mm"));
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

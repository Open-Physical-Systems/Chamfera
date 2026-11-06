//! The window's frame around the viewport: the app bar (menus, save, undo and redo, the
//! design name), the ribbon with its workspace tabs, and the status bar.

use crate::icons::Icon;
use crate::widgets::{self, S2, S3, T_SMALL};
use crate::{ChamferaApp, ThemeChoice, label};
use egui::{Align, CornerRadius, FontId, Layout, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::json;

impl ChamferaApp {
    /// The design's file name, or "Untitled".
    pub(crate) fn design_name(&self) -> String {
        self.session
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(
                || "Untitled".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            )
    }

    /// The top bar: brand, File and View menus, save, undo, redo, and the design's name.
    pub(crate) fn app_bar(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            // Menu buttons are flat until the pointer is over them.
            let w = &mut ui.style_mut().visuals.widgets.inactive;
            w.weak_bg_fill = egui::Color32::TRANSPARENT;
            w.bg_stroke = Stroke::NONE;
            ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
            let (brand, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            self.icons.paint_brand(ui.painter(), brand);
            ui.add_space(S2);
            ui.menu_button("File", |ui| self.file_menu(ui));
            ui.menu_button("View", |ui| self.view_menu(ui));
            ui.menu_button("Help", |ui| self.help_menu(ui));
            widgets::bar_divider(ui, self.look(), 20.0);
            let look = self.look();
            let desktop = cfg!(not(target_arch = "wasm32"));
            if widgets::icon_button(ui, look, Icon::Save, 30.0, 20.0, desktop, false)
                .on_hover_text("Save the design  (Ctrl+S)")
                .clicked()
            {
                self.save(false);
            }
            let look = self.look();
            let can_undo = self.session.can_undo();
            if widgets::icon_button(ui, look, Icon::Undo, 30.0, 20.0, can_undo, false)
                .on_hover_text(format!("{}  (Ctrl+Z)", label("edit.undo")))
                .clicked()
            {
                self.run("edit.undo", json!({}));
                self.refresh_sketch_status();
            }
            let look = self.look();
            let can_redo = self.session.can_redo();
            if widgets::icon_button(ui, look, Icon::Redo, 30.0, 20.0, can_redo, false)
                .on_hover_text(format!("{}  (Ctrl+Shift+Z)", label("edit.redo")))
                .clicked()
            {
                self.run("edit.redo", json!({}));
                self.refresh_sketch_status();
            }
        });
        // The design's name, centred in the bar, with a dot while changes are unsaved.
        let look = self.look();
        let name = self.design_name();
        let unsaved = self.session.has_unsaved_changes();
        let galley = ui.painter().layout_no_wrap(
            name,
            FontId::proportional(widgets::T_BODY),
            look.c(|t| t.text),
        );
        let dot = if unsaved { 14.0 } else { 0.0 };
        let width = galley.size().x + dot;
        let pos = egui::pos2(
            full.center().x - width * 0.5,
            full.center().y - galley.size().y * 0.5,
        );
        let text_rect = Rect::from_min_size(pos, vec2(width, galley.size().y));
        ui.painter().galley(pos, galley, look.c(|t| t.text));
        if unsaved {
            ui.painter().circle_filled(
                egui::pos2(text_rect.max.x - 4.0, full.center().y + 0.5),
                3.5,
                look.c(|t| t.icon_accent),
            );
            let hint = ui.interact(text_rect, ui.scope_id().with("unsaved dot"), Sense::hover());
            hint.on_hover_text("Unsaved changes");
        }
    }

    /// The View menu: home, fit, standard views, projection and theme.
    pub(crate) fn view_menu(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        ui.set_min_width(220.0);
        let shortcut = |ui: &egui::Ui, r: &egui::Response, keys: &str| {
            ui.painter().text(
                egui::pos2(r.rect.max.x - 8.0, r.rect.center().y),
                egui::Align2::RIGHT_CENTER,
                keys,
                FontId::proportional(T_SMALL),
                look.c(|t| t.text_muted),
            );
        };
        let r = widgets::menu_item(ui, look, Icon::Home, label("view.home"), true);
        shortcut(ui, &r, "H");
        let home = r.clicked();
        let r = widgets::menu_item(ui, look, Icon::Fit, label("view.fit"), true);
        shortcut(ui, &r, "F");
        let fit = r.clicked();
        ui.separator();
        let mut view = None;
        for (name, text, icon) in [
            ("front", "Front", Icon::ViewFront),
            ("back", "Back", Icon::ViewFront),
            ("left", "Left", Icon::ViewRight),
            ("right", "Right", Icon::ViewRight),
            ("top", "Top", Icon::ViewTop),
            ("bottom", "Bottom", Icon::ViewTop),
            ("iso", "Isometric", Icon::ViewIso),
        ] {
            if widgets::menu_item(ui, look, icon, text, true).clicked() {
                view = Some(name);
            }
        }
        if home {
            ui.close();
            self.run("view.home", self.viewport_params());
        }
        if fit {
            ui.close();
            self.run("view.fit", self.viewport_params());
        }
        if let Some(name) = view {
            ui.close();
            self.standard_view(name);
        }
        ui.separator();
        let ortho = self.session.camera.projection == chamfera_render::Projection::Orthographic;
        let mut o = ortho;
        if ui.checkbox(&mut o, "Orthographic").changed() {
            let mode = if o { "orthographic" } else { "perspective" };
            self.run("view.projection", json!({ "mode": mode }));
        }
        ui.separator();
        widgets::caption(ui, self.look(), "THEME");
        for (c, text) in [
            (ThemeChoice::System, "Follow system"),
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
        ] {
            if ui.radio(self.theme_choice == c, text).clicked() {
                self.theme_choice = c;
                self.styled_for = None;
            }
        }
    }

    /// The ribbon: workspace tabs, then the tools of the active one.
    pub(crate) fn ribbon(&mut self, ui: &mut egui::Ui) {
        let sketching = self.session.active_sketch.is_some();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            self.tab(ui, "SOLID", !sketching, !sketching)
                .on_hover_text(if sketching {
                    "Finish the sketch to use the solid tools"
                } else {
                    "Solid modelling: sketches, extrudes and revolves"
                });
            if sketching {
                self.tab(ui, "SKETCH", true, true)
                    .on_hover_text("Editing a sketch: draw, constrain and dimension");
            }
        });
        let look = self.look();
        let r = ui.min_rect();
        ui.painter().hline(
            r.x_range(),
            r.max.y.round() + 0.5,
            Stroke::new(1.0, look.c(|t| t.border)),
        );
        ui.add_space(4.0);
        egui::ScrollArea::horizontal()
            .id_salt("ribbon tools")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if sketching {
                        self.sketch_toolbar(ui);
                    } else {
                        self.solid_toolbar(ui);
                    }
                });
            });
    }

    /// A workspace tab: caps text, the accent underline when active.
    fn tab(&self, ui: &mut egui::Ui, text: &str, active: bool, enabled: bool) -> egui::Response {
        let look = self.look();
        let colour = if active {
            look.c(|t| t.text)
        } else {
            look.c(|t| t.text_muted)
        };
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_owned(), FontId::proportional(T_SMALL), colour);
        let size = vec2(galley.size().x + 24.0, widgets::TAB_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(
            size,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if response.hovered() && !active && enabled {
            ui.painter().rect_filled(
                rect.shrink2(vec2(0.0, 3.0)),
                CornerRadius::same(widgets::R_SMALL),
                look.c(|t| t.hover),
            );
        }
        ui.painter()
            .galley(rect.center() - galley.size() * 0.5, galley, colour);
        if active {
            let bar = Rect::from_min_max(
                egui::pos2(rect.min.x + 8.0, rect.max.y - 2.0),
                egui::pos2(rect.max.x - 8.0, rect.max.y),
            );
            ui.painter()
                .rect_filled(bar, CornerRadius::same(1), look.c(|t| t.accent));
        }
        response
    }

    /// The status bar: the tool hint, the sketch's constraint state, messages, selection.
    pub(crate) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let look = self.look();
        let muted = look.c(|t| t.text_muted);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = S3;
            let small = |text: &str, c| egui::RichText::new(text).size(T_SMALL).color(c);
            if self.session.active_sketch.is_some() {
                if let Some(state) = self.sketch.state {
                    let (dot, text) = match state {
                        crate::sketch_ui::SketchState::Full => {
                            (look.c(|t| t.success), "Fully constrained".to_owned())
                        }
                        crate::sketch_ui::SketchState::Over => {
                            (look.c(|t| t.error), "Over-constrained".to_owned())
                        }
                        crate::sketch_ui::SketchState::Free(n) => (
                            look.c(|t| t.warning),
                            if n == 1 {
                                "1 degree of freedom".to_owned()
                            } else {
                                format!("{n} degrees of freedom")
                            },
                        ),
                        crate::sketch_ui::SketchState::Unsolved => {
                            (look.c(|t| t.error), "Not solved".to_owned())
                        }
                    };
                    let r = widgets::dot(ui, dot, 8.0);
                    let text_r = ui.label(small(&text, look.c(|t| t.text)));
                    let tip = self.sketch.status.clone().unwrap_or_default();
                    r.union(text_r).on_hover_text(tip);
                    widgets::bar_divider(ui, look, 14.0);
                }
                ui.label(small(crate::sketch_ui::tool_hint(self), muted));
            } else if self.solid.pick_face {
                ui.label(small("Click a flat face of a solid to sketch on it · Esc cancels", muted));
            } else if self.solid.blend.is_some() {
                ui.label(small("Click edges of a solid to pick them · click again to unpick · Esc cancels", muted));
            } else if self.solid.extrude.is_some() {
                ui.label(small("Click inside closed profiles to pick them · click again to unpick", muted));
            } else {
                ui.label(small(
                    "Click: select · Middle-drag: pan · Right-drag or Shift+middle: orbit · Wheel: zoom · F: fit · H: home",
                    muted,
                ));
            }
            if let Some((m, error)) = &self.message {
                widgets::bar_divider(ui, look, 14.0);
                if *error {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    let e = look.c(|t| t.error);
                    look.icons.paint(ui.painter(), Icon::Error, r, e, e);
                    ui.label(small(m, e));
                } else {
                    ui.label(small(m, look.c(|t| t.text)));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = S3;
                let tris: usize = self
                    .session
                    .scene
                    .bodies
                    .iter()
                    .map(|b| b.mesh.triangles.len())
                    .sum();
                let n = self.session.scene.bodies.len();
                let bodies = if n == 1 { "body" } else { "bodies" };
                ui.label(small(&format!("{n} {bodies} · {tris} triangles"), muted));
                if let Some(h) = self.session.scene.selection {
                    let body = self.session.scene.body(h.body);
                    let name = body.map(|b| b.name.as_str()).unwrap_or("");
                    let n = body
                        .and_then(|b| b.mesh.triangle_normal(h.triangle))
                        .unwrap_or_default();
                    widgets::bar_divider(ui, look, 14.0);
                    ui.label(small(
                        &format!(
                            "{name} · ({:.2}, {:.2}, {:.2}) mm · normal ({:.2}, {:.2}, {:.2})",
                            h.point.x, h.point.y, h.point.z, n.x, n.y, n.z
                        ),
                        look.c(|t| t.text),
                    ));
                } else if !self.session.sketch_selection.is_empty() {
                    let k = self.session.sketch_selection.len();
                    widgets::bar_divider(ui, look, 14.0);
                    ui.label(small(
                        &format!("{k} selected"),
                        look.c(|t| t.text),
                    ));
                }
            });
        });
    }

    /// A quiet hint in the middle of an empty viewport.
    pub(crate) fn viewport_hint(&self, ui: &mut egui::Ui, rect: Rect) {
        let empty = self.session.scene.bodies.is_empty()
            && self.session.doc.sketches.is_empty()
            && self.session.active_sketch.is_none();
        if !empty {
            return;
        }
        let look = self.look();
        let card = Rect::from_center_size(
            egui::pos2(rect.center().x, rect.min.y + rect.height() * 0.16),
            vec2(360.0, 44.0),
        );
        ui.painter().rect(
            card,
            CornerRadius::same(widgets::R_LARGE),
            look.c(|t| t.panel).gamma_multiply(0.94),
            Stroke::new(1.0, look.c(|t| t.border)),
            egui::StrokeKind::Inside,
        );
        let icon = Rect::from_center_size(
            egui::pos2(card.min.x + 26.0, card.center().y),
            Vec2::splat(22.0),
        );
        look.icon(ui, Icon::SketchCreate, icon, true);
        ui.painter().text(
            egui::pos2(card.min.x + 48.0, card.center().y),
            egui::Align2::LEFT_CENTER,
            "Start with Create Sketch, then pick a plane",
            FontId::proportional(widgets::T_BODY),
            look.c(|t| t.text),
        );
    }
}

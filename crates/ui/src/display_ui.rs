//! How the model is displayed: the visual style, projection and section items of the View
//! menu, the display bar at the bottom of the viewport, and the Appearance panel. Every
//! change goes through a command (`view.style`, `view.projection`, `view.section`,
//! `view.section_clear`, `body.appearance`).

use crate::icons::Icon;
use crate::widgets::{self, Look, S2, S3, T_SMALL};
use crate::{ChamferaApp, label};
use chamfera_render::display::PRESETS;
use chamfera_render::{Projection, VisualStyle};
use egui::{Color32, Rect, vec2};
use serde_json::json;

/// The display UI's own state.
pub struct DisplayUi {
    /// The Appearance panel is open.
    pub appearance_open: bool,
    /// The colour being picked (sRGB).
    colour: [u8; 3],
    opacity: f32,
    /// The section plane being set up: `xy`, `xz` or `yz`, and its offset in mm.
    plane: &'static str,
    offset: f64,
    flip: bool,
}

impl Default for DisplayUi {
    fn default() -> Self {
        DisplayUi {
            appearance_open: false,
            colour: [128, 128, 128],
            opacity: 1.0,
            plane: "xy",
            offset: 0.0,
            flip: false,
        }
    }
}

const PLANES: [(&str, &str); 3] = [
    ("xy", "XY (top)"),
    ("xz", "XZ (front)"),
    ("yz", "YZ (right)"),
];

impl ChamferaApp {
    fn set_style(&mut self, style: VisualStyle) {
        self.run("view.style", json!({ "style": style.id() }));
    }

    fn set_projection(&mut self, projection: Projection) {
        let mode = match projection {
            Projection::Perspective => "perspective",
            Projection::Orthographic => "orthographic",
        };
        self.run("view.projection", json!({ "mode": mode }));
    }

    fn apply_section(&mut self) {
        let d = &self.display;
        let params = json!({ "plane": d.plane, "offset": d.offset, "flip": d.flip });
        self.run("view.section", params);
    }

    /// The View menu's display items: visual styles, section analysis and appearance.
    pub(crate) fn display_menu(&mut self, ui: &mut egui::Ui) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        widgets::caption(ui, look, "VISUAL STYLE");
        let current = self.session.scene.display.style;
        let mut chosen = None;
        for style in VisualStyle::ALL {
            if ui.radio(current == style, style.label()).clicked() {
                chosen = Some(style);
            }
        }
        ui.separator();
        widgets::caption(ui, look, "SECTION ANALYSIS");
        let mut plane = None;
        for (id, text) in PLANES {
            if widgets::menu_item(ui, look, Icon::Plane, text, true).clicked() {
                plane = Some(id);
            }
        }
        let has_section = self.session.scene.display.section.is_some();
        let clear = widgets::menu_item(
            ui,
            look,
            Icon::Close,
            label("view.section_clear"),
            has_section,
        )
        .clicked();
        ui.separator();
        let appearance =
            widgets::menu_item(ui, look, Icon::Body, label("body.appearance"), true).clicked();
        if let Some(style) = chosen {
            self.set_style(style);
        }
        if let Some(id) = plane {
            ui.close();
            self.display.plane = id;
            self.display.offset = 0.0;
            self.display.flip = false;
            self.apply_section();
        }
        if clear {
            ui.close();
            self.run("view.section_clear", json!({}));
        }
        if appearance {
            ui.close();
            self.display.appearance_open = true;
        }
    }

    /// The display bar at the bottom of the viewport: projection, visual style,
    /// section and appearance.
    pub(crate) fn display_bar(&mut self, ctx: &egui::Context, rect: Rect) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let projection = self.session.camera.projection;
        let style = self.session.scene.display.style;
        let section = self.session.scene.display.section.is_some();
        let mut new_projection = None;
        let mut new_style = None;
        let mut toggle_section = false;
        let mut offset_changed = false;
        let mut open_appearance = false;
        egui::Area::new(egui::Id::unique("display bar"))
            .order(egui::Order::Middle)
            .fade_in(false)
            // Bottom right, clear of the selection filter at the bottom left.
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(rect.right_bottom() + vec2(-S3, -S3))
            .show(ctx, |ui| {
                widgets::dialog_frame(look)
                    .inner_margin(egui::Margin::symmetric(S2 as i8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            widgets::caption(ui, look, "DISPLAY");
                            for (p, text, tip) in [
                                (Projection::Perspective, "Persp", "Perspective projection"),
                                (Projection::Orthographic, "Ortho", "Orthographic projection"),
                            ] {
                                let r = ui
                                    .add(egui::Button::selectable(
                                        projection == p,
                                        egui::RichText::new(text).size(T_SMALL),
                                    ))
                                    .on_hover_text(tip);
                                if r.clicked() && projection != p {
                                    new_projection = Some(p);
                                }
                            }
                            widgets::bar_divider(ui, look, 18.0);
                            egui::ComboBox::from_id_salt("visual style")
                                .selected_text(egui::RichText::new(style.label()).size(T_SMALL))
                                .width(200.0)
                                .show_ui(ui, |ui| {
                                    for s in VisualStyle::ALL {
                                        if ui.selectable_label(s == style, s.label()).clicked() {
                                            new_style = Some(s);
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(label("view.style"));
                            widgets::bar_divider(ui, look, 18.0);
                            let r = ui
                                .add(egui::Button::selectable(
                                    section,
                                    egui::RichText::new("Section").size(T_SMALL),
                                ))
                                .on_hover_text("Cut the view with a plane (display only)");
                            toggle_section = r.clicked();
                            if section {
                                egui::ComboBox::from_id_salt("section plane")
                                    .selected_text(
                                        egui::RichText::new(self.display.plane.to_uppercase())
                                            .size(T_SMALL),
                                    )
                                    .width(48.0)
                                    .show_ui(ui, |ui| {
                                        for (id, text) in PLANES {
                                            if ui
                                                .selectable_label(self.display.plane == id, text)
                                                .clicked()
                                            {
                                                self.display.plane = id;
                                                offset_changed = true;
                                            }
                                        }
                                    });
                                let r = ui.add(
                                    egui::DragValue::new(&mut self.display.offset)
                                        .speed(0.5)
                                        .range(-1e5..=1e5)
                                        .suffix(" mm"),
                                );
                                offset_changed |= r.changed();
                                if ui
                                    .checkbox(&mut self.display.flip, "Flip")
                                    .on_hover_text("Cut away the other side")
                                    .changed()
                                {
                                    offset_changed = true;
                                }
                            }
                            widgets::bar_divider(ui, look, 18.0);
                            open_appearance = ui
                                .add(egui::Button::selectable(
                                    self.display.appearance_open,
                                    egui::RichText::new("Appearance").size(T_SMALL),
                                ))
                                .on_hover_text("Colour and opacity of the selected bodies")
                                .clicked();
                        });
                    });
            });
        if let Some(p) = new_projection {
            self.set_projection(p);
        }
        if let Some(s) = new_style {
            self.set_style(s);
        }
        if toggle_section {
            if section {
                self.run("view.section_clear", json!({}));
            } else {
                self.apply_section();
            }
        } else if offset_changed && section {
            self.apply_section();
        }
        if open_appearance {
            self.display.appearance_open = !self.display.appearance_open;
        }
    }

    /// The Appearance panel: named appearances, a colour picker and opacity, applied to
    /// the selected bodies.
    pub(crate) fn appearance_window(&mut self, ctx: &egui::Context) {
        if !self.display.appearance_open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let bodies = self.session.scene.selected_bodies();
        let mut close = false;
        let mut preset = None;
        let mut colour = false;
        let mut opacity = false;
        let mut reset = false;
        egui::Window::new("Appearance")
            .id(egui::Id::unique("appearance window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(self.tool_dialog_pos(280.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(280.0);
                if widgets::dialog_title(ui, look, Icon::Body, "Appearance") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    if bodies.is_empty() {
                        ui.label(
                            egui::RichText::new("Select one or more bodies first.")
                                .color(look.c(|t| t.text_muted)),
                        );
                    }
                    ui.add_enabled_ui(!bodies.is_empty(), |ui| {
                        widgets::caption(ui, look, "LIBRARY");
                        egui::Grid::new("appearance presets")
                            .num_columns(2)
                            .spacing([S2, S2])
                            .show(ui, |ui| {
                                for (i, p) in PRESETS.iter().enumerate() {
                                    // A preset's colour is material data, not a theme
                                    // colour: the swatch shows what the body will look like.
                                    let c = p.appearance.colour.unwrap_or(self.theme.body);
                                    let alpha = (p.appearance.opacity * 255.0) as u8;
                                    let swatch =
                                        Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha);
                                    let r = ui.add(
                                        egui::Button::new(
                                            egui::RichText::new(p.label).size(T_SMALL),
                                        )
                                        .fill(look.c(|t| t.panel_alt))
                                        .min_size(vec2(124.0, 22.0)),
                                    );
                                    let dot = Rect::from_center_size(
                                        r.rect.left_center() + vec2(10.0, 0.0),
                                        vec2(10.0, 10.0),
                                    );
                                    ui.painter().rect_filled(dot, 2.0, swatch);
                                    if r.clicked() {
                                        preset = Some(p.id);
                                    }
                                    if i % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.add_space(S2);
                        widgets::caption(ui, look, "CUSTOM");
                        ui.horizontal(|ui| {
                            widgets::field_label(ui, look, "Colour");
                            colour = egui::color_picker::color_edit_button_srgb(
                                ui,
                                &mut self.display.colour,
                            )
                            .changed();
                        });
                        ui.horizontal(|ui| {
                            widgets::field_label(ui, look, "Opacity");
                            opacity = ui
                                .add(
                                    egui::Slider::new(&mut self.display.opacity, 0.05..=1.0)
                                        .fixed_decimals(2),
                                )
                                .changed();
                        });
                        ui.add_space(S2);
                        reset = widgets::secondary_button(ui, look, "Default appearance").clicked();
                    });
                });
            });
        let selected_bodies = bodies.clone();
        for body in selected_bodies {
            if let Some(p) = preset {
                self.run("body.appearance", json!({ "body": body, "preset": p }));
            }
            if colour {
                let [r, g, b] = self.display.colour;
                let hex = format!("#{r:02x}{g:02x}{b:02x}");
                self.run("body.appearance", json!({ "body": body, "colour": hex }));
            }
            if opacity {
                let o = f64::from(self.display.opacity);
                self.run("body.appearance", json!({ "body": body, "opacity": o }));
            }
            if reset {
                self.run(
                    "body.appearance",
                    json!({ "body": body, "preset": "default" }),
                );
            }
        }
        if close {
            self.display.appearance_open = false;
        }
    }
}

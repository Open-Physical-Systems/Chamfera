//! How the model is displayed: the visual style, projection and section items of the View
//! menu, the display bar at the bottom of the viewport, and the Appearance panel. Every
//! change goes through a command (`view.style`, `view.projection`, `view.section`,
//! `view.section_clear`, `body.appearance`).

use crate::icons::Icon;
use crate::widgets::{self, Look, S2, S3, T_SMALL};
use crate::{ChamferaApp, label};
use chamfera_render::display::presets;
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
    /// The section shown is not on an origin plane (a face's), so the fields do not
    /// describe it and editing them would replace it.
    custom_section: bool,
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
            custom_section: false,
        }
    }
}

/// The section planes, named with the world axis their offset runs along.
const PLANES: [(&str, &str); 3] = [
    ("xy", "XY (top, +Z)"),
    ("xz", "XZ (front, +Y)"),
    ("yz", "YZ (right, +X)"),
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

    /// Takes the display bar's plane, offset and flip from the section shown, when a
    /// command (`view.section` from a script or the View menu) set one the fields do not
    /// describe. A cut that is not parallel to an origin plane (on a face) leaves the
    /// fields alone and marks the bar as showing a custom cut.
    pub(crate) fn sync_section_fields(&mut self) {
        let Some((origin, normal)) = self
            .session
            .scene
            .display
            .section
            .as_ref()
            .map(|x| (x.origin, x.normal))
        else {
            self.display.custom_section = false;
            return;
        };
        let frames = [
            ("xy", chamfera_geom::Frame::XY),
            ("xz", chamfera_geom::Frame::XZ),
            ("yz", chamfera_geom::Frame::YZ),
        ];
        let o = origin - chamfera_geom::Point3::ORIGIN;
        for (id, f) in frames {
            let n = f.z();
            let along = normal.dot(n);
            if (along.abs() - 1.0).abs() < chamfera_geom::ANGULAR_TOLERANCE {
                let axis = chamfera_engine::display::section_axis(id).unwrap_or(n);
                self.display.plane = id;
                self.display.offset = o.dot(axis);
                self.display.flip = along < 0.0;
                self.display.custom_section = false;
                return;
            }
        }
        self.display.custom_section = true;
    }

    /// Where a section turned on from the display bar starts: through the middle of the
    /// model along the plane's normal (a cut at 0 of a part standing on XY shows only
    /// its footprint), or 0 with no model.
    pub(crate) fn section_start_offset(&self) -> f64 {
        let n = chamfera_engine::display::section_axis(self.display.plane)
            .unwrap_or(chamfera_geom::Vec3::Z);
        self.session.scene.bounds().map_or(0.0, |b| {
            let mid = b.min + (b.max - b.min) * 0.5;
            (mid - chamfera_geom::Point3::ORIGIN).dot(n)
        })
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
        self.sync_section_fields();
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
        // Bottom right, clear of the selection filter at the bottom left: when the viewport
        // is too narrow for both side by side (as they were last drawn), it sits above it.
        let id = egui::Id::unique("display bar");
        let mut at = rect.right_bottom() + vec2(-S3, -S3);
        let (bar, filter) = ctx.memory(|m| {
            (
                m.area_rect(id),
                m.area_rect(egui::Id::unique("selection filter")),
            )
        });
        if let (Some(bar), Some(filter)) = (bar, filter)
            && at.x - bar.width() < filter.max.x + S3
        {
            at.y = filter.min.y - S2;
        }
        egui::Area::new(id)
            .order(egui::Order::Middle)
            .fade_in(false)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(at)
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
                                .on_hover_text(format!(
                                    "{}\nShaded, with or without edges, or wireframe",
                                    label("view.style")
                                ));
                            widgets::bar_divider(ui, look, 18.0);
                            let r = ui
                                .add(egui::Button::selectable(
                                    section,
                                    egui::RichText::new("Section").size(T_SMALL),
                                ))
                                .on_hover_text("Section\nCut the view with a plane (display only)");
                            toggle_section = r.clicked();
                            if section && self.display.custom_section {
                                ui.label(egui::RichText::new("On a face").size(T_SMALL))
                                    .on_hover_text("Set from a face: Section toggles it off");
                            } else if section {
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
                                    .on_hover_text("Flip\nCut away the other side")
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
                                .on_hover_text(
                                    "Appearance\nColour and opacity of the selected bodies",
                                )
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
                self.display.offset = self.section_start_offset();
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
                                for (i, p) in presets().enumerate() {
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
                    });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                    reset = ui
                        .add_enabled_ui(!bodies.is_empty(), |ui| {
                            widgets::secondary_button(ui, look, "Default Appearance").clicked()
                        })
                        .inner;
                });
            });
        // By name: each change rebuilds the model and gives its bodies new ids.
        let selected_bodies: Vec<String> = bodies
            .iter()
            .filter_map(|id| self.session.scene.body(*id).map(|b| b.name.clone()))
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The display bar follows a section set by command, and a section turned on from
    /// the bar starts through the middle of the model (QA user guide pass: the bar showed
    /// "XY 0.0 mm" for a script's cut; the default XY cut at 0 left only the footprint of
    /// a part standing on XY). The 50 x 30 x 10 box's middle is at z = 5, y = 15.
    #[test]
    fn the_section_fields_follow_the_cut_and_start_mid_model() {
        let mut app = ChamferaApp::headless();
        app.run("sketch.create", json!({ "plane": "xy" }));
        app.run(
            "sketch.rectangle",
            json!({ "x1": 0, "y1": 0, "x2": 50, "y2": 30 }),
        );
        app.run("sketch.finish", json!({}));
        app.run(
            "solid.extrude",
            json!({ "sketch": 1, "profiles": [{ "x": 25, "y": 15 }], "distance": 10 }),
        );
        app.run(
            "view.section",
            json!({ "plane": "xz", "offset": -12, "flip": true }),
        );
        app.sync_section_fields();
        assert_eq!(app.display.plane, "xz");
        assert!((app.display.offset + 12.0).abs() < 1e-9);
        assert!(app.display.flip);
        assert!(!app.display.custom_section);
        app.display.plane = "xy";
        assert!((app.section_start_offset() - 5.0).abs() < 1e-9);
        // XZ's offset runs along +Y: the middle y = 15 is the offset +15.
        app.display.plane = "xz";
        assert!((app.section_start_offset() - 15.0).abs() < 1e-9);
    }
}

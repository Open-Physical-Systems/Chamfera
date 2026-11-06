//! The Preferences window (Ctrl+, and File > Preferences): General, Navigation, Units
//! and Grid pages. User preferences are saved to their file as soon as they change; the
//! Units page edits the design (`design.units`), so it is saved with the design.

use crate::icons::Icon;
use crate::navigation::{MAX_SENSITIVITY, MIN_SENSITIVITY, Preset};
use crate::prefs::{self, MAX_GRID, MAX_RECENT, MAX_UI_SCALE, MIN_GRID, MIN_UI_SCALE, Preferences};
use crate::widgets::{self, Look, S2, S3, S4, T_BODY, T_SMALL};
use crate::{ChamferaApp, ThemeChoice};
use chamfera_doc::LengthUnit;
use egui::vec2;
use serde_json::json;

/// A page of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    General,
    Navigation,
    Units,
    Grid,
}

impl Page {
    pub const ALL: [Page; 4] = [Page::General, Page::Navigation, Page::Units, Page::Grid];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Navigation => "Navigation",
            Page::Units => "Units",
            Page::Grid => "Grid and Snap",
        }
    }

    /// `--preferences <page>`.
    pub fn parse(s: &str) -> Option<Page> {
        match s {
            "general" => Some(Page::General),
            "navigation" => Some(Page::Navigation),
            "units" => Some(Page::Units),
            "grid" => Some(Page::Grid),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct PrefsUi {
    pub open: bool,
    pub page: Page,
    /// The grid spacing being typed (in the design's unit).
    grid_text: Option<String>,
    /// The last save error, shown once.
    error: Option<String>,
}

fn row_label(ui: &mut egui::Ui, look: Look, text: &str) {
    widgets::fixed_text(
        ui,
        150.0,
        egui::RichText::new(text)
            .size(T_BODY)
            .color(look.c(|t| t.text_muted)),
        egui::Align::Min,
    );
}

fn hint(ui: &mut egui::Ui, look: Look, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(T_SMALL)
            .color(look.c(|t| t.text_muted)),
    );
}

impl ChamferaApp {
    /// Applies preferences that act on the app at once, then saves them.
    pub(crate) fn preferences_changed(&mut self) {
        // Applying the preferences again (a design opened, the recent list changed) keeps
        // a theme forced at start; picking another theme ends it.
        self.theme_choice = match self.forced_theme {
            Some((forced, base)) if base == self.prefs.theme => forced,
            _ => {
                self.forced_theme = None;
                self.prefs.theme
            }
        };
        self.styled_for = None;
        let result = if self.prefs_store {
            prefs::save(&self.prefs)
        } else {
            Ok(())
        };
        self.prefs_ui.error = result.err();
    }

    /// Keeps the view's grid on the preferences (a new design resets the scene).
    pub(crate) fn sync_grid(&mut self) {
        if self.session.scene.grid_spacing != self.prefs.grid_spacing {
            let spacing = self.prefs.grid_spacing;
            self.run("view.grid", json!({ "spacing": spacing }));
        }
    }

    pub(crate) fn preferences_window(&mut self, ctx: &egui::Context) {
        if !self.prefs_ui.open {
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let vr = self.viewport_rect;
        let before = self.prefs.clone();
        let units_before = self.session.doc.units;
        let mut units = units_before;
        let mut page = self.prefs_ui.page;
        let mut close = false;
        let mut grid_text = self.prefs_ui.grid_text.take();
        let error = self.prefs_ui.error.clone();
        let p = &mut self.prefs;
        egui::Window::new("Preferences")
            .id(egui::Id::unique("preferences window"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(egui::pos2(vr.center().x - 300.0, vr.min.y + 40.0))
            .fade_in(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(600.0);
                if widgets::dialog_title(ui, look, Icon::Parameters, "Preferences") {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(130.0);
                            for pg in Page::ALL {
                                let text = egui::RichText::new(pg.label()).size(T_BODY);
                                if ui
                                    .add_sized(
                                        vec2(130.0, 26.0),
                                        egui::Button::selectable(page == pg, text),
                                    )
                                    .clicked()
                                {
                                    page = pg;
                                }
                            }
                        });
                        ui.add_space(S3);
                        // A divider as tall as the pages, not the screen.
                        let (line, _) =
                            ui.allocate_exact_size(vec2(1.0, 480.0), egui::Sense::hover());
                        ui.painter().vline(
                            line.center().x,
                            line.y_range(),
                            egui::Stroke::new(1.0, look.c(|t| t.border)),
                        );
                        ui.add_space(S4);
                        ui.vertical(|ui| {
                            ui.set_min_height(480.0);
                            ui.spacing_mut().item_spacing.y = S2 + 2.0;
                            match page {
                                Page::General => general_page(ui, look, p),
                                Page::Navigation => navigation_page(ui, look, p),
                                Page::Units => units_page(ui, look, &mut units),
                                Page::Grid => grid_page(ui, look, p, &mut grid_text, units),
                            }
                            if let Some(e) = &error {
                                ui.add_space(S2);
                                ui.label(
                                    egui::RichText::new(format!("Not saved: {e}"))
                                        .size(T_SMALL)
                                        .color(look.c(|t| t.error)),
                                );
                            }
                        });
                    });
                });
                widgets::dialog_footer(ui, look, |ui| {
                    if widgets::primary_button(ui, look, "Close", true).clicked() {
                        close = true;
                    }
                    hint(ui, look, "Changes apply and are kept at once.");
                });
            });
        self.prefs_ui.page = page;
        self.prefs_ui.grid_text = grid_text;
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.prefs_ui.open = false;
        }
        if self.prefs != before {
            if self.prefs.ui_scale != before.ui_scale {
                self.zoom = Some(self.prefs.ui_scale);
            }
            self.preferences_changed();
            self.sync_grid();
        }
        if units != units_before {
            self.run(
                "design.units",
                json!({ "length": units.length.symbol(), "precision": units.length_decimals,
                        "angle_precision": units.angle_decimals }),
            );
            self.refresh_sketch_status();
        }
    }
}

fn general_page(ui: &mut egui::Ui, look: Look, p: &mut Preferences) {
    widgets::caption(ui, look, "APPEARANCE");
    ui.horizontal(|ui| {
        row_label(ui, look, "Theme");
        for (c, text) in [
            (ThemeChoice::System, "Follow system"),
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
        ] {
            ui.radio_value(&mut p.theme, c, text);
        }
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "UI scale");
        let mut pct = p.ui_scale * 100.0;
        let r = ui.add(
            egui::Slider::new(&mut pct, MIN_UI_SCALE * 100.0..=MAX_UI_SCALE * 100.0)
                .step_by(5.0)
                .suffix(" %"),
        );
        if r.drag_stopped() || (r.changed() && !r.dragged()) {
            p.ui_scale = (pct / 100.0).clamp(MIN_UI_SCALE, MAX_UI_SCALE);
        }
    });
    ui.add_space(S2);
    widgets::caption(ui, look, "NEW DESIGNS");
    ui.horizontal(|ui| {
        row_label(ui, look, "Default units");
        unit_combo(ui, "default units", &mut p.default_length);
    });
    ui.add_space(S2);
    widgets::caption(ui, look, "FILES");
    ui.horizontal(|ui| {
        row_label(ui, look, "Recent files listed");
        let mut n = p.recent_count;
        ui.add(egui::DragValue::new(&mut n).range(0..=MAX_RECENT));
        p.recent_count = n.min(MAX_RECENT);
    });
    if ui.button("Clear recent files").clicked() {
        p.recent.clear();
    }
}

fn unit_combo(ui: &mut egui::Ui, id: &str, unit: &mut LengthUnit) {
    egui::ComboBox::from_id_salt(id)
        .width(180.0)
        .selected_text(unit.name())
        .show_ui(ui, |ui| {
            for u in LengthUnit::ALL {
                ui.selectable_value(unit, u, u.name());
            }
        });
}

fn navigation_page(ui: &mut egui::Ui, look: Look, p: &mut Preferences) {
    let nav = &mut p.navigation;
    widgets::caption(ui, look, "MOUSE PRESET");
    for preset in Preset::ALL {
        ui.radio_value(&mut nav.preset, preset, preset.label());
    }
    ui.add_space(S2);
    egui::Frame::new()
        .fill(look.c(|t| t.chrome))
        .corner_radius(egui::CornerRadius::same(widgets::R_SMALL))
        .inner_margin(egui::Margin::same(S3 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("navigation summary")
                .num_columns(2)
                .spacing(vec2(S4, 4.0))
                .show(ui, |ui| {
                    for (gesture, does) in nav.preset.summary() {
                        ui.label(
                            egui::RichText::new(*gesture)
                                .size(T_BODY)
                                .strong()
                                .color(look.c(|t| t.text)),
                        );
                        ui.label(
                            egui::RichText::new(*does)
                                .size(T_BODY)
                                .color(look.c(|t| t.text)),
                        );
                        ui.end_row();
                    }
                });
        });
    hint(
        ui,
        look,
        "A left click always selects; in a sketch the left button draws.",
    );
    ui.add_space(S2);
    widgets::caption(ui, look, "ZOOM AND ORBIT");
    ui.checkbox(&mut nav.invert_zoom, "Invert wheel zoom direction");
    ui.checkbox(&mut nav.zoom_to_cursor, "Zoom toward the cursor");
    ui.horizontal(|ui| {
        row_label(ui, look, "Orbit about");
        ui.radio_value(&mut nav.orbit_about_cursor, false, "Model centre");
        ui.radio_value(&mut nav.orbit_about_cursor, true, "Point under cursor");
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "Mouse sensitivity");
        ui.add(
            egui::Slider::new(&mut nav.sensitivity, MIN_SENSITIVITY..=MAX_SENSITIVITY)
                .logarithmic(true)
                .fixed_decimals(2)
                .suffix("×"),
        );
    });
}

fn units_page(ui: &mut egui::Ui, look: Look, units: &mut chamfera_doc::DisplayUnits) {
    widgets::caption(ui, look, "THIS DESIGN");
    ui.horizontal(|ui| {
        row_label(ui, look, "Length unit");
        unit_combo(ui, "design units", &mut units.length);
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "Length decimals");
        ui.add(
            egui::DragValue::new(&mut units.length_decimals)
                .range(0..=chamfera_doc::units::MAX_DECIMALS),
        );
        hint(ui, look, &format!("e.g. {}", units.length(12.3456)));
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "Angle decimals");
        ui.add(
            egui::DragValue::new(&mut units.angle_decimals)
                .range(0..=chamfera_doc::units::MAX_DECIMALS),
        );
        hint(ui, look, &format!("e.g. {}", units.angle(0.5)));
    });
    ui.add_space(S2);
    hint(
        ui,
        look,
        &format!(
            "Saved with the design. Dimensions, dialogs, measurements and the status bar use \
             these units; a number typed without a unit is read in {}. Changing the unit keeps \
             every size.",
            units.length.symbol()
        ),
    );
}

fn grid_page(
    ui: &mut egui::Ui,
    look: Look,
    p: &mut Preferences,
    text: &mut Option<String>,
    units: chamfera_doc::DisplayUnits,
) {
    widgets::caption(ui, look, "GRID");
    let mut auto = p.grid_spacing.is_none();
    ui.checkbox(&mut auto, "Automatic grid (follows the zoom)");
    if auto {
        p.grid_spacing = None;
        *text = None;
    } else {
        let scale = units.length.mm();
        let current = p.grid_spacing.unwrap_or(10.0);
        let t = text.get_or_insert_with(|| units.length_number(current));
        ui.horizontal(|ui| {
            row_label(ui, look, "Grid spacing");
            ui.add(egui::TextEdit::singleline(t).desired_width(80.0));
            ui.label(units.length.symbol());
        });
        let typed = t.trim().replace(',', ".");
        match chamfera_doc::expr::evaluate_as_in(
            &typed,
            chamfera_doc::expr::Dim::LENGTH,
            scale,
            &chamfera_doc::expr::NoNames,
        ) {
            Ok(v) if (MIN_GRID..=MAX_GRID).contains(&v) => p.grid_spacing = Some(v),
            _ => {
                p.grid_spacing = Some(current);
                hint(
                    ui,
                    look,
                    &format!("Type a length from {MIN_GRID} to {MAX_GRID} mm."),
                );
            }
        }
    }
    ui.add_space(S2);
    widgets::caption(ui, look, "SKETCH");
    ui.checkbox(&mut p.snap_to_grid, "Snap to grid while sketching");
    hint(
        ui,
        look,
        "Clicks on empty space land on the nearest grid point; clicks on a point still snap \
         to the point.",
    );
}

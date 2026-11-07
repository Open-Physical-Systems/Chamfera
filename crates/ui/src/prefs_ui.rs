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
    Assistant,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::General,
        Page::Navigation,
        Page::Units,
        Page::Grid,
        Page::Assistant,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Navigation => "Navigation",
            Page::Units => "Units",
            Page::Grid => "Grid and Snap",
            Page::Assistant => "Assistant",
        }
    }

    /// `--preferences <page>`.
    pub fn parse(s: &str) -> Option<Page> {
        match s {
            "general" => Some(Page::General),
            "navigation" => Some(Page::Navigation),
            "units" => Some(Page::Units),
            "grid" => Some(Page::Grid),
            "assistant" => Some(Page::Assistant),
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
        self.session.history.limits = self.prefs.history_limits();
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
        let assistant = &mut self.assistant;
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
                            // The page fits beside the list: long hints wrap instead of
                            // widening the window past its title bar.
                            ui.set_max_width(PAGE_WIDTH);
                            ui.spacing_mut().item_spacing.y = S2 + 2.0;
                            match page {
                                Page::General => general_page(ui, look, p),
                                Page::Navigation => navigation_page(ui, look, p),
                                Page::Units => units_page(ui, look, &mut units),
                                Page::Grid => grid_page(ui, look, p, &mut grid_text, units),
                                Page::Assistant => assistant_page(ui, look, p, assistant),
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
    widgets::caption(ui, look, "START");
    ui.horizontal(|ui| {
        row_label(ui, look, "Welcome screen");
        ui.checkbox(&mut p.show_welcome, "Show at start");
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
    ui.add_space(S2);
    widgets::caption(ui, look, "AUTOSAVE AND HISTORY");
    ui.horizontal(|ui| {
        row_label(ui, look, "Autosave every");
        let mut on = p.autosave_secs > 0;
        if ui.checkbox(&mut on, "").changed() {
            p.autosave_secs = if on { 60 } else { 0 };
        }
        if on {
            let mut secs = p.autosave_secs.max(prefs::MIN_AUTOSAVE_SECS);
            ui.add(
                egui::DragValue::new(&mut secs)
                    .range(prefs::MIN_AUTOSAVE_SECS..=prefs::MAX_AUTOSAVE_SECS)
                    .suffix(" s"),
            );
            p.autosave_secs = secs;
        }
    });
    hint(
        ui,
        look,
        "Unsaved work goes to a recovery file, offered again after a crash.",
    );
    ui.horizontal(|ui| {
        row_label(ui, look, "Versions kept");
        let mut n = p.history_count;
        ui.add(egui::DragValue::new(&mut n).range(1..=prefs::MAX_HISTORY_COUNT));
        p.history_count = n.clamp(1, prefs::MAX_HISTORY_COUNT);
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "History size, at most");
        let mut mb = p.history_mb;
        ui.add(
            egui::DragValue::new(&mut mb)
                .range(1..=prefs::MAX_HISTORY_MB)
                .suffix(" MB"),
        );
        p.history_mb = mb.clamp(1, prefs::MAX_HISTORY_MB);
    });
    hint(
        ui,
        look,
        "Each save keeps a version in a folder beside the design file.",
    );
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
    snaps_section(ui, look, &mut p.snaps);
}

/// The object snaps and tracking guides used while drawing.
fn snaps_section(ui: &mut egui::Ui, look: Look, s: &mut prefs::SnapPrefs) {
    use chamfera_sketch::SnapKind;
    ui.add_space(S2);
    widgets::caption(ui, look, "OBJECT SNAPS");
    egui::Grid::new("snap kinds")
        .num_columns(3)
        .spacing(vec2(S4, S2))
        .show(ui, |ui| {
            for (i, kind) in SnapKind::ALL.into_iter().enumerate() {
                let mut on = s.kinds.contains(kind);
                if ui.checkbox(&mut on, kind.label()).changed() {
                    s.kinds = s.kinds.with(kind, on);
                }
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    ui.horizontal(|ui| {
        row_label(ui, look, "Snap radius");
        ui.add(
            egui::Slider::new(
                &mut s.radius_px,
                prefs::MIN_SNAP_RADIUS_PX..=prefs::MAX_SNAP_RADIUS_PX,
            )
            .step_by(1.0)
            .suffix(" px"),
        );
    });
    ui.add_space(S2);
    widgets::caption(ui, look, "TRACKING");
    ui.checkbox(&mut s.alignment, "Alignment guides through existing points");
    ui.checkbox(
        &mut s.infer,
        "Perpendicular and tangent guides from the line or arc a line starts on",
    );
    ui.horizontal(|ui| {
        ui.checkbox(&mut s.polar, "Polar guides every");
        ui.add_enabled(
            s.polar,
            egui::DragValue::new(&mut s.polar_step_deg)
                .range(prefs::MIN_POLAR_STEP_DEG..=prefs::MAX_POLAR_STEP_DEG)
                .speed(1.0)
                .suffix("°"),
        );
    });
    hint(
        ui,
        look,
        "Hold Ctrl while drawing to place a point freely, with no snap, guide or grid.",
    );
}

/// The width of a page: the window's 600 less the page list (130) and the gaps and
/// divider between them.
const PAGE_WIDTH: f32 = 600.0 - 130.0 - S3 - S4 - 1.0;

fn assistant_page(
    ui: &mut egui::Ui,
    look: Look,
    p: &mut Preferences,
    a: &mut crate::assistant_ui::AssistantUi,
) {
    use chamfera_assistant::{Effort, Model};
    let ap = &mut p.assistant;
    widgets::caption(ui, look, "API KEY");
    #[cfg(target_arch = "wasm32")]
    {
        let _ = a;
        hint(
            ui,
            look,
            "The Assistant works in the desktop app for now: a web page that calls the Claude \
             API would expose your key to the page.",
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use chamfera_assistant::key::KeySource;
        let status = match &a.key {
            Some((k, KeySource::File)) => format!("Saved on this computer ({})", k.hint()),
            Some((k, KeySource::Environment)) => {
                format!("From ANTHROPIC_API_KEY ({})", k.hint())
            }
            None => "No key yet".to_owned(),
        };
        ui.horizontal(|ui| {
            row_label(ui, look, "Key");
            ui.label(
                egui::RichText::new(status)
                    .size(T_BODY)
                    .color(look.c(|t| t.text)),
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, look, "New key");
            ui.add(
                egui::TextEdit::singleline(&mut a.key_text)
                    .password(true)
                    .hint_text("sk-ant-…")
                    .desired_width(150.0)
                    .char_limit(chamfera_assistant::key::MAX_KEY_BYTES),
            );
            if ui.button("Save").clicked() {
                let result = chamfera_assistant::ApiKey::new(&a.key_text).and_then(|k| {
                    let path = crate::assistant_ui::key_path()
                        .ok_or("no config folder (HOME is not set)")?;
                    chamfera_assistant::key::save(&path, &k)?;
                    a.key = Some((k, KeySource::File));
                    Ok(())
                });
                a.key_text.clear();
                a.key_message = Some(match result {
                    Ok(()) => ("Key saved, readable by you only.".into(), false),
                    Err(e) => (e, true),
                });
            }
            if ui.button("Remove").clicked() {
                let result = crate::assistant_ui::key_path()
                    .map_or(Ok(()), |p| chamfera_assistant::key::remove(&p));
                a.load_key();
                a.key_message = Some(match result {
                    Ok(()) => ("Saved key removed.".into(), false),
                    Err(e) => (e, true),
                });
            }
        });
        if let Some((m, err)) = &a.key_message {
            ui.label(egui::RichText::new(m).size(T_SMALL).color(if *err {
                look.c(|t| t.error)
            } else {
                look.c(|t| t.text_muted)
            }));
        }
        hint(
            ui,
            look,
            "The key is kept in its own file in your config folder (or read from \
             ANTHROPIC_API_KEY), never in a design or the preferences file.",
        );
    }
    ui.add_space(S2);
    widgets::caption(ui, look, "MODEL");
    ui.horizontal(|ui| {
        row_label(ui, look, "Model");
        egui::ComboBox::from_id_salt("assistant model")
            .width(180.0)
            .selected_text(ap.model.name())
            .show_ui(ui, |ui| {
                for m in Model::ALL {
                    ui.selectable_value(&mut ap.model, m, m.name());
                }
            });
    });
    ui.horizontal(|ui| {
        row_label(ui, look, "Effort");
        egui::ComboBox::from_id_salt("assistant effort")
            .width(180.0)
            .selected_text(ap.effort.key())
            .show_ui(ui, |ui| {
                for e in Effort::ALL {
                    ui.selectable_value(&mut ap.effort, e, e.key());
                }
            });
    });
    hint(
        ui,
        look,
        "More effort thinks longer: slower and costlier, better on hard parts. Changes apply \
         to the next conversation (Clear in the panel).",
    );
    ui.add_space(S2);
    widgets::caption(ui, look, "SAFETY");
    ui.checkbox(
        &mut ap.confirm_destructive,
        "Ask before Claude deletes features or bodies or starts a new design",
    );
    ui.checkbox(
        &mut ap.allow_file_commands,
        "Let Claude open, save, import and export files (asks each time)",
    );
    ui.checkbox(
        &mut ap.screenshots,
        "Let Claude look at the viewport (screenshots)",
    );
}

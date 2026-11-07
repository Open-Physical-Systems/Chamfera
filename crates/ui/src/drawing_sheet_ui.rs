//! The Sheet Settings dialog of the DRAWING workspace: paper size and orientation,
//! scale, projection angle, the sheet's name and the title block's fields. It is a form
//! over `drawing.edit_sheet`: OK runs that one command (one undo step) with every field.

use crate::drawing_tools_ui::parse_scale;
use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_doc::drawing::{Orientation, Paper, ProjectionAngle, Sheet, TitleBlock};
use serde_json::{Value, json};

/// The dialog's fields, as typed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SheetForm {
    /// The sheet it edits.
    pub sheet: u32,
    pub paper: Paper,
    pub orientation: Orientation,
    /// `1:2`, `2:1` or `0.5`.
    pub scale: String,
    pub projection: ProjectionAngle,
    pub name: String,
    pub title: TitleBlock,
    /// Why the last OK was refused.
    pub error: Option<String>,
}

/// A paper's name in the dialog.
fn paper_name(p: Paper) -> &'static str {
    match p {
        Paper::A4 => "A4 (297 x 210)",
        Paper::A3 => "A3 (420 x 297)",
        Paper::A2 => "A2 (594 x 420)",
        Paper::AnsiA => "ANSI A (11 x 8.5 in)",
        Paper::AnsiB => "ANSI B (17 x 11 in)",
    }
}

/// The title block's fields in the dialog, by their parameter name.
const FIELDS: [(&str, &str); 7] = [
    ("part_name", "Part"),
    ("drawing_number", "Drawing no."),
    ("revision", "Revision"),
    ("material", "Material"),
    ("drawn_by", "Drawn by"),
    ("checked_by", "Checked by"),
    ("date", "Date"),
];

impl SheetForm {
    pub fn of(sheet: &Sheet) -> SheetForm {
        SheetForm {
            sheet: sheet.id,
            paper: sheet.paper,
            orientation: sheet.orientation,
            scale: chamfera_drawing::scale_text(sheet.scale),
            projection: sheet.projection,
            name: sheet.name.clone(),
            title: sheet.title.clone(),
            error: None,
        }
    }

    /// The `drawing.edit_sheet` parameters, or why the scale cannot be read.
    pub fn params(&self) -> Result<Value, String> {
        let scale = parse_scale(&self.scale)
            .ok_or_else(|| format!("{:?} is not a scale: type 1:2, 2:1 or 0.5", self.scale))?;
        let mut p = json!({
            "sheet": self.sheet,
            "paper": self.paper.id(),
            "orientation": match self.orientation {
                Orientation::Landscape => "landscape",
                Orientation::Portrait => "portrait",
            },
            "scale": scale,
            "projection": match self.projection {
                ProjectionAngle::Third => "third",
                ProjectionAngle::First => "first",
            },
            "name": self.name,
        });
        for (name, value) in TitleBlock::NAMES.iter().zip(self.title.fields()) {
            p[*name] = json!(value);
        }
        Ok(p)
    }
}

impl ChamferaApp {
    /// Sheet Settings: opens the dialog on the shown sheet, or closes it.
    pub(crate) fn toggle_sheet_settings(&mut self) {
        if self.drawing.settings.is_some() {
            self.drawing.settings = None;
            return;
        }
        let Some(id) = self.drawing_sheet() else {
            return;
        };
        if let Ok(sheet) = self.session.doc.drawing.sheet(id) {
            self.drawing.settings = Some(SheetForm::of(sheet));
        }
    }

    /// OK: runs `drawing.edit_sheet` with the form; closes it unless that was refused.
    pub(crate) fn apply_sheet_settings(&mut self) {
        let Some(form) = self.drawing.settings.clone() else {
            return;
        };
        let params = match form.params() {
            Ok(p) => p,
            Err(e) => {
                if let Some(f) = self.drawing.settings.as_mut() {
                    f.error = Some(e);
                }
                return;
            }
        };
        if self.run("drawing.edit_sheet", params).is_some() {
            self.drawing.settings = None;
            self.drawing_refit();
            self.message = Some((label("drawing.edit_sheet").to_owned(), false));
        } else {
            let why = self.message.as_ref().map(|m| m.0.clone());
            if let Some(f) = self.drawing.settings.as_mut() {
                f.error = why;
            }
        }
    }

    /// The dialog, when it is open.
    pub(crate) fn sheet_settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.drawing.settings.clone() else {
            return;
        };
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let title = label("drawing.edit_sheet");
        let (mut ok, mut close) = (false, false);
        const WIDTH: f32 = 340.0;
        egui::Window::new(title)
            .id(egui::Id::unique("sheet settings"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fade_in(false)
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Parameters, title) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sheet settings grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            widgets::field_label(ui, look, "Paper");
                            egui::ComboBox::from_id_salt("sheet paper")
                                .width(widgets::FIELD_WIDTH)
                                .selected_text(paper_name(form.paper))
                                .show_ui(ui, |ui| {
                                    for p in Paper::ALL {
                                        ui.selectable_value(&mut form.paper, p, paper_name(p));
                                    }
                                });
                            ui.end_row();
                            widgets::field_label(ui, look, "Orientation");
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut form.orientation,
                                    Orientation::Landscape,
                                    "Landscape",
                                );
                                ui.radio_value(
                                    &mut form.orientation,
                                    Orientation::Portrait,
                                    "Portrait",
                                );
                            });
                            ui.end_row();
                            widgets::field_label(ui, look, "Scale");
                            ui.add_sized(
                                widgets::FIELD,
                                egui::TextEdit::singleline(&mut form.scale).hint_text("1:1"),
                            );
                            ui.end_row();
                            widgets::field_label(ui, look, "Projection");
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut form.projection,
                                    ProjectionAngle::Third,
                                    "Third angle",
                                );
                                ui.radio_value(
                                    &mut form.projection,
                                    ProjectionAngle::First,
                                    "First angle",
                                );
                            });
                            ui.end_row();
                            widgets::field_label(ui, look, "Sheet name");
                            ui.add_sized(
                                widgets::FIELD,
                                egui::TextEdit::singleline(&mut form.name).char_limit(64),
                            );
                            ui.end_row();
                            for (key, name) in FIELDS {
                                if let Some(v) = form.title.field_mut(key) {
                                    widgets::field_label(ui, look, name);
                                    ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(v).char_limit(64),
                                    );
                                    ui.end_row();
                                }
                            }
                        });
                    let note = form.error.clone().unwrap_or_else(|| {
                        "Projected views move to the other side of their base view when the \
                         projection changes."
                            .to_owned()
                    });
                    let colour = if form.error.is_some() {
                        look.c(|t| t.error)
                    } else {
                        look.c(|t| t.text_muted)
                    };
                    ui.label(
                        egui::RichText::new(note)
                            .size(widgets::T_SMALL)
                            .color(colour),
                    );
                });
                widgets::dialog_footer(ui, look, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, look, "OK", true).clicked() {
                            ok = true;
                        }
                        if widgets::secondary_button(ui, look, "Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.drawing.settings = None;
            return;
        }
        // Editing a field clears the last refusal.
        if self.drawing.settings.as_ref().is_some_and(|s| *s != form) {
            form.error = None;
        }
        self.drawing.settings = Some(form);
        if ok {
            self.apply_sheet_settings();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChamferaApp;

    #[test]
    fn sheet_settings_ok_sets_every_field_with_one_command_and_one_undo_step() {
        let mut app = ChamferaApp::headless();
        app.drawing.active = true;
        app.run("drawing.new_sheet", json!({ "part_name": "Plate" }));
        app.toggle_sheet_settings();
        let form = app.drawing.settings.as_mut().unwrap();
        assert_eq!(form.scale, "1:1");
        assert_eq!(form.title.part_name, "Plate");
        form.paper = Paper::A4;
        form.orientation = Orientation::Portrait;
        form.scale = "1:2".into();
        form.projection = ProjectionAngle::First;
        form.title.drawing_number = "CH-7".into();
        form.title.revision = "A".into();
        form.title.material = "S235".into();
        form.title.checked_by = "QA".into();
        let original = app.session.doc.drawing.clone();
        app.apply_sheet_settings();
        assert!(app.drawing.settings.is_none(), "OK closes the dialog");
        let s = &app.session.doc.drawing.sheets[0];
        // A4 portrait is 210 wide and 297 high; 1:2 is 0.5 sheet mm per model mm.
        assert_eq!(s.size(), (210.0, 297.0));
        assert_eq!(s.scale, 0.5);
        assert_eq!(s.projection, ProjectionAngle::First);
        assert_eq!(
            s.title.fields(),
            [
                "Plate",
                "",
                s.title.date.as_str(),
                "CH-7",
                "A",
                "S235",
                "QA"
            ]
        );
        let edited = app.session.doc.drawing.clone();
        app.run("edit.undo", json!({}));
        assert_eq!(
            app.session.doc.drawing, original,
            "one undo step undoes it all"
        );
        app.run("edit.redo", json!({}));
        assert_eq!(app.session.doc.drawing, edited);
        // A scale that does not read stays in the dialog with the reason; nothing changes.
        app.toggle_sheet_settings();
        app.drawing.settings.as_mut().unwrap().scale = "half".into();
        let before = app.session.doc.clone();
        app.apply_sheet_settings();
        assert!(app.drawing.settings.as_ref().unwrap().error.is_some());
        assert_eq!(app.session.doc, before);
        // A field the command refuses (65 characters) too.
        let f = app.drawing.settings.as_mut().unwrap();
        f.scale = "1:1".into();
        f.title.material = "x".repeat(65);
        app.apply_sheet_settings();
        assert!(app.drawing.settings.as_ref().unwrap().error.is_some());
        assert_eq!(app.session.doc, before);
    }
}

//! The Text dialog of the sketch (SKETCH > CREATE > Text): the words, the height of the
//! capitals, the alignment and where the first line starts; OK runs `sketch.text`, which
//! draws the letters as closed profiles.

use crate::icons::Icon;
use crate::sketch_ui::parse_value;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use serde_json::json;

/// Width of the dialog (points).
const WIDTH: f32 = 340.0;
/// The alignments, as `sketch.text` takes them, with their names.
const ALIGNS: [(&str, &str); 3] = [("left", "Left"), ("centre", "Centre"), ("right", "Right")];

/// The open Text dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextDialog {
    pub(crate) text: String,
    pub(crate) height: String,
    pub(crate) align: usize,
    pub(crate) x: String,
    pub(crate) y: String,
    pub(crate) error: Option<String>,
}

impl ChamferaApp {
    /// Opens the Text dialog, starting at the last point placed (or the origin).
    pub(crate) fn open_text_dialog(&mut self) {
        let at = self.sketch.last_at.unwrap_or(chamfera_geom::Point2::ORIGIN);
        let u = self.units();
        self.sketch.text = Some(TextDialog {
            text: "Chamfera".into(),
            height: u.length_number(10.0),
            align: 0,
            x: u.length_number(at.x),
            y: u.length_number(at.y),
            error: None,
        });
    }

    /// OK in the Text dialog.
    pub(crate) fn text_ok(&mut self) {
        let Some(d) = self.sketch.text.clone() else {
            return;
        };
        let u = self.units();
        let num = |s: &str, what: &str| {
            parse_value(u, "length", s).ok_or_else(|| format!("{what}: type a number"))
        };
        let params = (|| -> Result<serde_json::Value, String> {
            Ok(json!({
                "text": d.text,
                "height": num(&d.height, "Height")?,
                "x": num(&d.x, "X")?,
                "y": num(&d.y, "Y")?,
                "align": ALIGNS.get(d.align).map_or("left", |a| a.0),
            }))
        })();
        let result = params.and_then(|p| {
            self.session
                .execute("sketch.text", &p)
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(r) => {
                self.tutorial_saw("sketch.text");
                self.refresh_sketch_status();
                self.sketch.text = None;
                if let Some(missing) = r["missing"].as_array().filter(|m| !m.is_empty()) {
                    let chars: String = missing.iter().filter_map(|c| c.as_str()).collect();
                    self.message = Some((
                        format!("Text: the font has no letters for {chars:?}; they were left out"),
                        false,
                    ));
                }
            }
            Err(e) => {
                if let Some(t) = self.sketch.text.as_mut() {
                    t.error = Some(e);
                }
            }
        }
    }

    /// The Text dialog, while open.
    pub(crate) fn text_dialog(&mut self, ctx: &egui::Context) {
        if self.sketch.text.is_none() {
            return;
        }
        if self.session.active_sketch.is_none() {
            self.sketch.text = None;
            return;
        }
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let unit = self.session.doc.units.length.symbol();
        let Some(d) = self.sketch.text.as_mut() else {
            return;
        };
        let title = label("sketch.text");
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new(title)
            .id(egui::Id::unique("sketch text dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Text, title) {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    egui::Grid::new("sketch text grid")
                        .num_columns(2)
                        .spacing([widgets::S3, widgets::S3])
                        .show(ui, |ui| {
                            let mut field =
                                |ui: &mut egui::Ui, caption: &str, v: &mut String, hint: &str| {
                                    widgets::field_label(ui, look, caption);
                                    let r = ui.add_sized(
                                        widgets::FIELD,
                                        egui::TextEdit::singleline(v)
                                            .hint_text(hint)
                                            .align(egui::Align2::LEFT_CENTER),
                                    );
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        ok = true;
                                    }
                                    ui.end_row();
                                };
                            field(ui, "Text", &mut d.text, "Words to draw");
                            field(ui, "Height", &mut d.height, unit);
                            widgets::field_label(ui, look, "Align");
                            egui::ComboBox::from_id_salt("sketch text align")
                                .selected_text(ALIGNS.get(d.align).map_or("", |a| a.1))
                                .width(widgets::FIELD_WIDTH)
                                .show_ui(ui, |ui| {
                                    for (i, (_, n)) in ALIGNS.iter().enumerate() {
                                        ui.selectable_value(&mut d.align, i, *n);
                                    }
                                });
                            ui.end_row();
                            field(ui, "Start X", &mut d.x, unit);
                            field(ui, "Start Y", &mut d.y, unit);
                        });
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Height is the height of the capitals; the start is on the \
                                 baseline. Each letter becomes closed profiles to extrude.",
                            )
                            .size(widgets::T_SMALL)
                            .color(look.c(|t| t.text_muted)),
                        )
                        .wrap(),
                    );
                    if let Some(e) = &d.error {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(e)
                                    .size(widgets::T_SMALL)
                                    .color(look.c(|t| t.error)),
                            )
                            .wrap(),
                        );
                    }
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
            self.text_ok();
        } else if cancel {
            self.sketch.text = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_dialog_draws_the_words_as_profiles() {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.open_text_dialog();
        if let Some(d) = app.sketch.text.as_mut() {
            d.text = "Hi".into();
            d.height = "8".into();
        }
        app.text_ok();
        assert!(app.sketch.text.is_none());
        let f = app.session.active_sketch().unwrap();
        // H, the stem and the dot of the i.
        assert_eq!(f.sketch.profiles().len(), 3);
        // A bad height keeps the dialog open with the reason.
        app.open_text_dialog();
        if let Some(d) = app.sketch.text.as_mut() {
            d.height = "tall".into();
        }
        app.text_ok();
        assert!(app.sketch.text.as_ref().is_some_and(|d| d.error.is_some()));
    }
}

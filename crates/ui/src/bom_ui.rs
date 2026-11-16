//! The Bill of Materials panel (ASSEMBLE > Bill of Materials): one row per part with its
//! quantity (linked copies counted), material, volume and mass, and Export CSV
//! (`bom.export_csv`). Reads the engine's `bom::bom`; it changes nothing.

use crate::ChamferaApp;
use crate::icons::Icon;
use crate::widgets::{self, Look};
use chamfera_engine::bom::BomRow;
use serde_json::Value;

/// The panel's state.
#[derive(Default)]
pub struct BomUi {
    pub open: bool,
    /// The rows as of an undo mark: worked out again only when the design changed.
    cache: Option<(chamfera_engine::UndoMark, Vec<BomRow>)>,
}

impl ChamferaApp {
    /// The bill of materials, from the cache while the design is unchanged.
    fn bom_rows(&mut self) -> Vec<BomRow> {
        let mark = self.session.undo_mark();
        match &self.bom.cache {
            Some((m, rows)) if *m == mark => rows.clone(),
            _ => {
                let rows = chamfera_engine::bom::bom(&self.session);
                self.bom.cache = Some((mark, rows.clone()));
                rows
            }
        }
    }

    /// A start-up script that runs `bom.list` opens the panel, so a screenshot shows it.
    pub(crate) fn bom_from_script(&mut self, steps: &[Value]) {
        if steps
            .iter()
            .any(|s| s["command"].as_str() == Some("bom.list"))
        {
            self.bom.open = true;
        }
    }

    /// The Bill of Materials panel.
    pub(crate) fn bom_panel(&mut self, ctx: &egui::Context) {
        const WIDTH: f32 = 460.0;
        if !self.bom.open {
            return;
        }
        let rows = self.bom_rows();
        let units = self.units();
        let pos = self.tool_dialog_pos(WIDTH);
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let title = crate::label("bom.list");
        let mut close = false;
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut export = false;
        egui::Window::new(title)
            .id(egui::Id::unique("bom panel"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Bom, title) {
                    close = true;
                }
                widgets::dialog_body(ui, |ui| {
                    if rows.is_empty() {
                        ui.label(
                            egui::RichText::new("No parts yet: build bodies first")
                                .color(look.c(|t| t.text_muted)),
                        );
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            egui::Grid::new("bom rows")
                                .num_columns(6)
                                .striped(true)
                                .spacing([widgets::S3, widgets::S2])
                                .show(ui, |ui| {
                                    for h in ["Item", "Part", "Qty", "Material", "Volume", "Mass"] {
                                        ui.label(
                                            egui::RichText::new(h)
                                                .strong()
                                                .color(look.c(|t| t.text_muted)),
                                        );
                                    }
                                    ui.end_row();
                                    for r in &rows {
                                        ui.label(r.item.to_string());
                                        ui.label(&r.part);
                                        ui.label(r.quantity.to_string());
                                        ui.label(if r.material.is_empty() {
                                            "-"
                                        } else {
                                            r.material.as_str()
                                        });
                                        ui.label(
                                            r.volume.map_or_else(
                                                || "-".to_owned(),
                                                |v| units.volume(v),
                                            ),
                                        );
                                        ui.label(r.mass.map_or_else(
                                            || "-".to_owned(),
                                            |m| format!("{m:.1} g"),
                                        ));
                                        ui.end_row();
                                    }
                                });
                        });
                    let parts: u64 = rows.iter().map(|r| u64::from(r.quantity)).sum();
                    ui.add_space(widgets::S2);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} part(s), {parts} in all; linked copies count toward their part",
                            rows.len()
                        ))
                        .color(look.c(|t| t.text_muted)),
                    );
                });
                widgets::dialog_footer(ui, look, |ui| {
                    #[cfg(not(target_arch = "wasm32"))]
                    if widgets::primary_button(ui, look, "Export CSV", !rows.is_empty()).clicked() {
                        export = true;
                    }
                    if widgets::secondary_button(ui, look, "Close").clicked() {
                        close = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            close = true;
        }
        if export {
            #[cfg(not(target_arch = "wasm32"))]
            self.export("CSV table", &["csv"], "bom.export_csv");
        }
        if close {
            self.bom.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ChamferaApp;
    use serde_json::json;

    #[test]
    fn the_panel_lists_a_part_and_its_linked_copy_as_two() {
        let mut app = ChamferaApp::headless();
        let sk = app.run("sketch.create", json!({"plane": "xy"})).unwrap();
        app.run(
            "sketch.rectangle",
            json!({"x1": 0, "y1": 0, "x2": 10, "y2": 10}),
        );
        app.run("sketch.finish", json!({}));
        let f = app
            .run(
                "solid.extrude",
                json!({"sketch": sk["sketch"], "distance": 10, "profiles": [{"x": 5, "y": 5}]}),
            )
            .unwrap();
        let c = app
            .run(
                "component.new",
                json!({"name": "Peg", "from_bodies": [f["feature"]]}),
            )
            .unwrap();
        app.run("component.copy", json!({"component": c["component"]}));
        app.run_tool("bom.list");
        assert!(app.bom.open);
        let rows = app.bom_rows();
        // One part, Peg, shown twice; 10 x 10 x 10 = 1000 mm³ each.
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].part.as_str(), rows[0].quantity), ("Peg", 2));
        assert!((rows[0].volume.unwrap() - 1000.0).abs() < 1e-6);
    }
}

//! Constraint markers on the canvas, and the dialog for a constraint that does not fit.
//!
//! **Markers.** While a sketch is edited, every geometric constraint shows a small marker
//! (its ribbon icon in a box) beside the geometry it holds (`Session::constraint_glyphs`).
//! Markers at the same place line up side by side. Hovering one names it; clicking selects
//! it (accent outline), clicking again or anywhere in the sketch drops it, and Del deletes
//! it (`sketch.remove {constraint}`).
//!
//! **Conflicts.** A constraint tool first asks `sketch.conflicts`. When the new constraint
//! does not fit, the Constraint Conflict dialog lists the constraints whose removal (one at
//! a time) lets it in; Replace removes that one and adds the new one in one step
//! (`sketch.constrain {replace}`), Cancel leaves the sketch as it was.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, color};
use chamfera_engine::sketching::constraint_title;
use egui::{Rect, Vec2};
use serde_json::{Value, json};

/// Width of the conflict dialog (points).
const WIDTH: f32 = 380.0;
/// A marker's box (points).
const GLYPH: f32 = 17.0;
/// Most markers drawn, so a huge sketch stays quick to draw.
const MAX_GLYPHS: usize = 600;

/// The open conflict dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Conflict {
    /// What was asked for, in words ("Vertical").
    pub(crate) title: String,
    /// The constraints to add, as `sketch.constrain` takes them.
    pub(crate) constraints: Vec<Value>,
    /// The constraints whose removal lets them in: id and how to name it.
    pub(crate) involved: Vec<(u64, String)>,
    /// Why the last Replace failed.
    pub(crate) error: Option<String>,
}

/// The ribbon icon of a constraint kind.
pub(crate) fn glyph_icon(kind: &str) -> Icon {
    match kind {
        "coincident" => Icon::ConstraintCoincident,
        "point_on_line" | "point_on_circle" => Icon::ConstraintPointOn,
        "horizontal" | "horizontal_points" => Icon::ConstraintHorizontal,
        "vertical" | "vertical_points" => Icon::ConstraintVertical,
        "parallel" => Icon::ConstraintParallel,
        "perpendicular" => Icon::ConstraintPerpendicular,
        "tangent" => Icon::ConstraintTangent,
        "equal" => Icon::ConstraintEqual,
        "fix" => Icon::ConstraintFix,
        "midpoint" => Icon::ConstraintMidpoint,
        "symmetric" => Icon::Mirror,
        "concentric" => Icon::ConstraintConcentric,
        "collinear" => Icon::ConstraintCollinear,
        _ => Icon::Dimension,
    }
}

/// How the dialog names a constraint: its kind, its value for a dimension, and its id.
fn involved_name(units: chamfera_doc::DisplayUnits, c: &Value) -> String {
    let kind = c["kind"].as_str().unwrap_or("");
    let id = c["constraint"].as_u64().unwrap_or(0);
    let title = constraint_title(kind);
    match c["value"].as_f64() {
        Some(v) if kind == "angle" => format!("{title} {} (#{id})", units.angle(v)),
        Some(v) => format!("{title} {} (#{id})", units.length(v)),
        None => format!("{title} (#{id})"),
    }
}

impl ChamferaApp {
    /// A constraint tool on the selection: adds the constraint, or opens the conflict
    /// dialog when it does not fit.
    pub(crate) fn constrain_selection_checked(&mut self, kind: &str) {
        match self
            .session
            .execute("sketch.conflicts", &json!({ "kind": kind }))
        {
            Ok(r) if r["conflict"] == true => {
                let title = crate::sketch_ui::CONSTRAINTS
                    .iter()
                    .find(|c| c.0 == kind)
                    .map_or("Constraint", |c| c.1);
                self.open_conflict(title, &r);
            }
            // It fits, or the selection does not suit it: the command says which.
            _ => {
                self.run_sketch("sketch.constrain_selection", json!({ "kind": kind }));
            }
        }
    }

    /// Opens the conflict dialog for a `sketch.conflicts` reply.
    pub(crate) fn open_conflict(&mut self, title: &str, reply: &Value) {
        let units = self.units();
        self.sketch.conflict = Some(Conflict {
            title: title.to_owned(),
            constraints: reply["constraints"].as_array().cloned().unwrap_or_default(),
            involved: reply["involved"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| Some((c["constraint"].as_u64()?, involved_name(units, c))))
                        .collect()
                })
                .unwrap_or_default(),
            error: None,
        });
    }

    /// `--dialog constraint-conflict:JSON`: the dialog for that constraint, if it conflicts.
    pub(crate) fn open_conflict_for(&mut self, constraint: &Value) -> Result<(), String> {
        let r = self
            .session
            .execute("sketch.conflicts", &json!({ "constraint": constraint }))
            .map_err(|e| e.to_string())?;
        if r["conflict"] != true {
            return Err("that constraint fits the sketch: there is no conflict".into());
        }
        let kind = constraint["kind"].as_str().unwrap_or("");
        self.open_conflict(constraint_title(kind), &r);
        Ok(())
    }

    /// The constraint markers over the viewport.
    pub(crate) fn constraint_glyphs_ui(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let glyphs = self.session.constraint_glyphs();
        if glyphs.is_empty() {
            self.sketch.glyph_selected = None;
            return;
        }
        if let Some(sel) = self.sketch.glyph_selected
            && !glyphs.iter().any(|g| g.constraint == sel)
        {
            self.sketch.glyph_selected = None;
        }
        let painter = ui.painter_at(rect);
        let bg = color(self.theme.panel).gamma_multiply(0.94);
        let border = color(self.theme.border);
        let accent = color(self.theme.accent);
        let (base, highlight) = (color(self.theme.icon), color(self.theme.icon_accent));
        let radius = egui::CornerRadius::same(widgets::R_SMALL);
        // Markers already placed: the anchor pixel and how many sit there.
        let mut stacks: Vec<(egui::Pos2, usize)> = Vec::new();
        let mut clicked = None;
        for g in glyphs.into_iter().take(MAX_GLYPHS) {
            let Some(p) = self.project(rect, g.anchor) else {
                continue;
            };
            let k = match stacks.iter_mut().find(|(q, _)| q.distance(p) < 4.0) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.1 - 1
                }
                None => {
                    stacks.push((p, 1));
                    0
                }
            };
            // Up and to the right of the place, clear of the point marker.
            let centre = p + Vec2::new(13.0 + k as f32 * (GLYPH + 2.0), -13.0);
            let r = Rect::from_center_size(centre, Vec2::splat(GLYPH));
            if !rect.contains_rect(r) {
                continue;
            }
            let id = ui.scope_id().with(("constraint glyph", g.constraint, k));
            let title = constraint_title(&g.kind);
            let resp = ui
                .interact(r, id, egui::Sense::click())
                .on_hover_text(format!("{title}\nClick to select it · Del deletes it"));
            let selected = self.sketch.glyph_selected == Some(g.constraint);
            let hot = resp.hovered() || selected;
            painter.rect_filled(
                r,
                radius,
                if selected {
                    accent.gamma_multiply(0.25)
                } else {
                    bg
                },
            );
            painter.rect_stroke(
                r,
                radius,
                egui::Stroke::new(
                    if hot { 1.5 } else { 1.0 },
                    if hot { accent } else { border },
                ),
                egui::StrokeKind::Inside,
            );
            self.icons.paint(
                &painter,
                glyph_icon(&g.kind),
                r.shrink(2.0),
                base,
                highlight,
            );
            if resp.clicked() {
                clicked = Some((g.constraint, selected));
            }
        }
        if let Some((c, was)) = clicked {
            self.sketch.glyph_selected = (!was).then_some(c);
        }
    }

    /// The Constraint Conflict dialog.
    pub(crate) fn conflict_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = self.sketch.conflict.clone() else {
            return;
        };
        if self.session.active_sketch.is_none() {
            self.sketch.conflict = None;
            return;
        }
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let pos = self.tool_dialog_pos(WIDTH);
        let mut replace = None;
        let mut cancel = false;
        let heading = format!("{} conflicts with the sketch", d.title);
        egui::Window::new("Constraint Conflict")
            .id(egui::Id::unique("constraint conflict dialog"))
            .title_bar(false)
            .frame(widgets::dialog_frame(look))
            .default_pos(pos)
            .fade_in(false)
            .default_width(WIDTH)
            .min_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                if widgets::dialog_title(ui, look, Icon::Warning, "Constraint Conflict") {
                    cancel = true;
                }
                widgets::dialog_body(ui, |ui| {
                    ui.label(egui::RichText::new(&heading).color(look.c(|t| t.text)));
                    ui.add_space(widgets::S2);
                    let muted = look.c(|t| t.text_muted);
                    if d.involved.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "No single constraint stands in its way: remove or \
                                     change several, or drag the geometry nearer first.",
                                )
                                .color(muted),
                            )
                            .wrap(),
                        );
                    } else {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Removing any one of these lets it in. Pick the one to \
                                     replace:",
                                )
                                .color(muted),
                            )
                            .wrap(),
                        );
                        ui.add_space(widgets::S2);
                        for (id, name) in &d.involved {
                            ui.horizontal(|ui| {
                                ui.label(name.as_str());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if widgets::secondary_button(ui, look, "Replace")
                                            .on_hover_text(format!(
                                                "Delete {name} and add {}",
                                                d.title
                                            ))
                                            .clicked()
                                        {
                                            replace = Some(*id);
                                        }
                                    },
                                );
                            });
                        }
                    }
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
                    if widgets::secondary_button(ui, look, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            cancel = true;
        }
        if let Some(id) = replace {
            self.conflict_replace(id);
        } else if cancel {
            self.sketch.conflict = None;
        }
    }

    /// Replace in the conflict dialog: deletes `id` and adds the new constraints, as one
    /// step.
    pub(crate) fn conflict_replace(&mut self, id: u64) {
        let Some(d) = self.sketch.conflict.clone() else {
            return;
        };
        let params = match d.constraints.as_slice() {
            [one] => json!({ "constraint": one, "replace": id }),
            many => json!({ "constraints": many, "replace": id }),
        };
        match self.session.execute("sketch.constrain", &params) {
            Ok(_) => {
                self.session.sketch_selection.clear();
                self.sketch.conflict = None;
                self.tutorial_saw("sketch.constrain");
                self.refresh_sketch_status();
            }
            Err(e) => {
                if let Some(c) = self.sketch.conflict.as_mut() {
                    c.error = Some(e.to_string());
                }
            }
        }
    }
}

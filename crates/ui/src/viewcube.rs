//! The view cube in the viewport's top-right corner: a small cube turned like the model,
//! with labelled faces to look straight at, a chamfered corner (the brand's corner) for
//! the isometric view, and Home and Fit buttons under it. Clicks run `view.*` commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_geom::Vec3;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};

/// The height the cube and its buttons take, from the viewport's top (points).
pub const CUBE_AREA: f32 = 176.0;
/// Half the cube's edge on screen (points, before projection).
const HALF: f32 = 30.0;
/// The chamfer's size along each edge, as a fraction of the half edge.
const CHAMFER: f64 = 0.5;

/// A face of the cube: the view it picks, its label, outward normal and corners.
struct Face {
    view: &'static str,
    text: &'static str,
    normal: [f64; 3],
    corners: [[f64; 3]; 4],
}

const FACES: [Face; 6] = [
    Face {
        view: "front",
        text: "FRONT",
        normal: [0.0, -1.0, 0.0],
        corners: [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
        ],
    },
    Face {
        view: "back",
        text: "BACK",
        normal: [0.0, 1.0, 0.0],
        corners: [
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
        ],
    },
    Face {
        view: "right",
        text: "RIGHT",
        normal: [1.0, 0.0, 0.0],
        corners: [
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, 1.0, 1.0],
            [1.0, -1.0, 1.0],
        ],
    },
    Face {
        view: "left",
        text: "LEFT",
        normal: [-1.0, 0.0, 0.0],
        corners: [
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
    },
    Face {
        view: "top",
        text: "TOP",
        normal: [0.0, 0.0, 1.0],
        corners: [
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
    },
    Face {
        view: "bottom",
        text: "BOTTOM",
        normal: [0.0, 0.0, -1.0],
        corners: [
            [-1.0, 1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, -1.0, -1.0],
            [-1.0, -1.0, -1.0],
        ],
    },
];

/// The corner the isometric view looks from: +X, -Y, +Z.
const ISO: [f64; 3] = [1.0, -1.0, 1.0];

fn v(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

fn same(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

/// Point-in-convex-polygon, either winding.
fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let mut sign = 0.0f32;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    sign != 0.0
}

impl ChamferaApp {
    /// The view cube and the Home and Fit buttons under it.
    pub(crate) fn view_cube(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let look = Look {
            theme: &self.theme,
            icons: &self.icons,
        };
        let basis = self.session.camera.basis();
        let toward_eye = -basis.forward;
        let centre = pos2(rect.max.x - 78.0, rect.min.y + 74.0);
        let scale = f64::from(HALF);
        let project = |p: Vec3| {
            pos2(
                centre.x + (p.dot(basis.right) * scale) as f32,
                centre.y - (p.dot(basis.up) * scale) as f32,
            )
        };
        // Visible faces, each with the iso corner cut off.
        let t = CHAMFER;
        let mut faces: Vec<(&Face, Vec<Pos2>, f64)> = Vec::new();
        for face in &FACES {
            let facing = v(face.normal).dot(toward_eye);
            if facing <= 0.02 {
                continue;
            }
            let mut poly = Vec::new();
            for k in 0..4 {
                let c = face.corners[k];
                if same(c, ISO) {
                    let prev = v(face.corners[(k + 3) % 4]);
                    let next = v(face.corners[(k + 1) % 4]);
                    let c = v(c);
                    poly.push(project(c + (prev - c) * (t * 0.5)));
                    poly.push(project(c + (next - c) * (t * 0.5)));
                } else {
                    poly.push(project(v(c)));
                }
            }
            faces.push((face, poly, facing));
        }
        let iso_normal = v(ISO) * (1.0 / 3f64.sqrt());
        let chamfer_facing = iso_normal.dot(toward_eye);
        let chamfer: Vec<Pos2> = [
            [1.0 - t, -1.0, 1.0],
            [1.0, -1.0 + t, 1.0],
            [1.0, -1.0, 1.0 - t],
        ]
        .iter()
        .map(|p| project(v(*p)))
        .collect();

        let area = Rect::from_center_size(centre, Vec2::splat(HALF * 4.0));
        let response = ui.interact(area, ui.scope_id().with("view cube"), Sense::click());
        let hover = response.hover_pos();
        let over_chamfer = chamfer_facing > 0.02 && hover.is_some_and(|p| inside(&chamfer, p));
        let over_face = if over_chamfer {
            None
        } else {
            hover.and_then(|p| faces.iter().position(|(_, poly, _)| inside(poly, p)))
        };

        let painter = ui.painter();
        let edge = Stroke::new(1.0, look.c(|t| t.cube_edge));
        for (k, (face, poly, facing)) in faces.iter().enumerate() {
            let fill = if over_face == Some(k) {
                look.c(|t| t.selected)
            } else {
                // Faces turned away a little are drawn a little darker, for depth.
                let shade = 0.82 + 0.18 * (*facing as f32);
                let c = look.c(|t| t.cube_face);
                Color32::from_rgba_unmultiplied(
                    (f32::from(c.r()) * shade) as u8,
                    (f32::from(c.g()) * shade) as u8,
                    (f32::from(c.b()) * shade) as u8,
                    c.a(),
                )
            };
            painter.add(Shape::convex_polygon(poly.clone(), fill, edge));
            if *facing > 0.3 {
                // The label, shrunk to fit the face as it turns away.
                // The face's centre, nudged away from the chamfered corner.
                let n = v(face.normal);
                let away = if face.corners.iter().any(|c| same(*c, ISO)) {
                    (n - v(ISO)) * 0.16
                } else {
                    Vec3::new(0.0, 0.0, 0.0)
                };
                let mid = project(n + away);
                let (lo, hi) = poly.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                    (lo.min(p.x), hi.max(p.x))
                });
                let room = (hi - lo) * 0.78;
                let alpha = ((*facing as f32 - 0.3) / 0.35).clamp(0.0, 1.0);
                let colour = if over_face == Some(k) {
                    look.c(|t| t.text)
                } else {
                    look.c(|t| t.text_muted)
                }
                .gamma_multiply(alpha);
                let full = painter.layout_no_wrap(
                    face.text.to_owned(),
                    FontId::proportional(10.0),
                    colour,
                );
                let size = if full.size().x > room {
                    10.0 * room / full.size().x.max(1.0)
                } else {
                    10.0
                };
                if size >= 7.0 {
                    painter.text(
                        mid,
                        Align2::CENTER_CENTER,
                        face.text,
                        FontId::proportional(size),
                        colour,
                    );
                }
            }
        }
        if chamfer_facing > 0.02 {
            let accent = look.c(|t| t.icon_accent);
            let fill = if over_chamfer {
                accent
            } else {
                accent.gamma_multiply(0.85)
            };
            painter.add(Shape::convex_polygon(chamfer.clone(), fill, edge));
        }
        // Axis colours along the three edges from the (-1, -1, -1) corner, where an edge
        // borders a visible face: X red, Y green, Z blue, as in the viewport.
        let origin = v([-1.0, -1.0, -1.0]);
        let seen = |n: [f64; 3]| v(n).dot(toward_eye) > 0.02;
        for (axis, sides, colour) in [
            (
                v([1.0, 0.0, 0.0]),
                [[0.0, -1.0, 0.0], [0.0, 0.0, -1.0]],
                look.c(|t| t.axis_x),
            ),
            (
                v([0.0, 1.0, 0.0]),
                [[-1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
                look.c(|t| t.axis_y),
            ),
            (
                v([0.0, 0.0, 1.0]),
                [[0.0, -1.0, 0.0], [-1.0, 0.0, 0.0]],
                look.c(|t| t.axis_z),
            ),
        ] {
            if sides.iter().any(|n| seen(*n)) {
                let (a, b) = (project(origin), project(origin + axis * 2.0));
                painter.line_segment([a, b], Stroke::new(2.0, colour));
            }
        }

        let mut view: Option<&str> = None;
        let response = if over_chamfer {
            response.on_hover_text("Isometric view")
        } else if let Some(k) = over_face {
            let text = faces.get(k).map_or("", |(f, _, _)| f.text);
            response.on_hover_text(format!("Look at the {} face", text.to_lowercase()))
        } else {
            response
        };
        if response.clicked() {
            if over_chamfer {
                view = Some("iso");
            } else if let Some(k) = over_face {
                view = faces.get(k).map(|(f, _, _)| f.view);
            }
        }

        // Home and Fit under the cube.
        let mut home = false;
        let mut fit = false;
        let bar = Rect::from_center_size(pos2(centre.x, rect.min.y + 152.0), vec2(64.0, 30.0));
        ui.painter().rect(
            bar,
            egui::CornerRadius::same(widgets::R_MEDIUM),
            look.c(|t| t.panel).gamma_multiply(0.94),
            Stroke::new(1.0, look.c(|t| t.border)),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink(2.0))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 4.0;
        if widgets::icon_button(&mut child, look, Icon::Home, 26.0, 18.0, true, false)
            .on_hover_text(crate::shortcuts::tip(
                &format!("{}\nGo back to the start view", label("view.home")),
                crate::shortcuts::Action::Home,
            ))
            .clicked()
        {
            home = true;
        }
        if widgets::icon_button(&mut child, look, Icon::Fit, 26.0, 18.0, true, false)
            .on_hover_text(crate::shortcuts::tip(
                &format!("{}\nShow the whole model", label("view.fit")),
                crate::shortcuts::Action::Fit,
            ))
            .clicked()
        {
            fit = true;
        }
        if let Some(name) = view {
            self.standard_view(name);
        }
        if home {
            self.run("view.home", self.viewport_params());
        }
        if fit {
            self.run("view.fit", self.viewport_params());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_point_outward_and_their_corners_lie_on_them() {
        for f in &FACES {
            let n = v(f.normal);
            for c in f.corners {
                assert!((v(c).dot(n) - 1.0).abs() < 1e-12, "{}", f.text);
            }
            // Corners wind counter-clockwise seen from outside: (b - a) x (c - b) along n.
            let [a, b, c, _] = f.corners.map(v);
            assert!((b - a).cross(c - b).dot(n) > 0.0, "{}", f.text);
        }
        // The iso corner is on exactly three faces: front, right and top.
        let on: Vec<&str> = FACES
            .iter()
            .filter(|f| f.corners.iter().any(|c| same(*c, ISO)))
            .map(|f| f.view)
            .collect();
        assert_eq!(on, ["front", "right", "top"]);
    }

    #[test]
    fn point_in_polygon() {
        let square = [
            pos2(0.0, 0.0),
            pos2(10.0, 0.0),
            pos2(10.0, 10.0),
            pos2(0.0, 10.0),
        ];
        assert!(inside(&square, pos2(5.0, 5.0)));
        assert!(!inside(&square, pos2(15.0, 5.0)));
        let reversed: Vec<Pos2> = square.iter().rev().copied().collect();
        assert!(inside(&reversed, pos2(1.0, 9.0)));
    }
}

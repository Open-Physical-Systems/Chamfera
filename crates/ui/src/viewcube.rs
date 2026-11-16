//! The view cube in the viewport's top-right corner: a small cube turned like the model,
//! with labelled faces. Each of its 26 regions (faces, edges, corners; the geometry is
//! `chamfera_render::viewcube`) is a view to click; a drag on the cube orbits. The
//! chamfered corner (the brand's corner) is the isometric view. Home and Fit buttons sit
//! under it. Clicks and drags run `view.*` commands.

use crate::icons::Icon;
use crate::widgets::{self, Look};
use crate::{ChamferaApp, label};
use chamfera_geom::Vec3;
use chamfera_render::viewcube::{BAND, CubeRegion, RegionKind, region_at};
use egui::{Align2, Color32, CursorIcon, FontId, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};
use serde_json::json;

/// The height the cube and its buttons take, from the viewport's top (points).
pub const CUBE_AREA: f32 = 176.0;
/// Half the cube's edge on screen (points, before projection).
const HALF: f32 = 30.0;

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

/// The axis (0 = X, 1 = Y, 2 = Z) and sign of a face's outward normal.
fn face_axis(normal: [f64; 3]) -> Option<(usize, i8)> {
    let i = normal.iter().position(|c| c.abs() > 0.5)?;
    Some((i, if normal.get(i)? > &0.0 { 1 } else { -1 }))
}

/// The `view.standard` name for a region: the iso corner keeps its own name.
fn view_name(r: CubeRegion) -> String {
    if r.0 == [1, -1, 1] {
        "iso".to_owned()
    } else {
        r.name()
    }
}

/// The tooltip for a region.
fn region_tip(r: CubeRegion) -> String {
    let words = r.name().replace('_', " ");
    let what = match r.kind() {
        RegionKind::Face => format!("Look at the {words} face"),
        RegionKind::Edge => format!("Look from the {words} edge"),
        RegionKind::Corner if r.0 == [1, -1, 1] => "Isometric view".to_owned(),
        RegionKind::Corner => format!("Look from the {words} corner"),
    };
    format!("{what}\nDrag the cube to orbit")
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
        // Visible faces, each with the iso corner cut off where the corner region ends.
        let cut = BAND * 0.5; // of an edge 2 half edges long
        let mut faces: Vec<(&Face, Vec<egui::Pos2>, f64)> = Vec::new();
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
                    poly.push(project(c + (prev - c) * cut));
                    poly.push(project(c + (next - c) * cut));
                } else {
                    poly.push(project(v(c)));
                }
            }
            faces.push((face, poly, facing));
        }
        let iso_normal = v(ISO) * (1.0 / 3f64.sqrt());
        let chamfer_facing = iso_normal.dot(toward_eye);
        let chamfer: Vec<egui::Pos2> = [
            [1.0 - BAND, -1.0, 1.0],
            [1.0, -1.0 + BAND, 1.0],
            [1.0, -1.0, 1.0 - BAND],
        ]
        .iter()
        .map(|p| project(v(*p)))
        .collect();

        let area = Rect::from_center_size(centre, Vec2::splat(HALF * 4.0));
        let response = ui.interact(
            area,
            ui.scope_id().with("view cube"),
            Sense::click_and_drag(),
        );
        // The region under the pointer, from the same projection the cube is drawn with.
        let hover = response
            .hover_pos()
            .filter(|_| !response.dragged())
            .and_then(|p| {
                let sx = f64::from((p.x - centre.x) / HALF);
                let sy = f64::from((centre.y - p.y) / HALF);
                region_at(&basis, sx, sy)
            });

        let painter = ui.painter();
        let edge = Stroke::new(1.0, look.c(|t| t.cube_edge));
        for (_, poly, facing) in &faces {
            // Faces turned away a little are drawn a little darker, for depth.
            let shade = 0.82 + 0.18 * (*facing as f32);
            let c = look.c(|t| t.cube_face);
            let fill = Color32::from_rgba_unmultiplied(
                (f32::from(c.r()) * shade) as u8,
                (f32::from(c.g()) * shade) as u8,
                (f32::from(c.b()) * shade) as u8,
                c.a(),
            );
            painter.add(Shape::convex_polygon(poly.clone(), fill, edge));
        }
        // The hovered region's patch on every visible face it touches.
        if let Some(r) = hover {
            for (face, _, _) in &faces {
                if let Some((axis, sign)) = face_axis(face.normal)
                    && let Some(quad) = r.patch(axis, sign)
                {
                    let pts: Vec<egui::Pos2> = quad.iter().map(|p| project(*p)).collect();
                    painter.add(Shape::convex_polygon(
                        pts,
                        look.c(|t| t.selected),
                        Stroke::NONE,
                    ));
                }
            }
        }
        for (face, poly, facing) in &faces {
            if *facing <= 0.3 {
                continue;
            }
            // The label, shrunk to fit the face as it turns away, at the face's centre
            // nudged away from the chamfered corner.
            let n = v(face.normal);
            let away = if face.corners.iter().any(|c| same(*c, ISO)) {
                (n - v(ISO)) * 0.12
            } else {
                Vec3::new(0.0, 0.0, 0.0)
            };
            let mid = project(n + away);
            let (lo, hi) = poly.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p.x), hi.max(p.x))
            });
            let room = (hi - lo) * 0.78;
            let alpha = ((*facing as f32 - 0.3) / 0.35).clamp(0.0, 1.0);
            let lit = hover.is_some_and(|r| r.name() == face.view);
            let colour = if lit {
                look.c(|t| t.text)
            } else {
                look.c(|t| t.text_muted)
            }
            .gamma_multiply(alpha);
            let full =
                painter.layout_no_wrap(face.text.to_owned(), FontId::proportional(10.0), colour);
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
        if chamfer_facing > 0.02 {
            let accent = look.c(|t| t.icon_accent);
            let fill = if hover.is_some_and(|r| r.0 == [1, -1, 1]) {
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

        // A drag on the cube orbits, through `view.orbit` like a drag in the viewport.
        let mut orbit = None;
        if response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let d = response.drag_delta();
            if d != Vec2::ZERO {
                let k = crate::navigation::ORBIT_PER_PX;
                orbit = Some((
                    (-f64::from(d.x) * k).clamp(-6.0, 6.0),
                    (f64::from(d.y) * k).clamp(-6.0, 6.0),
                ));
            }
        }
        let response = match hover {
            Some(r) => response.on_hover_text(region_tip(r)),
            None => response,
        };
        let view = if response.clicked() {
            hover.map(view_name)
        } else {
            None
        };

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
        if let Some((yaw, pitch)) = orbit {
            self.run("view.orbit", json!({ "yaw": yaw, "pitch": pitch }));
        }
        if let Some(name) = view {
            self.standard_view(&name);
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
    fn each_face_names_its_own_view_and_axis() {
        for f in &FACES {
            let (axis, sign) = face_axis(f.normal).unwrap();
            let mut r = [0i8; 3];
            r[axis] = sign;
            assert_eq!(CubeRegion(r).name(), f.view);
        }
        assert_eq!(view_name(CubeRegion([1, -1, 1])), "iso");
        assert_eq!(view_name(CubeRegion([-1, -1, 1])), "top_front_left");
        assert!(region_tip(CubeRegion([0, -1, 1])).starts_with("Look from the top front edge"));
    }

    #[test]
    fn a_click_on_a_cube_corner_turns_the_view_to_it() {
        // Through the same command path a cube click takes.
        let mut app = ChamferaApp::headless();
        app.standard_view(&view_name(CubeRegion([-1, 1, 1])));
        let c = &app.session.camera;
        let to_eye = (c.eye() - c.target).normalized().unwrap();
        let want = Vec3::new(-1.0, 1.0, 1.0) * (1.0 / 3f64.sqrt());
        assert!((to_eye - want).length() < 1e-9, "{to_eye:?}");
    }
}

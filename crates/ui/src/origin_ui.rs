//! The origin in the UI: the Browser's Origin folder (an eye per item runs
//! `view.origin`), clicks on shown origin planes and axes (sketch on a plane, fill a
//! construction tool's input) and "select through" with Alt+click.

use crate::browser::{Row, tree_row};
use crate::icons::Icon;
use crate::{ChamferaApp, label};
use serde_json::{Value, json};

/// The origin point and axes in the Browser, before the planes (`sketch_ui::PLANES`).
const POINT_AND_AXES: [(&str, &str, Icon); 4] = [
    ("origin", "Origin point", Icon::ConstructPoint),
    ("x", "X axis", Icon::ConstructAxis),
    ("y", "Y axis", Icon::ConstructAxis),
    ("z", "Z axis", Icon::ConstructAxis),
];

/// How close (points) a second Alt+click must be to the first to go one further back.
const SAME_SPOT: f32 = 4.0;

/// Where the last select-through click was and how far back it went.
#[derive(Debug, Default, Clone, Copy)]
pub struct SelectThrough {
    pub at: Option<(f32, f32)>,
    pub depth: u64,
}

impl ChamferaApp {
    fn origin_shown(&self, item: &str) -> bool {
        chamfera_engine::origin::OriginItem::parse(item)
            .is_some_and(|i| self.session.origin_shown().get(i))
    }

    /// The rows of the Browser's Origin folder.
    pub(crate) fn browser_origin(&mut self, ui: &mut egui::Ui, sketching: bool) {
        let mut rows: Vec<(&str, &str, Icon, Option<&str>)> = POINT_AND_AXES
            .iter()
            .map(|(id, text, icon)| (*id, *text, *icon, None))
            .collect();
        rows.extend(
            crate::sketch_ui::PLANES
                .iter()
                .map(|(id, text, view)| (*id, *text, Icon::Plane, Some(*view))),
        );
        for (id, text, icon, view) in rows {
            let visible = self.origin_shown(id);
            let r = tree_row(
                ui,
                self.look(),
                ui.scope_id().with(("origin", id)),
                &Row {
                    depth: 2,
                    icon: Some(icon),
                    text,
                    dimmed: !visible,
                    eye: Some(visible),
                    ..Row::default()
                },
            );
            if let Some(eye) = r.eye {
                let tip = if visible {
                    "Hide in the viewport"
                } else {
                    "Show in the viewport"
                };
                if eye
                    .on_hover_text(format!("{}\n{tip}", label("view.origin")))
                    .clicked()
                {
                    self.run("view.origin", json!({ "item": id, "visible": !visible }));
                }
            }
            let Some(view) = view else {
                continue;
            };
            let row = r.row.on_hover_text(if sketching {
                "Finish the sketch to start another"
            } else {
                "Double-click to sketch on this plane"
            });
            if row.double_clicked() && !sketching {
                self.create_sketch(id, view);
            }
        }
    }

    /// The shown origin item under a pixel: its name (`xy`, `x`, `origin`) and kind.
    pub(crate) fn origin_under(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> Option<(String, String)> {
        let r = self
            .session
            .execute(
                "view.origin_at",
                &json!({ "x": x, "y": y, "width": w, "height": h }),
            )
            .ok()?;
        Some((
            r["item"].as_str()?.to_owned(),
            r["kind"].as_str()?.to_owned(),
        ))
    }

    /// A click while waiting for a face to sketch on, over a shown origin plane: sketches
    /// on that plane. False when no origin plane is there.
    pub(crate) fn origin_plane_click(&mut self, x: f64, y: f64, w: f64, h: f64) -> bool {
        let Some((item, kind)) = self.origin_under(x, y, w, h) else {
            return false;
        };
        if kind != "plane" {
            return false;
        }
        let Some((plane, _, view)) = crate::sketch_ui::PLANES
            .iter()
            .find(|(p, _, _)| *p == item)
            .copied()
        else {
            return false;
        };
        self.solid.pick_face = false;
        self.create_sketch(plane, view);
        true
    }

    /// For a construction tool's input: the shown origin item under a pixel as the
    /// reference that input takes (`"xy"` for a plane, `"x"` for an axis, `"origin"` for
    /// a point), with its text, when it fits.
    pub(crate) fn origin_pick_for(
        &mut self,
        slot_takes: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> Option<(Value, String)> {
        let (item, kind) = self.origin_under(x, y, w, h)?;
        if kind != slot_takes {
            return None;
        }
        let text = match kind.as_str() {
            "plane" => format!("Origin plane {}", item.to_uppercase()),
            "axis" => format!("{} axis", item.to_uppercase()),
            _ => "Origin point".to_owned(),
        };
        Some((json!(item), text))
    }

    /// Alt+click: selects the body one further behind each time the same spot is
    /// clicked (`select.pick` with `cycle`), wrapping to the nearest after the last.
    pub(crate) fn select_through(&mut self, at: (f32, f32), w: f64, h: f64) {
        let again = self.select_through.at.is_some_and(|(px, py)| {
            (px - at.0).abs() <= SAME_SPOT && (py - at.1).abs() <= SAME_SPOT
        });
        let depth = if again {
            self.select_through.depth.saturating_add(1)
        } else {
            1
        };
        let Some(r) = self.run(
            "select.pick",
            json!({ "x": at.0, "y": at.1, "width": w, "height": h, "cycle": depth }),
        ) else {
            return;
        };
        let count = r["candidates"].as_array().map_or(0, Vec::len);
        self.select_through = SelectThrough {
            at: Some(at),
            depth,
        };
        self.message = Some((
            match count {
                0 => "Nothing under the cursor".to_owned(),
                1 => "Only one body under the cursor".to_owned(),
                n => {
                    let k = depth % n as u64;
                    format!(
                        "{} ({} of {n} under the cursor; Alt+click again for the next)",
                        r["name"].as_str().unwrap_or("Body"),
                        k + 1
                    )
                }
            },
            false,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::Frame;

    #[test]
    fn a_click_on_a_shown_origin_plane_starts_a_sketch_on_it() {
        let mut app = ChamferaApp::headless();
        // From the front, the XZ plane faces the eye and the centre pixel's ray meets it
        // at the target (the origin).
        app.run("view.standard", json!({ "view": "front" }));
        app.start_sketch_on_face();
        // Hidden: nothing to click.
        assert!(!app.origin_plane_click(400.0, 300.0, 800.0, 600.0));
        app.run("view.origin", json!({ "item": "xz", "visible": true }));
        assert!(app.origin_plane_click(400.0, 300.0, 800.0, 600.0));
        assert!(!app.solid.pick_face);
        let sk = app.session.active_sketch.unwrap();
        assert_eq!(app.session.doc.sketch(sk).unwrap().plane, Frame::XZ);
    }

    #[test]
    fn alt_clicks_on_one_spot_go_one_body_further_back_each_time() {
        let mut app = ChamferaApp::headless();
        // Two 10 mm cubes stacked 20 apart in Z, seen from the top over their middle.
        for z in [0.0, 20.0] {
            app.run("sketch.create", json!({ "plane": "xy" }));
            app.run(
                "sketch.rectangle",
                json!({ "x1": 0, "y1": 0, "x2": 10, "y2": 10 }),
            );
            let sk = app.session.active_sketch.unwrap();
            app.run("sketch.finish", json!({}));
            app.run(
                "solid.extrude",
                json!({ "sketch": sk, "profiles": [{ "x": 5, "y": 5 }], "distance": 10,
                        "start_offset": z }),
            );
        }
        app.run("view.standard", json!({ "view": "top" }));
        app.session.camera.target = chamfera_geom::Point3::new(5.0, 5.0, 0.0);
        let picked = |app: &ChamferaApp| {
            app.session
                .scene
                .selection
                .and_then(|h| app.session.scene.body(h.body))
                .map(|b| b.mesh.bounds().unwrap().max.z)
        };
        // The first Alt+click goes one behind the nearest (the high cube, top z 30):
        // the low one, top z 10. The next wraps to the nearest again.
        app.select_through((400.0, 300.0), 800.0, 600.0);
        let first = picked(&app);
        app.select_through((401.0, 300.0), 800.0, 600.0);
        let second = picked(&app);
        assert_eq!(first, Some(10.0));
        assert_eq!(second, Some(30.0));
        assert!(app.message.as_ref().is_some_and(|m| m.0.contains("of 2")));
    }
}

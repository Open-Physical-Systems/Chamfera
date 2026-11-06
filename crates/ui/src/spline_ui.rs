//! The Spline tool of sketch mode: click the fit points one after another; a second
//! click on the last point (a double-click) or Enter finishes the spline, a click on the
//! first point closes it, Esc cancels. Clicks on existing sketch points reuse them. The
//! spline is made in one `sketch.spline` command when it is finished; until then the
//! preview shows the curve through the clicks and the cursor.
//!
//! `sketch_ui.rs` hands the tool its clicks (`spline_click`), Enter (`spline_finish`)
//! and the preview (`spline_preview`); everything else is here.

use crate::ChamferaApp;
use crate::sketch_ui::Click;
use chamfera_engine::sketching::PICK_POINT_PX;
use chamfera_geom::{BSplineCurve2, Point2};
use serde_json::{Value, json};

/// The tool's tooltip.
pub(crate) const TOOLTIP: &str = "Spline\nA smooth curve through the points you click. \
     Double-click or press Enter to finish; click the first point to close it";

/// Preview points per knot span of the curve being drawn.
const PREVIEW_SAMPLES_PER_SPAN: usize = 16;

/// The status hint after `clicks` points.
pub(crate) fn hint(clicks: usize) -> &'static str {
    match clicks {
        0 => "Click the first point of the spline",
        1 => "Click the next point · Esc cancels",
        _ => "Click more points · double-click or Enter finishes · click the first point to close",
    }
}

impl ChamferaApp {
    /// A click of the Spline tool at pixel `px = (x, y, w, h)`, landing at `click`.
    pub(crate) fn spline_click(&mut self, click: Click, px: (f64, f64, f64, f64)) {
        let (x, y, w, h) = px;
        // How far (mm) the pick radius reaches at this zoom.
        let reach = match (
            self.session.sketch_plane_point(x, y, w, h),
            self.session.sketch_plane_point(x + PICK_POINT_PX, y, w, h),
        ) {
            (Some(a), Some(b)) => a.distance(b),
            _ => 0.0,
        };
        let same = |a: &Click, b: &Click| {
            (a.point.is_some() && a.point == b.point) || a.at.distance(b.at) <= reach
        };
        let clicks = &self.sketch.clicks;
        if let Some(last) = clicks.last()
            && same(last, &click)
        {
            self.spline_finish(false);
            return;
        }
        if clicks.len() >= 3
            && let Some(first) = clicks.first()
            && same(first, &click)
        {
            self.spline_finish(true);
            return;
        }
        self.sketch.clicks.push(click);
    }

    /// Makes the spline from the clicks so far (closed or open) and ends the tool's run.
    pub(crate) fn spline_finish(&mut self, closed: bool) {
        let clicks = std::mem::take(&mut self.sketch.clicks);
        let least = if closed { 3 } else { 2 };
        if clicks.len() < least {
            if !clicks.is_empty() {
                self.message = Some((
                    "A spline needs at least two points: keep clicking".into(),
                    true,
                ));
                self.sketch.clicks = clicks;
            }
            return;
        }
        let points: Vec<Value> = clicks
            .iter()
            .map(|c| match c.point {
                Some(id) => json!({ "point": id }),
                None => json!({ "x": c.at.x, "y": c.at.y }),
            })
            .collect();
        self.run_sketch(
            "sketch.spline",
            json!({ "points": points, "kind": "fit", "closed": closed }),
        );
    }

    /// The curve through the clicks and the cursor, in sketch coordinates.
    pub(crate) fn spline_preview(&self, hover: Point2) -> Vec<Vec<Point2>> {
        let mut pts: Vec<Point2> = self.sketch.clicks.iter().map(|c| c.at).collect();
        if pts
            .last()
            .is_none_or(|l| l.distance(hover) > chamfera_geom::LINEAR_TOLERANCE)
        {
            pts.push(hover);
        }
        if pts.len() < 2 {
            return Vec::new();
        }
        match BSplineCurve2::interpolate(&pts, None, None) {
            Ok(c) => vec![c.polyline(PREVIEW_SAMPLES_PER_SPAN)],
            Err(_) => vec![pts],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ChamferaApp;
    use crate::sketch_ui::Tool;
    use serde_json::json;

    /// A top view of the XY plane, 800 x 600, as in the sketch tool tests: `px(x, y)` is the
    /// pixel of sketch point (x, y).
    fn app_on_top_plane() -> (ChamferaApp, impl Fn(f64, f64) -> (f64, f64)) {
        let mut app = ChamferaApp::headless();
        app.create_sketch("xy", "top");
        app.session.camera.target = chamfera_geom::Point3::new(40.0, 25.0, 0.0);
        app.session.camera.projection = chamfera_render::Projection::Orthographic;
        let cam = app.session.camera;
        let per_mm = 600.0 / (2.0 * cam.half_height());
        let px = move |x: f64, y: f64| (400.0 + (x - 40.0) * per_mm, 300.0 - (y - 25.0) * per_mm);
        (app, px)
    }

    fn click(app: &mut ChamferaApp, p: (f64, f64)) {
        app.sketch_click(p.0, p.1, 800.0, 600.0, false);
    }

    #[test]
    fn clicks_then_a_double_click_make_a_fit_spline_through_them() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Spline);
        for (x, y) in [(0.0, 0.0), (20.0, 15.0), (40.0, 0.0), (60.0, 10.0)] {
            click(&mut app, px(x, y));
        }
        // Nothing is made until the spline is finished.
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        assert!(g["curves"].as_array().unwrap().is_empty());
        // The second click of a double-click lands on the last point.
        click(&mut app, px(60.0, 10.0));
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        let curves = g["curves"].as_array().unwrap();
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0]["geometry"]["kind"], "spline");
        assert_eq!(g["points"].as_array().unwrap().len(), 4);
        // The curve passes through the clicked points (to the pixel's precision).
        let sk = &app.session.active_sketch().unwrap().sketch;
        let (id, _) = sk.curves().next().unwrap();
        let c = sk.spline_curve(id).unwrap();
        let per_mm = px(1.0, 0.0).0 - px(0.0, 0.0).0;
        for (x, y) in [(0.0, 0.0), (20.0, 15.0), (40.0, 0.0), (60.0, 10.0)] {
            let d = c.closest_point(chamfera_geom::Point2::new(x, y)).distance;
            assert!(d < 1.0 / per_mm, "({x}, {y}) is {d} mm away");
        }
        assert!(app.sketch.clicks.is_empty());
    }

    #[test]
    fn clicking_the_first_point_closes_the_spline_into_a_profile() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Spline);
        for (x, y) in [
            (0.0, 0.0),
            (40.0, 0.0),
            (40.0, 40.0),
            (0.0, 40.0),
            (0.0, 0.0),
        ] {
            click(&mut app, px(x, y));
        }
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        assert_eq!(g["curves"][0]["geometry"]["closed"], true);
        // A smooth closed curve through the corners of a 40 mm square bulges outside it.
        let area = g["profiles"][0]["area"].as_f64().unwrap();
        assert!(area > 1600.0 && area < 3200.0, "{area}");
    }

    #[test]
    fn enter_finishes_and_one_point_is_not_a_spline() {
        let (mut app, px) = app_on_top_plane();
        app.sketch.tool = Some(Tool::Spline);
        click(&mut app, px(0.0, 0.0));
        app.spline_finish(false);
        assert_eq!(app.sketch.clicks.len(), 1, "one point waits for more");
        click(&mut app, px(30.0, 20.0));
        app.spline_finish(false);
        let g = app.session.execute("sketch.get", &json!({})).unwrap();
        assert_eq!(g["curves"].as_array().unwrap().len(), 1);
        // The preview follows the cursor.
        click(&mut app, px(0.0, 30.0));
        assert!(
            !app.spline_preview(chamfera_geom::Point2::new(10.0, 40.0))
                .is_empty()
        );
    }
}

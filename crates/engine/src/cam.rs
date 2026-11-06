//! Manufacturing in the session: toolpaths worked out from the design's CAM section and
//! the built solids, after every rebuild, and drawn as viewport overlays.

use crate::Session;
use chamfera_cam::Toolpath;
use chamfera_geom::Point3;
use chamfera_render::{Overlay, OverlayStyle};
use std::collections::BTreeMap;

/// Dash and gap lengths of a rapid move on screen (mm along the move).
const DASH: f64 = 2.0;
const GAP: f64 = 1.5;
/// Most overlay lines all toolpaths together may add (the scene has its own cap).
const MAX_TOOLPATH_LINES: usize = 400_000;

/// The toolpath of each operation (or why it could not be made), and whether they show.
#[derive(Debug, Default, Clone)]
pub struct CamState {
    pub paths: BTreeMap<u32, Result<Toolpath, String>>,
    pub visible: bool,
}

/// `[a, b]` as dashes.
fn dashes(a: Point3, b: Point3, out: &mut Vec<[Point3; 2]>) {
    let d = b - a;
    let len = d.length();
    if !(len.is_finite() && len > 0.0) {
        return;
    }
    let mut t = 0.0;
    while t < len && out.len() < MAX_TOOLPATH_LINES {
        let e = (t + DASH).min(len);
        out.push([a + d * (t / len), a + d * (e / len)]);
        t = e + GAP;
    }
}

impl Session {
    /// Works out every operation's toolpath from the model as it is now.
    pub fn regenerate_cam(&mut self) {
        self.cam.paths.clear();
        if self.doc.cam.operations.is_empty() {
            return;
        }
        let bodies: Vec<chamfera_kernel::Body> =
            self.built.solids.iter().map(|s| s.body.clone()).collect();
        for op in &self.doc.cam.operations {
            let r = if op.suppressed {
                Err("suppressed".to_owned())
            } else {
                chamfera_cam::generate(&self.doc.cam, op, &bodies, crate::solids::edge_ref)
                    .map_err(|e| e.to_string())
            };
            self.cam.paths.insert(op.id, r);
        }
    }

    /// The toolpath of operation `id`, or why there is none.
    pub fn toolpath(&self, id: u32) -> Option<&Result<Toolpath, String>> {
        self.cam.paths.get(&id)
    }

    /// True when toolpaths are drawn in the viewport.
    pub fn toolpaths_visible(&self) -> bool {
        self.cam.visible
    }

    /// Shows or hides the toolpaths.
    pub(crate) fn set_toolpaths_visible(&mut self, visible: bool) {
        self.cam.visible = visible;
    }

    /// Feeds solid, rapids dashed, over the model.
    pub(crate) fn toolpath_overlays(&self) -> Vec<Overlay> {
        if !self.cam.visible {
            return Vec::new();
        }
        let mut feeds = Vec::new();
        let mut rapids = Vec::new();
        for tp in self.cam.paths.values().flatten() {
            let (r, f) = tp.polylines(0.1);
            for [a, b] in r {
                dashes(a, b, &mut rapids);
            }
            feeds.extend(f);
            if feeds.len() + rapids.len() > MAX_TOOLPATH_LINES {
                feeds.truncate(MAX_TOOLPATH_LINES);
                break;
            }
        }
        let mut out = Vec::new();
        for (style, lines) in [
            (OverlayStyle::ToolpathRapid, rapids),
            (OverlayStyle::ToolpathFeed, feeds),
        ] {
            if !lines.is_empty() {
                out.push(Overlay {
                    style,
                    lines,
                    points: Vec::new(),
                    on_top: true,
                });
            }
        }
        out
    }
}

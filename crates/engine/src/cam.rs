//! Manufacturing in the session: toolpaths worked out from the design's CAM section and
//! the built solids, after every rebuild, and drawn as viewport overlays.

use crate::Session;
use chamfera_cam::Toolpath;
use chamfera_cam::drop::{DropModel, ToolShape};
use chamfera_cam::sim::{self, Dexels, Report};
use chamfera_geom::Point3;
use chamfera_render::{Overlay, OverlayStyle, StockMesh, StockStyle};
use std::collections::BTreeMap;

/// Dash and gap lengths of a rapid move on screen (mm along the move).
const DASH: f64 = 2.0;
const GAP: f64 = 1.5;
/// Most overlay lines all toolpaths together may add (the scene has its own cap).
const MAX_TOOLPATH_LINES: usize = 400_000;

/// Display cells per side of a simulated stock's mesh (the simulation itself may be
/// finer; blocks of columns show at their lowest).
const STOCK_MESH_SIDE: usize = 256;

/// The toolpath of each operation (or why it could not be made), whether they show, and
/// the stock simulation.
#[derive(Debug, Default, Clone)]
pub struct CamState {
    pub paths: BTreeMap<u32, Result<Toolpath, String>>,
    pub visible: bool,
    pub sim: Option<SimState>,
    /// The last 3D toolpaths and what they were made from (the model, the operation, its
    /// setup and tool): these take seconds, so they are made again only when one of those
    /// changed.
    cache: BTreeMap<u32, Cached>,
}

#[derive(Debug, Clone)]
struct Cached {
    bodies: Vec<chamfera_kernel::Body>,
    inputs: (
        chamfera_doc::cam::CamOperation,
        Option<chamfera_doc::cam::Setup>,
        Option<chamfera_doc::cam::Tool>,
    ),
    result: Result<Toolpath, String>,
}

/// A stock simulation: what it simulates, the part's heights over its columns (kept, so
/// moving the time slider only cuts again) and the result at `time`.
#[derive(Debug, Clone)]
pub struct SimState {
    pub setup: u32,
    pub ops: Vec<u32>,
    pub resolution: usize,
    pub tolerance: f64,
    pub time: f64,
    pub blank: Dexels,
    pub heights: Vec<Option<f64>>,
    pub report: Report,
    pub cell: f64,
}

/// What `cam.simulate` asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct SimRequest {
    pub setup: Option<u32>,
    pub ops: Option<Vec<u32>>,
    pub resolution: usize,
    pub tolerance: f64,
    pub time: f64,
}

/// The cutter shape a tool sweeps in the simulation: drills and chamfer mills as flat
/// cylinders of their diameter (their points are not modelled).
fn sim_shape(t: &chamfera_doc::cam::Tool) -> Result<ToolShape, String> {
    use chamfera_doc::cam::ToolKind;
    let r = t.diameter / 2.0;
    let corner = match t.kind {
        ToolKind::BallEndMill => r,
        ToolKind::BullNose => t.corner_radius,
        _ => 0.0,
    };
    ToolShape::new(r, corner).map_err(|e| e.to_string())
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
        // The model or the operations changed: an old simulation no longer holds.
        self.clear_simulation();
        if self.doc.cam.operations.is_empty() {
            return;
        }
        let bodies: Vec<chamfera_kernel::Body> =
            self.built.solids.iter().map(|s| s.body.clone()).collect();
        let mut cache = std::mem::take(&mut self.cam.cache);
        for op in &self.doc.cam.operations {
            let inputs = (
                op.clone(),
                self.doc.cam.setup(op.setup).cloned(),
                self.doc.cam.tool(op.tool).cloned(),
            );
            let three_d = matches!(
                op.op,
                chamfera_doc::cam::OpKind::Parallel { .. }
                    | chamfera_doc::cam::OpKind::Waterline { .. }
                    | chamfera_doc::cam::OpKind::Adaptive { .. }
            );
            if three_d
                && let Some(c) = cache.get(&op.id)
                && c.inputs == inputs
                && c.bodies == bodies
            {
                self.cam.paths.insert(op.id, c.result.clone());
                continue;
            }
            let r = if op.suppressed {
                Err("suppressed".to_owned())
            } else {
                chamfera_cam::generate(&self.doc.cam, op, &bodies, crate::solids::edge_ref)
                    .map_err(|e| e.to_string())
            };
            if three_d {
                cache.insert(
                    op.id,
                    Cached {
                        bodies: bodies.clone(),
                        inputs,
                        result: r.clone(),
                    },
                );
            }
            self.cam.paths.insert(op.id, r);
        }
        cache.retain(|id, _| self.doc.cam.operation(*id).is_some());
        self.cam.cache = cache;
    }

    /// Ends the stock simulation and takes its stock out of the viewport.
    pub fn clear_simulation(&mut self) {
        self.cam.sim = None;
        self.scene.stock.clear();
    }

    /// The stock simulation, if one shows.
    pub fn simulation(&self) -> Option<&SimState> {
        self.cam.sim.as_ref()
    }

    /// Simulates the setup's operations (or `ops`) on a dexel stock and shows it. With
    /// the same setup, operations and resolution as the last run only the cut is redone
    /// (the time slider).
    pub fn simulate(&mut self, req: &SimRequest) -> Result<&SimState, String> {
        if !(req.time.is_finite() && (0.0..=1.0).contains(&req.time)) {
            return Err("`time` must be from 0 to 1".into());
        }
        if !(req.tolerance.is_finite() && req.tolerance >= 0.0 && req.tolerance <= 10.0) {
            return Err("`tolerance` must be from 0 to 10 mm".into());
        }
        let setup_id = match req.setup {
            Some(id) => id,
            None => self
                .doc
                .cam
                .setups
                .first()
                .map(|s| s.id)
                .ok_or_else(|| "there is no setup to simulate".to_owned())?,
        };
        let setup = self
            .doc
            .cam
            .setup(setup_id)
            .cloned()
            .ok_or_else(|| format!("no setup {setup_id}"))?;
        let ops: Vec<u32> = match &req.ops {
            Some(list) => {
                for id in list {
                    let o = self
                        .doc
                        .cam
                        .operation(*id)
                        .ok_or_else(|| format!("no operation {id}"))?;
                    if o.setup != setup_id {
                        return Err(format!("operation {id} is in another setup"));
                    }
                }
                list.clone()
            }
            None => self
                .doc
                .cam
                .operations
                .iter()
                .filter(|o| o.setup == setup_id && !o.suppressed)
                .map(|o| o.id)
                .collect(),
        };
        if ops.is_empty() {
            return Err("the setup has no operations to simulate".into());
        }
        if self.cam.paths.is_empty() {
            self.regenerate_cam();
        }
        let same =
            self.cam.sim.as_ref().is_some_and(|s| {
                s.setup == setup_id && s.ops == ops && s.resolution == req.resolution
            });
        let (blank, heights, cell) = if same {
            match self.cam.sim.take() {
                Some(s) => (s.blank, s.heights, s.cell),
                None => return Err("no simulation".into()),
            }
        } else {
            let bodies: Vec<chamfera_kernel::Body> =
                self.built.solids.iter().map(|s| s.body.clone()).collect();
            let g = chamfera_cam::setup_geometry(&setup, &bodies, crate::solids::edge_ref)
                .map_err(|e| e.to_string())?;
            let blank =
                Dexels::new(g.stock_min, g.stock_max, req.resolution).map_err(|e| e.to_string())?;
            let model = DropModel::new(&bodies).map_err(|e| e.to_string())?;
            let heights = sim::part_heights(&model, &blank).map_err(|e| e.to_string())?;
            let cell = blank.cell;
            (blank, heights, cell)
        };
        // Each operation's share of the time, in order.
        let mut paths = Vec::new();
        for id in &ops {
            let o = self
                .doc
                .cam
                .operation(*id)
                .ok_or_else(|| format!("no operation {id}"))?;
            let tool = self
                .doc
                .cam
                .tool(o.tool)
                .ok_or_else(|| format!("no tool {}", o.tool))?;
            let shape = sim_shape(tool)?;
            match self.cam.paths.get(id) {
                Some(Ok(tp)) => paths.push((shape, tp.clone())),
                Some(Err(e)) => return Err(format!("{} has no toolpath: {e}", o.name)),
                None => return Err(format!("{} has no toolpath", o.name)),
            }
        }
        let rate = setup.rapid_rate;
        let minutes: Vec<f64> = paths.iter().map(|(_, tp)| tp.stats(rate).minutes).collect();
        let total: f64 = minutes.iter().sum();
        let mut left = total * req.time;
        let mut stock = blank.clone();
        for ((shape, tp), m) in paths.iter().zip(&minutes) {
            if req.time >= 1.0 || *m <= left {
                stock.cut_toolpath(shape, tp, rate, 1.0);
                left -= m;
            } else {
                if *m > 0.0 && left > 0.0 {
                    stock.cut_toolpath(shape, tp, rate, left / m);
                }
                break;
            }
        }
        let report = sim::report(&stock, &heights, req.tolerance);
        let (good, bad) = sim::meshes(&stock, &heights, req.tolerance, STOCK_MESH_SIDE);
        self.scene.stock.clear();
        for (style, mesh) in [(StockStyle::Stock, good), (StockStyle::Gouge, bad)] {
            if !mesh.triangles.is_empty() {
                self.scene.stock.push(StockMesh { style, mesh });
            }
        }
        self.cam.sim = Some(SimState {
            setup: setup_id,
            ops,
            resolution: req.resolution,
            tolerance: req.tolerance,
            time: req.time,
            blank,
            heights,
            report,
            cell,
        });
        self.cam.sim.as_ref().ok_or_else(|| "no simulation".into())
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

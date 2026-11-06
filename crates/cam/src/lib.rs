//! Layer 3: CAM, milestone M10: 2.5D and 3-axis milling, stock simulation and G-code.
//!
//! Our own, clean-room, from textbooks, papers and public specifications only: Held, "On
//! the Computational Geometry of Pocket Machining" (1991) for offsets and pocketing;
//! Choi and Jerard, "Sculptured Surface Machining" (1998) for toolpath conventions; the
//! NIST RS274/NGC interpreter specification v3 for G-code. No GPL or LGPL CAM code is
//! read, copied or linked.
//!
//! - [`path2`]: exact lines and arcs in XY; [`offset`]: offsets by trimming and joining;
//! - [`model`]: boundaries, pocket regions and holes read from the B-rep;
//! - [`ops`]: facing, contour, pocket and drilling; [`toolpath`]: moves and cycle time;
//! - [`drop`]: the exact drop-cutter for flat, ball and bull-nose tools; [`ops3d`]:
//!   parallel and waterline finishing and adaptive roughing; [`pencil`]: passes along
//!   concave corners; [`scallop`]: constant-stepover passes (experimental); [`sim`]:
//!   stock simulation on a dexel heightfield; [`grid`]: contours and distance transforms
//!   on grids;
//! - [`holder`]: the shank and holder, checked against the stock and the part;
//!   [`cancel`]: stopping a job on a worker thread that is no longer wanted;
//! - [`post`]: G-code for LinuxCNC and GRBL;
//! - [`generate`] and [`setup_geometry`]: the document's CAM section to toolpaths.
//!
//! Model decisions use the exact geometry; nothing here meshes.

pub mod cancel;
pub mod drop;
pub mod grid;
pub mod holder;
pub mod model;
pub mod offset;
pub mod ops;
pub mod ops3d;
pub mod path2;
pub mod pencil;
pub mod post;
pub mod scallop;
pub mod sim;
pub mod toolpath;

pub use ops::{Cutter, Hole, Region};
pub use post::{Dialect, Wcs};
pub use toolpath::{Move, Stats, Toolpath};

use chamfera_doc::cam::{Cam, CamOperation, OpKind, Setup, Stock, WcsOrigin};
use chamfera_doc::naming::{self, Found, Kind, NameError, Want};
use chamfera_geom::{Point2, Point3};
use chamfera_kernel::{Body, EdgeId, FaceId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum CamError {
    #[error("{0}")]
    Invalid(String),
    /// The model has something these operations cannot cut, and why.
    #[error("{0}")]
    Refused(String),
    #[error("too complex: {0}")]
    TooComplex(String),
    #[error("the model: {0}")]
    Model(String),
    /// A newer job was asked for while this one ran on a worker thread ([`cancel`]).
    #[error("cancelled: a newer toolpath job replaced this one")]
    Cancelled,
}

/// Names an edge the way the document refers to it (the engine's `edge_ref`).
pub type EdgeNamer = fn(&Body, EdgeId) -> Option<String>;

/// A setup's stock box, work coordinates and safe height, from the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetupGeometry {
    pub stock_min: Point3,
    pub stock_max: Point3,
    pub model_min: Point3,
    pub model_max: Point3,
    pub wcs: Wcs,
    /// Absolute height for rapid moves: the stock top plus the clearance.
    pub clearance_z: f64,
}

fn model_box(bodies: &[Body]) -> Result<(Point3, Point3), CamError> {
    let mut b: Option<chamfera_geom::Aabb> = None;
    for body in bodies {
        if let Some(x) = body.bounding_box() {
            b = Some(match b {
                Some(o) => o.union(x),
                None => x,
            });
        }
    }
    b.map(|b| (b.min, b.max))
        .ok_or_else(|| CamError::Refused("there is no model to machine".into()))
}

/// What `name` resolves to, through the design's persistent naming (rung K8,
/// `chamfera_doc::naming`): a piece of a split face by `#k`, a clear error when a single
/// face or edge is needed and the name split.
fn resolved(bodies: &[Body], kind: Kind, name: &str, want: Want) -> Result<Vec<Found>, CamError> {
    let list: Vec<&Body> = bodies.iter().collect();
    naming::resolve(&list, kind, name, None, want).map_err(|e| match e {
        NameError::Gone { kind, .. } => {
            CamError::Refused(format!("the {kind} {name:?} is no longer in the model"))
        }
        e => CamError::Refused(e.to_string()),
    })
}

fn find_face(bodies: &[Body], name: &str) -> Result<(usize, FaceId), CamError> {
    resolved(bodies, Kind::Face, name, Want::One)?
        .first()
        .and_then(|f| Some((f.body, f.face()?)))
        .ok_or_else(|| CamError::Refused(format!("the face {name:?} is no longer in the model")))
}

/// `_namer` is kept for callers; edge names follow `chamfera_doc::naming::edge_ref`.
fn find_edge(bodies: &[Body], name: &str, _namer: EdgeNamer) -> Result<(usize, EdgeId), CamError> {
    resolved(bodies, Kind::Edge, name, Want::One)?
        .first()
        .and_then(|f| Some((f.body, f.edge()?)))
        .ok_or_else(|| CamError::Refused(format!("the edge {name:?} is no longer in the model")))
}

/// The stock, work coordinates and clearance of `setup` for the model `bodies`.
pub fn setup_geometry(
    setup: &Setup,
    bodies: &[Body],
    namer: EdgeNamer,
) -> Result<SetupGeometry, CamError> {
    let (lo, hi) = model_box(bodies)?;
    let (smin, smax) = match setup.stock {
        Stock::FromModel { side, top, bottom } => (
            Point3::new(lo.x - side, lo.y - side, lo.z - bottom),
            Point3::new(hi.x + side, hi.y + side, hi.z + top),
        ),
        Stock::Fixed { x, y, z } => {
            let (cx, cy) = ((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
            (
                Point3::new(cx - x / 2.0, cy - y / 2.0, lo.z),
                Point3::new(cx + x / 2.0, cy + y / 2.0, lo.z + z),
            )
        }
    };
    let origin = match setup.origin {
        WcsOrigin::StockTopCorner => Point3::new(smin.x, smin.y, smax.z),
        WcsOrigin::StockTopCentre => {
            Point3::new((smin.x + smax.x) / 2.0, (smin.y + smax.y) / 2.0, smax.z)
        }
        WcsOrigin::StockBottomCorner => smin,
        WcsOrigin::ModelOrigin => Point3::new(0.0, 0.0, 0.0),
        WcsOrigin::Point { x, y, z } => Point3::new(x, y, z),
    };
    let angle = match &setup.x_axis {
        None => 0.0,
        Some(name) => {
            let (i, e) = find_edge(bodies, name, namer)?;
            let body = bodies
                .get(i)
                .ok_or_else(|| CamError::Model("missing body".into()))?;
            let h = body
                .edge_half_edges(e)
                .map_err(|x| CamError::Model(x.to_string()))?[0];
            match model::half_edge_seg(body, h)?.0 {
                path2::Seg::Line { a, b } => (b - a).angle(),
                path2::Seg::Arc { .. } => {
                    return Err(CamError::Refused("the X axis edge must be straight".into()));
                }
            }
        }
    };
    Ok(SetupGeometry {
        stock_min: smin,
        stock_max: smax,
        model_min: lo,
        model_max: hi,
        wcs: Wcs { origin, angle },
        clearance_z: smax.z + setup.clearance,
    })
}

/// The toolpath of one operation, worked out from the model `bodies` (whose faces and
/// edges carry the names the operation refers to).
pub fn generate(
    cam: &Cam,
    op: &CamOperation,
    bodies: &[Body],
    namer: EdgeNamer,
) -> Result<Toolpath, CamError> {
    let setup = cam
        .setup(op.setup)
        .ok_or_else(|| CamError::Invalid(format!("no setup {}", op.setup)))?;
    let tool = cam
        .tool(op.tool)
        .ok_or_else(|| CamError::Invalid(format!("no tool {}", op.tool)))?;
    let g = setup_geometry(setup, bodies, namer)?;
    let cutter = Cutter::from(tool);
    let top = g.stock_max.z;
    let body_of = |i: usize| {
        bodies
            .get(i)
            .ok_or_else(|| CamError::Model("missing body".into()))
    };
    match &op.op {
        OpKind::Facing { stepover, stepdown } => ops::facing(
            Point2::new(g.stock_min.x, g.stock_min.y),
            Point2::new(g.stock_max.x, g.stock_max.y),
            top,
            g.model_max.z,
            &cutter,
            *stepover,
            *stepdown,
            g.clearance_z,
        ),
        OpKind::Contour {
            face,
            edges,
            side,
            stepdown,
            lead_radius,
            bottom_offset,
            climb,
        } => {
            let (boundary, z) = match face {
                Some(name) => {
                    let (i, f) = find_face(bodies, name)?;
                    model::face_outline(body_of(i)?, f)?
                }
                None => {
                    let mut ids = Vec::new();
                    let mut body = None;
                    for name in edges {
                        let (i, e) = find_edge(bodies, name, namer)?;
                        if body.is_some_and(|b| b != i) {
                            return Err(CamError::Refused(
                                "the edges are on different bodies".into(),
                            ));
                        }
                        body = Some(i);
                        ids.push(e);
                    }
                    let i = body.ok_or_else(|| CamError::Invalid("no edges".into()))?;
                    model::edge_chain(body_of(i)?, &ids)?
                }
            };
            ops::contour(
                &boundary,
                z,
                *side,
                *climb,
                &cutter,
                top,
                *stepdown,
                *lead_radius,
                *bottom_offset,
                g.clearance_z,
            )
        }
        OpKind::Pocket {
            face,
            stepover,
            stepdown,
            entry,
            ramp_angle,
            helix_radius,
        } => {
            let (i, f) = find_face(bodies, face)?;
            let p = model::pocket_region(body_of(i)?, f)?;
            ops::pocket(
                &p.region,
                p.floor_z,
                top.max(p.top_z),
                &cutter,
                *stepover,
                *stepdown,
                *entry,
                *ramp_angle,
                *helix_radius,
                g.clearance_z,
            )
        }
        OpKind::Parallel {
            stepover,
            scallop,
            angle,
            tolerance,
            stock_to_leave,
            holder_check,
        } => {
            let shape = ops3d::tool_shape(tool)?;
            let dm = drop::DropModel::new(bodies)?;
            let cl = ops3d::Cl::new(&dm, shape, *stock_to_leave, g.model_min.z)?;
            let step = match scallop {
                Some(h) => {
                    if tool.kind != chamfera_doc::cam::ToolKind::BallEndMill {
                        return Err(CamError::Refused(
                            "a scallop height needs a ball end mill".into(),
                        ));
                    }
                    ops3d::scallop_stepover(shape.radius, *h)?
                }
                None => stepover * tool.diameter,
            };
            let check = holder_check_of(tool, *holder_check, *stock_to_leave)?;
            ops3d::parallel_with(
                &cl,
                &cutter,
                stock_area(&g),
                step,
                *angle,
                *tolerance,
                g.clearance_z,
                check.as_ref().map(|c| ops3d::HolderUse {
                    check: c,
                    mode: *holder_check,
                }),
            )
        }
        OpKind::Waterline {
            stepdown,
            tolerance,
            stock_to_leave,
        } => {
            let shape = ops3d::tool_shape(tool)?;
            let dm = drop::DropModel::new(bodies)?;
            let cl = ops3d::Cl::new(&dm, shape, *stock_to_leave, g.model_min.z)?;
            ops3d::waterline(
                &cl,
                &cutter,
                stock_area(&g),
                g.model_max.z.min(top),
                g.model_min.z,
                *stepdown,
                *tolerance,
                g.clearance_z,
            )
        }
        OpKind::Adaptive {
            stepdown,
            max_engagement,
            stock_to_leave,
            holder_check,
        } => {
            let shape = ops3d::tool_shape(tool)?;
            let dm = drop::DropModel::new(bodies)?;
            let cl = ops3d::Cl::new(&dm, shape, *stock_to_leave, g.stock_min.z)?;
            let check = holder_check_of(tool, *holder_check, *stock_to_leave)?;
            ops3d::adaptive_with(
                &cl,
                &cutter,
                ops3d::AdaptiveParams {
                    stock_min: Point2::new(g.stock_min.x, g.stock_min.y),
                    stock_max: Point2::new(g.stock_max.x, g.stock_max.y),
                    stock_top: top,
                    bottom: g.model_min.z.max(g.stock_min.z),
                    stepdown: *stepdown,
                    max_engagement: *max_engagement,
                    clearance: g.clearance_z,
                },
                check.as_ref().map(|c| ops3d::HolderUse {
                    check: c,
                    mode: *holder_check,
                }),
            )
            .map(|r| r.toolpath)
        }
        OpKind::Pencil {
            tolerance,
            stock_to_leave,
            min_angle,
        } => {
            let shape = ops3d::tool_shape(tool)?;
            let dm = drop::DropModel::new(bodies)?;
            let cl = ops3d::Cl::new(&dm, shape, *stock_to_leave, g.model_min.z)?;
            pencil::pencil(
                &cl,
                &cutter,
                stock_area(&g),
                *tolerance,
                *min_angle,
                g.clearance_z,
            )
        }
        OpKind::Scallop {
            stepover,
            tolerance,
            stock_to_leave,
            max_slope,
        } => {
            let shape = ops3d::tool_shape(tool)?;
            let dm = drop::DropModel::new(bodies)?;
            let cl = ops3d::Cl::new(&dm, shape, *stock_to_leave, g.model_min.z)?;
            scallop::scallop(
                &cl,
                &cutter,
                stock_area(&g),
                stepover * tool.diameter,
                *tolerance,
                *max_slope,
                g.clearance_z,
            )
        }
        OpKind::Drill {
            faces,
            peck,
            retract,
            breakthrough,
        } => {
            let holes = drill_holes(bodies, faces)?;
            ops::drill(
                &holes,
                &cutter,
                top + retract,
                *peck,
                *breakthrough,
                g.clearance_z,
            )
        }
    }
}

/// The part check of an operation's holder check, if it checks one: refused when the
/// tool has no holder to check.
fn holder_check_of(
    tool: &chamfera_doc::cam::Tool,
    mode: chamfera_doc::cam::HolderCheck,
    leave: f64,
) -> Result<Option<holder::PartCheck>, CamError> {
    if mode == chamfera_doc::cam::HolderCheck::Ignore {
        return Ok(None);
    }
    let Some(nc) = holder::NonCutting::of(tool)? else {
        return Err(CamError::Refused(format!(
            "the holder check needs a holder on the tool {:?}: give it one",
            tool.name
        )));
    };
    holder::PartCheck::new(&nc, leave).map(Some)
}

/// The stock's XY box, which the tool's centre covers in the 3D operations.
fn stock_area(g: &SetupGeometry) -> ops3d::Area {
    ops3d::Area {
        min: Point2::new(g.stock_min.x, g.stock_min.y),
        max: Point2::new(g.stock_max.x, g.stock_max.y),
    }
}

/// The holes a drill operation drills: those of the named faces, or every vertical hole.
pub fn drill_holes(bodies: &[Body], faces: &[String]) -> Result<Vec<Hole>, CamError> {
    let mut holes = Vec::new();
    if faces.is_empty() {
        for b in bodies {
            holes.extend(model::find_holes(b, None)?.into_iter().map(|h| h.0));
        }
    } else {
        // Every piece of each named face (a hole wall split by a later cut is still one
        // hole to drill).
        let mut found = Vec::new();
        for n in faces {
            found.extend(resolved(bodies, Kind::Face, n, Want::All)?);
        }
        for (i, b) in bodies.iter().enumerate() {
            let mut mine: Vec<FaceId> = found
                .iter()
                .filter(|f| f.body == i)
                .filter_map(Found::face)
                .collect();
            mine.dedup();
            if !mine.is_empty() {
                holes.extend(model::find_holes(b, Some(&mine))?.into_iter().map(|h| h.0));
            }
        }
    }
    Ok(holes)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests3d;
#[cfg(test)]
mod tests_v3;

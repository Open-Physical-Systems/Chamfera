//! The tool's non-cutting parts: the shank above the flutes and the holder, as a stack
//! of solids of revolution about the tool's axis (cylinders and cones), measured from the
//! tool's tip.
//!
//! Two questions are asked of them:
//! - **Against a heightfield** (the simulated stock, [`crate::sim`]): over a column at a
//!   distance `ρ` from the axis the lowest non-cutting surface is `low(ρ)` above the tip
//!   (the bottom of the lowest piece at least `ρ` wide there; inside a cone, the height
//!   where its radius reaches `ρ`). A column taller than the tip plus `low(ρ)` touches
//!   the holder or shank.
//! - **Against the exact part** (the 3D operations): each piece is replaced by flat
//!   cylinders that contain it ([`NonCutting::cylinders`]: a cylinder as it is, a cone by
//!   [`CONE_STEPS`] steps each as wide as the cone's widest point in it), and a flat
//!   cylinder of radius `r` whose bottom is `a` above the tip clears the part when the
//!   tip is at least `drop(r) - a` (the exact drop-cutter of a flat end mill of radius
//!   `r`, [`crate::drop`]). The steps contain the cone, so a collision is never missed;
//!   near a cone's side the check can be up to one step's widening cautious.
//!
//! The cutting part (the flutes, up to the flute length) is the cutter itself and is not
//! checked here: what it touches it cuts. The shank's diameter defaults to the tool's.

use crate::CamError;
use crate::drop::{DropModel, ToolShape};
use chamfera_doc::cam::Tool;

/// Steps a cone is split into for the checks against the part.
pub const CONE_STEPS: usize = 8;

/// One piece: from `z0` to `z1` above the tip, radius `r0` at the bottom to `r1` at the
/// top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    pub z0: f64,
    pub z1: f64,
    pub r0: f64,
    pub r1: f64,
}

impl Piece {
    /// The lowest height of this piece at least `rho` from the axis, if any.
    fn low(&self, rho: f64) -> Option<f64> {
        if rho <= self.r0 {
            return Some(self.z0);
        }
        if rho <= self.r1 && self.r1 > self.r0 {
            let t = (rho - self.r0) / (self.r1 - self.r0);
            return Some(self.z0 + t * (self.z1 - self.z0));
        }
        None
    }
}

/// The shank and holder of a tool, bottom first.
#[derive(Debug, Clone, PartialEq)]
pub struct NonCutting {
    pub pieces: Vec<Piece>,
}

impl NonCutting {
    /// The tool's shank and holder, or `None` when it has no holder.
    pub fn of(tool: &Tool) -> Result<Option<NonCutting>, CamError> {
        let Some(h) = &tool.holder else {
            return Ok(None);
        };
        let ok = |v: f64| v.is_finite() && v > 0.0;
        let mut pieces = Vec::new();
        let flutes = tool.flute_length.max(0.0);
        if !(ok(h.stickout) && h.stickout >= flutes) {
            return Err(CamError::Invalid(
                "the stickout must be at least the flute length".into(),
            ));
        }
        let shank = h.shank_diameter.unwrap_or(tool.diameter) / 2.0;
        if !ok(shank) {
            return Err(CamError::Invalid(
                "the shank diameter must be positive".into(),
            ));
        }
        if h.stickout > flutes {
            pieces.push(Piece {
                z0: flutes,
                z1: h.stickout,
                r0: shank,
                r1: shank,
            });
        }
        let mut z = h.stickout;
        for s in &h.segments {
            let r0 = s.diameter / 2.0;
            let r1 = s.top_diameter.unwrap_or(s.diameter) / 2.0;
            if !(ok(r0) && ok(r1) && ok(s.length)) {
                return Err(CamError::Invalid(
                    "a holder segment needs a positive diameter and length".into(),
                ));
            }
            pieces.push(Piece {
                z0: z,
                z1: z + s.length,
                r0,
                r1,
            });
            z += s.length;
        }
        Ok((!pieces.is_empty()).then_some(NonCutting { pieces }))
    }

    /// The widest radius of any piece.
    pub fn max_radius(&self) -> f64 {
        self.pieces
            .iter()
            .map(|p| p.r0.max(p.r1))
            .fold(0.0, f64::max)
    }

    /// The lowest height above the tip of any piece.
    pub fn lowest(&self) -> f64 {
        self.pieces
            .iter()
            .map(|p| p.z0)
            .fold(f64::INFINITY, f64::min)
    }

    /// The lowest non-cutting surface above the tip at `rho` from the axis, or `None`
    /// when no piece is that wide.
    pub fn low(&self, rho: f64) -> Option<f64> {
        self.pieces
            .iter()
            .filter_map(|p| p.low(rho))
            .min_by(f64::total_cmp)
    }

    /// Flat cylinders `(radius, bottom above the tip)` that together contain every piece.
    pub fn cylinders(&self) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for p in &self.pieces {
            if (p.r1 - p.r0).abs() <= 1e-12 {
                out.push((p.r0.max(p.r1), p.z0));
                continue;
            }
            for k in 0..CONE_STEPS {
                let a = k as f64 / CONE_STEPS as f64;
                let b = (k + 1) as f64 / CONE_STEPS as f64;
                let r = (p.r0 + (p.r1 - p.r0) * a).max(p.r0 + (p.r1 - p.r0) * b);
                out.push((r, p.z0 + (p.z1 - p.z0) * a));
            }
        }
        out
    }
}

/// Checks a tool position against the exact part: the lowest tip height at `(x, y)` at
/// which the holder and shank clear the part (`-inf` where nothing is under them).
#[derive(Debug, Clone)]
pub struct PartCheck {
    shapes: Vec<(ToolShape, f64)>,
}

impl PartCheck {
    /// `leave`: how far the holder is kept off the part (the operation's stock to leave).
    pub fn new(nc: &NonCutting, leave: f64) -> Result<PartCheck, CamError> {
        let mut shapes = Vec::new();
        for (r, z0) in nc.cylinders() {
            shapes.push((ToolShape::new(r + leave.max(0.0), 0.0)?, z0));
        }
        Ok(PartCheck { shapes })
    }

    /// The lowest tip height that keeps the holder clear of the part at `(x, y)`.
    pub fn required(&self, model: &DropModel, x: f64, y: f64) -> f64 {
        let mut z = f64::NEG_INFINITY;
        for (shape, z0) in &self.shapes {
            if let Some(d) = model.drop(shape, x, y) {
                z = z.max(d - z0);
            }
        }
        z
    }
}

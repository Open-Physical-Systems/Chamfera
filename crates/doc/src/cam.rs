//! The CAM section of a design (milestone M10): setups, tools and machining operations.
//!
//! Only what the user asked for is stored: the stock, the work coordinate system, the
//! tools and each operation with the model faces and edges it machines, by their
//! persistent names (`2:top`, `between 2:side 0 and 2:top`). Toolpaths are worked out
//! again from the model after every change (`chamfera-cam`), so they follow edits.
//! Lengths are millimetres, angles radians, feeds millimetres per minute.

use crate::DocError;
use serde::{Deserialize, Serialize};

/// Most setups, tools or operations one design may hold (each).
pub const MAX_CAM_ITEMS: usize = 1_000;
/// Most faces or edges one operation may name.
pub const MAX_OP_REFS: usize = 1_000;
/// Longest name (of an item, a face or an edge), in characters.
pub const MAX_CAM_NAME: usize = 256;
/// Largest tool diameter, flute length or tool size (mm).
pub const MAX_TOOL_SIZE: f64 = 1_000.0;
/// Largest spindle speed (rpm).
pub const MAX_SPINDLE_RPM: f64 = 200_000.0;
/// Largest feed rate (mm/min).
pub const MAX_FEED: f64 = 100_000.0;

/// How the stock (the raw block) is sized.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stock {
    /// The model's bounding box grown by `side` on each side, `top` above, `bottom` below.
    FromModel { side: f64, top: f64, bottom: f64 },
    /// A block of this size, centred on the model in X and Y, its bottom at the model's.
    Fixed { x: f64, y: f64, z: f64 },
}

/// Where the work coordinate system's origin (part zero) is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WcsOrigin {
    /// The stock's top face, at its corner of least X and Y.
    StockTopCorner,
    /// The centre of the stock's top face.
    StockTopCentre,
    /// The stock's bottom face, at its corner of least X and Y.
    StockBottomCorner,
    /// The model's own origin.
    ModelOrigin,
    /// A point in model coordinates.
    Point { x: f64, y: f64, z: f64 },
}

/// One setup: the stock, part zero and the machine's rapid rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub id: u32,
    pub name: String,
    pub stock: Stock,
    pub origin: WcsOrigin,
    /// A straight horizontal edge (by reference name) whose direction is the work X axis;
    /// `None` for the model's X axis. Z is always the model's Z (3-axis milling).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_axis: Option<String>,
    /// Height above the stock top for rapid moves between cuts (mm).
    pub clearance: f64,
    /// The machine's rapid traverse rate, for the cycle time (mm/min).
    pub rapid_rate: f64,
}

/// The kind of cutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    FlatEndMill,
    BallEndMill,
    BullNose,
    Drill,
    ChamferMill,
}

impl ToolKind {
    pub fn parse(s: &str) -> Option<ToolKind> {
        Some(match s {
            "flat_end_mill" => ToolKind::FlatEndMill,
            "ball_end_mill" => ToolKind::BallEndMill,
            "bull_nose" => ToolKind::BullNose,
            "drill" => ToolKind::Drill,
            "chamfer_mill" => ToolKind::ChamferMill,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            ToolKind::FlatEndMill => "flat_end_mill",
            ToolKind::BallEndMill => "ball_end_mill",
            ToolKind::BullNose => "bull_nose",
            ToolKind::Drill => "drill",
            ToolKind::ChamferMill => "chamfer_mill",
        }
    }

    /// Words for the user: "flat end mill".
    pub fn words(self) -> &'static str {
        match self {
            ToolKind::FlatEndMill => "flat end mill",
            ToolKind::BallEndMill => "ball end mill",
            ToolKind::BullNose => "bull nose mill",
            ToolKind::Drill => "drill",
            ToolKind::ChamferMill => "chamfer mill",
        }
    }
}

/// A cutter in the tool library, with its default feeds and speeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: u32,
    /// The pocket number in the machine's tool changer (`T` word).
    pub number: u32,
    pub name: String,
    pub kind: ToolKind,
    pub diameter: f64,
    pub flute_length: f64,
    /// Corner radius: half the diameter for a ball end mill, 0 for a flat one.
    pub corner_radius: f64,
    pub flutes: u32,
    /// Point angle of a drill or included angle of a chamfer mill (radians); 0 otherwise.
    pub tip_angle: f64,
    pub spindle_rpm: f64,
    /// Cutting feed (mm/min).
    pub feed: f64,
    /// Feed for plunges and ramps (mm/min).
    pub plunge_feed: f64,
}

/// Which side of the path the tool runs on, looking along a counter-clockwise boundary
/// (seen from above): right is outside the boundary, left is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    #[default]
    Right,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "left" | "inside" => Some(Side::Left),
            "right" | "outside" => Some(Side::Right),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// How a pocket's tool gets down into the material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    Plunge,
    /// Down along the first pass at the ramp angle.
    Ramp,
    /// Down a helix at the ramp angle.
    #[default]
    Helix,
}

impl Entry {
    pub fn parse(s: &str) -> Option<Entry> {
        match s {
            "plunge" => Some(Entry::Plunge),
            "ramp" => Some(Entry::Ramp),
            "helix" => Some(Entry::Helix),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Entry::Plunge => "plunge",
            Entry::Ramp => "ramp",
            Entry::Helix => "helix",
        }
    }
}

/// What an operation does, with its own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpKind {
    /// Flattens the stock top down to the model's top, in zigzag passes along X.
    Facing {
        /// Distance between passes as a fraction of the tool diameter (0 to 1].
        stepover: f64,
        /// Most material removed per depth (mm).
        stepdown: f64,
    },
    /// Follows a closed boundary at the tool radius from it, from the stock top down to
    /// the boundary's height.
    Contour {
        /// A flat horizontal face whose outer boundary is followed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        face: Option<String>,
        /// Or a closed chain of horizontal edges, by reference name.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        edges: Vec<String>,
        side: Side,
        stepdown: f64,
        /// Radius of the tangent lead-in and lead-out arcs; 0 for none.
        lead_radius: f64,
        /// How far below the boundary to cut (to break through the bottom).
        bottom_offset: f64,
        /// Climb milling (tool rotating clockwise, material on the right of travel).
        climb: bool,
    },
    /// Clears a flat-bottomed region with contour-parallel passes.
    Pocket {
        /// The pocket's floor: a flat face facing up, with vertical walls rising from it.
        face: String,
        stepover: f64,
        stepdown: f64,
        entry: Entry,
        /// Ramp and helix angle from the horizontal (radians).
        ramp_angle: f64,
        /// Helix radius; it is made smaller where the pocket is too narrow.
        helix_radius: f64,
    },
    /// Drills holes: vertical cylindrical holes found in the model.
    Drill {
        /// The holes' cylindrical faces; empty for every vertical hole of the model.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        faces: Vec<String>,
        /// Peck depth (G83); `None` drills in one stroke (G81).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        peck: Option<f64>,
        /// Height of the retract plane above the stock top (mm).
        retract: f64,
        /// How far below each hole's bottom to drill.
        breakthrough: f64,
    },
    /// 3D finishing in raster passes over the stock, the tool dropped onto the model.
    Parallel {
        /// Distance between passes as a fraction of the tool diameter (0 to 1].
        stepover: f64,
        /// A cusp height for a ball mill instead: the stepover is `2 sqrt(2 R h - h²)`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scallop: Option<f64>,
        /// Direction of the passes from +X (radians).
        angle: f64,
        /// Largest height error between the samples along a pass (mm).
        tolerance: f64,
        /// Material left on the part (mm).
        #[serde(default)]
        stock_to_leave: f64,
    },
    /// 3D finishing in contours at Z levels, where the tool touches the part.
    Waterline {
        stepdown: f64,
        tolerance: f64,
        #[serde(default)]
        stock_to_leave: f64,
    },
    /// Roughing of the stock minus the part in levels, the tool's engagement bounded.
    Adaptive {
        stepdown: f64,
        /// Largest engagement angle (radians).
        max_engagement: f64,
        #[serde(default)]
        stock_to_leave: f64,
    },
}

impl OpKind {
    pub fn key(&self) -> &'static str {
        match self {
            OpKind::Facing { .. } => "facing",
            OpKind::Contour { .. } => "contour",
            OpKind::Pocket { .. } => "pocket",
            OpKind::Drill { .. } => "drill",
            OpKind::Parallel { .. } => "parallel",
            OpKind::Waterline { .. } => "waterline",
            OpKind::Adaptive { .. } => "adaptive",
        }
    }
}

/// One machining operation in a setup, cut with one tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamOperation {
    pub id: u32,
    pub name: String,
    pub setup: u32,
    pub tool: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
    pub op: OpKind,
}

/// Setups, tools and operations, in order. Operations run in list order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cam {
    #[serde(default)]
    pub setups: Vec<Setup>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub operations: Vec<CamOperation>,
    /// The last CAM id handed out (setups, tools and operations share it); never reused.
    #[serde(default)]
    pub last_id: u32,
}

fn bad(reason: String) -> DocError {
    DocError::BadFeature {
        reason: format!("CAM: {reason}"),
    }
}

fn check_len(what: &str, v: f64, min_exclusive: f64, max: f64) -> Result<(), DocError> {
    if !(v.is_finite() && v > min_exclusive && v <= max) {
        return Err(bad(format!(
            "{what} must be a number above {min_exclusive} and at most {max}, not {v}"
        )));
    }
    Ok(())
}

fn check_range(what: &str, v: f64, min: f64, max: f64) -> Result<(), DocError> {
    if !(v.is_finite() && v >= min && v <= max) {
        return Err(bad(format!(
            "{what} must be a number from {min} to {max}, not {v}"
        )));
    }
    Ok(())
}

fn check_name(what: &str, s: &str) -> Result<(), DocError> {
    if s.chars().count() > MAX_CAM_NAME {
        return Err(bad(format!(
            "{what} is longer than {MAX_CAM_NAME} characters"
        )));
    }
    Ok(())
}

impl Cam {
    pub fn is_empty(&self) -> bool {
        self.setups.is_empty() && self.tools.is_empty() && self.operations.is_empty()
    }

    /// Hands out the next CAM id.
    pub fn next_id(&mut self) -> Result<u32, DocError> {
        let id = self
            .last_id
            .checked_add(1)
            .ok_or_else(|| bad("no more ids".into()))?;
        self.last_id = id;
        Ok(id)
    }

    pub fn setup(&self, id: u32) -> Option<&Setup> {
        self.setups.iter().find(|s| s.id == id)
    }

    pub fn tool(&self, id: u32) -> Option<&Tool> {
        self.tools.iter().find(|t| t.id == id)
    }

    pub fn operation(&self, id: u32) -> Option<&CamOperation> {
        self.operations.iter().find(|o| o.id == id)
    }

    /// Checks every value and reference: a file and a command are hostile input.
    pub fn validate(&self) -> Result<(), DocError> {
        let m = chamfera_geom::MAX_COORDINATE;
        for (what, n) in [
            ("setups", self.setups.len()),
            ("tools", self.tools.len()),
            ("operations", self.operations.len()),
        ] {
            if n > MAX_CAM_ITEMS {
                return Err(bad(format!(
                    "a design holds at most {MAX_CAM_ITEMS} {what}"
                )));
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        let ids_ok = |id: u32, ids: &mut std::collections::BTreeSet<u32>| {
            if id == 0 || id > self.last_id || !ids.insert(id) {
                return Err(bad(format!("id {id} is duplicate or out of range")));
            }
            Ok(())
        };
        for s in &self.setups {
            ids_ok(s.id, &mut ids)?;
            check_name("a setup name", &s.name)?;
            match s.stock {
                Stock::FromModel { side, top, bottom } => {
                    check_range("the stock side offset", side, 0.0, MAX_TOOL_SIZE)?;
                    check_range("the stock top offset", top, 0.0, MAX_TOOL_SIZE)?;
                    check_range("the stock bottom offset", bottom, 0.0, MAX_TOOL_SIZE)?;
                }
                Stock::Fixed { x, y, z } => {
                    check_len("the stock size x", x, 0.0, m)?;
                    check_len("the stock size y", y, 0.0, m)?;
                    check_len("the stock size z", z, 0.0, m)?;
                }
            }
            if let WcsOrigin::Point { x, y, z } = s.origin {
                for (w, v) in [("x", x), ("y", y), ("z", z)] {
                    check_range(&format!("the origin {w}"), v, -m, m)?;
                }
            }
            if let Some(e) = &s.x_axis {
                check_name("the x axis edge", e)?;
            }
            check_len("the clearance", s.clearance, 0.0, MAX_TOOL_SIZE)?;
            check_len("the rapid rate", s.rapid_rate, 0.0, MAX_FEED)?;
        }
        for t in &self.tools {
            ids_ok(t.id, &mut ids)?;
            check_name("a tool name", &t.name)?;
            if t.number > 9_999 {
                return Err(bad("a tool number must be at most 9999".into()));
            }
            check_len("the tool diameter", t.diameter, 0.0, MAX_TOOL_SIZE)?;
            check_len("the flute length", t.flute_length, 0.0, MAX_TOOL_SIZE)?;
            check_range("the corner radius", t.corner_radius, 0.0, t.diameter / 2.0)?;
            if t.flutes == 0 || t.flutes > 100 {
                return Err(bad("a tool has 1 to 100 flutes".into()));
            }
            check_range("the tip angle", t.tip_angle, 0.0, std::f64::consts::PI)?;
            check_len("the spindle speed", t.spindle_rpm, 0.0, MAX_SPINDLE_RPM)?;
            check_len("the feed", t.feed, 0.0, MAX_FEED)?;
            check_len("the plunge feed", t.plunge_feed, 0.0, MAX_FEED)?;
        }
        for o in &self.operations {
            ids_ok(o.id, &mut ids)?;
            check_name("an operation name", &o.name)?;
            if self.setup(o.setup).is_none() {
                return Err(bad(format!("operation {} uses a missing setup", o.id)));
            }
            if self.tool(o.tool).is_none() {
                return Err(bad(format!("operation {} uses a missing tool", o.id)));
            }
            check_op(&o.op)?;
        }
        Ok(())
    }
}

fn check_refs(what: &str, refs: &[String]) -> Result<(), DocError> {
    if refs.len() > MAX_OP_REFS {
        return Err(bad(format!(
            "an operation names at most {MAX_OP_REFS} {what}"
        )));
    }
    refs.iter().try_for_each(|r| check_name(what, r))
}

fn check_op(op: &OpKind) -> Result<(), DocError> {
    let half_turn = std::f64::consts::FRAC_PI_2;
    match op {
        OpKind::Facing { stepover, stepdown } => {
            check_len("the stepover", *stepover, 0.0, 1.0)?;
            check_len("the stepdown", *stepdown, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Contour {
            face,
            edges,
            stepdown,
            lead_radius,
            bottom_offset,
            ..
        } => {
            if face.is_none() == edges.is_empty() {
                return Err(bad(
                    "a contour follows either a face or a chain of edges".into()
                ));
            }
            if let Some(f) = face {
                check_name("the face", f)?;
            }
            check_refs("edges", edges)?;
            check_len("the stepdown", *stepdown, 0.0, MAX_TOOL_SIZE)?;
            check_range("the lead radius", *lead_radius, 0.0, MAX_TOOL_SIZE)?;
            check_range("the bottom offset", *bottom_offset, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Pocket {
            face,
            stepover,
            stepdown,
            ramp_angle,
            helix_radius,
            ..
        } => {
            check_name("the face", face)?;
            check_len("the stepover", *stepover, 0.0, 1.0)?;
            check_len("the stepdown", *stepdown, 0.0, MAX_TOOL_SIZE)?;
            check_len("the ramp angle", *ramp_angle, 0.0, half_turn)?;
            check_range("the helix radius", *helix_radius, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Drill {
            faces,
            peck,
            retract,
            breakthrough,
        } => {
            check_refs("faces", faces)?;
            if let Some(p) = peck {
                check_len("the peck depth", *p, 0.0, MAX_TOOL_SIZE)?;
            }
            check_len("the retract height", *retract, 0.0, MAX_TOOL_SIZE)?;
            check_range("the breakthrough", *breakthrough, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Parallel {
            stepover,
            scallop,
            angle,
            tolerance,
            stock_to_leave,
        } => {
            check_len("the stepover", *stepover, 0.0, 1.0)?;
            if let Some(h) = scallop {
                check_len("the scallop height", *h, 0.0, MAX_TOOL_SIZE)?;
            }
            check_range(
                "the angle",
                *angle,
                -std::f64::consts::TAU,
                std::f64::consts::TAU,
            )?;
            check_range("the tolerance", *tolerance, 1e-4, 10.0)?;
            check_range("the stock to leave", *stock_to_leave, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Waterline {
            stepdown,
            tolerance,
            stock_to_leave,
        } => {
            check_len("the stepdown", *stepdown, 0.0, MAX_TOOL_SIZE)?;
            check_range("the tolerance", *tolerance, 1e-4, 10.0)?;
            check_range("the stock to leave", *stock_to_leave, 0.0, MAX_TOOL_SIZE)?;
        }
        OpKind::Adaptive {
            stepdown,
            max_engagement,
            stock_to_leave,
        } => {
            check_len("the stepdown", *stepdown, 0.0, MAX_TOOL_SIZE)?;
            check_range(
                "the engagement limit",
                *max_engagement,
                0.05,
                std::f64::consts::PI,
            )?;
            check_range("the stock to leave", *stock_to_leave, 0.0, MAX_TOOL_SIZE)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: u32) -> Tool {
        Tool {
            id,
            number: 1,
            name: "6 mm flat".into(),
            kind: ToolKind::FlatEndMill,
            diameter: 6.0,
            flute_length: 20.0,
            corner_radius: 0.0,
            flutes: 3,
            tip_angle: 0.0,
            spindle_rpm: 10_000.0,
            feed: 800.0,
            plunge_feed: 300.0,
        }
    }

    #[test]
    fn a_cam_section_round_trips_and_checks_its_references() {
        let mut cam = Cam::default();
        let s = cam.next_id().unwrap();
        cam.setups.push(Setup {
            id: s,
            name: "Setup 1".into(),
            stock: Stock::FromModel {
                side: 2.0,
                top: 1.0,
                bottom: 0.0,
            },
            origin: WcsOrigin::StockTopCorner,
            x_axis: None,
            clearance: 10.0,
            rapid_rate: 5000.0,
        });
        let t = cam.next_id().unwrap();
        cam.tools.push(tool(t));
        let o = cam.next_id().unwrap();
        cam.operations.push(CamOperation {
            id: o,
            name: "Pocket 1".into(),
            setup: s,
            tool: t,
            suppressed: false,
            op: OpKind::Pocket {
                face: "3:bottom".into(),
                stepover: 0.5,
                stepdown: 2.0,
                entry: Entry::Helix,
                ramp_angle: 0.05,
                helix_radius: 1.5,
            },
        });
        cam.validate().unwrap();
        let text = serde_json::to_string(&cam).unwrap();
        let back: Cam = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cam);
        let mut broken = cam.clone();
        broken.operations[0].tool = 99;
        assert!(broken.validate().is_err());
        let mut broken = cam.clone();
        broken.tools[0].diameter = f64::NAN;
        assert!(broken.validate().is_err());
        let mut broken = cam;
        broken.tools[0].corner_radius = 4.0; // more than half the 6 mm diameter
        assert!(broken.validate().is_err());
    }
}

//! `cam.*`: setups, the tool library, machining operations, toolpaths and G-code.
//!
//! Operations name model faces and edges by their persistent names (`2:top`, see
//! `solids::face_key` and `solids::edge_ref`), so their toolpaths are worked out again
//! when the model changes. Lengths are mm (a number, or an expression such as "2 mm");
//! angles are degrees when given as a number; stepovers are percent of the tool diameter.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::cam::{
    CamOperation, Entry, Holder, HolderCheck, HolderSegment, LibraryTool, OpKind, Setup, Side,
    Stock, Tool, ToolKind, ToolLibrary, WcsOrigin,
};
use chamfera_doc::expr::Dim;
use serde_json::{Map, Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "cam.generate",
        label: "Generate Toolpaths",
        params: "operation: integer (optional) - just this one; default every operation\n\
                 Works the toolpaths out again, shows them, and replies with each one's \
                 moves, cut and rapid lengths and cycle time (minutes), or its error",
        mutates: false,
        run: generate,
    },
    CommandSpec {
        id: "cam.list",
        label: "List Setups and Operations",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "cam.op_add",
        label: "Add Machining Operation",
        params: "kind: string (required) - facing, contour, pocket, drill, parallel, waterline, adaptive, pencil or scallop (experimental)\n\
                 tool: integer (required) - a tool from cam.tool_add\n\
                 setup: integer (optional) - default the last setup (one is made if none)\n\
                 name: string (optional)\n\
                 stepover: number (optional) - percent of the tool diameter (facing 50, pocket 40, parallel 10)\n\
                 stepdown: length (optional) - most depth per pass (facing 1, else 2)\n\
                 face: string (contour, pocket) - a face name such as \"2:bottom\"\n\
                 edges: list of strings (contour) - a closed chain of edge names, instead of face\n\
                 side: string (contour) - outside (right, default) or inside (left)\n\
                 lead_radius: length (contour) - tangent lead arcs; default the tool radius outside, 0 inside\n\
                 bottom_offset: length (contour) - cut this far below the boundary (default 0)\n\
                 climb: boolean (contour) - climb milling (default true)\n\
                 entry: string (pocket) - helix (default), ramp or plunge\n\
                 ramp_angle: angle (pocket) - degrees, default 3\n\
                 helix_radius: length (pocket) - default a quarter of the tool diameter\n\
                 faces: list of strings (drill) - hole faces; default every vertical hole\n\
                 peck: length (drill) - peck depth (G83); default none (G81)\n\
                 retract: length (drill) - retract plane above the stock (default 2)\n\
                 breakthrough: length (drill) - past the hole bottom (default 0)\n\
                 scallop: length (parallel) - cusp height for a ball end mill, instead of stepover\n\
                 angle: angle (parallel) - pass direction from +X, degrees (default 0)\n\
                 tolerance: length (parallel, waterline) - largest height error between samples (default 0.01)\n\
                 stock_to_leave: length (parallel, waterline, adaptive, pencil, scallop) - material left on the part (default 0; adaptive 0.3)\n\
                 max_engagement: angle (adaptive) - largest tool engagement, degrees (default 60)\n\
                 holder_check: string (parallel, adaptive) - ignore (default), lift or refuse where the tool's holder would hit the part\n\
                 min_angle: angle (pencil) - least change of slope across a corner that is traced, degrees (default 10)\n\
                 max_slope: angle (scallop) - steeper than this is a border, degrees (default 60)\n\
                 The scallop stepover is measured along the surface (percent of the tool diameter, default 10)",
        mutates: true,
        run: op_add,
    },
    CommandSpec {
        id: "cam.op_delete",
        label: "Delete Machining Operation",
        params: "operation: integer (required)",
        mutates: true,
        run: op_delete,
    },
    CommandSpec {
        id: "cam.op_edit",
        label: "Edit Machining Operation",
        params: "operation: integer (required)\n\
                 suppressed: boolean (optional)\n\
                 tool, setup, name: as in cam.op_add (optional)\n\
                 and any setting of its kind, as in cam.op_add (optional)",
        mutates: true,
        run: op_edit,
    },
    CommandSpec {
        id: "cam.post",
        label: "Post G-code",
        params: "path: string (optional) - where to write the .nc file; without it the \
                 G-code comes back as `gcode`\n\
                 dialect: string (optional) - linuxcnc (default) or grbl (no M6, no canned cycles)\n\
                 setup: integer (optional) - default the first setup",
        mutates: false,
        run: post,
    },
    CommandSpec {
        id: "cam.simulate",
        label: "Simulate",
        params: "setup: integer (optional) - default the first setup\n\
                 ops: list of integers (optional) - these operations, in order; default the setup's\n\
                 resolution: integer (optional) - stock columns along its longer side, 1 to 1024 (default 128)\n\
                 time: number (optional) - how far through the cycle time, 0 to 1 (default 1)\n\
                 tolerance: length (optional) - a cut this far below the part is a gouge (default 0.02)\n\
                 clear: boolean (optional) - true ends the simulation\n\
                 Cuts a dexel stock along the toolpaths, shows it (gouges and holder collisions in \
                 their own colours) and replies with removed_volume, rest_volume (left above the \
                 part), gouges {cells, volume, max_depth}, holder {checked, moves, cells, \
                 max_depth, incomplete} (where a tool's holder or shank hit the stock or part) \
                 and the cell size",
        mutates: false,
        run: simulate,
    },
    CommandSpec {
        id: "cam.setup",
        label: "Setup",
        params: "setup: integer (optional) - edit this setup; default make a new one\n\
                 name: string (optional)\n\
                 stock: object (optional) - {\"kind\": \"from_model\", \"side\", \"top\", \"bottom\"} \
                 (offsets, default 2, 1, 0) or {\"kind\": \"fixed\", \"x\", \"y\", \"z\"} (size)\n\
                 origin: string or {x, y, z} (optional) - stock_top_corner (default), \
                 stock_top_centre, stock_bottom_corner, model_origin, or a point\n\
                 x_axis: string or null (optional) - a straight edge along the work X axis\n\
                 clearance: length (optional) - rapid height above the stock (default 10)\n\
                 rapid_rate: number (optional) - mm/min for the cycle time (default 5000)",
        mutates: true,
        run: setup,
    },
    CommandSpec {
        id: "cam.show",
        label: "Show Toolpaths",
        params: "visible: boolean (required)",
        mutates: false,
        run: show,
    },
    CommandSpec {
        id: "cam.tool_add",
        label: "Add Tool",
        params: "kind: string (required) - flat_end_mill, ball_end_mill, bull_nose, drill, chamfer_mill\n\
                 diameter: length (required)\n\
                 name: string (optional)\n\
                 number: integer (optional) - T number; default the next free one\n\
                 flute_length: length (optional) - default 3 diameters\n\
                 corner_radius: length (optional) - required for bull_nose; ball is half the diameter\n\
                 flutes: integer (optional) - default 2 for drills, 3 for mills\n\
                 tip_angle: angle (optional) - degrees; default 118 for drills, 90 for chamfer mills\n\
                 spindle_rpm: number (optional) - default 10000\n\
                 feed: number (optional) - mm/min, default 600\n\
                 plunge_feed: number (optional) - mm/min, default a third of the feed\n\
                 holder: object or null (optional) - {\"stickout\": length from the tip to the holder, \
                 \"shank_diameter\": length (default the tool's), \"segments\": [{\"diameter\", \
                 \"top_diameter\" (a cone), \"length\"}, ...] bottom first}",
        mutates: true,
        run: tool_add,
    },
    CommandSpec {
        id: "cam.tool_delete",
        label: "Delete Tool",
        params: "tool: integer (required) - refused while an operation uses it",
        mutates: true,
        run: tool_delete,
    },
    CommandSpec {
        id: "cam.tool_edit",
        label: "Edit Tool",
        params: "tool: integer (required)\n\
                 and any setting of cam.tool_add (optional); holder null removes the holder",
        mutates: true,
        run: tool_edit,
    },
    CommandSpec {
        id: "cam.tool_library_export",
        label: "Export Tool Library",
        params: "path: string (required) - the .json file to write\n\
                 tools: list of integers (optional) - these tools; default all\n\
                 Writes the tools as a Chamfera tool library ({\"format\": \"chamfera-tools\", \
                 \"version\": 1, \"tools\": [...]})",
        mutates: false,
        run: tool_library_export,
    },
    CommandSpec {
        id: "cam.tool_library_import",
        label: "Import Tool Library",
        params: "path: string (required) - a tool library .json file\n\
                 replace: boolean (optional) - first remove the tools no operation uses (default false)\n\
                 Adds the library's tools (a T number already in use gets the next free one) and \
                 replies with their ids",
        mutates: true,
        run: tool_library_import,
    },
];

fn bad(s: impl Into<String>) -> CommandError {
    CommandError::BadParams(s.into())
}

fn obj(p: &Value) -> Result<&Map<String, Value>, CommandError> {
    p.as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))
}

/// A length: a number (mm) or an expression with units.
fn length(s: &Session, p: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match obj(p)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(_)) => params::req_f64(p, key).map(Some),
        Some(Value::String(e)) => {
            if e.chars().count() > chamfera_doc::expr::MAX_EXPRESSION_LEN {
                return Err(bad(format!("`{key}` is too long")));
            }
            Ok(Some(s.doc.evaluate_feature_value(e, Dim::LENGTH)?))
        }
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

/// An angle: a number in degrees or an expression with units; returned in radians.
fn angle(s: &Session, p: &Value, key: &str) -> Result<Option<f64>, CommandError> {
    match obj(p)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(_)) => params::req_f64(p, key).map(|d| Some(d.to_radians())),
        Some(Value::String(e)) => {
            if e.chars().count() > chamfera_doc::expr::MAX_EXPRESSION_LEN {
                return Err(bad(format!("`{key}` is too long")));
            }
            Ok(Some(s.doc.evaluate_feature_value(e, Dim::ANGLE)?))
        }
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match obj(p)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        _ => params::req_bool(p, key).map(Some),
    }
}

fn opt_id(p: &Value, key: &str) -> Result<Option<u32>, CommandError> {
    match obj(p)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        _ => params::req_id(p, key).map(Some),
    }
}

fn opt_name(p: &Value) -> Result<Option<String>, CommandError> {
    match params::opt_str(p, "name")? {
        Some(n) if n.chars().count() > chamfera_doc::cam::MAX_CAM_NAME => {
            Err(bad("`name` is too long"))
        }
        Some(n) => Ok(Some(n.to_owned())),
        None => Ok(None),
    }
}

fn str_list(p: &Value, key: &str) -> Result<Option<Vec<String>>, CommandError> {
    match obj(p)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => {
            if a.len() > chamfera_doc::cam::MAX_OP_REFS {
                return Err(bad(format!("`{key}` holds too many names")));
            }
            a.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| bad(format!("`{key}` must be a list of names")))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        Some(_) => Err(bad(format!("`{key}` must be a list of names"))),
    }
}

/// A stepover given in percent, as a fraction.
fn stepover(p: &Value) -> Result<Option<f64>, CommandError> {
    match params::opt_f64(p, "stepover")? {
        Some(v) if v > 0.0 && v <= 100.0 => Ok(Some(v / 100.0)),
        Some(_) => Err(bad(
            "`stepover` is a percent of the tool diameter, above 0 up to 100",
        )),
        None => Ok(None),
    }
}

/// Checks the CAM section after a change and reports the first problem as bad input.
fn checked(s: &Session) -> Result<(), CommandError> {
    s.doc
        .cam
        .validate()
        .map_err(|e| bad(e.to_string().replace("bad feature: ", "")))
}

fn stock(p: &Value, old: Stock) -> Result<Stock, CommandError> {
    let Some(v) = obj(p)?.get("stock").filter(|v| !v.is_null()) else {
        return Ok(old);
    };
    let kind = params::req_str(v, "kind")?;
    let num = |k: &str, d: f64| params::opt_f64(v, k).map(|x| x.unwrap_or(d));
    Ok(match kind {
        "from_model" => Stock::FromModel {
            side: num("side", 2.0)?,
            top: num("top", 1.0)?,
            bottom: num("bottom", 0.0)?,
        },
        "fixed" => Stock::Fixed {
            x: params::req_f64(v, "x")?,
            y: params::req_f64(v, "y")?,
            z: params::req_f64(v, "z")?,
        },
        _ => return Err(bad("`stock.kind` is from_model or fixed")),
    })
}

fn origin(p: &Value, old: WcsOrigin) -> Result<WcsOrigin, CommandError> {
    Ok(match obj(p)?.get("origin") {
        None | Some(Value::Null) => old,
        Some(Value::String(s)) => match s.as_str() {
            "stock_top_corner" => WcsOrigin::StockTopCorner,
            "stock_top_centre" | "stock_top_center" => WcsOrigin::StockTopCentre,
            "stock_bottom_corner" => WcsOrigin::StockBottomCorner,
            "model_origin" => WcsOrigin::ModelOrigin,
            _ => return Err(bad("unknown `origin`: see cam.setup's parameters")),
        },
        Some(v @ Value::Object(_)) => WcsOrigin::Point {
            x: params::req_f64(v, "x")?,
            y: params::req_f64(v, "y")?,
            z: params::req_f64(v, "z")?,
        },
        Some(_) => return Err(bad("`origin` is a name or a point {x, y, z}")),
    })
}

fn setup_json(s: &Session, st: &Setup) -> Value {
    let bodies: Vec<chamfera_kernel::Body> = s.solids().iter().map(|x| x.body.clone()).collect();
    let geo = chamfera_cam::setup_geometry(st, &bodies, crate::solids::edge_ref);
    let pt = |p: chamfera_geom::Point3| json!({"x": p.x, "y": p.y, "z": p.z});
    match geo {
        Ok(g) => json!({
            "setup": st.id,
            "name": st.name,
            "stock": {"min": pt(g.stock_min), "max": pt(g.stock_max)},
            "origin": pt(g.wcs.origin),
            "x_axis_angle": g.wcs.angle,
            "clearance_z": g.clearance_z,
        }),
        Err(e) => json!({"setup": st.id, "name": st.name, "error": e.to_string()}),
    }
}

fn setup(s: &mut Session, p: &Value) -> CmdResult {
    let edit = opt_id(p, "setup")?;
    let mut st = match edit {
        Some(id) => s
            .doc
            .cam
            .setup(id)
            .cloned()
            .ok_or_else(|| bad(format!("no setup {id}")))?,
        None => Setup {
            id: 0,
            name: format!("Setup {}", s.doc.cam.setups.len() + 1),
            stock: Stock::FromModel {
                side: 2.0,
                top: 1.0,
                bottom: 0.0,
            },
            origin: WcsOrigin::StockTopCorner,
            x_axis: None,
            clearance: 10.0,
            rapid_rate: 5000.0,
        },
    };
    if let Some(n) = opt_name(p)? {
        st.name = n;
    }
    st.stock = stock(p, st.stock)?;
    st.origin = origin(p, st.origin)?;
    match obj(p)?.get("x_axis") {
        None => {}
        Some(Value::Null) => st.x_axis = None,
        Some(Value::String(e)) => st.x_axis = Some(e.clone()),
        Some(_) => return Err(bad("`x_axis` is an edge name or null")),
    }
    if let Some(c) = length(s, p, "clearance")? {
        st.clearance = c;
    }
    if let Some(r) = params::opt_f64(p, "rapid_rate")? {
        st.rapid_rate = r;
    }
    match edit {
        Some(id) => {
            if let Some(slot) = s.doc.cam.setups.iter_mut().find(|x| x.id == id) {
                *slot = st.clone();
            }
        }
        None => {
            st.id = s.doc.cam.next_id()?;
            s.doc.cam.setups.push(st.clone());
        }
    }
    checked(s)?;
    if st.x_axis.is_some() {
        let bodies: Vec<chamfera_kernel::Body> =
            s.solids().iter().map(|x| x.body.clone()).collect();
        chamfera_cam::setup_geometry(&st, &bodies, crate::solids::edge_ref)
            .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    }
    Ok(setup_json(s, &st))
}

/// The `holder` parameter: `None` when not given, `Some(None)` for null.
fn holder_param(s: &Session, p: &Value) -> Result<Option<Option<Holder>>, CommandError> {
    let v = match obj(p)?.get("holder") {
        None => return Ok(None),
        Some(Value::Null) => return Ok(Some(None)),
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad("`holder` is an object or null")),
    };
    let stickout = length(s, v, "stickout")?.ok_or_else(|| bad("`holder.stickout` is required"))?;
    let shank_diameter = length(s, v, "shank_diameter")?;
    let mut segments = Vec::new();
    match obj(v)?.get("segments") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) => {
            if a.len() > chamfera_doc::cam::MAX_HOLDER_SEGMENTS {
                return Err(bad(format!(
                    "a holder has at most {} segments",
                    chamfera_doc::cam::MAX_HOLDER_SEGMENTS
                )));
            }
            for seg in a {
                if !seg.is_object() {
                    return Err(bad(
                        "each holder segment is {diameter, top_diameter, length}",
                    ));
                }
                segments.push(HolderSegment {
                    diameter: length(s, seg, "diameter")?
                        .ok_or_else(|| bad("a holder segment needs `diameter`"))?,
                    top_diameter: length(s, seg, "top_diameter")?,
                    length: length(s, seg, "length")?
                        .ok_or_else(|| bad("a holder segment needs `length`"))?,
                });
            }
        }
        Some(_) => return Err(bad("`holder.segments` is a list")),
    }
    Ok(Some(Some(Holder {
        stickout,
        shank_diameter,
        segments,
    })))
}

/// Applies `cam.tool_add`'s settings in `p` onto `t` (a new tool's defaults, or a tool
/// being edited: then only what `p` gives changes).
fn apply_tool(s: &Session, p: &Value, t: &mut Tool, new: bool) -> Result<(), CommandError> {
    if let Some(kind_s) = params::opt_str(p, "kind")? {
        t.kind = ToolKind::parse(kind_s).ok_or_else(|| {
            bad("`kind` is flat_end_mill, ball_end_mill, bull_nose, drill or chamfer_mill")
        })?;
    }
    if let Some(d) = length(s, p, "diameter")? {
        t.diameter = d;
    }
    let corner = length(s, p, "corner_radius")?;
    t.corner_radius = match (t.kind, corner) {
        (ToolKind::BallEndMill, _) => t.diameter / 2.0,
        (ToolKind::BullNose, None) if new => {
            return Err(bad("a bull nose mill needs `corner_radius`"));
        }
        (_, Some(r)) => r,
        (ToolKind::BullNose, None) => t.corner_radius,
        (_, None) => 0.0,
    };
    match obj(p)?.get("flutes") {
        None | Some(Value::Null) => {
            if new {
                t.flutes = if t.kind == ToolKind::Drill { 2 } else { 3 };
            }
        }
        _ => t.flutes = params::req_id(p, "flutes")?,
    }
    match angle(s, p, "tip_angle")? {
        Some(a) => t.tip_angle = a,
        None if new => {
            t.tip_angle = match t.kind {
                ToolKind::Drill => 118f64.to_radians(),
                ToolKind::ChamferMill => 90f64.to_radians(),
                _ => 0.0,
            }
        }
        None => {}
    }
    if let Some(f) = params::opt_f64(p, "feed")? {
        t.feed = f;
        if new {
            t.plunge_feed = f / 3.0;
        }
    }
    if let Some(f) = params::opt_f64(p, "plunge_feed")? {
        t.plunge_feed = f;
    }
    if let Some(r) = params::opt_f64(p, "spindle_rpm")? {
        t.spindle_rpm = r;
    }
    match length(s, p, "flute_length")? {
        Some(l) => t.flute_length = l,
        None if new => t.flute_length = 3.0 * t.diameter,
        None => {}
    }
    match obj(p)?.get("number") {
        None | Some(Value::Null) => {}
        _ => t.number = params::req_id(p, "number")?,
    }
    if let Some(n) = opt_name(p)? {
        t.name = n;
    }
    if let Some(h) = holder_param(s, p)? {
        t.holder = h;
    }
    Ok(())
}

fn tool_add(s: &mut Session, p: &Value) -> CmdResult {
    params::req_str(p, "kind")?;
    if length(s, p, "diameter")?.is_none() {
        return Err(bad("`diameter` is required"));
    }
    let mut t = Tool {
        id: 0,
        number: s.doc.cam.tools.iter().map(|t| t.number).max().unwrap_or(0) + 1,
        name: String::new(),
        kind: ToolKind::FlatEndMill,
        diameter: 1.0,
        flute_length: 3.0,
        corner_radius: 0.0,
        flutes: 3,
        tip_angle: 0.0,
        spindle_rpm: 10_000.0,
        feed: 600.0,
        plunge_feed: 200.0,
        holder: None,
    };
    apply_tool(s, p, &mut t, true)?;
    if t.name.is_empty() {
        t.name = format!(
            "{} mm {}",
            chamfera_cam::post::number(t.diameter),
            t.kind.words()
        );
    }
    t.id = s.doc.cam.next_id()?;
    let (id, number, name) = (t.id, t.number, t.name.clone());
    s.doc.cam.tools.push(t);
    checked(s)?;
    Ok(json!({"tool": id, "number": number, "name": name}))
}

fn tool_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "tool")?;
    let mut t = s
        .doc
        .cam
        .tool(id)
        .cloned()
        .ok_or_else(|| bad(format!("no tool {id}")))?;
    apply_tool(s, p, &mut t, false)?;
    let (number, name) = (t.number, t.name.clone());
    if let Some(slot) = s.doc.cam.tools.iter_mut().find(|x| x.id == id) {
        *slot = t;
    }
    checked(s)?;
    s.regenerate_cam_now();
    Ok(json!({"tool": id, "number": number, "name": name}))
}

fn tool_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "tool")?;
    if s.doc.cam.tool(id).is_none() {
        return Err(bad(format!("no tool {id}")));
    }
    if let Some(o) = s.doc.cam.operations.iter().find(|o| o.tool == id) {
        return Err(CommandError::Unavailable(format!(
            "{} uses this tool: give it another tool or delete it first",
            o.name
        )));
    }
    s.doc.cam.tools.retain(|t| t.id != id);
    Ok(json!({"deleted": id}))
}

fn tool_library_export(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?.to_owned();
    let only = match obj(p)?.get("tools") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            if a.len() > chamfera_doc::cam::MAX_CAM_ITEMS {
                return Err(bad("`tools` holds too many tools"));
            }
            let ids = a
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|x| u32::try_from(x).ok())
                        .ok_or_else(|| bad("`tools` must be a list of tool ids"))
                })
                .collect::<Result<Vec<u32>, _>>()?;
            for id in &ids {
                if s.doc.cam.tool(*id).is_none() {
                    return Err(bad(format!("no tool {id}")));
                }
            }
            Some(ids)
        }
        Some(_) => return Err(bad("`tools` must be a list of tool ids")),
    };
    let tools: Vec<LibraryTool> = s
        .doc
        .cam
        .tools
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.id)))
        .map(LibraryTool::from_tool)
        .collect();
    let n = tools.len();
    let text = ToolLibrary::new(tools).to_json()?;
    std::fs::write(&path, text.as_bytes()).map_err(chamfera_io::IoError::from)?;
    Ok(json!({"path": path, "tools": n}))
}

fn tool_library_import(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?.to_owned();
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let bytes = chamfera_io::read_limited(
        std::path::Path::new(&path),
        chamfera_doc::cam::MAX_TOOL_LIBRARY_BYTES,
    )?;
    let text = String::from_utf8(bytes).map_err(|_| bad("the tool library is not UTF-8 text"))?;
    let lib = ToolLibrary::from_json(&text)?;
    if replace {
        let used: Vec<u32> = s.doc.cam.operations.iter().map(|o| o.tool).collect();
        s.doc.cam.tools.retain(|t| used.contains(&t.id));
    }
    if s.doc.cam.tools.len() + lib.tools.len() > chamfera_doc::cam::MAX_CAM_ITEMS {
        return Err(bad(format!(
            "a design holds at most {} tools",
            chamfera_doc::cam::MAX_CAM_ITEMS
        )));
    }
    let mut ids = Vec::new();
    for lt in &lib.tools {
        let id = s.doc.cam.next_id()?;
        let mut t = lt.to_tool(id);
        if s.doc.cam.tools.iter().any(|x| x.number == t.number) {
            t.number = s.doc.cam.tools.iter().map(|x| x.number).max().unwrap_or(0) + 1;
        }
        s.doc.cam.tools.push(t);
        ids.push(id);
    }
    checked(s)?;
    Ok(json!({"path": path, "tools": ids}))
}

/// Applies the kind's settings in `p` onto `op` (new or edited).
fn apply(s: &Session, p: &Value, op: &mut OpKind, tool_d: f64) -> Result<(), CommandError> {
    let so = stepover(p)?;
    let sd = length(s, p, "stepdown")?;
    match op {
        OpKind::Facing { stepover, stepdown } => {
            *stepover = so.unwrap_or(*stepover);
            *stepdown = sd.unwrap_or(*stepdown);
        }
        OpKind::Contour {
            face,
            edges,
            side,
            stepdown,
            lead_radius,
            bottom_offset,
            climb,
        } => {
            if let Some(f) = params::opt_str(p, "face")? {
                *face = Some(f.to_owned());
                edges.clear();
            }
            if let Some(e) = str_list(p, "edges")? {
                *edges = e;
                *face = None;
            }
            if let Some(sd_) = params::opt_str(p, "side")? {
                *side = Side::parse(sd_)
                    .ok_or_else(|| bad("`side` is outside, inside, left or right"))?;
                if length(s, p, "lead_radius")?.is_none() {
                    *lead_radius = if *side == Side::Right {
                        tool_d / 2.0
                    } else {
                        0.0
                    };
                }
            }
            *stepdown = sd.unwrap_or(*stepdown);
            *lead_radius = length(s, p, "lead_radius")?.unwrap_or(*lead_radius);
            *bottom_offset = length(s, p, "bottom_offset")?.unwrap_or(*bottom_offset);
            *climb = opt_bool(p, "climb")?.unwrap_or(*climb);
        }
        OpKind::Pocket {
            face,
            stepover,
            stepdown,
            entry,
            ramp_angle,
            helix_radius,
        } => {
            if let Some(f) = params::opt_str(p, "face")? {
                *face = f.to_owned();
            }
            *stepover = so.unwrap_or(*stepover);
            *stepdown = sd.unwrap_or(*stepdown);
            if let Some(e) = params::opt_str(p, "entry")? {
                *entry = Entry::parse(e).ok_or_else(|| bad("`entry` is helix, ramp or plunge"))?;
            }
            *ramp_angle = angle(s, p, "ramp_angle")?.unwrap_or(*ramp_angle);
            *helix_radius = length(s, p, "helix_radius")?.unwrap_or(*helix_radius);
        }
        OpKind::Drill {
            faces,
            peck,
            retract,
            breakthrough,
        } => {
            if let Some(f) = str_list(p, "faces")? {
                *faces = f;
            }
            match obj(p)?.get("peck") {
                None => {}
                Some(Value::Null) => *peck = None,
                Some(_) => *peck = length(s, p, "peck")?,
            }
            *retract = length(s, p, "retract")?.unwrap_or(*retract);
            *breakthrough = length(s, p, "breakthrough")?.unwrap_or(*breakthrough);
        }
        OpKind::Parallel {
            stepover,
            scallop,
            angle: a,
            tolerance,
            stock_to_leave,
            holder_check,
        } => {
            *stepover = so.unwrap_or(*stepover);
            match obj(p)?.get("scallop") {
                None => {}
                Some(Value::Null) => *scallop = None,
                Some(_) => *scallop = length(s, p, "scallop")?,
            }
            if so.is_some() && obj(p)?.get("scallop").is_none() {
                *scallop = None;
            }
            *a = angle(s, p, "angle")?.unwrap_or(*a);
            *tolerance = length(s, p, "tolerance")?.unwrap_or(*tolerance);
            *stock_to_leave = length(s, p, "stock_to_leave")?.unwrap_or(*stock_to_leave);
            *holder_check = holder_check_param(p)?.unwrap_or(*holder_check);
        }
        OpKind::Waterline {
            stepdown,
            tolerance,
            stock_to_leave,
        } => {
            *stepdown = sd.unwrap_or(*stepdown);
            *tolerance = length(s, p, "tolerance")?.unwrap_or(*tolerance);
            *stock_to_leave = length(s, p, "stock_to_leave")?.unwrap_or(*stock_to_leave);
        }
        OpKind::Adaptive {
            stepdown,
            max_engagement,
            stock_to_leave,
            holder_check,
        } => {
            *stepdown = sd.unwrap_or(*stepdown);
            *max_engagement = angle(s, p, "max_engagement")?.unwrap_or(*max_engagement);
            *stock_to_leave = length(s, p, "stock_to_leave")?.unwrap_or(*stock_to_leave);
            *holder_check = holder_check_param(p)?.unwrap_or(*holder_check);
        }
        OpKind::Pencil {
            tolerance,
            stock_to_leave,
            min_angle,
        } => {
            *tolerance = length(s, p, "tolerance")?.unwrap_or(*tolerance);
            *stock_to_leave = length(s, p, "stock_to_leave")?.unwrap_or(*stock_to_leave);
            *min_angle = angle(s, p, "min_angle")?.unwrap_or(*min_angle);
        }
        OpKind::Scallop {
            stepover,
            tolerance,
            stock_to_leave,
            max_slope,
        } => {
            *stepover = so.unwrap_or(*stepover);
            *tolerance = length(s, p, "tolerance")?.unwrap_or(*tolerance);
            *stock_to_leave = length(s, p, "stock_to_leave")?.unwrap_or(*stock_to_leave);
            *max_slope = angle(s, p, "max_slope")?.unwrap_or(*max_slope);
        }
    }
    Ok(())
}

fn holder_check_param(p: &Value) -> Result<Option<HolderCheck>, CommandError> {
    match params::opt_str(p, "holder_check")? {
        None => Ok(None),
        Some(h) => HolderCheck::parse(h)
            .map(Some)
            .ok_or_else(|| bad("`holder_check` is ignore, lift or refuse")),
    }
}

fn default_op(kind: &str, tool_d: f64) -> Result<OpKind, CommandError> {
    Ok(match kind {
        "facing" => OpKind::Facing {
            stepover: 0.5,
            stepdown: 1.0,
        },
        "contour" => OpKind::Contour {
            face: None,
            edges: Vec::new(),
            side: Side::Right,
            stepdown: 2.0,
            lead_radius: tool_d / 2.0,
            bottom_offset: 0.0,
            climb: true,
        },
        "pocket" => OpKind::Pocket {
            face: String::new(),
            stepover: 0.4,
            stepdown: 2.0,
            entry: Entry::Helix,
            ramp_angle: 3f64.to_radians(),
            helix_radius: tool_d / 4.0,
        },
        "drill" => OpKind::Drill {
            faces: Vec::new(),
            peck: None,
            retract: 2.0,
            breakthrough: 0.0,
        },
        "parallel" => OpKind::Parallel {
            stepover: 0.1,
            scallop: None,
            angle: 0.0,
            tolerance: 0.01,
            stock_to_leave: 0.0,
            holder_check: HolderCheck::Ignore,
        },
        "waterline" => OpKind::Waterline {
            stepdown: 1.0,
            tolerance: 0.01,
            stock_to_leave: 0.0,
        },
        "adaptive" => OpKind::Adaptive {
            stepdown: 2.0,
            max_engagement: 60f64.to_radians(),
            stock_to_leave: 0.3,
            holder_check: HolderCheck::Ignore,
        },
        "pencil" => OpKind::Pencil {
            tolerance: 0.01,
            stock_to_leave: 0.0,
            min_angle: 10f64.to_radians(),
        },
        "scallop" => OpKind::Scallop {
            stepover: 0.1,
            tolerance: 0.01,
            stock_to_leave: 0.0,
            max_slope: 60f64.to_radians(),
        },
        _ => {
            return Err(bad(
                "`kind` is facing, contour, pocket, drill, parallel, waterline, adaptive, \
                 pencil or scallop",
            ));
        }
    })
}

fn op_reply(s: &Session, id: u32) -> Value {
    let rapid_rate = s
        .doc
        .cam
        .operation(id)
        .and_then(|o| s.doc.cam.setup(o.setup))
        .map_or(5000.0, |st| st.rapid_rate);
    let mut v = json!({"operation": id});
    if let Some(o) = s.doc.cam.operation(id) {
        v["name"] = json!(o.name);
        v["kind"] = json!(o.op.key());
        v["setup"] = json!(o.setup);
        v["tool"] = json!(o.tool);
        v["suppressed"] = json!(o.suppressed);
        if matches!(o.op, OpKind::Scallop { .. }) {
            v["experimental"] = json!(true);
        }
        if let OpKind::Drill { faces, .. } = &o.op {
            let bodies: Vec<chamfera_kernel::Body> =
                s.solids().iter().map(|x| x.body.clone()).collect();
            if let Ok(h) = chamfera_cam::drill_holes(&bodies, faces) {
                if let Some(t) = s.doc.cam.tool(o.tool) {
                    let w = chamfera_cam::ops::drill_warnings(
                        t.diameter,
                        t.kind == chamfera_doc::cam::ToolKind::Drill,
                        t.kind.words(),
                        &h,
                    );
                    if !w.is_empty() {
                        v["warnings"] = json!(w);
                    }
                }
                v["holes"] = json!(
                    h.iter()
                        .map(
                            |h| json!({"x": h.centre.x, "y": h.centre.y, "diameter": h.diameter,
                                        "top": h.top, "bottom": h.bottom})
                        )
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    match s.toolpath(id) {
        Some(Ok(tp)) => {
            let st = tp.stats(rapid_rate);
            v["moves"] = json!(tp.moves.len());
            v["feed_length"] = json!(st.feed_length);
            v["rapid_length"] = json!(st.rapid_length);
            v["minutes"] = json!(st.minutes);
        }
        Some(Err(e)) => v["error"] = json!(e),
        None => {}
    }
    v
}

fn op_add(s: &mut Session, p: &Value) -> CmdResult {
    let kind = params::req_str(p, "kind")?;
    let tool_id = params::req_id(p, "tool")?;
    let tool_d = s
        .doc
        .cam
        .tool(tool_id)
        .map(|t| t.diameter)
        .ok_or_else(|| bad(format!("no tool {tool_id}: add one with cam.tool_add")))?;
    let mut op = default_op(kind, tool_d)?;
    let setup = match opt_id(p, "setup")? {
        Some(id) => id,
        None => match s.doc.cam.setups.last() {
            Some(st) => st.id,
            None => setup(s, &json!({}))?["setup"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| bad("could not make a setup"))?,
        },
    };
    apply(s, p, &mut op, tool_d)?;
    if let OpKind::Pocket { face, .. } = &op
        && face.is_empty()
    {
        return Err(bad("a pocket needs `face`: its floor"));
    }
    if let OpKind::Contour { face, edges, .. } = &op
        && face.is_none()
        && edges.is_empty()
    {
        return Err(bad("a contour needs `face` or `edges`"));
    }
    let id = s.doc.cam.next_id()?;
    let n = s
        .doc
        .cam
        .operations
        .iter()
        .filter(|o| o.op.key() == kind)
        .count()
        + 1;
    let mut word = kind.to_owned();
    if let Some(f) = word.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    let name = opt_name(p)?.unwrap_or_else(|| format!("{word} {n}"));
    s.doc.cam.operations.push(CamOperation {
        id,
        name,
        setup,
        tool: tool_id,
        suppressed: false,
        op,
    });
    checked(s)?;
    s.regenerate_cam_now();
    s.set_toolpaths_visible(true);
    Ok(op_reply(s, id))
}

fn op_edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "operation")?;
    let mut o = s
        .doc
        .cam
        .operation(id)
        .cloned()
        .ok_or_else(|| bad(format!("no operation {id}")))?;
    if let Some(t) = opt_id(p, "tool")? {
        o.tool = t;
    }
    if let Some(st) = opt_id(p, "setup")? {
        o.setup = st;
    }
    if let Some(n) = opt_name(p)? {
        o.name = n;
    }
    if let Some(b) = opt_bool(p, "suppressed")? {
        o.suppressed = b;
    }
    let tool_d = s.doc.cam.tool(o.tool).map_or(1.0, |t| t.diameter);
    apply(s, p, &mut o.op, tool_d)?;
    if let Some(slot) = s.doc.cam.operations.iter_mut().find(|x| x.id == id) {
        *slot = o;
    }
    checked(s)?;
    s.regenerate_cam_now();
    Ok(op_reply(s, id))
}

fn op_delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "operation")?;
    let before = s.doc.cam.operations.len();
    s.doc.cam.operations.retain(|o| o.id != id);
    if s.doc.cam.operations.len() == before {
        return Err(bad(format!("no operation {id}")));
    }
    Ok(json!({"deleted": id}))
}

fn generate(s: &mut Session, p: &Value) -> CmdResult {
    let only = opt_id(p, "operation")?;
    if let Some(id) = only
        && s.doc.cam.operation(id).is_none()
    {
        return Err(bad(format!("no operation {id}")));
    }
    s.regenerate_cam_now();
    s.set_toolpaths_visible(true);
    s.refresh_overlays();
    let ids: Vec<u32> = s
        .doc
        .cam
        .operations
        .iter()
        .map(|o| o.id)
        .filter(|id| only.is_none_or(|o| o == *id))
        .collect();
    let ops: Vec<Value> = ids.iter().map(|id| op_reply(s, *id)).collect();
    let minutes: f64 = ops.iter().filter_map(|o| o["minutes"].as_f64()).sum();
    let errors = ops.iter().filter(|o| o.get("error").is_some()).count();
    // Each operation's warnings, named after it: worth reading, but the toolpath stands.
    let warnings: Vec<String> = ops
        .iter()
        .flat_map(|o| {
            let name = o["name"].as_str().unwrap_or("").to_owned();
            o["warnings"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |w| w.as_str().map(|w| format!("{name}: {w}")))
        })
        .collect();
    Ok(json!({"operations": ops, "minutes": minutes, "errors": errors, "warnings": warnings}))
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.ensure_cam_current();
    let setups: Vec<Value> = s
        .doc
        .cam
        .setups
        .iter()
        .map(|st| setup_json(s, st))
        .collect();
    let tools: Vec<Value> = s
        .doc
        .cam
        .tools
        .iter()
        .map(|t| {
            json!({"tool": t.id, "number": t.number, "name": t.name, "kind": t.kind.key(),
                   "diameter": t.diameter, "corner_radius": t.corner_radius,
                   "flutes": t.flutes, "spindle_rpm": t.spindle_rpm, "feed": t.feed,
                   "plunge_feed": t.plunge_feed, "flute_length": t.flute_length,
                   "holder": t.holder})
        })
        .collect();
    let ids: Vec<u32> = s.doc.cam.operations.iter().map(|o| o.id).collect();
    let ops: Vec<Value> = ids.iter().map(|id| op_reply(s, *id)).collect();
    Ok(
        json!({"setups": setups, "tools": tools, "operations": ops, "visible": s.toolpaths_visible()}),
    )
}

fn simulate(s: &mut Session, p: &Value) -> CmdResult {
    if opt_bool(p, "clear")? == Some(true) {
        s.clear_simulation();
        return Ok(json!({"cleared": true}));
    }
    let ops = match obj(p)?.get("ops") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            if a.len() > chamfera_doc::cam::MAX_CAM_ITEMS {
                return Err(bad("`ops` holds too many operations"));
            }
            Some(
                a.iter()
                    .map(|v| {
                        v.as_u64()
                            .and_then(|x| u32::try_from(x).ok())
                            .ok_or_else(|| bad("`ops` must be a list of operation ids"))
                    })
                    .collect::<Result<Vec<u32>, _>>()?,
            )
        }
        Some(_) => return Err(bad("`ops` must be a list of operation ids")),
    };
    let resolution = match obj(p)?.get("resolution") {
        None | Some(Value::Null) => 128,
        _ => {
            let r = params::req_id(p, "resolution")?;
            let max = chamfera_cam::sim::MAX_SIM_RESOLUTION;
            if r == 0 || r as usize > max {
                return Err(bad(format!(
                    "`resolution` must be from 1 to {max} columns per side"
                )));
            }
            r as usize
        }
    };
    let time = params::opt_f64(p, "time")?.unwrap_or(1.0);
    if !(0.0..=1.0).contains(&time) {
        return Err(bad("`time` must be from 0 to 1"));
    }
    let tolerance = length(s, p, "tolerance")?.unwrap_or(0.02);
    if !(0.0..=10.0).contains(&tolerance) {
        return Err(bad("`tolerance` must be from 0 to 10 mm"));
    }
    let req = crate::cam::SimRequest {
        setup: opt_id(p, "setup")?,
        ops,
        resolution,
        tolerance,
        time,
    };
    let st = s.simulate(&req).map_err(CommandError::Unavailable)?;
    let r = st.report;
    let h = st.holder;
    Ok(json!({
        "setup": st.setup,
        "ops": st.ops,
        "resolution": st.resolution,
        "cell": st.cell,
        "time": st.time,
        "removed_volume": r.removed_volume,
        "rest_volume": r.rest_volume,
        "gouges": {"cells": r.gouge_cells, "volume": r.gouge_volume, "max_depth": r.max_gouge},
        "holder": {"checked": h.checked, "moves": h.moves, "cells": h.cells,
                   "max_depth": h.max_depth, "incomplete": h.incomplete},
    }))
}

fn show(s: &mut Session, p: &Value) -> CmdResult {
    let v = params::req_bool(p, "visible")?;
    s.set_toolpaths_visible(v);
    s.refresh_overlays();
    Ok(json!({"visible": v}))
}

fn post(s: &mut Session, p: &Value) -> CmdResult {
    let dialect = match params::opt_str(p, "dialect")? {
        None => chamfera_cam::Dialect::LinuxCnc,
        Some(d) => {
            chamfera_cam::Dialect::parse(d).ok_or_else(|| bad("`dialect` is linuxcnc or grbl"))?
        }
    };
    let path = params::opt_str(p, "path")?.map(str::to_owned);
    let setup_id = match opt_id(p, "setup")? {
        Some(id) => id,
        None => s
            .doc
            .cam
            .setups
            .first()
            .map(|st| st.id)
            .ok_or_else(|| CommandError::Unavailable("there is no setup to post".into()))?,
    };
    let st = s
        .doc
        .cam
        .setup(setup_id)
        .cloned()
        .ok_or_else(|| bad(format!("no setup {setup_id}")))?;
    s.regenerate_cam_now();
    let mut ops = Vec::new();
    for o in s
        .doc
        .cam
        .operations
        .iter()
        .filter(|o| o.setup == setup_id && !o.suppressed)
    {
        match s.toolpath(o.id) {
            Some(Ok(tp)) => ops.push((o.name.clone(), tp.clone())),
            Some(Err(e)) => {
                return Err(CommandError::Unavailable(format!(
                    "{} has no toolpath: {e}",
                    o.name
                )));
            }
            None => {}
        }
    }
    if ops.is_empty() {
        return Err(CommandError::Unavailable(
            "the setup has no operations to post".into(),
        ));
    }
    let bodies: Vec<chamfera_kernel::Body> = s.solids().iter().map(|x| x.body.clone()).collect();
    let geo = chamfera_cam::setup_geometry(&st, &bodies, crate::solids::edge_ref)
        .map_err(|e| CommandError::Unavailable(e.to_string()))?;
    let program = s.path.as_ref().and_then(|p| p.file_stem()).map_or_else(
        || "Chamfera".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let text = chamfera_cam::post::post(&format!("{program} {}", st.name), &ops, &geo.wcs, dialect);
    let lines = text.lines().count();
    match path {
        Some(path) => {
            std::fs::write(&path, text.as_bytes()).map_err(chamfera_io::IoError::from)?;
            Ok(json!({"path": path, "lines": lines, "operations": ops.len()}))
        }
        None => Ok(json!({"gcode": text, "lines": lines, "operations": ops.len()})),
    }
}

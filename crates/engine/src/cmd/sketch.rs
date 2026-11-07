//! `sketch.*`: create sketches on a plane, draw, constrain, dimension and drag, the
//! sketch tools (trim, extend, split, fillet, chamfer, offset, mirror, patterns), the
//! shapes (polygon, slot, ellipse) and projecting model edges into a sketch.
//!
//! Drawing commands act on the active sketch (`sketch.create` or `sketch.edit` makes one
//! active, `sketch.finish` ends editing). Coordinates are sketch-plane millimetres.
//! Commands that change a sketch solve it, and a failed solve changes nothing.

use super::CommandSpec;
use crate::naming::{Kind, NameError, Want};
use crate::sketching::SketchItem;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::{Frame, Point2};
use chamfera_sketch::{Constraint, Curve, CurveId, OffsetCorners, PointId, Sketch, ToolOp};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sketch.arc",
        label: "Arc",
        params: "cx, cy: number (required) - centre\n\
                 x1, y1: number (required) - start; the radius is its distance from the centre\n\
                 x2, y2: number (required) - end direction; the arc runs counter-clockwise",
        mutates: true,
        run: arc,
    },
    CommandSpec {
        id: "sketch.arc.tangent",
        label: "Tangent Arc",
        params: "point: integer (required) - the free end of a line or arc to continue from\n\
                 x, y: number (required) - where the arc ends",
        mutates: true,
        run: arc_tangent,
    },
    CommandSpec {
        id: "sketch.arc.three_point",
        label: "3-Point Arc",
        params: "x1, y1: number (required) - start\n\
                 x2, y2: number (required) - a point the arc passes through\n\
                 x3, y3: number (required) - end",
        mutates: true,
        run: arc_three_point,
    },
    CommandSpec {
        id: "sketch.chamfer",
        label: "Sketch Chamfer",
        params: "point: integer (required) - the corner where two lines meet\n\
                 distance: number (required) - mm along each line from the corner\n\
                 distance2: number (optional) - mm along the second line instead",
        mutates: true,
        run: chamfer,
    },
    CommandSpec {
        id: "sketch.circle",
        label: "Circle",
        params: "cx, cy: number (required) - centre\nradius: number (required) - mm, above 0",
        mutates: true,
        run: circle,
    },
    CommandSpec {
        id: "sketch.circle.tangent",
        label: "Tangent Circle",
        params: "curves: array of integers (required) - two or three lines, circles or arcs to touch\n\
                 x, y: number (required) - a first guess of the centre; the circle found is the nearest one, \
                 on the side of each curve the guess is on\n\
                 radius: number (required with two curves) - mm; becomes a radius dimension",
        mutates: true,
        run: circle_tangent,
    },
    CommandSpec {
        id: "sketch.circle.three_point",
        label: "3-Point Circle",
        params: "x1, y1, x2, y2, x3, y3: number (required) - three points on the circle",
        mutates: true,
        run: circle_three_point,
    },
    CommandSpec {
        id: "sketch.circle.two_point",
        label: "2-Point Circle",
        params: "x1, y1, x2, y2: number (required) - the ends of a diameter",
        mutates: true,
        run: circle_two_point,
    },
    CommandSpec {
        id: "sketch.constrain",
        label: "Add Constraint",
        params: "constraint: object (required unless constraints) - {\"kind\": ..., ids and value}, for example \
                 {\"kind\": \"horizontal\", \"line\": 3} or {\"kind\": \"length\", \"line\": 3, \"value\": 50}\n\
                 constraints: array of objects (optional) - several at once, as one undo step; each one that \
                 would conflict or is already implied is left out and counted in `skipped`",
        mutates: true,
        run: constrain,
    },
    CommandSpec {
        id: "sketch.constrain_selection",
        label: "Constrain Selection",
        params: "kind: string (required) - horizontal or vertical (lines, or two points: level or \
                 one above the other), coincident, point_on, parallel, perpendicular, tangent, \
                 equal, midpoint, concentric (two circles or arcs), collinear (two lines) or fix",
        mutates: true,
        run: constrain_selection,
    },
    CommandSpec {
        id: "sketch.construction",
        label: "Construction",
        params: "curve: integer (required)\nconstruction: boolean (required)",
        mutates: true,
        run: construction,
    },
    CommandSpec {
        id: "sketch.create",
        label: "Create Sketch",
        params: "plane: string or integer (required) - xy, xz or yz (top, front, right), or the\n\
                 id of a construction plane (the sketch follows it when the design rebuilds)\n\
                 offset: number (optional) - move an origin plane along its normal, in mm \
                 (profiles for a loft)",
        mutates: true,
        run: create,
    },
    CommandSpec {
        id: "sketch.create_on_face",
        label: "Create Sketch on Face",
        params: "x, y, width, height: number - the pixel of a flat face of a solid; or\n\
                 feature: integer and face: string - a face by its name, such as \"top\" (\"top#2\" for one piece of a split face)\n\
                 point: {x, y, z} (optional) - a point on the face: it picks the piece of a split face, and the sketch stays on the piece holding it when the face splits later",
        mutates: true,
        run: create_on_face,
    },
    CommandSpec {
        id: "sketch.delete",
        label: "Delete Sketch",
        params: "sketch: integer (required)",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "sketch.dimension",
        label: "Edit Dimension",
        params: "constraint: integer (required) - a dimension\n\
                 value: number - mm or radians; or\n\
                 expression: string - such as \"width / 2\"; the dimension then follows it",
        mutates: true,
        run: dimension,
    },
    CommandSpec {
        id: "sketch.dimension.driven",
        label: "Driven Dimension",
        params: "constraint: integer (required) - a dimension\n\
                 driven: boolean (required) - true: it only shows the size the geometry has \
                 (a reference dimension); false: it sets the size again",
        mutates: true,
        run: dimension_driven,
    },
    CommandSpec {
        id: "sketch.dimension_selection",
        label: "Sketch Dimension",
        params: "dimensions the selection at its current size: one line (length), a circle (diameter), \
                 an arc (radius), two points (distance), a point and a line (distance) or two lines (angle)\n\
                 x, y: number (optional) - where the dimension is placed, in sketch mm: for two points or \
                 a slanted line, beside their span across X gives a horizontal dimension, beside their span \
                 across Y a vertical one, elsewhere an aligned one\n\
                 direction: string (optional) - aligned, horizontal or vertical, instead of choosing by place\n\
                 arc_length: boolean (optional) - for an arc, its length instead of its radius\n\
                 driven: boolean (optional) - only show the size; a dimension that would over-constrain \
                 the sketch is made driven anyway, and the reply says so",
        mutates: true,
        run: dimension_selection,
    },
    CommandSpec {
        id: "sketch.drag",
        label: "Drag Sketch Point",
        params: "point: integer (required)\nx, y: number (required) - target in sketch mm",
        mutates: true,
        run: drag,
    },
    CommandSpec {
        id: "sketch.edit",
        label: "Edit Sketch",
        params: "sketch: integer (required)",
        mutates: false,
        run: edit,
    },
    CommandSpec {
        id: "sketch.ellipse",
        label: "Ellipse",
        params: "cx, cy: number (required) - centre\n\
                 x, y: number (required) - the end of one axis\n\
                 minor: number (required) - mm, the other semi-axis",
        mutates: true,
        run: ellipse,
    },
    CommandSpec {
        id: "sketch.ellipse.arc",
        label: "Elliptical Arc",
        params: "cx, cy: number (required) - centre\n\
                 x, y: number (required) - the end of one axis\n\
                 minor: number (required) - mm, the other semi-axis\n\
                 x1, y1, x2, y2: number (required) - start and end directions from the centre; \
                 the arc runs counter-clockwise",
        mutates: true,
        run: ellipse_arc,
    },
    CommandSpec {
        id: "sketch.extend",
        label: "Extend",
        params: "curve: integer (required) - a line or arc\n\
                 x, y: number (required) - near the end to lengthen; it runs on to the first curve it meets",
        mutates: true,
        run: extend,
    },
    CommandSpec {
        id: "sketch.fillet",
        label: "Sketch Fillet",
        params: "point: integer (required) - the corner where two lines meet\n\
                 radius: number (required) - mm",
        mutates: true,
        run: fillet,
    },
    CommandSpec {
        id: "sketch.finish",
        label: "Finish Sketch",
        params: "(none)",
        mutates: false,
        run: finish,
    },
    CommandSpec {
        id: "sketch.get",
        label: "Get Sketch",
        params: "sketch: integer (optional) - default the active sketch",
        mutates: false,
        run: get,
    },
    CommandSpec {
        id: "sketch.line",
        label: "Line",
        params: "x1, y1, x2, y2: number (required) - ends\n\
                 start, end: integer (optional) - existing points to use instead of new ones\n\
                 snap: boolean (optional) - add horizontal or vertical within 2 degrees",
        mutates: true,
        run: line,
    },
    CommandSpec {
        id: "sketch.mirror",
        label: "Mirror",
        params: "axis: integer (required) - the line to mirror in\n\
                 curves, points: array of integers (optional) - what to mirror; default the selection",
        mutates: true,
        run: mirror,
    },
    CommandSpec {
        id: "sketch.move",
        label: "Move Sketch Geometry",
        params: "curves, points: array of integers (optional) - what to move; default the selection\n\
                 dx, dy: number (required) - mm\n\
                 copy: boolean (optional) - move a copy and keep the original (default false)",
        mutates: true,
        run: move_geometry,
    },
    CommandSpec {
        id: "sketch.offset",
        label: "Offset",
        params: "curves: array of integers (optional) - a chain of lines and arcs, one curve for its \
                 whole chain, or a circle; default the selection\n\
                 distance: number (required) - mm; positive outward (closed chains) or to the left\n\
                 x, y: number (optional) - offset toward this side instead\n\
                 corners: string (optional) - sharp (default) or round\n\
                 dimension: boolean (optional) - add the distance dimensions (default true)",
        mutates: true,
        run: offset,
    },
    CommandSpec {
        id: "sketch.pattern.circular",
        label: "Circular Pattern",
        params: "curves, points: array of integers (optional) - what to copy; default the selection\n\
                 cx, cy: number (required) - the centre\n\
                 count: integer (required) - instances in all, 2 or more\n\
                 angle: number (optional) - radians to spread them over; default a full turn",
        mutates: true,
        run: pattern_circular,
    },
    CommandSpec {
        id: "sketch.pattern.rectangular",
        label: "Rectangular Pattern",
        params: "curves, points: array of integers (optional) - what to copy; default the selection\n\
                 columns: integer (required) - instances along X; dx: number (required) - spacing\n\
                 rows: integer (optional) - instances along Y, default 1; dy: number (optional) - spacing",
        mutates: true,
        run: pattern_rectangular,
    },
    CommandSpec {
        id: "sketch.pick",
        label: "Pick in Sketch",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: pick,
    },
    CommandSpec {
        id: "sketch.plane_point",
        label: "Point on Sketch Plane",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: plane_point,
    },
    CommandSpec {
        id: "sketch.point",
        label: "Point",
        params: "x, y: number (required)",
        mutates: true,
        run: point,
    },
    CommandSpec {
        id: "sketch.polygon",
        label: "Polygon",
        params: "cx, cy: number (required) - centre\n\
                 x, y: number (required) - a corner, or with circumscribed the middle of a side\n\
                 sides: integer (required) - 3 to 64\n\
                 mode: string (optional) - inscribed (default) or circumscribed",
        mutates: true,
        run: polygon,
    },
    CommandSpec {
        id: "sketch.project",
        label: "Project",
        params: "edges: list of strings (optional) - model edges by name (solid.edges)\n\
                 faces: list of {feature: integer, face: string} (optional) - every edge of each face\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, else that face's edges\n\
                 linked: boolean (optional) - follow the edges when the model rebuilds (default true)",
        mutates: true,
        run: project,
    },
    CommandSpec {
        id: "sketch.rectangle",
        label: "Rectangle",
        params: "x1, y1, x2, y2: number (required) - opposite corners",
        mutates: true,
        run: rectangle,
    },
    CommandSpec {
        id: "sketch.rectangle.centre",
        label: "Centre Rectangle",
        params: "cx, cy: number (required) - centre\n\
                 x, y: number (required) - a corner",
        mutates: true,
        run: rectangle_centre,
    },
    CommandSpec {
        id: "sketch.rectangle.three_point",
        label: "3-Point Rectangle",
        params: "x1, y1, x2, y2: number (required) - the ends of one side, at any angle\n\
                 x3, y3: number (required) - a point on the opposite side",
        mutates: true,
        run: rectangle_three_point,
    },
    CommandSpec {
        id: "sketch.remove",
        label: "Delete",
        params: "(none) - deletes the selected points and curves, and what depends on them\n\
                 constraint: integer (optional) - delete this constraint instead",
        mutates: true,
        run: remove,
    },
    CommandSpec {
        id: "sketch.rename",
        label: "Rename Sketch",
        params: "sketch: integer (required)\n\
                 name: string (required) - the new name, 1 to 80 characters",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "sketch.rotate",
        label: "Rotate Sketch Geometry",
        params: "curves, points: array of integers (optional) - what to turn; default the selection\n\
                 cx, cy: number (required) - the centre\n\
                 angle: number (required) - radians, counter-clockwise\n\
                 copy: boolean (optional) - turn a copy and keep the original (default false)",
        mutates: true,
        run: rotate_geometry,
    },
    CommandSpec {
        id: "sketch.scale",
        label: "Scale Sketch Geometry",
        params: "curves, points: array of integers (optional) - what to scale; default the selection\n\
                 cx, cy: number (required) - the centre\n\
                 factor: number (required) - above 0 and at most 1e6; dimensions scale with it\n\
                 copy: boolean (optional) - scale a copy and keep the original (default false)",
        mutates: true,
        run: scale_geometry,
    },
    CommandSpec {
        id: "sketch.select",
        label: "Select in Sketch",
        params: "x, y, width, height: number (required) - pixel and viewport size\n\
                 add: boolean (optional) - add to the selection instead of replacing it\n\
                 filter: list of strings (optional) - what may be picked: vertices (points), \
                 sketch (curves) and construction (construction curves); default all",
        mutates: false,
        run: select,
    },
    CommandSpec {
        id: "sketch.select_clear",
        label: "Clear Sketch Selection",
        params: "(none)",
        mutates: false,
        run: select_clear,
    },
    CommandSpec {
        id: "sketch.slot",
        label: "Slot",
        params: "x1, y1, x2, y2: number (required) - the centres of the round ends, or with \
                 overall their tips\n\
                 width: number (required) - mm\n\
                 overall: boolean (optional) - the points are the tips (default false)",
        mutates: true,
        run: slot,
    },
    CommandSpec {
        id: "sketch.split",
        label: "Split",
        params: "curve: integer (optional) - split this curve and the curves crossing it where they \
                 cross; without it every crossing in the sketch, so overlapping shapes give their regions",
        mutates: true,
        run: split,
    },
    CommandSpec {
        id: "sketch.trim",
        label: "Trim",
        params: "curve: integer (required)\n\
                 x, y: number (required) - on the piece to remove, between the neighbouring crossings",
        mutates: true,
        run: trim,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn sketch_err(e: impl std::fmt::Display) -> CommandError {
    CommandError::Unavailable(e.to_string())
}

fn active_id(s: &Session) -> Result<u32, CommandError> {
    s.active_sketch().map(|f| f.id).ok_or_else(|| {
        CommandError::Unavailable("no sketch is being edited: Create Sketch or Edit Sketch".into())
    })
}

fn active(s: &mut Session) -> Result<&mut Sketch, CommandError> {
    let id = active_id(s)?;
    Ok(&mut s.doc.sketch_mut(id)?.sketch)
}

fn coord(p: &Value, key: &str) -> Result<f64, CommandError> {
    let v = params::req_f64(p, key)?;
    if !chamfera_geom::is_valid_coordinate(v) {
        return Err(bad(format!("`{key}` is outside the working range")));
    }
    Ok(v)
}

fn pt(p: &Value, x: &str, y: &str) -> Result<Point2, CommandError> {
    Ok(Point2::new(coord(p, x)?, coord(p, y)?))
}

fn opt_point_id(p: &Value, key: &str) -> Result<Option<PointId>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_id(p, key).map(|v| Some(PointId(v))),
    }
}

/// Solves the active sketch; the caller's snapshot rolls back on error.
fn solve(sk: &mut Sketch) -> Result<(), CommandError> {
    sk.solve().map(|_| ()).map_err(sketch_err)
}

fn create(s: &mut Session, p: &Value) -> CmdResult {
    if p.get("plane").is_some_and(Value::is_number) {
        let construct = params::req_id(p, "plane")?;
        let plane = match s.constructs().get(&construct) {
            Some(crate::construct::Constructed::Plane { frame, .. }) => *frame,
            _ => {
                return Err(bad(format!(
                    "feature {construct} is not a built construction plane"
                )));
            }
        };
        let id = s.doc.add_sketch(plane)?;
        if let Ok(f) = s.doc.sketch_mut(id) {
            f.on_plane = Some(construct);
        }
        s.active_sketch = Some(id);
        s.sketch_selection.clear();
        let n = plane.z();
        return Ok(json!({
            "sketch": id,
            "plane": construct,
            "origin": plane.origin(),
            "normal": { "x": n.x, "y": n.y, "z": n.z },
        }));
    }
    let plane = match params::req_str(p, "plane")? {
        "xy" => Frame::XY,
        "xz" => Frame::XZ,
        "yz" => Frame::YZ,
        other => return Err(bad(format!("unknown plane {other:?}: use xy, xz or yz"))),
    };
    let plane = match params::opt_f64(p, "offset")? {
        None => plane,
        Some(d) if d.is_finite() && d.abs() <= chamfera_geom::MAX_COORDINATE => plane
            .with_origin(plane.origin() + plane.z() * d)
            .ok_or_else(|| bad("the offset plane is out of range"))?,
        Some(_) => {
            return Err(bad(format!(
                "`offset` must be a finite distance of at most {} mm",
                chamfera_geom::MAX_COORDINATE
            )));
        }
    };
    let id = s.doc.add_sketch(plane)?;
    s.active_sketch = Some(id);
    s.sketch_selection.clear();
    Ok(json!({ "sketch": id }))
}

fn create_on_face(s: &mut Session, p: &Value) -> CmdResult {
    let mut point = params::opt_point3(p, "point")?;
    let (feature, face, plane) = if p.get("feature").is_some_and(|v| !v.is_null()) {
        let feature = params::req_id(p, "feature")?;
        let face = params::req_str(p, "face")?.to_owned();
        // The face may be in any body: joins move a feature's faces into another body.
        // Through the resolver (rung K8): `top#2` names one piece of a split face, and
        // `point` picks the piece it lies on.
        let key = crate::solids::face_key(feature, &face);
        let found = s
            .resolve_at_point(Kind::Face, &key, point, Want::Surface)
            .map_err(|e| match e {
                NameError::Gone { .. } => {
                    bad(format!("feature {feature} has no face named {face:?}"))
                }
                e => bad(e.to_string()),
            })?;
        let f = found
            .first()
            .ok_or_else(|| bad(format!("feature {feature} has no face named {face:?}")))?;
        let plane = s
            .solids()
            .get(f.body)
            .zip(f.face())
            .and_then(|(solid, id)| crate::solids::face_plane(&solid.body, id))
            .ok_or_else(|| bad("sketches go on flat faces"))?;
        (feature, face, plane)
    } else {
        let (x, y, w, h) = params::pixel(p)?;
        let at = s.face_hit_at(x, y, w, h).map(|h| h.2);
        let found = s
            .face_at(x, y, w, h)
            .ok_or_else(|| bad("no flat face of a solid is under that point"))?;
        point = point.or(at);
        found
    };
    let id = s.doc.add_sketch(plane)?;
    let r = chamfera_doc::FaceRef {
        feature,
        face: face.clone(),
    };
    if let Ok(f) = s.doc.sketch_mut(id) {
        f.attached = Some(r.clone());
    }
    // What the sketch is on, so that it stays on that piece when the face splits.
    s.record_face_signature(id, &r, point);
    s.active_sketch = Some(id);
    s.sketch_selection.clear();
    let n = plane.z();
    Ok(json!({
        "sketch": id,
        "feature": feature,
        "face": face,
        "origin": plane.origin(),
        "normal": { "x": n.x, "y": n.y, "z": n.z },
    }))
}

fn edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "sketch")?;
    s.doc.sketch(id)?;
    s.active_sketch = Some(id);
    s.sketch_selection.clear();
    Ok(json!({ "sketch": id }))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "sketch")?;
    let name = params::req_display_name(p, "name")?;
    s.doc.sketch(id)?;
    s.doc.rename_item(id, &name)?;
    Ok(json!({ "sketch": id, "name": s.doc.sketch(id)?.name }))
}

fn finish(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let id = active_id(s)?;
    s.active_sketch = None;
    s.sketch_selection.clear();
    Ok(json!({ "sketch": id }))
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "sketch")?;
    let users = s.doc.sketch_users(id);
    if !users.is_empty() {
        return Err(CommandError::Unavailable(format!(
            "{} uses this sketch; delete or change it first",
            users.join(", ")
        )));
    }
    s.doc.remove_sketch(id)?;
    if s.active_sketch == Some(id) {
        s.active_sketch = None;
    }
    Ok(json!({ "deleted": id }))
}

fn point(s: &mut Session, p: &Value) -> CmdResult {
    let at = pt(p, "x", "y")?;
    let id = active(s)?.add_point(at).map_err(sketch_err)?;
    Ok(json!({ "point": id.0 }))
}

/// Uses the given point id (checked) or adds a point at the coordinates.
fn point_or_new(sk: &mut Sketch, id: Option<PointId>, at: Point2) -> Result<PointId, CommandError> {
    match id {
        Some(id) if sk.point(id).is_some() => Ok(id),
        Some(id) => Err(bad(format!("no {id}"))),
        None => sk.add_point(at).map_err(sketch_err),
    }
}

fn line(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    let start = opt_point_id(p, "start")?;
    let end = opt_point_id(p, "end")?;
    let snap = match p.get("snap") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "snap")?,
    };
    let sk = active(s)?;
    let start = point_or_new(sk, start, a)?;
    let end = point_or_new(sk, end, b)?;
    if start == end {
        return Err(bad("a line needs two different points"));
    }
    let id = sk.add_line(start, end).map_err(sketch_err)?;
    let mut added = None;
    if snap && let (Some(a), Some(b)) = (sk.point(start), sk.point(end)) {
        let ang = (b - a).angle().abs();
        let tol = 2f64.to_radians();
        let c = if ang < tol || (std::f64::consts::PI - ang) < tol {
            Some(Constraint::Horizontal { line: id })
        } else if (ang - std::f64::consts::FRAC_PI_2).abs() < tol {
            Some(Constraint::Vertical { line: id })
        } else {
            None
        };
        if let Some(c) = c {
            added = Some(c.kind_name());
            sk.add_constraint(c).map_err(sketch_err)?;
        }
    }
    solve(sk)?;
    Ok(json!({ "curve": id.0, "start": start.0, "end": end.0, "snapped": added }))
}

fn rectangle(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    if (a.x - b.x).abs() < chamfera_geom::LINEAR_TOLERANCE
        || (a.y - b.y).abs() < chamfera_geom::LINEAR_TOLERANCE
    {
        return Err(bad("a rectangle needs a width and a height"));
    }
    let sk = active(s)?;
    let corners = [a, Point2::new(b.x, a.y), b, Point2::new(a.x, b.y)];
    let mut pts = Vec::with_capacity(4);
    for c in corners {
        pts.push(sk.add_point(c).map_err(sketch_err)?);
    }
    let mut lines = Vec::with_capacity(4);
    for k in 0..4 {
        lines.push(sk.add_line(pts[k], pts[(k + 1) % 4]).map_err(sketch_err)?);
    }
    for (k, l) in lines.iter().enumerate() {
        let c = if k % 2 == 0 {
            Constraint::Horizontal { line: *l }
        } else {
            Constraint::Vertical { line: *l }
        };
        sk.add_constraint(c).map_err(sketch_err)?;
    }
    solve(sk)?;
    Ok(json!({
        "points": pts.iter().map(|p| p.0).collect::<Vec<_>>(),
        "curves": lines.iter().map(|c| c.0).collect::<Vec<_>>(),
    }))
}

fn circle(s: &mut Session, p: &Value) -> CmdResult {
    let c = pt(p, "cx", "cy")?;
    let r = coord(p, "radius")?;
    if r <= chamfera_geom::LINEAR_TOLERANCE {
        return Err(bad("`radius` must be above 0"));
    }
    let sk = active(s)?;
    let centre = sk.add_point(c).map_err(sketch_err)?;
    let id = sk.add_circle(centre, r).map_err(sketch_err)?;
    Ok(json!({ "curve": id.0, "centre": centre.0 }))
}

fn arc(s: &mut Session, p: &Value) -> CmdResult {
    let c = pt(p, "cx", "cy")?;
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    let r = c.distance(a);
    let dir = (b - c)
        .normalized()
        .ok_or_else(|| bad("the end must not be on the centre"))?;
    if r <= chamfera_geom::LINEAR_TOLERANCE {
        return Err(bad("the start must not be on the centre"));
    }
    // The end lies on the circle through the start, in the direction of (x2, y2).
    let end_at = c + dir * r;
    if end_at.coincident(a) {
        return Err(bad("an arc needs different start and end directions"));
    }
    let sk = active(s)?;
    let centre = sk.add_point(c).map_err(sketch_err)?;
    let start = sk.add_point(a).map_err(sketch_err)?;
    let end = sk.add_point(end_at).map_err(sketch_err)?;
    let id = sk.add_arc(centre, start, end).map_err(sketch_err)?;
    solve(sk)?;
    Ok(json!({ "curve": id.0, "centre": centre.0, "start": start.0, "end": end.0 }))
}

/// At most this many constraints in one `sketch.constrain` call.
const MAX_CONSTRAINT_BATCH: usize = 64;

fn constrain(s: &mut Session, p: &Value) -> CmdResult {
    if let Some(list) = p.get("constraints") {
        return constrain_many(s, list);
    }
    let raw = p
        .get("constraint")
        .ok_or_else(|| bad("`constraint` is required"))?;
    let c: Constraint =
        serde_json::from_value(raw.clone()).map_err(|e| bad(format!("`constraint`: {e}")))?;
    let sk = active(s)?;
    let id = sk.try_add_constraint(c).map_err(sketch_err)?;
    Ok(json!({ "constraint": id.0 }))
}

/// Adds each constraint that solves and is not implied by the others; skips the rest.
fn constrain_many(s: &mut Session, list: &Value) -> CmdResult {
    let items = list
        .as_array()
        .ok_or_else(|| bad("`constraints` must be an array of constraint objects"))?;
    if items.len() > MAX_CONSTRAINT_BATCH {
        return Err(bad(format!(
            "`constraints` takes at most {MAX_CONSTRAINT_BATCH} constraints"
        )));
    }
    let mut parsed = Vec::with_capacity(items.len());
    for (k, raw) in items.iter().enumerate() {
        let c: Constraint = serde_json::from_value(raw.clone())
            .map_err(|e| bad(format!("`constraints[{k}]`: {e}")))?;
        parsed.push(c);
    }
    let sk = active(s)?;
    let mut added = Vec::new();
    let mut skipped = 0;
    for c in parsed {
        match sk.try_add_constraint(c) {
            Ok((id, report)) if report.dof.redundant.contains(&id) => {
                // Already implied: keep the sketch free of redundant constraints.
                sk.remove_constraint(id).map_err(sketch_err)?;
                skipped += 1;
            }
            Ok((id, _)) => added.push(id.0),
            Err(_) => skipped += 1,
        }
    }
    Ok(json!({ "constraints": added, "skipped": skipped }))
}

fn dimension(s: &mut Session, p: &Value) -> CmdResult {
    let id = chamfera_sketch::ConstraintId(params::req_id(p, "constraint")?);
    let sketch_id = active_id(s)?;
    let expression = params::opt_str(p, "expression")?.map(str::to_owned);
    let f = s.doc.sketch_mut(sketch_id)?;
    if f.sketch.constraint(id).is_none_or(|c| !c.is_dimension()) {
        return Err(bad(format!("{id} is not a dimension")));
    }
    if f.sketch.is_driven(id) {
        return Err(bad(format!(
            "{id} is a driven dimension: it shows a size; make it driving first (sketch.dimension.driven)"
        )));
    }
    match expression {
        // An expression drives the dimension from now on; evaluating applies and solves.
        Some(e) => {
            f.expressions.insert(id.0, e);
            s.doc.evaluate()?;
        }
        None => {
            let value = params::req_f64(p, "value")?;
            f.expressions.remove(&id.0);
            f.sketch.try_set_dimension(id, value).map_err(sketch_err)?;
        }
    }
    let f = s.doc.sketch(sketch_id)?;
    let value = f.sketch.constraint(id).and_then(|c| c.value());
    Ok(json!({
        "constraint": id.0,
        "value": value,
        "expression": f.expressions.get(&id.0),
    }))
}

fn dimension_driven(s: &mut Session, p: &Value) -> CmdResult {
    let id = chamfera_sketch::ConstraintId(params::req_id(p, "constraint")?);
    let driven = params::req_bool(p, "driven")?;
    let sketch_id = active_id(s)?;
    let f = s.doc.sketch_mut(sketch_id)?;
    let backup = f.sketch.clone();
    f.sketch.set_driven(id, driven).map_err(sketch_err)?;
    if driven {
        // A driven dimension follows the geometry, so no expression drives it.
        f.expressions.remove(&id.0);
    }
    if let Err(e) = f.sketch.solve() {
        f.sketch = backup;
        return Err(sketch_err(e));
    }
    let value = f.sketch.constraint(id).and_then(|c| c.value());
    Ok(json!({ "constraint": id.0, "driven": driven, "value": value }))
}

fn drag(s: &mut Session, p: &Value) -> CmdResult {
    let point = PointId(params::req_id(p, "point")?);
    let target = pt(p, "x", "y")?;
    let sk = active(s)?;
    let r = sk.drag(point, target).map_err(sketch_err)?;
    let at = sk.point(point);
    Ok(json!({ "point": point.0, "at": at, "reached": r.reached }))
}

fn construction(s: &mut Session, p: &Value) -> CmdResult {
    let curve = CurveId(params::req_id(p, "curve")?);
    let value = params::req_bool(p, "construction")?;
    active(s)?
        .set_construction(curve, value)
        .map_err(sketch_err)?;
    Ok(json!({ "curve": curve.0, "construction": value }))
}

fn selection(s: &Session) -> (Vec<PointId>, Vec<CurveId>) {
    let mut points = Vec::new();
    let mut curves = Vec::new();
    for item in &s.sketch_selection {
        match item {
            SketchItem::Point(id) => points.push(PointId(*id)),
            SketchItem::Curve(id) => curves.push(CurveId(*id)),
        }
    }
    (points, curves)
}

fn constrain_selection(s: &mut Session, p: &Value) -> CmdResult {
    let kind = params::req_str(p, "kind")?.to_owned();
    let (points, curves) = selection(s);
    let sk = active(s)?;
    let is_line = |sk: &Sketch, c: CurveId| sk.curve(c).is_some_and(Curve::is_line);
    let need = |ok: bool, what: &str| {
        if ok {
            Ok(())
        } else {
            Err(bad(format!("{kind} needs {what} selected")))
        }
    };
    let mut new: Vec<Constraint> = Vec::new();
    match kind.as_str() {
        "horizontal" | "vertical" if curves.is_empty() && points.len() == 2 => {
            let (a, b) = (points[0], points[1]);
            new.push(if kind == "horizontal" {
                Constraint::HorizontalPoints { a, b }
            } else {
                Constraint::VerticalPoints { a, b }
            });
        }
        "concentric" => {
            let circular = |c: &CurveId| sk.curve(*c).is_some_and(Curve::is_circular);
            need(
                points.is_empty() && curves.len() == 2 && curves.iter().all(circular),
                "two circles or arcs",
            )?;
            new.push(Constraint::Concentric {
                a: curves[0],
                b: curves[1],
            });
        }
        "collinear" => {
            need(
                points.is_empty() && curves.len() == 2 && curves.iter().all(|c| is_line(sk, *c)),
                "two lines",
            )?;
            new.push(Constraint::Collinear {
                a: curves[0],
                b: curves[1],
            });
        }
        "horizontal" | "vertical" => {
            need(
                !curves.is_empty() && points.is_empty(),
                "one or more lines, or two points",
            )?;
            for &line in &curves {
                need(is_line(sk, line), "only lines")?;
                new.push(if kind == "horizontal" {
                    Constraint::Horizontal { line }
                } else {
                    Constraint::Vertical { line }
                });
            }
        }
        "coincident" => {
            need(points.len() == 2 && curves.is_empty(), "two points")?;
            new.push(Constraint::Coincident {
                a: points[0],
                b: points[1],
            });
        }
        "point_on" => {
            need(
                points.len() == 1 && curves.len() == 1,
                "a point and a curve",
            )?;
            new.push(if is_line(sk, curves[0]) {
                Constraint::PointOnLine {
                    point: points[0],
                    line: curves[0],
                }
            } else {
                Constraint::PointOnCircle {
                    point: points[0],
                    curve: curves[0],
                }
            });
        }
        "midpoint" => {
            need(
                points.len() == 1 && curves.len() == 1 && is_line(sk, curves[0]),
                "a point and a line",
            )?;
            new.push(Constraint::Midpoint {
                point: points[0],
                line: curves[0],
            });
        }
        "parallel" | "perpendicular" => {
            need(
                points.is_empty() && curves.len() == 2 && curves.iter().all(|c| is_line(sk, *c)),
                "two lines",
            )?;
            let (a, b) = (curves[0], curves[1]);
            new.push(if kind == "parallel" {
                Constraint::Parallel { a, b }
            } else {
                Constraint::Perpendicular { a, b }
            });
        }
        "tangent" | "equal" => {
            need(points.is_empty() && curves.len() == 2, "two curves")?;
            let (a, b) = (curves[0], curves[1]);
            new.push(if kind == "tangent" {
                Constraint::Tangent { a, b }
            } else {
                Constraint::Equal { a, b }
            });
        }
        "fix" => {
            need(
                !points.is_empty() && curves.is_empty(),
                "one or more points",
            )?;
            for &point in &points {
                let at = sk
                    .point(point)
                    .ok_or_else(|| bad("a selected point is gone"))?;
                new.push(Constraint::Fix { point, at });
            }
        }
        other => return Err(bad(format!("unknown constraint kind {other:?}"))),
    }
    let mut ids = Vec::new();
    for c in new {
        ids.push(sk.try_add_constraint(c).map_err(sketch_err)?.0);
    }
    s.sketch_selection.clear();
    Ok(json!({ "constraints": ids }))
}

/// Which way a linear dimension between two points measures.
#[derive(Clone, Copy, PartialEq)]
enum Along {
    Aligned,
    Horizontal,
    Vertical,
}

/// The direction for a dimension between `a` and `b`: the one asked for, or by where it is
/// placed (`at`): beside the span across X (above or below the points) measures X, beside
/// the span across Y (left or right of them) measures Y, anywhere else the true distance.
/// Points level or one above the other always get an aligned dimension.
fn along(a: Point2, b: Point2, at: Option<Point2>, asked: Option<Along>) -> Along {
    if let Some(d) = asked {
        return d;
    }
    let tol = chamfera_geom::LINEAR_TOLERANCE;
    let Some(at) = at else {
        return Along::Aligned;
    };
    if (a.x - b.x).abs() <= tol || (a.y - b.y).abs() <= tol {
        return Along::Aligned;
    }
    let in_x = (a.x.min(b.x)..=a.x.max(b.x)).contains(&at.x);
    let in_y = (a.y.min(b.y)..=a.y.max(b.y)).contains(&at.y);
    match (in_x, in_y) {
        (true, false) => Along::Horizontal,
        (false, true) => Along::Vertical,
        _ => Along::Aligned,
    }
}

/// The dimension between points `a` and `b` (at `pa`, `pb`) measured `along`.
fn point_dimension(a: PointId, b: PointId, pa: Point2, pb: Point2, along: Along) -> Constraint {
    match along {
        Along::Aligned => Constraint::Distance {
            a,
            b,
            value: pa.distance(pb),
        },
        Along::Horizontal => Constraint::HorizontalDistance {
            a,
            b,
            value: (pb.x - pa.x).abs(),
        },
        Along::Vertical => Constraint::VerticalDistance {
            a,
            b,
            value: (pb.y - pa.y).abs(),
        },
    }
}

fn dimension_selection(s: &mut Session, p: &Value) -> CmdResult {
    let at = match (p.get("x"), p.get("y")) {
        (None | Some(Value::Null), None | Some(Value::Null)) => None,
        _ => Some(pt(p, "x", "y")?),
    };
    let asked = match params::opt_str(p, "direction")? {
        None => None,
        Some("aligned") => Some(Along::Aligned),
        Some("horizontal") => Some(Along::Horizontal),
        Some("vertical") => Some(Along::Vertical),
        Some(other) => {
            return Err(bad(format!(
                "unknown direction {other:?}: use aligned, horizontal or vertical"
            )));
        }
    };
    let opt_bool = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(_) => params::req_bool(p, key),
    };
    let arc_length = opt_bool("arc_length")?;
    let want_driven = opt_bool("driven")?;
    let (points, curves) = selection(s);
    let sk = active(s)?;
    let pos =
        |sk: &Sketch, id: PointId| sk.point(id).ok_or_else(|| bad("a selected point is gone"));
    let line_ends = |sk: &Sketch, c: CurveId| match sk.curve(c) {
        Some(Curve::Line { start, end, .. }) => Some((*start, *end)),
        _ => None,
    };
    let c = match (points.as_slice(), curves.as_slice()) {
        ([], [c]) => match sk.curve(*c) {
            Some(Curve::Line { start, end, .. }) => {
                let (pa, pb) = (pos(sk, *start)?, pos(sk, *end)?);
                match along(pa, pb, at, asked) {
                    Along::Aligned => Constraint::Length {
                        line: *c,
                        value: pa.distance(pb),
                    },
                    d => point_dimension(*start, *end, pa, pb, d),
                }
            }
            Some(Curve::Arc { .. }) if arc_length => Constraint::ArcLength {
                curve: *c,
                value: sk
                    .measure(&Constraint::ArcLength {
                        curve: *c,
                        value: 0.0,
                    })
                    .ok_or_else(|| bad("the arc has no length"))?,
            },
            Some(Curve::Circle { radius, .. }) => Constraint::Diameter {
                curve: *c,
                value: 2.0 * radius,
            },
            Some(Curve::Arc { .. }) => Constraint::Radius {
                curve: *c,
                value: sk.radius(*c).ok_or_else(|| bad("the arc has no radius"))?,
            },
            Some(Curve::Spline { .. }) => {
                return Err(bad(
                    "a spline has no dimension of its own; dimension its points",
                ));
            }
            Some(Curve::Ellipse { .. } | Curve::EllipticalArc { .. }) => {
                return Err(bad(
                    "dimension an ellipse by its points: select its centre and an axis end",
                ));
            }
            None => return Err(bad("the selected curve is gone")),
        },
        ([a, b], []) => {
            let (pa, pb) = (pos(sk, *a)?, pos(sk, *b)?);
            point_dimension(*a, *b, pa, pb, along(pa, pb, at, asked))
        }
        ([point], [line]) => {
            let (s0, s1) = line_ends(sk, *line).ok_or_else(|| bad("select a point and a line"))?;
            let (q, a, b) = (pos(sk, *point)?, pos(sk, s0)?, pos(sk, s1)?);
            let dir = (b - a)
                .normalized()
                .ok_or_else(|| bad("the line has no length"))?;
            Constraint::PointLineDistance {
                point: *point,
                line: *line,
                value: dir.cross(q - a).abs(),
            }
        }
        ([], [l1, l2]) => {
            let (a0, a1) = line_ends(sk, *l1).ok_or_else(|| bad("select two lines"))?;
            let (b0, b1) = line_ends(sk, *l2).ok_or_else(|| bad("select two lines"))?;
            let u = pos(sk, a1)? - pos(sk, a0)?;
            let v = pos(sk, b1)? - pos(sk, b0)?;
            Constraint::Angle {
                a: *l1,
                b: *l2,
                value: u.cross(v).atan2(u.dot(v)).abs(),
            }
        }
        _ => {
            return Err(bad(
                "select one line, circle or arc; two points; a point and a line; or two lines",
            ));
        }
    };
    let value = c.value();
    let kind = c.kind_name();
    let backup = sk.clone();
    let id = sk.add_constraint(c).map_err(sketch_err)?;
    let mut driven = want_driven;
    if driven {
        sk.set_driven(id, true).map_err(sketch_err)?;
    }
    match sk.solve() {
        // Measured at the size it has, a dimension that over-constrains the sketch holds
        // but is implied by the others: it becomes a driven dimension instead.
        Ok(report) if !driven && report.dof.redundant.contains(&id) => {
            sk.set_driven(id, true).map_err(sketch_err)?;
            driven = true;
        }
        Ok(_) => {}
        Err(e) => {
            *sk = backup;
            return Err(sketch_err(e));
        }
    }
    s.sketch_selection.clear();
    Ok(json!({ "constraint": id.0, "kind": kind, "value": value, "driven": driven }))
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    if p.get("constraint").is_some_and(|v| !v.is_null()) {
        let id = chamfera_sketch::ConstraintId(params::req_id(p, "constraint")?);
        let sk = active(s)?;
        sk.remove_constraint(id).map_err(sketch_err)?;
        solve(sk)?;
        prune_expressions(s)?;
        return Ok(json!({ "removed_constraint": id.0 }));
    }
    let (points, curves) = selection(s);
    if points.is_empty() && curves.is_empty() {
        return Err(bad("nothing is selected"));
    }
    let sk = active(s)?;
    for c in &curves {
        if sk.curve(*c).is_some() {
            sk.remove_curve(*c).map_err(sketch_err)?;
        }
    }
    for pt in &points {
        if sk.point(*pt).is_some() {
            sk.remove_point(*pt).map_err(sketch_err)?;
        }
    }
    solve(sk)?;
    prune_expressions(s)?;
    s.sketch_selection.clear();
    Ok(json!({ "removed_points": points.len(), "removed_curves": curves.len() }))
}

fn pick(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    active_id(s)?;
    Ok(json!({ "item": s.pick_sketch_item(x, y, w, h) }))
}

fn plane_point(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    active_id(s)?;
    let at = s
        .sketch_plane_point(x, y, w, h)
        .ok_or_else(|| CommandError::Unavailable("the sketch plane is seen edge-on".into()))?;
    Ok(json!({ "x": at.x, "y": at.y }))
}

fn select(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    let add = match p.get("add") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "add")?,
    };
    let filter = crate::pick_filter::PickFilter::from_params(p)?.unwrap_or_default();
    active_id(s)?;
    let item = s.pick_sketch_item_filtered(x, y, w, h, &filter);
    if !add {
        s.sketch_selection.clear();
    }
    if let Some(item) = item {
        if let Some(i) = s.sketch_selection.iter().position(|x| *x == item) {
            s.sketch_selection.remove(i);
        } else {
            s.sketch_selection.push(item);
        }
    }
    Ok(json!({ "item": item, "selection": s.sketch_selection }))
}

fn select_clear(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    s.sketch_selection.clear();
    Ok(json!({ "selection": [] }))
}

fn get(s: &mut Session, p: &Value) -> CmdResult {
    let id = match p.get("sketch") {
        None | Some(Value::Null) => active_id(s)?,
        Some(_) => params::req_id(p, "sketch")?,
    };
    let f = s.doc.sketch(id)?;
    let dof = f.sketch.analyze().ok();
    let regions: Vec<Value> = f
        .sketch
        .profiles()
        .iter()
        .map(|r| json!({ "area": r.area, "holes": r.holes.len() }))
        .collect();
    let points: Vec<Value> = f
        .sketch
        .points()
        .map(|(id, p)| json!({ "point": id.0, "x": p.x, "y": p.y }))
        .collect();
    let curves: Vec<Value> = f
        .sketch
        .curves()
        .map(|(id, c)| json!({ "curve": id.0, "geometry": c }))
        .collect();
    let constraints: Vec<Value> = f
        .sketch
        .constraints()
        .map(|(id, c)| {
            if f.sketch.is_driven(id) {
                json!({ "constraint": id.0, "definition": c, "driven": true })
            } else {
                json!({ "constraint": id.0, "definition": c })
            }
        })
        .collect();
    Ok(json!({
        "sketch": id,
        "name": f.name,
        "active": s.active_sketch == Some(id),
        "points": points,
        "curves": curves,
        "constraints": constraints,
        "dof": dof.as_ref().map(|d| d.dof),
        "status": dof.as_ref().map(|d| format!("{:?}", d.status)),
        "profiles": regions,
        // Curves cross without a shared point. The profiles above already split them
        // there; `sketch.split` makes the pieces real curves (to trim or constrain them).
        "needs_split": f.sketch.has_unsplit_crossings(),
        "expressions": f.expressions,
        "selection": s.sketch_selection,
    }))
}

// ----- sketch tools (`chamfera_sketch::ToolOp`) -----

/// Forgets the expressions of dimensions and the projections of curves that are gone, so
/// the design still loads.
fn prune_expressions(s: &mut Session) -> Result<(), CommandError> {
    let id = active_id(s)?;
    let f = s.doc.sketch_mut(id)?;
    let sketch = &f.sketch;
    f.expressions.retain(|c, _| {
        sketch
            .constraint(chamfera_sketch::ConstraintId(*c))
            .is_some()
    });
    f.projections
        .retain(|p| sketch.curve(CurveId(p.curve)).is_some());
    Ok(())
}

/// Runs a tool on the active sketch and reports what it made, changed and removed.
fn run_tool(s: &mut Session, op: ToolOp) -> CmdResult {
    let sk = active(s)?;
    let r = sk.apply_tool(&op).map_err(sketch_err)?;
    prune_expressions(s)?;
    if let Some(f) = s.active_sketch() {
        let keep: Vec<SketchItem> = s
            .sketch_selection
            .iter()
            .copied()
            .filter(|i| match i {
                SketchItem::Point(p) => f.sketch.point(PointId(*p)).is_some(),
                SketchItem::Curve(c) => f.sketch.curve(CurveId(*c)).is_some(),
            })
            .collect();
        s.sketch_selection = keep;
    }
    let curves = |v: &[CurveId]| v.iter().map(|c| c.0).collect::<Vec<_>>();
    let points = |v: &[PointId]| v.iter().map(|c| c.0).collect::<Vec<_>>();
    Ok(json!({
        "created_curves": curves(&r.created_curves),
        "changed_curves": curves(&r.changed_curves),
        "removed_curves": curves(&r.removed_curves),
        "created_points": points(&r.created_points),
        "removed_points": points(&r.removed_points),
        "created_constraints": r.created_constraints.iter().map(|c| c.0).collect::<Vec<_>>(),
        "removed_constraints": r.removed_constraints.iter().map(|c| c.0).collect::<Vec<_>>(),
    }))
}

/// A list of ids, or `None` when the key is missing.
fn opt_ids(p: &Value, key: &str) -> Result<Option<Vec<u32>>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > chamfera_sketch::MAX_CURVES + chamfera_sketch::MAX_POINTS {
                return Err(bad(format!("`{key}` is too long")));
            }
            items
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| bad(format!("`{key}` must list whole number ids")))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        Some(_) => Err(bad(format!("`{key}` must be a list of ids"))),
    }
}

/// The curves and points to act on: the lists given, or else the selection.
fn targets(s: &Session, p: &Value) -> Result<(Vec<CurveId>, Vec<PointId>), CommandError> {
    let curves = opt_ids(p, "curves")?;
    let points = opt_ids(p, "points")?;
    if curves.is_none() && points.is_none() {
        let (points, curves) = selection(s);
        return Ok((curves, points));
    }
    Ok((
        curves
            .unwrap_or_default()
            .into_iter()
            .map(CurveId)
            .collect(),
        points
            .unwrap_or_default()
            .into_iter()
            .map(PointId)
            .collect(),
    ))
}

/// A whole number from 0 to `max`.
fn count(p: &Value, key: &str, max: u32) -> Result<u32, CommandError> {
    let v = params::req_f64(p, key)?;
    if v.fract() != 0.0 || !(0.0..=f64::from(max)).contains(&v) {
        return Err(bad(format!(
            "`{key}` must be a whole number from 0 to {max}"
        )));
    }
    Ok(v as u32)
}

fn trim(s: &mut Session, p: &Value) -> CmdResult {
    let curve = CurveId(params::req_id(p, "curve")?);
    let at = pt(p, "x", "y")?;
    run_tool(s, ToolOp::Trim { curve, at })
}

fn extend(s: &mut Session, p: &Value) -> CmdResult {
    let curve = CurveId(params::req_id(p, "curve")?);
    let at = pt(p, "x", "y")?;
    run_tool(s, ToolOp::Extend { curve, at })
}

fn split(s: &mut Session, p: &Value) -> CmdResult {
    let curve = match p.get("curve") {
        None | Some(Value::Null) => None,
        Some(_) => Some(CurveId(params::req_id(p, "curve")?)),
    };
    run_tool(s, ToolOp::Split { curve })
}

fn fillet(s: &mut Session, p: &Value) -> CmdResult {
    let corner = PointId(params::req_id(p, "point")?);
    let radius = params::req_f64(p, "radius")?;
    run_tool(s, ToolOp::Fillet { corner, radius })
}

fn chamfer(s: &mut Session, p: &Value) -> CmdResult {
    let corner = PointId(params::req_id(p, "point")?);
    let distance = params::req_f64(p, "distance")?;
    let distance2 = params::opt_f64(p, "distance2")?;
    run_tool(
        s,
        ToolOp::Chamfer {
            corner,
            distance,
            distance2,
        },
    )
}

fn offset(s: &mut Session, p: &Value) -> CmdResult {
    let (curves, _) = targets(s, p)?;
    let distance = params::req_f64(p, "distance")?;
    let side = match (p.get("x"), p.get("y")) {
        (None | Some(Value::Null), None | Some(Value::Null)) => None,
        _ => Some(pt(p, "x", "y")?),
    };
    let corners = match params::opt_str(p, "corners")? {
        None | Some("sharp") => OffsetCorners::Sharp,
        Some("round") => OffsetCorners::Round,
        Some(other) => {
            return Err(bad(format!(
                "unknown corners {other:?}: use sharp or round"
            )));
        }
    };
    let dimension = match p.get("dimension") {
        None | Some(Value::Null) => true,
        Some(_) => params::req_bool(p, "dimension")?,
    };
    run_tool(
        s,
        ToolOp::Offset {
            curves,
            distance,
            side,
            corners,
            dimension,
        },
    )
}

fn mirror(s: &mut Session, p: &Value) -> CmdResult {
    let axis = CurveId(params::req_id(p, "axis")?);
    let (curves, points) = targets(s, p)?;
    run_tool(
        s,
        ToolOp::Mirror {
            curves,
            points,
            axis,
        },
    )
}

fn pattern_rectangular(s: &mut Session, p: &Value) -> CmdResult {
    let (curves, points) = targets(s, p)?;
    let limit = chamfera_sketch::MAX_PATTERN_COPIES as u32 + 1;
    let columns = count(p, "columns", limit)?;
    let dx = params::req_f64(p, "dx")?;
    let rows = match p.get("rows") {
        None | Some(Value::Null) => 1,
        Some(_) => count(p, "rows", limit)?,
    };
    let dy = params::opt_f64(p, "dy")?.unwrap_or(0.0);
    run_tool(
        s,
        ToolOp::RectangularPattern {
            curves,
            points,
            columns,
            rows,
            dx,
            dy,
        },
    )
}

fn pattern_circular(s: &mut Session, p: &Value) -> CmdResult {
    let (curves, points) = targets(s, p)?;
    let centre = pt(p, "cx", "cy")?;
    let count = count(p, "count", chamfera_sketch::MAX_PATTERN_COPIES as u32 + 1)?;
    let angle = params::opt_f64(p, "angle")?.unwrap_or(std::f64::consts::TAU);
    run_tool(
        s,
        ToolOp::CircularPattern {
            curves,
            points,
            centre,
            count,
            angle,
        },
    )
}

fn rectangle_centre(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let corner = pt(p, "x", "y")?;
    run_tool(s, ToolOp::CentreRectangle { centre, corner })
}

fn arc_three_point(s: &mut Session, p: &Value) -> CmdResult {
    let start = pt(p, "x1", "y1")?;
    let through = pt(p, "x2", "y2")?;
    let end = pt(p, "x3", "y3")?;
    run_tool(
        s,
        ToolOp::ThreePointArc {
            start,
            through,
            end,
        },
    )
}

fn rectangle_three_point(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    let c = pt(p, "x3", "y3")?;
    run_tool(s, ToolOp::ThreePointRectangle { a, b, c })
}

fn circle_two_point(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    run_tool(s, ToolOp::TwoPointCircle { a, b })
}

fn circle_three_point(s: &mut Session, p: &Value) -> CmdResult {
    let a = pt(p, "x1", "y1")?;
    let b = pt(p, "x2", "y2")?;
    let c = pt(p, "x3", "y3")?;
    run_tool(s, ToolOp::ThreePointCircle { a, b, c })
}

fn circle_tangent(s: &mut Session, p: &Value) -> CmdResult {
    let curves: Vec<CurveId> = opt_ids(p, "curves")?
        .ok_or_else(|| bad("`curves` is required: two or three curve ids"))?
        .into_iter()
        .map(CurveId)
        .collect();
    let near = pt(p, "x", "y")?;
    let radius = params::opt_f64(p, "radius")?;
    run_tool(
        s,
        ToolOp::TangentCircle {
            curves,
            near,
            radius,
        },
    )
}

fn opt_copy(p: &Value) -> Result<bool, CommandError> {
    match p.get("copy") {
        None | Some(Value::Null) => Ok(false),
        Some(_) => params::req_bool(p, "copy"),
    }
}

fn transform_tool(
    s: &mut Session,
    p: &Value,
    transform: chamfera_sketch::SketchTransform,
) -> CmdResult {
    let (curves, points) = targets(s, p)?;
    let copy = opt_copy(p)?;
    run_tool(
        s,
        ToolOp::Transform {
            curves,
            points,
            transform,
            copy,
        },
    )
}

fn move_geometry(s: &mut Session, p: &Value) -> CmdResult {
    let dx = params::req_f64(p, "dx")?;
    let dy = params::req_f64(p, "dy")?;
    transform_tool(s, p, chamfera_sketch::SketchTransform::Translate { dx, dy })
}

fn rotate_geometry(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let angle = params::req_f64(p, "angle")?;
    transform_tool(
        s,
        p,
        chamfera_sketch::SketchTransform::Rotate { centre, angle },
    )
}

fn scale_geometry(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let factor = params::req_f64(p, "factor")?;
    transform_tool(
        s,
        p,
        chamfera_sketch::SketchTransform::Scale { centre, factor },
    )
}

fn arc_tangent(s: &mut Session, p: &Value) -> CmdResult {
    let from = PointId(params::req_id(p, "point")?);
    let to = pt(p, "x", "y")?;
    run_tool(s, ToolOp::TangentArc { from, to })
}

// ----- shapes and projection -----

fn polygon(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let corner = pt(p, "x", "y")?;
    let sides = count(p, "sides", chamfera_sketch::MAX_POLYGON_SIDES)?;
    let inscribed = match params::opt_str(p, "mode")? {
        None | Some("inscribed") => true,
        Some("circumscribed") => false,
        Some(other) => {
            return Err(bad(format!(
                "unknown mode {other:?}: use inscribed or circumscribed"
            )));
        }
    };
    run_tool(
        s,
        ToolOp::Polygon {
            centre,
            corner,
            sides,
            inscribed,
        },
    )
}

fn slot(s: &mut Session, p: &Value) -> CmdResult {
    let start = pt(p, "x1", "y1")?;
    let end = pt(p, "x2", "y2")?;
    let width = params::req_f64(p, "width")?;
    let overall = match p.get("overall") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "overall")?,
    };
    run_tool(
        s,
        ToolOp::Slot {
            start,
            end,
            width,
            overall,
        },
    )
}

fn ellipse(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let major = pt(p, "x", "y")?;
    let minor = params::req_f64(p, "minor")?;
    run_tool(
        s,
        ToolOp::Ellipse {
            centre,
            major,
            minor,
        },
    )
}

fn ellipse_arc(s: &mut Session, p: &Value) -> CmdResult {
    let centre = pt(p, "cx", "cy")?;
    let major = pt(p, "x", "y")?;
    let minor = params::req_f64(p, "minor")?;
    let start = pt(p, "x1", "y1")?;
    let end = pt(p, "x2", "y2")?;
    run_tool(
        s,
        ToolOp::EllipticalArc {
            centre,
            major,
            minor,
            start,
            end,
        },
    )
}

/// Most edges and faces one projection takes.
const MAX_PROJECT_ITEMS: usize = 10_000;

/// The edge names a projection asks for: listed edges, every edge of listed faces, or the
/// edge (else the face) under a pixel.
fn project_targets(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    let mut names: Vec<String> = Vec::new();
    match p.get("edges") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) if list.len() <= MAX_PROJECT_ITEMS => {
            for e in list {
                let name = e
                    .as_str()
                    .ok_or_else(|| bad("`edges` must be a list of edge names"))?;
                if name.chars().count() > chamfera_doc::MAX_EDGE_REF_LEN {
                    return Err(bad("an edge name is too long"));
                }
                names.push(name.to_owned());
            }
        }
        Some(_) => return Err(bad("`edges` must be a list of edge names")),
    }
    match p.get("faces") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) if list.len() <= MAX_PROJECT_ITEMS => {
            for f in list {
                let feature = params::req_id(f, "feature")?;
                let face = params::req_str(f, "face")?;
                names.extend(s.face_edge_refs(feature, face).map_err(bad)?);
            }
        }
        Some(_) => return Err(bad("`faces` must be a list of {feature, face}")),
    }
    match p.get("pick") {
        None | Some(Value::Null) => {}
        Some(pick) => {
            let (x, y) = (params::req_f64(pick, "x")?, params::req_f64(pick, "y")?);
            let (w, h) = (
                params::req_f64(pick, "width")?,
                params::req_f64(pick, "height")?,
            );
            if let Some((_, _, name)) = s.edge_at(x, y, w, h) {
                names.push(name);
            } else if let Some((feature, face, _)) = s.face_at(x, y, w, h) {
                names.extend(s.face_edge_refs(feature, &face).map_err(bad)?);
            } else {
                return Err(CommandError::Unavailable(
                    "there is no model edge or face under that pixel".into(),
                ));
            }
        }
    }
    if names.is_empty() {
        return Err(bad("give `edges`, `faces` or `pick`"));
    }
    let mut seen = std::collections::BTreeSet::new();
    names.retain(|n| seen.insert(n.clone()));
    Ok(names)
}

fn project(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = active_id(s)?;
    let linked = match p.get("linked") {
        None | Some(Value::Null) => true,
        Some(_) => params::req_bool(p, "linked")?,
    };
    let names = project_targets(s, p)?;
    let made = s
        .project_edges(sketch, &names, linked)
        .map_err(CommandError::Unavailable)?;
    let projected: Vec<Value> = made
        .iter()
        .map(|(edge, curve)| json!({ "edge": edge, "curve": curve }))
        .collect();
    Ok(json!({
        "projected": projected,
        "curves": made.iter().map(|m| m.1).collect::<Vec<_>>(),
        "linked": linked,
    }))
}

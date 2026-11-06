//! `sketch.*`: create sketches on a plane, draw, constrain, dimension and drag, and the
//! sketch tools (trim, extend, split, fillet, chamfer, offset, mirror, patterns).
//!
//! Drawing commands act on the active sketch (`sketch.create` or `sketch.edit` makes one
//! active, `sketch.finish` ends editing). Coordinates are sketch-plane millimetres.
//! Commands that change a sketch solve it, and a failed solve changes nothing.

use super::CommandSpec;
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
        id: "sketch.constrain",
        label: "Add Constraint",
        params: "constraint: object (required) - {\"kind\": ..., ids and value}, for example \
                 {\"kind\": \"horizontal\", \"line\": 3} or {\"kind\": \"length\", \"line\": 3, \"value\": 50}",
        mutates: true,
        run: constrain,
    },
    CommandSpec {
        id: "sketch.constrain_selection",
        label: "Constrain Selection",
        params: "kind: string (required) - horizontal, vertical, coincident, point_on, parallel, \
                 perpendicular, tangent, equal, midpoint or fix",
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
                 id of a construction plane (the sketch follows it when the design rebuilds)",
        mutates: true,
        run: create,
    },
    CommandSpec {
        id: "sketch.create_on_face",
        label: "Create Sketch on Face",
        params: "x, y, width, height: number - the pixel of a flat face of a solid; or\n\
                 feature: integer and face: string - a face by its name, such as \"top\"",
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
        id: "sketch.dimension_selection",
        label: "Sketch Dimension",
        params: "(none) - dimensions the selection at its current size: one line (length), \
                 a circle (diameter), an arc (radius), two points (distance), a point and a line \
                 (distance) or two lines (angle)",
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
    let id = s.doc.add_sketch(plane)?;
    s.active_sketch = Some(id);
    s.sketch_selection.clear();
    Ok(json!({ "sketch": id }))
}

fn create_on_face(s: &mut Session, p: &Value) -> CmdResult {
    let (feature, face, plane) = if p.get("feature").is_some_and(|v| !v.is_null()) {
        let feature = params::req_id(p, "feature")?;
        let face = params::req_str(p, "face")?.to_owned();
        // The face may be in any body: joins move a feature's faces into another body.
        let key = crate::solids::face_key(feature, &face);
        let found = s.solids().iter().find_map(|x| {
            x.body
                .faces()
                .into_iter()
                .find(|f| x.body.face_name(*f).ok().flatten() == Some(key.as_str()))
                .map(|f| (x, f))
        });
        let (solid, id) =
            found.ok_or_else(|| bad(format!("feature {feature} has no face named {face:?}")))?;
        let plane = crate::solids::face_plane(&solid.body, id)
            .ok_or_else(|| bad("sketches go on flat faces"))?;
        (feature, face, plane)
    } else {
        let (x, y, w, h) = params::pixel(p)?;
        s.face_at(x, y, w, h)
            .ok_or_else(|| bad("no flat face of a solid is under that point"))?
    };
    let id = s.doc.add_sketch(plane)?;
    if let Ok(f) = s.doc.sketch_mut(id) {
        f.attached = Some(chamfera_doc::FaceRef {
            feature,
            face: face.clone(),
        });
    }
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

fn constrain(s: &mut Session, p: &Value) -> CmdResult {
    let raw = p
        .get("constraint")
        .ok_or_else(|| bad("`constraint` is required"))?;
    let c: Constraint =
        serde_json::from_value(raw.clone()).map_err(|e| bad(format!("`constraint`: {e}")))?;
    let sk = active(s)?;
    let id = sk.try_add_constraint(c).map_err(sketch_err)?;
    Ok(json!({ "constraint": id.0 }))
}

fn dimension(s: &mut Session, p: &Value) -> CmdResult {
    let id = chamfera_sketch::ConstraintId(params::req_id(p, "constraint")?);
    let sketch_id = active_id(s)?;
    let expression = params::opt_str(p, "expression")?.map(str::to_owned);
    let f = s.doc.sketch_mut(sketch_id)?;
    if f.sketch.constraint(id).is_none_or(|c| !c.is_dimension()) {
        return Err(bad(format!("{id} is not a dimension")));
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
        "horizontal" | "vertical" => {
            need(!curves.is_empty() && points.is_empty(), "one or more lines")?;
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

fn dimension_selection(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
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
            Some(Curve::Line { start, end, .. }) => Constraint::Length {
                line: *c,
                value: pos(sk, *start)?.distance(pos(sk, *end)?),
            },
            Some(Curve::Circle { radius, .. }) => Constraint::Diameter {
                curve: *c,
                value: 2.0 * radius,
            },
            Some(Curve::Arc { .. }) => Constraint::Radius {
                curve: *c,
                value: sk.radius(*c).ok_or_else(|| bad("the arc has no radius"))?,
            },
            None => return Err(bad("the selected curve is gone")),
        },
        ([a, b], []) => Constraint::Distance {
            a: *a,
            b: *b,
            value: pos(sk, *a)?.distance(pos(sk, *b)?),
        },
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
    let id = sk.try_add_constraint(c).map_err(sketch_err)?;
    s.sketch_selection.clear();
    Ok(json!({ "constraint": id.0, "kind": kind, "value": value }))
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

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "sketch")?;
    let name = params::req_display_name(p, "name")?;
    s.doc.sketch_mut(id)?.name = name.clone();
    Ok(json!({ "sketch": id, "name": name }))
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
        .map(|(id, c)| json!({ "constraint": id.0, "definition": c }))
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
        "expressions": f.expressions,
        "selection": s.sketch_selection,
    }))
}

// ----- sketch tools (`chamfera_sketch::ToolOp`) -----

/// Forgets the expressions of dimensions that are gone, so the design still loads.
fn prune_expressions(s: &mut Session) -> Result<(), CommandError> {
    let id = active_id(s)?;
    let f = s.doc.sketch_mut(id)?;
    let sketch = &f.sketch;
    f.expressions.retain(|c, _| {
        sketch
            .constraint(chamfera_sketch::ConstraintId(*c))
            .is_some()
    });
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

fn arc_tangent(s: &mut Session, p: &Value) -> CmdResult {
    let from = PointId(params::req_id(p, "point")?);
    let to = pt(p, "x", "y")?;
    run_tool(s, ToolOp::TangentArc { from, to })
}

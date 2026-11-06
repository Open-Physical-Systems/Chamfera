//! `construct.*`: construction planes, axes and points on the timeline.
//!
//! References in the parameters:
//! - a plane: `"xy"`, `"xz"` or `"yz"`; `{"feature": 2, "face": "top"}` (a flat face); or a
//!   construction plane's feature id;
//! - an axis: `"x"`, `"y"` or `"z"`; `{"faces": [face, face]}` (the straight edge where two
//!   faces meet); or a construction axis's id;
//! - a point: `"origin"`; `{"x", "y", "z"}` (numbers in mm or expressions);
//!   `{"vertex": [face, face, ...]}` (where those faces meet); `{"centre": [face, face]}`
//!   (the centre of the circular edge between two faces); or a construction point's id.
//!
//! `construct.pick` names the face, edge and vertex under a pixel in these forms.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{
    AxisRef, Construction, EdgeRef, FaceRef, FeatureKind, OriginAxis, OriginPlane, PlaneRef,
    PointRef,
};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "construct.axis_cylinder",
        label: "Axis Through Cylinder",
        params: "face: {feature, face} (required) - a cylindrical or conical face",
        mutates: true,
        run: axis_cylinder,
    },
    CommandSpec {
        id: "construct.axis_edge",
        label: "Axis Along Edge",
        params: "edge: {faces: [face, face]} (required) - the straight edge two faces share",
        mutates: true,
        run: axis_edge,
    },
    CommandSpec {
        id: "construct.axis_two_planes",
        label: "Axis Through Two Planes",
        params: "planes: [plane, plane] (required) - two planes that are not parallel",
        mutates: true,
        run: axis_two_planes,
    },
    CommandSpec {
        id: "construct.axis_two_points",
        label: "Axis Through Two Points",
        params: "points: [point, point] (required) - from the first to the second",
        mutates: true,
        run: axis_two_points,
    },
    CommandSpec {
        id: "construct.pick",
        label: "Pick Face, Edge or Vertex",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: pick,
    },
    CommandSpec {
        id: "construct.plane_angle",
        label: "Plane at Angle",
        params: "axis: axis (required) - the line the plane turns about\n\
                 angle: string or number (required) - such as 30 or \"tilt * 2\" (plain = degrees)\n\
                 base: plane (optional) - the plane at 0 degrees (default: XY, else XZ)",
        mutates: true,
        run: plane_angle,
    },
    CommandSpec {
        id: "construct.plane_midplane",
        label: "Midplane",
        params: "planes: [plane, plane] (required) - two parallel planes or flat faces",
        mutates: true,
        run: plane_midplane,
    },
    CommandSpec {
        id: "construct.plane_offset",
        label: "Offset Plane",
        params: "base: plane (required) - an origin plane, a flat face or a construction plane\n\
                 distance: string or number (required) - along the base's normal, such as 15 or \"gap\"",
        mutates: true,
        run: plane_offset,
    },
    CommandSpec {
        id: "construct.plane_tangent",
        label: "Tangent Plane",
        params: "face: {feature, face} (required) - a cylindrical face\n\
                 point: point (required) - the plane touches the face on the line nearest it",
        mutates: true,
        run: plane_tangent,
    },
    CommandSpec {
        id: "construct.plane_three_points",
        label: "Plane Through Three Points",
        params: "points: [point, point, point] (required) - not on one line; normal (p1-p0)x(p2-p0)",
        mutates: true,
        run: plane_three_points,
    },
    CommandSpec {
        id: "construct.point_centre",
        label: "Point at Centre",
        params: "edge: {faces: [face, face]} (required) - a circular edge two faces share",
        mutates: true,
        run: point_centre,
    },
    CommandSpec {
        id: "construct.point_coordinates",
        label: "Point at Coordinates",
        params: "x, y, z: string or number (required) - mm, or expressions",
        mutates: true,
        run: point_coordinates,
    },
    CommandSpec {
        id: "construct.point_vertex",
        label: "Point at Vertex",
        params: "faces: [face, ...] (required) - 2 to 8 faces that meet at the vertex",
        mutates: true,
        run: point_vertex,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn field<'a>(p: &'a Value, key: &str) -> Result<&'a Value, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Err(bad(format!("`{key}` is required"))),
        Some(v) => Ok(v),
    }
}

/// A feature id given as a bare JSON number.
fn id_value(v: &Value, what: &str) -> Result<u32, CommandError> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| bad(format!("{what} must be a whole number feature id")))
}

fn face_ref(v: &Value) -> Result<FaceRef, CommandError> {
    let feature = params::req_id(v, "feature")?;
    let face = params::req_str(v, "face")?;
    if face.is_empty() || face.chars().count() > chamfera_doc::construct::MAX_FACE_NAME {
        return Err(bad("a face name must have 1 to 256 characters"));
    }
    Ok(FaceRef {
        feature,
        face: face.to_owned(),
    })
}

fn face_list(v: &Value, min: usize, max: usize) -> Result<Vec<FaceRef>, CommandError> {
    let list = v
        .as_array()
        .filter(|l| (min..=max).contains(&l.len()))
        .ok_or_else(|| bad(format!("give {min} to {max} faces as {{feature, face}}")))?;
    list.iter().map(face_ref).collect()
}

fn edge_ref(v: &Value) -> Result<EdgeRef, CommandError> {
    let faces = face_list(field(v, "faces")?, 2, 2)?;
    match <[FaceRef; 2]>::try_from(faces) {
        Ok(faces) => Ok(EdgeRef { faces }),
        Err(_) => Err(bad("an edge is named by two faces")),
    }
}

fn plane_ref(v: &Value) -> Result<PlaneRef, CommandError> {
    match v {
        Value::String(s) => OriginPlane::parse(s)
            .map(|plane| PlaneRef::Origin { plane })
            .ok_or_else(|| bad(format!("unknown plane {s:?}: use xy, xz or yz"))),
        Value::Number(_) => Ok(PlaneRef::Construct {
            feature: id_value(v, "a plane")?,
        }),
        Value::Object(_) => Ok(PlaneRef::Face(face_ref(v)?)),
        _ => Err(bad(
            "a plane is \"xy\", \"xz\", \"yz\", {feature, face} or a construction plane id",
        )),
    }
}

fn axis_ref(v: &Value) -> Result<AxisRef, CommandError> {
    match v {
        Value::String(s) => OriginAxis::parse(s)
            .map(|axis| AxisRef::Origin { axis })
            .ok_or_else(|| bad(format!("unknown axis {s:?}: use x, y or z"))),
        Value::Number(_) => Ok(AxisRef::Construct {
            feature: id_value(v, "an axis")?,
        }),
        Value::Object(_) => Ok(AxisRef::Edge(edge_ref(v)?)),
        _ => Err(bad(
            "an axis is \"x\", \"y\", \"z\", {faces: [face, face]} or a construction axis id",
        )),
    }
}

/// A length given as a number (mm) or an expression.
fn length(v: &Value, key: &str) -> Result<String, CommandError> {
    match v.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(_)) => Ok(format!("{} mm", params::req_f64(v, key)?)),
        None | Some(Value::Null) => Err(bad(format!("`{key}` is required"))),
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

fn point_ref(v: &Value) -> Result<PointRef, CommandError> {
    match v {
        Value::String(s) if s == "origin" => Ok(PointRef::Origin),
        Value::Number(_) => Ok(PointRef::Construct {
            feature: id_value(v, "a point")?,
        }),
        Value::Object(o) if o.contains_key("vertex") => Ok(PointRef::Vertex {
            faces: face_list(
                field(v, "vertex")?,
                2,
                chamfera_doc::construct::MAX_VERTEX_FACES,
            )?,
        }),
        Value::Object(o) if o.contains_key("centre") => {
            let faces = face_list(field(v, "centre")?, 2, 2)?;
            match <[FaceRef; 2]>::try_from(faces) {
                Ok(faces) => Ok(PointRef::Centre {
                    edge: EdgeRef { faces },
                }),
                Err(_) => Err(bad("an edge is named by two faces")),
            }
        }
        Value::Object(_) => Ok(PointRef::Coordinates {
            x: length(v, "x")?,
            y: length(v, "y")?,
            z: length(v, "z")?,
        }),
        _ => Err(bad(
            "a point is \"origin\", {x, y, z}, {vertex: [faces]}, {centre: [face, face]} or a construction point id",
        )),
    }
}

fn list<'a>(p: &'a Value, key: &str, n: usize) -> Result<&'a [Value], CommandError> {
    field(p, key)?
        .as_array()
        .filter(|l| l.len() == n)
        .map(Vec::as_slice)
        .ok_or_else(|| bad(format!("`{key}` must be a list of {n}")))
}

/// The reply for a construction: what it built, and its error if it failed.
pub(crate) fn construct_reply(s: &Session, id: u32) -> Value {
    use crate::construct::Constructed;
    let f = s.doc.feature(id).ok();
    let v = |x: chamfera_geom::Vec3| json!({ "x": x.x, "y": x.y, "z": x.z });
    let geometry = match s.constructs().get(&id) {
        Some(Constructed::Plane { frame, .. }) => json!({
            "kind": "plane",
            "origin": frame.origin(),
            "normal": v(frame.z()),
            "x_axis": v(frame.x()),
        }),
        Some(Constructed::Axis {
            origin, direction, ..
        }) => json!({ "kind": "axis", "origin": origin, "direction": v(*direction) }),
        Some(Constructed::Point(p)) => json!({ "kind": "point", "point": p }),
        None => Value::Null,
    };
    json!({
        "feature": id,
        "name": f.map(|f| f.name.clone()),
        "definition": f.map(|f| &f.kind),
        "suppressed": f.map(|f| f.suppressed),
        "geometry": geometry,
        "error": s.feature_errors().get(&id),
    })
}

fn add(s: &mut Session, c: Construction) -> CmdResult {
    let id = s.doc.add_feature(FeatureKind::Construct(c))?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(construct_reply(s, id))
}

fn plane_offset(s: &mut Session, p: &Value) -> CmdResult {
    let base = plane_ref(field(p, "base")?)?;
    let distance = length(p, "distance")?;
    add(s, Construction::PlaneOffset { base, distance })
}

fn plane_angle(s: &mut Session, p: &Value) -> CmdResult {
    let axis = axis_ref(field(p, "axis")?)?;
    let angle = match p.get("angle") {
        Some(Value::String(a)) => a.clone(),
        Some(Value::Number(_)) => format!("{} deg", params::req_f64(p, "angle")?),
        None | Some(Value::Null) => return Err(bad("`angle` is required")),
        Some(_) => return Err(bad("`angle` must be a number or an expression")),
    };
    let base = match p.get("base") {
        None | Some(Value::Null) => None,
        Some(b) => Some(plane_ref(b)?),
    };
    add(s, Construction::PlaneAngle { axis, angle, base })
}

fn plane_midplane(s: &mut Session, p: &Value) -> CmdResult {
    let l = list(p, "planes", 2)?;
    let planes = [plane_ref(&l[0])?, plane_ref(&l[1])?];
    add(s, Construction::PlaneMidplane { planes })
}

fn plane_three_points(s: &mut Session, p: &Value) -> CmdResult {
    let l = list(p, "points", 3)?;
    let points = [point_ref(&l[0])?, point_ref(&l[1])?, point_ref(&l[2])?];
    add(s, Construction::PlaneThreePoints { points })
}

fn plane_tangent(s: &mut Session, p: &Value) -> CmdResult {
    let face = face_ref(field(p, "face")?)?;
    let point = point_ref(field(p, "point")?)?;
    add(s, Construction::PlaneTangent { face, point })
}

fn axis_cylinder(s: &mut Session, p: &Value) -> CmdResult {
    let face = face_ref(field(p, "face")?)?;
    add(s, Construction::AxisCylinder { face })
}

fn axis_edge(s: &mut Session, p: &Value) -> CmdResult {
    let edge = edge_ref(field(p, "edge")?)?;
    add(s, Construction::AxisEdge { edge })
}

fn axis_two_points(s: &mut Session, p: &Value) -> CmdResult {
    let l = list(p, "points", 2)?;
    let points = [point_ref(&l[0])?, point_ref(&l[1])?];
    add(s, Construction::AxisTwoPoints { points })
}

fn axis_two_planes(s: &mut Session, p: &Value) -> CmdResult {
    let l = list(p, "planes", 2)?;
    let planes = [plane_ref(&l[0])?, plane_ref(&l[1])?];
    add(s, Construction::AxisTwoPlanes { planes })
}

fn point_vertex(s: &mut Session, p: &Value) -> CmdResult {
    let faces = face_list(
        field(p, "faces")?,
        2,
        chamfera_doc::construct::MAX_VERTEX_FACES,
    )?;
    add(
        s,
        Construction::Point {
            at: PointRef::Vertex { faces },
        },
    )
}

fn point_centre(s: &mut Session, p: &Value) -> CmdResult {
    let edge = edge_ref(field(p, "edge")?)?;
    add(
        s,
        Construction::Point {
            at: PointRef::Centre { edge },
        },
    )
}

fn point_coordinates(s: &mut Session, p: &Value) -> CmdResult {
    let at = PointRef::Coordinates {
        x: length(p, "x")?,
        y: length(p, "y")?,
        z: length(p, "z")?,
    };
    add(s, Construction::Point { at })
}

fn pick(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    let Some(picked) = s.construct_pick(x, y, w, h) else {
        return Ok(json!({ "face": null }));
    };
    let face = |f: &FaceRef| json!({ "feature": f.feature, "face": f.face });
    Ok(json!({
        "face": face(&picked.face),
        "surface": picked.surface,
        "point": picked.point,
        "edge": picked.edge.as_ref().map(|(e, kind)| json!({
            "faces": e.faces.iter().map(face).collect::<Vec<_>>(),
            "curve": kind,
        })),
        "vertex": picked.vertex.as_ref().map(|(faces, at)| json!({
            "faces": faces.iter().map(face).collect::<Vec<_>>(),
            "point": at,
        })),
    }))
}

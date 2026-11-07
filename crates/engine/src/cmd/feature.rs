//! `solid.*` and `feature.*`: features on the timeline that build solids.

use super::CommandSpec;
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{
    Blend, BlendKind, Extrude, ExtrudeDirection, ExtrudeExtent, ExtrudeTo, FaceRef, FeatureKind,
    Loft, LoftSection, Operation, Revolve, Sweep,
};
use chamfera_geom::Point2;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "feature.delete",
        label: "Delete Feature",
        params: "feature: integer (required)",
        mutates: true,
        run: delete,
    },
    CommandSpec {
        id: "feature.edit",
        label: "Edit Feature",
        params: "feature: integer (required)\n\
                 distance: string or number (optional) - new extrude or offset plane distance\n\
                 angle: string or number (optional) - new angle of a plane at an angle\n\
                 direction: string (optional) - normal, reversed, symmetric or two_sides\n\
                 extent, distance2, to, start_offset, taper, thin: (optional) - extrude options as for solid.extrude (0 removes an offset or taper, thin: false a wall)\n\
                 operation: string (optional) - new_body, join, cut or intersect\n\
                 size: string or number (optional) - new fillet radius or chamfer distance\n\
                 size2, chamfer_angle (optional) - a chamfer's new second distance or angle (\"none\" removes it)\n\
                 reference_face: string, flip: boolean, tangent_chain: boolean (optional) - a blend's options\n\
                 path_curves: list of integers (optional) - new sweep path curves\n\
                 sections: list of {sketch, x, y} (optional) - new loft profiles, in order\n\
                 points, kind, diameter, depth, through_all, cb_diameter, cb_depth, cs_diameter,\n\
                 cs_angle, tip_angle (optional) - a hole's new values, as for solid.hole\n\
                 plane: string or object (optional) - a mirror's new plane, as in solid.mirror\n\
                 dx, dy, dz, angle, axis, origin, from, to (optional) - a move's new transform\n\
                 copy: boolean (optional) - whether a move keeps the originals\n\
                 thickness: string or number (optional) - new shell wall or thicken thickness\n\
                 height, angle, position, side, offset_start, offset_end, edge (optional) - a flange's new values, as for sheet.flange\n\
                 rule: string, reversed: boolean (optional) - a base flange's new rule and direction\n\
                 faces: list of strings (optional) - new open faces of a shell, faces of a face edit, thicken or split face\n\
                 distance, angle (optional) - a face edit's new distance (offset, move) and turn angle\n\
                 target, tools, keep_tools (optional) - a combine's bodies and keep flag\n\
                 bodies: list of strings (optional) - new bodies of a split body or remove\n\
                 plane, sketch, line (optional) - a split's new plane or sketch line\n\
                 sides: string (optional) - a thicken's one_side, symmetric or both_sides\n\
                 count: string or number (optional) - a pattern's new count\n\
                 spacing: string or number (optional) - a rectangular pattern's new spacing\n\
                 suppressed: boolean (optional)",
        mutates: true,
        run: edit,
    },
    CommandSpec {
        id: "feature.list",
        label: "List Features",
        params: "(none)",
        mutates: false,
        run: list,
    },
    CommandSpec {
        id: "feature.rename",
        label: "Rename Feature",
        params: "feature: integer (required)\n\
                 name: string (required) - the new name, 1 to 80 characters",
        mutates: true,
        run: rename,
    },
    CommandSpec {
        id: "solid.chamfer",
        label: "Chamfer",
        params: "edges: list of strings (optional) - edge names from solid.edges or solid.edge_at\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, instead\n\
                 distance: string or number (required) - such as 1 or \"1 mm\"; on each edge's reference face\n\
                 distance2: string or number (optional) - a different distance on each edge's other face\n\
                 angle: string or number (optional) - instead, the bevel's angle from the reference face (plain = degrees)\n\
                 reference_face: string (optional) - the face, such as \"1:top\", that takes `distance` on the edges bounding it\n\
                 flip: boolean (optional) - swap each edge's reference face and other face\n\
                 tangent_chain: boolean (optional) - also chamfer every edge that continues one tangentially",
        mutates: true,
        run: chamfer,
    },
    CommandSpec {
        id: "solid.edge_at",
        label: "Edge Under Cursor",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: edge_at,
    },
    CommandSpec {
        id: "solid.edges",
        label: "List Edges",
        params: "(none)",
        mutates: false,
        run: edges,
    },
    CommandSpec {
        id: "solid.extrude",
        label: "Extrude",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (required) - a point inside each profile, in sketch mm\n\
                 distance: string or number (required for the distance extent) - such as 10, \"10 mm\" or \"thickness * 2\"\n\
                 direction: string (optional) - normal (default), reversed, symmetric or two_sides\n\
                 distance2: string or number (optional) - the second side's distance, for two_sides\n\
                 extent: string (optional) - distance (default), through_all (past every body) or to_object\n\
                 to: object (optional) - for to_object: {feature, face}, {plane: {origin, normal}} or {pick: {x, y, width, height}}; a parallel flat face or plane\n\
                 start_offset: string or number (optional) - start this far from the sketch plane along its normal\n\
                 taper: string or number (optional) - draft angle, a plain number is degrees; positive narrows\n\
                 thin: {thickness, side, curves} (optional) - a wall of this thickness instead of the solid profile: \
                 side inside (default), outside or both; curves: an open (or closed) chain of the sketch's lines and \
                 arcs in order, instead of profiles (inside is then left of the chain)\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: extrude,
    },
    CommandSpec {
        id: "solid.extrude_preview",
        label: "Extrude Preview",
        params: "the parameters of solid.extrude - shows the shape it would make, translucent, without changing the design\n\
                 clear: boolean (optional) - true removes the preview instead",
        mutates: false,
        run: extrude_preview,
    },
    CommandSpec {
        id: "solid.revolve",
        label: "Revolve",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (required) - a point inside each profile, in sketch mm\n\
                 axis: integer (required unless axis_feature) - a line of the sketch to turn around\n\
                 axis_feature: integer (optional) - a construction axis in the sketch plane instead\n\
                 angle: string or number (optional) - default 360 deg; a plain number is degrees\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: revolve,
    },
    CommandSpec {
        id: "solid.fillet",
        label: "Fillet",
        params: "edges: list of strings (optional) - edge names from solid.edges or solid.edge_at\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, instead\n\
                 radius: string or number (required) - such as 2 or \"2 mm\"\n\
                 tangent_chain: boolean (optional) - also fillet every edge that continues one tangentially",
        mutates: true,
        run: fillet,
    },
    CommandSpec {
        id: "solid.loft",
        label: "Loft",
        params: "sections: list of {sketch, x, y} (required) - a point inside each profile, in order, \
                 2 or more, on parallel planes\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: loft,
    },
    CommandSpec {
        id: "solid.shell",
        label: "Shell",
        params: "faces: list of strings (optional) - faces to remove (open), names like \"2:top\"; none: closed and hollow\n\
                 pick: {x, y, width, height} (optional) - open the face under this pixel, instead\n\
                 thickness: string or number (required) - wall thickness, such as 2 or \"2 mm\"; negative grows outward",
        mutates: true,
        run: shell,
    },
    CommandSpec {
        id: "solid.pattern_circular",
        label: "Circular Pattern",
        params: "feature: integer (required) - the feature to repeat\n\
                 count: string or number (required) - copies including the original, 2 to 1000\n\
                 origin: {x, y, z} (required) - a point on the axis\n\
                 axis: {x, y, z} (required unless axis_feature) - the axis direction\n\
                 axis_feature: integer (optional) - a construction axis to turn around instead\n\
                 angle: string or number (optional) - spread, default 360 deg (plain = degrees)",
        mutates: true,
        run: pattern_circular,
    },
    CommandSpec {
        id: "solid.pattern_linear",
        label: "Rectangular Pattern",
        params: "feature: integer (required) - the feature to repeat\n\
                 count: string or number (required) - copies including the original, 2 to 1000\n\
                 direction: {x, y, z} (required) - which way the copies go\n\
                 spacing: string or number (required) - distance between copies",
        mutates: true,
        run: pattern_linear,
    },
    CommandSpec {
        id: "solid.path_at",
        label: "Path Under Cursor",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: path_at,
    },
    CommandSpec {
        id: "solid.profile_at",
        label: "Profile Under Cursor",
        params: "x, y, width, height: number (required) - pixel and viewport size",
        mutates: false,
        run: profile_at,
    },
    CommandSpec {
        id: "solid.tangent_chain",
        label: "Tangent Chain",
        params: "edges: list of strings (required) - edge names; replies with them and every edge that continues one tangentially",
        mutates: false,
        run: tangent_chain,
    },
    CommandSpec {
        id: "solid.sweep",
        label: "Sweep",
        params: "sketch: integer (required) - the profile's sketch\n\
                 profiles: list of {x, y} (required) - a point inside each profile, in sketch mm\n\
                 path_sketch: integer (required) - the sketch the path is drawn in\n\
                 path_curves: list of integers (required) - its lines and arcs, joined tangentially, \
                 starting on the profile's plane\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: sweep,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// A distance given as a number (mm) or an expression string.
fn distance(p: &Value) -> Result<Option<String>, CommandError> {
    match p.get("distance") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => Ok(Some(format!("{} mm", params::req_f64(p, "distance")?))),
        Some(_) => Err(bad("`distance` must be a number or an expression")),
    }
}

fn direction(p: &Value) -> Result<Option<ExtrudeDirection>, CommandError> {
    match params::opt_str(p, "direction")? {
        None => Ok(None),
        Some(o) => ExtrudeDirection::parse(o).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown direction {o:?}: use normal, reversed, symmetric or two_sides"
            ))
        }),
    }
}

pub(super) fn operation(p: &Value) -> Result<Option<Operation>, CommandError> {
    match params::opt_str(p, "operation")? {
        None => Ok(None),
        Some(o) => Operation::parse(o).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown operation {o:?}: use new_body, join, cut or intersect"
            ))
        }),
    }
}

pub(super) fn seeds(p: &Value) -> Result<Vec<Point2>, CommandError> {
    let list = p
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`profiles` must be a list of {x, y}"))?;
    if list.len() > chamfera_doc::feature::MAX_FEATURE_PROFILES {
        return Err(bad("too many profiles"));
    }
    list.iter()
        .map(|v| {
            let x = params::req_f64(v, "x")?;
            let y = params::req_f64(v, "y")?;
            let pt = Point2::new(x, y);
            if !pt.is_valid() {
                return Err(bad("a profile point is outside the working range"));
            }
            Ok(pt)
        })
        .collect()
}

/// The reply for a feature: its solids' exact volumes from the kernel, and any error.
pub(super) fn feature_reply(s: &Session, id: u32) -> Value {
    // A trim keeps its surface's name and faces, so it names that surface itself.
    let trimmed = match s.doc.feature(id).map(|f| &f.kind) {
        Ok(chamfera_doc::FeatureKind::Surface(chamfera_doc::SurfaceFeature::Trim(t))) => {
            Some(t.body.as_str())
        }
        _ => None,
    };
    let solids: Vec<Value> = s
        .solids()
        .iter()
        .filter(|x| {
            x.feature == id
                || crate::solids::body_has_feature(&x.body, id)
                || s.edited_solids(id).contains(&x.name)
                || trimmed == Some(x.name.as_str())
        })
        .map(|x| {
            json!({
                "name": x.name,
                "body": x.scene_id,
                "volume": x.body.volume().ok(),
                "area": x.body.area().ok(),
                "surface": x.body.is_open(),
            })
        })
        .collect();
    let f = s.doc.feature(id).ok();
    json!({
        "feature": id,
        "name": f.map(|f| f.name.clone()),
        "definition": f.map(|f| &f.kind),
        "suppressed": f.map(|f| s.doc.is_suppressed(f)),
        "solids": solids,
        "error": s.feature_errors().get(&id),
        "display_error": s.display_errors().get(&id),
    })
}

/// The parameter names the extrude options use (besides `sketch`, `profiles`,
/// `distance`, `direction` and `operation`).
const EXTRUDE_OPTIONS: [&str; 6] = ["extent", "distance2", "start_offset", "taper", "to", "thin"];

/// What a to-object extrude goes up to: `{feature, face}`, `{plane: {origin, normal}}`
/// or `{pick: {x, y, width, height}}` (the flat face under that pixel).
fn extrude_to(s: &Session, v: &Value) -> Result<ExtrudeTo, CommandError> {
    if !v.is_object() {
        return Err(bad(
            "`to` must be {feature, face}, {plane: {origin, normal}} or {pick: {x, y, width, height}}",
        ));
    }
    if let Some(pick) = v.get("pick") {
        let (x, y, w, h) = params::pixel(pick)?;
        let (feature, face, _) = s.face_at(x, y, w, h).ok_or_else(|| {
            CommandError::Unavailable("there is no flat solid face under that pixel".into())
        })?;
        return Ok(ExtrudeTo::Face(FaceRef { feature, face }));
    }
    if let Some(plane) = v.get("plane") {
        let o = vec3(plane, "origin")?;
        let n = vec3(plane, "normal")?;
        let origin = chamfera_geom::Point3::new(o.x, o.y, o.z);
        let frame = chamfera_geom::Frame::from_normal_and_x(origin, n, chamfera_geom::Vec3::X)
            .or_else(|| chamfera_geom::Frame::from_normal_and_x(origin, n, chamfera_geom::Vec3::Y))
            .ok_or_else(|| bad("the plane needs a point in range and a normal with a length"))?;
        return Ok(ExtrudeTo::Plane(frame));
    }
    let feature = params::req_id(v, "feature")?;
    let face = params::req_str(v, "face")?;
    // `2:top` is accepted as well as `top`.
    let face = match crate::solids::split_face_key(face) {
        Some((f, role)) if f == feature => role.to_owned(),
        _ => face.to_owned(),
    };
    Ok(ExtrudeTo::Face(FaceRef { feature, face }))
}

/// Sets the extrude options present in `p` on `e` (for `solid.extrude` and
/// `feature.edit`). A 0 start offset or taper removes it.
fn extrude_options(s: &Session, p: &Value, e: &mut Extrude) -> Result<(), CommandError> {
    if let Some(d) = distance(p)? {
        e.distance = d;
    }
    if let Some(d) = direction(p)? {
        e.direction = d;
    }
    if let Some(o) = operation(p)? {
        e.operation = o;
    }
    let to = match p.get("to") {
        None | Some(Value::Null) => None,
        Some(v) => Some(extrude_to(s, v)?),
    };
    match params::opt_str(p, "extent")? {
        Some(x) => {
            e.extent = ExtrudeExtent::parse(x).ok_or_else(|| {
                bad(format!(
                    "unknown extent {x:?}: use distance, through_all or to_object"
                ))
            })?;
        }
        None if to.is_some() => e.extent = ExtrudeExtent::ToObject,
        None => {}
    }
    if let Some(t) = to {
        e.to = Some(t);
    }
    if let Some(d) = expression(p, "distance2", "mm")? {
        e.distance2 = Some(d);
    }
    let zero = |x: &str| matches!(x.trim(), "0" | "0 mm" | "0 deg");
    if let Some(o) = expression(p, "start_offset", "mm")? {
        e.start_offset = (!zero(&o)).then_some(o);
    }
    if let Some(t) = expression(p, "taper", "deg")? {
        e.taper = (!zero(&t)).then_some(t);
    }
    match p.get("thin") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(false)) => e.thin = None,
        Some(v) if v.is_object() => e.thin = Some(thin(v, e.thin.as_ref())?),
        Some(_) => return Err(bad("`thin` must be {thickness, side, curves} or false")),
    }
    Ok(())
}

/// A thin extrude's wall from `{thickness, side, curves}`; missing keys keep `old`'s.
fn thin(v: &Value, old: Option<&chamfera_doc::Thin>) -> Result<chamfera_doc::Thin, CommandError> {
    let thickness = match (expression(v, "thickness", "mm")?, old) {
        (Some(t), _) => t,
        (None, Some(o)) => o.thickness.clone(),
        (None, None) => return Err(bad("`thin` needs a `thickness`")),
    };
    let side = match params::opt_str(v, "side")? {
        Some(s) => chamfera_doc::ThinSide::parse(s)
            .ok_or_else(|| bad(format!("unknown side {s:?}: use inside, outside or both")))?,
        None => old.map(|o| o.side).unwrap_or_default(),
    };
    let curves = match v.get("curves") {
        None | Some(Value::Null) => old.map(|o| o.curves.clone()).unwrap_or_default(),
        Some(_) => {
            let c = ids(v, "curves")?;
            if c.len() > chamfera_doc::feature::MAX_THIN_CURVES {
                return Err(bad("`curves` has too many curves"));
            }
            c
        }
    };
    Ok(chamfera_doc::Thin {
        thickness,
        side,
        curves,
    })
}

/// The extrude `solid.extrude` and `solid.extrude_preview` describe.
pub(super) fn new_extrude(s: &Session, p: &Value) -> Result<Extrude, CommandError> {
    // A thin extrude along a chain of curves takes no profiles.
    let chain = p
        .get("thin")
        .and_then(|t| t.get("curves"))
        .is_some_and(|c| !c.is_null());
    let profiles = if chain && matches!(p.get("profiles"), None | Some(Value::Null)) {
        Vec::new()
    } else {
        seeds(p)?
    };
    let mut e = Extrude {
        sketch: params::req_id(p, "sketch")?,
        profiles,
        ..Extrude::default()
    };
    extrude_options(s, p, &mut e)?;
    let by_distance = e.extent == ExtrudeExtent::Distance;
    if by_distance && distance(p)?.is_none() {
        return Err(bad("`distance` is required"));
    }
    Ok(e)
}

/// Builds the extrude `p` describes after every solid, tessellates it into the scene's
/// translucent previews and replies with its exact volume and bounds; the design does not
/// change.
fn extrude_preview(s: &mut Session, p: &Value) -> CmdResult {
    s.scene.previews.clear();
    if matches!(p.get("clear"), Some(Value::Bool(true))) {
        return Ok(json!({ "cleared": true }));
    }
    let e = new_extrude(s, p)?;
    let bodies = s.extrude_shapes(&e).map_err(CommandError::Unavailable)?;
    let mut volume = 0.0;
    let mut bounds: Option<chamfera_geom::Aabb> = None;
    let mut triangles = 0;
    for b in &bodies {
        volume += b
            .volume()
            .map_err(|x| CommandError::Unavailable(x.to_string()))?;
        if let Some(bb) = b.bounding_box() {
            bounds = Some(match bounds {
                Some(a) => a.union(bb),
                None => bb,
            });
        }
        let mesh = chamfera_kernel::tessellate(b, &chamfera_kernel::TessOptions::default())
            .map_err(|x| CommandError::Unavailable(format!("the preview cannot be shown: {x}")))?;
        triangles += mesh.mesh.triangles.len();
        s.scene.previews.push(mesh.mesh);
    }
    let pt = |q: chamfera_geom::Point3| json!([q.x, q.y, q.z]);
    Ok(json!({
        "volume": volume,
        "bodies": bodies.len(),
        "triangles": triangles,
        "bounds": bounds.map(|b| json!({ "min": pt(b.min), "max": pt(b.max) })),
    }))
}

fn extrude(s: &mut Session, p: &Value) -> CmdResult {
    let e = new_extrude(s, p)?;
    let to = match &e.to {
        Some(ExtrudeTo::Face(r)) => Some(r.clone()),
        _ => None,
    };
    let kind = FeatureKind::Extrude(e);
    s.scene.previews.clear();
    let id = s.doc.add_feature(kind)?;
    if let Some(r) = to {
        s.record_face_signature(id, &r, None);
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn revolve(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = params::req_id(p, "sketch")?;
    let axis_feature = match p.get("axis_feature") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_id(p, "axis_feature")?),
    };
    // With a construction axis the sketch line is not used (0 is no curve id).
    let axis = match axis_feature {
        Some(_) => 0,
        None => params::req_id(p, "axis")?,
    };
    let angle = match p.get("angle") {
        None | Some(Value::Null) => "360 deg".to_owned(),
        Some(Value::String(a)) => a.clone(),
        Some(Value::Number(_)) => format!("{} deg", params::req_f64(p, "angle")?),
        Some(_) => return Err(bad("`angle` must be a number or an expression")),
    };
    let kind = FeatureKind::Revolve(Revolve {
        sketch,
        profiles: seeds(p)?,
        axis,
        axis_feature,
        angle,
        operation: operation(p)?.unwrap_or_default(),
    });
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

/// A list of curve ids.
pub(super) fn ids(p: &Value, key: &str) -> Result<Vec<u32>, CommandError> {
    let list = p
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| bad(format!("`{key}` must be a list of curve ids")))?;
    if list.len() > chamfera_doc::feature::MAX_SWEEP_PATH_CURVES {
        return Err(bad(format!("`{key}` has too many curves")));
    }
    list.iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| bad(format!("`{key}` must hold curve ids (whole numbers)")))
        })
        .collect()
}

/// A loft's profiles: `[{sketch, x, y}, ...]`.
fn sections(p: &Value) -> Result<Vec<LoftSection>, CommandError> {
    let list = p
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`sections` must be a list of {sketch, x, y}"))?;
    if list.len() > chamfera_doc::feature::MAX_LOFT_SECTIONS {
        return Err(bad("too many sections"));
    }
    list.iter()
        .map(|v| {
            let sketch = params::req_id(v, "sketch")?;
            let profile = Point2::new(params::req_f64(v, "x")?, params::req_f64(v, "y")?);
            if !profile.is_valid() {
                return Err(bad("a profile point is outside the working range"));
            }
            Ok(LoftSection { sketch, profile })
        })
        .collect()
}

fn add_feature(s: &mut Session, kind: FeatureKind) -> CmdResult {
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn sweep(s: &mut Session, p: &Value) -> CmdResult {
    let kind = FeatureKind::Sweep(Sweep {
        sketch: params::req_id(p, "sketch")?,
        profiles: seeds(p)?,
        path_sketch: params::req_id(p, "path_sketch")?,
        path: ids(p, "path_curves")?,
        operation: operation(p)?.unwrap_or_default(),
    });
    add_feature(s, kind)
}

fn loft(s: &mut Session, p: &Value) -> CmdResult {
    let kind = FeatureKind::Loft(Loft {
        sections: sections(p)?,
        operation: operation(p)?.unwrap_or_default(),
    });
    add_feature(s, kind)
}

/// The edges a fillet or chamfer names: a list, or the one under a pixel.
fn blend_edges(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    match (p.get("edges"), p.get("pick")) {
        (Some(Value::Array(list)), None | Some(Value::Null)) => list
            .iter()
            .map(|e| {
                e.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad("`edges` must be a list of edge names"))
            })
            .collect(),
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y) = (params::req_f64(pick, "x")?, params::req_f64(pick, "y")?);
            let (w, h) = (
                params::req_f64(pick, "width")?,
                params::req_f64(pick, "height")?,
            );
            s.edge_at(x, y, w, h)
                .map(|(_, _, r)| vec![r])
                .ok_or_else(|| {
                    CommandError::Unavailable("there is no edge under that pixel".into())
                })
        }
        (None | Some(Value::Null), None) => Err(bad("give `edges` or `pick`")),
        _ => Err(bad("give either `edges` (a list) or `pick`, not both")),
    }
}

/// The options of a fillet or chamfer present in `p`, applied to `b` (for the blend
/// commands and `feature.edit`, where the second distance is `size2` and the angle
/// `chamfer_angle`, since extrudes and planes use `distance2` and `angle`).
pub(super) fn blend_options(
    b: &mut Blend,
    p: &Value,
    distance2: &str,
    angle: &str,
) -> Result<(), CommandError> {
    if let Some(v) = opt_bool(p, "tangent_chain")? {
        b.tangent_chain = v;
    }
    if let Some(v) = opt_bool(p, "flip")? {
        b.flip = v;
    }
    let none = |x: &str| matches!(x.trim(), "" | "none" | "0" | "0 mm" | "0 deg");
    if let Some(d) = expression(p, distance2, "mm")? {
        b.distance2 = (!none(&d)).then_some(d);
        if b.distance2.is_some() {
            b.angle = None;
        }
    }
    if let Some(a) = expression(p, angle, "deg")? {
        b.angle = (!none(&a)).then_some(a);
        if b.angle.is_some() {
            b.distance2 = None;
        }
    }
    match p.get("reference_face") {
        None => {}
        Some(Value::Null) => b.reference_face = None,
        Some(Value::String(f)) if f.is_empty() => b.reference_face = None,
        Some(Value::String(f)) => {
            if crate::solids::split_face_key(f).is_none() {
                return Err(bad(
                    "`reference_face` must be a face name such as \"1:top\" (feature:role)",
                ));
            }
            b.reference_face = Some(f.clone());
        }
        Some(_) => return Err(bad("`reference_face` must be a face name")),
    }
    Ok(())
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => params::req_bool(p, key).map(Some),
    }
}

fn blend(s: &mut Session, p: &Value, kind: BlendKind, key: &str) -> CmdResult {
    let edges = blend_edges(s, p)?;
    let size = expression(p, key, "mm")?.ok_or_else(|| bad(format!("`{key}` is required")))?;
    let mut b = Blend::new(kind, edges.clone(), size);
    blend_options(&mut b, p, "distance2", "angle")?;
    if kind == BlendKind::Fillet
        && ["distance2", "angle", "reference_face", "flip"]
            .iter()
            .any(|k| !matches!(p.get(*k), None | Some(Value::Null)))
    {
        return Err(bad(
            "distance2, angle, reference_face and flip belong to chamfers",
        ));
    }
    let reference = b.reference_face.clone();
    let id = s.doc.add_feature(FeatureKind::Blend(b))?;
    // What each edge is now, so that the fillet keeps to the piece it names when the
    // edge splits, and finds the edge again if a boolean renames it (rung K8).
    for e in &edges {
        s.record_signature(id, crate::naming::Kind::Edge, e, None);
    }
    if let Some(f) = reference {
        s.record_signature(id, crate::naming::Kind::Face, &f, None);
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn tangent_chain(s: &mut Session, p: &Value) -> CmdResult {
    let list = p
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`edges` must be a list of edge names"))?;
    if list.len() > chamfera_doc::feature::MAX_BLEND_EDGES {
        return Err(bad("too many edges"));
    }
    let names = list
        .iter()
        .map(|e| {
            e.as_str()
                .map(str::to_owned)
                .ok_or_else(|| bad("`edges` must be a list of edge names"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let chain = s
        .tangent_chain_names(&names)
        .map_err(CommandError::Unavailable)?;
    Ok(json!({ "edges": chain }))
}

fn fillet(s: &mut Session, p: &Value) -> CmdResult {
    blend(s, p, BlendKind::Fillet, "radius")
}

fn chamfer(s: &mut Session, p: &Value) -> CmdResult {
    blend(s, p, BlendKind::Chamfer, "distance")
}

/// The faces a shell opens: a list (possibly empty), the one under a pixel, or none.
fn shell_faces(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    match (p.get("faces"), p.get("pick")) {
        (Some(Value::Array(list)), None | Some(Value::Null)) => list
            .iter()
            .map(|f| {
                f.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad("`faces` must be a list of face names"))
            })
            .collect(),
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y) = (params::req_f64(pick, "x")?, params::req_f64(pick, "y")?);
            let (w, h) = (
                params::req_f64(pick, "width")?,
                params::req_f64(pick, "height")?,
            );
            s.face_ref_at(x, y, w, h).map(|r| vec![r]).ok_or_else(|| {
                CommandError::Unavailable("there is no face of a solid under that pixel".into())
            })
        }
        (None | Some(Value::Null), None) => Ok(Vec::new()),
        (Some(_), None | Some(Value::Null)) => Err(bad("`faces` must be a list of face names")),
        _ => Err(bad("give either `faces` (a list) or `pick`, not both")),
    }
}

fn shell(s: &mut Session, p: &Value) -> CmdResult {
    let faces = shell_faces(s, p)?;
    let thickness =
        expression(p, "thickness", "mm")?.ok_or_else(|| bad("`thickness` is required"))?;
    let id = s.doc.add_feature(FeatureKind::Shell(chamfera_doc::Shell {
        faces: faces.clone(),
        thickness,
    }))?;
    for f in &faces {
        s.record_signature(id, crate::naming::Kind::Face, f, None);
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn edge_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y) = (params::req_f64(p, "x")?, params::req_f64(p, "y")?);
    let (w, h) = (params::req_f64(p, "width")?, params::req_f64(p, "height")?);
    Ok(match s.edge_at(x, y, w, h) {
        Some((si, _, r)) => json!({
            "edge": r,
            "solid": s.solids().get(si).map(|x| x.name.clone()),
            "body": s.solids().get(si).and_then(|x| x.scene_id),
        }),
        None => json!({ "edge": null }),
    })
}

/// Every edge of every solid, by its unique name (`2:top 0`, `2:top 0#2` for a piece of
/// a split edge), with its end points.
fn edges(s: &mut Session, _p: &Value) -> CmdResult {
    let pt = |q: chamfera_geom::Point3| json!([q.x, q.y, q.z]);
    let names: std::collections::BTreeMap<(usize, usize), String> = s
        .reference_names(crate::naming::Kind::Edge)
        .into_iter()
        .filter_map(|(f, n)| Some(((f.body, f.edge()?.index()), n)))
        .collect();
    let solids: Vec<Value> = s
        .solids()
        .iter()
        .enumerate()
        .map(|(si, x)| {
            let edges: Vec<Value> = x
                .body
                .edges()
                .into_iter()
                .filter_map(|e| {
                    let name = names.get(&(si, e.index()))?.clone();
                    let [a, b] = x.body.edge_vertices(e).ok()?;
                    Some(json!({
                        "edge": name,
                        "from": pt(x.body.vertex_point(a).ok()?),
                        "to": pt(x.body.vertex_point(b).ok()?),
                    }))
                })
                .collect();
            json!({ "solid": x.name, "body": x.scene_id, "edges": edges })
        })
        .collect();
    Ok(json!({ "solids": solids }))
}

pub(super) fn expression(p: &Value, key: &str, unit: &str) -> Result<Option<String>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(_)) => {
            let v = params::req_f64(p, key)?;
            Ok(Some(if unit.is_empty() {
                format!("{v}")
            } else {
                format!("{v} {unit}")
            }))
        }
        Some(_) => Err(bad(format!("`{key}` must be a number or an expression"))),
    }
}

pub(super) fn vec3(p: &Value, key: &str) -> Result<chamfera_geom::Vec3, CommandError> {
    let v = p
        .get(key)
        .ok_or_else(|| bad(format!("`{key}` is required")))?;
    Ok(chamfera_geom::Vec3::new(
        params::req_f64(v, "x")?,
        params::req_f64(v, "y")?,
        params::req_f64(v, "z")?,
    ))
}

fn add_pattern(s: &mut Session, pattern: chamfera_doc::Pattern) -> CmdResult {
    let id = s.doc.add_feature(FeatureKind::Pattern(pattern))?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn pattern_linear(s: &mut Session, p: &Value) -> CmdResult {
    let source = params::req_id(p, "feature")?;
    let count = expression(p, "count", "")?.ok_or_else(|| bad("`count` is required"))?;
    let spacing = expression(p, "spacing", "mm")?.ok_or_else(|| bad("`spacing` is required"))?;
    let direction = vec3(p, "direction")?;
    add_pattern(
        s,
        chamfera_doc::Pattern {
            source,
            count,
            layout: chamfera_doc::PatternLayout::Linear { direction, spacing },
        },
    )
}

fn pattern_circular(s: &mut Session, p: &Value) -> CmdResult {
    let source = params::req_id(p, "feature")?;
    let count = expression(p, "count", "")?.ok_or_else(|| bad("`count` is required"))?;
    let angle = expression(p, "angle", "deg")?.unwrap_or_else(|| "360 deg".to_owned());
    let axis_feature = match p.get("axis_feature") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_id(p, "axis_feature")?),
    };
    // A construction axis gives the origin and direction (followed on rebuild).
    let (o, axis) = match axis_feature {
        Some(id) => match s.constructs().get(&id) {
            Some(crate::construct::Constructed::Axis {
                origin, direction, ..
            }) => (
                chamfera_geom::Vec3::new(origin.x, origin.y, origin.z),
                *direction,
            ),
            _ => {
                return Err(bad(format!(
                    "feature {id} is not a built construction axis"
                )));
            }
        },
        None => (vec3(p, "origin")?, vec3(p, "axis")?),
    };
    add_pattern(
        s,
        chamfera_doc::Pattern {
            source,
            count,
            layout: chamfera_doc::PatternLayout::Circular {
                origin: chamfera_geom::Point3::new(o.x, o.y, o.z),
                axis,
                angle,
                axis_feature,
            },
        },
    )
}

fn edit(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "feature")?;
    if super::face_edit::is_face_edit(&s.doc.feature(id)?.kind) {
        return super::face_edit::edit(s, id, p);
    }
    if matches!(s.doc.feature(id)?.kind, FeatureKind::Surface(_)) {
        return super::surface::edit(s, id, p);
    }
    if matches!(s.doc.feature(id)?.kind, FeatureKind::Thread(_)) {
        return super::thread::edit(s, id, p);
    }
    refuse_unknown_edit_keys(p)?;
    let new_distance = distance(p)?;
    let new_direction = direction(p)?;
    let new_operation = operation(p)?;
    let new_angle = expression(p, "angle", "deg")?;
    let suppressed = match p.get("suppressed") {
        None | Some(Value::Null) => None,
        Some(_) => Some(params::req_bool(p, "suppressed")?),
    };
    let mut kind = s.doc.feature(id)?.kind.clone();
    if let Some(o) = new_operation {
        match &mut kind {
            FeatureKind::Extrude(e) => e.operation = o,
            FeatureKind::Revolve(r) => r.operation = o,
            FeatureKind::Sweep(w) => w.operation = o,
            FeatureKind::Loft(l) => l.operation = o,
            FeatureKind::Pattern(_) => {
                return Err(bad("a pattern does what the feature it repeats does"));
            }
            FeatureKind::Construct(_) => {
                return Err(bad("construction geometry has no operation"));
            }
            FeatureKind::Blend(_) => {
                return Err(bad("a fillet or chamfer has no operation"));
            }
            FeatureKind::Hole(_) => return Err(bad("a hole always cuts")),
            FeatureKind::Mirror(m) => m.operation = o,
            FeatureKind::Move(_) => return Err(bad("a move has no operation")),
            FeatureKind::Shell(_) => {
                return Err(bad("a shell has no operation"));
            }
            FeatureKind::SheetBase(_)
            | FeatureKind::SheetFlange(_)
            | FeatureKind::SheetUnfold(_) => {
                return Err(bad("sheet metal features have no operation"));
            }
            FeatureKind::OffsetFace(_) | FeatureKind::DeleteFace(_) | FeatureKind::MoveFace(_) => {
                return Err(bad("a face edit has no operation"));
            }
            FeatureKind::Combine(c) => c.operation = o,
            FeatureKind::Thicken(t) => t.operation = o,
            FeatureKind::Surface(_) => {
                return Err(bad("a surface feature always makes a new body"));
            }
            FeatureKind::SplitBody(_) | FeatureKind::SplitFace(_) | FeatureKind::Remove(_) => {
                return Err(bad("a split or remove has no operation"));
            }
            FeatureKind::Thread(_) => return Err(bad("a thread has no operation")),
        }
    }
    super::sheet::edit(s, &mut kind, p)?;
    // `to` is shared with extrudes and moves.
    let hole_keys = super::hole::EDIT_KEYS
        .iter()
        .filter(|k| **k != "to")
        .any(|k| p.get(*k).is_some_and(|v| !v.is_null()));
    if let FeatureKind::Hole(h) = &mut kind {
        super::hole::apply(s, h, p)?;
    } else if hole_keys {
        return Err(bad("points, kind and the hole sizes belong to holes"));
    }
    super::transform::edit_transform(&mut kind, p)?;
    super::body_tools::edit_body_tool(&mut kind, p)?;
    let new_size = expression(p, "size", "mm")?;
    if let Some(size) = new_size {
        let FeatureKind::Blend(b) = &mut kind else {
            return Err(bad("size belongs to fillets and chamfers"));
        };
        b.size = size;
    }
    if BLEND_EDIT_KEYS
        .iter()
        .any(|k| !matches!(p.get(*k), None | Some(Value::Null)))
    {
        let FeatureKind::Blend(b) = &mut kind else {
            return Err(bad(
                "tangent_chain, size2, chamfer_angle, reference_face and flip belong to fillets and chamfers",
            ));
        };
        blend_options(b, p, "size2", "chamfer_angle")?;
        if let Some(f) = b.reference_face.clone() {
            s.record_signature(id, crate::naming::Kind::Face, &f, None);
        }
    }
    let new_thickness = expression(p, "thickness", "mm")?;
    let new_faces = match p.get("faces") {
        None | Some(Value::Null) => None,
        Some(_) => Some(shell_faces(s, &json!({ "faces": p.get("faces") }))?),
    };
    if new_thickness.is_some() || new_faces.is_some() {
        // Faces new to the feature get a signature; removed ones lose theirs (rung K8).
        let old: Vec<String> = match &kind {
            FeatureKind::Shell(sh) => sh.faces.clone(),
            FeatureKind::Thicken(th) => th.faces.clone(),
            FeatureKind::SplitFace(sf) => sf.faces.clone(),
            _ => Vec::new(),
        };
        if let Some(f) = new_faces.as_ref() {
            for name in f.iter().filter(|n| !old.contains(n)) {
                s.record_signature(id, crate::naming::Kind::Face, name, None);
            }
            let keep: Vec<&str> = f.iter().map(String::as_str).collect();
            s.doc.retain_signatures(id, &keep);
        }
        match &mut kind {
            FeatureKind::Shell(sh) => {
                if let Some(t) = new_thickness {
                    sh.thickness = t;
                }
                if let Some(f) = new_faces {
                    sh.faces = f;
                }
            }
            FeatureKind::Thicken(th) => {
                if let Some(t) = new_thickness {
                    th.thickness = t;
                }
                if let Some(f) = new_faces {
                    th.faces = f;
                }
            }
            FeatureKind::SplitFace(sf) if new_thickness.is_none() => {
                if let Some(f) = new_faces {
                    sf.faces = f;
                }
            }
            _ => {
                return Err(bad(
                    "thickness belongs to shells and thickens; faces to shells, thickens and split faces",
                ));
            }
        }
    }
    // A hole's `to` was applied with its other values above.
    let hole = matches!(kind, FeatureKind::Hole(_));
    let extrude_keys = EXTRUDE_OPTIONS
        .iter()
        .filter(|k| !(hole && **k == "to"))
        .any(|k| !matches!(p.get(*k), None | Some(Value::Null)));
    match &mut kind {
        FeatureKind::Extrude(e) => extrude_options(s, p, e)?,
        FeatureKind::Construct(chamfera_doc::Construction::PlaneOffset { distance, .. })
            if new_direction.is_none() && !extrude_keys =>
        {
            if let Some(d) = new_distance {
                *distance = d;
            }
        }
        // A move's `to` (point to point) was applied by edit_transform above.
        FeatureKind::Move(_) if new_distance.is_none() && new_direction.is_none() => {}
        _ if new_distance.is_some() || new_direction.is_some() || extrude_keys => {
            return Err(bad(
                "distance belongs to extrudes and offset planes; direction, extent, distance2, \
                 start_offset, taper, to and thin to extrudes",
            ));
        }
        _ => {}
    }
    match &mut kind {
        FeatureKind::Construct(chamfera_doc::Construction::PlaneAngle { angle, .. }) => {
            if let Some(a) = new_angle {
                *angle = a;
            }
        }
        // A move's angle was applied by edit_transform above, a flange's by sheet::edit.
        FeatureKind::Move(_) | FeatureKind::SheetFlange(_) => {}
        _ if new_angle.is_some() => {
            return Err(bad("angle belongs to planes at an angle"));
        }
        _ => {}
    }
    let new_count = expression(p, "count", "")?;
    let new_spacing = expression(p, "spacing", "mm")?;
    if new_count.is_some() || new_spacing.is_some() {
        let FeatureKind::Pattern(pat) = &mut kind else {
            return Err(bad("count and spacing belong to patterns"));
        };
        if let Some(c) = new_count {
            pat.count = c;
        }
        if let Some(sp) = new_spacing {
            let chamfera_doc::PatternLayout::Linear { spacing, .. } = &mut pat.layout else {
                return Err(bad("spacing belongs to rectangular (linear) patterns"));
            };
            *spacing = sp;
        }
    }
    if p.get("path_curves").is_some_and(|v| !v.is_null()) {
        let FeatureKind::Sweep(w) = &mut kind else {
            return Err(bad("path_curves belong to sweeps"));
        };
        w.path = ids(p, "path_curves")?;
    }
    if p.get("sections").is_some_and(|v| !v.is_null()) {
        let FeatureKind::Loft(l) = &mut kind else {
            return Err(bad("sections belong to lofts"));
        };
        l.sections = sections(p)?;
    }
    s.doc.check_feature(&kind)?;
    let f = s.doc.feature_mut(id)?;
    f.kind = kind;
    if let Some(v) = suppressed {
        s.doc.set_suppressed(id, v)?;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

/// The keys `feature.edit` hands to [`blend_options`].
const BLEND_EDIT_KEYS: &[&str] = &[
    "tangent_chain",
    "size2",
    "chamfer_angle",
    "reference_face",
    "flip",
];

/// Every key `feature.edit` reads (outside face edits and surfaces, which have their own). A key not
/// in it is refused: a misspelt or unsupported key used to succeed without changing
/// anything and without an undo step, so the next undo removed an earlier change.
const EDIT_KEYS: &[&str] = &[
    "feature",
    "distance",
    "angle",
    "direction",
    "extent",
    "distance2",
    "to",
    "start_offset",
    "taper",
    "thin",
    "operation",
    "size",
    "path_curves",
    "sections",
    "plane",
    "dx",
    "dy",
    "dz",
    "axis",
    "origin",
    "from",
    "copy",
    "thickness",
    "faces",
    "reversed",
    "target",
    "tools",
    "keep_tools",
    "bodies",
    "sketch",
    "line",
    "sides",
    "suppressed",
    "count",
    "spacing",
];

fn refuse_unknown_edit_keys(p: &Value) -> Result<(), CommandError> {
    let Some(obj) = p.as_object() else {
        return Ok(());
    };
    for k in obj.keys() {
        let known = EDIT_KEYS.contains(&k.as_str())
            || BLEND_EDIT_KEYS.contains(&k.as_str())
            || super::hole::EDIT_KEYS.contains(&k.as_str())
            || super::sheet::EDIT_KEYS.contains(&k.as_str());
        if !known {
            return Err(bad(format!(
                "feature.edit has no parameter `{k}` (see `app.commands` for the list)"
            )));
        }
    }
    Ok(())
}

fn delete(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "feature")?;
    let users = s.doc.feature_users(id);
    if !users.is_empty() {
        let name = s
            .doc
            .feature(id)
            .map_or_else(|_| id.to_string(), |f| f.name.clone());
        return Err(CommandError::Unavailable(format!(
            "{name} is used by {}; delete or change them first",
            users.join(", ")
        )));
    }
    s.doc.remove_feature(id)?;
    s.rebuild();
    Ok(json!({ "deleted": id }))
}

fn rename(s: &mut Session, p: &Value) -> CmdResult {
    let id = params::req_id(p, "feature")?;
    let name = params::req_display_name(p, "name")?;
    s.doc.feature(id)?;
    s.doc.rename_item(id, &name)?;
    // Bodies are named after their features.
    s.rebuild();
    Ok(feature_reply(s, id))
}

fn list(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    let ids: Vec<u32> = s.doc.features.iter().map(|f| f.id).collect();
    let features: Vec<Value> = ids.into_iter().map(|id| feature_reply(s, id)).collect();
    Ok(json!({ "features": features }))
}

fn path_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    Ok(match s.path_at(x, y, w, h) {
        Some((sketch, curve, chain)) => json!({
            "sketch": sketch, "curve": curve, "chain": chain,
        }),
        None => json!({ "sketch": null }),
    })
}

fn profile_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    Ok(match s.profile_at(x, y, w, h) {
        Some((sketch, at, area)) => json!({
            "sketch": sketch, "x": at.x, "y": at.y, "area": area,
        }),
        None => json!({ "sketch": null }),
    })
}

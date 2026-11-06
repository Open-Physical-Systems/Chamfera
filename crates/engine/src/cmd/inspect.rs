//! `inspect.*`: measuring solids. Mass properties, and distance, angle, length, area and
//! radius between picked entities, all from the exact kernel geometry (never the mesh).
//!
//! # Entity references
//! An entity is a JSON object naming a solid and, optionally, one of its pieces:
//! - `{"solid": "Extrude 1"}`: the whole solid (its boundary); `"body": 3` (the scene id
//!   that shows it) may stand in for `"solid"`;
//! - `{"solid": "Extrude 1", "face": "2:top"}`: a face by its name (as in
//!   `sketch.create_on_face`);
//! - `{"solid": "Extrude 1", "edge": "2:top 0"}`: an edge by the name `solid.edges` and
//!   `solid.edge_at` give;
//! - `{"solid": "Extrude 1", "vertex": "2:..."}`: a vertex by name, or `"vertex": [x, y, z]`
//!   for the vertex at that point;
//! - `{"point": [x, y, z]}`: a free point in space.
//!
//! A face, edge or vertex without a name is `"#n"` (its index in the body), which is what
//! `inspect.measure_at` gives for them.

use super::CommandSpec;
use crate::solids::{EDGE_PICK_PIXELS, Solid, edge_ref};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_geom::{LINEAR_TOLERANCE, Point3};
use chamfera_kernel::{Body, Curve, Entity, Surface};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "inspect.interference",
        label: "Interference",
        params: "components: [integer] (optional) - check only the bodies of these components (0 is the root)\n\
                 bodies: [string] (optional) - check only the bodies with these names\n\
                 at_keys: boolean (optional) - also check at each motion study keyframe\n\
                 clear: boolean (optional) - remove the overlap highlight and check nothing\n\
                 Replies with each overlapping pair (as placed) and its overlap volume in mm³, from the exact intersection; the overlaps are drawn highlighted.",
        mutates: false,
        run: super::motion::interference,
    },
    CommandSpec {
        id: "inspect.mass",
        label: "Mass Properties",
        params: "body: string or number (required) - the solid's name, or the scene id that shows it\n\
                 density: number (optional) - mass per mm³, default 1",
        mutates: false,
        run: mass,
    },
    CommandSpec {
        id: "inspect.measure",
        label: "Measure",
        params: "a: object (required) - an entity: {solid, face|edge|vertex} or {point: [x, y, z]}\n\
                 b: object (optional) - a second entity; with it: distance, closest points, angle",
        mutates: false,
        run: measure,
    },
    CommandSpec {
        id: "inspect.measure_at",
        label: "Measure At",
        params: "x, y: number (required) - pixel, origin top-left\n\
                 width, height: number (required) - viewport size in pixels\n\
                 from: object (optional) - an entity picked before; measures from it to this one",
        mutates: false,
        run: measure_at,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

/// A resolved entity: the solid it is on (none for a free point) and the kernel entity.
struct Target {
    solid: Option<usize>,
    entity: Entity,
    /// The reference that names it, echoed in replies.
    reference: Value,
}

fn point_of(v: &Value, key: &str) -> Result<Point3, CommandError> {
    let xs = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| bad(format!("`{key}` must be [x, y, z]")))?;
    let mut c = [0.0; 3];
    for (k, x) in c.iter_mut().zip(xs) {
        *k = x
            .as_f64()
            .filter(|f| chamfera_geom::is_valid_coordinate(*f))
            .ok_or_else(|| bad(format!("`{key}` must hold finite coordinates in range")))?;
    }
    Ok(Point3::new(c[0], c[1], c[2]))
}

/// The solid a parameter names: a solid name, or the scene id that shows it.
fn find_solid(s: &Session, v: &Value, key: &str) -> Result<usize, CommandError> {
    let found = match v {
        Value::String(name) => s.solids().iter().position(|x| x.name == *name),
        Value::Number(n) => n
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .and_then(|id| s.solids().iter().position(|x| x.scene_id == Some(id))),
        _ => return Err(bad(format!("`{key}` must be a solid name or a scene id"))),
    };
    found.ok_or_else(|| {
        bad(format!(
            "there is no solid {v} (see body.list or solid.edges)"
        ))
    })
}

/// `#n`: an index into a list of ids.
fn by_index<T: Copy>(name: &str, ids: Vec<T>, index: impl Fn(T) -> usize) -> Option<T> {
    let n: usize = name.strip_prefix('#')?.parse().ok()?;
    ids.into_iter().find(|i| index(*i) == n)
}

fn named(v: &Value, key: &str) -> Result<String, CommandError> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= chamfera_kernel::MAX_NAME_LENGTH * 2 + 16)
        .map(str::to_owned)
        .ok_or_else(|| bad(format!("`{key}` must be a name")))
}

fn resolve(s: &Session, r: &Value, key: &str) -> Result<Target, CommandError> {
    let obj = r
        .as_object()
        .ok_or_else(|| bad(format!("`{key}` must be an entity object")))?;
    if let Some(p) = obj.get("point") {
        return Ok(Target {
            solid: None,
            entity: Entity::Point(point_of(p, &format!("{key}.point"))?),
            reference: r.clone(),
        });
    }
    let solid_key = if obj.contains_key("solid") {
        "solid"
    } else {
        "body"
    };
    let which = obj
        .get(solid_key)
        .ok_or_else(|| bad(format!("`{key}` needs `solid` (or `body`) or `point`")))?;
    let si = find_solid(s, which, &format!("{key}.{solid_key}"))?;
    let body = &s
        .solids()
        .get(si)
        .ok_or_else(|| bad("that solid is gone"))?
        .body;
    let entity = if let Some(f) = obj.get("face") {
        let name = named(f, &format!("{key}.face"))?;
        let face = body
            .faces()
            .into_iter()
            .find(|x| body.face_name(*x).ok().flatten() == Some(name.as_str()))
            .or_else(|| by_index(&name, body.faces(), |x| x.index()))
            .ok_or_else(|| bad(format!("the solid has no face {name:?}")))?;
        Entity::Face(face)
    } else if let Some(e) = obj.get("edge") {
        let name = named(e, &format!("{key}.edge"))?;
        let edge = body
            .edges()
            .into_iter()
            .find(|x| edge_ref(body, *x).as_deref() == Some(name.as_str()))
            .or_else(|| by_index(&name, body.edges(), |x| x.index()))
            .ok_or_else(|| bad(format!("the solid has no edge {name:?}")))?;
        Entity::Edge(edge)
    } else if let Some(v) = obj.get("vertex") {
        let vertex = if v.is_array() {
            let p = point_of(v, &format!("{key}.vertex"))?;
            body.vertices().into_iter().find(|x| {
                body.vertex_point(*x)
                    .is_ok_and(|q| q.distance(p) <= 1e3 * LINEAR_TOLERANCE)
            })
        } else {
            let name = named(v, &format!("{key}.vertex"))?;
            body.vertices()
                .into_iter()
                .find(|x| body.vertex_name(*x).ok().flatten() == Some(name.as_str()))
                .or_else(|| by_index(&name, body.vertices(), |x| x.index()))
        }
        .ok_or_else(|| bad(format!("the solid has no vertex {v}")))?;
        Entity::Vertex(vertex)
    } else {
        Entity::Body
    };
    Ok(Target {
        solid: Some(si),
        entity,
        reference: r.clone(),
    })
}

fn kernel(e: chamfera_kernel::KernelError) -> CommandError {
    CommandError::Unavailable(e.to_string())
}

fn xyz(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

/// What one entity measures on its own: kind, and length, area, radius, volume.
fn describe(s: &Session, t: &Target) -> Result<Value, CommandError> {
    let empty = Body::new();
    let solid: Option<&Solid> = t.solid.and_then(|i| s.solids().get(i));
    let body = solid.map_or(&empty, |x| &x.body);
    let mut out = json!({ "ref": t.reference });
    if let Some(x) = solid {
        out["solid"] = json!(x.name);
    }
    match t.entity {
        Entity::Point(p) => {
            out["kind"] = json!("point");
            out["point"] = xyz(p);
        }
        Entity::Vertex(v) => {
            out["kind"] = json!("vertex");
            out["point"] = xyz(body.vertex_point(v).map_err(kernel)?);
        }
        Entity::Edge(e) => {
            out["kind"] = json!("edge");
            let curve = match body.edge_curve(e).map_err(kernel)? {
                Some((Curve::Line(_) | Curve::Segment(_), ..)) => "line",
                Some((Curve::Circle(_), ..)) => "circle",
                Some((Curve::Arc(_), ..)) => "arc",
                Some((Curve::Ellipse(_) | Curve::EllipticalArc(_), ..)) => "ellipse",
                Some((Curve::BSpline(_), ..)) => "spline",
                None => "none",
            };
            out["curve"] = json!(curve);
            out["length"] = json!(body.edge_length(e).map_err(kernel)?);
            if let Some(r) = body.edge_radius(e).map_err(kernel)? {
                out["radius"] = json!(r);
                out["diameter"] = json!(2.0 * r);
            }
        }
        Entity::Face(f) => {
            out["kind"] = json!("face");
            let surface = match body.face_surface(f).map_err(kernel)? {
                Some((Surface::Plane(_), _)) => "plane",
                Some((Surface::Cylinder(_), _)) => "cylinder",
                Some((Surface::Cone(_), _)) => "cone",
                Some((Surface::Sphere(_), _)) => "sphere",
                Some((Surface::Torus(_), _)) => "torus",
                Some((Surface::Extrusion(_), _)) => "extrusion",
                None => "none",
            };
            out["surface"] = json!(surface);
            out["area"] = json!(body.face_area(f).map_err(kernel)?);
            if let Some(r) = body.face_radius(f).map_err(kernel)? {
                out["radius"] = json!(r);
                out["diameter"] = json!(2.0 * r);
            }
        }
        Entity::Body => {
            out["kind"] = json!("body");
            out["volume"] = json!(body.volume().map_err(kernel)?);
            out["area"] = json!(body.area().map_err(kernel)?);
        }
    }
    Ok(out)
}

/// The reply for one entity, or for a pair: distance, closest points and angle.
fn measure_targets(s: &Session, a: &Target, b: Option<&Target>) -> CmdResult {
    let mut out = json!({ "a": describe(s, a)? });
    let Some(b) = b else {
        return Ok(out);
    };
    out["b"] = describe(s, b)?;
    let empty = Body::new();
    let body = |t: &Target| {
        t.solid
            .and_then(|i| s.solids().get(i))
            .map_or(&empty, |x| &x.body)
    };
    let (ba, bb) = (body(a), body(b));
    let c = chamfera_kernel::closest_points(ba, a.entity, bb, b.entity).map_err(kernel)?;
    out["distance"] = json!(c.distance);
    out["on_a"] = xyz(c.on_a);
    out["on_b"] = xyz(c.on_b);
    out["delta"] = json!([
        c.on_b.x - c.on_a.x,
        c.on_b.y - c.on_a.y,
        c.on_b.z - c.on_a.z
    ]);
    let same = a.solid.is_some() && a.solid == b.solid;
    let angle = if same {
        chamfera_kernel::angle_between(ba, a.entity, ba, b.entity)
    } else {
        chamfera_kernel::angle_between(ba, a.entity, bb, b.entity)
    }
    .map_err(kernel)?;
    if let Some(t) = angle {
        out["angle"] = json!(t);
        out["angle_degrees"] = json!(t.to_degrees());
    }
    Ok(out)
}

fn mass(s: &mut Session, p: &Value) -> CmdResult {
    let which = p
        .as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))?
        .get("body")
        .ok_or_else(|| bad("`body` is required"))?;
    let si = find_solid(s, which, "body")?;
    let density = params::opt_f64(p, "density")?.unwrap_or(1.0);
    let solid = s
        .solids()
        .get(si)
        .ok_or_else(|| bad("that solid is gone"))?;
    let m = solid.body.mass_properties(density).map_err(kernel)?;
    Ok(json!({
        "solid": solid.name,
        "body": solid.scene_id,
        "density": m.density,
        "volume": m.volume,
        "area": m.area,
        "mass": m.mass,
        "centre_of_mass": xyz(m.centre_of_mass),
        "inertia": m.inertia,
        "principal_moments": m.principal_moments,
        "principal_axes": m.principal_axes.map(|a| [a.x, a.y, a.z]),
    }))
}

fn measure(s: &mut Session, p: &Value) -> CmdResult {
    let obj = p
        .as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))?;
    let a = resolve(s, obj.get("a").ok_or_else(|| bad("`a` is required"))?, "a")?;
    let b = match obj.get("b") {
        None | Some(Value::Null) => None,
        Some(v) => Some(resolve(s, v, "b")?),
    };
    measure_targets(s, &a, b.as_ref())
}

impl Session {
    /// The entity under pixel `(x, y)`: a vertex near the pointer at the end of the edge
    /// under it, else that edge, else the face the ray meets first. Returns its reference
    /// (see the module documentation).
    pub fn entity_at(&self, x: f64, y: f64, w: f64, h: f64) -> Option<Value> {
        use chamfera_render::{mat_mul, transform_point};
        let bounds = self.scene.bounds();
        if let Some((si, e, name)) = self.edge_at(x, y, w, h) {
            let solid = self.solids().get(si)?;
            let vp = mat_mul(
                &self.camera.projection_matrix(w / h, bounds.as_ref()),
                &self.camera.view_matrix(),
            );
            let screen = |p: Point3| -> Option<(f64, f64)> {
                let n = transform_point(&vp, p)?;
                Some(((n[0] + 1.0) * 0.5 * w, (1.0 - n[1]) * 0.5 * h))
            };
            for v in solid.body.edge_vertices(e).ok()? {
                let p = solid.body.vertex_point(v).ok()?;
                if let Some((sx, sy)) = screen(p)
                    && (sx - x).hypot(sy - y) <= EDGE_PICK_PIXELS
                {
                    let vname = solid
                        .body
                        .vertex_name(v)
                        .ok()
                        .flatten()
                        .map_or_else(|| format!("#{}", v.index()), str::to_owned);
                    return Some(json!({ "solid": solid.name, "vertex": vname }));
                }
            }
            return Some(json!({ "solid": solid.name, "edge": name }));
        }
        let ray = self.camera.ray_through(x, y, w, h, bounds.as_ref())?;
        let hit = self.scene.pick(&ray)?;
        let solid = self
            .solids()
            .iter()
            .find(|s| s.scene_id == Some(hit.body))?;
        let face = *solid.faces.get(hit.triangle)?;
        let name = solid
            .body
            .face_name(face)
            .ok()
            .flatten()
            .map_or_else(|| format!("#{}", face.index()), str::to_owned);
        Some(json!({ "solid": solid.name, "face": name }))
    }
}

fn measure_at(s: &mut Session, p: &Value) -> CmdResult {
    let (x, y, w, h) = params::pixel(p)?;
    let from = match p.get("from") {
        None | Some(Value::Null) => None,
        Some(v) => Some(resolve(s, v, "from")?),
    };
    let Some(r) = s.entity_at(x, y, w, h) else {
        return Ok(json!({ "entity": null }));
    };
    let here = resolve(s, &r, "entity")?;
    let mut out = match &from {
        Some(f) => {
            let mut m = measure_targets(s, f, Some(&here))?;
            m["entity"] = r;
            m
        }
        None => {
            let mut m = measure_targets(s, &here, None)?;
            m["entity"] = r;
            m
        }
    };
    if out.get("entity").is_none() {
        out["entity"] = Value::Null;
    }
    Ok(out)
}

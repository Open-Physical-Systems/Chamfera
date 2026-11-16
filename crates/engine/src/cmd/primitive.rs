//! Modelling quick wins: the primitives (`solid.box`, `solid.cylinder`, `solid.sphere`,
//! `solid.torus`, `solid.cone`), `solid.scale` and `solid.align` (timeline features),
//! `body.material` (a physical material with its density, stored per feature's bodies)
//! and `inspect.check` (the kernel's validity check of bodies).

use super::CommandSpec;
use super::feature::{expression, feature_reply};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{
    FeatureKind, Move, MoveTransform, Operation, PhysicalMaterial, Primitive, PrimitiveAxis,
    PrimitiveShape, Scale,
};
use chamfera_geom::Point3;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "body.material",
        label: "Physical Material",
        params: "body: string or integer (optional) - the body's name or scene id; default the selected body\n\
                 preset: string (optional) - steel, stainless_steel, cast_iron, aluminium, brass, copper, \
                 titanium, abs, pla, nylon, acrylic, polycarbonate, rubber, glass, wood_oak, water\n\
                 density: number or string (optional) - g/cm³, or with a unit (\"7850 kg/m3\")\n\
                 name: string (optional) - the material's name with a typed density\n\
                 clear: boolean (optional) - remove the material\n\
                 Saved with the design for every body of the feature that made it; inspect.mass uses it.",
        mutates: true,
        run: material,
    },
    CommandSpec {
        id: "inspect.check",
        label: "Check Bodies",
        params: "bodies: list of strings (optional) - body names; default every body\n\
                 Replies per body with `valid`, the kernel's `error` (topology, geometry, orientation, \
                 enclosed volume), counts, genus and Euler characteristic.",
        mutates: false,
        run: check,
    },
    CommandSpec {
        id: "solid.align",
        label: "Align",
        params: "from: object (required) - the entity to move: {solid, face|edge|vertex} (as inspect.measure)\n\
                 to: object (required) - the entity to meet: {solid, face|edge|vertex} or {point: [x, y, z]}\n\
                 body: string (optional) - the body to move; default the one `from` is on\n\
                 copy: boolean (optional) - keep the original and move a copy\n\
                 Points, straight edges and axes (cylinder or cone faces, circle centres) and flat faces \
                 meet by the shortest move; adds a Move feature.",
        mutates: true,
        run: align,
    },
    CommandSpec {
        id: "solid.box",
        label: "Box",
        params: "length, width, height: string or number (required) - sizes (plain = mm); centred on the origin across its base\n\
                 origin, axis, operation: as solid.cylinder",
        mutates: true,
        run: make_box,
    },
    CommandSpec {
        id: "solid.cone",
        label: "Cone",
        params: "radius: string or number (required) - the base radius\n\
                 top_radius: string or number (optional) - default 0 (pointed)\n\
                 height: string or number (required)\n\
                 origin, axis, operation: as solid.cylinder",
        mutates: true,
        run: make_cone,
    },
    CommandSpec {
        id: "solid.cylinder",
        label: "Cylinder",
        params: "radius, height: string or number (required) - (plain = mm); its base circle on the origin\n\
                 origin: {x, y, z} (optional) - where it sits, mm; default 0, 0, 0\n\
                 axis: string (optional) - x, y or z (default): the direction of its height or axis\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: make_cylinder,
    },
    CommandSpec {
        id: "solid.scale",
        label: "Scale",
        params: "bodies: list of strings (optional) - body names; default the selected bodies\n\
                 factor: string or number (optional) - one factor for all three axes\n\
                 x, y, z: string or number (optional) - a factor per axis instead (default 1)\n\
                 origin: {x, y, z} (optional) - the point that stays put, mm; default 0, 0, 0\n\
                 Factors are 0.0001 to 10000; a non-uniform scale keeps curved faces only where they stay \
                 cylinders, cones, spheres or tori.",
        mutates: true,
        run: scale,
    },
    CommandSpec {
        id: "solid.sphere",
        label: "Sphere",
        params: "radius: string or number (required) - (plain = mm); centred on the origin\n\
                 origin, axis, operation: as solid.cylinder",
        mutates: true,
        run: make_sphere,
    },
    CommandSpec {
        id: "solid.torus",
        label: "Torus",
        params: "major_radius: string or number (required) - the radius of the tube's centre circle\n\
                 minor_radius: string or number (required) - the tube's radius, below major_radius\n\
                 origin, axis, operation: as solid.cylinder; centred on the origin, turned about the axis",
        mutates: true,
        run: make_torus,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn given(p: &Value, key: &str) -> bool {
    p.get(key).is_some_and(|v| !v.is_null())
}

/// A length expression under `key`, or an error naming it when it is required.
fn size(p: &Value, key: &str) -> Result<Option<String>, CommandError> {
    expression(p, key, "mm")
}

fn required(p: &Value, key: &str) -> Result<String, CommandError> {
    size(p, key)?.ok_or_else(|| bad(format!("`{key}` is required")))
}

fn point(p: &Value, key: &str) -> Result<Option<Point3>, CommandError> {
    if !given(p, key) {
        return Ok(None);
    }
    let v = super::feature::vec3(p, key)?;
    let q = Point3::new(v.x, v.y, v.z);
    if !q.is_valid() {
        return Err(bad(format!("`{key}` is outside the working range")));
    }
    Ok(Some(q))
}

fn axis(p: &Value) -> Result<Option<PrimitiveAxis>, CommandError> {
    params::opt_str(p, "axis")?
        .map(|a| {
            PrimitiveAxis::parse(a).ok_or_else(|| bad(format!("unknown axis {a:?}: use x, y or z")))
        })
        .transpose()
}

fn operation(p: &Value) -> Result<Option<Operation>, CommandError> {
    params::opt_str(p, "operation")?
        .map(|o| {
            Operation::parse(o).ok_or_else(|| {
                bad(format!(
                    "unknown operation {o:?}: use new_body, join, cut or intersect"
                ))
            })
        })
        .transpose()
}

/// Adds the feature, rebuilds and replies, or fails (the session rolls back).
fn add(s: &mut Session, kind: FeatureKind) -> CmdResult {
    let id = s.doc.add_feature(kind)?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn primitive(s: &mut Session, p: &Value, shape: PrimitiveShape) -> CmdResult {
    let prim = Primitive {
        shape,
        origin: point(p, "origin")?.unwrap_or(Point3::ORIGIN),
        axis: axis(p)?.unwrap_or_default(),
        operation: operation(p)?.unwrap_or_default(),
    };
    let values = s.doc.primitive_values(&prim)?;
    let mut r = add(s, FeatureKind::Primitive(prim))?;
    r["expected_volume"] = json!(values.volume());
    Ok(r)
}

fn make_box(s: &mut Session, p: &Value) -> CmdResult {
    let shape = PrimitiveShape::Box {
        length: required(p, "length")?,
        width: required(p, "width")?,
        height: required(p, "height")?,
    };
    primitive(s, p, shape)
}

fn make_cylinder(s: &mut Session, p: &Value) -> CmdResult {
    let shape = PrimitiveShape::Cylinder {
        radius: required(p, "radius")?,
        height: required(p, "height")?,
    };
    primitive(s, p, shape)
}

fn make_sphere(s: &mut Session, p: &Value) -> CmdResult {
    let shape = PrimitiveShape::Sphere {
        radius: required(p, "radius")?,
    };
    primitive(s, p, shape)
}

fn make_torus(s: &mut Session, p: &Value) -> CmdResult {
    let shape = PrimitiveShape::Torus {
        major_radius: required(p, "major_radius")?,
        minor_radius: required(p, "minor_radius")?,
    };
    primitive(s, p, shape)
}

fn make_cone(s: &mut Session, p: &Value) -> CmdResult {
    let shape = PrimitiveShape::Cone {
        radius: required(p, "radius")?,
        top_radius: size(p, "top_radius")?.unwrap_or_else(|| "0 mm".into()),
        height: required(p, "height")?,
    };
    primitive(s, p, shape)
}

/// A plain-number factor under `key`.
fn factor(p: &Value, key: &str) -> Result<Option<String>, CommandError> {
    expression(p, key, "")
}

/// `bodies` as given, or the selected bodies.
fn bodies(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    let list = match p.get("bodies") {
        None | Some(Value::Null) => s.selected_solid_names(),
        Some(Value::Array(list)) => {
            if list.len() > chamfera_doc::primitive::MAX_SCALE_BODIES {
                return Err(bad("too many bodies"));
            }
            list.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| bad("`bodies` must be a list of body names"))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        Some(_) => return Err(bad("`bodies` must be a list of body names")),
    };
    if list.is_empty() {
        return Err(bad("give `bodies` (body names) or select a body first"));
    }
    Ok(list)
}

/// The factors from `factor`, or `x`, `y`, `z` (each default 1); both is refused.
fn factors(p: &Value) -> Result<Option<[String; 3]>, CommandError> {
    let one = factor(p, "factor")?;
    let each = [factor(p, "x")?, factor(p, "y")?, factor(p, "z")?];
    match (one, each.iter().any(Option::is_some)) {
        (Some(_), true) => Err(bad("give either `factor` or `x`, `y`, `z`, not both")),
        (Some(f), false) => Ok(Some([f.clone(), f.clone(), f])),
        (None, true) => {
            let [x, y, z] = each;
            let d = || "1".to_owned();
            Ok(Some([
                x.unwrap_or_else(d),
                y.unwrap_or_else(d),
                z.unwrap_or_else(d),
            ]))
        }
        (None, false) => Ok(None),
    }
}

fn scale(s: &mut Session, p: &Value) -> CmdResult {
    let bodies = bodies(s, p)?;
    let [x, y, z] = factors(p)?.ok_or_else(|| bad("give `factor`, or `x`, `y` and `z`"))?;
    let origin = point(p, "origin")?.unwrap_or(Point3::ORIGIN);
    // The volume each body should have afterwards, for the reply.
    let before: Vec<(String, Option<f64>)> = bodies
        .iter()
        .map(|b| {
            let v = s
                .solids()
                .iter()
                .find(|x| x.name == *b)
                .and_then(|x| x.body.volume().ok());
            (b.clone(), v)
        })
        .collect();
    let sc = Scale {
        bodies,
        origin,
        x,
        y,
        z,
    };
    let f = s.doc.scale_factors(&sc)?;
    let mut r = add(s, FeatureKind::Scale(sc))?;
    r["factors"] = json!(f);
    r["volume_ratio"] = json!(f[0] * f[1] * f[2]);
    r["volumes_before"] = before
        .into_iter()
        .map(|(n, v)| (n, json!(v)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    Ok(r)
}

fn align(s: &mut Session, p: &Value) -> CmdResult {
    let obj = p
        .as_object()
        .ok_or_else(|| bad("parameters must be a JSON object"))?;
    let from = super::inspect::resolve(
        s,
        obj.get("from").ok_or_else(|| bad("`from` is required"))?,
        "from",
    )?;
    let to = super::inspect::resolve(
        s,
        obj.get("to").ok_or_else(|| bad("`to` is required"))?,
        "to",
    )?;
    let body_of = |t: &super::inspect::Target| t.solid.and_then(|i| s.solids().get(i));
    let a = {
        let b = body_of(&from).ok_or_else(|| bad("`from` must be on a body"))?;
        crate::primitive::anchor(&b.body, from.entity).map_err(bad)?
    };
    let b = match body_of(&to) {
        Some(x) => crate::primitive::anchor(&x.body, to.entity),
        None => crate::primitive::anchor(&chamfera_kernel::Body::new(), to.entity),
    }
    .map_err(bad)?;
    let moved = match params::opt_str(p, "body")? {
        Some(n) => n.to_owned(),
        None => body_of(&from)
            .map(|x| x.name.clone())
            .ok_or_else(|| bad("`from` must be on a body"))?,
    };
    if from.solid.is_some() && from.solid == to.solid && params::opt_str(p, "body")?.is_none() {
        return Err(bad(
            "`from` and `to` are on the same body: pick `to` on another body or a point",
        ));
    }
    let d = crate::primitive::align_translation(a, b).map_err(CommandError::Unavailable)?;
    let start = a.point();
    let copy = if given(p, "copy") {
        params::req_bool(p, "copy")?
    } else {
        false
    };
    let mut r = add(
        s,
        FeatureKind::Move(Move {
            bodies: vec![moved],
            transform: MoveTransform::PointToPoint {
                from: start,
                to: start + d,
            },
            copy,
        }),
    )?;
    r["translation"] = json!([d.x, d.y, d.z]);
    r["distance"] = json!(d.length());
    r["from_kind"] = json!(a.kind());
    r["to_kind"] = json!(b.kind());
    Ok(r)
}

fn material(s: &mut Session, p: &Value) -> CmdResult {
    let si = match p.get("body") {
        None | Some(Value::Null) => {
            let names = s.selected_solid_names();
            let first = names
                .first()
                .ok_or_else(|| bad("give `body` or select a body first"))?;
            s.solids()
                .iter()
                .position(|x| x.name == *first)
                .ok_or_else(|| bad("the selected body is gone"))?
        }
        Some(v) => super::inspect::find_solid(s, v, "body")?,
    };
    let (feature, name) = s
        .solids()
        .get(si)
        .map(|x| (x.feature, x.name.clone()))
        .ok_or_else(|| bad("that body is gone"))?;
    if given(p, "clear") && params::req_bool(p, "clear")? {
        if given(p, "preset") || given(p, "density") {
            return Err(bad("`clear` takes no `preset` or `density`"));
        }
        s.doc.materials.remove(&feature);
        return Ok(json!({ "body": name, "feature": feature, "material": null }));
    }
    let preset = params::opt_str(p, "preset")?;
    let m = match (preset, p.get("density").filter(|v| !v.is_null())) {
        (Some(_), Some(_)) => return Err(bad("give `preset` or `density`, not both")),
        (Some(key), None) => chamfera_doc::physical::library(key).ok_or_else(|| {
            bad(format!(
                "unknown material {:?} (see body.material's presets)",
                key.chars().take(40).collect::<String>()
            ))
        })?,
        (None, Some(v)) => {
            // g/cm³ as a plain number, or any density unit inspect.mass reads.
            let per_mm3 = match v {
                Value::String(t) => match super::inspect::density_with_unit(t) {
                    Some((d, Some("g"))) => d,
                    Some((d, Some("kg"))) => d * 1000.0,
                    Some((d, Some("lb"))) => d * 453.592_37,
                    _ => {
                        return Err(bad(
                            "`density` needs a unit such as g/cm3 or kg/m3 (a plain number is g/cm³)",
                        ));
                    }
                },
                _ => params::req_f64(p, "density")? / 1000.0,
            };
            PhysicalMaterial {
                name: params::opt_str(p, "name")?.unwrap_or("custom").to_owned(),
                density: per_mm3 * 1000.0,
            }
        }
        (None, None) => return Err(bad("give `preset`, `density` or `clear`")),
    };
    chamfera_doc::physical::check_one(&m)?;
    let reply_mass = s
        .solids()
        .get(si)
        .and_then(|x| x.body.volume().ok())
        .map(|v| v * m.grams_per_mm3());
    let out = json!({
        "body": name,
        "feature": feature,
        "material": m.name,
        "density": m.density,
        "density_unit": "g/cm3",
        "mass": reply_mass,
        "mass_unit": "g",
    });
    s.doc.materials.insert(feature, m);
    Ok(out)
}

fn check(s: &mut Session, p: &Value) -> CmdResult {
    let wanted: Option<Vec<String>> = match p.get("bodies") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => Some(
            list.iter()
                .take(10_000)
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| bad("`bodies` must be a list of body names"))
                })
                .collect::<Result<_, _>>()?,
        ),
        Some(_) => return Err(bad("`bodies` must be a list of body names")),
    };
    if let Some(w) = &wanted
        && let Some(missing) = w.iter().find(|n| !s.solids().iter().any(|x| x.name == **n))
    {
        return Err(bad(format!(
            "there is no body {:?}",
            missing.chars().take(64).collect::<String>()
        )));
    }
    let reports: Vec<Value> = s
        .solids()
        .iter()
        .filter(|x| wanted.as_ref().is_none_or(|w| w.contains(&x.name)))
        .map(|x| {
            let mut r = crate::primitive::check_report(&x.name, &x.body);
            r["body"] = json!(x.scene_id);
            r
        })
        .collect();
    let invalid = reports
        .iter()
        .filter(|r| r["valid"] == json!(false))
        .count();
    Ok(json!({
        "checked": reports.len(),
        "invalid": invalid,
        "bodies": reports,
    }))
}

/// The keys `feature.edit` takes for a primitive or a scale.
const EDIT_KEYS: [&str; 17] = [
    "feature",
    "suppressed",
    "origin",
    "axis",
    "operation",
    "length",
    "width",
    "height",
    "radius",
    "top_radius",
    "major_radius",
    "minor_radius",
    "bodies",
    "factor",
    "x",
    "y",
    "z",
];

/// `feature.edit` of a primitive (sizes, origin, axis, operation) or a scale (bodies,
/// factors, origin).
pub(super) fn edit(s: &mut Session, id: u32, p: &Value) -> CmdResult {
    if let Some(obj) = p.as_object()
        && let Some(k) = obj.keys().find(|k| !EDIT_KEYS.contains(&k.as_str()))
    {
        return Err(bad(format!(
            "`{}` does not belong to a primitive or a scale",
            k.chars().take(40).collect::<String>()
        )));
    }
    let mut kind = s.doc.feature(id)?.kind.clone();
    match &mut kind {
        FeatureKind::Primitive(pr) => {
            if let Some(o) = point(p, "origin")? {
                pr.origin = o;
            }
            if let Some(a) = axis(p)? {
                pr.axis = a;
            }
            if let Some(o) = operation(p)? {
                pr.operation = o;
            }
            for (key, slot) in pr.shape.sizes_mut() {
                if let Some(v) = size(p, key)? {
                    *slot = v;
                }
            }
            for key in [
                "length",
                "width",
                "height",
                "radius",
                "top_radius",
                "major_radius",
                "minor_radius",
            ] {
                if given(p, key) && !pr.shape.sizes_mut().iter().any(|(k, _)| *k == key) {
                    return Err(bad(format!(
                        "a {} has no {key}",
                        pr.shape.label().to_lowercase()
                    )));
                }
            }
            if ["bodies", "factor", "x", "y", "z"]
                .iter()
                .any(|k| given(p, k))
            {
                return Err(bad("bodies and factors belong to scales"));
            }
        }
        FeatureKind::Scale(sc) => {
            if given(p, "bodies") {
                sc.bodies = bodies(s, p)?;
            }
            match factor(p, "factor")? {
                Some(_) if ["x", "y", "z"].iter().any(|k| given(p, k)) => {
                    return Err(bad("give either `factor` or `x`, `y`, `z`, not both"));
                }
                Some(f) => {
                    sc.x = f.clone();
                    sc.y = f.clone();
                    sc.z = f;
                }
                None => {
                    for (key, slot) in [("x", &mut sc.x), ("y", &mut sc.y), ("z", &mut sc.z)] {
                        if let Some(v) = factor(p, key)? {
                            *slot = v;
                        }
                    }
                }
            }
            if let Some(o) = point(p, "origin")? {
                sc.origin = o;
            }
        }
        _ => return Err(bad("not a primitive or a scale")),
    }
    s.doc.check_feature(&kind)?;
    s.doc.feature_mut(id)?.kind = kind;
    if given(p, "suppressed") {
        s.doc
            .set_suppressed(id, params::req_bool(p, "suppressed")?)?;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

//! `surface.*`: surface bodies (open shells) on the timeline: swept surfaces without
//! caps, patches, stitches and trims. Thickening a surface is `solid.thicken`.

use super::CommandSpec;
use super::feature::{expression, feature_reply, ids, seeds};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::surface::MAX_SURFACE_ITEMS;
use chamfera_doc::{
    FeatureKind, SurfaceExtrude, SurfaceFeature, SurfacePatch, SurfaceRevolve, SurfaceShapes,
    SurfaceStitch, SurfaceSweep, SurfaceTrim, TrimKeep, TrimTool,
};
use serde_json::Value;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "surface.extrude",
        label: "Surface Extrude",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (optional) - a point inside each closed profile; its sides are swept\n\
                 curves: list of integers (optional) - open sketch curves joined end to end, in order\n\
                 distance: string or number (required) - such as 10 or \"10 mm\"; negative goes against the normal\n\
                 symmetric: boolean (optional) - half the distance each way",
        mutates: true,
        run: extrude,
    },
    CommandSpec {
        id: "surface.patch",
        label: "Patch",
        params: "sketch: integer, profiles: list of {x, y} (optional) - closed sketch profiles to fill\n\
                 edges: list of strings (optional) - a closed, flat loop of model edges of one body instead",
        mutates: true,
        run: patch,
    },
    CommandSpec {
        id: "surface.revolve",
        label: "Surface Revolve",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (optional) - closed profiles\n\
                 curves: list of integers (optional) - open sketch curves joined end to end\n\
                 axis: integer (required unless axis_feature) - a line of the sketch\n\
                 axis_feature: integer (optional) - a construction axis in the sketch plane\n\
                 angle: string or number (optional) - default 360 deg; a plain number is degrees",
        mutates: true,
        run: revolve,
    },
    CommandSpec {
        id: "surface.stitch",
        label: "Stitch",
        params: "bodies: list of strings (optional) - surface bodies that share edges; default the selected bodies",
        mutates: true,
        run: stitch,
    },
    CommandSpec {
        id: "surface.sweep",
        label: "Surface Sweep",
        params: "sketch: integer (required)\n\
                 profiles: list of {x, y} (optional) - closed profiles\n\
                 curves: list of integers (optional) - open sketch curves joined end to end\n\
                 path_sketch: integer (required) - the sketch of the path\n\
                 path_curves: list of integers (required) - lines and arcs of the path",
        mutates: true,
        run: sweep,
    },
    CommandSpec {
        id: "surface.trim",
        label: "Trim",
        params: "body: string (optional) - the surface body; default the selected body\n\
                 plane: string, integer or object (optional) - xy, xz, yz, a construction plane id or {feature, face}\n\
                 tool: string (optional) - a body to trim with instead: a closed solid or a flat one-face surface\n\
                 keep: string (optional) - below (default: behind the plane, outside a solid) or above (in front, inside)",
        mutates: true,
        run: trim,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn given(p: &Value, key: &str) -> bool {
    p.get(key).is_some_and(|v| !v.is_null())
}

fn names(p: &Value, key: &str) -> Result<Option<Vec<String>>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) => {
            if list.len() > MAX_SURFACE_ITEMS {
                return Err(bad(format!("at most {MAX_SURFACE_ITEMS} {key}")));
            }
            list.iter()
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

fn shapes(p: &Value) -> Result<SurfaceShapes, CommandError> {
    let sketch = params::req_id(p, "sketch")?;
    let profiles = if given(p, "profiles") {
        seeds(p)?
    } else {
        Vec::new()
    };
    let curves = if given(p, "curves") {
        ids(p, "curves")?
    } else {
        Vec::new()
    };
    if profiles.is_empty() && curves.is_empty() {
        return Err(bad("give `profiles` (closed) or `curves` (open), or both"));
    }
    Ok(SurfaceShapes {
        sketch,
        profiles,
        curves,
    })
}

fn angle(p: &Value) -> Result<Option<String>, CommandError> {
    expression(p, "angle", "deg")
}

fn keep(p: &Value) -> Result<Option<TrimKeep>, CommandError> {
    match params::opt_str(p, "keep")? {
        None => Ok(None),
        Some(k) => TrimKeep::parse(k)
            .map(Some)
            .ok_or_else(|| bad(format!("unknown keep {k:?}: use below or above"))),
    }
}

fn trim_tool(p: &Value) -> Result<Option<TrimTool>, CommandError> {
    match (given(p, "plane"), given(p, "tool")) {
        (true, true) => Err(bad("give either `plane` or `tool`, not both")),
        (true, false) => Ok(Some(TrimTool::Plane {
            plane: super::construct::plane_ref(p.get("plane").unwrap_or(&Value::Null))?,
        })),
        (false, true) => Ok(Some(TrimTool::Body {
            body: params::req_str(p, "tool")?.to_owned(),
        })),
        (false, false) => Ok(None),
    }
}

fn add(s: &mut Session, sf: SurfaceFeature) -> CmdResult {
    let id = s.doc.add_feature(FeatureKind::Surface(sf))?;
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn extrude(s: &mut Session, p: &Value) -> CmdResult {
    let shapes = shapes(p)?;
    let distance = expression(p, "distance", "mm")?.ok_or_else(|| bad("`distance` is required"))?;
    let symmetric = if given(p, "symmetric") {
        params::req_bool(p, "symmetric")?
    } else {
        false
    };
    add(
        s,
        SurfaceFeature::Extrude(SurfaceExtrude {
            shapes,
            distance,
            symmetric,
        }),
    )
}

fn revolve(s: &mut Session, p: &Value) -> CmdResult {
    let shapes = shapes(p)?;
    let axis_feature = if given(p, "axis_feature") {
        Some(params::req_id(p, "axis_feature")?)
    } else {
        None
    };
    let axis = match axis_feature {
        Some(_) => 0,
        None => params::req_id(p, "axis")?,
    };
    add(
        s,
        SurfaceFeature::Revolve(SurfaceRevolve {
            shapes,
            axis,
            axis_feature,
            angle: angle(p)?.unwrap_or_else(|| "360 deg".into()),
        }),
    )
}

fn sweep(s: &mut Session, p: &Value) -> CmdResult {
    let shapes = shapes(p)?;
    let path_sketch = params::req_id(p, "path_sketch")?;
    let path = ids(p, "path_curves")?;
    add(
        s,
        SurfaceFeature::Sweep(SurfaceSweep {
            shapes,
            path_sketch,
            path,
        }),
    )
}

fn patch(s: &mut Session, p: &Value) -> CmdResult {
    let edges = names(p, "edges")?.unwrap_or_default();
    let patch = if edges.is_empty() {
        if !given(p, "sketch") {
            return Err(bad("give `sketch` and `profiles`, or `edges`"));
        }
        SurfacePatch {
            sketch: Some(params::req_id(p, "sketch")?),
            profiles: seeds(p)?,
            edges,
        }
    } else {
        if given(p, "sketch") || given(p, "profiles") {
            return Err(bad("give either `sketch` and `profiles`, or `edges`, not both"));
        }
        SurfacePatch {
            sketch: None,
            profiles: Vec::new(),
            edges,
        }
    };
    add(s, SurfaceFeature::Patch(patch))
}

fn stitch(s: &mut Session, p: &Value) -> CmdResult {
    let bodies = match names(p, "bodies")? {
        Some(b) => b,
        None => s.selected_solid_names(),
    };
    if bodies.is_empty() {
        return Err(bad("give `bodies` or select the surfaces first"));
    }
    add(s, SurfaceFeature::Stitch(SurfaceStitch { bodies }))
}

fn trim(s: &mut Session, p: &Value) -> CmdResult {
    let body = match params::opt_str(p, "body")? {
        Some(b) => b.to_owned(),
        None => s
            .selected_solid_names()
            .first()
            .cloned()
            .ok_or_else(|| bad("give `body` or select the surface first"))?,
    };
    let tool = trim_tool(p)?.ok_or_else(|| bad("give a `plane` or a `tool` body"))?;
    add(
        s,
        SurfaceFeature::Trim(SurfaceTrim {
            body,
            tool,
            keep: keep(p)?.unwrap_or_default(),
        }),
    )
}

/// The keys `feature.edit` takes for surface features.
const KEYS: [&str; 15] = [
    "feature",
    "suppressed",
    "profiles",
    "curves",
    "distance",
    "symmetric",
    "axis",
    "axis_feature",
    "angle",
    "path_curves",
    "edges",
    "bodies",
    "body",
    "plane",
    "tool",
];

/// `feature.edit` for a surface feature: the values its command takes (and `keep`).
pub(super) fn edit(s: &mut Session, id: u32, p: &Value) -> CmdResult {
    if let Some(obj) = p.as_object() {
        if let Some(k) = obj
            .keys()
            .find(|k| !KEYS.contains(&k.as_str()) && k.as_str() != "keep")
        {
            return Err(bad(format!("`{k}` does not belong to a surface feature")));
        }
    }
    let FeatureKind::Surface(mut sf) = s.doc.feature(id)?.kind.clone() else {
        return Err(bad("not a surface feature"));
    };
    let wrong = |k: &str| bad(format!("`{k}` does not belong to this surface feature"));
    let set_shapes = |sh: &mut SurfaceShapes| -> Result<(), CommandError> {
        if given(p, "profiles") {
            sh.profiles = seeds(p)?;
        }
        if given(p, "curves") {
            sh.curves = ids(p, "curves")?;
        }
        Ok(())
    };
    let no = |keys: &[&str]| -> Result<(), CommandError> {
        match keys.iter().find(|k| given(p, k)) {
            Some(k) => Err(wrong(k)),
            None => Ok(()),
        }
    };
    match &mut sf {
        SurfaceFeature::Extrude(e) => {
            no(&["axis", "axis_feature", "angle", "path_curves", "edges", "bodies", "body", "plane", "tool", "keep"])?;
            set_shapes(&mut e.shapes)?;
            if let Some(d) = expression(p, "distance", "mm")? {
                e.distance = d;
            }
            if given(p, "symmetric") {
                e.symmetric = params::req_bool(p, "symmetric")?;
            }
        }
        SurfaceFeature::Revolve(r) => {
            no(&["distance", "symmetric", "path_curves", "edges", "bodies", "body", "plane", "tool", "keep"])?;
            set_shapes(&mut r.shapes)?;
            if given(p, "axis") {
                r.axis = params::req_id(p, "axis")?;
            }
            if given(p, "axis_feature") {
                r.axis_feature = Some(params::req_id(p, "axis_feature")?);
            }
            if let Some(a) = angle(p)? {
                r.angle = a;
            }
        }
        SurfaceFeature::Sweep(w) => {
            no(&["distance", "symmetric", "axis", "axis_feature", "angle", "edges", "bodies", "body", "plane", "tool", "keep"])?;
            set_shapes(&mut w.shapes)?;
            if given(p, "path_curves") {
                w.path = ids(p, "path_curves")?;
            }
        }
        SurfaceFeature::Patch(pa) => {
            no(&["curves", "distance", "symmetric", "axis", "axis_feature", "angle", "path_curves", "bodies", "body", "plane", "tool", "keep"])?;
            if given(p, "profiles") {
                pa.profiles = seeds(p)?;
            }
            if let Some(e) = names(p, "edges")? {
                pa.edges = e;
            }
        }
        SurfaceFeature::Stitch(st) => {
            no(&["profiles", "curves", "distance", "symmetric", "axis", "axis_feature", "angle", "path_curves", "edges", "body", "plane", "tool", "keep"])?;
            if let Some(b) = names(p, "bodies")? {
                st.bodies = b;
            }
        }
        SurfaceFeature::Trim(t) => {
            no(&["profiles", "curves", "distance", "symmetric", "axis", "axis_feature", "angle", "path_curves", "edges", "bodies"])?;
            if let Some(b) = params::opt_str(p, "body")? {
                t.body = b.to_owned();
            }
            if let Some(tool) = trim_tool(p)? {
                t.tool = tool;
            }
            if let Some(k) = keep(p)? {
                t.keep = k;
            }
        }
    }
    let kind = FeatureKind::Surface(sf);
    s.doc.check_feature(&kind)?;
    let suppressed = if given(p, "suppressed") {
        Some(params::req_bool(p, "suppressed")?)
    } else {
        None
    };
    let f = s.doc.feature_mut(id)?;
    f.kind = kind;
    if let Some(v) = suppressed {
        f.suppressed = v;
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

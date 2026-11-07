//! `solid.combine`, `solid.split_body`, `solid.split_face`, `solid.thicken` and
//! `solid.remove`: body tools on the timeline.

use super::CommandSpec;
use super::feature::{expression, feature_reply};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::body_tools::MAX_BODY_TOOL_ITEMS;
use chamfera_doc::{
    Combine, FeatureKind, Operation, RemoveBodies, SplitBody, SplitFace, SplitFaceTool, Thicken,
    ThickenDirection,
};
use serde_json::Value;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "solid.combine",
        label: "Combine",
        params: "target: string (optional) - the body to change; default the first selected body\n\
                 tools: list of strings (optional) - the tool bodies; default the other selected bodies\n\
                 operation: string (optional) - join (default), cut or intersect\n\
                 keep_tools: boolean (optional) - keep the tool bodies (default false)",
        mutates: true,
        run: combine,
    },
    CommandSpec {
        id: "solid.remove",
        label: "Remove",
        params: "bodies: list of strings (optional) - body names; default the selected bodies",
        mutates: true,
        run: remove,
    },
    CommandSpec {
        id: "solid.split_body",
        label: "Split Body",
        params: "bodies: list of strings (optional) - body names; default the selected bodies\n\
                 plane: string, integer or object (required) - xy, xz or yz; a construction plane id; \
                 or {feature, face} for a flat face",
        mutates: true,
        run: split_body,
    },
    CommandSpec {
        id: "solid.split_face",
        label: "Split Face",
        params: "faces: list of strings (optional) - flat faces, names like \"2:top\"; default the selected face\n\
                 plane: string, integer or object (optional) - xy, xz, yz, a construction plane id or {feature, face}\n\
                 sketch, line: integer (optional) - a sketch line instead, carried square to its sketch",
        mutates: true,
        run: split_face,
    },
    CommandSpec {
        id: "solid.thicken",
        label: "Thicken",
        params: "faces: list of strings (optional) - flat or cylindrical faces; default the selected face\n\
                 thickness: string or number (required) - such as 3 or \"3 mm\"; negative goes inward\n\
                 sides: string (optional) - one_side (default), symmetric or both_sides\n\
                 operation: string (optional) - new_body (default), join, cut or intersect",
        mutates: true,
        run: thicken,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn given(p: &Value, key: &str) -> bool {
    p.get(key).is_some_and(|v| !v.is_null())
}

/// A list of names under `key`, or `None` when it is left out.
fn names(p: &Value, key: &str) -> Result<Option<Vec<String>>, CommandError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) => {
            if list.len() > MAX_BODY_TOOL_ITEMS {
                return Err(bad(format!("at most {MAX_BODY_TOOL_ITEMS} {key}")));
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

/// `bodies`, or the selected bodies.
fn bodies(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    let list = match names(p, "bodies")? {
        Some(l) => l,
        None => s.selected_solid_names(),
    };
    if list.is_empty() {
        return Err(bad("give `bodies` (body names) or select a body first"));
    }
    Ok(list)
}

/// `faces`, or the selected face.
fn faces(s: &Session, p: &Value) -> Result<Vec<String>, CommandError> {
    let list = match names(p, "faces")? {
        Some(l) => l,
        None => s.selected_face().into_iter().collect(),
    };
    if list.is_empty() {
        return Err(bad(
            "give `faces` (names like \"2:top\") or select a face first",
        ));
    }
    Ok(list)
}

fn operation(p: &Value) -> Result<Option<Operation>, CommandError> {
    match params::opt_str(p, "operation")? {
        None => Ok(None),
        Some(o) => Operation::parse(o).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown operation {o:?}: use new_body, join, cut or intersect"
            ))
        }),
    }
}

fn side(p: &Value) -> Result<Option<ThickenDirection>, CommandError> {
    match params::opt_str(p, "sides")? {
        None => Ok(None),
        Some(o) => ThickenDirection::parse(o).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown sides {o:?}: use one_side, symmetric or both_sides"
            ))
        }),
    }
}

fn keep_tools(p: &Value) -> Result<Option<bool>, CommandError> {
    if given(p, "keep_tools") {
        Ok(Some(params::req_bool(p, "keep_tools")?))
    } else {
        Ok(None)
    }
}

/// What a split face cuts with: `plane`, or `sketch` and `line`.
fn split_tool(p: &Value) -> Result<Option<SplitFaceTool>, CommandError> {
    match (given(p, "plane"), given(p, "line")) {
        (true, true) => Err(bad("give either `plane` or `sketch` and `line`, not both")),
        (true, false) => {
            let plane = super::construct::plane_ref(p.get("plane").unwrap_or(&Value::Null))?;
            Ok(Some(SplitFaceTool::Plane { plane }))
        }
        (false, true) => Ok(Some(SplitFaceTool::SketchLine {
            sketch: params::req_id(p, "sketch")?,
            curve: params::req_id(p, "line")?,
        })),
        (false, false) => Ok(None),
    }
}

/// Adds the feature, rebuilds and replies, or fails (the session rolls back).
fn add(s: &mut Session, kind: FeatureKind) -> CmdResult {
    // What the tool's faces are now (rung K8), so they follow splits and renames.
    let mut faces: Vec<String> = match &kind {
        FeatureKind::SplitFace(f) => f.faces.clone(),
        FeatureKind::Thicken(t) => t.faces.clone(),
        _ => Vec::new(),
    };
    let plane = match &kind {
        FeatureKind::SplitBody(b) => Some(&b.plane),
        FeatureKind::SplitFace(chamfera_doc::SplitFace {
            tool: chamfera_doc::SplitFaceTool::Plane { plane },
            ..
        }) => Some(plane),
        _ => None,
    };
    if let Some(chamfera_doc::PlaneRef::Face(r)) = plane {
        faces.push(crate::naming::face_ref_name(r));
    }
    let id = s.doc.add_feature(kind)?;
    for f in &faces {
        s.record_signature(id, crate::naming::Kind::Face, f, None);
    }
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn combine(s: &mut Session, p: &Value) -> CmdResult {
    let selected = s.selected_solid_names();
    let target = match params::opt_str(p, "target")? {
        Some(t) => t.to_owned(),
        None => selected
            .first()
            .cloned()
            .ok_or_else(|| bad("give `target` or select the target body first"))?,
    };
    let tools = match names(p, "tools")? {
        Some(t) => t,
        None => selected.into_iter().filter(|n| *n != target).collect(),
    };
    if tools.is_empty() {
        return Err(bad(
            "give `tools` or select the tool bodies after the target",
        ));
    }
    add(
        s,
        FeatureKind::Combine(Combine {
            target,
            tools,
            operation: operation(p)?.unwrap_or(Operation::Join),
            keep_tools: keep_tools(p)?.unwrap_or(false),
        }),
    )
}

fn remove(s: &mut Session, p: &Value) -> CmdResult {
    let bodies = bodies(s, p)?;
    add(s, FeatureKind::Remove(RemoveBodies { bodies }))
}

fn split_body(s: &mut Session, p: &Value) -> CmdResult {
    let bodies = bodies(s, p)?;
    let plane = super::construct::plane_ref(
        p.get("plane")
            .filter(|v| !v.is_null())
            .ok_or_else(|| bad("`plane` is required"))?,
    )?;
    add(s, FeatureKind::SplitBody(SplitBody { bodies, plane }))
}

fn split_face(s: &mut Session, p: &Value) -> CmdResult {
    let faces = faces(s, p)?;
    let tool = split_tool(p)?.ok_or_else(|| bad("give a `plane`, or a `sketch` and `line`"))?;
    add(s, FeatureKind::SplitFace(SplitFace { faces, tool }))
}

fn thicken(s: &mut Session, p: &Value) -> CmdResult {
    let faces = faces(s, p)?;
    let thickness =
        expression(p, "thickness", "mm")?.ok_or_else(|| bad("`thickness` is required"))?;
    add(
        s,
        FeatureKind::Thicken(Thicken {
            faces,
            thickness,
            direction: side(p)?.unwrap_or_default(),
            operation: operation(p)?.unwrap_or(Operation::NewBody),
        }),
    )
}

/// The keys `feature.edit` takes for body tools only.
const OWN_KEYS: [&str; 6] = ["target", "tools", "keep_tools", "bodies", "sides", "line"];

/// `feature.edit` for body tools: a combine's target, tools and keep flag; the bodies of
/// a split or remove; a split's plane or line; a thicken's side. (Operations, thickness
/// and faces are applied by `feature.edit` itself.) These keys on other features are
/// refused.
pub(super) fn edit_body_tool(kind: &mut FeatureKind, p: &Value) -> Result<(), CommandError> {
    let any = OWN_KEYS.iter().any(|k| given(p, k));
    let split = matches!(kind, FeatureKind::SplitBody(_) | FeatureKind::SplitFace(_));
    if !kind.is_body_tool() {
        if any {
            return Err(bad(
                "target, tools and keep_tools belong to combines; bodies to splits and removes; \
                 sides to thickens; line to split faces",
            ));
        }
        return Ok(());
    }
    let wrong = |k: &str| bad(format!("{k} does not belong to this feature"));
    let target = params::opt_str(p, "target")?.map(str::to_owned);
    let tools = names(p, "tools")?;
    let keep = keep_tools(p)?;
    let bodies = names(p, "bodies")?;
    let new_side = side(p)?;
    let mut tool = if split { split_tool(p)? } else { None };
    if let FeatureKind::Combine(c) = kind {
        if let Some(t) = target {
            c.target = t;
        }
        if let Some(t) = tools {
            c.tools = t;
        }
        if let Some(k) = keep {
            c.keep_tools = k;
        }
    } else if target.is_some() || tools.is_some() || keep.is_some() {
        return Err(wrong("target, tools and keep_tools"));
    }
    match kind {
        FeatureKind::SplitBody(sb) => {
            if let Some(b) = bodies {
                sb.bodies = b;
            }
            match tool.take() {
                Some(SplitFaceTool::Plane { plane }) => sb.plane = plane,
                Some(SplitFaceTool::SketchLine { .. }) => return Err(wrong("line")),
                None => {}
            }
        }
        FeatureKind::Remove(r) => {
            if let Some(b) = bodies {
                r.bodies = b;
            }
        }
        _ if bodies.is_some() => return Err(wrong("bodies")),
        _ => {}
    }
    if let FeatureKind::SplitFace(sf) = kind
        && let Some(t) = tool
    {
        sf.tool = t;
    }
    if let Some(d) = new_side {
        let FeatureKind::Thicken(t) = kind else {
            return Err(wrong("sides"));
        };
        t.direction = d;
    }
    Ok(())
}

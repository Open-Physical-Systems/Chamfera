//! `sheet.*`: sheet metal rules, base flanges, flanges and flat patterns.

use super::CommandSpec;
use super::feature::{expression, feature_reply};
use crate::{CmdResult, CommandError, Session, params};
use chamfera_doc::{
    FeatureKind, FlangePosition, FlangeSide, ReliefShape, SheetBase, SheetFlange, SheetRule,
    SheetUnfold,
};
use chamfera_geom::Point2;
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "sheet.base",
        label: "Base Flange",
        params: "sketch: integer (required)\n\
                 profile: {x, y} (required) - a point inside the closed profile, in sketch mm\n\
                 rule: string (optional) - the sheet metal rule; default the first one (a 2 mm \"Default\" rule is made if there is none)\n\
                 reversed: boolean (optional) - thicken against the sketch plane's normal",
        mutates: true,
        run: base,
    },
    CommandSpec {
        id: "sheet.export_flat",
        label: "Export Flat Pattern (DXF)",
        params: "path: string (required) - the .dxf file to write\n\
                 feature: integer (optional) - the flat pattern; default the last one",
        mutates: false,
        run: export_flat,
    },
    CommandSpec {
        id: "sheet.flange",
        label: "Flange",
        params: "edge: string (optional) - a straight edge of the sheet, a name from solid.edges such as \"2:top 1\"\n\
                 pick: {x, y, width, height} (optional) - the edge under this pixel, instead\n\
                 height: string or number (required) - from the outer virtual sharp to the end, such as 30 or \"30 mm\"\n\
                 angle: string or number (optional) - the bend angle, default 90 deg; a plain number is degrees\n\
                 position: string (optional) - inside (default: keeps the outline), outside or bend_outside\n\
                 side: string (optional) - same (default: toward the face the edge borders) or opposite\n\
                 offset_start, offset_end: string or number (optional) - stop short of the edge's ends (bend reliefs are cut there)",
        mutates: true,
        run: flange,
    },
    CommandSpec {
        id: "sheet.flat_pattern",
        label: "Flat Pattern",
        params: "base: integer (optional) - the base flange; default the last one\n\
                 (replies with the flat length, width, area and each bend's allowance)",
        mutates: true,
        run: flat_pattern,
    },
    CommandSpec {
        id: "sheet.rule",
        label: "Sheet Metal Rule",
        params: "name: string (required) - the rule's name; a new name makes a rule\n\
                 thickness, bend_radius: number (optional) - mm\n\
                 k_factor: number (optional) - 0 to 1\n\
                 relief: string (optional) - rectangular, obround or rip\n\
                 relief_width, relief_depth: number (optional) - mm\n\
                 remove: boolean (optional) - delete the rule (refused while a base flange uses it)",
        mutates: true,
        run: rule,
    },
    CommandSpec {
        id: "sheet.rules",
        label: "List Sheet Metal Rules",
        params: "(none)",
        mutates: false,
        run: rules,
    },
];

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::BadParams(msg.into())
}

fn rule_json(r: &SheetRule) -> Value {
    json!({
        "name": r.name,
        "thickness": r.thickness,
        "bend_radius": r.bend_radius,
        "k_factor": r.k_factor,
        "relief": r.relief.key(),
        "relief_width": r.relief_width,
        "relief_depth": r.relief_depth,
    })
}

fn rules_reply(s: &Session) -> Value {
    let list: Vec<Value> = s.doc.sheet_rules.iter().map(rule_json).collect();
    json!({ "rules": list, "errors": s.feature_errors() })
}

fn rules(s: &mut Session, p: &Value) -> CmdResult {
    params::none(p)?;
    Ok(rules_reply(s))
}

fn rule(s: &mut Session, p: &Value) -> CmdResult {
    let name = params::req_str(p, "name")?.to_owned();
    let remove = match p.get("remove") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "remove")?,
    };
    if remove {
        s.doc.remove_sheet_rule(&name)?;
        s.rebuild();
        return Ok(rules_reply(s));
    }
    let mut r = s
        .doc
        .sheet_rule(&name)
        .cloned()
        .unwrap_or_else(|_| SheetRule::default_named(&name));
    let set = |key: &str, slot: &mut f64| -> Result<(), CommandError> {
        if let Some(v) = params::opt_f64(p, key)? {
            *slot = v;
        }
        Ok(())
    };
    set("thickness", &mut r.thickness)?;
    set("bend_radius", &mut r.bend_radius)?;
    set("k_factor", &mut r.k_factor)?;
    set("relief_width", &mut r.relief_width)?;
    set("relief_depth", &mut r.relief_depth)?;
    if let Some(k) = params::opt_str(p, "relief")? {
        r.relief = ReliefShape::parse(k).ok_or_else(|| {
            bad(format!(
                "unknown relief {k:?}: use rectangular, obround or rip"
            ))
        })?;
    }
    s.doc.set_sheet_rule(r)?;
    s.rebuild();
    Ok(rules_reply(s))
}

fn point(v: &Value) -> Result<Point2, CommandError> {
    let pt = Point2::new(params::req_f64(v, "x")?, params::req_f64(v, "y")?);
    if !pt.is_valid() {
        return Err(bad("the profile point is outside the working range"));
    }
    Ok(pt)
}

/// Runs `id`'s rebuild and replies, or fails with its error (the session rolls back).
fn built_reply(s: &mut Session, id: u32) -> CmdResult {
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(feature_reply(s, id))
}

fn base(s: &mut Session, p: &Value) -> CmdResult {
    let sketch = params::req_id(p, "sketch")?;
    let profile = point(
        p.get("profile")
            .ok_or_else(|| bad("`profile` is required: a point {x, y} inside the profile"))?,
    )?;
    let rule = match params::opt_str(p, "rule")? {
        Some(r) => r.to_owned(),
        None => match s.doc.sheet_rules.first() {
            Some(r) => r.name.clone(),
            None => {
                s.doc.set_sheet_rule(SheetRule::default_named("Default"))?;
                "Default".to_owned()
            }
        },
    };
    let reversed = match p.get("reversed") {
        None | Some(Value::Null) => false,
        Some(_) => params::req_bool(p, "reversed")?,
    };
    let id = s.doc.add_feature(FeatureKind::SheetBase(SheetBase {
        sketch,
        profiles: vec![profile],
        rule,
        reversed,
    }))?;
    built_reply(s, id)
}

fn angle(p: &Value) -> Result<Option<String>, CommandError> {
    expression(p, "angle", "deg")
}

fn position(p: &Value) -> Result<Option<FlangePosition>, CommandError> {
    match params::opt_str(p, "position")? {
        None => Ok(None),
        Some(k) => FlangePosition::parse(k).map(Some).ok_or_else(|| {
            bad(format!(
                "unknown position {k:?}: use inside, outside or bend_outside"
            ))
        }),
    }
}

fn side(p: &Value) -> Result<Option<FlangeSide>, CommandError> {
    match params::opt_str(p, "side")? {
        None => Ok(None),
        Some(k) => FlangeSide::parse(k)
            .map(Some)
            .ok_or_else(|| bad(format!("unknown side {k:?}: use same or opposite"))),
    }
}

/// The edge a flange goes on: a name, or the one under a pixel.
fn flange_edge(s: &Session, p: &Value) -> Result<Option<String>, CommandError> {
    match (p.get("edge"), p.get("pick")) {
        (None | Some(Value::Null), None | Some(Value::Null)) => Ok(None),
        (Some(Value::String(e)), None | Some(Value::Null)) => Ok(Some(e.clone())),
        (Some(_), None | Some(Value::Null)) => Err(bad("`edge` must be an edge name")),
        (None | Some(Value::Null), Some(pick)) => {
            let (x, y, w, h) = params::pixel(pick)?;
            s.edge_at(x, y, w, h)
                .map(|(_, _, r)| Some(r))
                .ok_or_else(|| {
                    CommandError::Unavailable("there is no edge under that pixel".into())
                })
        }
        _ => Err(bad("give either `edge` or `pick`, not both")),
    }
}

fn flange(s: &mut Session, p: &Value) -> CmdResult {
    let edge = flange_edge(s, p)?.ok_or_else(|| bad("give `edge` or `pick`"))?;
    let height = expression(p, "height", "mm")?.ok_or_else(|| bad("`height` is required"))?;
    let f = SheetFlange {
        edge,
        height,
        angle: angle(p)?.unwrap_or_else(|| "90 deg".into()),
        position: position(p)?.unwrap_or_default(),
        side: side(p)?.unwrap_or_default(),
        offset_start: expression(p, "offset_start", "mm")?,
        offset_end: expression(p, "offset_end", "mm")?,
    };
    let edge = f.edge.clone();
    let id = s.doc.add_feature(FeatureKind::SheetFlange(f))?;
    s.record_signature(id, crate::naming::Kind::Edge, &edge, None);
    built_reply(s, id)
}

/// The flat pattern's measures: its extent in the base's sketch plane (`length` along the
/// plane's x, `width` along y), area and bends.
fn flat_reply(s: &Session, id: u32) -> Value {
    let Some(f) = s.flat_patterns().get(&id) else {
        return json!({ "feature": id, "error": s.feature_errors().get(&id) });
    };
    let bends: Vec<Value> = f
        .bends
        .iter()
        .map(|b| {
            json!({
                "angle": b.angle.to_degrees(),
                "radius": b.radius,
                "allowance": b.allowance,
                "length": b.length,
            })
        })
        .collect();
    json!({
        "feature": id,
        "base": f.base,
        "length": f.max.x - f.min.x,
        "width": f.max.y - f.min.y,
        "min": [f.min.x, f.min.y],
        "max": [f.max.x, f.max.y],
        "area": f.area,
        "thickness": f.thickness,
        "bends": bends,
        "outline_edges": f.outline.len(),
    })
}

fn flat_pattern(s: &mut Session, p: &Value) -> CmdResult {
    let base = match p.get("base") {
        None | Some(Value::Null) => s
            .doc
            .features
            .iter()
            .rev()
            .find(|f| matches!(f.kind, FeatureKind::SheetBase(_)))
            .map(|f| f.id)
            .ok_or_else(|| CommandError::Unavailable("there is no base flange to unfold".into()))?,
        Some(_) => params::req_id(p, "base")?,
    };
    let existing = s
        .doc
        .features
        .iter()
        .find(|f| matches!(&f.kind, FeatureKind::SheetUnfold(u) if u.base == base))
        .map(|f| f.id);
    let id = match existing {
        Some(id) => id,
        None => s
            .doc
            .add_feature(FeatureKind::SheetUnfold(SheetUnfold { base }))?,
    };
    s.rebuild();
    if let Some(e) = s.feature_errors().get(&id) {
        return Err(CommandError::Unavailable(e.clone()));
    }
    Ok(flat_reply(s, id))
}

fn export_flat(s: &mut Session, p: &Value) -> CmdResult {
    let path = params::req_str(p, "path")?;
    let id = match p.get("feature") {
        None | Some(Value::Null) => {
            s.flat_patterns()
                .keys()
                .next_back()
                .copied()
                .ok_or_else(|| {
                    CommandError::Unavailable(
                        "there is no flat pattern: make one with sheet.flat_pattern".into(),
                    )
                })?
        }
        Some(_) => params::req_id(p, "feature")?,
    };
    let f = s.flat_patterns().get(&id).ok_or_else(|| {
        CommandError::Unavailable(format!("feature {id} is not a built flat pattern"))
    })?;
    let text = crate::sheet::flat_dxf(f);
    std::fs::write(path, text.as_bytes()).map_err(chamfera_io::IoError::from)?;
    Ok(json!({ "path": path, "feature": id, "entities": f.outline.len() }))
}

/// The keys `feature.edit` hands to [`edit`].
pub(super) const EDIT_KEYS: [&str; 7] = [
    "height",
    "position",
    "side",
    "offset_start",
    "offset_end",
    "edge",
    "rule",
];

/// `feature.edit` for sheet metal features: a flange's height, angle, position, side,
/// offsets and edge; a base flange's rule and direction (`reversed`).
pub(super) fn edit(s: &Session, kind: &mut FeatureKind, p: &Value) -> Result<(), CommandError> {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    match kind {
        FeatureKind::SheetFlange(f) => {
            if given("rule") || given("reversed") {
                return Err(bad("rule and reversed belong to base flanges"));
            }
            if let Some(h) = expression(p, "height", "mm")? {
                f.height = h;
            }
            if let Some(a) = angle(p)? {
                f.angle = a;
            }
            if let Some(x) = position(p)? {
                f.position = x;
            }
            if let Some(x) = side(p)? {
                f.side = x;
            }
            if given("offset_start") {
                f.offset_start = expression(p, "offset_start", "mm")?;
            }
            if given("offset_end") {
                f.offset_end = expression(p, "offset_end", "mm")?;
            }
            if let Some(e) = flange_edge(s, p)? {
                f.edge = e;
            }
        }
        FeatureKind::SheetBase(b) => {
            if EDIT_KEYS.iter().any(|k| *k != "rule" && given(k)) {
                return Err(bad(
                    "height, position, side, offsets and edge belong to flanges",
                ));
            }
            if let Some(r) = params::opt_str(p, "rule")? {
                b.rule = r.to_owned();
            }
            if given("reversed") {
                b.reversed = params::req_bool(p, "reversed")?;
            }
        }
        _ => {
            if EDIT_KEYS.iter().any(|k| given(k)) || given("reversed") {
                return Err(bad(
                    "height, position, side, offsets, edge, rule and reversed belong to sheet metal features",
                ));
            }
        }
    }
    Ok(())
}

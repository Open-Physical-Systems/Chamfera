//! Version history of the open design (`version.*`): kept in the history folder beside
//! the design's file (`chamfera_io::history`), so every path-free command works on the
//! open design's own history and never anywhere else.
//!
//! A comparison builds both versions in scratch sessions, lists what differs (parameters,
//! timeline items added, removed or changed, the volume) and can show both versions'
//! bodies over each other (`Scene::compare`).

use crate::{CommandError, Session};
use chamfera_doc::Document;
use chamfera_io::history::{self, Kind, Limits, Summary};
use chamfera_render::{CompareMesh, CompareSide};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Most differences a comparison lists per kind (the counts are always exact).
pub const MAX_DIFFS: usize = 500;

/// Whether saves keep versions, and how many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySettings {
    /// Each `design.save` keeps a version.
    pub enabled: bool,
    pub limits: Limits,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            limits: Limits::default(),
        }
    }
}

/// The total volume of a session's solids, if each could be worked out (surfaces count
/// as none).
fn volume_of(s: &Session) -> Option<f64> {
    let mut total = 0.0;
    for solid in s.solids() {
        if let Ok(v) = solid.body.volume() {
            total += v;
        }
    }
    total.is_finite().then_some(total)
}

/// What the history list shows of the session's design.
pub fn summary_of(s: &Session) -> Summary {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Summary {
        volume: volume_of(s),
        features: count(s.doc.features.len()),
        sketches: count(s.doc.sketches.len()),
        bodies: count(s.solids().len()),
        parameters: count(s.doc.parameters.len()),
    }
}

/// The open design's history folder, or why it has none.
pub fn folder(s: &Session) -> Result<PathBuf, CommandError> {
    let path = s.path.as_ref().ok_or_else(|| {
        CommandError::Unavailable(
            "the design has no file yet: save it first, and its versions are kept beside it".into(),
        )
    })?;
    Ok(history::folder_for(path)?)
}

/// Keeps a version of the open design in its history.
pub fn keep(s: &Session, kind: Kind, note: &str) -> Result<history::Added, CommandError> {
    let dir = folder(s)?;
    Ok(history::add(
        &dir,
        &s.doc,
        kind,
        note,
        summary_of(s),
        s.history.limits,
    )?)
}

/// A scratch session with `doc` built: its solids and scene bodies.
pub(crate) fn built(doc: Document) -> Session {
    let mut s = Session::new();
    s.replace_document(doc);
    s.rebuild();
    s
}

/// What a design looks like built: its scene, for a thumbnail.
pub fn preview_scene(doc: Document) -> chamfera_render::Scene {
    built(doc).scene
}

/// One side of a comparison: a version id, or the open design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Version(u32),
    Current,
}

impl Side {
    pub fn parse(v: Option<&Value>, key: &str) -> Result<Side, CommandError> {
        let bad = || {
            CommandError::BadParams(format!(
                "`{key}` must be a version id (a whole number) or \"current\""
            ))
        };
        match v {
            None | Some(Value::Null) => Ok(Side::Current),
            Some(Value::String(t)) if t == "current" => Ok(Side::Current),
            Some(Value::Number(n)) => n
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .filter(|id| *id > 0)
                .map(Side::Version)
                .ok_or_else(bad),
            Some(_) => Err(bad()),
        }
    }

    fn label(self) -> Value {
        match self {
            Side::Version(id) => json!(id),
            Side::Current => json!("current"),
        }
    }
}

/// The design of one side.
fn design_of(s: &Session, side: Side) -> Result<Document, CommandError> {
    match side {
        Side::Current => Ok(s.doc.clone()),
        Side::Version(id) => Ok(history::read(&folder(s)?, id)?),
    }
}

/// A parameter as compared: its expression (or value) and value.
fn parameter_table(doc: &Document) -> BTreeMap<String, (String, Option<f64>)> {
    let mut out = BTreeMap::new();
    for (name, p) in &doc.parameters {
        let text = p
            .expression
            .clone()
            .unwrap_or_else(|| format!("{}", p.value));
        out.insert(name.clone(), (text, Some(p.value)));
    }
    for m in doc.model_parameter_list() {
        let text = match (&m.expression, m.plain_value) {
            (Some(e), _) => e.clone(),
            (None, Some(v)) => format!("{v}"),
            (None, None) => String::new(),
        };
        let value = doc.parameter_quantity(&m.name).ok().map(|q| q.value);
        out.entry(m.name.clone()).or_insert((text, value));
    }
    out
}

/// The timeline's items, by id: (kind, name, JSON).
fn items(doc: &Document) -> BTreeMap<u32, (&'static str, String, Value)> {
    let mut out = BTreeMap::new();
    for f in &doc.sketches {
        let v = serde_json::to_value(f).unwrap_or(Value::Null);
        out.insert(f.id, ("sketch", f.name.clone(), v));
    }
    for f in &doc.features {
        let v = serde_json::to_value(f).unwrap_or(Value::Null);
        out.insert(f.id, ("feature", f.name.clone(), v));
    }
    out
}

/// The top-level fields that differ between two JSON objects.
fn changed_fields(a: &Value, b: &Value) -> Vec<String> {
    let empty = Map::new();
    let (ma, mb) = (
        a.as_object().unwrap_or(&empty),
        b.as_object().unwrap_or(&empty),
    );
    let mut keys: Vec<&String> = ma.keys().chain(mb.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| ma.get(*k) != mb.get(*k))
        .map(|k| k.to_string())
        .collect()
}

/// What differs between two designs, A before B. `volume` is each side's built volume.
pub fn diff(a: &Document, b: &Document, volume: (Option<f64>, Option<f64>)) -> Value {
    let (pa, pb) = (parameter_table(a), parameter_table(b));
    let mut parameters = Vec::new();
    let mut names: Vec<&String> = pa.keys().chain(pb.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let (x, y) = (pa.get(name), pb.get(name));
        let change = match (x, y) {
            // What drives it changed: the text. A value that only follows another
            // parameter (a dimension driven by `w`) is not a change of its own.
            (Some(x), Some(y)) if x.0 == y.0 => continue,
            (Some(_), Some(_)) => "changed",
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (None, None) => continue,
        };
        parameters.push(json!({
            "name": name,
            "change": change,
            "from": x.map(|p| p.0.clone()),
            "to": y.map(|p| p.0.clone()),
            "from_value": x.and_then(|p| p.1),
            "to_value": y.and_then(|p| p.1),
        }));
    }
    let (ia, ib) = (items(a), items(b));
    let entry = |id: &u32, kind: &str, name: &str| json!({ "id": id, "kind": kind, "name": name });
    let added: Vec<Value> = ib
        .iter()
        .filter(|(id, _)| !ia.contains_key(id))
        .map(|(id, (k, n, _))| entry(id, k, n))
        .collect();
    let removed: Vec<Value> = ia
        .iter()
        .filter(|(id, _)| !ib.contains_key(id))
        .map(|(id, (k, n, _))| entry(id, k, n))
        .collect();
    let changed: Vec<Value> = ia
        .iter()
        .filter_map(|(id, (k, n, va))| {
            let (_, nb, vb) = ib.get(id)?;
            (va != vb).then(|| {
                let mut e = entry(id, k, nb);
                e["fields"] = json!(changed_fields(va, vb));
                if n != nb {
                    e["renamed_from"] = json!(n);
                }
                e
            })
        })
        .collect();
    let order_changed = a.timeline_order() != b.timeline_order();
    let delta = match volume {
        (Some(x), Some(y)) => Some(y - x),
        _ => None,
    };
    let count = parameters.len() + added.len() + removed.len() + changed.len();
    let cap = |v: Vec<Value>| v.into_iter().take(MAX_DIFFS).collect::<Vec<_>>();
    json!({
        "parameters": cap(parameters),
        "features": {
            "added": cap(added),
            "removed": cap(removed),
            "changed": cap(changed),
        },
        "timeline_order_changed": order_changed,
        "volume_a": volume.0,
        "volume_b": volume.1,
        "volume_delta": delta,
        "differences": count + usize::from(order_changed),
        "same": count == 0 && !order_changed && a == b,
    })
}

/// The shown solids of a built session, as comparison meshes.
fn meshes(s: &Session, side: CompareSide) -> Vec<CompareMesh> {
    s.solids()
        .iter()
        .filter_map(|solid| solid.scene_id.and_then(|id| s.scene.body(id)))
        .filter(|b| b.visible && !b.mesh.triangles.is_empty())
        .map(|b| CompareMesh {
            side,
            mesh: b.mesh.clone(),
        })
        .collect()
}

/// Compares two sides; with `show`, puts both versions' bodies in the viewport.
pub fn compare(s: &mut Session, a: Side, b: Side, show: bool) -> Result<Value, CommandError> {
    let da = design_of(s, a)?;
    let db = design_of(s, b)?;
    let sa = built(da.clone());
    let sb = built(db.clone());
    let mut reply = diff(&da, &db, (volume_of(&sa), volume_of(&sb)));
    reply["a"] = a.label();
    reply["b"] = b.label();
    s.scene.compare.clear();
    if show {
        s.scene.compare.extend(meshes(&sa, CompareSide::A));
        s.scene.compare.extend(meshes(&sb, CompareSide::B));
    }
    reply["shown"] = json!(!s.scene.compare.is_empty());
    Ok(reply)
}

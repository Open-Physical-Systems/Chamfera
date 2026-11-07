//! Model parameters: every value a feature or a sketch dimension is driven by, listed
//! beside the user parameters with a name of its own.
//!
//! - A feature's value is named after the feature and the field: `Extrude1.distance`,
//!   `Hole1.diameter`, `Plane1.p2_x`. A sketch dimension is named after its sketch and
//!   constraint: `Sketch1.d13`.
//! - Any expression can use these names, so `Extrude2.distance` may be
//!   `Extrude1.distance / 2`. They are evaluated on demand, in dependency order; a loop
//!   is an error that names each step.
//! - A model parameter can be renamed (`thickness`): the new name is stored by the
//!   parameter's key (`f2.distance`, `s1.d13`) in [`Document::model_parameters`], and
//!   every expression that used the old name is rewritten. Renaming a feature or sketch
//!   rewrites the expressions that use its default names the same way.
//! - Model parameters carry a comment and a favourite mark, as user parameters do.

use crate::construct::{Construction, PointRef};
use crate::expr::{self, Dim, Quantity};
use crate::surface::SurfaceFeature;
use crate::{DocError, Document, FeatureKind, MoveTransform, PatternLayout, Unit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest comment on a parameter.
pub const MAX_COMMENT_LEN: usize = 512;
/// Most steps one parameter may refer through (a chain `a` → `b` → … ).
pub const MAX_CHAIN: usize = 512;

const L: Dim = Dim::LENGTH;
const A: Dim = Dim::ANGLE;
const N: Dim = Dim::NONE;

fn is_false(b: &bool) -> bool {
    !*b
}

/// What the user set on a model parameter: a name of its own, a comment, a favourite mark.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub favourite: bool,
}

impl ModelMeta {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.comment.is_empty() && !self.favourite
    }
}

/// One model parameter, as the parameter table shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelParameter {
    /// Stable key: `f{feature}.{field}` or `s{sketch}.d{constraint}`.
    pub key: String,
    /// The name expressions use.
    pub name: String,
    /// The feature or sketch that holds it.
    pub owner: u32,
    pub owner_name: String,
    /// `distance`, `diameter`, `d13`.
    pub field: String,
    pub dim: Dim,
    /// What drives it; `None` for a sketch dimension that holds a plain value.
    pub expression: Option<String>,
    /// A plain sketch dimension's value (mm or radians).
    pub plain_value: Option<f64>,
    pub comment: String,
    pub favourite: bool,
}

fn push_opt<'a>(
    out: &mut Vec<(String, Dim, &'a mut String)>,
    name: &str,
    d: Dim,
    v: &'a mut Option<String>,
) {
    if let Some(s) = v {
        out.push((name.to_owned(), d, s));
    }
}

fn push_point<'a>(out: &mut Vec<(String, Dim, &'a mut String)>, prefix: &str, p: &'a mut PointRef) {
    if let PointRef::Coordinates { x, y, z } = p {
        out.push((format!("{prefix}x"), L, x));
        out.push((format!("{prefix}y"), L, y));
        out.push((format!("{prefix}z"), L, z));
    }
}

impl FeatureKind {
    /// Every expression the feature holds, with its field name and the unit it needs.
    pub fn slots_mut(&mut self) -> Vec<(String, Dim, &mut String)> {
        let mut out: Vec<(String, Dim, &mut String)> = Vec::new();
        let o = &mut out;
        match self {
            FeatureKind::Extrude(e) => {
                o.push(("distance".into(), L, &mut e.distance));
                push_opt(o, "distance2", L, &mut e.distance2);
                push_opt(o, "start_offset", L, &mut e.start_offset);
                push_opt(o, "taper", A, &mut e.taper);
                if let Some(t) = &mut e.thin {
                    o.push(("thickness".into(), L, &mut t.thickness));
                }
            }
            FeatureKind::Revolve(r) => o.push(("angle".into(), A, &mut r.angle)),
            FeatureKind::Thread(t) => {
                o.push(("pitch".into(), L, &mut t.pitch));
                push_opt(o, "length", L, &mut t.length);
            }
            FeatureKind::Blend(b) => {
                o.push(("size".into(), L, &mut b.size));
                push_opt(o, "distance2", L, &mut b.distance2);
                push_opt(o, "angle", A, &mut b.angle);
            }
            FeatureKind::Hole(h) => {
                o.push(("diameter".into(), L, &mut h.diameter));
                push_opt(o, "depth", L, &mut h.depth);
                push_opt(o, "cb_diameter", L, &mut h.cb_diameter);
                push_opt(o, "cb_depth", L, &mut h.cb_depth);
                push_opt(o, "cs_diameter", L, &mut h.cs_diameter);
                push_opt(o, "cs_angle", A, &mut h.cs_angle);
                push_opt(o, "tip_angle", A, &mut h.tip_angle);
            }
            FeatureKind::Move(m) => match &mut m.transform {
                MoveTransform::Translate { dx, dy, dz } => {
                    o.push(("dx".into(), L, dx));
                    o.push(("dy".into(), L, dy));
                    o.push(("dz".into(), L, dz));
                }
                MoveTransform::Rotate { angle, .. } => o.push(("angle".into(), A, angle)),
                MoveTransform::PointToPoint { .. } => {}
            },
            FeatureKind::Shell(s) => o.push(("thickness".into(), L, &mut s.thickness)),
            FeatureKind::SheetFlange(f) => {
                o.push(("height".into(), L, &mut f.height));
                o.push(("angle".into(), A, &mut f.angle));
                push_opt(o, "offset_start", L, &mut f.offset_start);
                push_opt(o, "offset_end", L, &mut f.offset_end);
            }
            FeatureKind::OffsetFace(f) => o.push(("distance".into(), L, &mut f.distance)),
            FeatureKind::MoveFace(m) => {
                o.push(("distance".into(), L, &mut m.distance));
                if let Some(t) = &mut m.turn {
                    o.push(("angle".into(), A, &mut t.angle));
                }
            }
            FeatureKind::Thicken(t) => o.push(("thickness".into(), L, &mut t.thickness)),
            FeatureKind::Pattern(p) => {
                o.push(("count".into(), N, &mut p.count));
                match &mut p.layout {
                    PatternLayout::Linear { spacing, .. } => {
                        o.push(("spacing".into(), L, spacing));
                    }
                    PatternLayout::Circular { angle, .. } => o.push(("angle".into(), A, angle)),
                }
            }
            FeatureKind::Surface(sf) => match sf {
                SurfaceFeature::Extrude(e) => o.push(("distance".into(), L, &mut e.distance)),
                SurfaceFeature::Revolve(r) => o.push(("angle".into(), A, &mut r.angle)),
                SurfaceFeature::Sweep(_)
                | SurfaceFeature::Patch(_)
                | SurfaceFeature::Stitch(_)
                | SurfaceFeature::Trim(_) => {}
            },
            FeatureKind::Construct(c) => match c {
                Construction::PlaneOffset { distance, .. } => {
                    o.push(("distance".into(), L, distance));
                }
                Construction::PlaneAngle { angle, .. } => o.push(("angle".into(), A, angle)),
                Construction::PlaneThreePoints { points } => {
                    for (i, p) in points.iter_mut().enumerate() {
                        push_point(o, &format!("p{}_", i + 1), p);
                    }
                }
                Construction::AxisTwoPoints { points } => {
                    for (i, p) in points.iter_mut().enumerate() {
                        push_point(o, &format!("p{}_", i + 1), p);
                    }
                }
                Construction::PlaneTangent { point, .. } => push_point(o, "", point),
                Construction::Point { at } => push_point(o, "", at),
                Construction::PlaneMidplane { .. }
                | Construction::AxisCylinder { .. }
                | Construction::AxisEdge { .. }
                | Construction::AxisTwoPlanes { .. } => {}
            },
            FeatureKind::Sweep(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::SheetBase(_)
            | FeatureKind::SheetUnfold(_)
            | FeatureKind::DeleteFace(_)
            | FeatureKind::Combine(_)
            | FeatureKind::SplitBody(_)
            | FeatureKind::SplitFace(_)
            | FeatureKind::Remove(_) => {}
        }
        out
    }

    /// [`FeatureKind::slots_mut`], read only.
    pub fn slots(&self) -> Vec<(String, Dim, String)> {
        let mut k = self.clone();
        k.slots_mut()
            .into_iter()
            .map(|(n, d, s)| (n, d, s.clone()))
            .collect()
    }
}

/// `text` as a name part: letters, digits and `_` only, not starting with a digit.
fn name_part(text: &str) -> String {
    let s: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if s.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        format!("_{s}")
    } else {
        s
    }
}

/// True for a model parameter's own name: a plain parameter name.
pub fn is_valid_model_name(name: &str) -> bool {
    crate::is_valid_name(name)
}

/// How a value is reached during evaluation.
#[derive(Debug, Clone)]
enum Entry {
    Value(Quantity),
    /// An expression, its unit, and the largest magnitude allowed (user parameters).
    Expr {
        src: String,
        dim: Dim,
        limit: Option<f64>,
    },
}

/// Every name an expression may use (user and model parameters), evaluated on demand.
pub(crate) struct Namespace {
    entries: BTreeMap<String, Entry>,
    pub(crate) values: BTreeMap<String, Quantity>,
    scale: f64,
}

/// The largest magnitude a user parameter of this unit may hold.
pub(crate) fn unit_limit(unit: Unit) -> f64 {
    match unit {
        Unit::Length => chamfera_geom::MAX_COORDINATE,
        Unit::Angle => 1e3 * std::f64::consts::TAU,
        Unit::Unitless => 1e9,
    }
}

impl Namespace {
    pub(crate) fn new(doc: &Document) -> Namespace {
        let mut entries = BTreeMap::new();
        for m in doc.model_parameter_list() {
            let e = match (&m.expression, m.plain_value) {
                (Some(src), _) => Entry::Expr {
                    src: src.clone(),
                    dim: m.dim,
                    limit: None,
                },
                (None, v) => Entry::Value(Quantity {
                    value: v.unwrap_or(0.0),
                    dim: m.dim,
                }),
            };
            entries.insert(m.name, e);
        }
        for (n, p) in &doc.parameters {
            let e = match doc.effective_expression(n) {
                Some(src) => Entry::Expr {
                    src: src.to_owned(),
                    dim: p.unit.dim(),
                    limit: Some(unit_limit(p.unit)),
                },
                None => Entry::Value(Quantity {
                    value: p.value,
                    dim: p.unit.dim(),
                }),
            };
            entries.insert(n.clone(), e);
        }
        Namespace {
            entries,
            values: BTreeMap::new(),
            scale: doc.length_scale(),
        }
    }

    /// Evaluates `name` and everything it refers to. An unknown name is left out (the
    /// expression that uses it reports it).
    pub(crate) fn resolve(&mut self, name: &str, path: &mut Vec<String>) -> Result<(), DocError> {
        if self.values.contains_key(name) {
            return Ok(());
        }
        if let Some(i) = path.iter().position(|n| n == name) {
            let mut cycle: Vec<String> = path.get(i..).unwrap_or_default().to_vec();
            cycle.push(name.to_owned());
            return Err(DocError::Cycle(cycle));
        }
        if path.len() >= MAX_CHAIN {
            return Err(DocError::BadValue {
                name: name.to_owned(),
                reason: format!("refers through more than {MAX_CHAIN} parameters"),
            });
        }
        let Some(entry) = self.entries.get(name).cloned() else {
            return Ok(());
        };
        let q = match entry {
            Entry::Value(q) => q,
            Entry::Expr { src, dim, limit } => {
                path.push(name.to_owned());
                for dep in expr::names_in(&src) {
                    self.resolve(&dep, path)?;
                }
                path.pop();
                let v =
                    expr::evaluate_as_in(&src, dim, self.scale, &self.values).map_err(|error| {
                        DocError::Expression {
                            name: name.to_owned(),
                            error,
                        }
                    })?;
                if let Some(limit) = limit
                    && v.abs() > limit
                {
                    return Err(DocError::BadValue {
                        name: name.to_owned(),
                        reason: format!("magnitude must be at most {limit}"),
                    });
                }
                Quantity { value: v, dim }
            }
        };
        self.values.insert(name.to_owned(), q);
        Ok(())
    }

    /// Evaluates an expression that is not itself a parameter (`what` names it in errors).
    pub(crate) fn eval(&mut self, what: &str, src: &str, dim: Dim) -> Result<f64, DocError> {
        let mut path = vec![what.to_owned()];
        for dep in expr::names_in(src) {
            self.resolve(&dep, &mut path)?;
        }
        expr::evaluate_as_in(src, dim, self.scale, &self.values).map_err(|error| {
            DocError::Expression {
                name: what.to_owned(),
                error,
            }
        })
    }

    /// The value of a parameter, evaluating it if needed.
    pub(crate) fn value(&mut self, name: &str) -> Result<Option<Quantity>, DocError> {
        self.resolve(name, &mut Vec::new())?;
        Ok(self.values.get(name).copied())
    }
}

impl Document {
    /// Every model parameter: sketch dimensions, then feature values, in design order.
    pub fn model_parameter_list(&self) -> Vec<ModelParameter> {
        let mut out = Vec::new();
        for s in &self.sketches {
            for (c, k) in s.sketch.constraints() {
                let Some(v) = k.value() else {
                    continue;
                };
                let dim = if k.kind_name() == "angle" { A } else { L };
                let expression = s.expressions.get(&c.0).cloned();
                out.push(ModelParameter {
                    key: format!("s{}.d{}", s.id, c.0),
                    name: format!("{}.d{}", name_part(&s.name), c.0),
                    owner: s.id,
                    owner_name: s.name.clone(),
                    field: format!("d{}", c.0),
                    dim,
                    plain_value: expression.is_none().then_some(v),
                    expression,
                    comment: String::new(),
                    favourite: false,
                });
            }
        }
        for f in &self.features {
            for (field, dim, src) in f.kind.slots() {
                out.push(ModelParameter {
                    key: format!("f{}.{field}", f.id),
                    name: format!("{}.{field}", name_part(&f.name)),
                    owner: f.id,
                    owner_name: f.name.clone(),
                    field,
                    dim,
                    expression: Some(src),
                    plain_value: None,
                    comment: String::new(),
                    favourite: false,
                });
            }
        }
        // Names of their own, then defaults made unique: the key, then `_` added.
        let mut taken: std::collections::BTreeSet<String> =
            self.parameters.keys().cloned().collect();
        for m in &mut out {
            if let Some(meta) = self.model_parameters.get(&m.key) {
                m.comment = meta.comment.clone();
                m.favourite = meta.favourite;
                if let Some(n) = &meta.name {
                    m.name = n.clone();
                    taken.insert(n.clone());
                }
            }
        }
        for m in &mut out {
            if self
                .model_parameters
                .get(&m.key)
                .is_some_and(|meta| meta.name.is_some())
            {
                continue;
            }
            if taken.contains(&m.name) {
                m.name = m.key.clone();
                while taken.contains(&m.name) {
                    m.name.push('_');
                }
            }
            taken.insert(m.name.clone());
        }
        out
    }

    /// The model parameter called `name`, if any.
    pub fn model_parameter(&self, name: &str) -> Option<ModelParameter> {
        self.model_parameter_list()
            .into_iter()
            .find(|m| m.name == name)
    }

    /// A parameter's expression as the design evaluates it: the active configuration's
    /// override, else its own.
    pub fn effective_expression(&self, name: &str) -> Option<&str> {
        if let Some(e) = self.configurations.active_override(name) {
            return Some(e);
        }
        self.parameters.get(name)?.expression.as_deref()
    }

    /// Evaluates one parameter (user or model) with everything it refers to.
    pub fn parameter_quantity(&self, name: &str) -> Result<Quantity, DocError> {
        let mut ns = Namespace::new(self);
        ns.value(name)?
            .ok_or_else(|| DocError::NoSuchParameter(name.to_owned()))
    }

    /// Every expression in the design, to rewrite a name: user parameters, sketch
    /// dimensions, feature values and configuration overrides.
    pub(crate) fn for_each_expression_mut(&mut self, mut f: impl FnMut(&mut String)) {
        for p in self.parameters.values_mut() {
            if let Some(e) = p.expression.as_mut() {
                f(e);
            }
        }
        for s in &mut self.sketches {
            for e in s.expressions.values_mut() {
                f(e);
            }
        }
        for feat in &mut self.features {
            for (_, _, e) in feat.kind.slots_mut() {
                f(e);
            }
        }
        for c in &mut self.configurations.list {
            for e in c.parameters.values_mut() {
                f(e);
            }
        }
    }

    /// Who uses `name` in an expression: parameters (by name), sketch dimensions and
    /// features (by their model parameter names), configurations.
    pub fn expression_users(&self, name: &str) -> Vec<String> {
        let uses = |e: &str| expr::names_in(e).iter().any(|x| x == name);
        let mut users: Vec<String> = self
            .parameters
            .iter()
            .filter(|(n, p)| n.as_str() != name && p.expression.as_deref().is_some_and(uses))
            .map(|(n, _)| n.clone())
            .collect();
        for m in self.model_parameter_list() {
            if m.name != name && m.expression.as_deref().is_some_and(uses) {
                users.push(m.name);
            }
        }
        for c in &self.configurations.list {
            if c.parameters.contains_key(name) || c.parameters.values().any(|e| uses(e)) {
                users.push(format!("configuration {}", c.name));
            }
        }
        users
    }

    /// Renames a user or model parameter and rewrites every expression that used the old
    /// name. Fails without changing the document.
    pub fn rename_parameter(&mut self, old: &str, new: &str) -> Result<(), DocError> {
        if old == new {
            return Ok(());
        }
        if !crate::is_valid_name(new) {
            return Err(DocError::BadName(new.to_owned()));
        }
        let models = self.model_parameter_list();
        if self.parameters.contains_key(new) || models.iter().any(|m| m.name == new) {
            return Err(DocError::BadValue {
                name: new.to_owned(),
                reason: "that name is taken by another parameter".into(),
            });
        }
        let before = self.clone();
        if let Some(p) = self.parameters.remove(old) {
            self.parameters.insert(new.to_owned(), p);
            for c in &mut self.configurations.list {
                if let Some(e) = c.parameters.remove(old) {
                    c.parameters.insert(new.to_owned(), e);
                }
            }
        } else if let Some(m) = models.iter().find(|m| m.name == old) {
            self.model_parameters.entry(m.key.clone()).or_default().name = Some(new.to_owned());
        } else {
            return Err(DocError::NoSuchParameter(old.to_owned()));
        }
        self.for_each_expression_mut(|e| *e = expr::rename_in(e, old, new));
        if let Err(e) = self.evaluate() {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Sets a model parameter's expression, then re-evaluates. Fails without changing the
    /// document.
    pub fn set_model_expression(&mut self, name: &str, expression: &str) -> Result<(), DocError> {
        if expression.chars().count() > expr::MAX_EXPRESSION_LEN {
            return Err(DocError::Expression {
                name: name.to_owned(),
                error: expr::ExprError::TooLong,
            });
        }
        let m = self
            .model_parameter(name)
            .ok_or_else(|| DocError::NoSuchParameter(name.to_owned()))?;
        let before = self.clone();
        if let Some(c) = m.key.strip_prefix(&format!("s{}.d", m.owner)) {
            let c: u32 = c
                .parse()
                .map_err(|_| DocError::NoSuchParameter(name.to_owned()))?;
            self.sketch_mut(m.owner)?
                .expressions
                .insert(c, expression.to_owned());
        } else {
            let f = self.feature_mut(m.owner)?;
            if let Some((_, _, e)) = f
                .kind
                .slots_mut()
                .into_iter()
                .find(|(field, _, _)| *field == m.field)
            {
                *e = expression.to_owned();
            }
        }
        // Its own value (catching a loop through it), then everything else.
        let result = self.parameter_quantity(name).and_then(|_| self.evaluate());
        if let Err(e) = result {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Sets the comment and favourite mark of a user or model parameter.
    pub fn annotate_parameter(
        &mut self,
        name: &str,
        comment: Option<&str>,
        favourite: Option<bool>,
    ) -> Result<(), DocError> {
        if let Some(c) = comment
            && (c.chars().count() > MAX_COMMENT_LEN
                || c.chars().any(|ch| ch.is_control() && ch != '\t'))
        {
            return Err(DocError::BadValue {
                name: name.to_owned(),
                reason: format!("a comment is at most {MAX_COMMENT_LEN} characters on one line"),
            });
        }
        if let Some(p) = self.parameters.get_mut(name) {
            if let Some(c) = comment {
                p.comment = c.to_owned();
            }
            if let Some(f) = favourite {
                p.favourite = f;
            }
            return Ok(());
        }
        let m = self
            .model_parameter(name)
            .ok_or_else(|| DocError::NoSuchParameter(name.to_owned()))?;
        let meta = self.model_parameters.entry(m.key.clone()).or_default();
        if let Some(c) = comment {
            meta.comment = c.to_owned();
        }
        if let Some(f) = favourite {
            meta.favourite = f;
        }
        if meta.is_empty() {
            self.model_parameters.remove(&m.key);
        }
        Ok(())
    }

    /// Creates or replaces many user parameters at once (`(name, expression, unit,
    /// comment)`), then evaluates once, so rows may refer to rows after them. Fails
    /// without changing the document.
    pub fn set_parameters(
        &mut self,
        rows: &[(String, String, Unit, String)],
    ) -> Result<(), DocError> {
        let before = self.clone();
        let result = (|| {
            for (name, expression, unit, comment) in rows {
                if !crate::is_valid_name(name) {
                    return Err(DocError::BadName(name.clone()));
                }
                if !self.parameters.contains_key(name) {
                    if self.parameters.len() >= crate::MAX_PARAMETERS {
                        return Err(DocError::TooManyParameters);
                    }
                    if self.model_parameter(name).is_some() {
                        return Err(DocError::BadValue {
                            name: name.clone(),
                            reason: "that name is taken by a model parameter".into(),
                        });
                    }
                }
                if expression.chars().count() > expr::MAX_EXPRESSION_LEN {
                    return Err(DocError::Expression {
                        name: name.clone(),
                        error: expr::ExprError::TooLong,
                    });
                }
                if comment.chars().count() > MAX_COMMENT_LEN
                    || comment.chars().any(|c| c.is_control() && c != '\t')
                {
                    return Err(DocError::BadValue {
                        name: name.clone(),
                        reason: format!(
                            "a comment is at most {MAX_COMMENT_LEN} characters on one line"
                        ),
                    });
                }
                let favourite = self.parameters.get(name).is_some_and(|p| p.favourite);
                self.parameters.insert(
                    name.clone(),
                    crate::Parameter {
                        value: 0.0,
                        unit: *unit,
                        expression: Some(expression.clone()),
                        comment: comment.clone(),
                        favourite,
                    },
                );
            }
            self.evaluate()
        })();
        if let Err(e) = result {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// The default model names of an item (sketch or feature), to follow its rename.
    pub(crate) fn default_model_names(&self, item: u32) -> BTreeMap<String, String> {
        self.model_parameter_list()
            .into_iter()
            .filter(|m| m.owner == item)
            .map(|m| (m.key, m.name))
            .collect()
    }

    /// After an item was renamed: expressions that used its old model names use the new.
    pub(crate) fn follow_model_renames(&mut self, old: &BTreeMap<String, String>) {
        let now: BTreeMap<String, String> = self
            .model_parameter_list()
            .into_iter()
            .map(|m| (m.key, m.name))
            .collect();
        for (key, was) in old {
            if let Some(is) = now.get(key)
                && is != was
            {
                let (was, is) = (was.clone(), is.clone());
                self.for_each_expression_mut(|e| *e = expr::rename_in(e, &was, &is));
            }
        }
    }

    /// Who uses the model parameters of an item, outside the item itself.
    pub(crate) fn model_users_of_item(&self, item: u32) -> Vec<String> {
        let mine: Vec<ModelParameter> = self
            .model_parameter_list()
            .into_iter()
            .filter(|m| m.owner == item)
            .collect();
        let mut users = Vec::new();
        for m in &mine {
            for u in self.expression_users(&m.name) {
                let own = mine.iter().any(|x| x.name == u);
                if !own && !users.contains(&u) {
                    users.push(u);
                }
            }
        }
        users
    }

    /// Checks model parameter metadata read from a file.
    pub(crate) fn check_model_meta(meta: &BTreeMap<String, ModelMeta>) -> Result<(), DocError> {
        if meta.len() > crate::MAX_PARAMETERS {
            return Err(DocError::TooManyParameters);
        }
        for (k, m) in meta {
            if k.chars().count() > 128 {
                return Err(DocError::Format("a model parameter key is too long".into()));
            }
            if m.name.as_deref().is_some_and(|n| !crate::is_valid_name(n)) {
                return Err(DocError::BadName(m.name.clone().unwrap_or_default()));
            }
            if m.comment.chars().count() > MAX_COMMENT_LEN {
                return Err(DocError::Format("a parameter comment is too long".into()));
            }
        }
        Ok(())
    }
}

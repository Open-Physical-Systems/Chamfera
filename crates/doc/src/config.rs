//! Configurations: a design table of named variants of one design.
//!
//! Each configuration overrides a set of user parameters (`w` = `40 mm` in "Small") and,
//! optionally, whether features are suppressed. One configuration (or none: the base
//! design) is active. The parameters keep their own expressions; while a configuration is
//! active the design evaluates its overrides instead ([`Document::effective_expression`])
//! and builds with its suppression ([`Document::is_suppressed`]), so switching back and
//! forth loses nothing.

use crate::{DocError, Document, Feature, expr};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most configurations one design may hold.
pub const MAX_CONFIGURATIONS: usize = 1000;
/// Longest configuration name.
pub const MAX_CONFIG_NAME_LEN: usize = 64;

/// One configuration: parameter overrides and feature suppression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration {
    pub name: String,
    /// User parameter name → the expression it takes in this configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    /// Feature id → whether it is suppressed in this configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub suppressed: BTreeMap<u32, bool>,
}

/// The design table and which row is active.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Configurations {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Configuration>,
    /// The active configuration's name; `None` builds the base design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadValue {
        name: "configuration".into(),
        reason: reason.into(),
    }
}

/// Checks a configuration name: 1 to 64 characters, no control characters.
pub fn check_config_name(name: &str) -> Result<String, DocError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_CONFIG_NAME_LEN {
        return Err(bad(format!(
            "a configuration name has 1 to {MAX_CONFIG_NAME_LEN} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(bad("a configuration name cannot hold control characters"));
    }
    Ok(name.to_owned())
}

impl Configurations {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.active.is_none()
    }

    pub fn get(&self, name: &str) -> Option<&Configuration> {
        self.list.iter().find(|c| c.name == name)
    }

    fn get_mut(&mut self, name: &str) -> Result<&mut Configuration, DocError> {
        self.list
            .iter_mut()
            .find(|c| c.name == name)
            .ok_or_else(|| bad(format!("no configuration named {name:?}")))
    }

    /// The active configuration, if any.
    pub fn active(&self) -> Option<&Configuration> {
        self.active.as_deref().and_then(|n| self.get(n))
    }

    /// The active configuration's override of a parameter.
    pub fn active_override(&self, parameter: &str) -> Option<&str> {
        self.active()?.parameters.get(parameter).map(String::as_str)
    }

    /// Every parameter some configuration overrides, sorted: the table's columns.
    pub fn columns(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .list
            .iter()
            .flat_map(|c| c.parameters.keys().cloned())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Every feature some configuration suppresses or unsuppresses, sorted.
    pub fn feature_columns(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .list
            .iter()
            .flat_map(|c| c.suppressed.keys().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl Document {
    /// Whether a feature is suppressed as built: the active configuration's choice, else
    /// the feature's own.
    pub fn is_suppressed(&self, f: &Feature) -> bool {
        self.configurations
            .active()
            .and_then(|c| c.suppressed.get(&f.id).copied())
            .unwrap_or(f.suppressed)
    }

    /// Suppresses a feature or brings it back: in the active configuration when that
    /// configuration decides it, else on the feature itself.
    pub fn set_suppressed(&mut self, id: u32, suppressed: bool) -> Result<(), DocError> {
        self.feature(id)?;
        if let Some(name) = self.configurations.active.clone()
            && let Ok(c) = self.configurations.get_mut(&name)
            && let Some(v) = c.suppressed.get_mut(&id)
        {
            *v = suppressed;
            return Ok(());
        }
        self.feature_mut(id)?.suppressed = suppressed;
        Ok(())
    }

    /// Adds a configuration, empty or copied from `from`.
    pub fn config_add(&mut self, name: &str, from: Option<&str>) -> Result<String, DocError> {
        let name = check_config_name(name)?;
        if self.configurations.get(&name).is_some() {
            return Err(bad(format!("a configuration named {name:?} exists")));
        }
        if self.configurations.list.len() >= MAX_CONFIGURATIONS {
            return Err(bad(format!(
                "a design holds at most {MAX_CONFIGURATIONS} configurations"
            )));
        }
        let mut c = match from {
            Some(f) => self
                .configurations
                .get(f)
                .cloned()
                .ok_or_else(|| bad(format!("no configuration named {f:?}")))?,
            None => Configuration {
                name: String::new(),
                parameters: BTreeMap::new(),
                suppressed: BTreeMap::new(),
            },
        };
        c.name = name.clone();
        self.configurations.list.push(c);
        Ok(name)
    }

    /// Sets (or, with `None`, clears) a configuration's override of a user parameter.
    /// The configuration must evaluate with it. Fails without changing the document.
    pub fn config_set(
        &mut self,
        config: &str,
        parameter: &str,
        expression: Option<&str>,
    ) -> Result<(), DocError> {
        if !self.parameters.contains_key(parameter) {
            return Err(if self.model_parameter(parameter).is_some() {
                bad(format!(
                    "{parameter} is a model parameter: configurations override user \
                     parameters, so drive it from one (such as `t`) and override that"
                ))
            } else {
                DocError::NoSuchParameter(parameter.to_owned())
            });
        }
        if let Some(e) = expression
            && e.chars().count() > expr::MAX_EXPRESSION_LEN
        {
            return Err(DocError::Expression {
                name: parameter.to_owned(),
                error: expr::ExprError::TooLong,
            });
        }
        let before = self.clone();
        let c = self.configurations.get_mut(config)?;
        match expression {
            Some(e) => {
                c.parameters
                    .insert(parameter.to_owned(), e.trim().to_owned());
            }
            None => {
                c.parameters.remove(parameter);
            }
        }
        // Check the configuration as it would build, then go back to the active one.
        let active = self.configurations.active.clone();
        self.configurations.active = Some(config.to_owned());
        let checked = self.evaluate();
        self.configurations.active = active;
        let result = checked.and_then(|()| self.evaluate());
        if let Err(e) = result {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Sets (or, with `None`, clears) whether a configuration suppresses a feature.
    pub fn config_suppress(
        &mut self,
        config: &str,
        feature: u32,
        suppressed: Option<bool>,
    ) -> Result<(), DocError> {
        self.feature(feature)?;
        let c = self.configurations.get_mut(config)?;
        match suppressed {
            Some(v) => {
                c.suppressed.insert(feature, v);
            }
            None => {
                c.suppressed.remove(&feature);
            }
        }
        Ok(())
    }

    /// Makes a configuration active (`None`: the base design) and re-evaluates. Fails
    /// without changing the document.
    pub fn config_activate(&mut self, config: Option<&str>) -> Result<(), DocError> {
        if let Some(n) = config
            && self.configurations.get(n).is_none()
        {
            return Err(bad(format!("no configuration named {n:?}")));
        }
        let before = self.clone();
        self.configurations.active = config.map(str::to_owned);
        if let Err(e) = self.evaluate() {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    /// Deletes a configuration; deleting the active one makes the base design active.
    pub fn config_delete(&mut self, config: &str) -> Result<(), DocError> {
        let i = self
            .configurations
            .list
            .iter()
            .position(|c| c.name == config)
            .ok_or_else(|| bad(format!("no configuration named {config:?}")))?;
        self.configurations.list.remove(i);
        if self.configurations.active.as_deref() == Some(config) {
            self.configurations.active = None;
            self.evaluate()?;
        }
        Ok(())
    }

    /// A removed feature leaves every configuration.
    pub(crate) fn configs_forget_feature(&mut self, id: u32) {
        for c in &mut self.configurations.list {
            c.suppressed.remove(&id);
        }
    }

    /// Checks configurations read from a file against the design.
    pub(crate) fn check_configurations(&self, c: &Configurations) -> Result<(), DocError> {
        if c.list.len() > MAX_CONFIGURATIONS {
            return Err(bad("too many configurations"));
        }
        let mut names = std::collections::BTreeSet::new();
        for k in &c.list {
            if check_config_name(&k.name)? != k.name || !names.insert(k.name.as_str()) {
                return Err(bad(format!("bad or repeated name {:?}", k.name)));
            }
            if k.parameters.len() > crate::MAX_PARAMETERS
                || k.suppressed.len() > crate::feature::MAX_FEATURES
            {
                return Err(bad(format!("{} holds too many entries", k.name)));
            }
            for (p, e) in &k.parameters {
                if !self.parameters.contains_key(p) || e.chars().count() > expr::MAX_EXPRESSION_LEN
                {
                    return Err(bad(format!(
                        "{} overrides {p:?}, which is not a parameter of the design",
                        k.name
                    )));
                }
            }
            for id in k.suppressed.keys() {
                if self.feature(*id).is_err() {
                    return Err(bad(format!("{} suppresses missing feature {id}", k.name)));
                }
            }
        }
        if let Some(a) = &c.active
            && !names.contains(a.as_str())
        {
            return Err(bad(format!(
                "the active configuration {a:?} does not exist"
            )));
        }
        Ok(())
    }
}

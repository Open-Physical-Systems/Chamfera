//! Ribs and webs on the timeline (format 40): a thin wall grown from an open chain of
//! lines and arcs of a sketch until it meets a solid (a rib) or through it (a web), joined
//! to that solid. The wall is `thickness` thick across the sketch plane, half on each
//! side. Faces are named after the feature like an extrude's (`7:side 0` along the first
//! curve), so later features find them again.

use crate::{DocError, Document, expr};
use serde::{Deserialize, Serialize};

/// Most curves one rib may follow.
pub const MAX_RIB_CURVES: usize = 1_000;

fn is_false(b: &bool) -> bool {
    !*b
}

/// A rib (or, with `web`, a web).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rib {
    /// The sketch the chain is in.
    pub sketch: u32,
    /// Lines and arcs of the sketch, joined end to end, in order; the chain is open.
    pub curves: Vec<u32>,
    /// The wall thickness, an expression such as `3 mm`.
    pub thickness: String,
    /// Through the solid, filling every gap it crosses, instead of stopping at the first
    /// faces it meets.
    #[serde(default, skip_serializing_if = "is_false")]
    pub web: bool,
    /// Grow away from the solid's side of the chain instead of towards it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip: bool,
    /// The body to join, by name; by default the one nearest the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

fn bad(reason: impl Into<String>) -> DocError {
    DocError::BadFeature {
        reason: reason.into(),
    }
}

impl Document {
    /// A rib's thickness, checked: a length above 0.
    pub fn rib_thickness(&self, r: &Rib) -> Result<f64, DocError> {
        let t = self.evaluate_feature_value(&r.thickness, expr::Dim::LENGTH)?;
        if !t.is_finite()
            || t <= chamfera_geom::LINEAR_TOLERANCE
            || t > chamfera_geom::MAX_COORDINATE
        {
            return Err(bad("the rib thickness must be a length above 0"));
        }
        Ok(t)
    }

    pub(crate) fn check_rib(&self, r: &Rib) -> Result<(), DocError> {
        let f = self.sketch(r.sketch)?;
        if r.curves.is_empty() || r.curves.len() > MAX_RIB_CURVES {
            return Err(bad(format!("pick 1 to {MAX_RIB_CURVES} lines or arcs")));
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in &r.curves {
            if !seen.insert(*id) {
                return Err(bad(format!("the rib uses curve {id} twice")));
            }
            match f.sketch.curve(chamfera_sketch::CurveId(*id)) {
                Some(chamfera_sketch::Curve::Line { .. } | chamfera_sketch::Curve::Arc { .. }) => {}
                Some(_) => {
                    return Err(bad(format!(
                        "curve {id} of {} is not a line or an arc",
                        f.name
                    )));
                }
                None => return Err(bad(format!("{} has no curve {id}", f.name))),
            }
        }
        if let Some(b) = &r.body
            && (b.is_empty() || b.chars().count() > 512)
        {
            return Err(bad("the body name is empty or too long"));
        }
        self.rib_thickness(r).map(|_| ())
    }
}

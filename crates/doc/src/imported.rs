//! Imported solid bodies (STEP): exact kernel bodies the design keeps as they are.
//!
//! They are not timeline features yet: they have no parameters and are not rebuilt,
//! only shown. Each takes an id from the same counter as sketches and features
//! (`Document::last_feature_id`), so ids never clash. A body is stored as the kernel's
//! serialized B-rep, which re-checks its structure on load; [`check`] adds the full
//! solid check.

use crate::{DocError, Document};
use chamfera_kernel::Body;
use serde::{Deserialize, Serialize};

/// Most imported bodies one design may hold.
pub const MAX_IMPORTED: usize = 10_000;
/// Longest name of an imported body, in characters.
pub const MAX_IMPORTED_NAME: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedBody {
    pub id: u32,
    pub name: String,
    pub body: Body,
}

fn bad(reason: String) -> DocError {
    DocError::BadFeature { reason }
}

impl Document {
    /// Adds an imported body (it must pass [`Body::validate`]) and returns its id.
    pub fn add_imported(&mut self, name: &str, body: Body) -> Result<u32, DocError> {
        if self.imported.len() >= MAX_IMPORTED {
            return Err(bad(format!(
                "a design holds at most {MAX_IMPORTED} imported bodies"
            )));
        }
        body.validate().map_err(|e| {
            bad(format!(
                "the imported body {name:?} is not a valid solid: {e}"
            ))
        })?;
        let id = self
            .last_feature_id
            .checked_add(1)
            .ok_or_else(|| bad("no more ids".into()))?;
        let name: String = name.chars().take(MAX_IMPORTED_NAME).collect();
        self.last_feature_id = id;
        self.imported.push(ImportedBody { id, name, body });
        Ok(id)
    }

    /// The imported body `id`.
    pub fn imported_body(&self, id: u32) -> Option<&ImportedBody> {
        self.imported.iter().find(|b| b.id == id)
    }

    /// Removes the imported body `id`.
    pub fn remove_imported(&mut self, id: u32) -> Result<ImportedBody, DocError> {
        let i = self
            .imported
            .iter()
            .position(|b| b.id == id)
            .ok_or(DocError::NoSuchFeature(id))?;
        Ok(self.imported.remove(i))
    }
}

/// Checks the imported bodies of a loaded design: count, ids unique among everything
/// in `seen` (sketches and features) and handed out, names, and each body a valid solid.
pub(crate) fn check(
    bodies: &[ImportedBody],
    seen: &mut std::collections::BTreeSet<u32>,
    last_id: u32,
) -> Result<(), DocError> {
    if bodies.len() > MAX_IMPORTED {
        return Err(bad("too many imported bodies".into()));
    }
    for b in bodies {
        if !seen.insert(b.id) || b.id == 0 || b.id > last_id {
            return Err(bad(format!(
                "imported body {} has a duplicate or out-of-range id",
                b.id
            )));
        }
        if b.name.chars().count() > MAX_IMPORTED_NAME {
            return Err(bad(format!(
                "imported body {} has a name longer than {MAX_IMPORTED_NAME}",
                b.id
            )));
        }
        b.body
            .validate()
            .map_err(|e| bad(format!("imported body {} is not a valid solid: {e}", b.id)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chamfera_geom::{Point3, Vec3};

    #[test]
    fn imported_bodies_survive_save_and_open_and_bad_ones_are_refused() {
        let mut doc = Document::default();
        let cube = Body::cuboid(Point3::ORIGIN, Vec3::new(2.0, 3.0, 4.0)).unwrap();
        let id = doc.add_imported("cube", cube.clone()).unwrap();
        assert_eq!(id, 1);
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        // 2 * 3 * 4 = 24.
        let v = back.imported_body(id).unwrap().body.volume().unwrap();
        assert!((v - 24.0).abs() < 1e-12);
        // An unfinished body is refused when added and when loaded.
        let mut skeleton = Body::new();
        skeleton.mvfs(Point3::ORIGIN).unwrap();
        assert!(doc.add_imported("bad", skeleton.clone()).is_err());
        let mut tampered = doc.clone();
        tampered.imported[0].body = skeleton;
        assert!(Document::from_json(&tampered.to_json().unwrap()).is_err());
        // Ids may not clash or run past the counter.
        let mut clash = doc.clone();
        clash.imported.push(clash.imported[0].clone());
        assert!(Document::from_json(&clash.to_json().unwrap()).is_err());
        let mut ahead = doc.clone();
        ahead.imported[0].id = 7;
        assert!(Document::from_json(&ahead.to_json().unwrap()).is_err());
        assert!(doc.remove_imported(id).is_ok());
        assert!(doc.remove_imported(id).is_err());
    }
}

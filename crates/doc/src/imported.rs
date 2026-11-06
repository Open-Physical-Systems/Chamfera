//! Imported bodies: exact solids (STEP) and meshes (STL, OBJ, 3MF) the design keeps as
//! they are.
//!
//! They are not timeline features yet: they have no parameters and are not rebuilt,
//! only shown. Each takes an id from the same counter as sketches and features
//! (`Document::last_feature_id`), so ids never clash. A body is stored as the kernel's
//! serialized B-rep, which re-checks its structure on load; [`check`] adds the full
//! solid check. A mesh is stored as its triangles, re-checked on load
//! ([`TriMesh::validate`]); it is shared ([`SharedMesh`]) so that undo, which keeps
//! copies of the design, does not copy it.

use crate::{DocError, Document};
use chamfera_geom::TriMesh;
use chamfera_kernel::Body;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

/// Most imported meshes one design may hold.
pub const MAX_IMPORTED_MESHES: usize = 10_000;

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

/// A mesh shared between copies of the design (undo steps); compared by content.
#[derive(Debug, Clone)]
pub struct SharedMesh(pub Arc<TriMesh>);

impl PartialEq for SharedMesh {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Serialize for SharedMesh {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for SharedMesh {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        TriMesh::deserialize(d).map(|m| SharedMesh(Arc::new(m)))
    }
}

/// A mesh body imported from an STL, OBJ or 3MF file (format 31).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedMesh {
    pub id: u32,
    pub name: String,
    pub mesh: SharedMesh,
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

    /// Adds an imported mesh (it must pass [`TriMesh::validate`]) and returns its id.
    pub fn add_imported_mesh(&mut self, name: &str, mesh: TriMesh) -> Result<u32, DocError> {
        if self.meshes.len() >= MAX_IMPORTED_MESHES {
            return Err(bad(format!(
                "a design holds at most {MAX_IMPORTED_MESHES} imported meshes"
            )));
        }
        mesh.validate()
            .map_err(|e| bad(format!("the imported mesh {name:?} is not usable: {e}")))?;
        let id = self
            .last_feature_id
            .checked_add(1)
            .ok_or_else(|| bad("no more ids".into()))?;
        let name: String = name.chars().take(MAX_IMPORTED_NAME).collect();
        self.last_feature_id = id;
        self.meshes.push(ImportedMesh {
            id,
            name,
            mesh: SharedMesh(Arc::new(mesh)),
        });
        Ok(id)
    }

    /// The imported mesh `id`.
    pub fn imported_mesh(&self, id: u32) -> Option<&ImportedMesh> {
        self.meshes.iter().find(|m| m.id == id)
    }

    /// Renames the imported mesh `id`.
    pub fn rename_imported_mesh(&mut self, id: u32, name: &str) -> Result<(), DocError> {
        let m = self
            .meshes
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or(DocError::NoSuchFeature(id))?;
        m.name = name.chars().take(MAX_IMPORTED_NAME).collect();
        Ok(())
    }

    /// Removes the imported mesh `id`.
    pub fn remove_imported_mesh(&mut self, id: u32) -> Result<ImportedMesh, DocError> {
        let i = self
            .meshes
            .iter()
            .position(|m| m.id == id)
            .ok_or(DocError::NoSuchFeature(id))?;
        Ok(self.meshes.remove(i))
    }

    /// Every name a body of the design has: features (their bodies take their names),
    /// imported solids and imported meshes.
    pub fn body_names(&self) -> Vec<&str> {
        self.features
            .iter()
            .map(|f| f.name.as_str())
            .chain(self.imported.iter().map(|b| b.name.as_str()))
            .chain(self.meshes.iter().map(|m| m.name.as_str()))
            .collect()
    }

    /// `name` if no body of the design has it, else the first free `name (2)`,
    /// `name (3)`, ...
    pub fn unique_body_name(&self, name: &str) -> String {
        let taken = self.body_names();
        if !taken.contains(&name) {
            return name.to_owned();
        }
        (2..=taken.len() + 2)
            .map(|k| format!("{name} ({k})"))
            .find(|n| !taken.contains(&n.as_str()))
            .unwrap_or_else(|| name.to_owned())
    }
}

/// Checks the imported meshes of a loaded design like [`check`], each mesh by
/// [`TriMesh::validate`].
pub(crate) fn check_meshes(
    meshes: &[ImportedMesh],
    seen: &mut std::collections::BTreeSet<u32>,
    last_id: u32,
) -> Result<(), DocError> {
    if meshes.len() > MAX_IMPORTED_MESHES {
        return Err(bad("too many imported meshes".into()));
    }
    for m in meshes {
        if !seen.insert(m.id) || m.id == 0 || m.id > last_id {
            return Err(bad(format!(
                "imported mesh {} has a duplicate or out-of-range id",
                m.id
            )));
        }
        if m.name.chars().count() > MAX_IMPORTED_NAME {
            return Err(bad(format!(
                "imported mesh {} has a name longer than {MAX_IMPORTED_NAME}",
                m.id
            )));
        }
        m.mesh
            .0
            .validate()
            .map_err(|e| bad(format!("imported mesh {} is not usable: {e}", m.id)))?;
    }
    Ok(())
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

    #[test]
    fn imported_meshes_survive_save_and_open_and_bad_ones_are_refused() {
        let mut doc = Document::default();
        // A 1 x 2 x 3 box: 12 triangles, volume 6.
        let mesh = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 2.0, 3.0));
        let id = doc.add_imported_mesh("box", mesh.clone()).unwrap();
        let back = Document::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(back, doc);
        let m = &back.imported_mesh(id).unwrap().mesh.0;
        assert_eq!(m.triangles.len(), 12);
        assert!((m.signed_volume() - 6.0).abs() < 1e-12);
        // An empty mesh is refused when added and when loaded; ids may not clash.
        assert!(doc.add_imported_mesh("empty", TriMesh::default()).is_err());
        let mut tampered = doc.clone();
        tampered.meshes[0].mesh = SharedMesh(Arc::new(TriMesh::default()));
        assert!(Document::from_json(&tampered.to_json().unwrap()).is_err());
        let mut clash = doc.clone();
        clash.meshes.push(clash.meshes[0].clone());
        assert!(Document::from_json(&clash.to_json().unwrap()).is_err());
        // A vertex index past the end is refused on load.
        let mut broken = doc.clone();
        let mut m = mesh;
        m.triangles[0] = [0, 1, 99];
        broken.meshes[0].mesh = SharedMesh(Arc::new(m));
        assert!(Document::from_json(&broken.to_json().unwrap()).is_err());
        // Names: the second "box" is "box (2)", the third "box (3)".
        assert_eq!(doc.unique_body_name("box"), "box (2)");
        let unit = TriMesh::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0));
        doc.add_imported_mesh("box (2)", unit).unwrap();
        assert_eq!(doc.unique_body_name("box"), "box (3)");
        assert_eq!(doc.unique_body_name("other"), "other");
        doc.rename_imported_mesh(id, "renamed").unwrap();
        assert_eq!(doc.imported_mesh(id).unwrap().name, "renamed");
        assert!(doc.remove_imported_mesh(id).is_ok());
        assert!(doc.remove_imported_mesh(id).is_err());
    }
}

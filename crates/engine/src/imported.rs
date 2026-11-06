//! Imported solid bodies (STEP) in the scene: shown and measured like the timeline's
//! solids, but not rebuilt from features.

use crate::Session;
use crate::solids::{Built, Solid, face_key};
use chamfera_kernel::{TessOptions, tessellate};

impl Session {
    /// Adds the design's imported bodies to `built` and to the scene, after the
    /// timeline's solids. Each becomes a [`Solid`] whose `feature` is the imported
    /// body's id, so export, measures and face picking treat it like a built solid. Its
    /// faces are named `id:role` like built faces ([`face_key`]); the role is the face
    /// name from the file, or `face N`. `rebuild` calls this before storing `built`
    /// (the scene's old solid bodies are already removed then).
    pub(crate) fn show_imported(&mut self, built: &mut Built) {
        let options = TessOptions::default();
        for imported in &self.doc.imported {
            let mut body = imported.body.clone();
            for (k, face) in body.faces().into_iter().enumerate() {
                let role = body
                    .face_name(face)
                    .ok()
                    .flatten()
                    .map_or_else(|| format!("face {}", k + 1), str::to_owned);
                let _ = body.set_face_name(face, &face_key(imported.id, &role));
            }
            let mut faces = Vec::new();
            let scene_id = match tessellate(&body, &options) {
                Ok(m) => {
                    faces = m.face_of_triangle;
                    let id = self.scene.add(&imported.name, m.mesh);
                    if id.is_none() {
                        built
                            .display_errors
                            .insert(imported.id, "the scene holds no more bodies".into());
                    }
                    id
                }
                Err(e) => {
                    built.display_errors.insert(
                        imported.id,
                        format!("the imported solid cannot be shown yet: {e}"),
                    );
                    None
                }
            };
            built.solids.push(Solid {
                feature: imported.id,
                name: imported.name.clone(),
                body,
                scene_id,
                faces,
            });
        }
    }
}

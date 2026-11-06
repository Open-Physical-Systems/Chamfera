//! Layer 4: the session and the command registry.
//!
//! Everything a user can do is a command. The UI, the CLI, scripts and the MCP server all call
//! [`Session::execute`] with a command id and JSON parameters, so they behave the same way.
//! Read `skills/chamfera-commands/SKILL.md` before adding one.

pub mod agent;
pub mod assembly;
mod body_tools;
pub mod box_select;
pub mod cam;
pub mod cam_cache;
#[cfg(test)]
mod cam_tests;
pub mod cmd;
pub mod construct;
pub mod display;
#[cfg(test)]
mod drawing_tests;
pub mod face_edit;
pub mod hole;
mod imported;
pub mod motion;
pub mod naming;
#[cfg(test)]
mod naming_tests;
mod params;
#[cfg(test)]
mod params_tests;
pub mod perf;
#[cfg(test)]
mod perf_tests;
pub mod pick_filter;
pub mod projection;
pub mod render;
#[cfg(test)]
mod render_tests;
pub mod script;
pub mod sheet;
pub mod sketching;
pub mod solids;
pub mod surface;
pub mod versions;

use chamfera_doc::{DocError, Document};
use chamfera_render::{Camera, Scene};
use serde_json::Value;
use thiserror::Error;

pub use chamfera_io::read_limited;
pub use cmd::{CommandSpec, registry};

/// How many undo steps a session keeps.
pub const UNDO_LIMIT: usize = 200;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("no command named {0:?}")]
    UnknownCommand(String),
    #[error("bad parameters: {0}")]
    BadParams(String),
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Doc(#[from] DocError),
    #[error(transparent)]
    Io(#[from] chamfera_io::IoError),
}

pub type CmdResult = Result<Value, CommandError>;

/// One open design, its history, and what the viewport shows.
#[derive(Debug, Default)]
pub struct Session {
    pub doc: Document,
    /// Bodies on screen: the solids the design builds, its imported solids and meshes
    /// (all saved and undone with the design), and the sample part (session only).
    pub scene: Scene,
    /// The scene body showing each imported mesh of the design, by the mesh's id. Kept
    /// across rebuilds so a mesh keeps its scene id ([`Session::sync_meshes`]).
    pub(crate) mesh_scene: std::collections::BTreeMap<u32, u32>,
    /// The view. Not part of the design or of undo.
    pub camera: Camera,
    /// The sketch being edited (`sketch.create`, `sketch.edit`), if any.
    pub active_sketch: Option<u32>,
    /// Selected points and curves of the active sketch.
    pub sketch_selection: Vec<sketching::SketchItem>,
    /// Selected edges of solids, by name (`select.pick` with a `filter` that has edges).
    pub selected_edges: Vec<String>,
    /// Solids built from the timeline (worked out again after every change).
    built: solids::Built,
    /// Checkpoints and meshes of earlier rebuilds, to rebuild only what changed. Not part
    /// of the design or of undo.
    pub(crate) caches: perf::Caches,
    /// Per-item times of the rebuilds since timing was switched on (`debug.timings`).
    pub(crate) timings: Vec<perf::FeatureTiming>,
    /// Toolpaths worked out from the CAM section, and whether they are shown.
    pub(crate) cam: cam::CamState,
    /// What a joint drag, the motion study's time and an exploded view show. Display
    /// state: not part of the design or of undo.
    pub(crate) view_state: motion::AssemblyView,
    /// The last presentation render started (`render.start`), stepped by whoever shows
    /// it. Not part of the design or of undo.
    pub render_job: Option<std::sync::Arc<chamfera_raytrace::Render>>,
    /// The file the design was last opened from or saved to.
    pub path: Option<std::path::PathBuf>,
    /// Whether saves keep versions beside the file, and how many (`versions`).
    pub history: versions::HistorySettings,
    /// The design as last opened or saved, to tell whether there are unsaved changes.
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    /// Undo steps recorded so far, and how often the history was rewound (undo, redo, a
    /// new design): an [`UndoMark`] holds both.
    undo_pushes: u64,
    history_epoch: u64,
}

/// A place in the undo history, so the steps recorded after it can be merged into one
/// ([`Session::group_undo_since`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoMark {
    pushes: u64,
    epoch: u64,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs a command. A command that fails leaves the design unchanged. A command that
    /// changes the design can be undone.
    pub fn execute(&mut self, id: &str, params: &Value) -> CmdResult {
        let spec = cmd::find(id).ok_or_else(|| CommandError::UnknownCommand(id.to_owned()))?;
        let state = (self.active_sketch, self.sketch_selection.clone());
        let result = if spec.mutates {
            let before = self.doc.clone();
            match (spec.run)(self, params) {
                Ok(reply) => {
                    if self.doc != before {
                        self.push_undo(before);
                        self.redo.clear();
                    }
                    Ok(reply)
                }
                Err(e) => {
                    self.doc = before;
                    (self.active_sketch, self.sketch_selection) = state.clone();
                    Err(e)
                }
            }
        } else {
            (spec.run)(self, params)
        };
        // Sketch display follows the design, the active sketch and its selection. Commands
        // that change none of these (camera moves, picks, queries: every mouse move) skip
        // the rebuild.
        // Solids follow the design: rebuild after anything that may have changed it.
        // solid.* and feature.* rebuild themselves; a failed command was rolled back, so
        // the solids must follow the restored design. The body commands that change
        // imported meshes show the change themselves ([`Session::sync_meshes`]); a
        // rebuild would also bring back a solid's scene body that `body.remove` hid.
        let rebuilt_itself = id.starts_with("solid.")
            || id.starts_with("feature.")
            || id.starts_with("component.")
            || id.starts_with("joint.")
            || id.starts_with("construct.")
            || (id.starts_with("body.") && id != "body.appearance");
        if (spec.mutates && (result.is_err() || !rebuilt_itself))
            || (id.starts_with("edit.") && result.is_ok())
            || (id.starts_with("design.") && result.is_ok() && id != "design.save")
        {
            self.rebuild();
        }
        if spec.mutates
            || id.starts_with("edit.")
            || id.starts_with("design.")
            || (self.active_sketch, &self.sketch_selection) != (state.0, &state.1)
        {
            self.refresh_overlays();
        }
        result
    }

    fn push_undo(&mut self, doc: Document) {
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(doc);
        self.undo_pushes += 1;
    }

    /// Where the undo history is now.
    pub fn undo_mark(&self) -> UndoMark {
        UndoMark {
            pushes: self.undo_pushes,
            epoch: self.history_epoch,
        }
    }

    /// Merges the undo steps recorded since `mark` into one, so a single undo takes the
    /// design back to how it was at the mark. Nothing is merged when the history was
    /// rewound in between (an undo, a redo, a new or opened design). Returns how many
    /// steps the merged one stands for.
    pub fn group_undo_since(&mut self, mark: UndoMark) -> usize {
        if mark.epoch != self.history_epoch || mark.pushes > self.undo_pushes {
            return 0;
        }
        let since = usize::try_from(self.undo_pushes - mark.pushes).unwrap_or(usize::MAX);
        // Steps older than the undo limit are gone; the oldest kept one stands in.
        let n = since.min(self.undo.len());
        if n >= 2 {
            let keep = self.undo.len() - n;
            // The snapshot taken before the first step is the one to go back to.
            self.undo.truncate(keep + 1);
        }
        n
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self) -> bool {
        self.history_epoch += 1;
        match self.undo.pop() {
            Some(prev) => {
                let current = std::mem::replace(&mut self.doc, prev);
                self.redo.push(current);
                true
            }
            None => false,
        }
    }

    pub(crate) fn redo(&mut self) -> bool {
        self.history_epoch += 1;
        match self.redo.pop() {
            Some(next) => {
                let current = std::mem::replace(&mut self.doc, next);
                self.push_undo(current);
                true
            }
            None => false,
        }
    }

    /// True when the design differs from the file it was last opened from or saved to.
    pub fn has_unsaved_changes(&self) -> bool {
        self.doc != self.saved
    }

    pub(crate) fn mark_saved(&mut self, path: std::path::PathBuf) {
        self.saved = self.doc.clone();
        self.path = Some(path);
    }

    /// Replaces the design and forgets its history (open, new).
    pub(crate) fn replace_document(&mut self, doc: Document) {
        self.saved = doc.clone();
        self.path = None;
        self.doc = doc;
        self.scene = Scene::default();
        self.mesh_scene.clear();
        self.view_state = motion::AssemblyView::default();
        self.undo.clear();
        self.redo.clear();
        self.history_epoch += 1;
    }
}

#[cfg(test)]
mod body_tools_tests;
#[cfg(test)]
mod qa_tests;
#[cfg(test)]
mod surface_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod versions_tests;

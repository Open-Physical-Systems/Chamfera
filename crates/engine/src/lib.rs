//! Layer 4: the session and the command registry.
//!
//! Everything a user can do is a command. The UI, the CLI, scripts and the MCP server all call
//! [`Session::execute`] with a command id and JSON parameters, so they behave the same way.
//! Read `skills/chamfera-commands/SKILL.md` before adding one.

pub mod assembly;
pub mod box_select;
pub mod cam;
#[cfg(test)]
mod cam_tests;
pub mod cmd;
pub mod construct;
pub mod display;
pub mod hole;
mod imported;
mod params;
pub mod pick_filter;
pub mod projection;
pub mod script;
pub mod sketching;
pub mod solids;

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
    /// Bodies on screen. In M1 these are mesh bodies (imported STL, samples). They are not
    /// saved in the design yet and are not part of undo.
    pub scene: Scene,
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
    /// Toolpaths worked out from the CAM section, and whether they are shown.
    pub(crate) cam: cam::CamState,
    /// The file the design was last opened from or saved to.
    pub path: Option<std::path::PathBuf>,
    /// The design as last opened or saved, to tell whether there are unsaved changes.
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
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
        // the solids must follow the restored design.
        let rebuilt_itself = id.starts_with("solid.")
            || id.starts_with("feature.")
            || id.starts_with("component.")
            || id.starts_with("joint.")
            || id.starts_with("construct.")
            || id == "body.import_step";
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
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo(&mut self) -> bool {
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
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests;

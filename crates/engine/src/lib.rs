//! Layer 4: the session and the command registry.
//!
//! Everything a user can do is a command. The UI, the CLI, scripts and the MCP server all call
//! [`Session::execute`] with a command id and JSON parameters, so they behave the same way.
//! Read `skills/chamfera-commands/SKILL.md` before adding one.

pub mod cmd;
mod params;

use chamfera_doc::{DocError, Document};
use serde_json::Value;
use thiserror::Error;

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

/// One open design and its history.
#[derive(Debug, Default)]
pub struct Session {
    pub doc: Document,
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
        if !spec.mutates {
            return (spec.run)(self, params);
        }
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
                Err(e)
            }
        }
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

    /// Replaces the design and forgets its history (open, new).
    pub(crate) fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests;

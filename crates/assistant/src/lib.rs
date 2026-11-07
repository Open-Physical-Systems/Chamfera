//! Layer 5: the in-app assistant. Claude builds a design live by calling Chamfera's
//! commands on the open session.
//!
//! - [`api`]: the Messages API request, the models and what a reply costs.
//! - [`sse`]: server-sent events from a streamed reply, put back together into a message.
//! - [`tools`]: Chamfera's command registry as Claude tools, and running one.
//! - [`agent`]: the tool-use loop over a [`agent::Transport`] and a [`agent::ToolHost`],
//!   so the UI runs tools on its own thread and tests use a scripted server.
//! - [`http`]: the HTTPS transport (desktop only; the browser build has none).
//! - [`key`]: the API key, which never reaches a log, a debug print or a design.
//! - [`script`]: a recorded conversation played back, for the demo and tests.
//!
//! No egui here: the panel is `crates/ui/src/assistant_ui.rs`. All input is hostile: the
//! stream, the tool inputs Claude writes and the key file are checked before use, and
//! nothing panics.

pub mod agent;
pub mod api;
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
pub mod key;
pub mod prompt;
pub mod script;
pub mod sse;
pub mod tools;

pub use agent::{Conversation, Event, RunEnd, ToolCall, ToolHost, ToolOutcome, Transport};
pub use api::{Effort, Model, Settings, Usage};
pub use key::ApiKey;

#[cfg(test)]
mod tests;

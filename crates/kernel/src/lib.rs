//! Layer 1: Chamfera's own boundary-representation (B-rep) kernel.
//!
//! Written from scratch, clean-room: from textbooks, papers and public specifications only
//! (see `skills/chamfera-kernel/SKILL.md` and `docs/kernel-ladder.md`). No code from
//! OpenCASCADE or any other GPL/LGPL kernel is read, copied or linked.
//!
//! M0 holds only the error type. The first rungs of the ladder (topology, analytic
//! surfaces, extrude) arrive in M3.

use thiserror::Error;

/// Why a kernel operation could not produce a valid solid. A failing operation never
/// returns a half-built body.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KernelError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error("the result would not be a valid solid: {0}")]
    InvalidResult(String),
}

pub type KernelResult<T> = Result<T, KernelError>;

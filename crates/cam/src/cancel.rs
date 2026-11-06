//! Stopping a toolpath job that is no longer wanted.
//!
//! A front end that works toolpaths out on a worker thread hands the worker a
//! [`Token`]: the job's generation and a shared counter of the latest generation. While
//! [`with`] runs a job on that thread, the long loops of the 3D operations call
//! [`check`], which fails with [`CamError::Cancelled`] once a newer generation was asked
//! for. On any thread without a token (scripts, tests, the engine's own synchronous
//! generation) [`check`] never fails, so results there are always complete and the same.

use crate::CamError;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A job's generation and the latest generation wanted.
#[derive(Debug, Clone)]
pub struct Token {
    pub latest: Arc<AtomicU64>,
    pub generation: u64,
}

impl Token {
    /// True once a newer generation than this job's was asked for.
    pub fn cancelled(&self) -> bool {
        self.latest.load(Ordering::Relaxed) != self.generation
    }
}

thread_local! {
    static TOKEN: RefCell<Option<Token>> = const { RefCell::new(None) };
}

/// Runs `f` with `token` as this thread's cancel token (restoring the one before).
pub fn with<T>(token: Token, f: impl FnOnce() -> T) -> T {
    let before = TOKEN.with(|t| t.replace(Some(token)));
    let out = f();
    TOKEN.with(|t| {
        t.replace(before);
    });
    out
}

/// `Err(Cancelled)` when this thread's job is no longer wanted.
pub fn check() -> Result<(), CamError> {
    let stop = TOKEN.with(|t| t.borrow().as_ref().is_some_and(Token::cancelled));
    if stop {
        Err(CamError::Cancelled)
    } else {
        Ok(())
    }
}

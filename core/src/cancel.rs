//! Cooperative cancellation, shared by every long-running operation in the core.
//!
//! `Arc` is load-bearing: the scan's walk hands a clone to each worker thread, so a bare
//! `AtomicBool` — which is not `Clone` — cannot do this job.
//!
//! It lives at the crate root rather than under `core::git` (R4) because it cancels a
//! directory walk as readily as a git process, and importing it from `core::git` to stop a
//! walk would misstate ownership for every plan that follows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A cooperative cancellation flag shared by a scan run's jobs and walks.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

/// The token fired. `core::git` converts this into `GitError::Cancelled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cancelled")
    }
}

impl std::error::Error for Cancelled {}

impl CancelToken {
    /// A token that has not fired.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fire it. Every waiter and every running child observes this.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether it has fired.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once it has fired.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

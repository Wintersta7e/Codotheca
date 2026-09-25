//! The trash double: it counts what an act sent and sends nothing.
//!
//! A test that proves a refusal reads [`CountingTrash::sends`] — **zero** is the assertion —
//! and a test that proves an act went through reads **one**. Nothing on disk is touched, so the
//! directory an act was refused over is still there to compare byte for byte.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::removal::{RemovalOutcome, Trash, TrashAvailability, TrashRefusal};

/// A [`Trash`] that records each `send` and removes nothing.
#[derive(Debug, Default)]
pub struct CountingTrash {
    sends: AtomicUsize,
}

impl CountingTrash {
    /// A trash that has been sent nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times `send` was called.
    #[must_use]
    pub fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }
}

impl Trash for CountingTrash {
    fn availability(&self, _path: &Path) -> TrashAvailability {
        TrashAvailability::Available
    }

    fn send(&self, _path: &Path) -> Result<RemovalOutcome, TrashRefusal> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(RemovalOutcome::Trashed)
    }
}

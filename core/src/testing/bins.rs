//! The bin doubles: settings that answer one fixed reading, and a trash that counts what an act
//! sent and sends nothing.
//!
//! A test that proves a refusal reads [`CountingTrash::sends`] — **zero** is the assertion —
//! and a test that proves an act went through reads **one**. Nothing on disk is touched, so the
//! directory an act was refused over is still there to compare byte for byte.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::removal::{
    trash_refusal_for, tree_bytes, BinFacts, BinSettings, RemovalOutcome, Trash, TrashAvailability,
    TrashRefusal,
};

/// [`BinSettings`] answering the same facts for every path.
#[derive(Debug, Clone)]
pub struct FixedBinSettings(pub BinFacts);

impl BinSettings for FixedBinSettings {
    fn for_path(&self, _path: &Path) -> BinFacts {
        self.0.clone()
    }
}

/// A [`Trash`] that records each `send` and removes nothing. Its availability is the production
/// classifier's over `bins`, or always available with none.
#[derive(Debug, Default)]
pub struct CountingTrash {
    sends: AtomicUsize,
    bins: Option<Arc<dyn BinSettings>>,
}

impl CountingTrash {
    /// A trash that has been sent nothing, and takes everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A trash whose availability `bins` decides, through [`trash_refusal_for`].
    #[must_use]
    pub fn with_bins(bins: Arc<dyn BinSettings>) -> Self {
        Self {
            sends: AtomicUsize::new(0),
            bins: Some(bins),
        }
    }

    /// How many times `send` was called.
    #[must_use]
    pub fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }
}

impl Trash for CountingTrash {
    fn availability(&self, path: &Path) -> TrashAvailability {
        self.bins
            .as_ref()
            .and_then(|bins| trash_refusal_for(&bins.for_path(path), || tree_bytes(path)))
            .map_or(TrashAvailability::Available, TrashAvailability::Unavailable)
    }

    fn send(&self, _path: &Path) -> Result<RemovalOutcome, TrashRefusal> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(RemovalOutcome::Trashed)
    }
}

//! §24.7F: OS trash where available, honest when not.
//!
//! **Both production implementations land with the trait** (R1's shape, four times recorded: a
//! trait declared for testability gets its fake and never its real implementation, compiles,
//! passes against the fake and fails at assembly). `HardDelete` has its consumer in this plan —
//! the staging path — and `SystemTrash` does not get one until p2-24b's Uninstall. It is still
//! written here, and written for real, because the alternative is a seam whose only body is a
//! test double.

use std::path::Path;
use std::sync::Arc;

use super::bins::{trash_refusal_for, tree_bytes, BinSettings};
use super::RemovalOutcome;
use crate::protocol::TrashRefusalKind;

/// Why a send failed: the platform's own reason, carried verbatim.
///
/// [p4] **What happens above a volume's quota is unmeasured.** The crate asks the shell for a
/// nuke warning (`FOF_WANTNUKEWARNING`, `trash-5.2.3/src/windows.rs:44`) under `FOF_NO_UI`, and
/// what the shell then does — prompt, refuse, or delete outright — is §46.7's probe to record on
/// a disposable Windows VM. The reasons a bin cannot take a copy are named **before** the send,
/// by [`TrashAvailability`]; this is only what a send that was attempted reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashRefusal {
    /// The attempt failed and the platform said why.
    Io(String),
}

/// Whether the OS trash will take this path.
///
/// A pre-flight, not a promise: it reports what can be established without writing. `send`
/// reports what actually happened. It surfaces as `UninstallVerdict.trashRefusal`, and
/// `trashAvailable` is its absence, from the same reading (§46.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashAvailability {
    /// Nothing that can be read stands in the way.
    Available,
    /// It will not work, and this is the reason to state.
    Unavailable(TrashRefusalKind),
}

/// Where removed bytes go.
///
/// Object-safe on purpose: `remove_warranted` takes `&dyn Trash`, so the destination is the
/// caller's decision and not the warrant's.
///
/// **Deviation from the plan's table, which writes `send(&self, &Path) -> Result<(),
/// TrashRefusal>` and puts `RemovalOutcome` on `remove_warranted` alone.** The implementation
/// that did the removing is the only thing that knows whether the bytes are recoverable, so it
/// returns that rather than having a second method — or a caller — restate it. One value, one
/// owner; the alternative is `RemovalOutcome` and the behaviour drifting apart.
pub trait Trash: Send + Sync + std::fmt::Debug {
    /// What `send` would do, established without writing anything.
    fn availability(&self, path: &Path) -> TrashAvailability;

    /// Remove `path`, recursively if it is a directory, and say whether it is recoverable.
    ///
    /// # Errors
    /// Returns the reason the platform gave, or the reason the pre-flight already knew.
    fn send(&self, path: &Path) -> Result<RemovalOutcome, TrashRefusal>;
}

/// Removes outright, with no recovery.
///
/// **This is what the staging path uses.** §24.3c removes bytes this process wrote in this
/// session into its own scratch directory: there is nothing there for a user to recover, and
/// routing a partial clone to the Recycle Bin would fill it with the app's own rubble.
#[derive(Debug, Clone, Copy, Default)]
pub struct HardDelete;

impl Trash for HardDelete {
    /// Always available: it needs no facility the filesystem does not already have.
    fn availability(&self, _path: &Path) -> TrashAvailability {
        TrashAvailability::Available
    }

    fn send(&self, path: &Path) -> Result<RemovalOutcome, TrashRefusal> {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        result
            .map(|()| RemovalOutcome::HardDeleted)
            .map_err(|error| TrashRefusal::Io(error.to_string()))
    }
}

/// §24.7F's Recycle Bin on Windows and XDG trash on Linux, its availability read from the
/// platform's bin settings (§46.7).
#[derive(Debug, Clone)]
pub struct SystemTrash {
    bins: Arc<dyn BinSettings>,
}

impl SystemTrash {
    /// The system trash, whose availability `bins` decides.
    #[must_use]
    pub fn new(bins: Arc<dyn BinSettings>) -> Self {
        Self { bins }
    }
}

impl Trash for SystemTrash {
    fn availability(&self, path: &Path) -> TrashAvailability {
        trash_refusal_for(&self.bins.for_path(path), || tree_bytes(path))
            .map_or(TrashAvailability::Available, TrashAvailability::Unavailable)
    }

    /// The crate's own message is carried verbatim; the reasons a bin cannot take a path were
    /// named before this was called (`remove_warranted` asks `availability` first).
    fn send(&self, path: &Path) -> Result<RemovalOutcome, TrashRefusal> {
        trash::delete(path)
            .map(|()| RemovalOutcome::Trashed)
            .map_err(|error| TrashRefusal::Io(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{HardDelete, RemovalOutcome, Trash as _, TrashAvailability, TrashRefusal};

    #[test]
    fn a_hard_delete_removes_a_tree_outright_and_says_it_is_unrecoverable() {
        let dir = tempfile::tempdir().expect("tmp");
        let target = dir.path().join("partial");
        std::fs::create_dir_all(target.join("nested")).expect("mkdir");
        std::fs::write(target.join("nested").join("a"), b"x").expect("write");

        assert_eq!(
            HardDelete.availability(&target),
            TrashAvailability::Available
        );
        assert_eq!(
            HardDelete.send(&target).expect("removed"),
            RemovalOutcome::HardDeleted,
            "the staging path must never report bytes as recoverable"
        );
        assert!(!target.exists(), "the staging tree must be gone");
        assert!(dir.path().exists(), "and its parent must not be touched");
    }

    #[test]
    fn a_hard_delete_reports_the_platform_reason_rather_than_succeeding_quietly() {
        let dir = tempfile::tempdir().expect("tmp");
        let missing = dir.path().join("never-existed");
        let error = HardDelete
            .send(&missing)
            .expect_err("a missing path is not a success");
        let TrashRefusal::Io(message) = error;
        assert!(!message.is_empty(), "the platform's own reason: {message}");
    }
}

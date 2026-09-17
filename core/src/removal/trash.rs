//! §24.7F: OS trash where available, honest when not.
//!
//! **Both production implementations land with the trait** (R1's shape, four times recorded: a
//! trait declared for testability gets its fake and never its real implementation, compiles,
//! passes against the fake and fails at assembly). `HardDelete` has its consumer in this plan —
//! the staging path — and `SystemTrash` does not get one until p2-24b's Uninstall. It is still
//! written here, and written for real, because the alternative is a seam whose only body is a
//! test double.

use std::path::Path;

use super::RemovalOutcome;

/// Why the OS trash cannot take a path.
///
/// **`OversizedFolder` has no producer in this plan, and that is stated rather than hidden.**
/// Windows' Recycle Bin silently permanently-deletes items above its per-volume quota; detecting
/// that needs the quota and a verified `IFileOperation` result code, neither of which this plan
/// can exercise — it has no uninstall path to run them against. **p2-24b owns producing it**, in
/// the change that gives `Trash` a consumer on a real working copy. Declaring it here keeps one
/// vocabulary for both plans; producing it from a guess would be worse than the gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashRefusal {
    /// This platform or this location has no trash to send to.
    Unsupported,
    /// Larger than the bin will hold, so sending would be a silent permanent delete.
    OversizedFolder,
    /// A network location. Windows does not recycle these; a delete there is permanent.
    NetworkDrive,
    /// The attempt failed and the platform said why.
    Io(String),
}

/// Whether the OS trash will take this path.
///
/// A pre-flight, not a promise: it reports what can be established cheaply and without writing.
/// `send` reports what actually happened. p2-24b surfaces this as `UninstallVerdict.trashAvailable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashAvailability {
    /// Nothing cheap and certain stands in the way.
    Available,
    /// It will not work, and this is the reason to state.
    Unavailable(TrashRefusal),
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
pub trait Trash: std::fmt::Debug {
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

/// §24.7F's Recycle Bin on Windows and XDG trash on Linux.
///
/// **Its consumer arrives with p2-24b** (deviation 9). It is real here rather than deferred
/// because a `Trash` seam whose only implementation removes bytes permanently would make the
/// recoverable path the one that was never written.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTrash;

/// A Windows UNC path — `\\server\share\…`.
///
/// Windows does not recycle network locations: a delete there is permanent and silent, which is
/// precisely the silent fallback §24.7F forbids. Checked by prefix because it needs no handle and
/// no syscall, and a false negative is caught by `send` reporting the platform's own reason.
#[cfg(windows)]
fn is_network_path(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    text.starts_with(r"\\") && !text.starts_with(r"\\?\")
}

impl Trash for SystemTrash {
    fn availability(&self, path: &Path) -> TrashAvailability {
        #[cfg(windows)]
        if is_network_path(path) {
            return TrashAvailability::Unavailable(TrashRefusal::NetworkDrive);
        }
        // The freedesktop trash is a directory under the user's data home. With no home there is
        // nowhere to put anything, and saying so is the honest answer rather than attempting it.
        #[cfg(all(unix, not(target_os = "macos")))]
        if std::env::var_os("XDG_DATA_HOME").is_none() && std::env::var_os("HOME").is_none() {
            return TrashAvailability::Unavailable(TrashRefusal::Unsupported);
        }
        let _ = path;
        TrashAvailability::Available
    }

    fn send(&self, path: &Path) -> Result<RemovalOutcome, TrashRefusal> {
        if let TrashAvailability::Unavailable(reason) = self.availability(path) {
            return Err(reason);
        }
        // The crate's own message is carried verbatim. Mapping an OS result code onto
        // `OversizedFolder` or `NetworkDrive` would be inventing a classification this plan
        // cannot run against a real Recycle Bin — see `TrashRefusal`'s note.
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
        match error {
            TrashRefusal::Io(message) => assert!(!message.is_empty(), "{message}"),
            other => panic!("expected the platform's own reason, got {other:?}"),
        }
    }
}

//! §24.3b: the clone runs into a staging directory and is renamed into place only after `git
//! clone` exits 0.
//!
//! Staging is a **sibling under the same root**, so the completing rename is same-filesystem and
//! atomic on NTFS and ext4. A cross-device staging root degrades that rename to a copy and
//! reintroduces the torn destination the staging directory exists to prevent — so it is refused
//! **before the clone spawns**, which is the only moment at which refusing it is free.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension as _, Transaction};

use crate::paths::path_from_bytes;
use crate::protocol::{InstallRefusal, InstallRunId};
use crate::removal::{SessionNonce, Warrant};
use crate::scan::links::file_id;

/// The one directory name a partial clone lives in.
///
/// **Stated once and read twice**: §4.3's skip list carries it so a partial clone is invisible to
/// the next scan by the value the scanner already reads, and the staging warrant's containment
/// check reads it so a forged warranted root cannot pass itself off as one. Two spellings of this
/// would be two different guarantees.
pub const STAGING_DIR_NAME: &str = ".codotheca-installing";

/// Where the clone for `seed_basename` runs, under `root`'s own staging directory.
///
/// # Errors
/// Refuses `root_unavailable` when the staging directory already exists on a different device
/// from the root — a symlink or a mount at that name — because the completing rename would then
/// be a copy.
pub fn staging_path_for(root: &Path, seed_basename: &str) -> Result<PathBuf, InstallRefusal> {
    let staging_root = root.join(STAGING_DIR_NAME);
    // Only meaningful when it already exists; a staging root this run is about to create is a
    // sibling by construction. `file_id`'s `volume` is `dev` on Unix and the volume serial on
    // Windows — §4.3's own identity, not a second one.
    if staging_root.exists() {
        let (Ok(root_id), Ok(staging_id)) = (file_id(root), file_id(&staging_root)) else {
            return Err(InstallRefusal::RootUnavailable);
        };
        if root_id.volume != staging_id.volume {
            return Err(InstallRefusal::RootUnavailable);
        }
    }
    Ok(staging_root.join(seed_basename))
}

/// The warrant authorising removal of one run's staging directory, built from the durable row.
///
/// **The warrant and its path are built together**, out of `install_run.staging_bytes`, which was
/// committed before the first byte was written. That is what makes *"a path this process created,
/// in this session, recorded before the first byte"* checkable rather than asserted — and it is
/// why there is no constructor taking a path (R63).
///
/// `Ok(None)` means **nothing here can be warranted**: no such run, or a row whose staging path
/// is not one this function can vouch for. §24.3c's sweep leaves those in place and reports them
/// as `abandoned_install` rather than removing them.
///
/// **Deviation from the plan's table, which writes `-> Result<Warrant, InstallFailure>`.**
/// `InstallFailure`'s seven variants are all ways a run that *started* ended badly — `network`,
/// `auth`, `disk_full`, `cancelled`, `git_failed`, `rename_failed`,
/// `filter_neutralisation_failed`. None of them says "there is no such row", and borrowing one
/// would state a reason that is not the reason.
///
/// # Errors
/// Fails when `install_run` cannot be read.
pub fn staging_warrant_for(
    tx: &Transaction<'_>,
    run: InstallRunId,
) -> rusqlite::Result<Option<Warrant>> {
    let staging_bytes = tx
        .query_row(
            "SELECT staging_bytes FROM install_run WHERE id = ?1",
            [run.0],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    let Some(staging_bytes) = staging_bytes else {
        return Ok(None);
    };
    let staging_path = path_from_bytes(&staging_bytes);
    // The row stores `<root>/.codotheca-installing/<seed_basename>`. A row whose path has no
    // parent, or no final component, is not one this function can vouch for.
    let (Some(staging_root), Some(seed_basename)) =
        (staging_path.parent(), staging_path.file_name())
    else {
        return Ok(None);
    };
    let Some(seed_basename) = seed_basename.to_str() else {
        return Ok(None);
    };
    Ok(Some(Warrant::for_staging(
        run,
        staging_root.to_path_buf(),
        seed_basename,
        SessionNonce::current(),
    )))
}

/// What §24.3c's start-up sweep did and what it refused to touch.
///
/// **Not `SweepReport`**, which is live at `core/src/art/store.rs:347` for the raster cache. Two
/// types of that name in one crate is a collision, not a coincidence.
///
/// `sweep_staging` itself is Task 14's; this plan declares the shape its ninth problem group is
/// built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StagingSweepReport {
    /// Directories this run warranted and removed.
    pub removed: usize,
    /// Directories it could not warrant. **Left in place**, and surfaced with their paths.
    pub unwarranted: Vec<PathBuf>,
}

/// §24.3c's start-up sweep: reconcile the staging directory against `install_run` rows.
///
/// **Anything it cannot warrant is left in place**, not removed and not hidden, and comes back in
/// `unwarranted` for `problems.list` to surface as `abandoned_install` with its path and a manual
/// removal. A sweep that deleted what it could not account for would be the one destructive
/// operation this boundary exists to prevent.
///
/// The warrant does the deciding: a directory left by a *previous* run of this core carries a
/// different `SessionNonce`, so `remove_warranted` refuses it and it is reported rather than
/// removed. That is the behaviour, not a special case in this function.
pub fn sweep_staging(
    index: &std::sync::Mutex<crate::index::Index>,
    roots: &[PathBuf],
) -> StagingSweepReport {
    let mut report = StagingSweepReport::default();
    for root in roots {
        let staging_root = root.join(STAGING_DIR_NAME);
        let Ok(entries) = std::fs::read_dir(&staging_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match warrant_for_path(index, &path) {
                Some(warrant) => {
                    if crate::removal::remove_warranted(&warrant, &crate::removal::HardDelete, None)
                        .is_ok()
                    {
                        report.removed = report.removed.saturating_add(1);
                    } else {
                        report.unwarranted.push(path);
                    }
                }
                None => report.unwarranted.push(path),
            }
        }
    }
    report
}

/// The warrant for a staging directory found on disk, looked up by its own path.
///
/// `None` when no `install_run` row records it — which is exactly the *cannot warrant* case: the
/// bytes may be another run's, another machine's, or something a user put there.
fn warrant_for_path(index: &std::sync::Mutex<crate::index::Index>, path: &Path) -> Option<Warrant> {
    let _guard = crate::proto::txguard::TxGuard::enter();
    let held = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tx = held.conn().unchecked_transaction().ok()?;
    let run: i64 = tx
        .query_row(
            "SELECT id FROM install_run WHERE staging_bytes = ?1 ORDER BY id DESC LIMIT 1",
            [crate::paths::path_bytes(path)],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    let warrant = staging_warrant_for(&tx, InstallRunId(run)).ok().flatten();
    // `tx` borrows the one connection behind the lock, so it ends first.
    drop(tx);
    drop(held);
    warrant
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{staging_path_for, STAGING_DIR_NAME};

    #[test]
    fn the_staging_path_is_a_sibling_under_the_destination_root() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = staging_path_for(dir.path(), "widget").expect("composed");
        assert_eq!(
            path.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new(STAGING_DIR_NAME))
        );
        assert_eq!(path.file_name(), Some(std::ffi::OsStr::new("widget")));
        assert!(
            path.starts_with(dir.path()),
            "staging must live under the root it serves"
        );
    }

    #[test]
    fn a_staging_root_that_does_not_exist_yet_is_composed_rather_than_refused() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(!dir.path().join(STAGING_DIR_NAME).exists());
        assert!(staging_path_for(dir.path(), "widget").is_ok());
    }
}

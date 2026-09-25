//! J2 — worktree status (§4.1). The one class §6 declares uncacheable. Every result is a
//! timestamped observation; absence of dirty means "no changes as of T", never "clean".
//!
//! **The porcelain-v2 parser this task originally declared is deleted.** Plan 05 ships
//! `core::git::parse_status_v2`, which is a strict superset — it consumes a rename record's
//! second field, reads the branch headers, and refuses an unknown record type instead of
//! ignoring it. Two parsers for one format is the defect this project keeps finding, and the
//! rename case is exactly where the second copy would have gone wrong first.
//!
//! What is left is §4.1's degrade: one full reading, and on a budget overrun one tracked-only
//! retry whose untracked count is NULL.

use crate::git::{GitBackend, GitError, JobContext, RepoHandle, StatusOptions, WorktreeStatus};

use super::JobError;

/// Whether a status reading is §4.1's degraded one.
///
/// **Ruling 2: "partial" needs no column.** A degraded reading is `worktree_observed_at` set
/// with `untracked_count` NULL, and never-observed is both NULL. That is the
/// never-render-unknown-as-zero encoding doing the work, so a `degraded: bool` beside the
/// `Option` would be the same fact stated twice.
#[must_use]
pub const fn is_degraded(status: &WorktreeStatus) -> bool {
    status.untracked_count.is_none()
}

/// Observe one worktree, degrading to tracked-only rather than failing.
///
/// §4.1: enumerating untracked files is what makes status expensive on a large tree, and a
/// repository the user is actively working in must still report whether it is dirty. So a
/// budget overrun costs the untracked count and nothing else.
///
/// A busy repository is **not** degraded to: §3.5 says defer rather than store a torn read, and
/// a tracked-only reading taken mid-rebase is still a reading taken mid-rebase.
///
/// # Errors
///
/// `JobError::RepositoryBusy` when an operation marker is in the way, `JobError::TornRead` when
/// the state moved mid-read, `JobError::BudgetExceeded` when the degraded retry also overruns,
/// and `JobError::Git` for any other git failure.
pub fn observe(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
) -> Result<WorktreeStatus, JobError> {
    match git.worktree_status(repo, StatusOptions::full(), ctx) {
        Ok(status) => Ok(status),
        // The retry's failure goes through `map_error` too: a second overrun must read as
        // BudgetExceeded, not as a generic git error, or the scheduler retries it forever.
        Err(GitError::Budget { .. }) => git
            .worktree_status(repo, StatusOptions::degraded(), ctx)
            .map_err(map_error),
        Err(e) => Err(map_error(e)),
    }
}

/// Turn a git failure into the job vocabulary the scheduler retries on.
fn map_error(e: GitError) -> JobError {
    match e {
        GitError::Busy { .. } => JobError::RepositoryBusy,
        GitError::TornRead => JobError::TornRead,
        GitError::Budget { .. } => JobError::BudgetExceeded,
        other => JobError::Git(other),
    }
}

/// Write one worktree observation.
///
/// `untracked_count` is written NULL on a degraded reading, which is the whole encoding of
/// §4.1's "the UI shows partial". There is no `refstate_basis` here and there never will be:
/// worktree state has no fingerprint (§6).
///
/// # Errors
///
/// `IndexError::Sqlite` when the update fails.
pub fn persist(
    tx: &rusqlite::Transaction<'_>,
    location: crate::protocol::LocationId,
    s: &WorktreeStatus,
) -> Result<(), crate::index::IndexError> {
    tx.execute(
        "UPDATE location SET is_dirty = ?2, untracked_count = ?3, worktree_observed_at = ?4
         WHERE id = ?1",
        rusqlite::params![
            location.0,
            i64::from(s.is_dirty),
            s.untracked_count.map(i64::from),
            s.observed_at,
        ],
    )?;
    Ok(())
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::cancel::CancelToken;
    use crate::git::{BusyMarker, JobClass, StoreKey};
    use crate::mount::StoreClass;
    use crate::testing::{FakeGitBackend, GitReply};

    fn handle() -> RepoHandle {
        RepoHandle::bare(
            std::path::Path::new("/w/repo"),
            StoreKey::new("s"),
            StoreClass::Local,
        )
    }

    fn status(untracked: Option<u32>) -> WorktreeStatus {
        WorktreeStatus {
            is_dirty: true,
            tracked_changes: 1,
            untracked_count: untracked,
            branch: Some("main".to_owned()),
            head_oid: Some("a".repeat(40)),
            ahead: None,
            behind: None,
            observed_at: 1_700_000_000,
        }
    }

    const fn ctx_with(cancel: &CancelToken) -> JobContext<'_> {
        JobContext::new(JobClass::Background, cancel, None)
    }

    #[test]
    fn a_full_reading_carries_the_untracked_count() {
        let git = FakeGitBackend::new();
        git.always_worktree_status(GitReply::Ok(status(Some(2))));
        let cancel = CancelToken::new();
        let out = observe(&git, &handle(), &ctx_with(&cancel)).unwrap();
        assert_eq!(out.untracked_count, Some(2));
        assert!(!is_degraded(&out));
    }

    #[test]
    fn exceeding_the_budget_degrades_to_tracked_only_and_leaves_untracked_null() {
        // §4.1: degrade — tracked-only, no untracked enumeration, and the UI shows partial.
        // Partial is exactly `worktree_observed_at` set with `untracked_count` NULL.
        let git = FakeGitBackend::new();
        git.on_worktree_status(None, GitReply::Err(GitError::Budget { after_ms: 500 }));
        git.on_worktree_status(None, GitReply::Ok(status(None)));
        let cancel = CancelToken::new();

        let out = observe(&git, &handle(), &ctx_with(&cancel)).unwrap();
        assert!(is_degraded(&out));
        assert!(out.is_dirty, "dirty is still answered after the degrade");
        assert_eq!(out.untracked_count, None);
        assert_eq!(out.observed_at, 1_700_000_000);
        assert_eq!(git.calls().len(), 2, "one full reading, then one degraded");
    }

    /// §3.5: a lock or an operation marker means defer, not degrade. A tracked-only reading
    /// taken mid-rebase is still a reading taken mid-rebase.
    #[test]
    fn a_busy_repository_defers_rather_than_degrading() {
        let git = FakeGitBackend::new();
        git.always_worktree_status(GitReply::Err(GitError::Busy {
            marker: BusyMarker::IndexLock,
        }));
        let cancel = CancelToken::new();
        let err = observe(&git, &handle(), &ctx_with(&cancel)).unwrap_err();
        assert!(matches!(err, JobError::RepositoryBusy));
        assert_eq!(git.calls().len(), 1, "no degraded retry after a defer");
    }

    #[test]
    fn a_torn_read_is_discarded_rather_than_stored() {
        let git = FakeGitBackend::new();
        git.always_worktree_status(GitReply::Err(GitError::TornRead));
        let cancel = CancelToken::new();
        let err = observe(&git, &handle(), &ctx_with(&cancel)).unwrap_err();
        assert!(matches!(err, JobError::TornRead));
    }

    /// The degraded retry may fail too, and then there is no reading at all — the job must not
    /// report a half-answer with an invented count.
    #[test]
    fn a_second_budget_overrun_fails_the_job() {
        let git = FakeGitBackend::new();
        git.always_worktree_status(GitReply::Err(GitError::Budget { after_ms: 500 }));
        let cancel = CancelToken::new();
        let err = observe(&git, &handle(), &ctx_with(&cancel)).unwrap_err();
        assert!(matches!(err, JobError::BudgetExceeded));
    }
}

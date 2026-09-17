//! J1 — ref state (§4.1).
//!
//! **R28 removed most of what this task originally declared.** `read_ref_state`, `divergence`,
//! `parse_head` and `parse_left_right` are plan 05's, in `core::git::refstate`, and this module
//! consumes them through `GitBackend` rather than restating them. What is left is the job: read
//! the ref state, count divergence against the configured upstream, and fold the two into one
//! `RefState`, because plan 05 deliberately leaves `ahead` and `behind` `None` — a graph walk is
//! not a file read, and an uncounted ahead is *not computed*, never `0` (§8.5.2).

use crate::git::{Divergence, GitBackend, JobContext, RefState, RepoHandle};

use super::JobError;

/// Read one location's ref state, divergence included.
///
/// Two calls: `ref_state` is file reads and takes no slot; `divergence` costs at most one bounded
/// `rev-list`, and nothing at all when the tips are equal or no upstream is configured.
pub fn observe(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
) -> Result<RefState, JobError> {
    let mut state = git.ref_state(repo, ctx)?;
    let counts = git.divergence(repo, &state, ctx)?;
    apply_divergence(&mut state, counts);
    Ok(state)
}

/// Fold a divergence result into the state it was counted for.
///
/// `None` leaves both fields `None`. That is the whole point: no upstream configured is *not
/// computed*, and §5.1 and §7.7 omit the chip entirely rather than drawing `BEHIND 0`. Equal
/// tips are a different answer — `Some(Divergence { 0, 0 })` — and a measured zero is real.
pub fn apply_divergence(state: &mut RefState, counts: Option<Divergence>) {
    state.ahead = counts.map(|d| d.ahead);
    state.behind = counts.map(|d| d.behind);
}

/// Write one location's ref state.
///
/// `ahead` and `behind` are written as they stand, `None` included: overwriting a NULL with a 0
/// would turn "not computed" into "level with upstream", which is a claim (§8.5.2).
///
/// **[p2-24b] `stash_count` joins them (R51).** `None` is an unreadable stash reflog and is
/// written as NULL, never 0 — a deletion gate reading 0 there would clear a copy holding work.
pub fn persist(
    tx: &rusqlite::Transaction<'_>,
    location: crate::protocol::LocationId,
    s: &RefState,
) -> Result<(), crate::index::IndexError> {
    tx.execute(
        "UPDATE location SET branch = ?2, head_oid = ?3, ahead = ?4, behind = ?5,
             stash_count = ?6, interrupted_op = ?7, fetch_head_at = ?8, reflog_tail_at = ?9,
             refstate_basis = ?10, refstate_observed_at = ?11
         WHERE id = ?1",
        rusqlite::params![
            location.0,
            s.branch,
            s.head_oid,
            s.ahead.map(i64::from),
            s.behind.map(i64::from),
            s.stash_count.map(i64::from),
            s.interrupted_op.map(|op| op.as_str()),
            s.fetch_head_at,
            s.reflog_tail_at,
            s.basis.as_str(),
            s.observed_at,
        ],
    )?;
    Ok(())
}

// The test doubles live behind `testkit`, which is default-off, so the module is gated on it
// too — otherwise a bare `cargo test` fails to compile rather than skipping. `--features testkit`
// is mandatory from plan 06 onward and every gate passes it.
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
    use crate::clock::Clock;
    use crate::git::{InterruptedOp, JobClass, RefFingerprint, StoreKey};
    use crate::mount::StoreClass;
    use crate::testing::{FakeClock, FakeGitBackend, GitReply};
    use std::sync::Arc;

    fn handle() -> RepoHandle {
        RepoHandle::bare(
            std::path::Path::new("/w/repo"),
            StoreKey::new("s"),
            StoreClass::Local,
        )
    }

    fn state_of(clock: &dyn Clock) -> RefState {
        RefState {
            head_oid: Some("a".repeat(40)),
            branch: Some("main".to_owned()),
            upstream: None,
            ahead: None,
            behind: None,
            tag_count: 0,
            stash_count: Some(0),
            is_shallow: false,
            is_bare: false,
            interrupted_op: None,
            fetch_head_at: None,
            reflog_tail_at: None,
            basis: RefFingerprint::from_hex(&"0".repeat(64)).unwrap(),
            observed_at: clock.now_unix(),
        }
    }

    fn run_with(divergence: Option<Divergence>) -> RefState {
        let clock = Arc::new(FakeClock::new(1_000));
        let git = FakeGitBackend::with_clock(Arc::clone(&clock));
        git.always_ref_state(GitReply::Ok(state_of(clock.as_ref())));
        git.always_divergence(GitReply::Ok(divergence));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        observe(&git, &handle(), &ctx).unwrap()
    }

    #[test]
    fn divergence_counts_land_on_the_state_j1_returns() {
        // Plan 05 leaves ahead/behind None on purpose; J1 is what fills them in.
        let s = run_with(Some(Divergence {
            ahead: 3,
            behind: 1,
        }));
        assert_eq!((s.ahead, s.behind), (Some(3), Some(1)));
    }

    #[test]
    fn equal_tips_are_a_real_zero() {
        // §1.3 / criterion 63: 0 here is a measured zero, and it is a different claim from NULL.
        let s = run_with(Some(Divergence {
            ahead: 0,
            behind: 0,
        }));
        assert_eq!((s.ahead, s.behind), (Some(0), Some(0)));
    }

    #[test]
    fn no_upstream_leaves_both_not_computed() {
        let s = run_with(None);
        assert_eq!((s.ahead, s.behind), (None, None));
    }

    /// A stale `Some` from a previous observation must not survive a run that could not count.
    #[test]
    fn a_previous_count_is_cleared_when_divergence_is_no_longer_computable() {
        let clock = FakeClock::new(0);
        let mut state = state_of(&clock);
        state.ahead = Some(9);
        state.behind = Some(9);
        apply_divergence(&mut state, None);
        assert_eq!((state.ahead, state.behind), (None, None));
    }

    #[test]
    fn an_operation_marker_is_reported_as_the_word_the_roast_uses() {
        // §5.6: phase 1 says merge or rebase and nothing else.
        assert_eq!(InterruptedOp::Merge.as_str(), "merge");
        assert_eq!(InterruptedOp::Rebase.as_str(), "rebase");
    }

    /// The backend's failure is the job's failure; nothing invents a ref state.
    #[test]
    fn an_unreadable_repository_fails_rather_than_reporting_an_empty_state() {
        let git = FakeGitBackend::new();
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        assert!(observe(&git, &handle(), &ctx).is_err());
    }
}

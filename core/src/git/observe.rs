//! §3.5: concurrent user activity.
//!
//! `--no-optional-locks` is etiquette, not protection — it gives no consistent snapshot and does
//! not stop the user checking out mid-scan. So a lock defers with bounded backoff, and every
//! multi-command observation is fingerprinted before and after.

use std::time::Duration;

use crate::cancel::CancelToken;
use crate::clock::Clock;

use super::error::{BusyMarker, GitError, GitResult};
use super::refstate::{observation_fingerprint, ref_fingerprint, RefFingerprint};
use super::repo::RepoHandle;

/// The bounded exponential backoff a held lock gets.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    attempts: u32,
}

impl Backoff {
    /// 25 · 50 · 100 · 200 · 400 · 800 ms — 1,575 ms in all, then defer.
    #[must_use]
    pub const fn bounded() -> Self {
        Self { attempts: 6 }
    }

    /// The delay sequence, oldest first.
    #[must_use]
    pub fn delays(&self) -> Vec<Duration> {
        (0..self.attempts)
            .map(|i| Duration::from_millis(25u64 << i))
            .collect()
    }
}

/// Which lock or operation marker is present right now, if any.
#[must_use]
pub fn busy_marker(repo: &RepoHandle) -> Option<BusyMarker> {
    if repo.git_dir.join("index.lock").exists() {
        return Some(BusyMarker::IndexLock);
    }
    if repo.git_dir.join("MERGE_HEAD").exists() {
        return Some(BusyMarker::Merge);
    }
    if repo.git_dir.join("REBASE_HEAD").exists()
        || repo.git_dir.join("rebase-merge").is_dir()
        || repo.git_dir.join("rebase-apply").is_dir()
    {
        return Some(BusyMarker::Rebase);
    }
    if repo.git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Some(BusyMarker::CherryPick);
    }
    if repo.git_dir.join("REVERT_HEAD").exists() {
        return Some(BusyMarker::Revert);
    }
    if repo.git_dir.join("BISECT_LOG").exists() {
        return Some(BusyMarker::Bisect);
    }
    None
}

/// Run `f` once the repository is not mid-operation, or defer.
///
/// Used by the index-touching jobs (J2, J3). **Not** used by the ref-state read, which is the
/// read that discovers the marker in the first place and must keep working on a repository the
/// user left mid-rebase.
///
/// # Errors
///
/// `GitError::Cancelled` when the token fires between attempts; `GitError::Busy` naming the
/// marker still present when the backoff runs out; otherwise whatever `f` fails with.
pub fn defer_while_locked<T>(
    repo: &RepoHandle,
    clock: &dyn Clock,
    cancel: &CancelToken,
    f: &mut dyn FnMut() -> GitResult<T>,
) -> GitResult<T> {
    let mut delays = Backoff::bounded().delays().into_iter();
    loop {
        cancel.check()?;
        let marker = busy_marker(repo);
        if marker.is_none() {
            match f() {
                Err(GitError::Busy { .. }) => {}
                other => return other,
            }
        }
        match delays.next() {
            Some(d) => clock.sleep(d),
            None => {
                return Err(GitError::Busy {
                    marker: busy_marker(repo).unwrap_or(BusyMarker::IndexLock),
                })
            }
        }
    }
}

/// A value plus the basis it was true against, and when it was read.
#[derive(Debug, Clone)]
pub struct Observation<T> {
    /// What was read.
    pub value: T,
    /// `location.refstate_basis` at the time — the §6 tuple, not the wider guard digest.
    pub basis: RefFingerprint,
    /// When the app looked; never when the user stopped (§5.6).
    pub observed_at: i64,
}

/// Fingerprint, observe, fingerprint again; discard the result if anything moved.
///
/// # Errors
///
/// `GitError::TornRead` when the fingerprint moved during `f`; `GitError::PathGone` or
/// `GitError::Unreadable` when a fingerprint cannot be taken; otherwise whatever `f` fails with.
pub fn observe_stable<T>(
    repo: &RepoHandle,
    clock: &dyn Clock,
    f: &mut dyn FnMut() -> GitResult<T>,
) -> GitResult<Observation<T>> {
    let before = observation_fingerprint(repo)?;
    let value = f()?;
    let after = observation_fingerprint(repo)?;
    if before != after {
        return Err(GitError::TornRead);
    }
    Ok(Observation {
        value,
        basis: ref_fingerprint(repo)?,
        observed_at: clock.now_unix(),
    })
}

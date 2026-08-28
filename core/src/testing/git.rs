//! `GitBackend` doubles: slow, failing and torn-read git.
//!
//! **These are semantic, not argv-level, and that is a deviation from plan 06 Task 5.** That
//! task specifies a `FakeGitBackend` matching on argv tokens and a `RecordingGitBackend`
//! wrapping the real one to capture argv. Plan 05 shipped `GitBackend` as one method per
//! operation, with no argv at the trait boundary at all, and `GitExec` — where argv does exist
//! — as a concrete struct rather than a seam. Argv rules therefore cannot be written here.
//!
//! The recorder's stated purpose is already met, and at a better level: plan 05's
//! `git_invocation.rs` proves §3.2's neutralising options and criterion 63's
//! `-c safe.directory=<exact-path>` directly against the argv builder, rather than through a
//! wrapper. What plans 07 and 09 still need from this file is the other half — an injectable
//! git that is slow, that fails, and that answers differently on two successive calls.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::git::{
    Authorship, CommitSubject, Divergence, GitBackend, GitError, GitResult, GitVersion, JobContext,
    RefState, RepoFacts, RepoHandle, RootCommit, StatusOptions, TrackedInventory, WorktreeStatus,
};
use crate::testing::FakeClock;

/// One call the fake answered, in the order it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedGitCall {
    /// The trait method, e.g. `ref_state`.
    pub op: &'static str,
    /// The repository it was asked about, or `None` for `version`.
    pub repo: Option<PathBuf>,
}

/// What the fake should do for one call.
#[derive(Debug, Clone)]
pub enum GitReply<T> {
    Ok(T),
    Err(GitError),
    /// Answer after `delay_ms`, which advances the injected clock rather than sleeping. A test
    /// for a 500 ms budget must not take 500 ms.
    Slow {
        delay_ms: u64,
        then: Box<GitReply<T>>,
    },
}

/// A queue of replies for one operation, plus the answer once the queue is empty.
#[derive(Debug)]
struct Op<T> {
    queued: Mutex<Vec<(Option<PathBuf>, GitReply<T>)>>,
    fallback: Mutex<Option<GitReply<T>>>,
}

impl<T> Default for Op<T> {
    fn default() -> Self {
        Self {
            queued: Mutex::new(Vec::new()),
            fallback: Mutex::new(None),
        }
    }
}

impl<T: Clone> Op<T> {
    fn push(&self, suffix: Option<PathBuf>, reply: GitReply<T>) {
        if let Ok(mut q) = self.queued.lock() {
            q.push((suffix, reply));
        }
    }

    fn set_fallback(&self, reply: GitReply<T>) {
        if let Ok(mut f) = self.fallback.lock() {
            *f = Some(reply);
        }
    }

    /// First queued reply whose suffix matches, removed so a second call gets the next one —
    /// which is how a torn read is expressed.
    fn take(&self, repo: Option<&Path>) -> Option<GitReply<T>> {
        if let Ok(mut q) = self.queued.lock() {
            let hit = q.iter().position(|(suffix, _)| match (suffix, repo) {
                (None, _) => true,
                (Some(s), Some(r)) => r.ends_with(s),
                (Some(_), None) => false,
            });
            if let Some(index) = hit {
                return Some(q.remove(index).1);
            }
        }
        self.fallback.lock().ok().and_then(|f| f.clone())
    }
}

/// A git backend a test drives.
#[derive(Debug, Default)]
pub struct FakeGitBackend {
    clock: Option<Arc<FakeClock>>,
    calls: Mutex<Vec<RecordedGitCall>>,
    version: Op<GitVersion>,
    repo_facts: Op<RepoFacts>,
    ref_state: Op<RefState>,
    divergence: Op<Option<Divergence>>,
    worktree_status: Op<WorktreeStatus>,
    tracked_inventory: Op<TrackedInventory>,
    submodule_gitlinks: Op<BTreeMap<Vec<u8>, String>>,
    root_commits: Op<Vec<RootCommit>>,
    authorship: Op<Authorship>,
    commit_subjects: Op<Vec<CommitSubject>>,
}

impl FakeGitBackend {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Give the fake a clock, so `Slow` moves time instead of spending it.
    #[must_use]
    pub fn with_clock(clock: Arc<FakeClock>) -> Self {
        Self {
            clock: Some(clock),
            ..Self::default()
        }
    }

    /// Every call the fake answered, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<RecordedGitCall> {
        self.calls.lock().map_or_else(|_| Vec::new(), |c| c.clone())
    }

    pub fn clear(&self) {
        if let Ok(mut c) = self.calls.lock() {
            c.clear();
        }
    }

    fn record(&self, op: &'static str, repo: Option<&Path>) {
        if let Ok(mut c) = self.calls.lock() {
            c.push(RecordedGitCall {
                op,
                repo: repo.map(Path::to_path_buf),
            });
        }
    }

    /// Resolve a reply, advancing the clock for a `Slow` one.
    fn settle<T>(&self, reply: GitReply<T>) -> GitResult<T> {
        match reply {
            GitReply::Ok(value) => Ok(value),
            GitReply::Err(e) => Err(e),
            GitReply::Slow { delay_ms, then } => {
                if let Some(clock) = self.clock.as_ref() {
                    clock.advance_ms(delay_ms);
                }
                self.settle(*then)
            }
        }
    }

    fn answer<T: Clone>(
        &self,
        op: &'static str,
        which: &Op<T>,
        repo: Option<&Path>,
        default: impl FnOnce() -> GitResult<T>,
    ) -> GitResult<T> {
        self.record(op, repo);
        match which.take(repo) {
            Some(reply) => self.settle(reply),
            None => default(),
        }
    }
}

/// `FakeGitBackend::on_<op>(suffix, reply)` and `always_<op>(reply)` for each operation.
macro_rules! setters {
    ($( $op:ident : $ty:ty , $on:ident , $always:ident ; )*) => {
        impl FakeGitBackend {
            $(
                /// Queue one reply, optionally scoped to a repository whose path ends with
                /// `suffix`. Queued replies are consumed in order, so two entries for one
                /// repository are a torn read.
                pub fn $on(&self, suffix: Option<&str>, reply: GitReply<$ty>) -> &Self {
                    self.$op.push(suffix.map(PathBuf::from), reply);
                    self
                }

                /// The reply for every call the queue does not cover.
                pub fn $always(&self, reply: GitReply<$ty>) -> &Self {
                    self.$op.set_fallback(reply);
                    self
                }
            )*
        }
    };
}

setters! {
    version: GitVersion, on_version, always_version;
    repo_facts: RepoFacts, on_repo_facts, always_repo_facts;
    ref_state: RefState, on_ref_state, always_ref_state;
    divergence: Option<Divergence>, on_divergence, always_divergence;
    worktree_status: WorktreeStatus, on_worktree_status, always_worktree_status;
    tracked_inventory: TrackedInventory, on_tracked_inventory, always_tracked_inventory;
    submodule_gitlinks: BTreeMap<Vec<u8>, String>, on_submodule_gitlinks, always_submodule_gitlinks;
    root_commits: Vec<RootCommit>, on_root_commits, always_root_commits;
    authorship: Authorship, on_authorship, always_authorship;
    commit_subjects: Vec<CommitSubject>, on_commit_subjects, always_commit_subjects;
}

/// An unconfigured operation fails loudly rather than inventing a value: a fake that answers
/// "no commits" or "clean" by default teaches a test the wrong thing, and absence is never
/// the same as a computed zero here.
fn unconfigured<T>(op: &str) -> GitResult<T> {
    Err(GitError::Unreadable {
        detail: format!("FakeGitBackend has no reply configured for `{op}`"),
    })
}

impl GitBackend for FakeGitBackend {
    fn version(&self, _ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        self.answer("version", &self.version, None, || unconfigured("version"))
    }

    fn repo_facts(&self, repo: &RepoHandle, _ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.answer("repo_facts", &self.repo_facts, Some(&repo.work_dir), || {
            unconfigured("repo_facts")
        })
    }

    fn ref_state(&self, repo: &RepoHandle, _ctx: &JobContext<'_>) -> GitResult<RefState> {
        self.answer("ref_state", &self.ref_state, Some(&repo.work_dir), || {
            unconfigured("ref_state")
        })
    }

    fn divergence(
        &self,
        repo: &RepoHandle,
        _state: &RefState,
        _ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.answer("divergence", &self.divergence, Some(&repo.work_dir), || {
            Ok(None)
        })
    }

    fn worktree_status(
        &self,
        repo: &RepoHandle,
        _opts: StatusOptions,
        _ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        self.answer(
            "worktree_status",
            &self.worktree_status,
            Some(&repo.work_dir),
            || unconfigured("worktree_status"),
        )
    }

    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        _ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.answer(
            "tracked_inventory",
            &self.tracked_inventory,
            Some(&repo.work_dir),
            || unconfigured("tracked_inventory"),
        )
    }

    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        _paths: &[Vec<u8>],
        _ctx: &JobContext<'_>,
    ) -> GitResult<BTreeMap<Vec<u8>, String>> {
        self.answer(
            "submodule_gitlinks",
            &self.submodule_gitlinks,
            Some(&repo.work_dir),
            || unconfigured("submodule_gitlinks"),
        )
    }

    fn root_commits(&self, repo: &RepoHandle, _ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.answer(
            "root_commits",
            &self.root_commits,
            Some(&repo.work_dir),
            || unconfigured("root_commits"),
        )
    }

    fn authorship(&self, repo: &RepoHandle, _ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.answer("authorship", &self.authorship, Some(&repo.work_dir), || {
            unconfigured("authorship")
        })
    }

    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        _limit: u32,
        _ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.answer(
            "commit_subjects",
            &self.commit_subjects,
            Some(&repo.work_dir),
            || unconfigured("commit_subjects"),
        )
    }
}

/// Wraps any backend and records which operations were asked for, in order.
///
/// This records *method calls*, not argv: plan 05's trait carries no argv. §3.2's neutralising
/// options and criterion 63's `-c safe.directory=<exact-path>` are proven directly in
/// `core/tests/git_invocation.rs`, against the argv builder itself.
#[derive(Debug)]
pub struct RecordingGitBackend<B> {
    inner: B,
    calls: Mutex<Vec<RecordedGitCall>>,
}

impl<B: GitBackend> RecordingGitBackend<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn calls(&self) -> Vec<RecordedGitCall> {
        self.calls.lock().map_or_else(|_| Vec::new(), |c| c.clone())
    }

    fn record(&self, op: &'static str, repo: Option<&Path>) {
        if let Ok(mut c) = self.calls.lock() {
            c.push(RecordedGitCall {
                op,
                repo: repo.map(Path::to_path_buf),
            });
        }
    }
}

impl<B: GitBackend> GitBackend for RecordingGitBackend<B> {
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        self.record("version", None);
        self.inner.version(ctx)
    }

    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.record("repo_facts", Some(&repo.work_dir));
        self.inner.repo_facts(repo, ctx)
    }

    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState> {
        self.record("ref_state", Some(&repo.work_dir));
        self.inner.ref_state(repo, ctx)
    }

    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.record("divergence", Some(&repo.work_dir));
        self.inner.divergence(repo, state, ctx)
    }

    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        self.record("worktree_status", Some(&repo.work_dir));
        self.inner.worktree_status(repo, opts, ctx)
    }

    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.record("tracked_inventory", Some(&repo.work_dir));
        self.inner.tracked_inventory(repo, ctx)
    }

    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<BTreeMap<Vec<u8>, String>> {
        self.record("submodule_gitlinks", Some(&repo.work_dir));
        self.inner.submodule_gitlinks(repo, paths, ctx)
    }

    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.record("root_commits", Some(&repo.work_dir));
        self.inner.root_commits(repo, ctx)
    }

    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.record("authorship", Some(&repo.work_dir));
        self.inner.authorship(repo, ctx)
    }

    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.record("commit_subjects", Some(&repo.work_dir));
        self.inner.commit_subjects(repo, limit, ctx)
    }
}

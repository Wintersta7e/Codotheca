//! The injection seam §15.2 requires, and the one real implementation.

use std::sync::Arc;
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::mount::StoreClass;

use super::error::{GitError, GitResult};
use super::exec::{GitExec, RunLimits};
use super::facts::{repo_facts, RepoFacts};
use super::history::{
    authorship, commit_subjects, root_commits, Authorship, CommitSubject, RootCommit,
};
use super::inventory::{submodule_gitlinks, tracked_inventory, TrackedInventory};
use super::refstate::{divergence, read_ref_state, Divergence, RefState};
use super::repo::{RepoHandle, StoreKey};
use super::slots::{GitSlots, JobClass};
use super::status::{worktree_status, StatusOptions, WorktreeStatus};
use super::version::{meets_floor, parse_version, GitVersion};

/// What one call is: its class, its cancellation token and its budget.
///
/// Budgets themselves belong to §4.1 and are chosen by the scheduler; the backend enforces
/// whatever this carries. `None` is J4's no-deadline case.
#[derive(Debug, Clone, Copy)]
pub struct JobContext<'a> {
    /// Which pool ceiling applies.
    pub job: JobClass,
    /// Cancellation for the whole scan run.
    pub cancel: &'a CancelToken,
    /// Wall-clock ceiling for each invocation this call makes.
    pub deadline: Option<Duration>,
}

impl<'a> JobContext<'a> {
    /// Assemble a context.
    #[must_use]
    pub fn new(job: JobClass, cancel: &'a CancelToken, deadline: Option<Duration>) -> Self {
        Self {
            job,
            cancel,
            deadline,
        }
    }

    fn limits(&self) -> RunLimits {
        RunLimits {
            deadline: self.deadline,
            tolerated_exit: None,
        }
    }
}

/// Everything the scanner is allowed to ask git for.
///
/// Every method is read-only: §17 gives phase 1 no destructive operation at all, and the
/// subcommand audit in `core/tests/git_readonly.rs` holds that mechanically.
pub trait GitBackend: Send + Sync + std::fmt::Debug {
    /// `git --version`, for the floor check and `app_meta.git_version`.
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion>;
    /// Bare, shallow, git dir, common dir.
    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts>;
    /// J1: ref state from file reads.
    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState>;
    /// Ahead/behind, or `None` when there is nothing to compare against.
    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>>;
    /// J2: one timestamped worktree observation.
    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus>;
    /// J3: tracked files and HEAD blob bytes.
    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory>;
    /// §4.4: the gitlink OID this repository's index records at each of `paths`, keyed by path.
    ///
    /// The scanner's, not a job's. `submodule_edge.gitlink_oid` (§1.9) has no other source —
    /// `ls-files -s` is the only read that reports a mode, and `TrackedInventory` discards it —
    /// and the scanner may not spawn git outside this seam (§15.2). A path with no gitlink is
    /// simply absent from the map; there is no placeholder OID.
    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<std::collections::BTreeMap<Vec<u8>, String>>;
    /// J4: the root set, with dates.
    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>>;
    /// J1.5: the full committer walk.
    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship>;
    /// J4: recent subjects, newest first.
    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>>;
}

/// Native git, under the slot caps.
#[derive(Debug, Clone)]
pub struct SystemGit {
    exec: Arc<GitExec>,
    slots: Arc<GitSlots>,
    clock: Arc<dyn Clock>,
}

impl SystemGit {
    /// Assemble the backend the app runs with.
    #[must_use]
    pub fn new(exec: Arc<GitExec>, slots: Arc<GitSlots>, clock: Arc<dyn Clock>) -> Self {
        Self { exec, slots, clock }
    }

    /// Run `body` holding one git slot for `repo`'s store.
    fn with_slot<T>(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
        body: impl FnOnce() -> GitResult<T>,
    ) -> GitResult<T> {
        let guard = self
            .slots
            .acquire(&repo.store, repo.store_class, ctx.job, ctx.cancel)?;
        let out = body();
        drop(guard);
        out
    }
}

impl GitBackend for SystemGit {
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        // `--version` needs no repository, so `-C` points at the empty hooks directory.
        let probe = RepoHandle::bare(
            self.exec.hooks_dir(),
            StoreKey::new("app"),
            StoreClass::Local,
        );
        let out = self.exec.run(
            &probe,
            &[std::ffi::OsStr::new("--version")],
            ctx.limits(),
            ctx.cancel,
        )?;
        parse_version(&out.stdout).ok_or(GitError::Missing)
    }

    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.with_slot(repo, ctx, || {
            repo_facts(&self.exec, repo, ctx.limits(), ctx.cancel)
        })
    }

    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState> {
        // File reads only, so no slot and no deferral: this is the read that *discovers* an
        // operation marker (§3.5).
        ctx.cancel.check()?;
        read_ref_state(repo, self.clock.as_ref())
    }

    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.with_slot(repo, ctx, || {
            divergence(&self.exec, repo, state, ctx.limits(), ctx.cancel)
        })
    }

    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        self.with_slot(repo, ctx, || {
            super::observe::defer_while_locked(repo, self.clock.as_ref(), ctx.cancel, &mut || {
                worktree_status(
                    &self.exec,
                    repo,
                    opts,
                    ctx.limits(),
                    ctx.cancel,
                    self.clock.as_ref(),
                )
            })
        })
    }

    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.with_slot(repo, ctx, || {
            super::observe::defer_while_locked(repo, self.clock.as_ref(), ctx.cancel, &mut || {
                tracked_inventory(
                    &self.exec,
                    repo,
                    ctx.limits(),
                    ctx.cancel,
                    self.clock.as_ref(),
                )
            })
        })
    }

    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<std::collections::BTreeMap<Vec<u8>, String>> {
        self.with_slot(repo, ctx, || {
            submodule_gitlinks(&self.exec, repo, paths, ctx.limits(), ctx.cancel)
        })
    }

    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.with_slot(repo, ctx, || {
            root_commits(&self.exec, repo, ctx.limits(), ctx.cancel)
        })
    }

    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.with_slot(repo, ctx, || {
            authorship(&self.exec, repo, ctx.limits(), ctx.cancel)
        })
    }

    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.with_slot(repo, ctx, || {
            commit_subjects(&self.exec, repo, limit, ctx.limits(), ctx.cancel)
        })
    }
}

/// Read the version and enforce the floor in one call — what startup uses (§3.1, §11.2).
pub fn require_floor(backend: &dyn GitBackend, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
    let v = backend.version(ctx)?;
    if meets_floor(&v) {
        Ok(v)
    } else {
        Err(GitError::TooOld { found: v.raw })
    }
}

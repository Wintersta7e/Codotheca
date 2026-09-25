//! §13 — the worker, seen by the rest of the core as an ordinary `GitBackend`.
//!
//! Every job plan 09 runs against a local repository runs against an in-distro one through this,
//! unchanged: same budgets, same freshness gate, same `JobOutcome`. The two failures that are
//! specific to a distro are the two §13 names — no git in the distro, and a distro that could
//! not be reached — and they map to different codes on purpose.

use crate::git::{
    Authorship, BlobBatch, CommitSubject, Divergence, GitBackend, GitError, GitResult, GitVersion,
    InterruptedOperation, JobContext, RefListing, RefState, RepoFacts, RepoHandle, RootCommit,
    StashEntries, StatusOptions, StoreKey, TrackedInventory, TreeEntry, UntrackedMode,
    WorktreeScan, WorktreeStatus,
};
use crate::mount::StoreClass;
use crate::wsl::conn::{WslError, WslWorker};
use crate::wsl::proto::{
    git_error_of, gitlinks_from_wire, WorkerGit, WorkerGitOp, WorkerJobClass, WorkerRepo,
    WorkerRequest,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A `GitBackend` whose every method runs inside one distro, through that distro's worker.
#[derive(Debug)]
pub struct WslGitBackend {
    worker: Arc<WslWorker>,
}

impl WslGitBackend {
    /// A backend that sends its git work to `worker`.
    #[must_use]
    pub const fn new(worker: Arc<WslWorker>) -> Self {
        Self { worker }
    }

    /// The distro this backend's git runs in.
    #[must_use]
    pub fn distro(&self) -> &str {
        self.worker.distro()
    }

    fn op<T: serde::de::DeserializeOwned>(
        &self,
        repo: &RepoHandle,
        op: WorkerGitOp,
        ctx: &JobContext<'_>,
    ) -> GitResult<T> {
        let value = self.raw(repo, op, ctx)?;
        serde_json::from_value(value).map_err(|e| GitError::Internal {
            detail: e.to_string(),
        })
    }

    fn raw(
        &self,
        repo: &RepoHandle,
        op: WorkerGitOp,
        ctx: &JobContext<'_>,
    ) -> GitResult<serde_json::Value> {
        if matches!(self.worker.git(), WorkerGit::Missing { .. }) {
            // No round trip: the distro told us in `Hello`, and asking again would only slow
            // the same answer down.
            return Err(GitError::Missing);
        }
        ctx.cancel.check()?;
        self.worker
            .call(&WorkerRequest::Git {
                repo: worker_repo(repo),
                op,
                job: WorkerJobClass::from_job_class(ctx.job),
                deadline_ms: ctx
                    .deadline
                    .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            })
            .map_err(wsl_to_git)
    }

    /// `git --version` needs no repository, and the worker builds its own handle from the mount
    /// table anyway, so the root stands in for one rather than a path being invented.
    fn version_probe(&self) -> RepoHandle {
        RepoHandle {
            work_dir: std::path::PathBuf::from("/"),
            git_dir: std::path::PathBuf::from("/"),
            common_dir: std::path::PathBuf::from("/"),
            store: StoreKey::new(format!("wsl:{}:?", self.distro())),
            store_class: StoreClass::Unknown,
            trusted: false,
        }
    }
}

/// The wire form of `repo`. Its store facts stay behind: the worker resolves those from its
/// own mount table.
#[must_use]
pub fn worker_repo(repo: &RepoHandle) -> WorkerRepo {
    WorkerRepo {
        work_dir: repo.work_dir.to_string_lossy().into_owned(),
        git_dir: repo.git_dir.to_string_lossy().into_owned(),
        common_dir: repo.common_dir.to_string_lossy().into_owned(),
        trusted: repo.trusted,
    }
}

/// A distro that could not be reached is **offline**, never gone: nothing observed the files, so
/// nothing may claim they are missing (§4.6, §6).
#[must_use]
pub fn wsl_to_git(err: WslError) -> GitError {
    match err {
        WslError::Fault(fault) => git_error_of(fault),
        // A distro that would not start and one the worker could not be installed into are the
        // same answer on purpose: in both, nothing looked at the files this generation.
        WslError::Launch { distro, detail } | WslError::Deploy { distro, detail } => {
            GitError::StoreOffline {
                detail: format!("{distro}: {detail}"),
            }
        }
        WslError::Closed => GitError::StoreOffline {
            detail: "the worker connection closed".to_owned(),
        },
        WslError::VersionMismatch { worker, core } => GitError::Internal {
            detail: format!("worker protocol {worker}, core {core}"),
        },
        WslError::Protocol { detail } => GitError::Internal { detail },
    }
}

impl GitBackend for WslGitBackend {
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        self.op(&self.version_probe(), WorkerGitOp::Version, ctx)
    }

    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.op(repo, WorkerGitOp::RepoFacts, ctx)
    }

    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState> {
        self.op(repo, WorkerGitOp::RefState, ctx)
    }

    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.op(
            repo,
            WorkerGitOp::Divergence {
                state: Box::new(state.clone()),
            },
            ctx,
        )
    }

    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        let untracked = matches!(opts.untracked, UntrackedMode::All);
        self.op(repo, WorkerGitOp::WorktreeStatus { untracked }, ctx)
    }

    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.op(repo, WorkerGitOp::TrackedInventory, ctx)
    }

    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<BTreeMap<Vec<u8>, String>> {
        // JSON has no byte-keyed map, so this one crosses as pairs and is folded back here.
        let value = self.raw(
            repo,
            WorkerGitOp::SubmoduleGitlinks {
                paths: paths.to_vec(),
            },
            ctx,
        )?;
        let pairs: Vec<(Vec<u8>, String)> =
            serde_json::from_value(value).map_err(|e| GitError::Internal {
                detail: e.to_string(),
            })?;
        Ok(gitlinks_from_wire(pairs))
    }

    fn remote_urls(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<(String, String)>> {
        self.op(repo, WorkerGitOp::RemoteUrls, ctx)
    }

    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.op(repo, WorkerGitOp::RootCommits, ctx)
    }

    fn unpushed_refs(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<String>> {
        self.op(repo, WorkerGitOp::UnpushedRefs, ctx)
    }

    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.op(repo, WorkerGitOp::Authorship, ctx)
    }

    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.op(repo, WorkerGitOp::CommitSubjects { limit }, ctx)
    }

    fn head_tree(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<TreeEntry>> {
        self.op(repo, WorkerGitOp::HeadTree, ctx)
    }

    fn read_blobs(
        &self,
        repo: &RepoHandle,
        oids: &[String],
        byte_cap: u64,
        budget_bytes: u64,
        ctx: &JobContext<'_>,
    ) -> GitResult<BlobBatch> {
        self.op(
            repo,
            WorkerGitOp::ReadBlobs {
                oids: oids.to_vec(),
                byte_cap,
                budget_bytes,
            },
            ctx,
        )
    }

    // D-11: the deletion analyser runs on the host's git only (`CoreHandler.git`); this backend
    // serves the scanner. Each of its six reads refuses **explicitly**, as an unreadable fact the
    // analyser maps to an unknown blocker, rather than answering from a worker that has no such
    // operation. The first job that needs one of them forwards it as a `WorkerGitOp` in its own
    // change (R169, R237).
    fn enumerate_refs(&self, _repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefListing> {
        host_only(ctx, "enumerate_refs")
    }

    fn stash_entries(&self, _repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<StashEntries> {
        host_only(ctx, "stash_entries")
    }

    fn worktree_scan(&self, _repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<WorktreeScan> {
        host_only(ctx, "worktree_scan")
    }

    fn objects_present(
        &self,
        _repo: &RepoHandle,
        _oids: &[String],
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<bool>> {
        host_only(ctx, "objects_present")
    }

    fn any_uncovered(
        &self,
        _repo: &RepoHandle,
        _roots: &[String],
        _covered: &[String],
        ctx: &JobContext<'_>,
    ) -> GitResult<bool> {
        host_only(ctx, "any_uncovered")
    }

    fn interrupted_ops(
        &self,
        _repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<InterruptedOperation>> {
        host_only(ctx, "interrupted_ops")
    }
}

/// D-11's refusal: the analyser's reads run on the host git, never through the worker.
fn host_only<T>(ctx: &JobContext<'_>, read: &str) -> GitResult<T> {
    ctx.cancel.check()?;
    Err(GitError::Unreadable {
        detail: format!(
            "{read} is the host-only deletion analyser's read; the WSL worker has none"
        ),
    })
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{wsl_to_git, WslGitBackend};
    use crate::cancel::CancelToken;
    use crate::git::{GitBackend, GitError, JobClass, JobContext, RepoHandle, StoreKey};
    use crate::mount::StoreClass;
    use crate::testing::wsl::LoopbackLauncher;
    use crate::testing::FakeGitBackend;
    use crate::wsl::conn::{WslError, WslWorkerPool};
    use crate::wsl::mounts::MountTable;
    use crate::wsl::proto::{WorkerFault, WorkerGit};
    use crate::wsl::serve::WorkerContext;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    const MOUNTINFO: &str = "28 1 8:32 / / rw - ext4 /dev/sdc rw\n";

    fn pool(presence: WorkerGit) -> WslWorkerPool {
        WslWorkerPool::new(Arc::new(LoopbackLauncher::new(move |distro: &str| {
            WorkerContext {
                distro: distro.to_owned(),
                git: Box::new(FakeGitBackend::new()),
                mounts: MountTable::from_mountinfo(MOUNTINFO),
                presence: presence.clone(),
            }
        })))
    }

    fn handle() -> RepoHandle {
        RepoHandle {
            work_dir: PathBuf::from("/home/me/widget"),
            git_dir: PathBuf::from("/home/me/widget/.git"),
            common_dir: PathBuf::from("/home/me/widget/.git"),
            store: StoreKey::new("wsl:alpha:/"),
            store_class: StoreClass::Local,
            trusted: false,
        }
    }

    #[test]
    fn a_distro_without_git_reports_git_missing_for_every_request() {
        // §13: the worker reports GIT_MISSING for that location, and the distro's repositories
        // render with an explained error rather than silently missing.
        let pool = pool(WorkerGit::Missing {
            detail: "not on PATH".to_owned(),
        });
        let backend = WslGitBackend::new(pool.get("alpha").expect("connects"));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);

        let err = backend.ref_state(&handle(), &ctx).expect_err("no git");
        assert!(matches!(err, GitError::Missing));
        assert_eq!(err.protocol_code(), Some("GIT_MISSING"));
        assert!(matches!(
            backend.version(&ctx).expect_err("no git"),
            GitError::Missing
        ));
        pool.shutdown_all();
    }

    #[test]
    fn a_fault_is_rebuilt_as_the_error_the_distro_raised() {
        assert!(matches!(
            wsl_to_git(WslError::Fault(WorkerFault::TornRead)),
            GitError::TornRead
        ));
        assert_eq!(
            wsl_to_git(WslError::Fault(WorkerFault::Untrusted {
                path: "/home/me/widget".to_owned()
            }))
            .protocol_code(),
            Some("UNTRUSTED_REPO")
        );
    }

    #[test]
    fn an_unreachable_distro_is_offline_and_never_gone() {
        // "Never claim currency you do not have": nothing observed the files, so nothing may say
        // they are missing. §11.5's STORE_OFFLINE prose is the honest one — frozen, not rotting.
        for err in [
            WslError::Launch {
                distro: "alpha".to_owned(),
                detail: "stopped".to_owned(),
            },
            WslError::Deploy {
                distro: "alpha".to_owned(),
                detail: "no tee".to_owned(),
            },
            WslError::Closed,
        ] {
            let mapped = wsl_to_git(err);
            assert_eq!(mapped.protocol_code(), Some("STORE_OFFLINE"));
            assert!(
                !mapped.implies_absent(),
                "an unreachable distro must not imply absence"
            );
        }
    }

    #[test]
    fn a_protocol_failure_is_internal_and_not_a_repository_problem() {
        assert_eq!(
            wsl_to_git(WslError::VersionMismatch { worker: 9, core: 1 }).protocol_code(),
            Some("INTERNAL")
        );
        assert_eq!(
            wsl_to_git(WslError::Protocol {
                detail: "bad frame".to_owned()
            })
            .protocol_code(),
            Some("INTERNAL")
        );
    }

    #[test]
    fn the_backend_names_the_distro_it_speaks_to() {
        let pool = pool(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let backend = WslGitBackend::new(pool.get("beta").expect("connects"));
        assert_eq!(backend.distro(), "beta");
        pool.shutdown_all();
    }

    #[test]
    fn a_gitlink_map_crosses_the_wire_and_comes_back_byte_for_byte() {
        // The one result whose wire shape differs from its in-process shape, because JSON has
        // no byte-keyed map. Both directions run here against the real serve loop, so a lossy
        // conversion on either side — which would silently rename a submodule — shows up.
        let mut expected = BTreeMap::new();
        expected.insert(vec![0x6c, 0xFF, 0x2F, 0x61], "0123abc".to_owned());
        expected.insert(b"lib/thing".to_vec(), "89defab".to_owned());
        let paths: Vec<Vec<u8>> = expected.keys().cloned().collect();

        let wanted = expected.clone();
        let pool = WslWorkerPool::new(Arc::new(LoopbackLauncher::new(move |distro: &str| {
            let fake = FakeGitBackend::new();
            fake.always_submodule_gitlinks(crate::testing::GitReply::Ok(wanted.clone()));
            WorkerContext {
                distro: distro.to_owned(),
                git: Box::new(fake),
                mounts: MountTable::from_mountinfo(MOUNTINFO),
                presence: WorkerGit::Present {
                    version: "2.43.0".to_owned(),
                },
            }
        })));
        let backend = WslGitBackend::new(pool.get("alpha").expect("connects"));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);

        let got = backend
            .submodule_gitlinks(&handle(), &paths, &ctx)
            .expect("answers");
        assert_eq!(got, expected);
        pool.shutdown_all();
    }
}

//! §13 — the worker side of the connection.
//!
//! Everything here runs *inside* the distro. It writes `Hello` before reading a byte, so the
//! core learns the protocol version and whether git exists without spending a request on it, and
//! it answers exactly one request at a time: the core writes a small frame and then reads, which
//! is the shape the `cat-file --batch-check` deadlock did not have.

use crate::cancel::CancelToken;
use crate::git::{
    GitBackend, GitResult, JobClass, JobContext, RepoHandle, StatusOptions, StoreKey,
};
use crate::index::path::PathPlatform;
use crate::mount::MountResolver;
use crate::paths::path_key;
use crate::proto::frame::{read_frame, FrameError};
use crate::proto::wire::RequestId;
use crate::scan::discover::{ProbeCtx, RepoCandidate};
use crate::scan::links::LinkPolicy;
use crate::scan::skiplist::SkipList;
use crate::scan::walk::{walk_root, WalkCtx};
use crate::scan::{WalkEvent, WalkOptions};
use crate::wsl::mounts::{class_slug, DistroMountResolver, MountTable, MountVerdict};
use crate::wsl::proto::{
    fault_of, gitlinks_to_wire, write_worker_frame, WalkRequest, WalkSummary, WireMountFacts,
    WorkerCall, WorkerEvent, WorkerFault, WorkerGit, WorkerGitOp, WorkerOutbound, WorkerRepo,
    WorkerRepoFound, WorkerRequest, WORKER_PROTOCOL_VERSION,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const WORKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// stdout is the frame stream. If something else has already claimed it the worker must not
/// write a single byte, because a half-frame is worse than no worker.
pub const EXIT_STDOUT_TAKEN: u8 = 4;

/// How a handler answers. `Send` is not decoration: the in-distro walk hands this to
/// `walk_root`, whose sink must be `Send + Sync`, and a bare `&mut dyn FnMut` is neither.
pub type Emit<'a> = dyn FnMut(&WorkerOutbound) -> std::io::Result<()> + Send + 'a;

/// Everything the worker needs, assembled once at startup.
pub struct WorkerContext {
    pub distro: String,
    pub git: Box<dyn GitBackend>,
    pub mounts: MountTable,
    pub presence: WorkerGit,
}

impl std::fmt::Debug for WorkerContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerContext")
            .field("distro", &self.distro)
            .field("presence", &self.presence)
            .finish_non_exhaustive()
    }
}

/// §13: a distro without git reports it, once, instead of every repository failing separately.
#[must_use]
pub fn probe_git(git: &dyn GitBackend) -> WorkerGit {
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Interactive, &cancel, None);
    match git.version(&ctx) {
        Ok(v) => WorkerGit::Present { version: v.raw },
        Err(err) => WorkerGit::Missing {
            detail: err.to_string(),
        },
    }
}

#[must_use]
pub fn hello_frame(ctx: &WorkerContext, pid: u32) -> WorkerOutbound {
    WorkerOutbound::Hello {
        protocol_version: WORKER_PROTOCOL_VERSION,
        worker_version: WORKER_VERSION.to_owned(),
        git: ctx.presence.clone(),
        pid,
    }
}

/// The handle is built from what the request already carries, so the worker does not spend a
/// `rev-parse` re-deriving what the core discovered during the walk.
///
/// R4: the scheduling class is `core::mount::StoreClass` on both sides of the hop. Plan 05's
/// two-way `{ Fast, Slow }` is deleted, so there is nothing left to convert between.
#[must_use]
pub fn repo_handle(ctx: &WorkerContext, repo: &WorkerRepo) -> RepoHandle {
    let facts = ctx.mounts.facts_for(&ctx.distro, &repo.work_dir);
    RepoHandle {
        work_dir: PathBuf::from(&repo.work_dir),
        git_dir: PathBuf::from(&repo.git_dir),
        common_dir: PathBuf::from(&repo.common_dir),
        store: StoreKey::new(facts.store_key),
        store_class: facts.class,
        trusted: repo.trusted,
    }
}

fn encode<T: serde::Serialize>(result: GitResult<T>) -> Result<serde_json::Value, WorkerFault> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|e| WorkerFault::Internal {
            detail: e.to_string(),
        }),
        Err(err) => Err(fault_of(&err)),
    }
}

fn run_git(
    ctx: &WorkerContext,
    repo: &WorkerRepo,
    op: &WorkerGitOp,
    job: JobClass,
    deadline_ms: Option<u64>,
) -> Result<serde_json::Value, WorkerFault> {
    if matches!(ctx.presence, WorkerGit::Missing { .. }) {
        return Err(WorkerFault::GitMissing);
    }
    let handle = repo_handle(ctx, repo);
    let cancel = CancelToken::new();
    let deadline = deadline_ms.map(std::time::Duration::from_millis);
    let jc = JobContext::new(job, &cancel, deadline);
    let git = ctx.git.as_ref();
    match op {
        WorkerGitOp::Version => encode(git.version(&jc)),
        WorkerGitOp::RepoFacts => encode(git.repo_facts(&handle, &jc)),
        WorkerGitOp::RefState => encode(git.ref_state(&handle, &jc)),
        WorkerGitOp::Divergence { state } => encode(git.divergence(&handle, state, &jc)),
        WorkerGitOp::WorktreeStatus { untracked } => {
            let opts = if *untracked {
                StatusOptions::full()
            } else {
                StatusOptions::degraded()
            };
            encode(git.worktree_status(&handle, opts, &jc))
        }
        WorkerGitOp::TrackedInventory => encode(git.tracked_inventory(&handle, &jc)),
        // JSON has no byte-keyed map, so this one result is re-shaped rather than encoded
        // straight. `gitlinks_to_wire` is the only place that shape is built.
        WorkerGitOp::SubmoduleGitlinks { paths } => encode(
            git.submodule_gitlinks(&handle, paths, &jc)
                .map(|m| gitlinks_to_wire(&m)),
        ),
        WorkerGitOp::RootCommits => encode(git.root_commits(&handle, &jc)),
        WorkerGitOp::Authorship => encode(git.authorship(&handle, &jc)),
        WorkerGitOp::CommitSubjects { limit } => encode(git.commit_subjects(&handle, *limit, &jc)),
    }
}

#[must_use]
pub fn repo_found_of(ctx: &WorkerContext, candidate: &RepoCandidate) -> WorkerRepoFound {
    let work_dir = candidate.path.to_string_lossy().into_owned();
    let facts = ctx.mounts.facts_for(&ctx.distro, &work_dir);
    WorkerRepoFound {
        work_dir,
        git_dir: candidate.git_dir.to_string_lossy().into_owned(),
        common_dir: candidate.common_dir.to_string_lossy().into_owned(),
        kind: candidate.kind.as_str().to_owned(),
        store_key: facts.store_key,
        volume_key: facts.volume_key,
        store_class: class_slug(facts.class).to_owned(),
    }
}

/// Runs plan 07's walk inside the distro and streams what it finds.
///
/// The one behaviour that is not plan 07's: a root standing on a Windows-backed filesystem is
/// refused by **type** (§4.5) and named, because those bytes belong to the native walk.
pub fn walk(
    ctx: &WorkerContext,
    request: &WalkRequest,
    id: RequestId,
    emit: &mut Emit<'_>,
) -> std::io::Result<WalkSummary> {
    if let MountVerdict::SkipWindowsBacked {
        mount_point,
        fstype,
    } = ctx.mounts.verdict_for(&request.root)
    {
        emit(&WorkerOutbound::Event {
            id,
            event: WorkerEvent::SkippedMount {
                mount_point,
                fstype,
            },
        })?;
        return Ok(WalkSummary {
            walked_dirs: 0,
            found_repos: 0,
            cancelled: false,
        });
    }

    let opts = WalkOptions {
        // One thread. The distro's cost is the git work, not the walk — which measured at 101k
        // dirs/s — and every event has to pass through one frame writer anyway.
        threads: 1,
        follow_links: request.follow_links,
        descend_into_repos: request.descend_into_repos,
        bare_candidates: request.bare_candidates,
    };
    let skip = SkipList::with_user_entries(&request.skip_extra);
    let cancel = CancelToken::new();
    let root = PathBuf::from(&request.root);
    let root_facts = ctx.mounts.facts_for(&ctx.distro, &request.root);
    // The resolver gets the *live* table. An empty one would answer `wsl:<distro>:?` for every
    // target, which never equals the root's store, and the link policy would then refuse every
    // link inside the distro while reporting that it had judged them.
    let resolver: Arc<dyn MountResolver> = Arc::new(DistroMountResolver::new(
        ctx.distro.clone(),
        ctx.mounts.clone(),
    ));
    let mut root_stores = BTreeSet::new();
    root_stores.insert(root_facts.store_key.clone());
    let links = Arc::new(LinkPolicy::new(
        request.follow_links,
        // R2: `path_key` takes the platform explicitly. Inside a distro the answer is always
        // `Unix` — two names differing only in case are two directories, whatever the host is.
        vec![path_key(&root, PathPlatform::Unix)],
        root_stores,
        resolver,
    ));
    let probe = ProbeCtx::new(
        ctx.git.as_ref(),
        StoreKey::new(root_facts.store_key),
        root_facts.class,
        &cancel,
    );
    let walk_ctx = WalkCtx {
        opts: &opts,
        skip: &skip,
        probe: &probe,
        links,
    };

    // `WalkSink` is `Fn` and `Send + Sync`, so the frame writer needs a lock and the counter
    // needs an atomic. A write that fails is counted rather than swallowed: the summary would
    // otherwise report repositories the core never received.
    let out = std::sync::Mutex::new(emit);
    let failed = AtomicU64::new(0);
    let sink = |event: WalkEvent| {
        let framed = match event {
            WalkEvent::Repo(candidate) => Some(WorkerEvent::Repo(repo_found_of(ctx, &candidate))),
            WalkEvent::Problem(problem) => Some(WorkerEvent::Problem {
                kind: problem.kind.as_str().to_owned(),
                path_display: problem.path_display,
                detail: problem.detail,
            }),
            WalkEvent::Progress {
                walked_dirs,
                found_repos,
            } => Some(WorkerEvent::Progress {
                walked_dirs,
                found_repos,
            }),
            // The distro never registers a bridge inside itself, and the core owns identity,
            // submodule edges and the raw directory counter: those events end here.
            WalkEvent::WslBridge { .. }
            | WalkEvent::Discovered(_)
            | WalkEvent::SubmoduleEdge(_)
            | WalkEvent::Walked { .. } => None,
        };
        let Some(event) = framed else { return };
        let Ok(mut guard) = out.lock() else {
            failed.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let write: &mut Emit<'_> = &mut *guard;
        if write(&WorkerOutbound::Event { id, event }).is_err() {
            failed.fetch_add(1, Ordering::Relaxed);
        }
    };

    let stats = walk_root(&root, &walk_ctx, &sink);
    if failed.load(Ordering::Relaxed) > 0 {
        return Err(std::io::Error::other(
            "the frame stream closed while the walk was reporting",
        ));
    }
    Ok(WalkSummary {
        walked_dirs: stats.walked_dirs,
        found_repos: stats.repos_found,
        cancelled: stats.cancelled,
    })
}

/// Answers one call. `Ok(false)` means the loop should stop.
pub fn handle(
    ctx: &WorkerContext,
    call: &WorkerCall,
    emit: &mut Emit<'_>,
) -> std::io::Result<bool> {
    let id = call.id;
    let outcome: Result<serde_json::Value, WorkerFault> = match &call.request {
        WorkerRequest::Ping => Ok(serde_json::json!({ "pong": true })),
        WorkerRequest::Mounts { path } => {
            let facts = ctx.mounts.facts_for(&ctx.distro, path);
            serde_json::to_value(WireMountFacts {
                store_key: facts.store_key,
                volume_key: facts.volume_key,
                class: class_slug(facts.class).to_owned(),
            })
            .map_err(|e| WorkerFault::Internal {
                detail: e.to_string(),
            })
        }
        WorkerRequest::Walk(request) => match walk(ctx, request, id, emit) {
            Ok(summary) => serde_json::to_value(summary).map_err(|e| WorkerFault::Internal {
                detail: e.to_string(),
            }),
            Err(err) => Err(WorkerFault::Internal {
                detail: err.to_string(),
            }),
        },
        WorkerRequest::Git {
            repo,
            op,
            job,
            deadline_ms,
        } => run_git(ctx, repo, op, job.to_job_class(), *deadline_ms),
        WorkerRequest::Shutdown => {
            emit(&WorkerOutbound::Reply {
                id,
                ok: serde_json::json!({ "bye": true }),
            })?;
            return Ok(false);
        }
    };
    match outcome {
        Ok(ok) => emit(&WorkerOutbound::Reply { id, ok })?,
        Err(fault) => emit(&WorkerOutbound::Fail { id, fault })?,
    }
    Ok(true)
}

/// Writes `Hello`, then answers frames until stdin closes or a `Shutdown` arrives.
pub fn serve<R: std::io::Read, W: std::io::Write + Send>(
    ctx: &WorkerContext,
    input: &mut R,
    output: &mut W,
    pid: u32,
) -> std::io::Result<()> {
    let mut emit = |frame: &WorkerOutbound| write_worker_frame(output, frame);
    emit(&hello_frame(ctx, pid))?;
    let mut buf = Vec::new();
    loop {
        match read_frame(input, &mut buf) {
            Ok(()) => {}
            Err(FrameError::Eof) => return Ok(()),
            Err(other) => return Err(crate::wsl::proto::frame_io_error(other)),
        }
        let call: WorkerCall = match serde_json::from_slice(&buf) {
            Ok(call) => call,
            Err(err) => {
                // A frame the worker cannot parse is a protocol fault, not a reason to die: the
                // core is entitled to a diagnosis on stderr and a live connection.
                eprintln!("codotheca-worker: unparseable frame: {err}");
                continue;
            }
        };
        if !handle(ctx, &call, &mut emit)? {
            return Ok(());
        }
    }
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{serve, WorkerContext};
    use crate::proto::frame::read_frame;
    use crate::proto::wire::RequestId;
    use crate::testing::FakeGitBackend;
    use crate::wsl::mounts::MountTable;
    use crate::wsl::proto::{
        write_worker_call, WorkerCall, WorkerGit, WorkerOutbound, WorkerRequest,
        WORKER_PROTOCOL_VERSION,
    };

    const FIXTURE: &str = "28 1 8:32 / / rw - ext4 /dev/sdc rw\n";

    pub(super) fn context(presence: WorkerGit) -> WorkerContext {
        WorkerContext {
            distro: "alpha".to_owned(),
            git: Box::new(FakeGitBackend::new()),
            mounts: MountTable::from_mountinfo(FIXTURE),
            presence,
        }
    }

    pub(super) fn drive(ctx: &WorkerContext, calls: &[WorkerCall]) -> Vec<WorkerOutbound> {
        let mut input = Vec::new();
        for call in calls {
            write_worker_call(&mut input, call).expect("frames");
        }
        let mut output = Vec::new();
        serve(ctx, &mut input.as_slice(), &mut output, 99).expect("serves");

        let mut frames = Vec::new();
        let mut cursor = output.as_slice();
        let mut buf = Vec::new();
        while read_frame(&mut cursor, &mut buf).is_ok() {
            frames.push(serde_json::from_slice(&buf).expect("parses"));
        }
        frames
    }

    #[test]
    fn hello_comes_first_and_before_anything_is_read() {
        let ctx = context(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let frames = drive(&ctx, &[]);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            WorkerOutbound::Hello {
                protocol_version,
                git,
                pid,
                ..
            } => {
                assert_eq!(*protocol_version, WORKER_PROTOCOL_VERSION);
                assert_eq!(*pid, 99);
                assert!(matches!(git, WorkerGit::Present { .. }));
            }
            other => panic!("first frame was {other:?}"),
        }
    }

    #[test]
    fn a_distro_without_git_says_so_in_hello_rather_than_failing_to_start() {
        let ctx = context(WorkerGit::Missing {
            detail: "not on PATH".to_owned(),
        });
        let frames = drive(&ctx, &[]);
        match &frames[0] {
            WorkerOutbound::Hello {
                git: WorkerGit::Missing { detail },
                ..
            } => {
                assert_eq!(detail, "not on PATH");
            }
            other => panic!("first frame was {other:?}"),
        }
    }

    #[test]
    fn a_ping_is_answered_on_its_own_id() {
        let ctx = context(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let frames = drive(
            &ctx,
            &[WorkerCall {
                id: RequestId(11),
                request: WorkerRequest::Ping,
            }],
        );
        assert_eq!(frames.len(), 2);
        match &frames[1] {
            WorkerOutbound::Reply { id, ok } => {
                assert_eq!(id.0, 11);
                assert_eq!(ok["pong"], true);
            }
            other => panic!("second frame was {other:?}"),
        }
    }

    #[test]
    fn mounts_answers_with_the_store_identity_the_core_will_persist() {
        let ctx = context(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let frames = drive(
            &ctx,
            &[WorkerCall {
                id: RequestId(3),
                request: WorkerRequest::Mounts {
                    path: "/home/me/widget".to_owned(),
                },
            }],
        );
        match &frames[1] {
            WorkerOutbound::Reply { ok, .. } => {
                assert_eq!(ok["store_key"], "wsl:alpha:/");
                assert_eq!(ok["class"], "local");
                assert_eq!(ok["volume_key"], "wsl-distro:alpha:/");
            }
            other => panic!("second frame was {other:?}"),
        }
    }

    #[test]
    fn a_git_request_without_git_fails_as_git_missing_and_the_loop_keeps_serving() {
        let ctx = context(WorkerGit::Missing {
            detail: "not on PATH".to_owned(),
        });
        let frames = drive(
            &ctx,
            &[
                WorkerCall {
                    id: RequestId(1),
                    request: WorkerRequest::Git {
                        repo: crate::wsl::proto::WorkerRepo {
                            work_dir: "/home/me/widget".to_owned(),
                            git_dir: "/home/me/widget/.git".to_owned(),
                            common_dir: "/home/me/widget/.git".to_owned(),
                            trusted: false,
                        },
                        op: crate::wsl::proto::WorkerGitOp::RefState,
                        job: crate::wsl::proto::WorkerJobClass::Background,
                        deadline_ms: Some(50),
                    },
                },
                WorkerCall {
                    id: RequestId(2),
                    request: WorkerRequest::Ping,
                },
            ],
        );
        assert!(matches!(
            &frames[1],
            WorkerOutbound::Fail {
                fault: crate::wsl::proto::WorkerFault::GitMissing,
                ..
            }
        ));
        assert!(matches!(&frames[2], WorkerOutbound::Reply { .. }));
    }

    #[test]
    fn the_walk_finds_a_repository_and_reports_its_store() {
        use crate::wsl::proto::{WalkRequest, WorkerEvent};

        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("p");
        std::fs::create_dir_all(repo.join(".git")).expect("mkdir");
        let root = repo
            .parent()
            .expect("has a parent")
            .to_string_lossy()
            .into_owned();

        let ctx = context(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let frames = drive(
            &ctx,
            &[WorkerCall {
                id: RequestId(5),
                request: WorkerRequest::Walk(WalkRequest {
                    root,
                    follow_links: false,
                    descend_into_repos: false,
                    bare_candidates: false,
                    skip_extra: Vec::new(),
                }),
            }],
        );

        let found: Vec<_> = frames
            .iter()
            .filter_map(|f| match f {
                WorkerOutbound::Event {
                    event: WorkerEvent::Repo(r),
                    ..
                } => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(
            found.len(),
            1,
            "expected exactly one repository, got {frames:?}"
        );
        assert!(found[0].work_dir.ends_with('p'));
        assert_eq!(found[0].kind, "worktree");
        // The mount table maps *Linux* paths, because the worker only ever runs inside a distro.
        // On a Windows host the temporary root is `C:\…`, which no distro mount covers, so the
        // key is honestly `wsl:alpha:?` — the "not determined" answer, not a wrong one. The
        // resolved key can only be asserted where the root is a path the table can cover.
        #[cfg(unix)]
        assert_eq!(found[0].store_key, "wsl:alpha:/");

        match frames.last().expect("has a last frame") {
            WorkerOutbound::Reply { id, ok } => {
                assert_eq!(id.0, 5);
                assert_eq!(ok["found_repos"], 1);
                assert_eq!(ok["cancelled"], false);
            }
            other => panic!("last frame was {other:?}"),
        }
    }

    #[test]
    fn a_windows_backed_mount_is_named_and_not_walked() {
        use crate::wsl::proto::{WalkRequest, WorkerEvent};

        // A 9p mount at an arbitrary point: §4.5 forbids deciding this by the name `/mnt/c`.
        let ctx = WorkerContext {
            distro: "alpha".to_owned(),
            git: Box::new(FakeGitBackend::new()),
            mounts: MountTable::from_mountinfo(
                "28 1 8:32 / / rw - ext4 /dev/sdc rw\n64 28 0:64 / /opt/win rw - 9p C:\\134 rw\n",
            ),
            presence: WorkerGit::Present {
                version: "2.43.0".to_owned(),
            },
        };
        let frames = drive(
            &ctx,
            &[WorkerCall {
                id: RequestId(6),
                request: WorkerRequest::Walk(WalkRequest {
                    root: "/opt/win".to_owned(),
                    follow_links: false,
                    descend_into_repos: false,
                    bare_candidates: false,
                    skip_extra: Vec::new(),
                }),
            }],
        );

        let skipped: Vec<_> = frames
            .iter()
            .filter_map(|f| match f {
                WorkerOutbound::Event {
                    event:
                        WorkerEvent::SkippedMount {
                            mount_point,
                            fstype,
                        },
                    ..
                } => Some((mount_point.clone(), fstype.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, vec![("/opt/win".to_owned(), "9p".to_owned())]);
        assert!(!frames.iter().any(|f| matches!(
            f,
            WorkerOutbound::Event {
                event: WorkerEvent::Repo(_),
                ..
            }
        )));
    }

    #[test]
    fn shutdown_replies_and_then_stops_reading() {
        let ctx = context(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let frames = drive(
            &ctx,
            &[
                WorkerCall {
                    id: RequestId(1),
                    request: WorkerRequest::Shutdown,
                },
                WorkerCall {
                    id: RequestId(2),
                    request: WorkerRequest::Ping,
                },
            ],
        );
        // Hello, the shutdown reply, and nothing after it.
        assert_eq!(frames.len(), 2);
    }
}

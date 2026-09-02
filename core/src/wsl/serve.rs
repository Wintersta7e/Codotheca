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
use crate::proto::frame::{read_frame, FrameError};
use crate::wsl::mounts::{class_slug, MountTable};
use crate::wsl::proto::{
    fault_of, gitlinks_to_wire, write_worker_frame, WireMountFacts, WorkerCall, WorkerFault,
    WorkerGit, WorkerGitOp, WorkerOutbound, WorkerRepo, WorkerRequest, WORKER_PROTOCOL_VERSION,
};
use std::path::PathBuf;

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
        WorkerRequest::Walk(_) => Err(WorkerFault::Internal {
            detail: "walk is not implemented yet".to_owned(),
        }),
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

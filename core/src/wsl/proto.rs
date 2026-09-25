//! §13 — the worker's vocabulary.
//!
//! The framing, the 8 MiB ceiling and the envelope shape are plan 03's, unchanged: this is "the
//! same protocol" in the sense that matters. The command names are not the renderer's, because
//! the renderer may never reach `walk` or `git`; §2.4's surface stays exactly as wide as it is.
//! `protocol/schema/protocol.json` is the renderer-facing contract and declaring these there
//! would generate TypeScript nobody imports and widen a surface §2.4 deliberately closes.

// `core::git`'s submodules are private and every name below is re-exported from the module
// root, so `crate::git::…` is the path even inside the crate.
use crate::git::{BusyMarker, GitError, JobClass, RefState};
use crate::proto::frame::{write_frame, FrameError};
use crate::proto::wire::RequestId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bumped when the frames below change shape. The core refuses a worker that does not match,
/// which is what makes a stale copy inside a distro a diagnosable failure instead of a silent
/// misparse.
pub const WORKER_PROTOCOL_VERSION: u32 = 1;

/// One request from the core to a worker, tagged with the id its answer will echo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerCall {
    /// Chosen by the core per request; every `Event`, `Reply` or `Fail` answering it carries it.
    pub id: RequestId,
    /// What the worker is asked to do.
    pub request: WorkerRequest,
}

/// Every command the worker answers. None of them is on the renderer's §2.4 surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerRequest {
    /// A liveness check, answered with `{"pong": true}`.
    Ping,
    /// The store facts for one path inside the distro, answered as a `WireMountFacts`.
    Mounts {
        /// A Linux path inside the distro.
        path: String,
    },
    /// Plan 07's walk over one root inside the distro: streamed as `WorkerEvent`s, answered
    /// with a `WalkSummary`.
    Walk(WalkRequest),
    /// One `GitBackend` method against one in-distro repository, answered with that method's
    /// result as JSON.
    Git {
        /// The repository, as the core discovered it.
        repo: WorkerRepo,
        /// Which `GitBackend` method to run.
        op: WorkerGitOp,
        /// The scheduling class the core's job runs under.
        job: WorkerJobClass,
        /// §4.1's per-job budget, carried across so a slow in-distro repository is bounded on
        /// the same terms as a slow local one.
        deadline_ms: Option<u64>,
    },
    /// Answered with `{"bye": true}`, after which the worker stops reading.
    Shutdown,
}

/// The in-distro walk's root and the `WalkOptions` that cross. The thread count does not: the
/// worker always walks on one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkRequest {
    /// The Linux path the walk starts from.
    pub root: String,
    /// Whether the walk follows symbolic links (§4.3).
    pub follow_links: bool,
    /// Whether the walk keeps descending below a directory it found a repository in.
    pub descend_into_repos: bool,
    /// Whether a directory shaped like a bare repository is probed as one (§4.2).
    pub bare_candidates: bool,
    /// Entries appended to the default skip list for this walk.
    pub skip_extra: Vec<String>,
}

/// The wire form of `RepoHandle`, minus the store facts the worker resolves for itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRepo {
    /// The directory git runs in (`-C`), as a Linux path.
    pub work_dir: String,
    /// This checkout's git dir.
    pub git_dir: String,
    /// The shared git dir: refs, packed-refs, config, objects.
    pub common_dir: String,
    /// True when `location.trusted_at` is non-NULL, which adds `-c safe.directory=<path>`.
    pub trusted: bool,
}

/// One `GitBackend` method, named.
///
/// Every method has a variant: a backend that answered nine of ten would compile only because
/// the tenth was written to fail, which is the shape of a seam that passes its tests and breaks
/// at assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerGitOp {
    /// `git --version`.
    Version,
    /// Bare, shallow, git dir, common dir.
    RepoFacts,
    /// J1: ref state from file reads.
    RefState,
    /// Ahead/behind, or `None` when there is nothing to compare against.
    Divergence {
        /// The ref state the comparison is made from.
        state: Box<RefState>,
    },
    /// J2: one timestamped worktree observation.
    WorktreeStatus {
        /// `true` is `StatusOptions::full`; `false` is the degraded read that lists no
        /// untracked files.
        untracked: bool,
    },
    /// J3: tracked files and HEAD blob bytes.
    TrackedInventory,
    /// §4.4. The paths are raw bytes because a Linux path is not a `String`.
    SubmoduleGitlinks {
        /// The submodule paths whose gitlink OIDs are wanted.
        paths: Vec<Vec<u8>>,
    },
    /// §1.1's remote evidence. A unit variant: the argv is the core's own builder, run inside
    /// the distro by the worker's `SystemGit`.
    RemoteUrls,
    /// J4: the root set, with dates.
    RootCommits,
    /// [p2-24b] §24.7A's reachability walk, run inside the distro.
    UnpushedRefs,
    /// J1.5: the full committer walk.
    Authorship,
    /// J4: recent subjects, newest first.
    CommitSubjects {
        /// The most subjects to return.
        limit: u32,
    },
    /// §29.1's HEAD enumeration, run inside the distro.
    HeadTree,
    /// §29.6's blob read. `byte_cap` crosses because it is the **caller's** cap: a blob over it
    /// is recorded at its size with its body discarded, and doing that inside the distro is what
    /// keeps the body off this wire.
    ReadBlobs {
        /// The blob object ids to read.
        oids: Vec<String>,
        /// Bytes kept per blob; a larger one is recorded at its size with no body.
        byte_cap: u64,
        /// Bytes read across the whole batch before it stops.
        budget_bytes: u64,
    },
}

/// The wire form of `JobClass`, so in-distro git runs under the class the core's job has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerJobClass {
    /// `JobClass::Interactive`: a visible tile's request.
    Interactive,
    /// `JobClass::Background`: ordinary scan work.
    Background,
    /// `JobClass::History`: J4, the history walk.
    History,
}

impl WorkerJobClass {
    /// The in-process `JobClass` this names.
    #[must_use]
    pub const fn to_job_class(self) -> JobClass {
        match self {
            Self::Interactive => JobClass::Interactive,
            Self::Background => JobClass::Background,
            Self::History => JobClass::History,
        }
    }

    /// The wire form of `job`.
    #[must_use]
    pub const fn from_job_class(job: JobClass) -> Self {
        match job {
            JobClass::Interactive => Self::Interactive,
            JobClass::Background => Self::Background,
            JobClass::History => Self::History,
        }
    }
}

/// Every frame the worker writes on its stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerOutbound {
    /// The first frame, written before a byte is read.
    Hello {
        /// The worker's `WORKER_PROTOCOL_VERSION`; the core refuses one that differs.
        protocol_version: u32,
        /// The crate version the worker was built from.
        worker_version: String,
        /// Whether the distro has a usable git.
        git: WorkerGit,
        /// The worker's process id inside the distro.
        pid: u32,
    },
    /// One result of a request still in flight. Only `Walk` streams these.
    Event {
        /// The request this event belongs to.
        id: RequestId,
        /// What the walk reported.
        event: WorkerEvent,
    },
    /// A request's success, which ends it.
    Reply {
        /// The request answered.
        id: RequestId,
        /// The result, as the JSON of the type the request's method returns.
        ok: serde_json::Value,
    },
    /// A request's failure, which ends it.
    Fail {
        /// The request answered.
        id: RequestId,
        /// The error, in its wire form.
        fault: WorkerFault,
    },
}

/// Whether the distro has a usable git, decided once at startup and reported in `Hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "git", rename_all = "snake_case")]
pub enum WorkerGit {
    /// `git --version` answered.
    Present {
        /// The version line git printed.
        version: String,
    },
    /// `git --version` failed, so every git request fails as `GitMissing`.
    Missing {
        /// Why the probe failed; diagnostic only.
        detail: String,
    },
}

/// One thing the in-distro walk reports while a `Walk` is in flight.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// A repository the walk found.
    Repo(WorkerRepoFound),
    /// A `ScanProblem` the walk raised.
    Problem {
        /// The `ScanProblemKind` slug.
        kind: String,
        /// The path the problem concerns, for display.
        path_display: String,
        /// Diagnostic detail.
        detail: String,
    },
    /// The walk's running counts.
    Progress {
        /// Directories visited so far.
        walked_dirs: u64,
        /// Repositories found so far.
        found_repos: u64,
    },
    /// §4.5 in the other direction: a Windows volume surfaced inside the distro is the native
    /// walk's territory, and this names the mount that was left alone rather than dropping it.
    SkippedMount {
        /// Where the Windows-backed filesystem is mounted inside the distro.
        mount_point: String,
        /// Its filesystem type as the kernel reports it.
        fstype: String,
    },
}

/// A repository the in-distro walk found, with the store facts the worker resolved for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRepoFound {
    /// The repository's Linux path: the directory git runs in.
    pub work_dir: String,
    /// This checkout's git dir.
    pub git_dir: String,
    /// The shared git dir.
    pub common_dir: String,
    /// The `RepoKind` slug, read back by `RepoKind::from_str`.
    pub kind: String,
    /// §4.7's runtime identity, `wsl:<distro>:<mount point>`.
    pub store_key: String,
    /// §4.7's persistent identity, or `None` where the mount has none (R27).
    pub volume_key: Option<String>,
    /// The `StoreClass` slug, from `class_slug`.
    pub store_class: String,
}

/// The reply to a `Walk`: its totals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkSummary {
    /// Directories the walk visited.
    pub walked_dirs: u64,
    /// Repositories it found.
    pub found_repos: u64,
    /// Whether the walk was cancelled before it finished.
    pub cancelled: bool,
}

/// The reply to `Mounts`: `MountFacts` with the class as a slug.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMountFacts {
    /// §4.7's runtime identity, `wsl:<distro>:<mount point>`.
    pub store_key: String,
    /// §4.7's persistent identity, or `None` where the mount has none.
    pub volume_key: Option<String>,
    /// The `StoreClass` slug, from `class_slug`.
    pub class: String,
}

/// A `submodule_gitlinks` map in its wire shape: the entries as pairs.
///
/// `submodule_gitlinks` returns a map keyed by raw path bytes, and JSON has no such key. The
/// pairs form is the wire shape; the two converters below are the only place it is built, so a
/// path that is not UTF-8 crosses intact instead of being lossily stringified.
#[must_use]
pub fn gitlinks_to_wire(map: &BTreeMap<Vec<u8>, String>) -> Vec<(Vec<u8>, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// `gitlinks_to_wire`'s inverse: the pairs folded back into the byte-keyed map.
#[must_use]
pub fn gitlinks_from_wire(pairs: Vec<(Vec<u8>, String)>) -> BTreeMap<Vec<u8>, String> {
    pairs.into_iter().collect()
}

/// The wire mirror of plan 05's `GitError`.
///
/// It exists so the core rebuilds the *same* error, not an approximation of it:
/// `GitError::Missing` has to arrive as `GitError::Missing` for §11.5's `GIT_MISSING` prose to
/// be reachable at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum WorkerFault {
    /// `GitError::Missing`: no git binary in the distro.
    GitMissing,
    /// `GitError::TooOld`: git is below the floor.
    GitTooOld {
        /// The `git --version` line as printed.
        found: String,
    },
    /// `GitError::Untrusted`: git refused the repository for dubious ownership.
    Untrusted {
        /// Lossy display form of the path git refused, diagnostic only.
        path: String,
    },
    /// `GitError::PermissionDenied`: the filesystem refused the read.
    PermissionDenied {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// `GitError::PathGone`: the path is genuinely not there, the only fault implying absence.
    PathGone {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// `GitError::StoreOffline`: the backing store answered as unmounted or unreachable.
    StoreOffline {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// `GitError::Unreadable`: it looks like a repository and git could not open it.
    Unreadable {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// `GitError::Stale`: the read failed in a way that means unknown, never absence.
    Stale {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
    /// `GitError::Busy`: a lock or operation marker is in the way.
    Busy {
        /// The `BusyMarker` as a slug; one this build does not know reads back as `IndexLock`.
        marker: String,
    },
    /// `GitError::TornRead`: the ref state moved during one observation.
    TornRead,
    /// `GitError::Budget`: the job's deadline elapsed and the process tree was killed.
    Budget {
        /// How long the child ran before it was killed, in milliseconds.
        after_ms: u64,
    },
    /// `GitError::Cancelled`: a cancellation token fired.
    Cancelled,
    /// `GitError::Internal`: a defect in this program.
    Internal {
        /// Diagnostic detail; never rendered raw.
        detail: String,
    },
}

/// The wire form of `err`, variant for variant.
#[must_use]
pub fn fault_of(err: &GitError) -> WorkerFault {
    match err {
        GitError::Missing => WorkerFault::GitMissing,
        GitError::TooOld { found } => WorkerFault::GitTooOld {
            found: found.clone(),
        },
        GitError::Untrusted { path } => WorkerFault::Untrusted { path: path.clone() },
        GitError::PermissionDenied { detail } => WorkerFault::PermissionDenied {
            detail: detail.clone(),
        },
        GitError::PathGone { detail } => WorkerFault::PathGone {
            detail: detail.clone(),
        },
        GitError::StoreOffline { detail } => WorkerFault::StoreOffline {
            detail: detail.clone(),
        },
        GitError::Unreadable { detail } => WorkerFault::Unreadable {
            detail: detail.clone(),
        },
        GitError::Stale { detail } => WorkerFault::Stale {
            detail: detail.clone(),
        },
        GitError::Busy { marker } => WorkerFault::Busy {
            marker: busy_slug(*marker).to_owned(),
        },
        GitError::TornRead => WorkerFault::TornRead,
        GitError::Budget { after_ms } => WorkerFault::Budget {
            after_ms: *after_ms,
        },
        GitError::Cancelled => WorkerFault::Cancelled,
        GitError::Internal { detail } => WorkerFault::Internal {
            detail: detail.clone(),
        },
    }
}

/// Rebuilds the `GitError` a fault was made from: `fault_of`'s inverse.
/// Rebuilds the `GitError` a fault was made from: `fault_of`'s inverse.
#[must_use]
pub fn git_error_of(fault: WorkerFault) -> GitError {
    match fault {
        WorkerFault::GitMissing => GitError::Missing,
        WorkerFault::GitTooOld { found } => GitError::TooOld { found },
        WorkerFault::Untrusted { path } => GitError::Untrusted { path },
        WorkerFault::PermissionDenied { detail } => GitError::PermissionDenied { detail },
        WorkerFault::PathGone { detail } => GitError::PathGone { detail },
        WorkerFault::StoreOffline { detail } => GitError::StoreOffline { detail },
        WorkerFault::Unreadable { detail } => GitError::Unreadable { detail },
        WorkerFault::Stale { detail } => GitError::Stale { detail },
        WorkerFault::Busy { marker } => GitError::Busy {
            marker: busy_of(&marker),
        },
        WorkerFault::TornRead => GitError::TornRead,
        WorkerFault::Budget { after_ms } => GitError::Budget { after_ms },
        WorkerFault::Cancelled => GitError::Cancelled,
        WorkerFault::Internal { detail } => GitError::Internal { detail },
    }
}

const fn busy_slug(marker: BusyMarker) -> &'static str {
    match marker {
        BusyMarker::IndexLock => "index_lock",
        BusyMarker::Merge => "merge",
        BusyMarker::Rebase => "rebase",
        BusyMarker::CherryPick => "cherry_pick",
        BusyMarker::Bisect => "bisect",
        BusyMarker::Revert => "revert",
    }
}

fn busy_of(slug: &str) -> BusyMarker {
    match slug {
        "merge" => BusyMarker::Merge,
        "rebase" => BusyMarker::Rebase,
        "cherry_pick" => BusyMarker::CherryPick,
        "bisect" => BusyMarker::Bisect,
        "revert" => BusyMarker::Revert,
        // An unknown marker is still a lock: refusing to defer would be worse than deferring on
        // the most common cause.
        _ => BusyMarker::IndexLock,
    }
}

/// A `FrameError` as the `std::io::Error` the worker's IO paths return: an IO failure as it
/// was, anything else wrapped with its message.
#[must_use]
pub fn frame_io_error(err: FrameError) -> std::io::Error {
    match err {
        FrameError::Io(io) => io,
        other => std::io::Error::other(format!("{other}")),
    }
}

fn write_json<W: std::io::Write, T: Serialize>(out: &mut W, value: &T) -> std::io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    write_frame(out, &payload).map_err(frame_io_error)?;
    out.flush()
}

/// Writes one worker-to-core frame and flushes it.
///
/// # Errors
/// Fails when `frame` cannot be serialised, when its payload exceeds `MAX_FRAME_BYTES`, or when
/// writing to or flushing `out` fails.
pub fn write_worker_frame<W: std::io::Write>(
    out: &mut W,
    frame: &WorkerOutbound,
) -> std::io::Result<()> {
    write_json(out, frame)
}

/// Writes one core-to-worker call and flushes it.
///
/// # Errors
/// Fails when `call` cannot be serialised, when its payload exceeds `MAX_FRAME_BYTES`, or when
/// writing to or flushing `out` fails.
pub fn write_worker_call<W: std::io::Write>(out: &mut W, call: &WorkerCall) -> std::io::Result<()> {
    write_json(out, call)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{
        fault_of, git_error_of, gitlinks_from_wire, gitlinks_to_wire, write_worker_call,
        write_worker_frame, WalkRequest, WorkerCall, WorkerFault, WorkerGit, WorkerGitOp,
        WorkerJobClass, WorkerOutbound, WorkerRepo, WorkerRequest,
    };
    use crate::git::{BusyMarker, GitError, JobClass};
    use crate::proto::frame::read_frame;
    use crate::proto::wire::RequestId;
    use crate::scan::discover::RepoKind;
    use std::collections::BTreeMap;

    fn call(request: WorkerRequest) -> WorkerCall {
        WorkerCall {
            id: RequestId(7),
            request,
        }
    }

    #[test]
    fn a_call_is_one_frame_and_reads_back_identical() {
        let original = call(WorkerRequest::Walk(WalkRequest {
            root: "/home/me/code".to_owned(),
            follow_links: false,
            descend_into_repos: false,
            bare_candidates: true,
            skip_extra: vec!["scratch".to_owned()],
        }));
        let mut wire = Vec::new();
        write_worker_call(&mut wire, &original).expect("frames");

        let mut buf = Vec::new();
        read_frame(&mut wire.as_slice(), &mut buf).expect("one frame");
        let back: WorkerCall = serde_json::from_slice(&buf).expect("parses");
        assert_eq!(
            serde_json::to_string(&back).expect("re-serialises"),
            serde_json::to_string(&original).expect("serialises")
        );
    }

    #[test]
    fn the_request_tag_is_the_command_name() {
        let json = serde_json::to_value(call(WorkerRequest::Ping)).expect("serialises");
        assert_eq!(json["id"], 7);
        assert_eq!(json["request"]["t"], "ping");
    }

    #[test]
    fn a_git_request_carries_its_job_class_and_its_deadline() {
        let request = WorkerRequest::Git {
            repo: WorkerRepo {
                work_dir: "/home/me/widget".to_owned(),
                git_dir: "/home/me/widget/.git".to_owned(),
                common_dir: "/home/me/widget/.git".to_owned(),
                trusted: true,
            },
            op: WorkerGitOp::WorktreeStatus { untracked: false },
            job: WorkerJobClass::Interactive,
            // §4.1's 500 ms interactive budget has to survive the hop, or one 25 s repository
            // blocks the distro the way it blocked the corpus.
            deadline_ms: Some(500),
        };
        let json = serde_json::to_value(&request).expect("serialises");
        assert_eq!(json["op"]["op"], "worktree_status");
        assert_eq!(json["job"], "interactive");
        assert_eq!(json["deadline_ms"], 500);
        assert!(matches!(
            WorkerJobClass::History.to_job_class(),
            JobClass::History
        ));
        assert!(matches!(
            WorkerJobClass::from_job_class(JobClass::Background),
            WorkerJobClass::Background
        ));
    }

    #[test]
    fn an_outbound_frame_round_trips() {
        let hello = WorkerOutbound::Hello {
            protocol_version: super::WORKER_PROTOCOL_VERSION,
            worker_version: "0.0.0".to_owned(),
            git: WorkerGit::Missing {
                detail: "no git on PATH".to_owned(),
            },
            pid: 42,
        };
        let mut wire = Vec::new();
        write_worker_frame(&mut wire, &hello).expect("frames");
        let mut buf = Vec::new();
        read_frame(&mut wire.as_slice(), &mut buf).expect("one frame");
        let back: WorkerOutbound = serde_json::from_slice(&buf).expect("parses");
        match back {
            WorkerOutbound::Hello {
                git: WorkerGit::Missing { detail },
                pid,
                ..
            } => {
                assert_eq!(detail, "no git on PATH");
                assert_eq!(pid, 42);
            }
            other => panic!("wrong frame: {other:?}"),
        }
    }

    #[test]
    fn every_git_error_survives_the_hop_unchanged() {
        let cases = [
            GitError::Missing,
            GitError::TooOld {
                found: "2.20.1".to_owned(),
            },
            GitError::Untrusted {
                path: "/home/me/widget".to_owned(),
            },
            GitError::PermissionDenied {
                detail: "EACCES".to_owned(),
            },
            GitError::PathGone {
                detail: "ENOENT".to_owned(),
            },
            GitError::StoreOffline {
                detail: "unmounted".to_owned(),
            },
            GitError::Unreadable {
                detail: "bad object".to_owned(),
            },
            GitError::Stale {
                detail: "refs moved".to_owned(),
            },
            GitError::Busy {
                marker: BusyMarker::IndexLock,
            },
            GitError::TornRead,
            GitError::Budget { after_ms: 500 },
            GitError::Cancelled,
            GitError::Internal {
                detail: "boom".to_owned(),
            },
        ];
        for original in cases {
            let fault = fault_of(&original);
            let json = serde_json::to_string(&fault).expect("serialises");
            let back: WorkerFault = serde_json::from_str(&json).expect("parses");
            let rebuilt = git_error_of(back);
            assert_eq!(
                rebuilt.protocol_code(),
                original.protocol_code(),
                "protocol code changed for {original:?}"
            );
            assert_eq!(rebuilt.implies_absent(), original.implies_absent());
            assert_eq!(rebuilt.is_deferral(), original.is_deferral());
        }
    }

    #[test]
    fn every_busy_marker_survives_the_hop_unchanged() {
        // The marker is a slug on the wire, so a new variant that nobody adds a slug for would
        // silently arrive as `IndexLock` and defer on the wrong cause.
        for marker in [
            BusyMarker::IndexLock,
            BusyMarker::Merge,
            BusyMarker::Rebase,
            BusyMarker::CherryPick,
            BusyMarker::Bisect,
            BusyMarker::Revert,
        ] {
            let rebuilt = git_error_of(fault_of(&GitError::Busy { marker }));
            assert_eq!(rebuilt, GitError::Busy { marker });
        }
    }

    #[test]
    fn a_distro_without_git_yields_the_code_the_shell_renders() {
        // §13: the worker reports GIT_MISSING for that location, and those repositories render
        // with an explained error rather than silently missing.
        assert_eq!(
            git_error_of(WorkerFault::GitMissing).protocol_code(),
            Some("GIT_MISSING")
        );
    }

    #[test]
    fn the_repo_kind_slug_round_trips() {
        // R7 put the inverse on `RepoKind` itself, so this plan declares no second one.
        for kind in [
            RepoKind::WorkTree,
            RepoKind::LinkedWorktree,
            RepoKind::SeparateGitDir,
            RepoKind::Bare,
        ] {
            assert_eq!(RepoKind::from_str(kind.as_str()), Some(kind));
        }
        assert_eq!(RepoKind::from_str("nonsense"), None);
    }

    #[test]
    fn a_gitlink_path_that_is_not_utf8_survives_the_hop() {
        // JSON has no byte-keyed map, so this is the one result whose wire shape differs from
        // its in-process shape. A lossy stringification here would rename a submodule.
        let mut map = BTreeMap::new();
        map.insert(vec![0x76, 0xFF, 0x2F, 0x61], "0123abc".to_owned());
        map.insert(b"lib/thing".to_vec(), "89defab".to_owned());
        let wire = gitlinks_to_wire(&map);
        let text = serde_json::to_string(&wire).expect("serialises");
        let back: Vec<(Vec<u8>, String)> = serde_json::from_str(&text).expect("parses");
        assert_eq!(gitlinks_from_wire(back), map);
    }
}

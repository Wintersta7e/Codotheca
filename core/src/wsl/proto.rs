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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerCall {
    pub id: RequestId,
    pub request: WorkerRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerRequest {
    Ping,
    Mounts {
        path: String,
    },
    Walk(WalkRequest),
    Git {
        repo: WorkerRepo,
        op: WorkerGitOp,
        job: WorkerJobClass,
        /// §4.1's per-job budget, carried across so a slow in-distro repository is bounded on
        /// the same terms as a slow local one.
        deadline_ms: Option<u64>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkRequest {
    pub root: String,
    pub follow_links: bool,
    pub descend_into_repos: bool,
    pub bare_candidates: bool,
    pub skip_extra: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRepo {
    pub work_dir: String,
    pub git_dir: String,
    pub common_dir: String,
    pub trusted: bool,
}

/// One `GitBackend` method, named. Every method has a variant: a backend that answered nine of
/// ten would compile only because the tenth was written to fail, which is the shape of a seam
/// that passes its tests and breaks at assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerGitOp {
    Version,
    RepoFacts,
    RefState,
    Divergence {
        state: Box<RefState>,
    },
    WorktreeStatus {
        untracked: bool,
    },
    TrackedInventory,
    /// §4.4. The paths are raw bytes because a Linux path is not a `String`.
    SubmoduleGitlinks {
        paths: Vec<Vec<u8>>,
    },
    /// §1.1's remote evidence. A unit variant: the argv is the core's own builder, run inside
    /// the distro by the worker's `SystemGit`.
    RemoteUrls,
    RootCommits,
    /// [p2-24b] §24.7A's reachability walk, run inside the distro.
    UnpushedRefs,
    Authorship,
    CommitSubjects {
        limit: u32,
    },
    /// §29.1's HEAD enumeration, run inside the distro.
    HeadTree,
    /// §29.6's blob read. `byte_cap` crosses because it is the **caller's** cap: a blob over it
    /// is recorded at its size with its body discarded, and doing that inside the distro is what
    /// keeps the body off this wire.
    ReadBlobs {
        oids: Vec<String>,
        byte_cap: u64,
        budget_bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerJobClass {
    Interactive,
    Background,
    History,
}

impl WorkerJobClass {
    #[must_use]
    pub fn to_job_class(self) -> JobClass {
        match self {
            Self::Interactive => JobClass::Interactive,
            Self::Background => JobClass::Background,
            Self::History => JobClass::History,
        }
    }

    #[must_use]
    pub fn from_job_class(job: JobClass) -> Self {
        match job {
            JobClass::Interactive => Self::Interactive,
            JobClass::Background => Self::Background,
            JobClass::History => Self::History,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerOutbound {
    Hello {
        protocol_version: u32,
        worker_version: String,
        git: WorkerGit,
        pid: u32,
    },
    Event {
        id: RequestId,
        event: WorkerEvent,
    },
    Reply {
        id: RequestId,
        ok: serde_json::Value,
    },
    Fail {
        id: RequestId,
        fault: WorkerFault,
    },
}

/// Whether the distro has a usable git, decided once at startup and reported in `Hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "git", rename_all = "snake_case")]
pub enum WorkerGit {
    Present { version: String },
    Missing { detail: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum WorkerEvent {
    Repo(WorkerRepoFound),
    Problem {
        kind: String,
        path_display: String,
        detail: String,
    },
    Progress {
        walked_dirs: u64,
        found_repos: u64,
    },
    /// §4.5 in the other direction: a Windows volume surfaced inside the distro is the native
    /// walk's territory, and this names the mount that was left alone rather than dropping it.
    SkippedMount {
        mount_point: String,
        fstype: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRepoFound {
    pub work_dir: String,
    pub git_dir: String,
    pub common_dir: String,
    pub kind: String,
    pub store_key: String,
    pub volume_key: Option<String>,
    pub store_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkSummary {
    pub walked_dirs: u64,
    pub found_repos: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMountFacts {
    pub store_key: String,
    pub volume_key: Option<String>,
    pub class: String,
}

/// `submodule_gitlinks` returns a map keyed by raw path bytes, and JSON has no such key. The
/// pairs form is the wire shape; the two converters below are the only place it is built, so a
/// path that is not UTF-8 crosses intact instead of being lossily stringified.
#[must_use]
pub fn gitlinks_to_wire(map: &BTreeMap<Vec<u8>, String>) -> Vec<(Vec<u8>, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[must_use]
pub fn gitlinks_from_wire(pairs: Vec<(Vec<u8>, String)>) -> BTreeMap<Vec<u8>, String> {
    pairs.into_iter().collect()
}

/// The wire mirror of plan 05's `GitError`. It exists so the core rebuilds the *same* error, not
/// an approximation of it: `GitError::Missing` has to arrive as `GitError::Missing` for §11.5's
/// `GIT_MISSING` prose to be reachable at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum WorkerFault {
    GitMissing,
    GitTooOld { found: String },
    Untrusted { path: String },
    PermissionDenied { detail: String },
    PathGone { detail: String },
    StoreOffline { detail: String },
    Unreadable { detail: String },
    Stale { detail: String },
    Busy { marker: String },
    TornRead,
    Budget { after_ms: u64 },
    Cancelled,
    Internal { detail: String },
}

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

pub fn write_worker_frame<W: std::io::Write>(
    out: &mut W,
    frame: &WorkerOutbound,
) -> std::io::Result<()> {
    write_json(out, frame)
}

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

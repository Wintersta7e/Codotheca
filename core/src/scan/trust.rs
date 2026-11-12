//! §11.1's `TRUST THIS REPOSITORY` for a folder git refused during a scan — `problems.trust`.
//!
//! An untrusted repository fails the hand-off, so no `location` row exists for it and
//! `locations.setTrusted` has nothing to name. This command is keyed by the `scan_problem` row
//! instead: the core reads the folder's raw bytes and its root from its own row (no path crosses
//! the wire, and `path_display` is never read back — §1.10), re-discovers that one folder with a
//! trusted probe, and runs the scan's own hand-off with `RepoHandle::trusted` set. That
//! hand-off's transaction writes `location.trusted_at` — still the one trust store — and deletes
//! the row. The user's git config is never written (§17): trust reaches git only as
//! `-c safe.directory=<exact-path>` on argv.
//!
//! **Off the index lock** (`Route::ProblemTrust`), like a scan: the guard is taken once to read
//! the row and released before any git runs, and the hand-off takes it again for its one
//! transaction. `std::sync::Mutex` is not reentrant, so a caller holding it would deadlock.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::OptionalExtension as _;

use crate::assembly::handoff::{hand_off_trusted, HandoffCtx, HandoffError};
use crate::cancel::CancelToken;
use crate::exclusion::gate::ExclusionGate;
use crate::git::{GitBackend, StoreKey};
use crate::index::Index;
use crate::jobs::JobSink;
use crate::mount::MountResolver;
use crate::paths::{path_display, path_from_bytes, path_key};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{ErrorCode, LocationId, ProblemsTrustArgs, ScanProblemId};
use crate::scan::discover::{classify_dir, ProbeCtx};
use crate::scan::run::{platform_of, Discovered};
use crate::scan::{ScanProblemKind, WalkEvent, WalkOptions};

/// Everything `problems.trust` needs. The index is the shared mutex, never a held guard.
pub struct TrustCtx<'a> {
    /// The one index, locked around the row read and the hand-off's transaction only.
    pub index: &'a Mutex<Index>,
    /// The read seam the re-discovery and the identity probe run through.
    pub git: &'a dyn GitBackend,
    /// Resolves the folder's store key, volume key and store class, as the walk does.
    pub mounts: &'a dyn MountResolver,
    /// Where the new location's ordinary pipeline is queued, as after a scan's hand-off.
    pub jobs: &'a dyn JobSink,
    /// The core's exclusion gate, which the hand-off asks before it reads and before it writes.
    pub exclusions: &'a ExclusionGate,
    /// The caller's clock reading, in unix seconds; `trusted_at` is stamped with it.
    pub now: i64,
}

impl std::fmt::Debug for TrustCtx<'_> {
    /// By hand: the seams are trait objects whose contents are not a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// `problems.trust`: answers the `LocationId` the trusted folder was indexed as.
///
/// # Errors
/// `PROTOCOL` for arguments that do not parse or a row of another kind; `PATH_GONE` for a row
/// that is not there (already answered), keeps no folder, or whose root was removed, and for a
/// folder no longer there; `STORE_OFFLINE` when its store is not mounted; `REPO_UNREADABLE` when
/// the folder is no longer a repository; git's own code when the trusted hand-off fails.
pub fn handle_trust(
    ctx: &TrustCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: ProblemsTrustArgs = parse_args(args)?;
    let location = trust_problem(ctx, args.problem_id)?;
    serde_json::to_value(location).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// What the row says about the folder, read under the guard and nothing else.
struct Target {
    path: PathBuf,
    path_bytes: Vec<u8>,
    root_id: i64,
    root_kind: String,
    distro: String,
    generation: i64,
}

fn trust_problem(ctx: &TrustCtx<'_>, problem: ScanProblemId) -> Result<LocationId, CommandFailure> {
    let target = read_target(ctx.index, problem)?;
    if !target.path.exists() {
        return Err(refusal(
            ErrorCode::PathGone,
            "the folder is no longer there",
        ));
    }
    let facts = ctx
        .mounts
        .resolve(&target.path)
        .map_err(|_| refusal(ErrorCode::StoreOffline, "the folder's store is not mounted"))?;

    // Discovery for this one folder, as the walk ran it but trusted: no lock is held here.
    let cancel = CancelToken::new();
    let probe = ProbeCtx::new(
        ctx.git,
        StoreKey::new(facts.store_key.clone()),
        facts.class,
        &cancel,
    )
    .trusted();
    let opts = WalkOptions {
        // A row only exists for a folder under an enabled root, where the walk tests bare
        // candidates too (§4.2).
        bare_candidates: true,
        ..WalkOptions::default()
    };
    let notes = Mutex::new(Vec::new());
    let candidate = classify_dir(&target.path, &opts, &probe, &|event| {
        if let WalkEvent::Problem(note) = event {
            notes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(note.detail);
        }
    })
    .ok_or_else(|| {
        let why = notes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .join("; ");
        refusal(
            ErrorCode::RepoUnreadable,
            format!("the folder is no longer a repository git can read: {why}"),
        )
    })?;

    let platform = platform_of(&target.root_kind);
    let discovered = Discovered {
        root_id: target.root_id,
        kind: target.root_kind,
        distro: target.distro,
        path_key: path_key(&target.path, platform),
        path_display: path_display(&target.path),
        path_bytes: target.path_bytes,
        store_key: facts.store_key,
        volume_key: facts.volume_key,
        candidate,
    };
    let handoff = HandoffCtx {
        git: ctx.git,
        cancel: &cancel,
        exclusions: ctx.exclusions,
        store_class: facts.class,
        // The run that walked this folder and recorded the row: the location claims the
        // generation of the walk that saw it, never one it invents (§4.6).
        generation: target.generation,
        now: ctx.now,
    };
    let indexed = hand_off_trusted(ctx.index, &handoff, &discovered, problem.0)
        .map_err(|err| handoff_refusal(&err))?;
    ctx.jobs.on_location_indexed(
        indexed.project,
        indexed.location,
        &discovered.store_key,
        facts.class,
    );
    Ok(indexed.location)
}

/// The row and its root, with each of the command's refusals.
fn read_target(index: &Mutex<Index>, problem: ScanProblemId) -> Result<Target, CommandFailure> {
    type Row = (
        String,
        Option<Vec<u8>>,
        Option<i64>,
        Option<String>,
        Option<String>,
        i64,
    );
    let guard = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let row: Option<Row> = guard
        .conn()
        .query_row(
            "SELECT p.kind, p.path_bytes, p.root_id, r.kind, r.distro, s.generation
               FROM scan_problem p
               JOIN scan_run s ON s.id = p.scan_run_id
               LEFT JOIN scan_root r ON r.id = p.root_id
              WHERE p.id = ?1",
            [problem.0],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    drop(guard);

    let Some((kind, path_bytes, root_id, root_kind, distro, generation)) = row else {
        return Err(refusal(
            ErrorCode::PathGone,
            format!(
                "no scan problem {}; it may already have been answered",
                problem.0
            ),
        ));
    };
    if kind != ScanProblemKind::UntrustedRepo.as_str() {
        return Err(CommandFailure::protocol(format!(
            "scan problem {} is {kind}, and only an untrusted repository can be trusted",
            problem.0
        )));
    }
    let Some(path_bytes) = path_bytes else {
        return Err(refusal(
            ErrorCode::PathGone,
            "this problem keeps no folder to trust",
        ));
    };
    let (Some(root_id), Some(root_kind), Some(distro)) = (root_id, root_kind, distro) else {
        return Err(refusal(
            ErrorCode::PathGone,
            "the root this folder was found under has been removed",
        ));
    };
    Ok(Target {
        path: path_from_bytes(&path_bytes),
        path_bytes,
        root_id,
        root_kind,
        distro,
        generation,
    })
}

/// A refusal that took no effect.
fn refusal(code: ErrorCode, message: impl Into<String>) -> CommandFailure {
    CommandFailure {
        code,
        message: message.into(),
        // `None` = definitely did not take effect: every refusal here precedes or rolls back
        // the one transaction.
        outcome: None,
    }
}

/// The hand-off's failure, under git's own code where it has one. The transaction rolled back,
/// so none of these took effect, and the problem row is still there to answer.
fn handoff_refusal(err: &HandoffError) -> CommandFailure {
    let code = match err {
        HandoffError::ProblemGone => ErrorCode::PathGone,
        HandoffError::Git(git) => git
            .protocol_code()
            .and_then(|code| serde_json::from_value(serde_json::Value::from(code)).ok())
            .unwrap_or(ErrorCode::Internal),
        // An exclusion's own code arrives with the command rows that answer it before any read.
        HandoffError::Identity(_)
        | HandoffError::UnknownKind(_)
        | HandoffError::Poisoned
        | HandoffError::Excluded => ErrorCode::Internal,
    };
    refusal(code, err.to_string())
}

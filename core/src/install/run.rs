//! §24.3e's commit order, in this order and no other.
//!
//! 1. `git clone` exits 0.
//! 2. Atomic rename into `<root>/<seed_basename>`.
//! 3. The `location` row is written **and** the `install_run` row moves to `done`.
//! 4. The ordinary J0–J6 pipeline is enqueued exactly as a scan enqueues it.
//!
//! **The `location` row is written after the rename, never optimistically.** A project is *not
//! cloned* until that row commits — mid-install is not a third era, and a crash between the clone
//! exiting and the rename completing leaves nothing any surface would read as a working copy.
//!
//! **Install owns no bespoke indexer.** Steps 3 and 4 are `hand_off_discovered` followed by
//! `JobSink::on_location_indexed` — the same pair `core/src/scan/run.rs:588-593` performs when
//! the walk indexes a location. Writing a `location` row here by hand would be a second
//! definition of what indexing means, and it would skip `resolve_identity`: the clone would be
//! bolted to the project that asked for it even when §1.5 says it belongs to another.
//!
//! **A run may not assume its location stays attached to the project that requested it.** J1 can
//! compute a `lineage_key` that collides with an existing project after the clone; §1.5's merge
//! and p2-22 own that reconciliation. This module asserts the run survives it, not that it
//! cannot happen — which is exactly why the identity decision is delegated rather than assumed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::assembly::handoff::{hand_off_discovered, HandoffCtx};
use crate::cancel::CancelToken;
use crate::git::GitBackend;
use crate::gitw::backend::MutatingGit;
use crate::gitw::intent::{Intent, RemoteUrl};
use crate::index::Index;
use crate::install::queue::InstallRequest;
use crate::install::stage::{parse_progress_line, StageMachine};
use crate::install::staging::staging_path_for;
use crate::install::state::InstallStateStore;
use crate::jobs::JobSink;
use crate::mount::MountResolver;
use crate::paths::{path_bytes, path_display, path_key};
use crate::proto::pubsub::EventSink;
use crate::protocol::{InstallFailure, InstallRunId, LocationId};
use crate::scan::discover::{RepoCandidate, RepoKind};
use crate::scan::run::{platform_of, Discovered};

/// Everything a run needs that is not the request.
///
/// **Deviation from the plan's table**, which writes
/// `run_install(&dyn MutatingGit, &Arc<Mutex<Index>>, &InstallRequest, &dyn EventSink)`. That
/// list cannot reach step 3 or step 4: indexing the clone needs the **read** git seam for
/// `probe_identity`, the mount resolver for `store_key`, and the job sink for the pipeline. An
/// install that could not reach them would have to own an indexer, which the same task forbids.
/// A context struct rather than eight positional parameters, because `type_complexity` fires on
/// far less than that.
pub struct InstallCtx<'a> {
    /// The write seam — §24.1's two subcommands and nothing else.
    pub git: &'a dyn MutatingGit,
    /// The read seam, for the identity probe after the rename.
    pub probe: &'a dyn GitBackend,
    pub index: &'a Arc<Mutex<Index>>,
    pub jobs: &'a dyn JobSink,
    pub mounts: &'a dyn MountResolver,
    /// §24.4's stage state, so a tile that mounts mid-clone reads a snapshot rather than waiting
    /// for the next event (R54).
    pub stages: &'a InstallStateStore,
    pub events: &'a dyn EventSink,
    pub cancel: &'a CancelToken,
    pub now: i64,
}

impl std::fmt::Debug for InstallCtx<'_> {
    /// By hand: a struct holding `&dyn Trait` raises `missing_debug_implementations`, and the
    /// contents of these seams are not a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Where a run's bytes go, re-derived from the request rather than carried in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPaths {
    pub staging: PathBuf,
    pub destination: PathBuf,
}

/// The root row's own facts, which a run needs and the queue does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootFacts {
    pub root_id: i64,
    pub path: PathBuf,
    /// `win` | `linux` | `wsl`.
    pub kind: String,
    /// `""` off WSL (§1.3).
    pub distro: String,
}

/// Re-derive the staging and destination paths for a request.
///
/// Separate from [`run_install`] so the caller writes both into the durable `install_run` row
/// **before** the clone starts — which is what the staging warrant rests on (§24.3c).
///
/// # Errors
/// Fails when the staging root is unusable, which must be known before anything is spawned.
pub fn paths_for(root_path: &Path, seed_basename: &str) -> Result<RunPaths, InstallFailure> {
    let staging =
        staging_path_for(root_path, seed_basename).map_err(|_| InstallFailure::RenameFailed)?;
    Ok(RunPaths {
        staging,
        destination: root_path.join(seed_basename),
    })
}

/// Run one install to its `location` row.
///
/// # Errors
/// Fails at the first step that cannot complete, leaving nothing outside the staging directory.
pub fn run_install(
    ctx: &InstallCtx<'_>,
    run: InstallRunId,
    request: &InstallRequest,
    root: &RootFacts,
    paths: &RunPaths,
    clone_url: &str,
) -> Result<LocationId, InstallFailure> {
    let url = RemoteUrl::parse(clone_url).map_err(|_| InstallFailure::GitFailed)?;

    // 1. The clone. `git clone` refuses a non-empty directory, which is why §24.3b stages rather
    //    than writing a marker file into the destination.
    if let Some(parent) = paths.staging.parent() {
        std::fs::create_dir_all(parent).map_err(|_| InstallFailure::DiskFull)?;
    }
    let intent = Intent::Clone {
        url,
        dest: paths.staging.clone(),
        depth: None,
    };
    // The child's stderr is read here and never forwarded to this process's stdout, which
    // carries protocol frames and nothing else.
    let mut machine = StageMachine::new();
    ctx.git
        .run(&intent, ctx.cancel, &mut |line| {
            let Some(observed) = parse_progress_line(line) else {
                return;
            };
            for entered in machine.advance(observed) {
                publish(ctx, run, entered);
            }
        })
        .map_err(|_| {
            if ctx.cancel.is_cancelled() {
                InstallFailure::Cancelled
            } else {
                InstallFailure::GitFailed
            }
        })?;

    // 2. The rename. Same filesystem by construction — staging is a sibling under the same root —
    //    so it is atomic on NTFS and ext4 and there is no window in which the destination is half
    //    written. Nothing above this line has touched the destination.
    std::fs::rename(&paths.staging, &paths.destination)
        .map_err(|_| InstallFailure::RenameFailed)?;

    // 3. Index it through the ordinary hand-off, which decides identity and writes the row.
    let location = index_destination(ctx, request, root, paths)?;

    // The run has settled: the location row is committed, which is the milestone §24.4's last
    // stage names. Entered after the row and not before it, so the readout cannot say `settled`
    // about a project that is not yet cloned.
    for entered in machine.settle() {
        publish(ctx, run, entered);
    }

    // 4. The ordinary pipeline, through the same sink the scanner uses.
    let facts = ctx.mounts.resolve(&paths.destination);
    let store_key = facts
        .as_ref()
        .map(|f| f.store_key.clone())
        .unwrap_or_default();
    let store_class = facts
        .as_ref()
        .map_or_else(|_| crate::mount::StoreClass::Local, |f| f.class);
    ctx.jobs
        .on_location_indexed(request.project, location, &store_key, store_class);

    // The run is complete only once its location exists. Marked here rather than before the
    // hand-off so a crash cannot leave a run reading `done` with no row to show for it.
    mark_done(ctx, run)?;
    Ok(location)
}

/// Step 3: the hand-off the scanner uses, on a directory this run just created.
fn index_destination(
    ctx: &InstallCtx<'_>,
    request: &InstallRequest,
    root: &RootFacts,
    paths: &RunPaths,
) -> Result<LocationId, InstallFailure> {
    let facts = ctx
        .mounts
        .resolve(&paths.destination)
        .map_err(|_| InstallFailure::RenameFailed)?;
    let git_dir = paths.destination.join(".git");
    let discovered = Discovered {
        candidate: RepoCandidate {
            path: paths.destination.clone(),
            kind: RepoKind::WorkTree,
            git_dir: git_dir.clone(),
            common_dir: git_dir,
        },
        root_id: root.root_id,
        kind: root.kind.clone(),
        distro: root.distro.clone(),
        path_bytes: path_bytes(&paths.destination),
        path_key: path_key(&paths.destination, platform_of(&root.kind)),
        path_display: path_display(&paths.destination),
        store_key: facts.store_key.clone(),
        volume_key: facts.volume_key.clone(),
    };
    let handoff = HandoffCtx {
        git: ctx.probe,
        cancel: ctx.cancel,
        store_class: facts.class,
        // A clone is not part of any scan run, so it claims none of their generations. §4.6's
        // sweep compares against the generation of the run that *walked*; a fresh clone has been
        // seen by no walk, and 0 is the value a row starts at rather than a run id it invents.
        generation: 0,
        now: ctx.now,
    };
    let indexed = hand_off_discovered(ctx.index, &handoff, &discovered)
        .map_err(|_| InstallFailure::GitFailed)?;
    let _ = request;
    Ok(indexed.location)
}

/// Move the run to `done`.
fn mark_done(ctx: &InstallCtx<'_>, run: InstallRunId) -> Result<(), InstallFailure> {
    let _guard = crate::proto::txguard::TxGuard::enter();
    let guard = ctx
        .index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .conn()
        .execute(
            "UPDATE install_run SET state = 'done', stage = 'settled', ended_at = ?2 WHERE id = ?1",
            rusqlite::params![run.0, ctx.now],
        )
        .map_err(|_| InstallFailure::RenameFailed)?;
    Ok(())
}

/// Read one root's own facts back for a run.
///
/// # Errors
/// Fails when `scan_root` cannot be read.
pub fn root_facts(
    tx: &rusqlite::Transaction<'_>,
    root: crate::protocol::RootId,
) -> rusqlite::Result<Option<RootFacts>> {
    use rusqlite::OptionalExtension as _;
    tx.query_row(
        "SELECT kind, distro, path_bytes FROM scan_root WHERE id = ?1",
        [root.0],
        |row| {
            Ok(RootFacts {
                root_id: root.0,
                kind: row.get::<_, String>(0)?,
                distro: row.get::<_, String>(1)?,
                path: crate::paths::path_from_bytes(&row.get::<_, Vec<u8>>(2)?),
            })
        },
    )
    .optional()
}

/// Record a stage and put it on the wire, in that order.
///
/// The store first: a subscriber that arrives between the two reads the snapshot and sees the
/// stage anyway, whereas the reverse order has a window in which the event has been sent and the
/// snapshot still says something older.
fn publish(
    ctx: &InstallCtx<'_>,
    run: InstallRunId,
    observed: crate::install::stage::StageObservation,
) {
    let stage = crate::protocol::InstallStage {
        run_id: run,
        stage: observed.kind,
        done: observed.done,
        total: observed.total,
        bytes: observed.bytes,
    };
    ctx.stages.record(stage.clone());
    if let Ok(payload) = serde_json::to_value(&stage) {
        ctx.events.emit("install", "stage", payload);
    }
}

/// §24.3c's cancel, end to end.
///
/// **Kills the group, not the child.** `WriteExec` spawns through `CommandGroup::group_spawn`
/// (`core/src/gitw/exec.rs:240`) and kills through the group handle when this token fires; a
/// Windows child killed without a Job Object leaves orphans holding file locks, and the staging
/// removal then fails with *Access is denied*. Recorded in this repository and in §24.3c.
///
/// The order after the kill is fixed and is what `install_staging.rs` asserts: the group is gone,
/// the staging directory is removed **under the staging warrant**, the `install_run` row reads
/// `cancelled`, and no `location` row exists — the run never reached the rename.
///
/// # Errors
/// Fails when the run is not one this core is currently running.
pub fn cancel_install(
    queue: &crate::install::queue::InstallQueue,
    run: InstallRunId,
) -> Result<(), crate::proto::dispatch::CommandFailure> {
    if queue.cancel(run) {
        return Ok(());
    }
    Err(crate::proto::dispatch::CommandFailure::internal(format!(
        "install run {} is not running here",
        run.0
    )))
}

/// What a run does once it has ended badly: clean up its own staging directory and say so.
///
/// Called on **every** failure arm, not only cancellation, because every one of them can leave a
/// partial clone in staging. Removal goes through the warrant, so a run that somehow no longer
/// has a durable row removes nothing and the sweep reports it instead.
pub fn finish_failed(
    ctx: &InstallCtx<'_>,
    run: InstallRunId,
    project: crate::protocol::ProjectId,
    reason: InstallFailure,
) {
    let removed = {
        let _guard = crate::proto::txguard::TxGuard::enter();
        let held = ctx
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let warrant = held.conn().unchecked_transaction().ok().and_then(|tx| {
            crate::install::staging::staging_warrant_for(&tx, run)
                .ok()
                .flatten()
        });
        warrant.is_some_and(|warrant| {
            crate::removal::remove_warranted(&warrant, &crate::removal::HardDelete).is_ok()
        })
    };
    let _ = removed;

    let state = if reason == InstallFailure::Cancelled {
        "cancelled"
    } else {
        "failed"
    };
    {
        let _guard = crate::proto::txguard::TxGuard::enter();
        let held = ctx
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = held.conn().execute(
            "UPDATE install_run SET state = ?2, reason = ?3, ended_at = ?4 WHERE id = ?1",
            rusqlite::params![
                run.0,
                state,
                serde_json::to_value(reason)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned)),
                ctx.now
            ],
        );
    }
    if let Ok(payload) = serde_json::to_value(crate::protocol::InstallFailed {
        run_id: run,
        project_id: project,
        reason,
        // The enum is what a surface branches on; this sentence may sit beside it, never in
        // place of it.
        detail: String::new(),
    }) {
        ctx.events.emit("install", "failed", payload);
    }
}

//! Install's durable run-state vocabulary, and where a clone lands.

pub mod destination;
pub mod queue;
pub mod run;
pub mod staging;

pub use destination::{compose_destination, is_safe_path_segment, refuse_unsafe_name};
pub use staging::{staging_path_for, staging_warrant_for, StagingSweepReport, STAGING_DIR_NAME};

use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{InstallPreview, InstallPreviewArgs};
use crate::surfaces::SurfaceCtx;

/// Preview the exact destination named by the supplied `RootId`.
///
/// This is a read-only transaction and spawns no process. The chooser may preview a root before
/// storing it, so this command deliberately does not consult `install_root_id`.
///
/// # Errors
/// Returns `PROTOCOL` for malformed arguments and `INTERNAL` for an index fault. A domain refusal
/// is returned inside [`InstallPreview`], never promoted to a command failure.
pub fn handle_preview(
    ctx: &SurfaceCtx<'_>,
    args: serde_json::Value,
) -> Result<InstallPreview, CommandFailure> {
    let args: InstallPreviewArgs = parse_args(args)?;
    let internal = |error: rusqlite::Error| CommandFailure::internal(error.to_string());
    let _tx_guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    let composed = destination::compose_destination_checked(&tx, args.project_id, args.root_id)
        .map_err(internal)?;
    drop(tx);

    Ok(match composed {
        Ok(destination) => InstallPreview {
            destination: Some(destination),
            refused_because: None,
        },
        Err(refused_because) => InstallPreview {
            destination: None,
            refused_because: Some(refused_because),
        },
    })
}

/// The lifecycle state stored in `install_run.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallRunState {
    /// The clone is still in progress.
    Running,
    /// The clone and location commit completed.
    Done,
    /// The run ended unsuccessfully.
    Failed,
    /// The user cancelled the run.
    Cancelled,
}

impl InstallRunState {
    /// Every stored state, so the Rust vocabulary can be checked against the database column.
    pub const ALL: [InstallRunState; 4] = [
        InstallRunState::Running,
        InstallRunState::Done,
        InstallRunState::Failed,
        InstallRunState::Cancelled,
    ];

    /// The value stored in `install_run.state`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            InstallRunState::Running => "running",
            InstallRunState::Done => "done",
            InstallRunState::Failed => "failed",
            InstallRunState::Cancelled => "cancelled",
        }
    }
}

/// §24.9's `install.start`: compose, record, queue, and begin.
///
/// **The verdict is recomputed here**, inside the mutating call, and the start refuses if it
/// changed — a verdict rendered thirty seconds ago in a preview is a cache, and the destination
/// may have been occupied since.
///
/// The clone itself runs on its own thread: `install.start` answers with a run id, and §24.4's
/// stages arrive as events. A command that blocked for the length of a clone would hold the
/// protocol loop for minutes.
///
/// # Errors
/// Fails when the arguments do not parse or the run row cannot be written. A **refusal** is not a
/// failure: it comes back in `InstallStart.refusedBecause`.
pub fn handle_start(
    ctx: &StartCtx<'_>,
    args: serde_json::Value,
) -> Result<crate::protocol::InstallStart, CommandFailure> {
    use crate::protocol::{InstallStart, InstallStartArgs};

    let args: InstallStartArgs = parse_args(args)?;
    let _guard = TxGuard::enter();
    let held = ctx
        .index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tx = held
        .conn()
        .unchecked_transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    let destination = match destination::compose_destination(&tx, args.project_id, args.root_id) {
        Ok(destination) => destination,
        Err(refused) => {
            return Ok(InstallStart {
                run_id: None,
                refused_because: Some(refused),
            })
        }
    };
    let root = match run::root_facts(&tx, args.root_id) {
        Ok(Some(root)) => root,
        Ok(None) => {
            return Ok(InstallStart {
                run_id: None,
                refused_because: Some(crate::protocol::InstallRefusal::RootUnavailable),
            })
        }
        Err(e) => return Err(CommandFailure::internal(e.to_string())),
    };
    let clone_url = match destination::clone_url(&tx, args.project_id) {
        Ok(Some(url)) => url,
        Ok(None) => {
            return Ok(InstallStart {
                run_id: None,
                refused_because: Some(crate::protocol::InstallRefusal::NoCloneUrl),
            })
        }
        Err(e) => return Err(CommandFailure::internal(e.to_string())),
    };
    let Ok(paths) = run::paths_for(&root.path, &destination.seed_basename) else {
        return Ok(InstallStart {
            run_id: None,
            refused_because: Some(crate::protocol::InstallRefusal::RootUnavailable),
        });
    };

    let request = queue::InstallRequest {
        project: args.project_id,
        root: args.root_id,
        destination,
    };
    let run_id = ctx
        .queue
        .push(
            &tx,
            request.clone(),
            &crate::paths::path_bytes(&paths.staging),
            &crate::paths::path_bytes(&paths.destination),
            ctx.now,
        )
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    tx.commit()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    drop(held);

    (ctx.begin)(run_id, request, root, paths, clone_url);
    Ok(InstallStart {
        run_id: Some(run_id),
        refused_because: None,
    })
}

/// What `handle_start` needs beyond its arguments.
///
/// `begin` is a callback rather than the seams themselves so this function stays testable without
/// a git binary: the assembly passes one that spawns the run thread, a test passes one that
/// records what it was asked to start.
pub struct StartCtx<'a> {
    pub index: &'a std::sync::Arc<std::sync::Mutex<crate::index::Index>>,
    pub queue: &'a queue::InstallQueue,
    pub now: i64,
    #[allow(clippy::type_complexity)]
    pub begin: &'a dyn Fn(
        crate::protocol::InstallRunId,
        queue::InstallRequest,
        run::RootFacts,
        run::RunPaths,
        String,
    ),
}

impl std::fmt::Debug for StartCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

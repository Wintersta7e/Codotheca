//! Install's durable run-state vocabulary, and where a clone lands.

pub mod destination;
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

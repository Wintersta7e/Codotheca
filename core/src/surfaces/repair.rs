//! §11.1's two drawn controls. Both are non-destructive: `setTrusted` writes one
//! nullable column in Codotheca's own database, and `requeue` moves scheduling
//! state for exactly one project.

use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    ErrorCode, LocationId, LocationsSetTrustedArgs, ProjectId, ProjectsRequeueArgs, RequeueResult,
};
use crate::surfaces::SurfaceCtx;

/// # Errors
/// Fails when the arguments do not parse, when no such location exists, or when the index
/// cannot be written.
pub fn handle_set_trusted(
    ctx: &SurfaceCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let args: LocationsSetTrustedArgs = parse_args(args)?;
    let found = set_trusted(ctx.index.conn(), args.location_id, ctx.now)
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    if !found {
        return Err(CommandFailure {
            code: ErrorCode::PathGone,
            message: format!("no location {}", args.location_id.0),
            // [R31] `Outcome` has only `unknown`; None = definitely did not take effect.
            outcome: None,
        });
    }
    Ok(serde_json::json!({}))
}

/// # Errors
/// Fails when the arguments do not parse, or when the index cannot be written.
pub fn handle_requeue(
    ctx: &SurfaceCtx<'_>,
    args: serde_json::Value,
) -> Result<RequeueResult, CommandFailure> {
    let args: ProjectsRequeueArgs = parse_args(args)?;
    let jobs_requeued = requeue(ctx.index.conn(), args.id, ctx.now)
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    Ok(RequeueResult { jobs_requeued })
}

/// §3.2 requires trust to be recorded somewhere Codotheca owns; §17 forbids
/// writing the user's git config. `location.trusted_at` is that record, and the
/// `-c safe.directory=<exact-path>` argument is assembled core-side from it.
///
/// Answers whether a row was found — a missing id is reported, never inserted.
///
/// # Errors
/// Fails when the index cannot be written.
pub fn set_trusted(
    conn: &rusqlite::Connection,
    location: LocationId,
    now: i64,
) -> Result<bool, IndexError> {
    let _guard = TxGuard::enter();
    let changed = conn.execute(
        "UPDATE location SET trusted_at = ?2 WHERE id = ?1",
        rusqlite::params![location.0, now],
    )?;
    Ok(changed == 1)
}

/// §11.1: re-queues exactly this project's failed and deferred rows, and clears the
/// never-succeeded state the button is offered from.
///
/// # Errors
/// Fails when the index cannot be written.
pub fn requeue(
    conn: &rusqlite::Connection,
    project: ProjectId,
    now: i64,
) -> Result<u32, IndexError> {
    let _guard = TxGuard::enter();
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE project_job_state
         SET state = 'queued', fail_count = 0, reason = NULL, at = ?2
         WHERE project_id = ?1 AND state IN ('failed', 'deferred_slow')",
        rusqlite::params![project.0, now],
    )?;
    tx.execute(
        "UPDATE project SET error_kind = NULL, error_detail = NULL, error_at = NULL
         WHERE id = ?1",
        [project.0],
    )?;
    tx.commit()?;
    Ok(u32::try_from(changed).unwrap_or(u32::MAX))
}

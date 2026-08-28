//! §2.4's three `scan.*` commands (R37). The scanner had every part but this one.
//!
//! R15: `parse_args` and `CommandFailure` are plan 03's, in `crate::proto::dispatch`. They are
//! imported, not re-declared and not re-exported, so there is one path to each.
//!
//! Three decisions are recorded here because each has a wrong answer that compiles.
//!
//! **`scan.start` coalesces.** Two runs in flight would each take a `next_generation`, and the
//! higher would mark every location the other is still walking `missing` (§4.6). So while a run
//! is live, `scan.start` returns the live run's id and starts nothing. Refusing is worse: the
//! callers are `SCAN` in the top bar and `RESCAN NOW` in the drawer, a second click means "I want
//! a scan", and that is already true — an error window for a system doing exactly what was asked
//! teaches the user the button is broken.
//!
//! **`scan.cancel` cancels through the token and removes nothing.** Everything already committed
//! stays committed; presence is not applied to a cancelled run; no row is deleted anywhere,
//! because `ScanStore` has no method that removes one. Cancelling a run that is no longer live is
//! a no-op success — a run can finish between the `scan.status` that drew the control and the
//! click that used it, and reporting that race as a fault is a lie about the system's state.
//!
//! **`scan.status` answers "what is true now", and `runId` separates the two empties.** Progress
//! arrives as events on the `scan` topic; this exists for the client that has no event history
//! because it just connected. `null` is *no scan has ever run*; `Some(id)` with `foundRepos: 0`
//! is *a scan ran and found nothing*. Plan 16's first-run gate is exactly `status.runId === null`.

use serde::de::IntoDeserializer as _;
use serde_json::Value;

use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    ScanCancelArgs, ScanMode, ScanRunId, ScanRunStarted, ScanStartArgs, ScanStatus,
};
use crate::scan::ScanCtx;

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

/// The wire spelling of a `ScanMode` lives in the generated `#[serde(rename = …)]` and nowhere
/// else (R31), so the `scan_run.mode` column is read back through serde rather than through a
/// second table of two strings.
fn scan_mode(raw: &str) -> Option<ScanMode> {
    let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
        raw.into_deserializer();
    serde::Deserialize::deserialize(de).ok()
}

/// §2.4: the only argument is `full`. No path crosses the wire — the roots come from `scan_root`,
/// read by the launcher.
pub fn handle_start(ctx: &ScanCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: ScanStartArgs = parse_args(args)?;
    let mode = if args.full {
        ScanMode::Full
    } else {
        ScanMode::Incremental
    };
    let (live, started) = ctx
        .scans
        .start(mode, ctx.now)
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    if started {
        let payload = encode(&ScanRunStarted {
            run_id: live.scan_run_id,
            generation: u32::try_from(live.generation).unwrap_or(u32::MAX),
            mode: live.mode,
            roots: live.roots.clone(),
            started_at: live.started_at,
        })?;
        ctx.events.emit("scan", "run_started", payload);
    }
    encode(&live.scan_run_id)
}

/// §4.8, cooperative. Returns an empty object whether or not the id named the live run.
///
/// It deliberately emits nothing: `ScanCancelled` carries `endedAt` and `indexedProjects`, which
/// are true only once the run has stopped. Cancelling also does not clear the live slot — the
/// run's own completion does. A request is not an ending.
pub fn handle_cancel(ctx: &ScanCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: ScanCancelArgs = parse_args(args)?;
    let _cancelled = ctx.scans.cancel(args.id);
    Ok(serde_json::json!({}))
}

pub fn handle_status(ctx: &ScanCtx<'_>) -> Result<Value, CommandFailure> {
    encode(&scan_status(ctx)?)
}

/// What is true now, for a client with no event history.
///
/// `problem_count` and `ambiguous_lineage_count` are always `None` here. They are
/// `problems.list`'s header, plan 17 computes them once, and no consumer reads them from this
/// type — recomputing them here would put one number in two places. The three non-nullable
/// counters read `0` beside a `None` `run_id`, which is the wire's floor and not a claim about a
/// run that did not happen: no consumer reads them without reading `run_id` or `running` first.
pub fn scan_status(ctx: &ScanCtx<'_>) -> Result<ScanStatus, CommandFailure> {
    let indexed_projects = ctx
        .store
        .indexed_project_count()
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .map_err(|e| CommandFailure::internal(e.to_string()))?;

    if let Some(live) = ctx.scans.live() {
        let (walked_dirs, found_repos) = live.progress.snapshot();
        return Ok(ScanStatus {
            run_id: Some(live.scan_run_id),
            running: true,
            generation: u32::try_from(live.generation).ok(),
            mode: Some(live.mode),
            started_at: Some(live.started_at),
            ended_at: None,
            // A cancel was asked for and the run has not stopped yet. Both are true.
            cancelled: live.cancel.is_cancelled(),
            walked_dirs: i64::try_from(walked_dirs).unwrap_or(i64::MAX),
            found_repos: u32::try_from(found_repos).unwrap_or(u32::MAX),
            indexed_projects,
            problem_count: None,
            ambiguous_lineage_count: None,
        });
    }

    let Some(row) = ctx
        .store
        .latest_scan_run()
        .map_err(|e| CommandFailure::internal(e.to_string()))?
    else {
        // No scan has ever run. `run_id: None` is that statement; the zeroes beside it are the
        // wire's floor, not a count.
        return Ok(ScanStatus {
            run_id: None,
            running: false,
            generation: None,
            mode: None,
            started_at: None,
            ended_at: None,
            cancelled: false,
            walked_dirs: 0,
            found_repos: 0,
            indexed_projects,
            problem_count: None,
            ambiguous_lineage_count: None,
        });
    };

    Ok(ScanStatus {
        run_id: Some(ScanRunId(row.id)),
        running: false,
        generation: u32::try_from(row.generation).ok(),
        mode: scan_mode(&row.mode),
        started_at: Some(row.started_at),
        ended_at: row.ended_at,
        cancelled: row.cancelled,
        walked_dirs: row.walked_dirs,
        found_repos: u32::try_from(row.found_repos).unwrap_or(u32::MAX),
        indexed_projects,
        problem_count: None,
        ambiguous_lineage_count: None,
    })
}

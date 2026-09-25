//! §24.7's two commands, assembled over the analyser.
//!
//! **This is the layer p2-24b's route arms named and did not build**, now the analyser's caller:
//! the row is read under the index guard, the guard is dropped, and [`analyse`] runs — every git
//! read and the verifying read of every remote off the lock.
//!
//! **Neither handler holds the index mutex across the network.** `SqliteScanStore` takes the
//! same `std::sync::Mutex`, which is not reentrant — so these take the guard, read, drop it, go to
//! the network, and take it again. It is `readme::handle_readme_assets_off_lock`'s shape, for the
//! same reason.
//!
//! **The verdict is computed once per call, by `analyse`.** Two computations at two freshnesses
//! is the drift this project has already paid for four times, and on a safety verdict it is the
//! difference between refusing a removal and performing one.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use crate::analyser::identity::{live_identity, LiveIdentity};
use crate::analyser::{
    analyse, read_location_row, resolve, AnalyserSeams, GovernedAct, LocationRow,
};
use crate::git::{JobClass, JobContext};
use crate::gitw::intent::GIT_INVOCATION_DEADLINE;
use crate::index::Index;
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::protocol::{
    LocationDetail, LocationId, LocationsUninstallArgs, LocationsUninstallPreflightArgs,
    UninstallDisposition,
};
use crate::uninstall::uninstall_location;

fn internal(e: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(e.to_string())
}

/// The one row read, under the guard and in one read transaction.
// **A read transaction, not a bare connection**: `gate_live_session` takes a `&Transaction`,
// and every rusqlite transaction in this core opens through `TxGuard`.
fn read_row(index: &Arc<Mutex<Index>>, id: LocationId) -> Result<LocationRow, CommandFailure> {
    let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    guard
        .with_tx(|tx| Ok(read_location_row(tx, id)))
        .map_err(internal)?
}

/// §24.7's pre-flight. **Unprivileged, and not callable on hover** — its only write is objects,
/// through §47.4's verifying read, and it goes to the network (R186).
///
/// # Errors
/// `PROTOCOL` for an argument shape the schema does not admit or an id that names no location;
/// `INTERNAL` for an index fault. A *blocked* verdict is a success, not an error.
pub fn handle_preflight_off_lock(
    index: &Arc<Mutex<Index>>,
    seams: &AnalyserSeams<'_>,
    args: Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let a: LocationsUninstallPreflightArgs = parse_args(args)?;
    let row = read_row(index, a.location_id)?;
    let analysis = analyse(&row, GovernedAct::Uninstall, seams, now);
    serde_json::to_value(analysis.verdict).map_err(internal)
}

/// §24.8's removal. **The verdict it acts on is the one it computes, inside this call.**
///
/// A verdict rendered thirty seconds ago is a cache, and *never claim currency you do not have*
/// applies to a safety verdict more than to anything else in this product. Nothing about the
/// verdict crosses a call boundary to get here: the argument is a `LocationId`.
///
/// # Errors
/// `PROTOCOL` when the freshly computed verdict is not `safe`, when the directory's identity no
/// longer matches its row, or when the removal itself refuses; `INTERNAL` for an index fault.
pub fn handle_uninstall_off_lock(
    index: &Arc<Mutex<Index>>,
    seams: &AnalyserSeams<'_>,
    args: Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let a: LocationsUninstallArgs = parse_args(args)?;
    let row = read_row(index, a.location_id)?;
    let analysis = analyse(&row, GovernedAct::Uninstall, seams, now);
    if analysis.verdict.disposition != UninstallDisposition::Safe {
        return Err(CommandFailure::protocol(format!(
            "uninstall refused: {:?} — {:?}",
            analysis.verdict.disposition, analysis.verdict.blockers
        )));
    }

    // §24.7E against the **row** (§45.6 step 1): the warrant expects the row's
    // `project.lineage_key` and is sealed over this analysis; the identity re-derived from the
    // directory at removal time is compared with it inside `remove_warranted`. Phase 2 built
    // `expected` from the directory it then checked, so a replaced directory matched itself
    // (§37.8).
    let warrant = crate::removal::Warrant::for_uninstall(
        row.id,
        row.path.clone(),
        row.lineage_key.clone(),
        analysis.seal.clone(),
    );
    let cancel = crate::cancel::CancelToken::new();
    let identity_now = resolve(&row).map_or(LiveIdentity::Underivable, |repo| {
        live_identity(
            seams.git,
            &repo,
            &JobContext::new(
                JobClass::Interactive,
                &cancel,
                Some(GIT_INVOCATION_DEADLINE),
            ),
        )
    });

    // **The refusal is carried out as a value, not flattened into an `IndexError`.** §2.2's code
    // is the whole answer here — a refused removal is `PROTOCOL`, and a retry is pointless —
    // whereas an `IndexError` reads as a fault in the index and carries `INTERNAL`.
    //
    // Committing an unchanged transaction on that path is safe, and it is safe for a stated
    // reason rather than by luck: `uninstall_location` writes **once**, in a single `UPDATE`
    // after the bytes are already gone (`core/src/uninstall/command.rs`), so every refusal
    // returns before anything is written and there is no partial state to roll back. A second
    // write added there would break that, which is what this note is for.
    let mut guard = index.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = guard
        .with_tx(|tx| {
            Ok(uninstall_location(
                tx,
                &analysis,
                &warrant,
                seams.trash,
                &identity_now,
                now,
            ))
        })
        .map_err(internal)??;
    // The same projection RELOCATE returns — the one the page already reads, never a second one.
    let detail: LocationDetail =
        crate::detail::get::location_detail(guard.conn(), removed.location)?;
    drop(guard);
    serde_json::to_value(detail).map_err(internal)
}

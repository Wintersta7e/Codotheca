//! §43: the motivation layer's presence — the one read the status band draws, the line that
//! waits for a click, and quiet mode, the closed list that hides the game and keeps every fact.
//!
//! `keys` owns the `app_meta` keys §43 adds, one reader and one writer each. `state` builds the
//! band's one payload; `line` holds its queue and `reason` what a rise is owed to.
//! `first_run` decides when the first-run stamp is written. `acknowledge` records what a
//! presentation the user saw writes; `absorb` is what a rise the user did not cause writes
//! instead. `publish` pushes the payload when it changed; `quiet` is what leaving quiet mode
//! writes. The two commands below answer §43.15.

pub mod absorb;
pub mod acknowledge;
pub mod first_run;
pub mod keys;
pub mod line;
pub mod publish;
pub mod quiet;
pub mod reason;
pub mod state;

use crate::clock::LocalZone;
use crate::index::{Index, IndexError};
use crate::proto::dispatch::{parse_args, CommandFailure};
use crate::proto::txguard::TxGuard;
use crate::protocol::{
    MotivationAck, MotivationAcknowledgeArgs, MotivationGetArgs, MotivationState, ScanRunId,
};

use acknowledge::acknowledge;
use first_run::FirstRunStamp;
use state::motivation_state;

/// The commands this module owns, as data, so the seam and the router's table cannot drift apart.
pub const MOTIVATION_COMMANDS: [&str; 2] = ["motivation.get", "motivation.acknowledge"];

/// What both commands are answered with.
#[derive(Debug)]
pub struct MotivationCtx<'a> {
    /// The index, under the handler's guard.
    pub index: &'a Index,
    /// The command's instant, in Unix seconds.
    pub now: i64,
    /// The machine's zone: day one and every local date the payload reads are `now`'s in it.
    pub zone: &'a dyn LocalZone,
    /// The scan run in progress, if any: `firstRunEnded` stamps at its end rather than now.
    pub live_run: Option<ScanRunId>,
    /// The handler's one first-run stamp, which `firstRunEnded` arms or writes.
    pub first_run: &'a mut FirstRunStamp,
    /// Set when this command wrote the first-run stamp; the caller then re-publishes
    /// `core/snapshot`, which carries it.
    pub first_run_stamped: bool,
}

/// `None` means "this module does not own that command", which is what the router chains on.
#[must_use]
pub fn dispatch_motivation_command(
    ctx: &mut MotivationCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "motivation.get" => Some(handle_get(ctx, args)),
        "motivation.acknowledge" => Some(handle_acknowledge(ctx, args)),
        _ => None,
    }
}

fn internal(error: impl std::fmt::Display) -> CommandFailure {
    CommandFailure::internal(error.to_string())
}

/// The band's one read. It writes nothing of §43's.
fn handle_get(
    ctx: &MotivationCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let MotivationGetArgs {} = parse_args(args)?;
    let _tx_guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    let state = motivation_state(&tx, ctx.now, ctx.zone).map_err(internal)?;
    tx.commit().map_err(internal)?;
    serde_json::to_value(state).map_err(internal)
}

/// Records the acknowledgement and answers the payload after the writes, all in one
/// transaction. A failure leaves the first-run stamp's arm as it was, since nothing it wrote was
/// committed.
fn handle_acknowledge(
    ctx: &mut MotivationCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let MotivationAcknowledgeArgs { event, level } = parse_args(args)?;
    let _tx_guard = TxGuard::enter();
    let tx = ctx.index.conn().unchecked_transaction().map_err(internal)?;
    let armed = *ctx.first_run;
    let answered = record(&tx, ctx, event, level)
        .map_err(internal)
        .and_then(|answer| tx.commit().map(|()| answer).map_err(internal));
    let (stamped, state) = answered.inspect_err(|_| *ctx.first_run = armed)?;
    ctx.first_run_stamped = stamped;
    serde_json::to_value(state).map_err(internal)
}

/// `event`'s writes, then the end of first run when it is one — against the run in progress —
/// and the payload they leave. `true` beside it when the first-run stamp was written.
fn record(
    tx: &rusqlite::Transaction<'_>,
    ctx: &mut MotivationCtx<'_>,
    event: MotivationAck,
    level: Option<u32>,
) -> Result<(bool, MotivationState), IndexError> {
    let effect = acknowledge(tx, event, level, ctx.now, ctx.zone)?;
    let stamped = effect.first_run_ended
        && ctx
            .first_run
            .on_first_run_ended(tx, ctx.live_run, ctx.now)?;
    Ok((stamped, motivation_state(tx, ctx.now, ctx.zone)?))
}

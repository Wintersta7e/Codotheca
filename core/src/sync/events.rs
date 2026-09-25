//! The six `sync` events, and `SyncStatus`'s assembly.
//!
//! **A `SyncStatus` has two sources and neither is a copy of the other.** `tasks` and `budgets`
//! are rows; `listing` and `notice` are **process-lifetime** state with no column anywhere, so
//! the runner owns them and hands a [`SyncLive`] over. After a restart the live half is empty,
//! which is the truth — this process has observed nothing yet — rather than a value restored from
//! a table that never held one.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::index::IndexError;
use crate::proto::EventSink;
use crate::protocol::{
    AccountId, SyncBudget, SyncListingProgress, SyncListingSummary, SyncNotice, SyncOutcomeKind,
    SyncStatus, SyncTaskKind, SyncTaskSettled, SyncTaskStarted,
};
use crate::sync::state::SyncTaskStateRow;
use crate::sync::store::load_all;

/// What one task step settled into, **as this process saw it**.
///
/// `sync_task_state` stores a state and a reason and has no column for either of these — §21.13's
/// DDL declares none — so they live here, with the run that observed them, and are empty after a
/// restart. That emptiness is the truth rather than a shortfall: nothing in *this* process has
/// observed an outcome yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastSettle {
    pub outcome: SyncOutcomeKind,
    pub summary: Option<SyncListingSummary>,
}

/// What only the running process knows.
///
/// Empty is *nothing observed in this process*, which is exactly what a fresh start is. There is
/// no column for any of it and there must not be: a listing's progress is meaningless across a
/// restart, and a notice restored from a table would claim a failure nothing has re-observed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncLive {
    pub listing: Option<SyncListingProgress>,
    pub notice: Option<SyncNotice>,
    /// Keyed by `(kind, key)`, exactly as `sync_task_state` is.
    pub last: HashMap<(SyncTaskKind, Option<i64>), LastSettle>,
}

/// One stored row as the wire's settled shape, plus whatever this process observed about it.
///
/// **`outcome` and `summary` come from `last`, never from the row.** `sync_task_state` stores the
/// *state* a row settled into and the *reason* it carries; it has no column for either of the
/// other two, and §21.13's DDL declares none. Deriving an outcome back from `(state, reason)`
/// would restate the classifier's decision without its inputs, which is the defect §21.13's own
/// `state` field exists to avoid from the other direction. `None` is therefore **not observed
/// here** — a queued row has produced none, and a process that has just started has seen none —
/// and a surface must not render it as *never settled*.
///
/// `notBefore` is `None` for a row with nothing scheduled — a terminal row's stored `0` is the
/// column default and not an instant, and rendering it as one would put 1970 on a screen.
#[must_use]
pub fn settled_of(row: &SyncTaskStateRow, last: Option<&LastSettle>) -> SyncTaskSettled {
    SyncTaskSettled {
        kind: row.kind,
        key: row.key,
        state: row.state,
        outcome: last.map(|l| l.outcome),
        not_before: (row.not_before > 0).then_some(row.not_before),
        summary: last.and_then(|l| l.summary.clone()),
    }
}

/// Every `sync_budget` row, as the wire sees it.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn budgets(conn: &Connection) -> Result<Vec<SyncBudget>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT account_id, resource, remaining, limit_, reset_at, observed_at
           FROM sync_budget ORDER BY resource, account_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SyncBudget {
                account_id: row.get::<_, Option<i64>>(0)?.map(AccountId),
                resource: row.get(1)?,
                remaining: row.get(2)?,
                limit: row.get(3)?,
                reset_at: row.get(4)?,
                observed_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `sync.status`' answer, and — byte for byte — the `sync` topic's snapshot.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn status_payload(conn: &Connection, live: &SyncLive) -> Result<SyncStatus, IndexError> {
    Ok(SyncStatus {
        tasks: load_all(conn)?
            .iter()
            .map(|row| settled_of(row, live.last.get(&(row.kind, row.key))))
            .collect(),
        budgets: budgets(conn)?,
        listing: live.listing.clone(),
        notice: live.notice,
    })
}

/// A task moved to `running`. Carries no outcome, because none exists yet.
pub fn emit_started(events: &dyn EventSink, started: &SyncTaskStarted) {
    emit(events, "started", started);
}

/// A task settled. **This is the only carrier of an observed outcome**, and the only place a
/// listing summary reaches a subscriber.
pub fn emit_settled(events: &dyn EventSink, settled: &SyncTaskSettled) {
    emit(events, "settled", settled);
}

/// §21.11's monotone count. Never a percentage, and `total` is `null` unless a response supplied
/// one.
pub fn emit_progress(events: &dyn EventSink, progress: &SyncListingProgress) {
    emit(events, "listing_progress", progress);
}

/// One pool's latest observation, so a surface renders `—` for what was never observed rather
/// than a zero nobody measured.
pub fn emit_budget(events: &dyn EventSink, budget: &SyncBudget) {
    emit(events, "budget", budget);
}

/// [p3] §32.12's alert — **the only event on this topic that is not a status delta.**
///
/// It carries no snapshot and is not replayed: a subscriber that missed it has missed the
/// notification, which is why the shell subscribes before the core is asked for anything. The
/// ledger it was decided from is what stops a second one, not a re-send.
pub fn emit_advisory_alert(events: &dyn EventSink, alert: &crate::protocol::AdvisoryAlert) {
    emit(events, "advisory_alert", alert);
}

/// §21.10's **one** non-modal banner. One candidate whatever the number of failed tasks.
pub fn emit_notice(events: &dyn EventSink, notice: SyncNotice) {
    emit(events, "notice", &notice);
}

/// The whole of `sync.status`, re-sent.
///
/// **This exists because a `notice` event cannot say *none*.** `SyncNotice` is a bare enum on the
/// wire, so a settle that clears the banner had nothing to send — and the renderer sets its notice
/// from a `notice` event or from a snapshot and from nothing else, so a `throttled` banner raised
/// at T stayed on screen for the life of the session after sync recovered. Making the bare enum
/// nullable would give one value two representations (R12); the snapshot already carries
/// `notice: SyncNotice | null`, which is the honest carrier for the cleared case.
///
/// **A delta named `snapshot`, not a §2.3 snapshot frame, and that is load-bearing.** The shell
/// subscribes with `onSnapshot: () => undefined` (`app/src/main/index.ts`), so a real snapshot
/// frame is discarded before it reaches the renderer: what `useSync` calls a snapshot is an
/// **event** named `snapshot`, delivered through `onEvent`. Sent any other way this would arrive
/// nowhere.
pub fn emit_snapshot(events: &dyn EventSink, status: &SyncStatus) {
    emit(events, "snapshot", status);
}

/// The one serialisation point, so a payload that cannot be serialised is dropped in one place
/// rather than panicking a worker thread — the core supervises a window, and a panic closes it.
fn emit<T: serde::Serialize>(events: &dyn EventSink, event: &str, payload: &T) {
    match serde_json::to_value(payload) {
        Ok(value) => events.emit("sync", event, value),
        // **No frame rather than a false one.** An empty object is a well-formed payload that
        // reads as *a listing of zero, no notice, no budget* — an unknown rendered as a fact,
        // which is the one thing §21 may not do. Still never a panic: the core supervises a
        // window and a panic closes it. Unreachable with today's types, which are all plain
        // structs of numbers and strings.
        Err(e) => eprintln!("sync: dropping a {event} event that will not serialise: {e}"),
    }
}

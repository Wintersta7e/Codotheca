//! The six `sync` events, and `SyncStatus`'s assembly.
//!
//! **A `SyncStatus` has two sources and neither is a copy of the other.** `tasks` and `budgets`
//! are rows; `listing` and `notice` are **process-lifetime** state with no column anywhere, so
//! the runner owns them and hands a [`SyncLive`] over. After a restart the live half is empty,
//! which is the truth — this process has observed nothing yet — rather than a value restored from
//! a table that never held one.

use rusqlite::Connection;
use serde_json::json;

use crate::index::IndexError;
use crate::proto::EventSink;
use crate::protocol::{
    AccountId, SyncBudget, SyncListingProgress, SyncNotice, SyncStatus, SyncTaskSettled,
    SyncTaskStarted,
};
use crate::sync::state::SyncTaskStateRow;
use crate::sync::store::load_all;

/// What only the running process knows.
///
/// Empty is *nothing observed in this process*, which is exactly what a fresh start is. There is
/// no column for either field and there must not be: a listing's progress is meaningless across a
/// restart, and a notice restored from a table would claim a failure nothing has re-observed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncLive {
    pub listing: Option<SyncListingProgress>,
    pub notice: Option<SyncNotice>,
}

/// One stored row as the wire's settled shape.
///
/// **`outcome` is `None` here and that is not a shortfall.** `sync_task_state` stores the *state*
/// a row settled into and the *reason* it carries; it has no outcome column, and §21.13's DDL
/// declares none. An observed outcome travels on the `settled` event, at the moment it is
/// observed. Deriving one back from `(state, reason)` would restate the classifier's decision
/// without its inputs, which is the defect §21.13's own `state` field exists to avoid from the
/// other direction.
///
/// `notBefore` is `None` for a row with nothing scheduled — a terminal row's stored `0` is the
/// column default and not an instant, and rendering it as one would put 1970 on a screen.
#[must_use]
pub fn settled_of(row: &SyncTaskStateRow) -> SyncTaskSettled {
    SyncTaskSettled {
        kind: row.kind,
        key: row.key,
        state: row.state,
        outcome: None,
        not_before: (row.not_before > 0).then_some(row.not_before),
        summary: None,
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
        tasks: load_all(conn)?.iter().map(settled_of).collect(),
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

/// §21.10's **one** non-modal banner. One candidate whatever the number of failed tasks.
pub fn emit_notice(events: &dyn EventSink, notice: SyncNotice) {
    emit(events, "notice", &notice);
}

/// The one serialisation point, so a payload that cannot be serialised is dropped in one place
/// rather than panicking a worker thread — the core supervises a window, and a panic closes it.
fn emit<T: serde::Serialize>(events: &dyn EventSink, event: &str, payload: &T) {
    let value = serde_json::to_value(payload).unwrap_or_else(|_| json!({}));
    events.emit("sync", event, value);
}

//! `sync_task_state` reads and writes, and the crash re-queue.
//!
//! Every function here takes a `&Transaction` or a `&Connection` **from its caller**, which is
//! how every writer in this codebase is shaped; the caller opens the transaction through
//! `crate::proto::txguard::TxGuard`.

use rusqlite::{Connection, Transaction};

use crate::index::IndexError;
use crate::protocol::{AccountId, SyncTaskKind, SyncTaskState};
use crate::sync::state::{state_slug, SyncResetCause, SyncTaskStateRow, SYNC_STATES};
use crate::sync::task::kind_slug;

/// Clear the one account-referencing table that can carry no foreign key.
///
/// **`sync_task_state` has no key into `account(id)` and cannot have one**: `key` is polymorphic
/// (§21.3) — an account id for `account_repos`, a *project* id for `project_remote` and
/// `rename_probe`. So `delete_account` cannot lean on a cascade here, and it calls this by name.
///
/// **The task filter is not optional.** A bare `DELETE FROM sync_task_state WHERE key = ?1`
/// satisfies every enumeration over the census while deleting another task's rows for whichever
/// project happens to share that integer — a bar written past its defect on the one command whose
/// job is to delete. That is why `("sync_task_state", "key")` is not representable under
/// `ACCOUNT_REFERENCING_TABLES`, and why the census is a census (R69).
///
/// Returns the number of rows removed, so a caller or a test can see that it did something.
///
/// # Errors
/// Fails when SQLite refuses the delete.
pub fn delete_account_tasks(tx: &Transaction<'_>, account: AccountId) -> Result<usize, IndexError> {
    let removed = tx.execute(
        "DELETE FROM sync_task_state WHERE task = ?1 AND key = ?2",
        rusqlite::params![kind_slug(SyncTaskKind::AccountRepos), account.0],
    )?;
    Ok(removed)
}

/// The columns every read below selects, stated once so the two readers cannot drift.
const COLUMNS: &str =
    "task, key, state, cursor, fail_count, throttle_count, reason, at, not_before";

/// A stored row into its Rust shape.
///
/// The two enums are resolved by **searching their own slug functions**, which is the same
/// mapping the CHECK constraints were written from. A value a newer build wrote falls back rather
/// than panicking the loop that read it — the core supervises a window and a panic closes it.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncTaskStateRow> {
    let task: String = row.get(0)?;
    let state: String = row.get(2)?;
    Ok(SyncTaskStateRow {
        kind: SyncTaskKind::ALL
            .into_iter()
            .find(|k| kind_slug(*k) == task)
            .unwrap_or(SyncTaskKind::AccountRepos),
        key: row.get(1)?,
        state: SYNC_STATES
            .into_iter()
            .find(|s| state_slug(*s) == state)
            .unwrap_or(SyncTaskState::Queued),
        cursor: row.get(3)?,
        fail_count: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
        throttle_count: row.get::<_, i64>(5)?.try_into().unwrap_or(0),
        reason: row.get(6)?,
        at: row.get(7)?,
        not_before: row.get(8)?,
    })
}

/// Write one row, replacing the existing one for that `(task, key)`.
///
/// The two statements differ only in their conflict target, which has to name the partial index
/// that actually covers each case: `sync_task_keyed` for a keyed row, `sync_task_global` for a
/// process-wide one. Phase 2 declares no process-wide task; the arm exists so the one phase 3
/// adds needs no migration.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn put(tx: &Transaction<'_>, row: &SyncTaskStateRow) -> Result<(), IndexError> {
    let task = kind_slug(row.kind);
    let state = state_slug(row.state);
    let fail = i64::from(row.fail_count);
    let throttle = i64::from(row.throttle_count);
    match row.key {
        Some(key) => tx.execute(
            "INSERT INTO sync_task_state
               (task, key, state, cursor, fail_count, throttle_count, reason, at, not_before)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(task, key) WHERE key IS NOT NULL DO UPDATE SET
               state = excluded.state, cursor = excluded.cursor,
               fail_count = excluded.fail_count, throttle_count = excluded.throttle_count,
               reason = excluded.reason, at = excluded.at, not_before = excluded.not_before",
            rusqlite::params![
                task,
                key,
                state,
                row.cursor,
                fail,
                throttle,
                row.reason,
                row.at,
                row.not_before
            ],
        )?,
        None => tx.execute(
            "INSERT INTO sync_task_state
               (task, key, state, cursor, fail_count, throttle_count, reason, at, not_before)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(task) WHERE key IS NULL DO UPDATE SET
               state = excluded.state, cursor = excluded.cursor,
               fail_count = excluded.fail_count, throttle_count = excluded.throttle_count,
               reason = excluded.reason, at = excluded.at, not_before = excluded.not_before",
            rusqlite::params![
                task,
                state,
                row.cursor,
                fail,
                throttle,
                row.reason,
                row.at,
                row.not_before
            ],
        )?,
    };
    Ok(())
}

/// One row, or `None` when the task has never run.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn load(
    conn: &Connection,
    kind: SyncTaskKind,
    key: Option<i64>,
) -> Result<Option<SyncTaskStateRow>, IndexError> {
    let task = kind_slug(kind);
    let found = match key {
        Some(key) => conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM sync_task_state WHERE task = ?1 AND key = ?2"),
                rusqlite::params![task, key],
                read_row,
            )
            .ok(),
        None => conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM sync_task_state WHERE task = ?1 AND key IS NULL"),
                rusqlite::params![task],
                read_row,
            )
            .ok(),
    };
    Ok(found)
}

/// Every row, in `not_before` order so the runner's own ordering is the table's.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn load_all(conn: &Connection) -> Result<Vec<SyncTaskStateRow>, IndexError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM sync_task_state ORDER BY not_before, id"
    ))?;
    let rows = stmt
        .query_map([], read_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// **AC-P2-21-12.** Re-queue every `running` row left behind by a crash.
///
/// Every sync task is a **GET**, so it is idempotent — unlike the operations §2.2 forbids
/// replaying — and an interrupted one is re-runnable rather than a failure to report. It leaves
/// `fail_count` and `throttle_count` untouched: **an interruption is not a failure and must not
/// count as one**, or three crashes would defer a task that never failed once.
///
/// Returns the count, so a sweep that swept nothing cannot report success.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn requeue_running(tx: &Transaction<'_>, now: i64) -> Result<usize, IndexError> {
    let moved = tx.execute(
        "UPDATE sync_task_state
            SET state = 'queued', not_before = 0, at = ?1
          WHERE state = 'running'",
        [now],
    )?;
    Ok(moved)
}

/// Revive every `deferred` row, recording the cause.
///
/// **`blocked` is untouched, and that is the point.** It is left only through an account state
/// change or an explicit user action, never by a clock — a revived `blocked` row is a retry loop
/// against an unauthorised token.
///
/// The `fail_count` that deferred the row is cleared, because a revival cause is a statement that
/// the conditions which produced those failures have changed.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn reset_for(
    tx: &Transaction<'_>,
    cause: SyncResetCause,
    now: i64,
) -> Result<usize, IndexError> {
    let revived = tx.execute(
        "UPDATE sync_task_state
            SET state = 'queued', fail_count = 0, not_before = 0, reason = ?1, at = ?2
          WHERE state = 'deferred'",
        rusqlite::params![cause.slug(), now],
    )?;
    Ok(revived)
}

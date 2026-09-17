//! `sync_task_state` reads and writes, and the crash re-queue.
//!
//! Every function here takes a `&Transaction` or a `&Connection` **from its caller**, which is
//! how every writer in this codebase is shaped; the caller opens the transaction through
//! `crate::proto::txguard::TxGuard`.

use rusqlite::{Connection, Transaction};

use crate::index::IndexError;
use crate::protocol::{AccountId, ProjectId, SyncTaskKind};
use crate::sync::state::{state_slug, SyncResetCause, SyncTaskStateRow, SYNC_STATES};
use crate::sync::task::kind_slug;

/// Clear the one account-referencing table that can carry no foreign key.
///
/// **`sync_task_state` has no key into `account(id)` and cannot have one**: `key` is polymorphic
/// (§21.3) — an account id for two of the three kinds, a *project* id for `project_remote`. So
/// `delete_account` cannot lean on a cascade here, and it calls this by name.
///
/// **The task filter is not optional.** A bare `DELETE FROM sync_task_state WHERE key = ?1`
/// satisfies every enumeration over the census while deleting `project_remote`'s rows for
/// whichever project happens to share that integer — a bar written past its defect on the one
/// command whose job is to delete. That is why `("sync_task_state", "key")` is not representable
/// under `ACCOUNT_REFERENCING_TABLES`, and why the census is a census (R69).
///
/// **Two of the three kinds are account-keyed**: `account_repos`, and `rename_probe`, which this
/// plan keys by account because p2-22's repair is one bounded pass per account rather than one
/// lookup per project (see `crate::sync::task::SyncTask::RenameProbe`). `project_remote` is the
/// project-keyed one and is exactly what the filter protects.
///
/// Returns the number of rows removed, so a caller or a test can see that it did something.
///
/// # Errors
/// Fails when SQLite refuses the delete.
pub fn delete_account_tasks(tx: &Transaction<'_>, account: AccountId) -> Result<usize, IndexError> {
    let removed = tx.execute(
        "DELETE FROM sync_task_state WHERE task IN (?1, ?2) AND key = ?3",
        rusqlite::params![
            kind_slug(SyncTaskKind::AccountRepos),
            kind_slug(SyncTaskKind::RenameProbe),
            account.0
        ],
    )?;
    Ok(removed)
}

/// The columns every read below selects, stated once so the two readers cannot drift.
const COLUMNS: &str =
    "task, key, state, cursor, fail_count, throttle_count, reason, at, not_before";

/// A stored row into its Rust shape, or **nothing** for a row this build cannot read.
///
/// The two enums are resolved by **searching their own slug functions**, which is the same
/// mapping the CHECK constraints were written from. A slug neither list holds is a row a newer
/// build wrote, and it is **skipped, with a line on stderr** — never resolved to a member of the
/// enum. Guessing the state would turn a row a newer build wrote as `blocked` into a runnable
/// `queued` one, which is the retry against an unauthorised token §21.4 exists to prevent;
/// guessing the kind would run the wrong task against the key. Never a panic either: the core
/// supervises a window and a panic closes it.
///
/// **Unreachable today, and that is the reason it matters.** The column's CHECK is written from
/// these same slugs, so the only thing that can store an unknown one is a schema widening —
/// exactly the case in which a guess is wrong. It is untested for the same reason: the DDL
/// refuses to store the value a test would need to write.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<SyncTaskStateRow>> {
    let task: String = row.get(0)?;
    let state_text: String = row.get(2)?;
    let (Some(kind), Some(state)) = (
        SyncTaskKind::ALL
            .into_iter()
            .find(|k| kind_slug(*k) == task),
        SYNC_STATES
            .into_iter()
            .find(|s| state_slug(*s) == state_text),
    ) else {
        eprintln!(
            "sync: skipping a task row this build cannot read: task={task}, state={state_text}"
        );
        return Ok(None);
    };
    Ok(Some(SyncTaskStateRow {
        kind,
        key: row.get(1)?,
        state,
        cursor: row.get(3)?,
        fail_count: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
        throttle_count: row.get::<_, i64>(5)?.try_into().unwrap_or(0),
        reason: row.get(6)?,
        at: row.get(7)?,
        not_before: row.get(8)?,
    }))
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
            .ok()
            .flatten(),
        None => conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM sync_task_state WHERE task = ?1 AND key IS NULL"),
                rusqlite::params![task],
                read_row,
            )
            .ok()
            .flatten(),
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
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
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

/// How much of the table one revival covers.
///
/// **A cause is not a scope**, and conflating them makes a button do more than it says. §21.4's
/// `AppUpgraded` is a statement about the build and reaches everything; a user pressing TRY AGAIN
/// on one project is a statement about that project and the account that reads it, and reviving
/// every other account's deferred work on that press would be the same defect as a bare
/// `WHERE key = ?1` — an action whose blast radius is wider than its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncResetScope {
    /// Every deferred row, whatever it is keyed by.
    Everything,
    /// The two **account-keyed** tasks for one account.
    Account(AccountId),
    /// The one **project-keyed** task for one project.
    Project(ProjectId),
}

/// Revive the `deferred` rows in `scope`, recording the cause. Returns how many moved.
///
/// **`blocked` is untouched, and that is the point.** It is left only through an account state
/// change or an explicit user action, never by a clock — a revived `blocked` row is a retry loop
/// against an unauthorised token.
///
/// The `fail_count` that deferred the row is cleared, because a revival cause is a statement that
/// the conditions which produced those failures have changed.
///
/// **The task filter on the two narrow scopes is not optional**, for the reason
/// [`delete_account_tasks`] gives: `key` is polymorphic, so a bare `WHERE key = ?1` would revive
/// whichever `project_remote` row happens to share an account's integer.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn reset_for(
    tx: &Transaction<'_>,
    scope: SyncResetScope,
    cause: SyncResetCause,
    now: i64,
) -> Result<usize, IndexError> {
    const SET: &str = "UPDATE sync_task_state SET state = 'queued', fail_count = 0, not_before = 0,
                reason = ?1, at = ?2
          WHERE state = 'deferred'";
    let revived = match scope {
        SyncResetScope::Everything => tx.execute(SET, rusqlite::params![cause.slug(), now])?,
        SyncResetScope::Account(account) => tx.execute(
            &format!("{SET} AND task IN (?3, ?4) AND key = ?5"),
            rusqlite::params![
                cause.slug(),
                now,
                kind_slug(SyncTaskKind::AccountRepos),
                kind_slug(SyncTaskKind::RenameProbe),
                account.0
            ],
        )?,
        SyncResetScope::Project(project) => tx.execute(
            &format!("{SET} AND task = ?3 AND key = ?4"),
            rusqlite::params![
                cause.slug(),
                now,
                kind_slug(SyncTaskKind::ProjectRemote),
                project.0
            ],
        )?,
    };
    Ok(revived)
}

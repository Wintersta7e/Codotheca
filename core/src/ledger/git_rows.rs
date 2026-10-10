//! §38.7.4: J4's reconcile — a project's git rows made equal to the set its history implies.
//!
//! An insert alone never corrects a row, so a deferred project whose copy returns, an address
//! the user dropped while the project stays theirs, and a rewritten history would each keep rows
//! nothing implies any more. The reconcile deletes those, ratcheting the floor first, and inserts
//! what is implied and absent.

use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction};

use super::floor::ratchet_from_reading;
use super::recompute::delete_git_rows;
use crate::index::IndexError;
use crate::jobs::state::ResetCause;
use crate::protocol::ProjectId;

/// One row a history implies, ready to insert under `track = 'git'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRow {
    /// `commit_day` or `release`.
    pub kind: &'static str,
    /// The key the row is found by, and the one that makes a retried insert a no-op.
    pub dedupe_key: String,
    /// `ProjectSubject::to_key()` of the project the history belongs to.
    pub subject_key: String,
    /// The day's midnight, `day·86400`: a git row is a day, not a moment.
    pub ts: i64,
    /// The row's `meta` as the history builds it; the reconcile adds `silent` where it applies.
    pub meta: String,
}

/// What one reconcile changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    /// Implied rows that were absent and are now written.
    pub inserted: usize,
    /// Rows the history no longer implies, deleted after the ratchet.
    pub deleted: usize,
}

/// Make `project`'s git rows exactly `implied`, in the settle's transaction.
///
/// Every git row whose key `implied` lacks is deleted — the floor ratcheted first, from the
/// reading about to change — and every implied row not already there is inserted. Rows found on
/// both sides are left as they are, so a row keeps the `silent` it was written with.
///
/// # Errors
/// Returns [`IndexError`] when a read, the ratchet, a delete or an insert fails.
pub fn reconcile_git_rows(
    tx: &Transaction<'_>,
    project: ProjectId,
    implied: &[GitRow],
    now: i64,
) -> Result<ReconcileReport, IndexError> {
    let silent = j4_settle_is_silent(tx, project)?;
    let existing: BTreeSet<String> = tx
        .prepare("SELECT dedupe_key FROM xp_events WHERE project_id = ?1 AND track = 'git'")?
        .query_map([project.0], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let wanted: BTreeSet<&str> = implied.iter().map(|row| row.dedupe_key.as_str()).collect();

    let stale: Vec<String> = existing
        .iter()
        .filter(|key| !wanted.contains(key.as_str()))
        .cloned()
        .collect();
    let mut report = ReconcileReport::default();
    if !stale.is_empty() {
        ratchet_from_reading(tx, now)?;
        report.deleted = delete_git_rows(tx, project, Some(&stale))?;
    }

    // `silent` goes in as `Receipt::silent` writes it: present and true, or absent.
    let mut insert = tx.prepare(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, 'git',
                 CASE WHEN ?7 THEN json_set(?6, '$.silent', json('true')) ELSE ?6 END)
         ON CONFLICT(dedupe_key) DO NOTHING",
    )?;
    for row in implied
        .iter()
        .filter(|row| !existing.contains(&row.dedupe_key))
    {
        report.inserted += insert.execute(rusqlite::params![
            row.ts,
            project.0,
            row.subject_key,
            row.kind,
            row.dedupe_key,
            row.meta,
            silent,
        ])?;
    }
    Ok(report)
}

/// Whether the rows this J4 settle writes for `project` are told to nobody.
///
/// They are when its history has never been read (no `fts_commits` row), or when this read was
/// asked for by an upgrade or by the user rather than by a new commit. Read before the settle
/// writes, in its transaction.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn j4_settle_is_silent(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let silent: bool = conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM fts_commits WHERE project_id = ?1)
             OR EXISTS (SELECT 1 FROM project_job_state
                         WHERE project_id = ?1 AND job = 'j4'
                           AND reason IN (?2, ?3))",
        rusqlite::params![
            project.0,
            ResetCause::AppUpgraded.slug(),
            ResetCause::UserRequested.slug()
        ],
        |r| r.get(0),
    )?;
    Ok(silent)
}

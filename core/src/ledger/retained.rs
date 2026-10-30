//! §38.2's one insert of a per-project retained row.
//!
//! Every retained key is built from the project's subject, and two live projects can hold one
//! subject (§48.8.4). The writer has already looked for its own row by project; when its insert
//! meets a key another project's row holds, it learns that row instead, and acts on it — so a
//! twin's write is neither lost nor paid twice.

use rusqlite::Transaction;

use crate::clock::LocalInstant;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// A per-project retained row to insert.
pub(crate) struct RetainedRow<'a> {
    /// The row's instant and the offset in force at it.
    pub(crate) at: LocalInstant,
    /// The project the row pays.
    pub(crate) project: ProjectId,
    /// The project's subject key.
    pub(crate) subject_key: &'a str,
    /// One of the retained kinds.
    pub(crate) kind: &'static str,
    /// The key that makes the payout once-only.
    pub(crate) dedupe_key: &'a str,
    /// The row's receipt.
    pub(crate) meta: &'a str,
}

/// What [`insert_retained`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retained {
    /// The row was written, as this id.
    Inserted(i64),
    /// Another row holds the key — a twin's: this id.
    Held(i64),
}

/// Insert `row`, or answer the row already holding its key.
///
/// # Errors
/// Returns [`IndexError`] when SQLite refuses the insert or the read.
pub(crate) fn insert_retained(
    tx: &Transaction<'_>,
    row: &RetainedRow<'_>,
) -> Result<Retained, IndexError> {
    let inserted = tx.execute(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'session', ?7)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![
            row.at.unix,
            row.at.offset_min,
            row.project.0,
            row.subject_key,
            row.kind,
            row.dedupe_key,
            row.meta
        ],
    )?;
    if inserted > 0 {
        return Ok(Retained::Inserted(tx.last_insert_rowid()));
    }
    Ok(Retained::Held(tx.query_row(
        "SELECT id FROM xp_events WHERE dedupe_key = ?1",
        [row.dedupe_key],
        |r| r.get(0),
    )?))
}

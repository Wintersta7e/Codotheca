//! §38.8.2's completion latch: a check that passes while the project's gate is open pays once,
//! and nothing it does afterwards takes the row back.

use rusqlite::Transaction;

use super::marginal::{completion_check_due, RowDue};
use super::receipt::Receipt;
use super::retained::{insert_retained, Retained, RetainedRow};
use crate::clock::LocalInstant;
use crate::completion::evaluate::CheckRow;
use crate::completion::store::slug;
use crate::index::IndexError;
use crate::protocol::{CheckState, ProjectId};

/// Write a `completion_check` row for each of `rows` that passes and has none yet; answers how
/// many it wrote.
///
/// `rows` are the ten the caller just computed, never the stored ones: a reading that was not
/// computed latches nothing. Each row is read through the one reading the marginal price quotes
/// (`completion_check_due`): held while a restored record may yet match the project and bring
/// its own latches back (§48.8.4), shut by the latch's gate, and already written when found by
/// project and check — never by key. A check that stops passing deletes nothing.
///
/// # Errors
/// Returns [`IndexError`] when a read fails or SQLite refuses the insert.
pub fn latch_completion_checks(
    tx: &Transaction<'_>,
    project: ProjectId,
    rows: &[CheckRow],
    at: LocalInstant,
) -> Result<u32, IndexError> {
    let mut passing = rows
        .iter()
        .filter(|row| row.state == CheckState::Pass)
        .peekable();
    if passing.peek().is_none() {
        return Ok(0);
    }
    let project_name: String =
        tx.query_row("SELECT name FROM project WHERE id = ?1", [project.0], |r| {
            r.get(0)
        })?;
    let mut written = 0_u32;
    for row in passing {
        let RowDue::Write {
            subject_key,
            dedupe_key,
        } = completion_check_due(tx, project, row.key)?
        else {
            continue;
        };
        let check = slug(&row.key);
        let inserted = insert_retained(
            tx,
            &RetainedRow {
                at,
                project,
                subject_key: &subject_key,
                kind: "completion_check",
                dedupe_key: &dedupe_key,
                meta: &Receipt::new(&check, at, &project_name).to_meta(),
            },
        )?;
        // A twin's row holding the key is the subject's latch, already paid.
        if matches!(inserted, Retained::Inserted(_)) {
            written = written.saturating_add(1);
        }
    }
    Ok(written)
}

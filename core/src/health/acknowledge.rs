//! §30.5 — the write-once `acknowledged_at` stamp, and **the ordering it must be read in**.
//!
//! `acknowledged_at` has been declared since `0001_meta_and_projects.sql:102`, read by the
//! new-arrival predicate, projected onto every row and restored by the sidecar — and written by
//! **nothing**. The settled backlog-suppression gate reads it, so until this writer existed the
//! gate answered *suppressed* for every project, for ever, with every test around it green
//! (§27.7, A11.1).
//!
//! Written by `projects.get` and `projects.launch`, and by nothing else. Never by
//! `projects.peek`, `projects.list` or the quick switch — §10.5a: *reading a card is not
//! acknowledging it*, and stamping on Peek would let one arrow-key run down a column enrol the
//! whole backlog. `app/src/renderer/firstrun/newArrivals.ts`'s `acknowledges()` already
//! classifies all three and is the vocabulary this writer mirrors.
//!
//! **Never derived from `last_interaction_at`.** `.dev/spec/01-data-model.md` forbids it, and the
//! reason is that the column counts reflog activity performed *outside* the app — deriving
//! enrolment from it would let activity outside the app forge it.

use rusqlite::Transaction;

use super::enrolment::is_enrolled;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// Stamp the project as acknowledged, **once**.
///
/// `WHERE acknowledged_at IS NULL` is what makes a replay a no-op, which is in turn what keeps
/// `projects.get`'s `read` classification (`app/src/main/core/idempotence.ts:41`) true: the
/// command may be replayed and the stored time cannot move.
///
/// Returns whether *this* call was the one that stamped.
///
/// # Errors
/// Fails when the index refuses the write.
pub fn stamp_acknowledged(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<bool, IndexError> {
    let changed = tx.execute(
        "UPDATE project SET acknowledged_at = ?2
          WHERE id = ?1 AND acknowledged_at IS NULL",
        rusqlite::params![project.0, now],
    )?;
    Ok(changed > 0)
}

/// The stored stamp, read back through the same transaction.
///
/// # Errors
/// Fails when the index refuses the read.
pub fn read_acknowledged_at(
    tx: &Transaction<'_>,
    project: ProjectId,
) -> Result<Option<i64>, IndexError> {
    tx.query_row(
        "SELECT acknowledged_at FROM project WHERE id = ?1",
        [project.0],
        |r| r.get::<_, Option<i64>>(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(IndexError::from(other)),
    })
}

/// Stamp, **then** read the enrolment the reading is computed from — in that order, in one
/// transaction.
///
/// **The order is the whole point and it is why these two statements live in one function.**
/// Reversed, the first `projects.get` on a project serves a reading computed while it was still
/// unenrolled: the user sees `suppressed`, and has to open the page twice to see a reading that
/// was available the first time. A caller that reads the column itself before calling the stamp
/// has re-created exactly that, so callers take the answer from here.
///
/// # Errors
/// Fails when the index refuses the write or the read.
pub fn stamp_and_read_enrolment(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<bool, IndexError> {
    stamp_acknowledged(tx, project, now)?;
    Ok(is_enrolled(read_acknowledged_at(tx, project)?))
}

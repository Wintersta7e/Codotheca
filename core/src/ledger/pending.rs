//! §48.8.4's hold on retained writes.
//!
//! A project's pending sidecar records match at its creation or at the history settle that fixes
//! its lineage, so until that history concludes a retained writer writes nothing for it — a
//! restored row comes back as the old row, never as a second payment.

use rusqlite::Connection;

use super::level::CONCLUDED_SQL;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// Whether a retained writer must write nothing for `project` yet: `sidecar_pending` holds a
/// record, and the project's history has not concluded. Every retained writer asks first.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn awaiting_restore(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let awaiting: bool = conn.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM sidecar_pending)
                AND NOT EXISTS (SELECT 1 FROM project p WHERE p.id = ?1 AND {CONCLUDED_SQL})"
        ),
        [project.0],
        |r| r.get(0),
    )?;
    Ok(awaiting)
}

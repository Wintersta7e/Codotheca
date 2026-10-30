//! §48.8.4's hold on retained writes.
//!
//! A project's pending sidecar records match at its creation or at the history settle that fixes
//! its lineage, so until that history concludes a retained writer writes nothing for it — a
//! restored row comes back as the old row, never as a second payment. Only the project's own
//! records hold it: a record for a subject that has not come back holds nobody — except a project
//! with no lineage yet, whose settle may make such a lineage-keyed record its own.

use rusqlite::Connection;

use super::level::CONCLUDED_SQL;
use crate::index::pending::{has_own_pending, may_gain_pending};
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// Whether a retained writer must write nothing for `project` yet.
///
/// True while `sidecar_pending` holds a record the restore would give this project, or — while it
/// has no lineage — a lineage-keyed record nobody holds, and its history has not concluded. Every
/// retained writer asks first.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn awaiting_restore(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    if !has_own_pending(conn, project)? && !may_gain_pending(conn, project)? {
        return Ok(false);
    }
    let concluded: bool = conn.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM project p WHERE p.id = ?1 AND {CONCLUDED_SQL})"),
        [project.0],
        |r| r.get(0),
    )?;
    Ok(!concluded)
}

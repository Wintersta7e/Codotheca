//! §56.8: the one SQL visibility fragment.
//!
//! A project is **fully excluded** when it has at least one copy not removed and every copy not
//! removed is excluded (§56.6). It is **visible** — on the shelf, in a count, in a live
//! population — when it is not hidden and not fully excluded. Every core query that decides
//! visibility splices these texts; `core/tests/visibility_fragment.rs` fails on an `is_hidden`
//! read anywhere else that it has not classified.

use rusqlite::{Connection, OptionalExtension as _};

use crate::index::IndexError;
use crate::protocol::ProjectId;

/// The SQL that is true when the `project` row named `alias` is fully excluded.
///
/// A project with no copy left, or whose only excluded copies were removed, is not fully
/// excluded: nothing the user listed covers a copy it still has.
#[must_use]
pub fn project_fully_excluded(alias: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM location vx WHERE vx.project_id = {alias}.id \
                    AND vx.removed_at IS NULL) \
          AND NOT EXISTS (SELECT 1 FROM location vx WHERE vx.project_id = {alias}.id \
                    AND vx.removed_at IS NULL AND vx.excluded = 0))"
    )
}

/// The SQL that is true when the `project` row named `alias` is on the shelf: not hidden and
/// not fully excluded.
#[must_use]
pub fn project_visible(alias: &str) -> String {
    format!(
        "({alias}.is_hidden = 0 AND NOT {})",
        project_fully_excluded(alias)
    )
}

/// Whether the project is fully excluded; `false` for an id no row carries.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn is_fully_excluded(conn: &Connection, project: ProjectId) -> Result<bool, IndexError> {
    let answer: Option<bool> = conn
        .query_row(
            &format!(
                "SELECT {} FROM project p WHERE p.id = ?1",
                project_fully_excluded("p")
            ),
            [project.0],
            |r| r.get(0),
        )
        .optional()?;
    Ok(answer.unwrap_or(false))
}

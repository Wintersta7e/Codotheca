//! §1.10: `completion_lit` NULL is not `completion_lit` 0.
//!
//! The columns are NULL-able with no default, and this writer is forbidden to write 0 for
//! unknown. v1 assigned that rule to the view layer, which cannot enforce a write rule; here it
//! is enforced by the type — `NotComputed` is the only way to say unknown, and it writes NULL.
//!
//! Nothing in phase 1 calls `set_completion`. §7.7a: every project's completion is uncomputed,
//! which is the only case rather than an edge case.

use rusqlite::Connection;

use super::IndexError;
use crate::protocol::ProjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// No tick row is drawn. Not "0 of 0", and not "0 of 10".
    NotComputed,
    /// `lit` of `applicable` checks are lit. `applicable` is always at least 1.
    Computed { lit: u32, applicable: u32 },
}

pub fn set_completion(
    conn: &Connection,
    project: ProjectId,
    value: Completion,
) -> Result<(), IndexError> {
    let (lit, applicable) = match value {
        Completion::NotComputed => (None, None),
        Completion::Computed { applicable: 0, .. } => {
            // Zero applicable checks is not a computed zero; it is unknown wearing a number.
            return Err(IndexError::CompletionNotComputable);
        }
        Completion::Computed { lit, applicable } => {
            if lit > applicable {
                return Err(IndexError::CompletionNotComputable);
            }
            (Some(i64::from(lit)), Some(i64::from(applicable)))
        }
    };
    conn.execute(
        "UPDATE project SET completion_lit = ?2, completion_applicable = ?3 WHERE id = ?1",
        rusqlite::params![project.0, lit, applicable],
    )?;
    Ok(())
}

pub fn get_completion(conn: &Connection, project: ProjectId) -> Result<Completion, IndexError> {
    let (lit, applicable): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT completion_lit, completion_applicable FROM project WHERE id = ?1",
        [project.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match (lit, applicable) {
        (Some(l), Some(a)) if a > 0 => Ok(Completion::Computed {
            lit: u32::try_from(l).unwrap_or(0),
            applicable: u32::try_from(a).unwrap_or(1),
        }),
        _ => Ok(Completion::NotComputed),
    }
}

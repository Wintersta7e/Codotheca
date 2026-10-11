//! §38.8.2's completion latch: a check that passes while the project's gate is open pays once,
//! and nothing it does afterwards takes the row back.

use rusqlite::{OptionalExtension as _, Transaction};

use super::pending::awaiting_restore;
use super::price::{completion_gate, Gate};
use super::receipt::Receipt;
use crate::clock::LocalInstant;
use crate::completion::evaluate::CheckRow;
use crate::completion::store::slug;
use crate::index::subject::subject_for_project;
use crate::index::IndexError;
use crate::protocol::{CheckState, ProjectId};

/// Write a `completion_check` row for each of `rows` that passes and has none yet; answers how
/// many it wrote.
///
/// `rows` are the ten the caller just computed, never the stored ones: a reading that was not
/// computed latches nothing. A row already written is found by project and check — never by
/// key, since the subject moves from `path:` to `lineage:` at a first commit and a merge
/// reparents rows without rewriting their keys. A check that stops passing deletes nothing.
///
/// # Errors
/// Returns [`IndexError`] when a read fails or SQLite refuses the insert.
pub fn latch_completion_checks(
    tx: &Transaction<'_>,
    project: ProjectId,
    rows: &[CheckRow],
    at: LocalInstant,
) -> Result<u32, IndexError> {
    // A restored record may yet match this project and bring its own latches back (§48.8.4).
    if awaiting_restore(tx, project)? {
        return Ok(0);
    }
    if completion_gate(tx, project)? != Gate::Open {
        return Ok(0);
    }
    let Some(subject) = subject_for_project(tx, project)? else {
        return Ok(0);
    };
    let subject_key = subject.to_key();
    let mut written = 0_u32;
    for row in rows.iter().filter(|row| row.state == CheckState::Pass) {
        let check = slug(&row.key);
        let latched: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM xp_events
                  WHERE project_id = ?1 AND kind = 'completion_check'
                    AND json_extract(meta, '$.what') = ?2
                  LIMIT 1",
                rusqlite::params![project.0, check],
                |r| r.get(0),
            )
            .optional()?;
        if latched.is_some() {
            continue;
        }
        let project_name: String =
            tx.query_row("SELECT name FROM project WHERE id = ?1", [project.0], |r| {
                r.get(0)
            })?;
        let inserted = tx.execute(
            "INSERT INTO xp_events
                (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
             VALUES (?1, ?2, ?3, ?4, 'completion_check', ?5, 'session', ?6)
             ON CONFLICT(dedupe_key) DO NOTHING",
            rusqlite::params![
                at.unix,
                at.offset_min,
                project.0,
                subject_key,
                format!("completion_check:{subject_key}:{check}"),
                Receipt::new(&check, at, &project_name).to_meta()
            ],
        )?;
        if inserted > 0 {
            written = written.saturating_add(1);
        }
    }
    Ok(written)
}

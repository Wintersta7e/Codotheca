//! §38.8.5's `clean_archive`: an archived project's once-ever award, paid when every copy was last
//! observed with its commits on a remote and a clean tree, and its debt last observed swept and
//! empty.
//!
//! It is an observation, never a safety claim: it reads only what J1, J2 and the debt sweeps
//! cached, and the receipt carries the age of the oldest of them. Nothing here reads git.

use rusqlite::{OptionalExtension as _, Transaction};

use super::pending::awaiting_restore;
use super::receipt::Receipt;
use crate::clock::LocalInstant;
use crate::health::enrolment::is_enrolled;
use crate::index::subject::subject_for_project;
use crate::index::IndexError;
use crate::projects::current::IS_CURRENT_SQL;
use crate::protocol::ProjectId;
use crate::surfaces::settings::content_scan_enabled;

/// How §38.8.5 clause 3 reads a project's debt: what must hold of it, and when the observation
/// that says so was made, both as SQL over the project `p`.
struct DebtReading {
    holds: &'static str,
    observed_at: &'static str,
}

/// Without the content-scan grant: a sweep any settle completes.
const SWEPT: DebtReading = DebtReading {
    holds: "EXISTS (SELECT 1 FROM debt_sweep s
                     WHERE s.project_id = p.id AND s.outcome = 'complete')",
    observed_at: "(SELECT MIN(s.observed_at) FROM debt_sweep s
                    WHERE s.project_id = p.id AND s.outcome = 'complete')",
};

/// With the grant: the scan complete at the HEAD of every current copy. Read off the scan's own
/// completion, not its `todo_marker` sweep: J7 reads no blob of an archived copy, so that sweep
/// can stay `complete` after the HEAD moves on from the tree it read.
const SCANNED: DebtReading = DebtReading {
    holds: "EXISTS (SELECT 1 FROM project_content_scan c
                     WHERE c.project_id = p.id AND c.complete_head_oid IS NOT NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM location l
                            WHERE l.project_id = p.id AND l.removed_at IS NULL
                              AND l.head_oid IS NOT c.complete_head_oid))",
    observed_at: "(SELECT c.completed_at FROM project_content_scan c WHERE c.project_id = p.id)",
};

/// The `clean_archive` row a settle wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanArchivePaid {
    /// The row's `xp_events` id.
    pub row_id: i64,
    /// The project it was paid to.
    pub project: ProjectId,
}

/// Pay `project`'s `clean_archive` if it qualifies as last observed; `None` when it does not, or
/// when its award exists.
///
/// In order: no `clean_archive` row for the project, found by project and never by key; no
/// restored record still waiting to match (§48.8.4); a subject to key the row on; then the
/// project archived, not Reference, not merged away and current, with every current copy last
/// observed with `archive_reached = 1`, a clean tree, no untracked file and no stash, no scored
/// debt item open or unverified, and the project's debt read. A copy never observed — any of
/// those columns NULL — withholds it. The receipt's `observed_at` is the oldest of the
/// observations it read. Its caller is the scheduler's settle, on every settle.
///
/// The debt is read by the content scan only where the user granted it: the scan turned on and
/// the project enrolled. There the scan must be complete at the HEAD every current copy was last
/// observed at; anywhere else one `complete` sweep of any source suffices, so no award depends on
/// granting consent.
///
/// # Errors
/// Returns [`IndexError`] when a read fails or SQLite refuses the insert.
pub fn evaluate_clean_archive(
    tx: &Transaction<'_>,
    project: ProjectId,
    at: LocalInstant,
) -> Result<Option<CleanArchivePaid>, IndexError> {
    let paid: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM xp_events WHERE project_id = ?1 AND kind = 'clean_archive')",
        [project.0],
        |r| r.get(0),
    )?;
    if paid || awaiting_restore(tx, project)? {
        return Ok(None);
    }
    let Some(subject) = subject_for_project(tx, project)? else {
        return Ok(None);
    };
    let acknowledged_at: Option<i64> = tx
        .query_row(
            "SELECT acknowledged_at FROM project WHERE id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let reading = if content_scan_enabled(tx)? && is_enrolled(acknowledged_at) {
        SCANNED
    } else {
        SWEPT
    };
    let qualified: Option<(String, i64)> = tx
        .query_row(
            &format!(
                "SELECT p.name,
                        MIN((SELECT MIN(MIN(l.refstate_observed_at, l.worktree_observed_at))
                               FROM location l
                              WHERE l.project_id = p.id AND l.removed_at IS NULL),
                            {observed_at})
                   FROM project p
                  WHERE p.id = ?1 AND p.is_archived = 1 AND p.is_reference = 0
                    AND p.merged_into IS NULL AND {IS_CURRENT_SQL}
                    AND NOT EXISTS (
                        SELECT 1 FROM location l
                         WHERE l.project_id = p.id AND l.removed_at IS NULL
                           AND NOT (l.archive_reached IS 1 AND l.is_dirty IS 0
                                    AND l.untracked_count IS 0 AND l.stash_count IS 0
                                    AND l.refstate_observed_at IS NOT NULL
                                    AND l.worktree_observed_at IS NOT NULL))
                    AND NOT EXISTS (
                        SELECT 1 FROM debt_item d
                         WHERE d.project_id = p.id AND d.scoring = 'scored'
                           AND d.state IN ('open', 'unverified'))
                    AND {holds}",
                observed_at = reading.observed_at,
                holds = reading.holds,
            ),
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((project_name, observed_at)) = qualified else {
        return Ok(None);
    };
    let subject_key = subject.to_key();
    let inserted = tx.execute(
        "INSERT INTO xp_events
            (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
         VALUES (?1, ?2, ?3, ?4, 'clean_archive', ?5, 'session', ?6)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![
            at.unix,
            at.offset_min,
            project.0,
            subject_key,
            format!("clean_archive:{subject_key}"),
            Receipt::new("clean_archive", at, &project_name)
                .extra("observed_at", observed_at)
                .to_meta()
        ],
    )?;
    Ok((inserted > 0).then(|| CleanArchivePaid {
        row_id: tx.last_insert_rowid(),
        project,
    }))
}

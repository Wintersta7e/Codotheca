//! §38.8.3: the `focus` row, written in the transaction of every segment close.
//!
//! It stores §39.3's marginal measure — the seconds of each local date the closing segment adds
//! to the union of the segments closed before it — so a date's rows add up to that date's Focus
//! over its closed segments, whatever order they closed in. A launch-only segment and a
//! Reference project's add nothing by the measure's construction, and so write nothing.
//! Forward-only: a segment closed before this writer existed pays nothing.

use rusqlite::{OptionalExtension as _, Transaction};

use super::pending::awaiting_restore;
use super::receipt::Receipt;
use super::retained::{insert_retained, Retained, RetainedRow};
use crate::clock::LocalZone;
use crate::index::subject::subject_for_project;
use crate::protocol::ProjectId;
use crate::session::measure::marginal_seconds;
use crate::session::SessionError;

/// Credit the segment that just closed: one `focus` row per local date it adds to, in the
/// caller's transaction. Returns how many rows it wrote or moved.
///
/// Nothing while a restored record may still match the project (§48.8.4), nor for a project
/// with no subject to key a row on. A date's row is found by project and date, never by key
/// (§38.2) — the lower id after a merge — and only its `meta.seconds` moves. A new one takes
/// the date's first credited instant, and the offset in force at it; when a twin's row already
/// holds its key, the seconds move on that row instead.
///
/// # Errors
/// `SessionError::Sqlite` or `SessionError::Index` when a read or a write fails.
pub fn credit_segment_close(
    tx: &Transaction<'_>,
    zone: &dyn LocalZone,
    segment_id: i64,
) -> Result<u32, SessionError> {
    let owner: Option<(i64, String)> = tx
        .query_row(
            "SELECT p.id, p.name FROM session_segment g
               JOIN session s ON s.id = g.session_id
               JOIN project p ON p.id = s.project_id
              WHERE g.id = ?1",
            [segment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((project, project_name)) = owner else {
        return Ok(0);
    };
    let project = ProjectId(project);
    if awaiting_restore(tx, project)? {
        return Ok(0);
    }
    let Some(subject) = subject_for_project(tx, project)? else {
        return Ok(0);
    };
    let subject_key = subject.to_key();
    let mut written = 0_u32;
    for share in marginal_seconds(tx, zone, segment_id)? {
        let first = zone.at(share.first_instant);
        let date = first.date();
        let row: Option<i64> = tx
            .query_row(
                "SELECT id FROM xp_events
                  WHERE project_id = ?1 AND kind = 'focus'
                    AND COALESCE(json_extract(meta, '$.when'),
                                 date(ts + COALESCE(tz_offset_min, 0) * 60, 'unixepoch')) = ?2
                  ORDER BY id LIMIT 1",
                rusqlite::params![project.0, date],
                |r| r.get(0),
            )
            .optional()?;
        let existing = match row {
            Some(id) => Some(id),
            None => match insert_retained(
                tx,
                &RetainedRow {
                    at: first,
                    project,
                    subject_key: &subject_key,
                    kind: "focus",
                    dedupe_key: &format!("focus:{subject_key}:{date}"),
                    meta: &Receipt::new("focus", first, &project_name)
                        .extra("seconds", share.seconds)
                        .to_meta(),
                },
            )? {
                Retained::Inserted(_) => None,
                // A twin's row holds the date: the seconds are the subject's, so they land there.
                Retained::Held(id) => Some(id),
            },
        };
        let changed = match existing {
            Some(id) => tx.execute(
                "UPDATE xp_events
                    SET meta = json_set(COALESCE(meta, '{}'), '$.seconds',
                                        COALESCE(json_extract(meta, '$.seconds'), 0) + ?2)
                  WHERE id = ?1",
                rusqlite::params![id, share.seconds],
            )?,
            None => 1,
        };
        written = written.saturating_add(u32::try_from(changed).unwrap_or(u32::MAX));
    }
    Ok(written)
}

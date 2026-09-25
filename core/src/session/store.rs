//! The only writer of `session` and `session_segment`.
//!
//! Nothing here reads a git-derived table. Playtime is one ledger and git-derived facts are
//! another; a join between them here is what would make the two summable (§9, §8.5.5), and
//! `core/tests/session_credit.rs` asserts over this file's source that none appears.
//!
//! **`credited_seconds` is recomputed, never accumulated.** Every read of a session's credit is
//! a `SUM` over its segments, so a lost tick, a double close or a crash cannot leave an
//! arithmetic residue behind. `session.credited_seconds` is a materialised copy of that sum,
//! written on close for the aggregate reads in §8.5.5.
//!
//! **The watermark on an open segment lives in `credited_seconds`, not in `ended_at`.** §1.6
//! puts `CHECK ((ended_at IS NULL) = (closed_by IS NULL))` on `session_segment`, so a row that
//! is still open cannot carry an end. Nothing is lost: the last observed activity is exactly
//! `started_at + credited_seconds`, which is what orphan recovery closes the row at.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::protocol::{CloseReason, LocationId, ProjectId, SessionId, SessionRef, TargetId};
use crate::session::segment::ClosedSegment;
use crate::session::{close_reason_from_str, close_reason_str, closed_by_str, SessionError};

/// What a new `session` row is opened with; its end, reason and credit start empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOpen {
    /// The project that was launched.
    pub project_id: ProjectId,
    /// The location the launch ran from; `None` only where §1.6 permits a NULL column.
    pub location_id: Option<LocationId>,
    /// The launch target that was run; `None` only where §1.6 permits a NULL column.
    pub target_id: Option<TargetId>,
    /// Wall-clock unix seconds at which the session opened.
    pub started_at: i64,
}

/// What [`close_session`] wrote for a session it closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionClosed {
    /// The session's final credit in seconds: the sum over its segments at close.
    pub credited_seconds: i64,
    /// What was actually stored, which is never earlier than the session's own `started_at`.
    pub ended_at: i64,
    /// `recompute` moved `condition_signal`; the command layer publishes the event.
    pub condition_changed: bool,
}

/// One session with no `ended_at`, as [`open_sessions`] reads it for orphan recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenSessionRow {
    /// The session left open.
    pub session_id: SessionId,
    /// The project the session was launched for.
    pub project_id: ProjectId,
    /// The location it was launched from; `None` where the column is NULL.
    pub location_id: Option<LocationId>,
    /// Wall-clock unix seconds at which the session opened.
    pub started_at: i64,
    /// The row id of its segment with no `closed_by`, or `None` when every segment is closed.
    pub open_segment_id: Option<i64>,
    /// The watermark on that open segment: the last activity folded into it, or the moment it
    /// opened when none has been. `None` only when the session has no open segment at all.
    pub last_activity_at: Option<i64>,
}

/// Insert a new open `session` row with no end and zero credit, returning its id.
///
/// # Errors
///
/// `SessionError::Sqlite` when the insert fails, e.g. a foreign key naming no project.
pub fn open_session(tx: &Transaction<'_>, open: &SessionOpen) -> Result<SessionId, SessionError> {
    tx.execute(
        "INSERT INTO session
           (project_id, location_id, target_id, started_at, ended_at, credited_seconds,
            close_reason)
         VALUES (?1, ?2, ?3, ?4, NULL, 0, NULL)",
        params![
            open.project_id.0,
            open.location_id.map(|l| l.0),
            open.target_id.map(|t| t.0),
            open.started_at
        ],
    )?;
    Ok(SessionId(tx.last_insert_rowid()))
}

/// Insert a new open `session_segment` row under `session`, returning the segment's row id.
///
/// # Errors
///
/// `SessionError::Sqlite` when the insert fails, e.g. `session` names no row.
pub fn open_segment(
    tx: &Transaction<'_>,
    session: SessionId,
    started_at: i64,
) -> Result<i64, SessionError> {
    tx.execute(
        "INSERT INTO session_segment
           (session_id, started_at, ended_at, credited_seconds, closed_by)
         VALUES (?1, ?2, NULL, 0, NULL)",
        params![session.0, started_at],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Keep the open row current, so a crash credits up to the last observed activity and invents
/// nothing after it.
///
/// `max(…, 0)` because a wall clock can move backwards, and the guard on `closed_by IS NULL` is
/// what stops a late activity batch reopening a segment already closed.
///
/// # Errors
///
/// `SessionError::Sqlite` when the update fails.
pub fn mark_activity(tx: &Transaction<'_>, segment_id: i64, at: i64) -> Result<(), SessionError> {
    tx.execute(
        "UPDATE session_segment
            SET credited_seconds = max(max(?2 - started_at, 0), credited_seconds)
          WHERE id = ?1 AND closed_by IS NULL",
        params![segment_id, at],
    )?;
    Ok(())
}

/// Stamp an open segment's end, credit and `closed_by`; a segment already closed is left as is.
///
/// # Errors
///
/// `SessionError::Sqlite` when the update fails, e.g. a negative credit breaks §1.6's CHECK.
pub fn close_segment(
    tx: &Transaction<'_>,
    segment_id: i64,
    seg: &ClosedSegment,
) -> Result<(), SessionError> {
    tx.execute(
        "UPDATE session_segment
            SET ended_at = ?2, credited_seconds = ?3, closed_by = ?4
          WHERE id = ?1 AND closed_by IS NULL",
        params![
            segment_id,
            seg.ended_at,
            seg.credited_seconds,
            closed_by_str(seg.closed_by)
        ],
    )?;
    Ok(())
}

/// `credited_seconds` is **always** the sum of segment credits (§9) — recomputed, never
/// accumulated, so no lost tick can leave a residue.
///
/// # Errors
///
/// `SessionError::NoSuchSession` when no `session` row has that id, `SessionError::Sqlite` when a
/// read or the update fails, and `SessionError::Index` when recomputing the project's derived
/// state fails.
pub fn close_session(
    tx: &Transaction<'_>,
    session: SessionId,
    ended_at: i64,
    reason: CloseReason,
    now: i64,
) -> Result<SessionClosed, SessionError> {
    let (project, started_at): (i64, i64) = tx
        .query_row(
            "SELECT project_id, started_at FROM session WHERE id = ?1",
            [session.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(SessionError::NoSuchSession(session.0))?;
    let credited = sum_segments(tx, session)?;
    let stored_end = ended_at.max(started_at);
    tx.execute(
        "UPDATE session
            SET ended_at = ?2, close_reason = ?3, credited_seconds = ?4
          WHERE id = ?1 AND ended_at IS NULL",
        params![session.0, stored_end, close_reason_str(reason), credited],
    )?;
    // §5.1's `last session end` term. Every close path would otherwise have to remember.
    let recomputed = crate::derive::persist::recompute(tx, ProjectId(project), now)?;
    Ok(SessionClosed {
        credited_seconds: credited,
        ended_at: stored_end,
        condition_changed: recomputed.changed_condition,
    })
}

/// The one place the credit of a session is computed. `&Transaction` coerces to `&Connection`,
/// so the writer and the readers share this SQL rather than restating it.
fn sum_segments(conn: &Connection, session: SessionId) -> Result<i64, SessionError> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(credited_seconds), 0) FROM session_segment WHERE session_id = ?1",
        [session.0],
        |r| r.get(0),
    )?)
}

/// The live credit of one session: the sum over its segments, closed and open alike.
///
/// # Errors
///
/// `SessionError::Sqlite` when the sum cannot be read.
pub fn credited_seconds(conn: &Connection, session: SessionId) -> Result<i64, SessionError> {
    sum_segments(conn, session)
}

/// `(project_id, location_id, target_id, started_at, ended_at, close_reason)` as §1.6 stores it.
/// Named because a six-tuple raises `clippy::type_complexity`.
type StoredSession = (
    i64,
    Option<i64>,
    Option<i64>,
    i64,
    Option<i64>,
    Option<String>,
);

/// The generated payload for `session/started` and `session/ended`.
///
/// `credited_seconds` comes from the segments rather than from `session.credited_seconds`, so a
/// reference to a live session can never report a stale total.
///
/// # Errors
///
/// `SessionError::NoSuchSession` when no row has that id; `SessionError::BadColumn` when
/// `location_id` or `target_id` is NULL or `close_reason` is outside §1.6's set;
/// `SessionError::Sqlite` when a read fails.
pub fn session_ref(conn: &Connection, session: SessionId) -> Result<SessionRef, SessionError> {
    // `project_id` and `started_at` are NOT NULL in §1.6; the other three are not.
    let row: StoredSession = conn
        .query_row(
            "SELECT project_id, location_id, target_id, started_at, ended_at, close_reason
               FROM session WHERE id = ?1",
            [session.0],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or(SessionError::NoSuchSession(session.0))?;
    let (project_id, location_id, target_id, started_at, ended_at, close_reason) = row;
    // `SessionRef` requires both ids (`protocol/schema/protocol.json`), while §1.6 permits NULL.
    // A row without them cannot be described over the protocol, so it is an error rather than an
    // invented sentinel; every production launch supplies both.
    let location_id = LocationId(location_id.ok_or(SessionError::BadColumn {
        column: "location_id",
        value: "NULL".to_owned(),
    })?);
    let target_id = TargetId(target_id.ok_or(SessionError::BadColumn {
        column: "target_id",
        value: "NULL".to_owned(),
    })?);
    let close_reason = match close_reason {
        None => None,
        Some(text) => Some(close_reason_from_str(&text).ok_or(SessionError::BadColumn {
            column: "close_reason",
            value: text,
        })?),
    };
    Ok(SessionRef {
        id: session,
        project_id: ProjectId(project_id),
        location_id,
        target_id,
        started_at,
        ended_at,
        credited_seconds: sum_segments(conn, session)?,
        close_reason,
    })
}

/// Every session left open, with its open segment and that segment's watermark.
///
/// # Errors
///
/// `SessionError::Sqlite` when the query cannot be prepared or a row cannot be read.
pub fn open_sessions(conn: &Connection) -> Result<Vec<OpenSessionRow>, SessionError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.project_id, s.location_id, s.started_at, g.id, g.started_at,
                g.credited_seconds
           FROM session s
           LEFT JOIN session_segment g
             ON g.session_id = s.id AND g.closed_by IS NULL
          WHERE s.ended_at IS NULL
          ORDER BY s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        let open_segment_id: Option<i64> = r.get(4)?;
        let segment_started_at: Option<i64> = r.get(5)?;
        let segment_credited: Option<i64> = r.get(6)?;
        Ok(OpenSessionRow {
            session_id: SessionId(r.get(0)?),
            project_id: ProjectId(r.get(1)?),
            location_id: r.get::<_, Option<i64>>(2)?.map(LocationId),
            started_at: r.get(3)?,
            open_segment_id,
            last_activity_at: match (segment_started_at, segment_credited) {
                (Some(started), Some(credited)) => Some(started.saturating_add(credited)),
                _ => None,
            },
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Launched-session time only (§9, §8.5.5's `PLAYTIME`). One table, one ledger: nothing here
/// joins a git-derived table, and there is no function anywhere that returns the two summed.
///
/// # Errors
///
/// `SessionError::Sqlite` when the sum cannot be read.
pub fn playtime_seconds(conn: &Connection, project: ProjectId) -> Result<i64, SessionError> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(credited_seconds), 0) FROM session WHERE project_id = ?1",
        [project.0],
        |r| r.get(0),
    )?)
}

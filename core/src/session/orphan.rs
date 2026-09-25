//! §9 and §11.2: close sessions orphaned by a crash or a power loss.
//!
//! Runs in the core startup lane — git floor check, close orphaned sessions, join — **before any
//! session can be opened in this process**, which is why every `ended_at IS NULL` row it finds is
//! from a previous one. That ordering is the whole safety argument, and it is why this is a
//! startup task and not a periodic sweep.
//!
//! **It invents nothing.** A segment still open is closed at the watermark `store::mark_activity`
//! last wrote — `started_at + credited_seconds` — and one that never saw activity is closed at
//! its own `started_at` for zero credit. The session is stamped at the **latest segment end**,
//! never at `now`: a session orphaned three days ago must not move `last_interaction_at` to
//! today, which would light a tile the user has not touched since.

use crate::index::Index;
use crate::protocol::{CloseReason, ProjectId};
use crate::session::{close_reason_str, closed_by_str, ClosedBy, SessionError};

/// What one startup recovery pass closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrphanReport {
    /// Sessions a previous process left open, now closed as `orphaned`.
    pub sessions_closed: u64,
    /// Segments left open under them, now closed at their watermark as `crash`.
    pub segments_closed: u64,
    /// The total credit of the closed sessions in seconds, all of it earned before the crash.
    pub credited_seconds: i64,
}

/// Close every session a previous process left open, in one transaction, at its watermark.
///
/// `now` is passed only to the derived-state recompute each close triggers; no row is stamped
/// with it.
///
/// # Errors
///
/// `SessionError::Sqlite` when the transaction or any statement in it fails, and
/// `SessionError::Index` when recomputing a project's derived state fails. Either rolls the whole
/// pass back.
pub fn close_orphans(index: &mut Index, now: i64) -> Result<OrphanReport, SessionError> {
    let _guard = crate::proto::txguard::TxGuard::enter();
    let tx = index.conn_mut().transaction()?;
    let mut report = OrphanReport::default();

    let open: Vec<(i64, i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT id, project_id, started_at FROM session WHERE ended_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    for (session_id, project_id, started_at) in open {
        // The watermark already lives in `credited_seconds`; §1.6's
        // `CHECK ((ended_at IS NULL) = (closed_by IS NULL))` is why it could not live in
        // `ended_at` while the row was open. Closing the row recovers the end exactly.
        report.segments_closed += u64::try_from(tx.execute(
            "UPDATE session_segment
                SET ended_at = started_at + credited_seconds,
                    closed_by = ?2
              WHERE session_id = ?1 AND closed_by IS NULL",
            rusqlite::params![session_id, closed_by_str(ClosedBy::Crash)],
        )?)
        // A row count cannot be negative, and `expect_used` is denied, so there is no failure
        // to report here.
        .unwrap_or(0);

        let (credited, latest_end): (i64, Option<i64>) = tx.query_row(
            "SELECT COALESCE(SUM(credited_seconds), 0), MAX(ended_at)
               FROM session_segment WHERE session_id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        // Never `now`.
        let ended_at = latest_end.unwrap_or(started_at).max(started_at);
        tx.execute(
            "UPDATE session SET ended_at = ?2, close_reason = ?3, credited_seconds = ?4
              WHERE id = ?1 AND ended_at IS NULL",
            rusqlite::params![
                session_id,
                ended_at,
                close_reason_str(CloseReason::Orphaned),
                credited
            ],
        )?;
        // §5.1's `last session end` term moved, so the derived clocks and §5.4's band follow.
        crate::derive::persist::recompute(&tx, ProjectId(project_id), now)?;
        report.sessions_closed += 1;
        report.credited_seconds = report.credited_seconds.saturating_add(credited);
    }

    tx.commit()?;
    Ok(report)
}

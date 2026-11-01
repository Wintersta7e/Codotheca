//! §39.6's witness (g): the local dates of the git ledger's commit-days. Reads `xp_events` and
//! `project` only.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::active::DaySpan;
use crate::index::IndexError;

const DAY_SECS: i64 = 86_400;

/// The days of `span` holding a `commit_day` row of a project that is not shallow, not a
/// Reference and not merged away.
///
/// Shallow clones are out of commit-day maths although J4 writes their rows, so the filter is
/// here. A removed project's rows still count: its history happened. A git row is stamped at its
/// day's midnight, so its day is its stamp's.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn git_days(conn: &Connection, span: DaySpan) -> Result<BTreeSet<i64>, IndexError> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT x.ts FROM xp_events x JOIN project p ON p.id = x.project_id
          WHERE x.kind = 'commit_day'
            AND p.is_shallow = 0 AND p.is_reference = 0 AND p.merged_into IS NULL
            AND x.ts >= ?1 AND x.ts < ?2",
    )?;
    let from = span.first.saturating_mul(DAY_SECS);
    let to = span.last.saturating_add(1).saturating_mul(DAY_SECS);
    let days = stmt
        .query_map([from, to], |r| r.get::<_, i64>(0))?
        .map(|ts| ts.map(|ts| ts.div_euclid(DAY_SECS)))
        .collect::<Result<_, _>>()?;
    Ok(days)
}

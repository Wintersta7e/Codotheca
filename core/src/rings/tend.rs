//! §39.6's witness (t): the local dates of `T`, the `debt_day` rows that earn (§39.5). Reads
//! `xp_events` and `project` only.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::active::DaySpan;
use crate::index::IndexError;
use crate::ledger::xp::debt_day_priced;

const DAY_SECS: i64 = 86_400;

/// The days of `span` holding a `T` row: a `debt_day` of a project that is not a Reference and not
/// merged away, which §38 prices above zero.
///
/// Each is dated in its own stored offset, as the ledger dates it. `T` carries none of the Tend
/// domain's suppression or removal clauses: a Seal or a removal later that week withdraws no day
/// the user tended.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub fn tend_days(conn: &Connection, span: DaySpan) -> Result<BTreeSet<i64>, IndexError> {
    let mut stmt = conn.prepare_cached(
        "SELECT x.ts, x.tz_offset_min, x.meta FROM xp_events x JOIN project p ON p.id = x.project_id
          WHERE x.kind = 'debt_day' AND p.is_reference = 0 AND p.merged_into IS NULL
            AND x.ts >= ?1 AND x.ts < ?2",
    )?;
    // Every offset is under a day, so a day either side of the span holds every candidate.
    let from = span.first.saturating_sub(1).saturating_mul(DAY_SECS);
    let to = span.last.saturating_add(2).saturating_mul(DAY_SECS);
    let mut days = BTreeSet::new();
    for row in stmt.query_map([from, to], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (ts, offset_min, meta) = row?;
        let day = ts
            .saturating_add(offset_min.unwrap_or(0).saturating_mul(60))
            .div_euclid(DAY_SECS);
        if (span.first..=span.last).contains(&day) && meta.as_deref().is_none_or(debt_day_priced) {
            days.insert(day);
        }
    }
    Ok(days)
}

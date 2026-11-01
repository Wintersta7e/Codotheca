//! §39.6's witness (s): the local dates whose Focus closed its ring. Reads the session ledger
//! only, through §39.3's one measure.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::active::DaySpan;
use crate::clock::LocalZone;
use crate::session::measure::focus_seconds_by_day;
use crate::session::SessionError;

/// The days of `span` whose `Focus(d)` reaches `target_secs` — the closed ring, never any credit.
///
/// One read of the span's counted segments, one union per day (§39.3); nothing after `now`.
///
/// # Errors
/// `SessionError::Sqlite` when the segments cannot be read.
pub fn focus_days(
    conn: &Connection,
    span: DaySpan,
    target_secs: i64,
    zone: &dyn LocalZone,
    now: i64,
) -> Result<BTreeSet<i64>, SessionError> {
    Ok(
        focus_seconds_by_day(conn, zone, span.first, span.last, now)?
            .into_iter()
            .filter(|&(_, seconds)| seconds >= target_secs)
            .map(|(day, _)| day)
            .collect(),
    )
}

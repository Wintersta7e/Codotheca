//! §39.3's Focus ring, and §39.6's witness (s): the local dates whose Focus closed its ring.
//! Reads the session ledger and the target only, through §39.3's one measure.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::active::DaySpan;
use super::targets::focus_target_minutes;
use crate::clock::LocalZone;
use crate::index::IndexError;
use crate::protocol::{FocusRing, RingStatus};
use crate::session::measure::{focus_seconds, focus_seconds_by_day};
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

/// Today's Focus ring: `Focus(today)` against the target in force (§39.3).
///
/// Open segments count to their watermark, nothing after `now`. The credit crosses the wire
/// floored to a whole minute (§39.11), so a live session moves it at most once a minute; closure
/// compares the unfloored seconds. Never `NotComputed`: the session ledger is complete by
/// construction (§39.1).
///
/// # Errors
/// Fails when the segments or the target cannot be read.
pub fn focus_ring(
    conn: &Connection,
    today: i64,
    zone: &dyn LocalZone,
    now: i64,
) -> Result<FocusRing, IndexError> {
    let seconds = focus_seconds(conn, zone, today, now).map_err(|error| match error {
        SessionError::Sqlite(e) => IndexError::Sqlite(e),
        SessionError::Index(e) => e,
        other => IndexError::Corrupt {
            detail: other.to_string(),
        },
    })?;
    let target_seconds = i64::from(focus_target_minutes(conn)?) * 60;
    Ok(FocusRing {
        credited_seconds: seconds - seconds.rem_euclid(60),
        target_seconds,
        status: if seconds >= target_seconds {
            RingStatus::Closed
        } else {
            RingStatus::Open
        },
    })
}

//! §39.4's Ship ring: the user's own push record, as J1 persisted it on `location`.
//!
//! Reads `location`'s push columns and `project` only. A push is an act, never a state: nothing
//! here says bytes exist remotely, and §45's analyser never reads these columns.

use rusqlite::Connection;

use crate::clock::{local_date, local_day, week_start};
use crate::index::IndexError;
use crate::protocol::{RingStatus, ShipRing};

/// This week's Ship ring, `today` a local day number.
///
/// The domain is every unremoved location of a non-Reference project, shallow copies included.
/// `R` is the domain less `no_remote` — a NULL record stays in `R`, because unknown is not *no
/// remote*. A push is dated in its reflog line's own offset. `observable` counts the `read`
/// locations of `R` and `unobservable` the rest, under every status.
///
/// # Errors
/// Fails when the locations cannot be read.
pub fn ship_ring(conn: &Connection, today: i64) -> Result<ShipRing, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT l.push_record, l.last_push_at, l.last_push_tz_offset_min
           FROM location l JOIN project p ON p.id = l.project_id
          WHERE l.removed_at IS NULL AND p.is_reference = 0
            AND l.push_record IS NOT 'no_remote'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i32>>(2)?,
        ))
    })?;
    let first = week_start(today);
    let (mut observable, mut unobservable) = (0_u32, 0_u32);
    let mut pushed_on: Option<i64> = None;
    for row in rows {
        let (record, at, offset) = row?;
        if record.as_deref() != Some("read") {
            unobservable += 1;
            continue;
        }
        observable += 1;
        if let (Some(at), Some(offset)) = (at, offset) {
            let day = local_day(at, offset);
            if (first..=first + 6).contains(&day) && pushed_on.is_none_or(|newest| day > newest) {
                pushed_on = Some(day);
            }
        }
    }
    let status = if observable + unobservable == 0 {
        RingStatus::NotApplicable
    } else if pushed_on.is_some() {
        RingStatus::Closed
    } else if observable == 0 {
        RingStatus::NotComputed
    } else {
        RingStatus::Open
    };
    Ok(ShipRing {
        status,
        pushed_on: pushed_on.map(local_date),
        observable,
        unobservable,
    })
}

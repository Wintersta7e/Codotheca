//! §39: the three rings — Focus, Ship and Tend — and the weekly rhythm.
//!
//! This module owns §39.12's constants; the renderer never restates one and reads its targets
//! from `RingsState`. `targets` holds the one reader and the one writer of each stored target.
//! [`read_rings`] is the one producer of `RingsState`.

use rusqlite::Connection;

use crate::clock::{local_date, week_start, LocalZone};
use crate::index::IndexError;
use crate::protocol::RingsState;

pub mod active;
pub mod commit_days;
pub mod focus;
pub mod rhythm;
pub mod ship;
pub mod targets;
pub mod tend;

/// [p4] §39.12 — the constants §39 owns. 45, 3, Monday weeks and "no daily Ship" are the
/// defaults the user may overturn.
pub const FOCUS_TARGET_DEFAULT_MINUTES: u32 = 45;
/// §39.3's floor: one touch must never close the ring (the assert lives in `session/mod.rs`).
pub const FOCUS_TARGET_MIN_MINUTES: u32 = 30;
/// The shortest local day, so every allowed target is closable on every day.
pub const FOCUS_TARGET_MAX_MINUTES: u32 = 1_380;
/// How many settled weeks the first settlement reads back when it derives the rhythm target.
pub const RHYTHM_WINDOW_WEEKS: u32 = 52;
/// The fewest active weeks in that window from which a target is derived rather than defaulted.
pub const RHYTHM_MIN_ACTIVE_WEEKS: usize = 4;
/// The rhythm target when too few weeks are active to derive one; unmeasured.
pub const RHYTHM_FALLBACK_TARGET: u32 = 3;

/// [p4] §39's pure function of the index and the instant, and the one producer of
/// `RingsState`. It writes nothing — no key, no settlement.
///
/// Day one, and a day one not yet stamped, waive the rings: `focus`, `ship` and `tend` are
/// absent, not zero, while the rhythm is present (§39.9). Its production caller is §43's
/// `motivation_state`, for `motivation.get` and `motivation/changed`; nothing else builds one.
///
/// # Errors
/// Fails when the index cannot be read, or a stored week is malformed.
pub fn read_rings(
    conn: &Connection,
    now: i64,
    zone: &dyn LocalZone,
) -> Result<RingsState, IndexError> {
    let today = zone.at(now).day();
    let day_one = rhythm::day_one_epoch(conn)?;
    // NULL waives too, so no reading before the first presentation — the tray's included —
    // shows a quantity the user was never told is waived.
    let waived = day_one.is_none_or(|d| d == today);
    let rhythm = rhythm::rhythm_state(conn, today, day_one, zone, now)?;
    let (focus, ship, tend) = if waived {
        (None, None, None)
    } else {
        (
            Some(focus::focus_ring(conn, today, zone, now)?),
            Some(ship::ship_ring(conn, today)?),
            Some(tend::tend_ring(conn, today)?),
        )
    };
    Ok(RingsState {
        today: local_date(today),
        week_start: local_date(week_start(today)),
        waived,
        focus,
        ship,
        tend,
        rhythm,
    })
}

//! §38.6: the floor — the highest level the user has been confirmed at, in the one-row
//! `level_floor` table. It only rises; the table's triggers are the backstop, and the one
//! writer here is the mechanism.

use rusqlite::{Connection, Transaction};

use super::curve::level_of;
use super::level::{pending_projects, state_of};
use super::xp::xp_by_term;
use crate::index::IndexError;
use crate::protocol::LevelState;
use crate::view::state::NOTICE_DISMISSED_PREFIX;

/// The floor's level.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn read_floor(conn: &Connection) -> Result<u32, IndexError> {
    let level: i64 = conn.query_row("SELECT level FROM level_floor WHERE id = 1", [], |r| {
        r.get(0)
    })?;
    Ok(u32::try_from(level).unwrap_or(1))
}

/// Whether the identity card has been answered (§1.4's two outcomes): the set was confirmed, or
/// the card was dismissed, which leaves the seeded set in force.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn identity_answered(conn: &Connection) -> Result<bool, IndexError> {
    let answered: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM identity WHERE confirmed_at IS NOT NULL)
             OR EXISTS (SELECT 1 FROM view_state WHERE k = ?1)",
        [format!("{NOTICE_DISMISSED_PREFIX}identity")],
        |r| r.get(0),
    )?;
    Ok(answered)
}

/// Raise the floor to `level` when it is lower, dating the raise; `true` when it rose. A
/// `level` at or below the floor writes nothing and leaves `raised_at` as it was.
///
/// # Errors
/// Returns [`IndexError`] when the write fails.
pub fn ratchet_level_floor(tx: &Transaction<'_>, level: u32, now: i64) -> Result<bool, IndexError> {
    let raised = tx.execute(
        "UPDATE level_floor SET level = ?1, raised_at = ?2 WHERE id = 1 AND level < ?1",
        rusqlite::params![level, now],
    )?;
    Ok(raised == 1)
}

/// §38.6.2: raise the floor to the computed level, but only when the reading is `computed` and
/// the identity card has been answered. Both are read before anything the caller writes, so a
/// caller about to delete history ratchets from the reading it is about to change.
///
/// The answer itself may lower the level: until it exists, the level counts addresses the user
/// may yet untick, and locking that in would keep them.
pub(crate) fn ratchet_from_reading(tx: &Transaction<'_>, now: i64) -> Result<bool, IndexError> {
    let computed = state_of(tx, pending_projects(tx)?)? == LevelState::Computed;
    if !computed || !identity_answered(tx)? {
        return Ok(false);
    }
    ratchet_level_floor(tx, level_of(xp_by_term(tx)?.total()), now)
}

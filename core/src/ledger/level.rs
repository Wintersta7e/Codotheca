//! §38.5: the one level reading, and the count of projects whose history it still waits on.
//!
//! **Unknown is never rendered as zero.** While a history is pending over an empty ledger the
//! reading carries no number at all; once some rows exist it is `partial`, and only when nothing
//! is pending is it `computed`.

use rusqlite::{Connection, Transaction};

use super::curve::{level_of, threshold};
use super::floor::{may_raise, ratchet_level_floor, read_floor};
use super::xp::xp_by_term;
use crate::index::IndexError;
use crate::projects::current::IS_CURRENT_SQL;
use crate::protocol::{LevelFigures, LevelReading, LevelState};

/// §38.5.1's *concluded*, for the project aliased `p`: its history finished (`j4` reads `ok`),
/// or something on its history chain failed or was deferred. The one spelling, spliced into every
/// statement that asks.
pub(crate) const CONCLUDED_SQL: &str = "EXISTS (SELECT 1 FROM project_job_state j
                 WHERE j.project_id = p.id
                   AND ((j.job = 'j4' AND j.state = 'ok')
                     OR (j.job IN ('j1', 'j1_5', 'j4')
                         AND j.state IN ('failed', 'deferred_slow'))))";

/// How many projects §38.5.1 counts as pending.
///
/// A project is pending when it is eligible — not merged away, not Reference, current, and with
/// a copy that is present — and not concluded: its history has not finished (`j4` reads `ok`)
/// and nothing on its history chain failed or was deferred.
///
/// A copy that is away is not pending: its rows stand frozen until it returns. A routine scan
/// does not make a project pending, because a job row is written at settle and not at enqueue.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn pending_projects(conn: &Connection) -> Result<u32, IndexError> {
    let pending: i64 = conn.query_row(
        &format!(
            "SELECT count(*) FROM project p
              WHERE p.merged_into IS NULL AND p.is_reference = 0 AND {IS_CURRENT_SQL}
                AND EXISTS (SELECT 1 FROM location l
                             WHERE l.project_id = p.id AND l.removed_at IS NULL
                               AND l.presence = 'present')
                AND NOT {CONCLUDED_SQL}"
        ),
        [],
        |r| r.get(0),
    )?;
    Ok(u32::try_from(pending).unwrap_or(u32::MAX))
}

/// §38.5.1's state, from the pending count and whether the ledger holds any row at all.
pub(crate) fn state_of(conn: &Connection, pending: u32) -> Result<LevelState, IndexError> {
    if pending == 0 {
        return Ok(LevelState::Computed);
    }
    let any: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM xp_events)", [], |r| r.get(0))?;
    Ok(if any {
        LevelState::Partial
    } else {
        LevelState::NotComputed
    })
}

/// The level reading, and the only production caller of the level function (§38.5.2).
///
/// It takes the reading once, ratchets the floor from it when the reading allows it, then
/// returns the XP and the displayed level, the greater of the computed level and the floor.
/// Under `notComputed` it carries no figures at all.
///
/// It reads no quiet-mode key — the reading is computed in every mode and its readers decide
/// what renders.
///
/// # Errors
/// Returns [`IndexError`] when a read or the floor's raise fails.
pub fn read_level(tx: &Transaction<'_>, now: i64) -> Result<LevelReading, IndexError> {
    let pending_projects = pending_projects(tx)?;
    let state = state_of(tx, pending_projects)?;
    if state == LevelState::NotComputed {
        return Ok(LevelReading {
            state,
            figures: None,
            pending_projects,
        });
    }
    let xp = xp_by_term(tx)?.total();
    let earned = level_of(xp);
    if may_raise(tx, state)? {
        ratchet_level_floor(tx, earned, now)?;
    }
    let floor = read_floor(tx)?;
    let level = earned.max(floor);
    Ok(LevelReading {
        state,
        figures: Some(LevelFigures {
            level,
            xp,
            level_at: threshold(level),
            next_level_at: threshold(level.saturating_add(1)),
            floored: floor > earned,
        }),
        pending_projects,
    })
}

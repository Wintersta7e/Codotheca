//! §21.5's cadence, and the five triggers that ask for a listing.

use rusqlite::Connection;

use crate::index::IndexError;
use crate::protocol::{AccountId, SyncTaskKind, SyncTaskState};
use crate::sync::state::state_slug;
use crate::sync::task::kind_slug;

/// §21.5: **every six hours**, plus the four event triggers below.
///
/// A listing costs about one request per 100 repositories, so cadence is not what the budget
/// constrains; per-repository work is.
///
/// **This is a scheduling interval, not a staleness threshold.** The renderer's
/// `REMOTE_STALE_AFTER_SECS` (`app/src/renderer/derive/observation.ts:28`) is a *rendering*
/// decision with its own owner, and R97(c) binds this plan to read that one rather than declare a
/// second. The two share a number because that constant's own derivation cites this cadence, and
/// `core/tests/sync_listing.rs` reads the TypeScript back to keep the citation true (R24).
pub const LISTING_INTERVAL_SECS: i64 = 6 * 60 * 60;

/// What asked for a listing. Recorded so a reader can tell a scheduled run from a triggered one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Connect,
    ScopeUpgrade,
    OrgOptInChange,
    Startup,
    Interval,
}

impl SyncTrigger {
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            SyncTrigger::Connect => "connect",
            SyncTrigger::ScopeUpgrade => "scope_upgrade",
            SyncTrigger::OrgOptInChange => "org_opt_in_change",
            SyncTrigger::Startup => "startup",
            SyncTrigger::Interval => "interval",
        }
    }
}

/// Which enabled accounts want a listing now.
///
/// An account with **no** `account_repos` row has never listed and is due at once — that is the
/// start-up case §21.5 names, and it is why the join is a `LEFT JOIN` rather than a filter.
/// An account whose row is in any state other than `ok` is already queued, running, parked,
/// deferred or blocked, and re-queueing it would either duplicate work or overwrite a terminal
/// row with a clock.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn due_listings(conn: &Connection, now: i64) -> Result<Vec<AccountId>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT a.id FROM account a
           LEFT JOIN sync_task_state s ON s.task = ?1 AND s.key = a.id
          WHERE a.is_enabled = 1
            AND (s.id IS NULL OR (s.state = ?2 AND s.at + ?3 <= ?4))
          ORDER BY a.id",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                kind_slug(SyncTaskKind::AccountRepos),
                state_slug(SyncTaskState::Ok),
                LISTING_INTERVAL_SECS,
                now
            ],
            |row| row.get::<_, i64>(0).map(AccountId),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

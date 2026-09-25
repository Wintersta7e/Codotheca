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
    /// An account was connected.
    Connect,
    /// An account's grant gained a scope.
    ScopeUpgrade,
    /// The user opted one of the account's organisations in or out.
    OrgOptInChange,
    /// The process started and found an account that has never listed.
    Startup,
    /// [`LISTING_INTERVAL_SECS`] passed since the account's last settled listing.
    Interval,
}

impl SyncTrigger {
    /// The trigger's `snake_case` spelling, in the shape of the other sync slugs.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::ScopeUpgrade => "scope_upgrade",
            Self::OrgOptInChange => "org_opt_in_change",
            Self::Startup => "startup",
            Self::Interval => "interval",
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

/// [p3] §32.2's cadence: **daily, because the source's own cadence is daily.**
///
/// §32.9's 30-day clean-result expiry is derived from it: a clean verdict that aged out means
/// ~30 consecutive attempts failed to reach the source, not that one request failed.
pub const ADVISORY_SWEEP_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// [p3] Whether the advisory sweep is due.
///
/// True when **no sweep has settled**, or the newest `settled_at` is older than the interval. A
/// sweep that **started** but never settled does not satisfy it — a start is not an observation,
/// and a crashed sweep must not push the next one a whole day away.
///
/// **A library with no dependency triples is never due**, on `due_listings`' own terms: that
/// function returns only *enabled* accounts rather than queuing work to discover there is none.
/// A sweep with nothing to ask about would settle a row claiming an observation of nothing, and
/// would take the process's one index mutex on every cadence turn to learn that.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn advisory_due(conn: &Connection, now: i64) -> Result<bool, IndexError> {
    let asks: i64 = conn.query_row(
        "SELECT count(*) FROM (SELECT 1 FROM project_dependency LIMIT 1)",
        [],
        |row| row.get(0),
    )?;
    if asks == 0 {
        return Ok(false);
    }
    let newest: Option<i64> = conn
        .query_row(
            "SELECT max(settled_at) FROM advisory_sweep WHERE settled_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(None);
    Ok(newest.map_or(true, |at| {
        at.saturating_add(ADVISORY_SWEEP_INTERVAL_SECS) <= now
    }))
}

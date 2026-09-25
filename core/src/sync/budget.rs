//! §21.6's rate budget — **mirrored, never counted**.
//!
//! `remaining`, `limit`, `reset` and `resource` are read from every response's `x-ratelimit-*`
//! headers, **including error responses**, and written here. The server stays the one owner of
//! the value, so a restart inside a reset window resumes from the last observation instead of
//! from zero or from the limit.
//!
//! Task 3's decorator **records**; this module **writes**. That split is what lets §21.9's rule 2
//! be honoured — a value and its clock commit in one transaction — while §21.6's *every response*
//! stays structural rather than repeated at each call site.

use rusqlite::{Connection, Transaction};

use crate::index::IndexError;
use crate::protocol::AccountId;
use crate::sync::classify::RateSnapshot;

/// §21.5's floor, as a **fraction of the pool it is spent from**.
///
/// **[p3] It replaces `ON_DEMAND_RESERVE: i64 = 200`, which is deleted rather than joined.**
/// Keeping both the absolute and the divisor would be one value stated twice, created on purpose;
/// the 200 survives as what this formula yields for the pool it was written for.
///
/// The constant was not wrong — it was an **absolute where the quantity is relative**. 200 against
/// §21.5's authenticated 5,000/hr is 4%. Against the unauthenticated 60/hr pool §32's sweep draws
/// on, `remaining` can never reach 200, so the sweep issued exactly one request in the lifetime of
/// the process and every later pick answered `Reserved` — and because `Reserved` issues nothing,
/// `remaining` was never re-observed and never rose. `sync.status` reported it as
/// `parked · reserve`, **which reads as correct throttling**.
pub const ON_DEMAND_RESERVE_DIVISOR: i64 = 25;

/// The reserve for a pool of this size, or **`None` where the limit is unobserved**.
///
/// `None` means *no reserve applies*, never *a reserve of zero* — the same answer [`may_spend`]
/// already gives for an unobserved `remaining`, so this adds no new unknown case and no new
/// branch. Exactly **200 at `limit = 5000`**, so the authenticated case does not move by one unit,
/// and **2 at `limit = 60`**.
#[must_use]
// Taking the `Option` is the contract, not a convenience: callers hand over the pool's `limit` as
// read, and `core/tests/sync_budget.rs` pins `reserve_for(None) == None` as the unobserved case.
#[allow(clippy::single_option_map)]
pub fn reserve_for(limit: Option<i64>) -> Option<i64> {
    limit.map(|limit| limit / ON_DEMAND_RESERVE_DIVISOR)
}

/// One pool's last observation.
///
/// **Fields are private and there is no `Default`.** A `BudgetRow` can only be built by something
/// that has an `observed_at` in hand — [`BudgetRow::observed`], called by [`read_budget`] over a
/// row the mirror wrote. A derived `Default` would hand out `observed_at: 0`, a real-looking
/// epoch second for a pool nothing has ever looked at, and every reader downstream would treat it
/// as an observation. R100: where a type can make the dishonest state unconstructable, that beats
/// asserting its absence.
///
/// Every number is optional because **unobserved is unknown, not zero**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetRow {
    remaining: Option<i64>,
    limit: Option<i64>,
    reset_at: Option<i64>,
    observed_at: i64,
}

impl BudgetRow {
    /// The only constructor, and it takes the observation time because that is the one thing a
    /// row cannot be honest without.
    #[must_use]
    pub const fn observed(
        remaining: Option<i64>,
        limit: Option<i64>,
        reset_at: Option<i64>,
        observed_at: i64,
    ) -> Self {
        Self {
            remaining,
            limit,
            reset_at,
            observed_at,
        }
    }

    /// Requests left in the pool as `x-ratelimit-remaining` last said, or `None` when that header
    /// was never observed.
    #[must_use]
    pub const fn remaining(&self) -> Option<i64> {
        self.remaining
    }

    /// The pool's size per window as `x-ratelimit-limit` last said, or `None` when unobserved.
    #[must_use]
    pub const fn limit(&self) -> Option<i64> {
        self.limit
    }

    /// Already on **our** clock: `crate::sync::classify::translate_instant` moved it there before
    /// it was ever written.
    #[must_use]
    pub const fn reset_at(&self) -> Option<i64> {
        self.reset_at
    }

    /// The Unix second, on our clock, at which the mirror last wrote this row.
    #[must_use]
    pub const fn observed_at(&self) -> i64 {
        self.observed_at
    }
}

/// What the runner may do with one task, given what is known about its pool.
///
/// **`ParkUntil` and `Reserved` are two verdicts and not one**, which is a widening of the three
/// this plan's task table names. Both park, and they park for different reasons: `ParkUntil` is
/// *the allowance is gone* (§21.6) and `Reserved` is *the allowance is being kept for what the
/// user is looking at* (§21.5). §21.5 requires the settled row's `reason` to record `reserve` so a
/// status reader can tell the three apart, and a single variant would make that reason a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetVerdict {
    /// Issue the request: the pool has room, or its reset has passed and the request is what
    /// refreshes a stale mirror.
    Spend,
    /// `remaining` is known to be 0 and the reset has not passed.
    ParkUntil(i64),
    /// `remaining` is known and below [`reserve_for`] this pool's limit, and this is a scheduled
    /// task.
    Reserved(i64),
    /// Nothing has been observed. **A task whose budget is unknown proceeds** — the first request
    /// is what discovers the number.
    Unknown,
}

/// Write one observation into the pool it names.
///
/// `Ok(false)` — and **no write at all** — when the response carried no `x-ratelimit-resource`:
/// there is nothing to key the row by, and §21.6 rules that non-observation must not move
/// `observed_at`. Returning `false` rather than erroring is the point: a response without rate
/// headers is ordinary, not a fault.
///
/// `account` is `None` for the unauthenticated **per-IP** pool, which is per-process and shared
/// across every account, so it is keyed by the *absence* of an account rather than by a sentinel
/// one. The two upserts differ only in their conflict target, which has to name the partial index
/// that actually covers each case.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn mirror(
    tx: &Transaction<'_>,
    account: Option<AccountId>,
    rate: &RateSnapshot,
    now: i64,
) -> Result<bool, IndexError> {
    let Some(resource) = rate.resource.as_deref() else {
        return Ok(false);
    };
    match account {
        Some(account) => tx.execute(
            "INSERT INTO sync_budget (account_id, resource, remaining, limit_, reset_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id, resource) WHERE account_id IS NOT NULL DO UPDATE SET
               remaining = excluded.remaining, limit_ = excluded.limit_,
               reset_at = excluded.reset_at, observed_at = excluded.observed_at",
            rusqlite::params![
                account.0,
                resource,
                rate.remaining,
                rate.limit,
                rate.reset_at,
                now
            ],
        )?,
        None => tx.execute(
            "INSERT INTO sync_budget (account_id, resource, remaining, limit_, reset_at, observed_at)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(resource) WHERE account_id IS NULL DO UPDATE SET
               remaining = excluded.remaining, limit_ = excluded.limit_,
               reset_at = excluded.reset_at, observed_at = excluded.observed_at",
            rusqlite::params![resource, rate.remaining, rate.limit, rate.reset_at, now],
        )?,
    };
    Ok(true)
}

/// The pool's last observation, or `None` when there has never been one.
///
/// **`None` is not an empty budget.** It is *nothing has been observed*, and [`may_spend`] answers
/// `Unknown` for it, which spends.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn read_budget(
    conn: &Connection,
    account: Option<AccountId>,
    resource: &str,
) -> Result<Option<BudgetRow>, IndexError> {
    let read = |row: &rusqlite::Row<'_>| {
        Ok(BudgetRow::observed(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
        ))
    };
    let found = account.map_or_else(
        || {
            conn.query_row(
                "SELECT remaining, limit_, reset_at, observed_at FROM sync_budget
                  WHERE account_id IS NULL AND resource = ?1",
                rusqlite::params![resource],
                read,
            )
            .ok()
        },
        |account| {
            conn.query_row(
                "SELECT remaining, limit_, reset_at, observed_at FROM sync_budget
                  WHERE account_id = ?1 AND resource = ?2",
                rusqlite::params![account.0, resource],
                read,
            )
            .ok()
        },
    );
    Ok(found)
}

/// §21.5 and §21.6's two rules, in the order they bind.
///
/// `on_demand` is the **on-demand** side, matching §21.5's *"only on-demand tasks may spend"*.
/// `crate::sync::is_on_demand` is what supplies it.
///
/// With `remaining` known to be 0 and the reset not yet reached, **no request is issued**: the
/// mirror is used, not merely recorded. Past the reset the mirror is stale and the request is
/// what refreshes it, so the answer is `Spend` — one request, self-correcting, rather than a park
/// that never ends.
///
/// **A reserve with no observed `reset_at` names `now`**, meaning *nothing observed an instant* —
/// not *release immediately*. `crate::sync::state::apply_outcome` is what turns the verdict into a
/// clock and floors an expired one at `crate::sync::state::SYNC_UNNAMED_PARK_SECS`; unfloored,
/// this path was measured at 3,334 park/re-pick cycles in 500 ms, each taking the process's one
/// index mutex twice and emitting three events. Recorded rather than hidden, because it is the one
/// place the reserve rests on a number nobody observed.
#[must_use]
pub fn may_spend(row: Option<&BudgetRow>, on_demand: bool, now: i64) -> BudgetVerdict {
    // Never observed, or observed without the number: unknown, and unknown spends.
    let Some(remaining) = row.and_then(BudgetRow::remaining) else {
        return BudgetVerdict::Unknown;
    };
    let reset_at = row.and_then(BudgetRow::reset_at);

    if remaining <= 0 {
        return match reset_at {
            Some(reset) if now < reset => BudgetVerdict::ParkUntil(reset),
            _ => BudgetVerdict::Spend,
        };
    }
    // **The reserve is a fraction of this pool, and an unobserved limit reserves nothing.**
    if !on_demand && reserve_for(row.and_then(BudgetRow::limit)).is_some_and(|r| remaining < r) {
        return BudgetVerdict::Reserved(reset_at.unwrap_or(now));
    }
    BudgetVerdict::Spend
}

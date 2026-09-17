//! `sync_task_state` — the durable scheduling state of one sync task (§21.4).
//!
//! **`fail_count` and `throttle_count` are two counters and never one.** `fail_count` counts
//! transient failures and drives `deferred` at three; `throttle_count` counts consecutive
//! secondary-limit parks and lengthens the park. **A park is not a failure**, so a throttled task
//! must never strand itself in `deferred` — which is what a single counter would do after three
//! parks against a forge that is behaving exactly as documented.
//!
//! §4's jobs double from one second (`core/src/jobs/state.rs:13-18`). **Sync does not reuse that
//! schedule**: a one-second retry against a forge's secondary limit is the behaviour that gets an
//! account blocked.

use crate::protocol::{SyncTaskKind, SyncTaskState};
use crate::sync::outcome::SyncOutcome;

/// Every state, in §21.4's table order. Walked by the CHECK-agreement gate, which inserts each
/// one against a **migrated database** rather than reading the DDL text.
pub const SYNC_STATES: [SyncTaskState; 6] = [
    SyncTaskState::Queued,
    SyncTaskState::Running,
    SyncTaskState::Parked,
    SyncTaskState::Ok,
    SyncTaskState::Deferred,
    SyncTaskState::Blocked,
];

/// The stored form, written into `sync_task_state.state` and constrained by that column's CHECK.
///
/// R64 keeps this name beside `core/src/art/store.rs`'s `state_slug`: two unrelated enums with
/// two unrelated CHECK lists, each the R26 mirror for its own column. Same name, different shape
/// — R15's rule, not a duplicate.
#[must_use]
pub fn state_slug(state: SyncTaskState) -> &'static str {
    match state {
        SyncTaskState::Queued => "queued",
        SyncTaskState::Running => "running",
        SyncTaskState::Parked => "parked",
        SyncTaskState::Ok => "ok",
        SyncTaskState::Deferred => "deferred",
        SyncTaskState::Blocked => "blocked",
    }
}

/// §21.4's retry budget.
///
/// **Deliberately not `MAX_TRANSIENT_FAILS`**, which `core/src/jobs/state.rs:11` already owns for
/// §4.1. Two independent retry budgets that happen to share the value `3`; importing the job one
/// would make a change to §4.1's local-scan retry policy silently change sync's, and the two
/// schedules already differ in every other respect — 1 s doubling to 60 s there, 5 s doubling to
/// 5 min here.
pub const SYNC_MAX_TRANSIENT_FAILS: u32 = 3;

/// §21.4's transient schedule: doubling from **5 s**, capped at **5 min**.
///
/// `fail_count` is the count **after** the failure, so the first retry waits 5 s.
#[must_use]
pub fn transient_backoff_secs(fail_count: u32) -> i64 {
    let steps = fail_count.saturating_sub(1).min(30);
    let doubled = 5_i64.checked_shl(steps).unwrap_or(300);
    doubled.clamp(5, 300)
}

/// The floor under a park whose instant the server did not usefully name (§21.4).
///
/// **The same 60 s `secondary_park_secs` floors at, and it is the same question**: the forge
/// declined to say when, so this process picks the number. §21.4's table sends a primary yield
/// *"to `reset_at` exactly"*, which presumes a `reset_at`; a `429` carrying neither `retry-after`
/// nor `x-ratelimit-reset` names no instant at all, and both of the places that fold that absence
/// into "now" (`crate::sync::classify` and `crate::sync::budget::may_spend`) then produce a park
/// that has already expired.
///
/// A park that releases at or before the instant it was made is not a park. `run_loop` re-picks a
/// runnable row with no sleep between iterations, so before this floor existed an independent
/// review measured **2,413 requests in 500 ms** against a forge already answering `429`, and
/// **3,334 park/re-pick cycles in 500 ms** on the reserve path — with a real clock as well as a
/// frozen one, because the park instant is `<= now` on the very next iteration either way.
pub const SYNC_UNNAMED_PARK_SECS: i64 = 60;

/// §21.4's secondary schedule: `Retry-After`, **floored at 60 s**, doubled per consecutive
/// `throttle_count`, capped at **1 h**.
///
/// `throttle_count` is the count of **previous** consecutive secondary parks, so the first one is
/// the floor undoubled.
///
/// **The 60 s floor is documentation knowledge, not measurement** (§21.4, R56). Its id is
/// `AC-P2-21-3-floor`, registered `deferred` in `acceptance/criteria.json` and verified against a
/// live response before shipping: a floor asserted from a document is a number that will be met
/// rather than checked.
#[must_use]
pub fn secondary_park_secs(retry_after: i64, throttle_count: u32) -> i64 {
    let base = retry_after.max(60);
    let steps = throttle_count.min(30);
    let doubled = base.checked_shl(steps).unwrap_or(3600);
    doubled.clamp(60, 3600)
}

/// Why a `deferred` task becomes runnable again.
///
/// **A separate enum from `core::jobs::state::ResetCause`**, because three of the five causes are
/// account-shaped and do not exist in the job vocabulary — compare shapes, not names (R15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncResetCause {
    AccountReconnected,
    ScopeUpgraded,
    OrgOptInChanged,
    AppUpgraded,
    UserRequested,
}

impl SyncResetCause {
    /// Written into `reason`, so a later reader can see why the row was revived.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            SyncResetCause::AccountReconnected => "account_reconnected",
            SyncResetCause::ScopeUpgraded => "scope_upgraded",
            SyncResetCause::OrgOptInChanged => "org_opt_in_changed",
            SyncResetCause::AppUpgraded => "app_upgraded",
            SyncResetCause::UserRequested => "user_requested",
        }
    }
}

/// One row of `sync_task_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTaskStateRow {
    pub kind: SyncTaskKind,
    /// Polymorphic: an account id for `account_repos`, a project id for the other two, `None`
    /// for a process-wide task. Phase 2 declares none.
    pub key: Option<i64>,
    pub state: SyncTaskState,
    /// Resumption state, and **only** resumption state (§21.7). Cleared on any settle that is not
    /// `NextPage`.
    pub cursor: Option<String>,
    pub fail_count: u32,
    pub throttle_count: u32,
    pub reason: Option<String>,
    pub at: i64,
    pub not_before: i64,
}

impl SyncTaskStateRow {
    /// A fresh row, runnable now.
    #[must_use]
    pub fn queued(kind: SyncTaskKind, key: Option<i64>, now: i64) -> SyncTaskStateRow {
        SyncTaskStateRow {
            kind,
            key,
            state: SyncTaskState::Queued,
            cursor: None,
            fail_count: 0,
            throttle_count: 0,
            reason: None,
            at: now,
            not_before: 0,
        }
    }

    /// Whether the runner may pick this row up.
    ///
    /// **It gates on the state first, and that is what makes a terminal row terminal.** A
    /// `blocked` row's `not_before` is 0 — the column's default — so a predicate reading only the
    /// clock would make every blocked task immediately runnable, which is the retry loop against
    /// an unauthorised token that §21.8 exists to prevent. `parked` is not runnable either: the
    /// loop promotes it to `queued` by the clock first, so *becoming runnable* and *being
    /// runnable* stay two separate facts.
    #[must_use]
    pub fn is_runnable(&self, now: i64) -> bool {
        self.state == SyncTaskState::Queued && self.not_before <= now
    }

    /// Whether the clock alone releases this row into `queued`.
    #[must_use]
    pub fn is_due_park(&self, now: i64) -> bool {
        self.state == SyncTaskState::Parked && self.not_before <= now
    }
}

/// §21.8's outcome table, as a pure function.
///
/// Returns the row to persist and, **when a retry is scheduled**, the epoch second it becomes
/// runnable. `None` is *nothing is scheduled*, and for `blocked` that is permanent: the row is
/// left only through an account state change or an explicit user action, and **never on a
/// backoff**.
///
/// It takes no database and no transport, so the whole table is testable from a fabricated
/// response through the real classifier.
#[must_use]
pub fn apply_outcome(
    row: &SyncTaskStateRow,
    outcome: &SyncOutcome,
    now: i64,
) -> (SyncTaskStateRow, Option<i64>) {
    let mut next = row.clone();
    next.at = now;

    match outcome {
        // 2xx with a body, and a 304. Both are responses the server produced, so both reset both
        // counters — §21.8's table, which is the binding form where §21.4's prose is loose.
        SyncOutcome::Done | SyncOutcome::NotModified => {
            next.state = SyncTaskState::Ok;
            next.fail_count = 0;
            next.throttle_count = 0;
            next.reason = None;
            next.cursor = None;
            next.not_before = 0;
            (next, None)
        }
        // §21.8's table leaves **both counters unchanged** here, unlike the two above: a 404 is
        // an answer about one resource and says nothing about whether this task is failing or
        // being throttled. It settles `ok`, deletes nothing, and is never terminal for the
        // account — the repository is *unseen*, never *gone*.
        SyncOutcome::NotFound => {
            next.state = SyncTaskState::Ok;
            next.reason = Some("not_found".to_owned());
            next.cursor = None;
            next.not_before = 0;
            (next, None)
        }
        // The same read continuing, not a new trigger: runnable immediately, cursor kept.
        SyncOutcome::NextPage { cursor } => {
            next.state = SyncTaskState::Queued;
            next.fail_count = 0;
            next.throttle_count = 0;
            next.reason = None;
            next.cursor = Some(cursor.clone());
            next.not_before = now;
            (next, Some(now))
        }
        SyncOutcome::Throttled { until, secondary } => {
            next.state = SyncTaskState::Parked;
            let at = if *secondary {
                next.reason = Some("secondary_limit".to_owned());
                let asked = until.saturating_sub(now).max(0);
                let park = secondary_park_secs(asked, next.throttle_count);
                next.throttle_count = next.throttle_count.saturating_add(1);
                now.saturating_add(park)
            } else {
                // A primary yield goes to `reset_at` **exactly** and increments nothing.
                next.reason = Some("rate_limited".to_owned());
                *until
            };
            // **A park releases strictly after the instant it was made** — see
            // [`SYNC_UNNAMED_PARK_SECS`]. The floor lives here rather than at the two call sites
            // that can produce an expired instant, because here it is the only arm that writes a
            // `parked` row: a third caller cannot reintroduce the loop (R100).
            let at = if at > now {
                at
            } else {
                now.saturating_add(SYNC_UNNAMED_PARK_SECS)
            };
            next.not_before = at;
            (next, Some(at))
        }
        SyncOutcome::Unauthorized { reason } => {
            next.state = SyncTaskState::Blocked;
            next.reason = Some(reason.slug().to_owned());
            next.not_before = 0;
            (next, None)
        }
        SyncOutcome::Rejected { status } => {
            next.state = SyncTaskState::Blocked;
            next.reason = Some(format!("rejected_{status}"));
            next.not_before = 0;
            (next, None)
        }
        SyncOutcome::TransientFail { reason } => {
            next.fail_count = next.fail_count.saturating_add(1);
            next.reason = Some(reason.clone());
            if next.fail_count >= SYNC_MAX_TRANSIENT_FAILS {
                next.state = SyncTaskState::Deferred;
                next.not_before = 0;
                (next, None)
            } else {
                next.state = SyncTaskState::Queued;
                let at = now.saturating_add(transient_backoff_secs(next.fail_count));
                next.not_before = at;
                (next, Some(at))
            }
        }
    }
}

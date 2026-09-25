#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §21.4's two counters, its two backoff schedules, and what is terminal.
//!
//! **Driven from a fabricated `HttpResponse` through the real `classify`**, so nothing above the
//! classifier is faked and the outcomes under test are the ones the product actually produces.
//! AC-P2-21-3's three 403 shapes and AC-P2-21-4's 401 are assertions about classification code;
//! a fake provider above it would assert the fake.
//!
//! Covers **AC-P2-21-3** (three separate cases), **AC-P2-21-4** and **AC-P2-21-3-floor**.

use codotheca_core::http::{HttpResponse, TransportError};
use codotheca_core::protocol::{SyncTaskKind, SyncTaskState};
use codotheca_core::sync::classify::classify;
use codotheca_core::sync::outcome::{SyncOutcome, UnauthorizedReason};
use codotheca_core::sync::state::{
    apply_outcome, secondary_park_secs, transient_backoff_secs, SyncResetCause, SyncTaskStateRow,
    SYNC_MAX_TRANSIENT_FAILS,
};
use codotheca_core::sync::store::{load, load_all, put, reset_for, SyncResetScope};
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_800_000_000;

fn row() -> SyncTaskStateRow {
    SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(1), NOW)
}

/// The outcome a response of this shape actually produces. Never a hand-built `SyncOutcome`:
/// the point of driving `classify` is that the state machine is fed what the wire produces.
fn outcome_of(status: u16, headers: &[(&str, &str)]) -> SyncOutcome {
    let response: Result<HttpResponse, TransportError> = Ok(HttpResponse {
        status,
        headers: codotheca_core::http::normalise_headers(headers.iter().copied()),
        body: b"{}".to_vec(),
    });
    classify(&response, NOW).0
}

/// **AC-P2-21-3, case 1 — a primary yield.** `403` + `x-ratelimit-remaining: 0` parks to
/// `reset_at` with **`fail_count` unchanged** and `throttle_count` unchanged. It did not fail and
/// it was not secondary-limited; it yielded to a budget the server owns.
#[test]
fn a_primary_yield_parks_to_the_reset_and_moves_neither_counter() {
    let reset = NOW + 900;
    let reset_text = reset.to_string();
    let outcome = outcome_of(
        403,
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset_text),
        ],
    );
    assert_eq!(
        outcome,
        SyncOutcome::Throttled {
            until: reset,
            secondary: false
        }
    );

    let mut before = row();
    before.fail_count = 2;
    before.throttle_count = 1;
    let (after, scheduled) = apply_outcome(&before, &outcome, NOW);
    assert_eq!(after.state, SyncTaskState::Parked);
    assert_eq!(scheduled, Some(reset));
    assert_eq!(after.not_before, reset);
    assert_eq!(after.fail_count, 2, "a park is not a failure");
    assert_eq!(
        after.throttle_count, 1,
        "and a primary yield is not secondary"
    );
}

/// **AC-P2-21-3, case 2 — a secondary limit.** `403` + `retry-after` with `remaining > 0` parks
/// **and increments `throttle_count`**, floored at 60 s and doubled on the second consecutive
/// one.
///
/// **AC-P2-21-3-floor.** The 60 s floor is §21.4's, and it is **documentation knowledge**: it is
/// registered `deferred` in `acceptance/criteria.json` and is verified against a live response
/// before shipping. What this asserts is that the implementation carries the floor, not that the
/// floor is the right number.
#[test]
fn a_secondary_limit_increments_its_own_counter_and_lengthens_the_park() {
    // A `retry-after` well under the floor, so the floor is what decides.
    let outcome = outcome_of(
        403,
        &[("retry-after", "5"), ("x-ratelimit-remaining", "4999")],
    );
    assert_eq!(
        outcome,
        SyncOutcome::Throttled {
            until: NOW + 5,
            secondary: true
        }
    );

    let (first, scheduled) = apply_outcome(&row(), &outcome, NOW);
    assert_eq!(first.state, SyncTaskState::Parked);
    assert_eq!(first.throttle_count, 1, "the secondary counter moved");
    assert_eq!(first.fail_count, 0, "and the failure counter did not");
    assert_eq!(
        scheduled,
        Some(NOW + 60),
        "a 5 s retry against a secondary limit is what gets an account blocked"
    );

    let (second, rescheduled) = apply_outcome(&first, &outcome, NOW);
    assert_eq!(second.throttle_count, 2);
    assert_eq!(
        rescheduled,
        Some(NOW + 120),
        "doubled on the second consecutive one"
    );
}

/// **AC-P2-21-3, case 3 — terminal.** `403` with **neither** header leaves the task `blocked`
/// with **no `not_before`** and no scheduled retry. None of a missing scope, an unauthorised SSO
/// organisation or revoked access is fixed by waiting, so waiting is not offered.
#[test]
fn a_terminal_403_blocks_with_no_scheduled_retry() {
    let outcome = outcome_of(403, &[]);
    assert_eq!(
        outcome,
        SyncOutcome::Unauthorized {
            reason: UnauthorizedReason::Forbidden
        }
    );
    let (after, scheduled) = apply_outcome(&row(), &outcome, NOW);
    assert_eq!(after.state, SyncTaskState::Blocked);
    assert_eq!(scheduled, None, "never retried on a backoff");
    assert_eq!(after.not_before, 0);
    assert!(
        !after.is_runnable(i64::MAX),
        "no passage of time makes a blocked row runnable"
    );
    assert_eq!(after.reason.as_deref(), Some("forbidden"));
}

/// **AC-P2-21-4**, asserted **separately** because it is the case the original question omitted:
/// the failure the settled row actually names is an expired token, and it arrives as **401**.
#[test]
fn a_401_is_terminal_and_names_the_token() {
    let outcome = outcome_of(401, &[]);
    let (after, scheduled) = apply_outcome(&row(), &outcome, NOW);
    assert_eq!(after.state, SyncTaskState::Blocked);
    assert_eq!(scheduled, None);
    assert_eq!(after.reason.as_deref(), Some("token_invalid"));
}

/// Any other 4xx is terminal too: a request this client formed wrongly will be formed wrongly
/// again, and retrying it spends the allowance to learn nothing.
#[test]
fn another_4xx_is_rejected_and_terminal() {
    let (after, scheduled) = apply_outcome(&row(), &outcome_of(422, &[]), NOW);
    assert_eq!(after.state, SyncTaskState::Blocked);
    assert_eq!(scheduled, None);
    assert_eq!(after.reason.as_deref(), Some("rejected_422"));
}

/// Three consecutive transient failures reach `deferred`, with backoffs **5, 10, —**. The fourth
/// schedules nothing: `deferred` is left by a revival cause, never by the clock.
#[test]
fn three_transient_failures_defer_with_a_five_second_doubling() {
    let outcome = outcome_of(503, &[]);
    let mut current = row();
    let mut backoffs = Vec::new();
    for _ in 0..4 {
        let (next, scheduled) = apply_outcome(&current, &outcome, NOW);
        backoffs.push(scheduled.map(|at| at - NOW));
        current = next;
    }
    assert_eq!(backoffs, [Some(5), Some(10), None, None]);
    assert_eq!(current.state, SyncTaskState::Deferred);
    assert_eq!(current.fail_count, 4, "the count keeps counting");
    assert_eq!(SYNC_MAX_TRANSIENT_FAILS, 3);

    // The schedule is §21.4's and not §4.1's: doubling from 5 s, capped at five minutes.
    assert_eq!(transient_backoff_secs(1), 5);
    assert_eq!(transient_backoff_secs(2), 10);
    assert_eq!(transient_backoff_secs(3), 20);
    assert_eq!(transient_backoff_secs(7), 300, "capped at five minutes");
    assert_eq!(transient_backoff_secs(40), 300, "and it cannot overflow");
}

/// **Ten consecutive secondary parks never reach `deferred`**, and the park is capped at an hour.
/// A park is not a failure, so a throttled task must never strand itself.
#[test]
fn ten_secondary_parks_never_defer_and_the_park_is_capped_at_an_hour() {
    let outcome = outcome_of(
        403,
        &[("retry-after", "10"), ("x-ratelimit-remaining", "4999")],
    );
    let mut current = row();
    for step in 1..=10 {
        let (next, scheduled) = apply_outcome(&current, &outcome, NOW);
        assert_eq!(next.state, SyncTaskState::Parked, "step {step}");
        assert_eq!(next.fail_count, 0, "step {step}: a park is not a failure");
        let park = scheduled.expect("a park always names an instant") - NOW;
        assert!((60..=3600).contains(&park), "step {step}: park {park}s");
        current = next;
    }
    assert_eq!(current.throttle_count, 10);
    assert_ne!(current.state, SyncTaskState::Deferred);

    assert_eq!(secondary_park_secs(5, 0), 60, "floored");
    assert_eq!(
        secondary_park_secs(90, 0),
        90,
        "and never below what was asked"
    );
    assert_eq!(
        secondary_park_secs(5, 1),
        120,
        "doubled per consecutive park"
    );
    assert_eq!(secondary_park_secs(5, 6), 3600, "capped at an hour");
    assert_eq!(secondary_park_secs(5, 40), 3600, "and it cannot overflow");
}

/// Any response the server actually produced resets **both** counters — including a `304`.
///
/// §21.4's sentence says *"any response that carried a body"* and a 304 carries none; §21.8's
/// table resets both on a `NotModified`. **The table is right**: §21.9's third rule makes a 304
/// *an observation and not the absence of one*, so treating it as a non-response would strand a
/// task that is succeeding on every conditional request.
#[test]
fn a_200_and_a_304_each_reset_both_counters() {
    let mut before = row();
    before.fail_count = 2;
    before.throttle_count = 5;
    before.reason = Some("server returned 503".to_owned());
    before.cursor = Some("page-3".to_owned());

    let (after, scheduled) = apply_outcome(&before, &outcome_of(200, &[]), NOW);
    assert_eq!(after.state, SyncTaskState::Ok);
    assert_eq!((after.fail_count, after.throttle_count), (0, 0));
    assert_eq!(after.reason, None);
    assert_eq!(
        after.cursor, None,
        "a settle that is not NextPage clears it"
    );
    assert_eq!(
        scheduled, None,
        "its next trigger schedules it, not a backoff"
    );

    let (after_not_modified, _) = apply_outcome(&before, &outcome_of(304, &[]), NOW);
    assert_eq!(after_not_modified.state, SyncTaskState::Ok);
    assert_eq!(
        (
            after_not_modified.fail_count,
            after_not_modified.throttle_count
        ),
        (0, 0)
    );
    assert_eq!(after_not_modified.cursor, None);
}

/// A `NextPage` is the one settle that keeps a cursor and is immediately runnable: the next page
/// is the same read continuing, not a new trigger.
#[test]
fn a_next_page_keeps_its_cursor_and_is_runnable_now() {
    let outcome = SyncOutcome::Done.with_next_page(Some("page-2".to_owned()));
    let (after, scheduled) = apply_outcome(&row(), &outcome, NOW);
    assert_eq!(after.state, SyncTaskState::Queued);
    assert_eq!(after.cursor.as_deref(), Some("page-2"));
    assert_eq!(scheduled, Some(NOW));
    assert!(after.is_runnable(NOW));
}

/// A `404` settles `ok` and deletes nothing: the repository is *unseen*, never *gone*.
#[test]
fn a_404_settles_ok_and_is_not_terminal_for_the_account() {
    let (after, scheduled) = apply_outcome(&row(), &outcome_of(404, &[]), NOW);
    assert_eq!(after.state, SyncTaskState::Ok);
    assert_eq!(scheduled, None);
    assert_eq!(after.fail_count, 0);
}

/// `reset_for` revives a `deferred` row and **leaves a `blocked` row alone**. `blocked` is left
/// only through an account state change or an explicit user action — never on a backoff, and
/// never by a cause that has nothing to say about the refusal.
#[test]
fn a_revival_cause_revives_deferred_and_never_blocked() {
    let mut fixture = TempIndex::new();
    let mut deferred = SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(1), NOW);
    deferred.state = SyncTaskState::Deferred;
    deferred.fail_count = 3;
    let mut blocked = SyncTaskStateRow::queued(SyncTaskKind::ProjectRemote, Some(7), NOW);
    blocked.state = SyncTaskState::Blocked;
    blocked.reason = Some("token_invalid".to_owned());

    fixture
        .index_mut()
        .with_tx(|tx| {
            put(tx, &deferred)?;
            put(tx, &blocked)?;
            let revived = reset_for(
                tx,
                SyncResetScope::Everything,
                SyncResetCause::AccountReconnected,
                NOW + 10,
            )?;
            assert_eq!(revived, 1, "exactly the deferred row");
            Ok(())
        })
        .expect("transaction");

    let conn = fixture.index().conn();
    let back = load(conn, SyncTaskKind::AccountRepos, Some(1))
        .expect("read")
        .expect("row");
    assert_eq!(back.state, SyncTaskState::Queued);
    assert_eq!(
        back.fail_count, 0,
        "a revival clears the count that deferred it"
    );
    assert_eq!(back.reason.as_deref(), Some("account_reconnected"));
    assert!(back.is_runnable(NOW + 10));

    let still = load(conn, SyncTaskKind::ProjectRemote, Some(7))
        .expect("read")
        .expect("row");
    assert_eq!(still.state, SyncTaskState::Blocked);
    assert_eq!(still.reason.as_deref(), Some("token_invalid"));

    assert_eq!(load_all(conn).expect("read").len(), 2);
}

/// Every revival cause has a slug, and the five are §21.4's. Three of them are account-shaped and
/// do not exist in `core::jobs::state::ResetCause`'s vocabulary — **compare shapes, not names**
/// (R15), which is why this is a separate enum rather than a reuse.
#[test]
fn the_five_revival_causes_each_have_their_own_slug() {
    let causes = [
        SyncResetCause::AccountReconnected,
        SyncResetCause::ScopeUpgraded,
        SyncResetCause::OrgOptInChanged,
        SyncResetCause::AppUpgraded,
        SyncResetCause::UserRequested,
    ];
    let slugs: Vec<&str> = causes.iter().map(|c| c.slug()).collect();
    eprintln!("sync_state: {} revival causes", slugs.len());
    assert_eq!(
        slugs,
        [
            "account_reconnected",
            "scope_upgraded",
            "org_opt_in_changed",
            "app_upgraded",
            "user_requested"
        ]
    );
}

/// A row round-trips through the table with every field intact, so `put` and `load` are one
/// value and not two spellings of it.
#[test]
fn a_row_round_trips_through_the_table() {
    let mut fixture = TempIndex::new();
    let original = SyncTaskStateRow {
        kind: SyncTaskKind::RenameProbe,
        key: Some(42),
        state: SyncTaskState::Parked,
        cursor: Some("cursor".to_owned()),
        fail_count: 1,
        throttle_count: 2,
        reason: Some("reserve".to_owned()),
        at: NOW,
        not_before: NOW + 300,
    };
    fixture
        .index_mut()
        .with_tx(|tx| {
            put(tx, &original)?;
            // A second `put` for the same (kind, key) replaces rather than duplicating.
            put(tx, &original)?;
            Ok(())
        })
        .expect("transaction");
    let back = load(fixture.index().conn(), SyncTaskKind::RenameProbe, Some(42))
        .expect("read")
        .expect("row");
    assert_eq!(back, original);
    assert_eq!(load_all(fixture.index().conn()).expect("read").len(), 1);
}

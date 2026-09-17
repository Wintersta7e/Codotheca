#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §21.6's mirror and §21.5's reserve.
//!
//! **We never count requests.** Every number here came off a response's own `x-ratelimit-*`
//! headers, which is what lets a restart inside a reset window resume from the last observation
//! instead of from zero or from the limit.
//!
//! Covers **AC-P2-21-5** (unobserved is unknown, never zero and never the limit) and
//! **AC-P2-21-14** (the on-demand reserve). The halves of both that need the runner — that a
//! request is actually issued, and that a scheduled task's row settles `parked` with
//! `reason = "reserve"` — land with the runner, in this same file.

use std::sync::Arc;

use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::http::{HttpResponse, HttpTransport, TransportError};
use codotheca_core::protocol::{AccountId, AuthKind, ScopeTier};
use codotheca_core::sync::budget::{
    may_spend, mirror, read_budget, BudgetRow, BudgetVerdict, ON_DEMAND_RESERVE,
};
use codotheca_core::sync::classify::{classify, RateSnapshot};
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_800_000_000;

fn seed_account(index: &mut codotheca_core::index::Index) -> AccountId {
    index
        .with_tx(|tx| {
            Ok(insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: "forge.example.invalid".to_owned(),
                    login: "owner".to_owned(),
                    display_name: None,
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Public,
                    granted_scopes: vec!["repo".to_owned()],
                    token_ref: "github:forge.example.invalid:owner".to_owned(),
                },
                NOW,
            )
            .expect("account"))
        })
        .expect("transaction")
}

/// The snapshot a response with these headers produces, through the **real classifier** rather
/// than through a hand-built struct: the parse is part of what this is asserting.
fn snapshot(status: u16, headers: &[(&str, &str)]) -> RateSnapshot {
    let response: Result<HttpResponse, TransportError> = Ok(HttpResponse {
        status,
        headers: codotheca_core::http::normalise_headers(headers.iter().copied()),
        body: b"{}".to_vec(),
    });
    classify(&response, NOW).1
}

/// **AC-P2-21-5.** An unobserved budget is **unknown**, and unknown **spends**.
///
/// Zero would stall sync forever; the limit would burn the allowance on an assumption. The first
/// request is what discovers the number, so a task whose budget is unknown proceeds.
#[test]
fn an_unobserved_budget_is_unknown_and_a_task_holding_one_proceeds() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut());

    let row = read_budget(fixture.index().conn(), Some(account), "core").expect("read");
    assert_eq!(row, None, "nothing has been observed yet");

    for on_demand in [true, false] {
        assert_eq!(
            may_spend(None, on_demand, NOW),
            BudgetVerdict::Unknown,
            "an unobserved budget is unknown whichever kind of task holds it"
        );
    }

    // And an observed row whose `remaining` the server did not send is equally unknown — the row
    // exists, the number does not.
    fixture
        .index_mut()
        .with_tx(|tx| {
            let wrote = mirror(
                tx,
                Some(account),
                &snapshot(200, &[("x-ratelimit-resource", "core")]),
                NOW,
            )?;
            assert!(wrote, "a resource was named, so a row is keyed");
            Ok(())
        })
        .expect("transaction");
    let row = read_budget(fixture.index().conn(), Some(account), "core")
        .expect("read")
        .expect("a row exists");
    assert_eq!(row.remaining(), None, "never 0 for an unsent header");
    assert_eq!(row.limit(), None, "and never the limit");
    assert_eq!(row.observed_at(), NOW);
    assert_eq!(may_spend(Some(&row), false, NOW), BudgetVerdict::Unknown);
}

/// §21.6: a response whose rate headers arrive **without** `x-ratelimit-resource` cannot be keyed,
/// so it is not mirrored at all — and that non-observation **must not move `observed_at`**.
#[test]
fn a_response_with_no_resource_header_writes_no_row_and_moves_no_clock() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut());

    fixture
        .index_mut()
        .with_tx(|tx| {
            mirror(
                tx,
                Some(account),
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-resource", "core"),
                        ("x-ratelimit-remaining", "4999"),
                    ],
                ),
                NOW,
            )?;
            Ok(())
        })
        .expect("first observation");

    fixture
        .index_mut()
        .with_tx(|tx| {
            let wrote = mirror(
                tx,
                Some(account),
                // Every rate header **except** the one that keys the row.
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-remaining", "10"),
                        ("x-ratelimit-limit", "5000"),
                    ],
                ),
                NOW + 3600,
            )?;
            assert!(!wrote, "there was nothing to key this observation by");
            Ok(())
        })
        .expect("second observation");

    let row = read_budget(fixture.index().conn(), Some(account), "core")
        .expect("read")
        .expect("the first row");
    assert_eq!(
        row.remaining(),
        Some(4999),
        "the unkeyable response overwrote nothing"
    );
    assert_eq!(
        row.observed_at(),
        NOW,
        "a call that observed nothing must not move the clock"
    );
    let rows: i64 = fixture
        .index()
        .conn()
        .query_row("SELECT count(*) FROM sync_budget", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(rows, 1, "no second row was invented");
}

/// §21.6's *before issuing* rule. With `remaining` known to be 0 and `now < reset_at`, the task
/// parks to `reset_at` — **the mirror is used, not merely recorded.**
#[test]
fn a_known_zero_budget_parks_to_the_reset_rather_than_issuing() {
    let reset = NOW + 900;
    let row = BudgetRow::observed(Some(0), Some(5000), Some(reset), NOW);
    assert_eq!(
        may_spend(Some(&row), false, NOW),
        BudgetVerdict::ParkUntil(reset)
    );
    assert_eq!(
        may_spend(Some(&row), true, NOW),
        BudgetVerdict::ParkUntil(reset),
        "an on-demand task cannot spend an allowance that is gone either"
    );
    assert_eq!(
        may_spend(Some(&row), false, reset),
        BudgetVerdict::Spend,
        "past the reset the mirror is stale and the request is what refreshes it"
    );
}

/// **AC-P2-21-14.** While `remaining` is **known** and below the reserve, scheduled tasks yield
/// and only on-demand tasks spend.
///
/// The whole argument for on-demand fetching is that the allowance belongs to what the user is
/// looking at; a scheduled listing that drains the last of it makes the opened page read `—`.
#[test]
fn the_reserve_yields_scheduled_work_and_admits_on_demand_work() {
    let reset = NOW + 900;
    assert_eq!(ON_DEMAND_RESERVE, 200);
    let scarce = BudgetRow::observed(Some(150), Some(5000), Some(reset), NOW);

    assert_eq!(
        may_spend(Some(&scarce), false, NOW),
        BudgetVerdict::Reserved(reset),
        "a scheduled task yields to the reserve, and says that is why"
    );
    assert_eq!(
        may_spend(Some(&scarce), true, NOW),
        BudgetVerdict::Spend,
        "the reserve exists so this one can spend"
    );

    // At the boundary the reserve is not yet in force: "below 200", not "at or below".
    let at_reserve = BudgetRow::observed(Some(ON_DEMAND_RESERVE), Some(5000), Some(reset), NOW);
    assert_eq!(
        may_spend(Some(&at_reserve), false, NOW),
        BudgetVerdict::Spend
    );

    // **Unknown is not below 200 — it is unknown, and it spends.**
    let unknown = BudgetRow::observed(None, None, None, NOW);
    for on_demand in [true, false] {
        assert_eq!(
            may_spend(Some(&unknown), on_demand, NOW),
            BudgetVerdict::Unknown
        );
    }
}

/// A reserve yield and an exhausted budget are **two verdicts**, because they settle the row with
/// two different reasons and a status reader has to tell them apart. Collapsing them would make
/// `reason` guess.
#[test]
fn a_reserve_yield_and_an_exhausted_budget_are_distinguishable() {
    let reset = NOW + 60;
    let empty = BudgetRow::observed(Some(0), Some(5000), Some(reset), NOW);
    let scarce = BudgetRow::observed(Some(1), Some(5000), Some(reset), NOW);
    assert_ne!(
        may_spend(Some(&empty), false, NOW),
        may_spend(Some(&scarce), false, NOW),
        "both park, and not for the same reason"
    );
}

/// §21.6's *including error responses*, all the way to the row. This is the one that decides
/// whether the app burns an account's allowance in a loop: a 403 that never updated the mirror
/// leaves the runner believing it still has budget.
#[test]
fn a_403s_headers_are_mirrored_exactly_as_a_200s_are() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut());
    let reset = NOW + 600;
    let reset_text = reset.to_string();
    let headers: Vec<(&str, &str)> = vec![
        ("x-ratelimit-resource", "core"),
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-limit", "5000"),
        ("x-ratelimit-reset", &reset_text),
    ];

    fixture
        .index_mut()
        .with_tx(|tx| {
            mirror(tx, Some(account), &snapshot(403, &headers), NOW)?;
            Ok(())
        })
        .expect("transaction");
    let from_403 = read_budget(fixture.index().conn(), Some(account), "core")
        .expect("read")
        .expect("row");

    fixture
        .index_mut()
        .with_tx(|tx| {
            mirror(tx, Some(account), &snapshot(200, &headers), NOW)?;
            Ok(())
        })
        .expect("transaction");
    let from_200 = read_budget(fixture.index().conn(), Some(account), "core")
        .expect("read")
        .expect("row");

    assert_eq!(from_403, from_200);
    assert_eq!(from_403.remaining(), Some(0));
    assert_eq!(from_403.reset_at(), Some(reset));
}

/// The per-IP pool is keyed by the **absence** of an account and is one row per resource, so a
/// later observation replaces it rather than adding to it.
#[test]
fn the_per_ip_pool_is_one_row_that_the_next_observation_replaces() {
    let mut fixture = TempIndex::new();
    let account = seed_account(fixture.index_mut());
    fixture
        .index_mut()
        .with_tx(|tx| {
            mirror(
                tx,
                None,
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-resource", "core"),
                        ("x-ratelimit-remaining", "59"),
                    ],
                ),
                NOW,
            )?;
            mirror(
                tx,
                None,
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-resource", "core"),
                        ("x-ratelimit-remaining", "58"),
                    ],
                ),
                NOW + 1,
            )?;
            // An account's pool on the same resource is a different pool.
            mirror(
                tx,
                Some(account),
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-resource", "core"),
                        ("x-ratelimit-remaining", "4999"),
                    ],
                ),
                NOW,
            )?;
            Ok(())
        })
        .expect("transaction");

    let anon = read_budget(fixture.index().conn(), None, "core")
        .expect("read")
        .expect("row");
    assert_eq!(anon.remaining(), Some(58), "the latest observation");
    assert_eq!(anon.observed_at(), NOW + 1);
    let owned = read_budget(fixture.index().conn(), Some(account), "core")
        .expect("read")
        .expect("row");
    assert_eq!(owned.remaining(), Some(4999), "a different pool entirely");

    let rows: i64 = fixture
        .index()
        .conn()
        .query_row("SELECT count(*) FROM sync_budget", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(rows, 2);
}

/// The decorator produces the snapshots this module writes, and the two meet at
/// `HttpObservation.rate` rather than at a second header parser.
#[test]
fn the_decorators_observation_is_what_reaches_the_row() {
    use codotheca_core::sync::http::ObservingTransport;
    use codotheca_core::testing::{FakeClock, FakeTransport};

    let inner = Arc::new(FakeTransport::new());
    inner.push(HttpResponse {
        status: 403,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "search"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-limit", "30"),
        ]),
        body: Vec::new(),
    });
    let observing = ObservingTransport::new(
        Arc::clone(&inner) as Arc<dyn HttpTransport>,
        Arc::new(FakeClock::new(NOW)),
    );
    let _ = observing.send(&codotheca_core::http::HttpRequest {
        method: "GET",
        url: "https://forge.example.invalid/search".to_owned(),
        headers: Vec::new(),
        body: None,
        limits: codotheca_core::http::ACCOUNT_LIMITS,
    });
    let observed = observing.drain();
    assert_eq!(observed.len(), 1);

    let mut fixture = TempIndex::new();
    fixture
        .index_mut()
        .with_tx(|tx| {
            let wrote = mirror(tx, None, &observed[0].rate, observed[0].at)?;
            assert!(wrote);
            Ok(())
        })
        .expect("transaction");
    let row = read_budget(fixture.index().conn(), None, "search")
        .expect("read")
        .expect("row");
    assert_eq!(row.remaining(), Some(0));
    assert_eq!(row.limit(), Some(30));
    assert_eq!(row.reset_at(), None, "the server named no reset");
}

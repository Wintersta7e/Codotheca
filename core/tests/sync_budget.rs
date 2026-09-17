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

use codotheca_core::accounts::keychain::TokenStore;
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

// ---------------------------------------------------------------------------------------------
// **AC-P2-21-14, end to end**: the reserve, through the runner.
// ---------------------------------------------------------------------------------------------

/// A fixture with one account, one bound project, and the runner over a scripted transport.
struct Lane {
    index: Arc<std::sync::Mutex<codotheca_core::index::Index>>,
    scripted: Arc<codotheca_core::testing::FakeTransport>,
    runner: Arc<codotheca_core::sync::runner::SyncRunner>,
    account: AccountId,
    project: codotheca_core::protocol::ProjectId,
    _dir: tempfile::TempDir,
}

/// Events are dropped: this file asserts requests and rows, and `core/tests/sync_runner.rs` owns
/// what the topic carries.
#[derive(Debug)]
struct Quiet;

impl codotheca_core::proto::EventSink for Quiet {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

fn lane() -> Lane {
    use codotheca_core::accounts::keychain::token_ref;
    const HOST: &str = "forge.example.invalid";
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = codotheca_core::index::Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let (account, project) = index
        .with_tx(|tx| {
            let account = insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: HOST.to_owned(),
                    login: "owner".to_owned(),
                    display_name: None,
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Private,
                    granted_scopes: vec!["repo".to_owned()],
                    token_ref: token_ref("github", HOST, "owner"),
                },
                NOW,
            )
            .expect("account");
            tx.execute(
                "INSERT INTO project (name, seed_basename, remote_key, provider,
                                      provider_repo_id, remote_link_basis, created_at, updated_at)
                 VALUES ('alpha', 'alpha', ?1, 'github', '7', 'provider_id', ?2, ?2)",
                rusqlite::params![format!("{HOST}/owner/alpha"), NOW],
            )?;
            Ok((
                account,
                codotheca_core::protocol::ProjectId(tx.last_insert_rowid()),
            ))
        })
        .expect("seed");

    let scripted = Arc::new(codotheca_core::testing::FakeTransport::new());
    let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
    let observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        Arc::clone(&scripted) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
    ));
    let tokens = Arc::new(codotheca_core::testing::FakeTokenStore::available());
    tokens
        .store(
            &token_ref("github", HOST, "owner"),
            &codotheca_core::accounts::keychain::SecretToken::new("t".to_owned()),
        )
        .expect("token");
    let provider = Arc::new(codotheca_core::provider::GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn HttpTransport>,
        HOST.to_owned(),
    ));
    let index = Arc::new(std::sync::Mutex::new(index));
    let runner = codotheca_core::sync::runner::SyncRunner::new(
        Arc::clone(&index),
        codotheca_core::sync::SyncDeps {
            provider,
            transport: observing,
            tokens,
            clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
            cancel: codotheca_core::cancel::CancelToken::new(),
        },
        Arc::new(Quiet) as Arc<dyn codotheca_core::proto::EventSink>,
    );

    Lane {
        index,
        scripted,
        runner,
        account,
        project,
        _dir: dir,
    }
}

/// Mirror one observation into the account's pool, so the runner has a budget to read.
fn seed_budget(lane: &Lane, remaining: i64, reset: i64) {
    let mut guard = lane.index.lock().expect("index");
    guard
        .with_tx(|tx| {
            mirror(
                tx,
                Some(lane.account),
                &snapshot(
                    200,
                    &[
                        ("x-ratelimit-resource", "core"),
                        ("x-ratelimit-remaining", &remaining.to_string()),
                        ("x-ratelimit-limit", "5000"),
                        ("x-ratelimit-reset", &reset.to_string()),
                    ],
                ),
                NOW,
            )?;
            Ok(())
        })
        .expect("seeded");
}

/// Run the loop until `expected` tasks have **reached the table and settled**.
///
/// **Both halves, and the first is not decoration.** `enqueue` records a task in memory and the
/// loop writes its row, so *"no queued or running rows"* is true before the loop has written
/// anything at all — a predicate that would let this return having run nothing and assert zero
/// requests as a pass. Waiting for the rows to exist first is what makes the settle mean a settle.
fn drain_queue(lane: &Lane, expected: i64) {
    lane.runner.start();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let (rows, pending): (i64, i64) = {
            let guard = lane.index.lock().expect("index");
            let conn = guard.conn();
            (
                conn.query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
                    .unwrap_or(0),
                conn.query_row(
                    "SELECT count(*) FROM sync_task_state WHERE state IN ('queued', 'running')",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(1),
            )
        };
        if rows >= expected && pending == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    lane.runner.request_stop();
    lane.runner.join();
    let rows: i64 = {
        let guard = lane.index.lock().expect("index");
        guard
            .conn()
            .query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
            .unwrap_or(0)
    };
    assert_eq!(
        rows, expected,
        "the loop did not run every task that was queued"
    );
}

/// **AC-P2-21-14.** With `remaining = 150` mirrored and both a scheduled and an on-demand task
/// queued, the transport records **exactly one** request and it is the on-demand one; the
/// scheduled row is `parked` with **both counters unchanged** and `reason = "reserve"`.
///
/// A reserve park is neither a failure nor a throttle, and `reason` is what lets a status reader
/// tell the three apart — three reserve parks counted as failures would strand a listing in
/// `deferred` for a budget that recovers on its own.
#[test]
fn a_scarce_budget_yields_the_listing_and_spends_on_the_opened_page() {
    let lane = lane();
    let reset = NOW + 900;
    seed_budget(&lane, 150, reset);
    // One answer, for the one request that is allowed through. A second request would exhaust the
    // script and show up as a transport error rather than passing unnoticed.
    //
    // **It carries the rate headers, and that is not decoration.** Every response re-mirrors the
    // pool, so an answer with no `x-ratelimit-remaining` would leave the budget *unknown* — and
    // unknown spends. The first version of this fixture omitted them and the listing went through
    // for exactly that reason, which is the mirror working rather than the reserve failing.
    let reset_text = reset.to_string();
    lane.scripted.push(HttpResponse {
        status: 404,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-remaining", "149"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-reset", reset_text.as_str()),
        ]),
        body: Vec::new(),
    });

    lane.runner
        .enqueue(codotheca_core::sync::task::SyncTask::AccountRepos {
            account_id: lane.account,
        });
    lane.runner
        .enqueue(codotheca_core::sync::task::SyncTask::ProjectRemote {
            project_id: lane.project,
        });
    drain_queue(&lane, 2);

    let requests = lane.scripted.request_count();
    eprintln!("sync_budget: {requests} request(s) issued under a 150-remaining budget");
    assert_eq!(requests, 1, "the scheduled listing must not have spent");
    let sent = lane.scripted.requests();
    assert!(
        sent[0].url.contains("/repos/"),
        "the one request was not the on-demand read: {}",
        sent[0].url
    );

    let guard = lane.index.lock().expect("index");
    let listing = codotheca_core::sync::store::load(
        guard.conn(),
        codotheca_core::protocol::SyncTaskKind::AccountRepos,
        Some(lane.account.0),
    )
    .expect("read")
    .expect("row");
    assert_eq!(
        listing.state,
        codotheca_core::protocol::SyncTaskState::Parked
    );
    assert_eq!(
        listing.not_before, reset,
        "parked to the budget's own reset"
    );
    assert_eq!(listing.fail_count, 0, "it did not fail");
    assert_eq!(
        listing.throttle_count, 0,
        "and the server did not throttle it"
    );
    assert_eq!(
        listing.reason.as_deref(),
        Some("reserve"),
        "a status reader cannot tell a reserve park from a rate limit without this"
    );
}

/// **Unknown is not below 200 — it is unknown, and it spends.** With no observation at all both
/// tasks proceed, which is what stops an unobserved budget stalling sync forever.
#[test]
fn an_unknown_budget_lets_both_kinds_of_task_through() {
    let lane = lane();
    for _ in 0..4 {
        lane.scripted.push(HttpResponse {
            status: 404,
            headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
            body: Vec::new(),
        });
    }
    lane.runner
        .enqueue(codotheca_core::sync::task::SyncTask::AccountRepos {
            account_id: lane.account,
        });
    lane.runner
        .enqueue(codotheca_core::sync::task::SyncTask::ProjectRemote {
            project_id: lane.project,
        });
    drain_queue(&lane, 2);

    let sent = lane.scripted.requests();
    eprintln!(
        "sync_budget: {} request(s) under an unknown budget",
        sent.len()
    );
    assert!(
        sent.len() >= 2,
        "an unobserved budget stalled sync: {} request(s)",
        sent.len()
    );
    assert!(
        sent.iter().any(|r| r.url.contains("/user/repos")),
        "the scheduled listing was held back by a budget nobody observed"
    );
}

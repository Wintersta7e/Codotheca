#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The runner: **one thread, one request, one stop order**.
//!
//! Covers **AC-P2-21-12** (a `running` row left by a crash is re-queued, not surfaced) and the
//! core half of **AC-P2-21-11** (a monotone listing count with no synthesised denominator).
//!
//! **R72 and R104 both bite here.** Every wait below spins on the state it is waiting for, with a
//! deadline, rather than sleeping a fixed time and hoping: a test that hopes to win a race is a
//! test that reports the machine it ran on. Every test in this file was run **at least three
//! times** before it was reported green.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::accounts::keychain::{token_ref, SecretToken, TokenStore};
use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::assembly::sync::SyncPump;
use codotheca_core::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use codotheca_core::index::Index;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    AccountId, AuthKind, ProjectId, ScopeTier, SyncTaskKind, SyncTaskState,
};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::budget::mirror;
use codotheca_core::sync::classify::classify;
use codotheca_core::sync::events::SyncLive;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::state::{SyncTaskStateRow, SYNC_UNNAMED_PARK_SECS};
use codotheca_core::sync::store::{load, put};
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";
/// Generous on purpose: every wait spins on a condition, so a slow machine takes longer and a
/// broken one still fails. A deadline that a correct build can miss is R72's race wearing a
/// number.
const DEADLINE: Duration = Duration::from_secs(10);

/// Every event the runner published, in order.
#[derive(Debug, Default)]
struct Recorder {
    seen: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl Recorder {
    fn events(&self, event: &str) -> Vec<serde_json::Value> {
        self.seen
            .lock()
            .expect("recorder")
            .iter()
            .filter(|(topic, name, _)| topic == "sync" && name == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }
}

impl EventSink for Recorder {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.seen
            .lock()
            .expect("recorder")
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

/// A transport that records how many calls are inside `send` **at the same moment**, and can be
/// told to dwell there.
///
/// It is the artefact AC-P2-21-12's neighbour needs: *"one request in flight"* asserted by
/// measuring concurrency rather than by counting threads, which would assert the design instead
/// of the behaviour.
#[derive(Debug)]
struct ConcurrencyProbe {
    inner: Arc<dyn HttpTransport>,
    live: AtomicUsize,
    max: AtomicUsize,
    dwell: Duration,
}

impl ConcurrencyProbe {
    fn new(inner: Arc<dyn HttpTransport>, dwell: Duration) -> Arc<Self> {
        Arc::new(ConcurrencyProbe {
            inner,
            live: AtomicUsize::new(0),
            max: AtomicUsize::new(0),
            dwell,
        })
    }
}

impl HttpTransport for ConcurrencyProbe {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(self.dwell);
        let answer = self.inner.send(req);
        self.live.fetch_sub(1, Ordering::SeqCst);
        answer
    }
}

struct Fixture {
    index: Arc<Mutex<Index>>,
    scripted: Arc<FakeTransport>,
    probe: Arc<ConcurrencyProbe>,
    events: Arc<Recorder>,
    clock: Arc<FakeClock>,
    deps: SyncDeps,
    account: AccountId,
    project: ProjectId,
    _dir: tempfile::TempDir,
}

fn fixture(dwell: Duration) -> Fixture {
    fixture_over(dwell, None)
}

/// The same lane over a transport of the caller's choosing.
///
/// **A scripted transport runs out**, and a loop with no floor under its park would then be
/// measuring the script's length rather than the loop's behaviour — so the two tests that measure
/// a retry loop hand in a forge that answers for ever.
fn fixture_over(dwell: Duration, forge: Option<Arc<dyn HttpTransport>>) -> Fixture {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = Index::open_at(dir.path(), NOW).expect("index");
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
            Ok((account, ProjectId(tx.last_insert_rowid())))
        })
        .expect("seed");

    let scripted = Arc::new(FakeTransport::new());
    let probe = ConcurrencyProbe::new(
        forge.unwrap_or_else(|| Arc::clone(&scripted) as Arc<dyn HttpTransport>),
        dwell,
    );
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&probe) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
    ));
    let tokens = Arc::new(FakeTokenStore::available());
    tokens
        .store(
            &token_ref("github", HOST, "owner"),
            &SecretToken::new("t".to_owned()),
        )
        .expect("token");
    let provider = Arc::new(GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn HttpTransport>,
        HOST.to_owned(),
    ));

    Fixture {
        index: Arc::new(Mutex::new(index)),
        scripted,
        probe,
        events: Arc::new(Recorder::default()),
        clock: Arc::clone(&clock),
        deps: SyncDeps {
            provider,
            transport: observing,
            tokens,
            clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
            cancel: codotheca_core::cancel::CancelToken::new(),
            // UTC in a test, so a local date never depends on the machine running it.
            tz_offset_min: 0,
        },
        account,
        project,
        _dir: dir,
    }
}

/// Spin until `check` holds, or fail with what it last saw. **Never a bare sleep**: a fixed wait
/// is a race that reports the machine it ran on (R72).
fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

fn ok_page(body: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: body.as_bytes().to_vec(),
    }
}

fn state_of(index: &Mutex<Index>, kind: SyncTaskKind, key: i64) -> Option<SyncTaskStateRow> {
    let guard = index.lock().expect("index");
    load(guard.conn(), kind, Some(key)).ok().flatten()
}

/// **AC-P2-21-12, the sweep itself.** A `running` row left behind by a crash is re-queued at
/// start-up with **both counters unchanged**: an interruption is not a failure and must not count
/// as one, or three crashes would defer a task that never failed once.
///
/// Asserted on `requeue_running` rather than after the loop has run, because the loop then settles
/// the task and a successful settle resets both counters — the assertion would pass for the wrong
/// reason. R99's split: each half asserts the strongest thing that is true of it.
#[test]
fn the_startup_sweep_requeues_a_running_row_and_moves_neither_counter() {
    let f = fixture(Duration::ZERO);
    let mut crashed = SyncTaskStateRow::queued(SyncTaskKind::RenameProbe, Some(f.account.0), NOW);
    crashed.state = SyncTaskState::Running;
    crashed.fail_count = 2;
    crashed.throttle_count = 1;
    crashed.not_before = NOW + 999;
    {
        let mut guard = f.index.lock().expect("index");
        guard.with_tx(|tx| put(tx, &crashed)).expect("seed");
        let moved = guard
            .with_tx(|tx| codotheca_core::sync::store::requeue_running(tx, NOW))
            .expect("swept");
        eprintln!("sync_runner: the sweep re-queued {moved} row(s)");
        assert_eq!(moved, 1, "a sweep that swept nothing cannot report success");
    }

    let after = state_of(&f.index, SyncTaskKind::RenameProbe, f.account.0).expect("row");
    assert_eq!(after.state, SyncTaskState::Queued);
    assert_eq!(
        after.not_before, 0,
        "runnable at once: it was interrupted, not parked"
    );
    assert_eq!(after.fail_count, 2, "an interruption is not a failure");
    assert_eq!(after.throttle_count, 1, "and it is not a throttle");
}

/// **AC-P2-21-12, the surface.** No sync task is surfaced as a failure for having been
/// interrupted: the pump sweeps the row and runs it, and nothing settles `deferred` or `blocked`.
#[test]
fn an_interrupted_task_is_never_surfaced_as_a_failure() {
    let f = fixture(Duration::ZERO);
    let mut crashed = SyncTaskStateRow::queued(SyncTaskKind::RenameProbe, Some(f.account.0), NOW);
    crashed.state = SyncTaskState::Running;
    {
        let mut guard = f.index.lock().expect("index");
        guard.with_tx(|tx| put(tx, &crashed)).expect("seed");
    }
    // Nothing scripted: the probe, once re-queued, finds no unmatched key and asks nothing.

    let index = Arc::clone(&f.index);
    let events = Arc::clone(&f.events);
    let account = f.account.0;
    let pump = SyncPump::start(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    until("the crashed row to settle", || {
        state_of(&index, SyncTaskKind::RenameProbe, account)
            .is_some_and(|row| row.state == SyncTaskState::Ok)
    });
    pump.stop();

    let failures: Vec<serde_json::Value> = events
        .events("settled")
        .into_iter()
        .filter(|p| p["state"] == "deferred" || p["state"] == "blocked")
        .collect();
    assert!(
        failures.is_empty(),
        "an interrupted task was surfaced as a failure: {failures:?}"
    );
}

/// **At most one request is in flight in the process**, measured rather than assumed.
///
/// The probe counts concurrent entries into `send` and keeps the maximum, so this asserts the
/// behaviour the invariant names instead of asserting that one thread was spawned.
#[test]
fn at_most_one_request_is_in_flight_and_at_most_one_row_is_running() {
    let f = fixture(Duration::from_millis(15));
    for _ in 0..6 {
        f.scripted.push(ok_page("[]"));
    }
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.enqueue(SyncTask::RenameProbe {
        account_id: f.account,
    });
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });

    let running_seen = Arc::new(AtomicUsize::new(0));
    let watch = {
        let index = Arc::clone(&f.index);
        let running_seen = Arc::clone(&running_seen);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let n: i64 = {
                    let guard = index.lock().expect("index");
                    guard
                        .conn()
                        .query_row(
                            "SELECT count(*) FROM sync_task_state WHERE state = 'running'",
                            [],
                            |r| r.get(0),
                        )
                        .unwrap_or(0)
                };
                running_seen.fetch_max(usize::try_from(n).unwrap_or(0), Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        (stop, handle)
    };

    runner.start();
    // The rows must **exist** before "nothing is pending" means anything: `enqueue` records a
    // task in memory and the loop writes its row.
    until("all three tasks to reach the table and settle", || {
        let guard = f.index.lock().expect("index");
        let conn = guard.conn();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
            .unwrap_or(0);
        let pending: i64 = conn
            .query_row(
                "SELECT count(*) FROM sync_task_state WHERE state IN ('queued', 'running')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1);
        rows >= 3 && pending == 0
    });
    runner.request_stop();
    runner.join();
    watch.0.store(true, Ordering::SeqCst);
    let _ = watch.1.join();

    let max = f.probe.max.load(Ordering::SeqCst);
    eprintln!(
        "sync_runner: max concurrent requests {max}, max rows running {}",
        running_seen.load(Ordering::SeqCst)
    );
    assert!(max <= 1, "{max} requests were in flight at once");
    assert!(
        running_seen.load(Ordering::SeqCst) <= 1,
        "two rows were `running` at once"
    );
}

/// `stop()` returns within a bounded wait while the transport is dwelling in a response, and a
/// **second** `stop()` is harmless.
///
/// The bound is not instant and must not be claimed as such: cancelling takes down no HTTP
/// request — there is no process to kill — so the ceiling is the per-request budget. What is
/// asserted is that the join happens rather than hanging, and that the second call is a no-op.
#[test]
fn stop_is_bounded_and_a_second_stop_is_harmless() {
    let f = fixture(Duration::from_millis(80));
    for _ in 0..4 {
        f.scripted.push(ok_page("[]"));
    }
    let pump = SyncPump::start(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    // Force the condition rather than wait for it: enqueue, then spin until a request is actually
    // in flight, so `stop()` is provably called against a dwelling transport.
    pump.sink_ref().on_project_visible(f.project);
    until("a request to be in flight", || {
        f.probe.live.load(Ordering::SeqCst) > 0
    });

    let started = Instant::now();
    pump.stop();
    let first = started.elapsed();
    pump.stop();
    let both = started.elapsed();
    eprintln!("sync_runner: stop() returned in {first:?}, twice in {both:?}");
    assert!(first < DEADLINE, "stop() did not join: {first:?}");
    assert!(both < DEADLINE, "the second stop() blocked: {both:?}");
}

/// §21.4: **a parked row returns to `queued` by the clock alone.** No event announces it and none
/// is emitted — a parked row that becomes runnable has not *settled*.
#[test]
fn a_park_whose_clock_has_come_is_picked_up_with_no_external_trigger() {
    let f = fixture(Duration::ZERO);
    f.scripted.push(ok_page("[]"));
    let mut parked = SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(f.account.0), NOW);
    parked.state = SyncTaskState::Parked;
    parked.not_before = NOW + 600;
    {
        let mut guard = f.index.lock().expect("index");
        guard.with_tx(|tx| put(tx, &parked)).expect("seed");
    }

    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let pump = SyncPump::start(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    // It must **not** run while the clock says not yet.
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        state_of(&index, SyncTaskKind::AccountRepos, account).map(|r| r.state),
        Some(SyncTaskState::Parked),
        "a park released early is a park that means nothing"
    );
    assert!(f.events.events("started").is_empty());

    // The clock comes round, and nothing else happens: no enqueue, no signal.
    f.clock.set_unix(NOW + 601);
    until("the park to be picked up", || {
        !f.events.events("started").is_empty()
    });
    pump.stop();
    // Counted by kind: the listing's terminal `Done` queues §22.7's rename probe, so *"how many
    // tasks started"* is no longer the same question as *"how many times did the clock release
    // this one"*.
    let listings = f
        .events
        .events("started")
        .into_iter()
        .filter(|event| event["kind"] == "account_repos")
        .count();
    assert_eq!(listings, 1);
}

/// §21.5: **an on-demand task queued behind a scheduled one runs first.** The allowance belongs
/// to what the user is looking at, and the ordering has to agree with the reserve about which
/// task that is.
#[test]
fn an_on_demand_task_queued_behind_a_scheduled_one_runs_first() {
    let f = fixture(Duration::ZERO);
    for _ in 0..4 {
        f.scripted.push(ok_page("[]"));
    }
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });
    runner.start();
    until("both tasks to start", || {
        f.events.events("started").len() >= 2
    });
    runner.request_stop();
    runner.join();

    let started = f.events.events("started");
    assert_eq!(
        started[0]["kind"], "project_remote",
        "the scheduled listing was queued first and must still yield"
    );
}

/// **AC-P2-21-11, the core half.** Two `listing_progress` events over a listing whose second page
/// reports fewer entries still carry a **non-decreasing** `listed`, and a listing with no observed
/// total carries `total: null` on every event — never a synthesised one.
#[test]
fn listing_progress_never_retreats_and_never_invents_a_denominator() {
    let f = fixture(Duration::ZERO);
    let repo = |id: u64, name: &str| {
        format!(
            r#"{{"id":{id},"clone_url":"https://{HOST}/owner/{name}.git",
                "owner":{{"login":"owner","type":"User"}},"name":"{name}",
                "fork":false,"archived":false,"private":false,
                "permissions":{{"push":true}}}}"#
        )
    };
    let next = format!("https://{HOST}/user/repos?page=2");
    f.scripted.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("link", &*format!(r#"<{next}>; rel="next""#)),
        ]),
        body: format!("[{},{}]", repo(1, "a"), repo(2, "b")).into_bytes(),
    });
    // Page two reports **one** entry where page one reported two.
    f.scripted.push(ok_page(&format!("[{}]", repo(3, "c"))));

    // Queued through `enqueue`, which is the production path: writing the row with `put` behind
    // the runner's back would assert against a door the product does not use.
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    until("two progress events", || {
        f.events.events("listing_progress").len() >= 2
    });
    runner.request_stop();
    runner.join();

    let progress = f.events.events("listing_progress");
    let listed: Vec<i64> = progress
        .iter()
        .map(|p| p["listed"].as_i64().expect("a count"))
        .collect();
    eprintln!("sync_runner: listing progress {listed:?}");
    for pair in listed.windows(2) {
        assert!(pair[1] >= pair[0], "progress retreated: {listed:?}");
    }
    for event in &progress {
        assert!(
            event["total"].is_null(),
            "a denominator nothing observed was rendered as a fact: {event}"
        );
        assert!(
            !event.to_string().contains('%'),
            "a percentage over an unknown denominator: {event}"
        );
    }
}

/// A snapshot with no runner is *measured, none*: `SyncLive::default()` carries no listing, no
/// notice and no observed outcome, which is what a process that has just started actually knows.
#[test]
fn a_fresh_process_knows_nothing_and_says_so() {
    let live = SyncLive::default();
    assert_eq!(live.listing, None);
    assert_eq!(live.notice, None);
    assert!(live.last.is_empty());
}

/// A `TokenStore` that tries the index guard **from inside `read`**, on the thread that asked.
///
/// Retried to a deadline rather than tried once, for the reason `assembly_dispatch.rs`'s probe
/// records: `std::sync::Mutex` is not reentrant, so a guard held by the *calling* path can never
/// be taken here however long this waits, while another thread holding it for a moment is
/// ordinary — and this test's own main thread reads the table while it waits.
#[derive(Debug)]
struct LockProbingTokens {
    index: Arc<Mutex<Index>>,
    inner: Arc<FakeTokenStore>,
    /// **Sticky, and never "the last call was fine".** The listing's read and the rename probe's
    /// read both land here, and a flag the last writer wins would let a correct second call erase
    /// a first one that held the guard — which is exactly how the first version of this bar passed
    /// against the defect it was written for.
    lock_was_held: std::sync::atomic::AtomicBool,
    calls: AtomicUsize,
}

impl codotheca_core::accounts::keychain::TokenStore for LockProbingTokens {
    fn probe(&self) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
        self.inner.probe()
    }
    fn store(
        &self,
        entry: &str,
        token: &SecretToken,
    ) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
        self.inner.store(entry, token)
    }
    fn delete(&self, entry: &str) -> Result<(), codotheca_core::accounts::keychain::KeychainError> {
        self.inner.delete(entry)
    }
    fn read(
        &self,
        entry: &str,
    ) -> Result<SecretToken, codotheca_core::accounts::keychain::KeychainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut free = false;
        while Instant::now() < deadline {
            if self.index.try_lock().is_ok() {
                free = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if !free {
            self.lock_was_held.store(true, Ordering::SeqCst);
        }
        self.inner.read(entry)
    }
}

/// **R75, on the sync worker's own path.** A listing must not hold the process's one index guard
/// while it reads a secret.
///
/// Reading a secret is a call into the OS credential store, and the guard it would be holding is
/// the mutex every command needs. `rename.rs` and `remote.rs` were already this shape and
/// `repos.rs` was not — invisible for as long as nothing in the product ever ran a listing, which
/// is what made this lane's R90 gap expensive rather than merely untidy.
#[test]
fn a_listing_holds_no_index_lock_while_it_reads_the_keychain() {
    let f = fixture(Duration::ZERO);
    for _ in 0..4 {
        f.scripted.push(ok_page("[]"));
    }
    let inner = Arc::new(FakeTokenStore::available());
    inner
        .store(
            &token_ref("github", HOST, "owner"),
            &SecretToken::new("t".to_owned()),
        )
        .expect("token");
    let probe = Arc::new(LockProbingTokens {
        index: Arc::clone(&f.index),
        inner,
        lock_was_held: std::sync::atomic::AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });

    let mut deps = f.deps;
    deps.tokens = Arc::clone(&probe) as Arc<dyn codotheca_core::accounts::keychain::TokenStore>;
    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    until("the listing to settle", || {
        state_of(&index, SyncTaskKind::AccountRepos, account)
            .is_some_and(|row| row.state == SyncTaskState::Ok)
    });
    runner.request_stop();
    runner.join();

    // No escape hatch: the keychain MUST have been reached, or this proves nothing.
    assert!(
        probe.calls.load(Ordering::SeqCst) > 0,
        "the listing never read a secret, so the guard was never at risk"
    );
    assert!(
        !probe.lock_was_held.load(Ordering::SeqCst),
        "a sync task read the keychain with the index lock held"
    );
}

/// A forge that answers the same thing for ever, and counts what it was asked.
#[derive(Debug)]
struct AlwaysAnswers {
    response: HttpResponse,
    calls: AtomicUsize,
}

impl HttpTransport for AlwaysAnswers {
    fn send(&self, _req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

/// **A throttle that names no instant parks; it does not retry.**
///
/// §21's goal sentence: *"a throttled or unauthorised forge produces one non-modal banner and **no
/// retry loop**."* A `429` carrying neither `retry-after` nor `x-ratelimit-reset` classifies as
/// `Throttled { until: now }` — the classifier saying *the server named no instant*, not *retry
/// now* — and `run_loop` re-picks a runnable row with no sleep between iterations. Before
/// `apply_outcome` floored the park, this exact shape was measured at **2,413 requests in 500 ms**
/// against a forge already answering `429`.
///
/// **What makes this bar different from the nine above it: it lets the loop run.**
/// `sync_classify.rs` asserts the classification and stops there, and `sync_budget.rs`'s
/// `drain_queue` waits for the queue to empty — so a test written that way against this defect
/// would hang rather than fail. Here the assertion is on requests issued *after* the row parked,
/// which is a number no run length can flatter.
#[test]
fn a_throttle_naming_no_instant_parks_once_instead_of_retrying() {
    let forge = Arc::new(AlwaysAnswers {
        response: HttpResponse {
            status: 429,
            headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
            body: Vec::new(),
        },
        calls: AtomicUsize::new(0),
    });
    let f = fixture_over(
        Duration::ZERO,
        Some(Arc::clone(&forge) as Arc<dyn HttpTransport>),
    );
    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    until("the listing to park", || {
        state_of(&index, SyncTaskKind::AccountRepos, account)
            .is_some_and(|row| row.state == SyncTaskState::Parked)
    });

    // **The window is the measurement, not a race** (R72): a correct build issues nothing in it
    // whatever its length, and a longer one only makes a loop look worse.
    let at_park = forge.calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    let after = forge.calls.load(Ordering::SeqCst);
    runner.request_stop();
    runner.join();

    eprintln!("sync_runner: {at_park} request(s) to reach the park, {after} after 300 ms");
    assert!(
        at_park >= 1,
        "the forge was never asked, so nothing was measured"
    );
    assert_eq!(
        after, at_park,
        "a forge that has just said stop was asked again while its task was parked"
    );

    let row = state_of(&index, SyncTaskKind::AccountRepos, account).expect("row");
    assert_eq!(
        row.not_before,
        NOW + SYNC_UNNAMED_PARK_SECS,
        "a park that releases at the instant it was made is not a park"
    );
    assert_eq!(row.reason.as_deref(), Some("rate_limited"));
}

/// **A reserve with no observed `reset_at` parks; it does not cycle.**
///
/// `may_spend` answers `Reserved(reset_at.unwrap_or(now))`, which names no instant either. This
/// path issues **no** request, so nothing about it is visible at a forge: what it costs is the
/// process's one index mutex, taken twice per cycle, and three protocol events per cycle — an
/// idle-looking app starving every command. Measured at **3,334 cycles in 500 ms** before the
/// floor.
#[test]
fn a_reserve_with_no_observed_reset_parks_once_instead_of_cycling() {
    let f = fixture(Duration::ZERO);
    // A known `remaining` below the reserve and **no** `x-ratelimit-reset`: the row exists, the
    // instant does not. Mirrored through the real classifier, because the parse is part of it.
    {
        let observed: Result<HttpResponse, TransportError> = Ok(HttpResponse {
            status: 200,
            headers: codotheca_core::http::normalise_headers([
                ("x-ratelimit-resource", "core"),
                ("x-ratelimit-remaining", "150"),
                ("x-ratelimit-limit", "5000"),
            ]),
            body: b"{}".to_vec(),
        });
        let rate = classify(&observed, NOW).1;
        assert_eq!(
            rate.reset_at, None,
            "the fixture named an instant after all"
        );
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                mirror(tx, Some(f.account), &rate, NOW)?;
                Ok(())
            })
            .expect("mirror");
    }

    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let events = Arc::clone(&f.events);
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    // **Wait for the settle, not for the row.** `settle` writes the row and *then* emits, so a
    // waiter watching the row can snapshot the event count at zero and then see that same settle
    // land inside the measurement window — which reads as a loop. Measured before this line
    // existed: red in two runs of three, and green in every single run (R104).
    until("the scheduled listing to yield to the reserve", || {
        !events.events("settled").is_empty()
    });
    assert_eq!(
        state_of(&index, SyncTaskKind::AccountRepos, account).map(|row| row.state),
        Some(SyncTaskState::Parked),
        "the reserve did not park the scheduled listing"
    );

    let at_park = events.events("settled").len();
    std::thread::sleep(Duration::from_millis(300));
    let after = events.events("settled").len();
    runner.request_stop();
    runner.join();

    eprintln!("sync_runner: {at_park} settle(s) to reach the reserve park, {after} after 300 ms");
    assert_eq!(
        after, at_park,
        "a reserved task was re-picked and re-settled in a loop"
    );
    assert_eq!(
        f.scripted.request_count(),
        0,
        "a task held back by the reserve must not spend"
    );

    let row = state_of(&index, SyncTaskKind::AccountRepos, account).expect("row");
    assert_eq!(row.reason.as_deref(), Some("reserve"));
    assert_eq!(
        row.not_before,
        NOW + SYNC_UNNAMED_PARK_SECS,
        "a reserve that releases at the instant it was made holds nothing back"
    );
}

/// **R90 — the shipped binary queues a listing, with nothing here enqueueing one.**
///
/// The bar every other test in this file misses. Each of them calls `enqueue` itself, so every
/// one of them passes against a build in which no production path ever asks for a listing — which
/// is what this lane shipped until an independent review traced the call graph and found it
/// terminating. `AC-P2-21-9-binary` could not see it either: it asserts the real binary answers
/// `sync.status` with empty arrays and two nulls, which is exactly what a permanently inert
/// runner answers.
///
/// The only thing driven here is `SyncPump::start`, which is what `core/src/main.rs` runs.
#[test]
fn a_connected_account_is_listed_with_nothing_enqueueing_it() {
    let f = fixture(Duration::ZERO);
    for _ in 0..4 {
        f.scripted.push(ok_page("[]"));
    }
    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let pump = SyncPump::start(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    until("the listing nothing asked for to settle", || {
        state_of(&index, SyncTaskKind::AccountRepos, account)
            .is_some_and(|row| row.state == SyncTaskState::Ok)
    });
    // §22.7's trigger, and the second half of the same gap: the probe is queued by the listing's
    // terminal `Done`, so `repair_renames` has a reachable caller rather than a written one.
    until("the rename probe the completed listing queues", || {
        state_of(&index, SyncTaskKind::RenameProbe, account).is_some()
    });
    pump.stop();

    let sent = f.scripted.requests();
    let urls: Vec<&str> = sent.iter().map(|r| r.url.as_str()).collect();
    eprintln!("sync_runner: a start with nothing enqueued issued {urls:?}");
    assert!(
        urls.iter().any(|url| url.contains("/user/repos")),
        "no listing was read, so the schedule reached no forge: {urls:?}"
    );
}

/// **A banner that was raised is taken down**, and the wire is what could not say so.
///
/// `live.notice` was already cleared by a clean settle and the core's own state was right the
/// whole time — but `notice` is a bare enum on the wire, so the cleared case had no event, and the
/// renderer sets its banner from a `notice` event or from a snapshot and from nothing else. A
/// `throttled` banner raised at T therefore stayed on screen for the life of the session after
/// sync recovered, unless the user dismissed it.
#[test]
fn a_clean_settle_publishes_the_status_that_carries_the_cleared_banner() {
    let f = fixture(Duration::ZERO);
    let reset = NOW + 600;
    f.scripted.push(HttpResponse {
        status: 429,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-reset", &reset.to_string()),
        ]),
        body: Vec::new(),
    });
    for _ in 0..3 {
        f.scripted.push(ok_page("[]"));
    }

    let events = Arc::clone(&f.events);
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    until("the throttle to raise a banner", || {
        !events.events("notice").is_empty()
    });
    assert!(
        events.events("snapshot").is_empty(),
        "a status was published before anything had been cleared"
    );

    // The park's clock comes round and the listing succeeds.
    f.clock.set_unix(reset + 1);
    until("the status that carries the cleared banner", || {
        !events.events("snapshot").is_empty()
    });
    runner.request_stop();
    runner.join();

    let published = events.events("snapshot");
    eprintln!(
        "sync_runner: {} status event(s) after recovery",
        published.len()
    );
    assert!(
        published[0]["notice"].is_null(),
        "the status still names a banner the core has cleared: {}",
        published[0]
    );
    assert_eq!(
        events.events("notice").len(),
        1,
        "a clean settle must not raise a second banner"
    );
}

/// **§21.6's *every response*, including one another thread made.**
///
/// The Device Flow pump shares this decorator — `core/src/main.rs` hands the same provider to
/// both — so a poll's rate headers land in the same channel as a task's. Before the channel was
/// tagged by thread, `observe_one` took the last observation and dropped the rest, so a poll that
/// landed during a task step was either mistaken for that step's own response or thrown away
/// unmirrored. It is mirrored now, and against the **per-IP** pool: an unauthenticated poll
/// spends no account's allowance, and §21.6 keys that pool by the absence of an account.
#[test]
fn a_response_another_thread_observed_is_mirrored_against_the_per_ip_pool() {
    let f = fixture(Duration::ZERO);
    let observing = Arc::clone(&f.deps.transport);

    // The poll's answer, observed on another thread before the listing runs.
    f.scripted.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-remaining", "4321"),
            ("x-ratelimit-limit", "5000"),
        ]),
        body: b"{}".to_vec(),
    });
    {
        let other = Arc::clone(&observing);
        std::thread::spawn(move || {
            let _ = other.send(&HttpRequest {
                method: "POST",
                url: format!("https://{HOST}/login/oauth/access_token"),
                headers: Vec::new(),
                body: None,
                limits: codotheca_core::http::ACCOUNT_LIMITS,
            });
        })
        .join()
        .expect("joined");
    }
    for _ in 0..4 {
        f.scripted.push(ok_page("[]"));
    }

    let index = Arc::clone(&f.index);
    let account = f.account.0;
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::AccountRepos {
        account_id: f.account,
    });
    runner.start();
    until("the listing to settle", || {
        state_of(&index, SyncTaskKind::AccountRepos, account)
            .is_some_and(|row| row.state == SyncTaskState::Ok)
    });
    runner.request_stop();
    runner.join();

    let guard = index.lock().expect("index");
    let pool = codotheca_core::sync::budget::read_budget(guard.conn(), None, "core")
        .expect("read")
        .expect("the per-IP pool was never mirrored");
    eprintln!(
        "sync_runner: per-IP pool remaining {:?} of {:?}",
        pool.remaining(),
        pool.limit()
    );
    assert_eq!(pool.remaining(), Some(4321));
    assert_eq!(pool.limit(), Some(5000));
}

/// **Cancelling alone stops the loop**, with no `request_stop` at all.
///
/// This is what the cancel half of `stop()` actually contributes, and the plan's own mutation
/// proof for the stop *order* does not bite: `JobPump`'s cancel takes down a git process tree, so
/// a worker parked in a long read returns at once, but **there is no process to kill for an HTTP
/// request**. Its bound is `crate::http::ACCOUNT_LIMITS.total_secs`, and a `request_stop` alone
/// joins in the same time. So the order is kept for the shape it shares with `JobPump` while the
/// separable property — that the token the loop reads is the one the deps carry — is asserted
/// here instead of asserted by a revert that stays green.
#[test]
fn cancelling_the_deps_token_stops_the_loop_without_a_request_stop() {
    let f = fixture(Duration::ZERO);
    let cancel = f.deps.cancel.clone();
    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.start();

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&done);
    let joiner = {
        let runner = Arc::clone(&runner);
        std::thread::spawn(move || {
            runner.join();
            flag.store(true, Ordering::SeqCst);
        })
    };
    cancel.cancel();
    until("the loop to exit on the cancel alone", || {
        done.load(Ordering::SeqCst)
    });
    let _ = joiner.join();
}

/// **R145's second call site, proved live.** `ci_red`'s input arrives on a **sync**, not on a
/// job: an evaluator hooked to `JobRunner::settle` alone holds §28's previous answer until some
/// unrelated job settles that project — the one-settle lag R145 was raised about.
///
/// Nothing here runs a job. A `ProjectRemote` sync settles, and the item exists afterwards.
#[test]
fn a_project_remote_sync_settle_opens_ci_red_with_no_job_involved() {
    let f = fixture(Duration::from_millis(0));
    // The listing and the repo read both answer emptily; what matters is that the task settles.
    for _ in 0..8 {
        f.scripted.push(ok_page("{}"));
    }

    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                // A primary copy on `main`, which is the branch R145's fallback reads.
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind, branch)
                     VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', 'present', 'worktree',
                             'main')",
                    [f.project.0],
                )?;
                // The latest concluded run on that branch failed.
                tx.execute(
                    "INSERT INTO remote_ci_run
                        (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                         run_number, started_at)
                     VALUES ('github', '7', 1, 'build', 'failure', 'main', 1, 100)",
                    [],
                )?;
                Ok(())
            })
            .expect("seed");
    }

    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });
    runner.start();

    until("the sync settle to open ci_red", || {
        let guard = f.index.lock().expect("index");
        let n: i64 = guard
            .conn()
            .query_row(
                "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'ci_red'",
                [f.project.0],
                |r| r.get(0),
            )
            .unwrap_or(0);
        n == 1
    });
}

/// **[p3] §31.5's hook site 2, proved live — the half no job-shaped trigger could ever satisfy.**
///
/// `description` reads the forge's own row, and two more of §31's ten checks come from sync or
/// from a scheduled sweep. An evaluator hooked to `JobRunner::settle` alone would leave a forge
/// description unread until some unrelated job settled that project.
///
/// **Nothing here runs a job.** No `project_check` row exists before the sync, ten exist after,
/// and the one this test is about reads `pass`.
#[test]
fn a_project_remote_sync_settle_writes_the_ten_completion_rows_with_no_job_involved() {
    let f = fixture(Duration::from_millis(0));
    for _ in 0..8 {
        f.scripted.push(ok_page("{}"));
    }

    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                // Past §31.8's gates: authored, not a reference, one present copy that has been
                // looked at.
                tx.execute(
                    "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
                    [f.project.0],
                )?;
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind, branch,
                                           refstate_observed_at)
                     VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', 'present', 'worktree',
                             'main', 100)",
                    [f.project.0],
                )?;
                // The forge row a sync writes, with a description and one topic.
                tx.execute(
                    "INSERT INTO remote_repo (provider, provider_repo_id, description, observed_at)
                     VALUES ('github', '7', 'a shaped description', 100)",
                    [],
                )?;
                tx.execute(
                    "INSERT INTO remote_topic (provider, provider_repo_id, topic)
                     VALUES ('github', '7', 'alpha')",
                    [],
                )?;
                Ok(())
            })
            .expect("seed");
    }

    let before: i64 = {
        let guard = f.index.lock().expect("index");
        guard
            .conn()
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1",
                [f.project.0],
                |r| r.get(0),
            )
            .unwrap_or(-1)
    };
    assert_eq!(
        before, 0,
        "the evaluator has not run, so there is nothing to read"
    );

    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });
    runner.start();

    until("the sync settle to write the ten completion rows", || {
        let guard = f.index.lock().expect("index");
        let n: i64 = guard
            .conn()
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1",
                [f.project.0],
                |r| r.get(0),
            )
            .unwrap_or(0);
        n == 10
    });

    let guard = f.index.lock().expect("index");
    let state: String = guard
        .conn()
        .query_row(
            "SELECT state FROM project_check WHERE project_id = ?1 AND check_key = 'description'",
            [f.project.0],
            |r| r.get(0),
        )
        .expect("the description row");
    assert_eq!(
        state, "pass",
        "a forge description and a topic, read at the settle that could have changed them"
    );
}

/// **[p3] `AC-P3-34-8`'s sync half — a scheduled sweep writes `background`.** §34.2's third
/// production caller: `ci_red`'s input arrives on a sync, so the transition that closes it is
/// observed by `SyncRunner::settle` and no job is involved. A failing run opens the item (a first
/// observation, no row); a later passing run closes it, and that settle writes one `cracks` row
/// whose provenance is the sweep's, never the user's attention.
#[test]
fn ac_p3_34_8_a_project_remote_sync_settle_writes_a_background_delta() {
    let f = fixture(Duration::from_millis(0));
    for _ in 0..16 {
        f.scripted.push(ok_page("{}"));
    }

    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                // An enrolled, authored project — a reading exists to move.
                tx.execute(
                    "UPDATE project SET authored_by_user = 1, is_reference = 0,
                                        acknowledged_at = ?2
                      WHERE id = ?1",
                    rusqlite::params![f.project.0, NOW - 100],
                )?;
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind, branch)
                     VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', 'present', 'worktree',
                             'main')",
                    [f.project.0],
                )?;
                tx.execute(
                    "INSERT INTO remote_ci_run
                        (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                         run_number, started_at)
                     VALUES ('github', '7', 1, 'build', 'failure', 'main', 1, 100)",
                    [],
                )?;
                Ok(())
            })
            .expect("seed");
    }

    let runner = SyncRunner::new(
        Arc::clone(&f.index),
        f.deps,
        Arc::clone(&f.events) as Arc<dyn EventSink>,
    );
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });
    runner.start();

    let count = |sql: &str| -> i64 {
        let guard = f.index.lock().expect("index");
        guard
            .conn()
            .query_row(sql, [f.project.0], |r| r.get(0))
            .unwrap_or(0)
    };
    until("the first sync settle to open ci_red", || {
        count("SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'ci_red'") == 1
    });
    assert_eq!(
        count("SELECT count(*) FROM health_delta WHERE project_id = ?1"),
        0,
        "a first observation wrote a row"
    );

    // The next run on the branch passed.
    {
        let guard = f.index.lock().expect("index");
        guard
            .conn()
            .execute(
                "INSERT INTO remote_ci_run
                    (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                     run_number, started_at)
                 VALUES ('github', '7', 2, 'build', 'success', 'main', 2, 200)",
                [],
            )
            .expect("a passing run");
    }
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: f.project,
    });
    until("the second sync settle to write a health delta", || {
        count("SELECT count(*) FROM health_delta WHERE project_id = ?1") == 1
    });

    let guard = f.index.lock().expect("index");
    let row: (String, f64, f64, String) = guard
        .conn()
        .query_row(
            "SELECT layer, from_value, to_value, detected_in FROM health_delta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("the row");
    drop(guard);
    eprintln!("sync settle: {row:?}");
    assert_eq!(
        row,
        ("cracks".to_owned(), 1.0, 0.0, "background".to_owned())
    );
    until("the delta to be announced after its commit", || {
        f.events
            .seen
            .lock()
            .expect("recorder")
            .iter()
            .any(|(topic, event, _)| topic == "projects" && event == "health_delta")
    });
}

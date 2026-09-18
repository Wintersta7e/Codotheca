#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §27.7's third confirmed defect: **a key-less sync task row is silently unrunnable and keeps the
//! runner awake for ever.**
//!
//! `sync_task_state.key` has been nullable since `0011_sync.sql`, `SyncTaskStateRow.key` is
//! `Option<i64>`, and `put`/`load` both take one — the persistence layer was finished. The *work
//! item* was not: `task_of` opened `let key = row.key?;` and is reached only through the pick's
//! `filter_map`, so a NULL-key row was filtered out before it could be claimed. It was never
//! claimed, never settled and raised **no error, no panic and no log line**, while
//! `any_outstanding` counted it by `state` alone and kept the loop polling a task it could not
//! pick. A correct migration and a correct scheduler would have done nothing at all, in a build
//! whose every suite was green.
//!
//! **R72 and R104 bite here**: every wait spins on the state it is waiting for with a deadline.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::accounts::keychain::{token_ref, SecretToken, TokenStore};
use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::index::Index;
use codotheca_core::protocol::{AccountId, AuthKind, ScopeTier, SyncTaskKind, SyncTaskState};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::state::SyncTaskStateRow;
use codotheca_core::sync::store::{delete_account_tasks, load, put};
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::{is_on_demand, SyncDeps};
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";
const DEADLINE: Duration = Duration::from_secs(10);

/// Everything the runner emits, dropped on the floor: this file asserts stored state, and a
/// recorder would be a second thing to keep in agreement with it.
#[derive(Debug)]
struct Quiet;

impl codotheca_core::proto::EventSink for Quiet {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

struct Fixture {
    index: Arc<Mutex<Index>>,
    deps: Option<SyncDeps>,
    account: AccountId,
    _dir: tempfile::TempDir,
}

impl Fixture {
    /// `SyncDeps` is not `Clone` — it holds a keychain and a provider — so the fixture hands it
    /// over once, to the one runner a test starts.
    fn take_deps(&mut self) -> SyncDeps {
        self.deps.take().expect("deps taken once")
    }
}

fn fixture() -> Fixture {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let account = index
        .with_tx(|tx| {
            Ok(insert_account(
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
            .expect("account"))
        })
        .expect("seed");

    let scripted = Arc::new(FakeTransport::new());
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&scripted) as Arc<dyn codotheca_core::http::HttpTransport>,
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
        Arc::clone(&observing) as Arc<dyn codotheca_core::http::HttpTransport>,
        HOST.to_owned(),
    ));

    Fixture {
        index: Arc::new(Mutex::new(index)),
        deps: Some(SyncDeps {
            provider,
            transport: observing,
            tokens,
            clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
            cancel: codotheca_core::cancel::CancelToken::new(),
            tz_offset_min: 0,
        }),
        account,
        _dir: dir,
    }
}

/// Spin until `check` holds, or fail with what it was waiting for. **Never a bare sleep** (R72).
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

fn state_of(index: &Mutex<Index>, kind: SyncTaskKind, key: Option<i64>) -> Option<SyncTaskState> {
    let guard = index.lock().expect("index");
    load(guard.conn(), kind, key)
        .ok()
        .flatten()
        .map(|r| r.state)
}

fn row_count(index: &Mutex<Index>) -> i64 {
    let guard = index.lock().expect("index");
    guard
        .conn()
        .query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
        .expect("count")
}

/// **AC-P3-32-2.** A `queued` row with `key IS NULL` is **claimed, run and settled**.
///
/// Against the shipped `task_of` this fails with the row still `queued`: no error, no panic and
/// no log line, which is the whole of the defect.
#[test]
fn a_key_less_task_row_is_picked_run_and_settled() {
    let mut fixture = fixture();
    {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
                )
            })
            .expect("seed the key-less row");
    }
    assert_eq!(
        state_of(&fixture.index, SyncTaskKind::Advisories, None),
        Some(SyncTaskState::Queued)
    );

    let runner = SyncRunner::new(
        Arc::clone(&fixture.index),
        fixture.take_deps(),
        Arc::new(Quiet),
    );
    runner.start();
    until("the key-less row to settle", || {
        matches!(
            state_of(&fixture.index, SyncTaskKind::Advisories, None),
            Some(SyncTaskState::Ok)
        )
    });
    runner.request_stop();
    runner.join();

    // It settled as the **key-less** row and not as some keyed one that happens to share the
    // slug: read the column, not the `load` helper that was given `None` to look with.
    let stored: (Option<i64>, String) = {
        let guard = fixture.index.lock().expect("index");
        guard
            .conn()
            .query_row(
                "SELECT key, state FROM sync_task_state WHERE task = 'advisories'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("exactly one advisories row")
    };
    assert_eq!(stored, (None, "ok".to_owned()));
}

/// The symptom a reader sees first and misreads as a busy runner: a row the pick cannot claim
/// keeps `outstanding` true for ever, because the wakefulness predicate counted by `state` alone.
///
/// **The unpickable row here is a kind/key-shape mismatch**, which is reachable: the column's
/// CHECK admits `advisories` with a key, and `task_of` refuses to guess which id it would be.
#[test]
fn a_row_the_pick_cannot_claim_does_not_keep_the_loop_awake() {
    let fixture = fixture();
    {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                // A process-wide kind carrying a key, and a keyed kind carrying none. Neither
                // shape is resolvable and neither may be guessed at.
                tx.execute(
                    "INSERT INTO sync_task_state (task, key, state, at)
                     VALUES ('advisories', 5, 'queued', ?1), ('account_repos', NULL, 'queued', ?1)",
                    [NOW],
                )?;
                Ok(())
            })
            .expect("seed the unresolvable rows");
    }

    let outstanding = {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(codotheca_core::sync::runner::any_outstanding)
            .expect("read")
    };
    assert!(
        !outstanding,
        "a row the pick can never claim must not keep the loop awake"
    );

    // And an empty table is not outstanding either — the zero case, stated so the assertion above
    // cannot pass by the predicate simply always answering false.
    {
        let mut guard = fixture.index.lock().expect("index");
        let before = guard
            .with_tx(|tx| {
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
                )?;
                codotheca_core::sync::runner::any_outstanding(tx)
            })
            .expect("read");
        assert!(before, "a claimable queued row IS outstanding");
    }
}

/// `delete_account_tasks` already filters by task as well as by key, and `key = NULL` is never
/// true in SQL — so the advisory row was safe from it **by accident**. This makes it design.
///
/// Disconnecting every account leaves the sweep running, which is correct: it never needed one.
#[test]
fn disconnecting_an_account_leaves_the_process_wide_task_alone() {
    let fixture = fixture();
    let account = fixture.account;
    {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(account.0), NOW),
                )?;
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::RenameProbe, Some(account.0), NOW),
                )?;
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
                )
            })
            .expect("seed");
    }

    let before = row_count(&fixture.index);
    let removed = {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(|tx| delete_account_tasks(tx, account))
            .expect("delete")
    };
    let after = row_count(&fixture.index);
    eprintln!("advisory_task_row: {before} rows before, {removed} removed, {after} after");
    assert_eq!(before, 3);
    assert_eq!(removed, 2, "a delete that removed nothing is not a pass");
    assert_eq!(after, 1);
    assert_eq!(
        state_of(&fixture.index, SyncTaskKind::Advisories, None),
        Some(SyncTaskState::Queued)
    );
}

/// Seed a settled sweep that named `resource`, and an exhausted pool under that name.
fn seed_sweep_and_pool(fixture: &Fixture, resource: Option<&str>, remaining: i64, reset: i64) {
    let mut guard = fixture.index.lock().expect("index");
    guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO advisory_sweep (started_at, settled_at, outcome, resource, complete)
                 VALUES (?1, ?1, 'done', ?2, 1)",
                rusqlite::params![NOW, resource],
            )?;
            if let Some(resource) = resource {
                tx.execute(
                    "INSERT INTO sync_budget
                       (account_id, resource, remaining, limit_, reset_at, observed_at)
                     VALUES (NULL, ?1, ?2, 60, ?3, ?4)",
                    rusqlite::params![resource, remaining, reset, NOW],
                )?;
            }
            Ok(())
        })
        .expect("seed the sweep and its pool");
}

fn queue_sweep(fixture: &Fixture) {
    let mut guard = fixture.index.lock().expect("index");
    guard
        .with_tx(|tx| {
            put(
                tx,
                &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
            )
        })
        .expect("queue the sweep");
}

fn run_until_settled(fixture: &mut Fixture) -> SyncTaskState {
    let runner = SyncRunner::new(
        Arc::clone(&fixture.index),
        fixture.take_deps(),
        Arc::new(Quiet),
    );
    runner.start();
    until("the sweep to leave queued", || {
        !matches!(
            state_of(&fixture.index, SyncTaskKind::Advisories, None),
            Some(SyncTaskState::Queued | SyncTaskState::Running) | None
        )
    });
    runner.request_stop();
    runner.join();
    state_of(&fixture.index, SyncTaskKind::Advisories, None).expect("a settled row")
}

/// **AC-P3-32-3.** The pre-issue budget read is keyed by the resource **the sweep's own last
/// response named**, not by a process-wide constant.
///
/// With the endpoint answering from a pool this build did not guess, a constant key looks up a
/// pool the sweep never writes: `may_spend` answers `Unknown` for ever, `Unknown` spends, and
/// every request issues **with no brake at all** until the source refuses.
#[test]
fn the_sweep_reads_the_pool_its_own_response_named() {
    let mut fixture = fixture();
    seed_sweep_and_pool(&fixture, Some("graphql"), 0, NOW + 900);
    queue_sweep(&fixture);
    assert_eq!(run_until_settled(&mut fixture), SyncTaskState::Parked);
}

/// Until one response has named a resource, the fallback is `core`, `may_spend` answers `Unknown`
/// and the task proceeds. **One request, self-correcting** — which is the whole of what a
/// constant may be relied on for.
#[test]
fn an_unnamed_pool_spends_once_rather_than_parking_for_ever() {
    let mut fixture = fixture();
    // A sweep that settled but named no resource: nothing was mirrored, so there is no pool and
    // no brake, and a synthetic key would invent one.
    seed_sweep_and_pool(&fixture, None, 0, NOW + 900);
    assert_eq!(
        {
            let guard = fixture.index.lock().expect("index");
            codotheca_core::advisories::store::last_settled_resource(guard.conn()).expect("read")
        },
        None,
        "a sweep that named no resource leaves the column NULL"
    );
    queue_sweep(&fixture);
    assert_eq!(run_until_settled(&mut fixture), SyncTaskState::Ok);
}

/// The sweep draws on the **NULL-account per-IP** pool, always. A per-account row at zero for the
/// same resource must not brake it: that is somebody else's allowance.
#[test]
fn the_sweep_reads_the_per_ip_pool_and_never_an_accounts() {
    let mut fixture = fixture();
    let account = fixture.account;
    seed_sweep_and_pool(&fixture, Some("graphql"), 5000, NOW + 900);
    {
        let mut guard = fixture.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO sync_budget
                       (account_id, resource, remaining, limit_, reset_at, observed_at)
                     VALUES (?1, 'graphql', 0, 5000, ?2, ?3)",
                    rusqlite::params![account.0, NOW + 900, NOW],
                )?;
                Ok(())
            })
            .expect("seed the account pool");
    }
    queue_sweep(&fixture);
    assert_eq!(run_until_settled(&mut fixture), SyncTaskState::Ok);
}

/// The sweep is **scheduled**, not on-demand: §21.5's allowance belongs to what the user is
/// looking at. `is_on_demand` compiles unchanged for the new variant, which is exactly why it
/// needs an assertion rather than an assumption.
#[test]
fn the_sweep_is_scheduled_and_carries_no_key() {
    assert!(!is_on_demand(&SyncTask::Advisories));
    assert_eq!(SyncTask::Advisories.key(), None);
    assert_eq!(
        SyncTask::AccountRepos {
            account_id: AccountId(3)
        }
        .key(),
        Some(3)
    );
    assert_eq!(
        SyncTask::RenameProbe {
            account_id: AccountId(4)
        }
        .key(),
        Some(4)
    );
    assert_eq!(
        SyncTask::ProjectRemote {
            project_id: codotheca_core::protocol::ProjectId(5)
        }
        .key(),
        Some(5)
    );
    assert_eq!(SyncTask::Advisories.kind(), SyncTaskKind::Advisories);
}

//! **R121** — a debt-set change reaches both surfaces through the existing `projects.upserted`,
//! gated on **two** conjuncts: the projected row actually changed, **and** the job's chain
//! originated from a user act rather than from the walk.
//!
//! **A change gate alone is not safe.** On a first scan the projected row genuinely changes on
//! almost every settle — J1 writes ref state, J1.5 writes `authored_by_user` and `is_reference`,
//! J2 writes `is_dirty`, J3 writes the inventory, J4 writes the commit clocks, J7 writes the debt
//! set — so a change-gated emit is still six or seven whole-row events per project across a first
//! run. That is the firehose phase 1 refused, arriving through the gate meant to prevent it.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::FakeClock;
use support::TestRepo;

/// **The clock is frozen**, so a second run of the same job writes the same observation times —
/// which is what makes *"the row did not change"* a state this test can actually reach. With a
/// system clock every settle moves `refstate_observed_at` and the first conjunct is untestable.
const T0: i64 = 1_700_000_000;

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

impl RecordingSink {
    fn count(&self, topic: &str, event: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .count()
    }

    fn total(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    fn payloads(&self, topic: &str, event: &str) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, p)| p.clone())
            .collect()
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    runner: Arc<JobRunner>,
    events: Arc<RecordingSink>,
    project: ProjectId,
    location: LocationId,
}

fn rig(repo: &TestRepo) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), 0).unwrap();
    let index = Arc::new(Mutex::new(index));

    let (project, location) = {
        let guard = index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('p', 'p', 0, 0)",
            [],
        )
        .unwrap();
        let project = ProjectId(conn.last_insert_rowid());
        let path = repo.path().to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
            rusqlite::params![project.0, path.as_bytes(), path],
        )
        .unwrap();
        (project, LocationId(conn.last_insert_rowid()))
    };

    let clock = Arc::new(FakeClock::new(T0));
    let events = Arc::new(RecordingSink::default());
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        )),
        clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    let runner = JobRunner::new(
        Arc::clone(&index),
        deps,
        Arc::clone(&events) as Arc<dyn EventSink>,
    );

    Rig {
        _dir: dir,
        index,
        runner,
        events,
        project,
        location,
    }
}

fn job(rig: &Rig, kind: JobKind, priority: Priority, origin: JobOrigin) -> Job {
    Job {
        kind,
        project_id: rig.project,
        location_id: rig.location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority,
        not_before: 0,
        origin,
    }
}

/// Wait until the runner is idle — nothing in flight and nothing due — and fail at the deadline.
///
/// A quiet window on the sink is not enough: on a slow machine one git call outlasts it, and the
/// rest of the chain lands in the next measurement. A worker settles, and so emits and enqueues
/// the follow-ups, before it releases its slot, which is what makes idleness the end of a chain.
fn quiesce(runner: &JobRunner, events: &RecordingSink, deadline: Duration) -> usize {
    let start = Instant::now();
    while start.elapsed() < deadline && !runner.is_idle() {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        runner.is_idle(),
        "the chain was still running after {deadline:?}"
    );
    events.total()
}

fn upserted_row(payload: &serde_json::Value) -> &serde_json::Value {
    &payload["row"]
}

/// A seeded repository, because a chain over an empty one settles almost nothing.
fn seeded() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    repo
}

/// **Conjunct 2, and the one that decides the volume question.** A walk-origin chain publishes
/// nothing even though the row demonstrably changed — the bulk first-scan case stays covered by
/// the existing `scan/finished` refresh.
#[test]
fn a_walk_originated_settle_emits_no_upserted_even_when_the_row_changes() {
    let repo = seeded();
    let rig = rig(&repo);

    let before = {
        let guard = rig.index.lock().unwrap();
        let payload = codotheca_core::detail::upserted_payload(guard.conn(), rig.project);
        drop(guard);
        payload.expect("a seeded project projects a row")
    };

    rig.runner.start(2);
    rig.runner.enqueue(job(
        &rig,
        JobKind::J1Refstate,
        Priority::RefState,
        JobOrigin::Walk,
    ));
    let total = quiesce(&rig.runner, &rig.events, Duration::from_secs(40));
    rig.runner.request_stop();
    rig.runner.join();

    let after = {
        let guard = rig.index.lock().unwrap();
        let payload = codotheca_core::detail::upserted_payload(guard.conn(), rig.project);
        drop(guard);
        payload.expect("a seeded project projects a row")
    };

    let settles = rig.events.count("scan", "job_done");
    let upserted = rig.events.count("projects", "upserted");
    eprintln!("walk chain: {settles} settles, {upserted} projects.upserted, {total} events total");
    assert!(
        settles > 0,
        "the walk chain settled nothing, so this proves nothing"
    );
    assert_ne!(
        upserted_row(&after),
        upserted_row(&before),
        "the row did not change, so a zero below would prove nothing"
    );
    assert_eq!(upserted, 0, "a walk chain published a row");
}

/// **Conjunct 1 wired, and the payoff path.** An interactive chain publishes the changed row, and
/// what it publishes is the whole `ProjectRow` — including §30.11's `healthSummary`, which is how
/// a debt-set change reaches an open page.
#[test]
fn a_settle_that_changes_the_row_on_an_interactive_chain_emits_projects_upserted() {
    let repo = seeded();
    let rig = rig(&repo);

    rig.runner.start(2);
    rig.runner.enqueue(job(
        &rig,
        JobKind::J1Refstate,
        Priority::RefState,
        JobOrigin::Interactive,
    ));
    let total = quiesce(&rig.runner, &rig.events, Duration::from_secs(40));
    rig.runner.request_stop();
    rig.runner.join();

    let settles = rig.events.count("scan", "job_done");
    let payloads = rig.events.payloads("projects", "upserted");
    eprintln!(
        "interactive chain: {settles} settles, {} projects.upserted, {total} events total",
        payloads.len()
    );
    assert!(settles > 0, "the chain settled nothing");
    assert!(
        !payloads.is_empty(),
        "an interactive chain that changed the row published nothing"
    );
    let row = upserted_row(&payloads[0]);
    assert_eq!(row["id"], serde_json::json!(rig.project.0));
    // The whole row, which is what makes this the debt-set change's route to both surfaces.
    assert!(
        row.get("healthSummary").is_some(),
        "the payload does not carry the health summary: {row}"
    );
    assert!(row.get("lifecycle").is_some());
}

/// **Conjunct 1.** The same chain run a second time against an unchanged repository and a frozen
/// clock settles the same jobs and publishes nothing: the gate compares the projected row, not
/// the fact that a settle happened.
#[test]
fn a_settle_that_changes_nothing_emits_nothing() {
    let repo = seeded();
    let rig = rig(&repo);

    rig.runner.start(2);
    rig.runner.enqueue(job(
        &rig,
        JobKind::J1Refstate,
        Priority::RefState,
        JobOrigin::Interactive,
    ));
    quiesce(&rig.runner, &rig.events, Duration::from_secs(40));
    let settles_first = rig.events.count("scan", "job_done");
    let upserted_first = rig.events.count("projects", "upserted");
    assert!(settles_first > 0, "the first chain settled nothing");
    assert!(upserted_first > 0, "the first chain published nothing");

    // The same work again, over a repository nobody touched, with the clock where it was.
    rig.runner.enqueue(job(
        &rig,
        JobKind::J1Refstate,
        Priority::RefState,
        JobOrigin::Interactive,
    ));
    quiesce(&rig.runner, &rig.events, Duration::from_secs(40));
    rig.runner.request_stop();
    rig.runner.join();

    let settles_second = rig.events.count("scan", "job_done") - settles_first;
    let upserted_second = rig.events.count("projects", "upserted") - upserted_first;
    eprintln!(
        "first pass: {settles_first} settles / {upserted_first} upserted; \
         second pass: {settles_second} settles / {upserted_second} upserted"
    );
    assert!(
        settles_second > 0,
        "the second pass settled nothing, so a zero below would prove nothing"
    );
    assert_eq!(
        upserted_second, 0,
        "a settle that changed no field published a row"
    );
}

/// The gate's comparison, stated as what it is: the payload is a pure projection of the stored
/// row, so *"did this settle change anything a surface draws"* is whole-row equality and **never
/// a hand-maintained list of fields a settle can move** — a list like that goes stale on the
/// first plan that adds a field.
#[test]
fn the_gate_compares_the_whole_row_and_not_a_list_of_fields() {
    let repo = seeded();
    let rig = rig(&repo);
    let guard = rig.index.lock().unwrap();
    let conn = guard.conn();

    let a = codotheca_core::detail::upserted_payload(conn, rig.project).unwrap();
    let b = codotheca_core::detail::upserted_payload(conn, rig.project).unwrap();
    assert_eq!(a, b, "the payload is not a pure projection of the row");

    // A field no settle-specific list would think to include, moved by hand.
    conn.execute(
        "UPDATE project SET is_pinned = 1 WHERE id = ?1",
        [rig.project.0],
    )
    .unwrap();
    let c = codotheca_core::detail::upserted_payload(conn, rig.project).unwrap();
    assert_ne!(a, c, "a changed field did not move the payload");
    drop(guard);
}

//! The scheduler, driven for real.
//!
//! `next_jobs_after` is pure and unit-tested beside the code. This file exercises the half that
//! **R39 is about**: a worker thread holding the index, running a job against a real repository,
//! and writing the result. Plan 09's own tests never crossed a thread boundary with the runner,
//! which is exactly why the `Arc<Index>` signature compiled there and could not have worked.

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
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobSink, JobState, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use support::TestRepo;

/// Records every event the runner publishes, so the `scan/job_done` frame is checked rather
/// than assumed.
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
    fn seen(&self) -> Vec<(String, String, serde_json::Value)> {
        self.events.lock().unwrap().clone()
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

    let events = Arc::new(RecordingSink::default());
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::new(SystemClock::new()),
        )),
        clock: Arc::new(SystemClock::new()),
        cancel: CancelToken::new(),
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

fn job(rig: &Rig, kind: JobKind, priority: Priority) -> Job {
    Job {
        kind,
        project_id: rig.project,
        location_id: rig.location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority,
        not_before: 0,
    }
}

/// Poll until `f` holds or the deadline passes. The pool is asynchronous by construction.
fn wait_for(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if f() {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn job_state(rig: &Rig, kind: JobKind) -> Option<JobState> {
    let guard = rig.index.lock().unwrap();
    let rows = codotheca_core::jobs::state::load(guard.conn(), rig.project).unwrap();
    drop(guard);
    rows.into_iter().find(|r| r.job == kind).map(|r| r.state)
}

/// **R39.** A worker thread runs a real job against a real repository and writes the result.
/// This is the test plan 09 did not have, and its absence is why the impossible signature
/// survived review.
#[test]
fn a_worker_thread_runs_a_job_and_persists_what_it_read() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");

    let rig = rig(&repo);
    rig.runner.start(1);
    assert!(rig
        .runner
        .enqueue(job(&rig, JobKind::J1Refstate, Priority::RefState)));

    let done = wait_for(Duration::from_secs(20), || {
        job_state(&rig, JobKind::J1Refstate) == Some(JobState::Done)
    });
    rig.runner.request_stop();
    rig.runner.join();
    assert!(done, "J1 never reached the done state");

    let guard = rig.index.lock().unwrap();
    let (branch, basis, observed): (Option<String>, Option<String>, Option<i64>) = guard
        .conn()
        .query_row(
            "SELECT branch, refstate_basis, refstate_observed_at FROM location WHERE id = ?1",
            [rig.location.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    drop(guard);
    assert_eq!(branch.as_deref(), Some("main"));
    assert_eq!(basis.map(|b| b.len()), Some(64));
    assert!(observed.is_some());
}

/// A finished J1 chains J1.5, and the event that says so reaches the sink.
#[test]
fn a_finished_job_chains_the_next_one_and_publishes_job_done() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");

    let rig = rig(&repo);
    rig.runner.start(2);
    rig.runner
        .enqueue(job(&rig, JobKind::J1Refstate, Priority::RefState));

    let chained = wait_for(Duration::from_secs(30), || {
        job_state(&rig, JobKind::J15Authorship) == Some(JobState::Done)
    });
    rig.runner.request_stop();
    rig.runner.join();
    assert!(chained, "J1.5 was never queued or never finished");

    let seen = rig.events.seen();
    assert!(
        seen.iter().any(|(topic, event, payload)| {
            topic == "scan"
                && event == "job_done"
                && payload.get("job").and_then(serde_json::Value::as_str) == Some("j1")
        }),
        "no scan/job_done frame for j1 in {seen:?}"
    );
}

/// The scanner's hand-off queues J1 for a newly indexed location — the production `JobSink`
/// implementation, not a fake of one.
#[test]
fn the_scan_hand_off_queues_ref_state_for_a_new_location() {
    let repo = TestRepo::init();
    let rig = rig(&repo);
    rig.runner
        .on_location_indexed(rig.project, rig.location, "store", StoreClass::Local);
    assert!(
        !rig.runner
            .enqueue(job(&rig, JobKind::J1Refstate, Priority::RefState)),
        "an equal-priority J1 for the same location is already queued"
    );
}

/// §6: a visible tile always re-observes. It is not gated on a fingerprint, and it jumps the
/// queue rather than waiting behind background work.
#[test]
fn a_visible_tile_queues_status_at_interactive_priority() {
    let repo = TestRepo::init();
    let rig = rig(&repo);
    rig.runner
        .enqueue(job(&rig, JobKind::J2Status, Priority::Deferred));
    rig.runner
        .on_visible(rig.project, rig.location, "store", StoreClass::Local);
    // Re-pushing at anything worse is refused, which is how the queued entry's priority is
    // observable from outside: `Standard` is better than the `Deferred` first push and would
    // have been accepted had `on_visible` not already raised it to `Interactive`.
    assert!(
        !rig.runner
            .enqueue(job(&rig, JobKind::J2Status, Priority::Standard)),
        "on_visible must have raised the queued J2 to Interactive"
    );
}

/// A repository that is not there fails hard rather than retrying forever.
#[test]
fn a_missing_repository_fails_rather_than_looping() {
    let repo = TestRepo::init();
    let rig = rig(&repo);
    {
        let guard = rig.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE location SET path_bytes = ?2, path_display = ?3 WHERE id = ?1",
                rusqlite::params![rig.location.0, b"/no/such/path".to_vec(), "/no/such/path"],
            )
            .unwrap();
    }

    rig.runner.start(1);
    rig.runner
        .enqueue(job(&rig, JobKind::J3Inventory, Priority::Standard));
    let settled = wait_for(Duration::from_secs(20), || {
        matches!(
            job_state(&rig, JobKind::J3Inventory),
            Some(JobState::Failed | JobState::DeferredSlow)
        )
    });
    rig.runner.request_stop();
    rig.runner.join();
    assert!(settled, "a missing repository never settled");
}

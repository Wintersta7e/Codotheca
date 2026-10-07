//! §46.9's current-state gate, driven through a real `JobRunner` against real repositories: no
//! job runs for a removed copy or for any copy of a removed project, and the check is made at
//! dispatch, so a job queued before the removal does not run after it. J5 art reads no repository
//! and is the one kind the gate lets through.
//!
//! Every test also runs a job for a live copy of another project in the same runner, so the
//! harness demonstrably sees a run and the absence of one means something.
//!
//! The page and the Peek of a removed copy, or of a removed project, answer from what is stored
//! and ask for no reading at all; a live copy's page in the same rig asks once.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

#[path = "support/detail_rig.rs"]
mod detail_rig;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::detail::get::handle_project_get;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::sync::runner::NullSyncSink;
use serde_json::json;
use support::TestRepo;

/// Records every event the runner publishes, so `scan/job_done` is read rather than assumed.
#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

/// One project with one copy, backed by a real repository holding one commit.
struct Copy {
    repo: TestRepo,
    project: ProjectId,
    location: LocationId,
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    runner: Arc<JobRunner>,
    events: Arc<RecordingSink>,
    /// The copy each test removes, or whose project it removes.
    gone: Copy,
    /// A copy of another project that nothing removes.
    live: Copy,
}

fn committed_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    repo
}

fn plant(index: &Mutex<Index>, repo: TestRepo, name: &str) -> Copy {
    let guard = index.lock().unwrap();
    let conn = guard.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 0, 0)",
        [name],
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
    let location = LocationId(conn.last_insert_rowid());
    drop(guard);
    Copy {
        repo,
        project,
        location,
    }
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), 0).unwrap()));
    let gone = plant(&index, committed_repo(), "gone");
    let live = plant(&index, committed_repo(), "live");

    let events = Arc::new(RecordingSink::default());
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(gone.repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::new(SystemClock::new()),
        )),
        clock: Arc::new(SystemClock::new()),
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    let runner = JobRunner::new(Arc::clone(&index), deps, events.clone());
    Rig {
        _dir: dir,
        index,
        runner,
        events,
        gone,
        live,
    }
}

fn job(copy: &Copy, kind: JobKind) -> Job {
    Job {
        kind,
        project_id: copy.project,
        location_id: copy.location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::RefState,
        not_before: 0,
        origin: JobOrigin::Walk,
    }
}

/// `locations.uninstall`'s own row write: `removed_at` and the columns it clears. `presence`
/// stays `present`, which is the shape the removal leaves.
fn uninstall_row(conn: &rusqlite::Connection, location: LocationId) {
    let clears = codotheca_core::uninstall::command::cleared_columns()
        .iter()
        .map(|column| format!("{column} = NULL"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!("UPDATE location SET removed_at = 5, {clears} WHERE id = ?1"),
        [location.0],
    )
    .unwrap();
}

fn uninstall(rig: &Rig, location: LocationId) {
    let guard = rig.index.lock().unwrap();
    uninstall_row(guard.conn(), location);
    drop(guard);
}

/// The user declares the whole project removed; its copy's row is untouched.
fn remove_project(rig: &Rig, project: ProjectId) {
    let guard = rig.index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE project SET removed_at = 5 WHERE id = ?1",
            [project.0],
        )
        .unwrap();
    drop(guard);
}

/// The `scan/job_done` frames published for `project`, as job slugs.
fn done_for(rig: &Rig, project: ProjectId) -> Vec<String> {
    rig.events
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(topic, event, payload)| {
            topic == "scan" && event == "job_done" && payload["projectId"] == project.0
        })
        .map(|(_, _, payload)| payload["job"].as_str().unwrap().to_owned())
        .collect()
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

/// Starts the pool and waits until the live copy's J1 has been reported and nothing is due, so
/// the gone copy's job has been taken off the queue one way or the other.
fn run_to_idle(rig: &Rig) {
    rig.runner.start(1);
    let settled = wait_for(Duration::from_secs(60), || {
        done_for(rig, rig.live.project).iter().any(|j| j == "j1") && rig.runner.is_idle()
    });
    rig.runner.request_stop();
    rig.runner.join();
    assert!(
        settled,
        "the live copy's J1 never ran, so the harness saw nothing"
    );
}

/// Nothing ran for the gone copy: no `job_done`, no job state, and J1's own column unwritten.
fn assert_nothing_ran(rig: &Rig) {
    let done = done_for(rig, rig.gone.project);
    let live = done_for(rig, rig.live.project);
    eprintln!("job_done frames: gone copy {done:?}, live copy {live:?}");
    assert!(
        live.iter().any(|j| j == "j1"),
        "the live copy's J1 is not in the events"
    );
    assert_eq!(
        done,
        Vec::<String>::new(),
        "a job ran for a removed copy: job_done {done:?}"
    );
    let guard = rig.index.lock().unwrap();
    let conn = guard.conn();
    let states: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_job_state WHERE project_id = ?1",
            [rig.gone.project.0],
            |r| r.get(0),
        )
        .unwrap();
    let observed: Option<i64> = conn
        .query_row(
            "SELECT refstate_observed_at FROM location WHERE id = ?1",
            [rig.gone.location.0],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    assert_eq!(states, 0, "a job state was written for the removed copy");
    assert_eq!(observed, None, "J1 wrote the removed copy's refstate");
}

#[test]
fn ac_p4_46_20_no_job_runs_for_a_removed_location() {
    let rig = rig();
    uninstall(&rig, rig.gone.location);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

#[test]
fn ac_p4_46_20_no_job_runs_for_any_location_of_a_removed_project() {
    let rig = rig();
    remove_project(&rig, rig.gone.project);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

/// What separates a dispatch-time gate from an enqueue-time one: the job is queued while the copy
/// is live, the copy is removed, and only then does the pool start.
#[test]
fn ac_p4_46_20_a_job_queued_before_the_removal_does_not_run() {
    let rig = rig();
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    uninstall(&rig, rig.gone.location);
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

/// J5 reads no repository and a removed project keeps its art, so its card can still be redrawn;
/// J1 on the same copy does not run.
#[test]
fn ac_p4_46_20_j5_art_still_runs_for_a_removed_project() {
    let rig = rig();
    uninstall(&rig, rig.gone.location);
    remove_project(&rig, rig.gone.project);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J5Art)));
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    let done = done_for(&rig, rig.gone.project);
    eprintln!("job_done frames for the removed project: {done:?}");
    assert_eq!(done, ["j5"], "J5 alone runs for a removed project");
}

/// A page rig: project 1's only copy is the one each test removes, or whose project it removes;
/// project 2's copy is live and is the control.
fn page_rig() -> detail_rig::Rig {
    let rig = detail_rig::Rig::new();
    rig.project(1, "gone");
    rig.location(1, 1, "/srv/work/gone", "present", Some("main"), None, None);
    rig.project(2, "live");
    rig.location(2, 2, "/srv/work/live", "present", Some("main"), None, None);
    rig
}

/// Every §6 reading the rig's commands asked for, as `(project, location)`.
fn asked(rig: &detail_rig::Rig) -> Vec<(i64, i64)> {
    rig.jobs.visible.lock().unwrap().clone()
}

/// A project whose only copy is removed still names that copy as its primary, so its page shows
/// it. The page answers from what is stored and asks for no reading of it.
#[test]
fn ac_p4_46_20_a_gone_copys_page_queues_no_job() {
    let rig = page_rig();
    uninstall_row(rig.conn(), LocationId(1));

    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("the page answers");
    let shown = serde_json::to_value(&detail).unwrap();
    eprintln!(
        "removed copy's page: row {}, asked {:?}",
        shown["row"]["id"],
        asked(&rig)
    );
    assert_eq!(shown["row"]["id"], 1);
    assert_eq!(
        asked(&rig),
        [],
        "the page asked a reading of a removed copy"
    );

    handle_project_get(&rig.ctx(), json!({ "id": 2 })).expect("the live page answers");
    assert_eq!(asked(&rig), [(2, 2)], "the live copy's page asks once");
}

/// A removed project's copy is not removed, and is still not read: the Peek answers and asks for
/// nothing. The live project's Peek, in the same rig, asks once.
#[test]
fn ac_p4_46_20_a_removed_projects_peek_queues_no_job() {
    let rig = page_rig();
    rig.conn()
        .execute("UPDATE project SET removed_at = 5 WHERE id = 1", [])
        .unwrap();
    let ctx = ProjectsCtx {
        index: &rig.index,
        events: &rig.sink,
        jobs: &rig.jobs,
        mounts: &rig.mount,
        sync: &NullSyncSink,
        now: detail_rig::NOW,
        tz_offset_min: 0,
    };

    let peek = dispatch_projects_command(&ctx, "projects.peek", json!({ "id": 1 }))
        .expect("owned")
        .expect("the Peek answers");
    eprintln!(
        "removed project's Peek: id {}, asked {:?}",
        peek["id"],
        asked(&rig)
    );
    assert_eq!(peek["id"], 1);
    assert_eq!(
        asked(&rig),
        [],
        "the Peek asked a reading for a removed project"
    );

    dispatch_projects_command(&ctx, "projects.peek", json!({ "id": 2 }))
        .expect("owned")
        .expect("the live Peek answers");
    assert_eq!(asked(&rig), [(2, 2)], "the live project's Peek asks once");
}

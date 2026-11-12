//! The kill ceiling each job's git runs under, and what an invocation killed at it does to the job.
//!
//! §4.1's budget table gives J1 50 ms and J3 300 ms with the rule *mark stale, requeue*; only J2's
//! 500 ms is a kill whose overrun degrades and retries. So `run_one` hands J1 and J3 a ceiling for a
//! git that hangs, and an invocation killed at it requeues the job rather than failing it.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitBackend, GitError, RefFingerprint, RefState};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::state::JobStateRow;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobKind, JobOrigin, JobState, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, FakeGitBackend, GitReply};

const T0: i64 = 1_800_000_000;

/// One project on one present copy. The copy's directory exists and holds nothing: every git
/// answer comes from the fake.
struct Rig {
    dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("copy");
        std::fs::create_dir(&copy).unwrap();
        let index = Index::open_at(dir.path(), T0).unwrap();
        let path = copy.to_string_lossy().into_owned();
        let conn = index.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('p', 'p', 0, 0)",
            [],
        )
        .unwrap();
        let project = ProjectId(conn.last_insert_rowid());
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
            rusqlite::params![project.0, path.as_bytes(), path],
        )
        .unwrap();
        let location = LocationId(conn.last_insert_rowid());
        Self {
            dir,
            index: Arc::new(Mutex::new(index)),
            project,
            location,
        }
    }

    fn job(&self, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }
    }

    fn row(&self, kind: JobKind) -> Option<JobStateRow> {
        let guard = self.index.lock().unwrap();
        let rows = codotheca_core::jobs::state::load(guard.conn(), self.project).unwrap();
        drop(guard);
        rows.into_iter().find(|r| r.job == kind)
    }
}

/// The clock never moves, so a requeued job's backoff never elapses and it is not run again.
fn deps(git: Arc<dyn GitBackend>) -> JobDeps {
    JobDeps {
        git,
        clock: Arc::new(FakeClock::new(T0)),
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    }
}

fn ref_state() -> RefState {
    RefState {
        head_oid: Some("a".repeat(40)),
        branch: Some("main".to_owned()),
        upstream: None,
        ahead: None,
        behind: None,
        tag_count: 0,
        stash_count: Some(0),
        is_shallow: false,
        is_bare: false,
        interrupted_op: None,
        fetch_head_at: None,
        reflog_tail_at: None,
        basis: RefFingerprint::from_hex(&"0".repeat(64)).unwrap(),
        observed_at: T0,
        push: codotheca_core::git::PushObservation::without_push(
            codotheca_core::git::PushRecord::NoRemote,
        ),
    }
}

/// Every git call `run_one` makes for J1, J2 and J3 carries that job's ceiling: five seconds and a
/// minute for the two whose §4.1 budget is *requeue*, and J2's own 500 ms. Before this, J1's git was
/// killed at 50 ms — a Windows git invocation's wall time idle.
#[test]
fn run_one_hands_each_job_its_invocation_deadline() {
    for (kind, want) in [
        (JobKind::J1Refstate, Duration::from_secs(5)),
        (JobKind::J2Status, Duration::from_millis(500)),
        (JobKind::J3Inventory, Duration::from_secs(60)),
    ] {
        let rig = Rig::new();
        let git = Arc::new(FakeGitBackend::new());
        git.always_ref_state(GitReply::Ok(ref_state()));
        git.always_divergence(GitReply::Ok(None));
        git.always_worktree_status(GitReply::Err(GitError::Missing));
        git.always_tracked_inventory(GitReply::Err(GitError::Missing));
        let deps = deps(git.clone());
        // The outcome is not the subject: only the ceiling each call was handed is.
        let _ = run_one(&rig.index, &deps, &rig.job(kind), &RefCell::new(Vec::new()));

        let seen = git.deadlines();
        assert!(!seen.is_empty(), "{kind:?} made no git call");
        for (op, deadline) in seen {
            assert_eq!(deadline, Some(want), "{kind:?} `{op}`");
        }
        assert_eq!(kind.invocation_deadline(), Some(want), "{kind:?}");
    }
}

/// §4.1: J1's and J3's budgets are *mark stale, requeue*. An invocation killed at its ceiling
/// requeues the job with a transient failure; it never fails it, which would hold until a reset.
#[test]
fn a_killed_j1_or_j3_invocation_requeues_the_job() {
    for kind in [JobKind::J1Refstate, JobKind::J3Inventory] {
        let rig = Rig::new();
        let git = FakeGitBackend::new();
        git.always_ref_state(GitReply::Ok(ref_state()));
        git.always_divergence(GitReply::Err(GitError::Budget { after_ms: 5_000 }));
        git.always_tracked_inventory(GitReply::Err(GitError::Budget { after_ms: 60_000 }));
        let runner = JobRunner::new(
            Arc::clone(&rig.index),
            deps(Arc::new(git)),
            Arc::new(ValidatingSink::default()),
        );
        runner.start(1);
        assert!(runner.enqueue(rig.job(kind)));

        let start = Instant::now();
        let row = loop {
            if let Some(row) = rig.row(kind).filter(|r| r.fail_count > 0) {
                break row;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "{kind:?} never settled"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        runner.request_stop();
        runner.join();

        assert_eq!(row.state, JobState::Queued, "{kind:?}: {:?}", row.reason);
        assert_eq!(row.fail_count, 1, "{kind:?}");
        assert_eq!(row.reason.as_deref(), Some("budget_exceeded"), "{kind:?}");
        assert!(rig.dir.path().exists());
    }
}

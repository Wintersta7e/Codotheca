//! The library the badge tests drive through their production callers: real repositories whose
//! commits fall on chosen days, each one project on one present copy, settled by a real J4.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::local_date;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::FakeClock;
use rusqlite::types::Value;

use super::TestRepo;

/// The runner's frozen clock, after every fixture date.
pub(crate) const T0: i64 = 1_800_000_000;

/// The address every fixture commit is made under, confirmed as the user's.
const ME: &str = "fixture@example.invalid";

/// A project's flags, as the scan would have written them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Flags {
    pub(crate) shallow: bool,
    pub(crate) reference: bool,
}

struct Copy {
    repo: TestRepo,
    project: ProjectId,
    location: LocationId,
}

/// One index, its projects, and every event the runners it built emitted.
pub(crate) struct BadgesRig {
    _dir: tempfile::TempDir,
    pub(crate) index: Arc<Mutex<Index>>,
    copies: Vec<Copy>,
    pub(crate) events: Arc<ValidatingSink>,
}

impl BadgesRig {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open_at(dir.path(), T0).unwrap();
        index
            .conn()
            .execute(
                "INSERT INTO identity (email, is_user, source, confirmed_at)
                 VALUES (?1, 1, 'manual', ?2)",
                rusqlite::params![ME, T0 - 100],
            )
            .unwrap();
        Self {
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            copies: Vec::new(),
            events: Arc::new(ValidatingSink::default()),
        }
    }

    /// A project named `name` whose repository holds one of the user's commits at noon UTC on
    /// each local day number in `days`, on one present copy.
    pub(crate) fn project(&mut self, name: &str, days: &[i64], flags: Flags) -> ProjectId {
        let repo = TestRepo::init();
        for &day in days {
            let date = local_date(day);
            repo.commit_at(&date, &format!("{date}T12:00:00+00:00"));
        }
        let path = repo.path().to_string_lossy().into_owned();
        let mut guard = self.index.lock().unwrap();
        let (project, location) = guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_shallow, is_reference, created_at, updated_at)
                     VALUES (?1, ?1, ?1, 1, ?2, ?3, 0, 0)",
                    rusqlite::params![name, i64::from(flags.shallow), i64::from(flags.reference)],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO project_committer (project_id, email, commits)
                     VALUES (?1, ?2, 1)",
                    rusqlite::params![project.0, ME],
                )?;
                Ok((project, location))
            })
            .unwrap();
        drop(guard);
        self.copies.push(Copy {
            repo,
            project,
            location,
        });
        project
    }

    /// One J4 for `project` on a fresh runner over real git, spun until the runner is idle.
    pub(crate) fn settle_j4(&self, project: ProjectId) {
        let copy = self
            .copies
            .iter()
            .find(|c| c.project == project)
            .expect("a project this rig planted");
        let clock = Arc::new(FakeClock::new(T0));
        let git = SystemGit::new(
            Arc::new(copy.repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        );
        let deps = JobDeps {
            git: Arc::new(git),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, self.events.clone());
        assert!(runner.enqueue(Job {
            kind: JobKind::J4History,
            project_id: project,
            location_id: copy.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(25));
        }
        let idle = runner.is_idle();
        runner.request_stop();
        runner.join();
        assert!(idle, "J4 did not settle for {project:?}");
        let state: String = self.query(
            "SELECT state FROM project_job_state WHERE project_id = ?1 AND job = 'j4'",
            project.0,
        );
        assert_eq!(state, "ok", "J4 did not finish for {project:?}");
    }

    fn query(&self, sql: &str, param: i64) -> String {
        let guard = self.index.lock().unwrap();
        let out = guard.conn().query_row(sql, [param], |r| r.get(0)).unwrap();
        drop(guard);
        out
    }

    /// Run `sql` on the index.
    pub(crate) fn sql(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute_batch(sql).unwrap();
        drop(guard);
    }

    /// Every `badge_award` row, every column, by kind.
    pub(crate) fn awards(&self) -> Vec<Vec<Value>> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare("SELECT * FROM badge_award ORDER BY kind")
            .unwrap();
        let width = st.column_count();
        let rows = st
            .query_map([], |r| (0..width).map(|i| r.get::<_, Value>(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    /// `kind`'s award as `(earned_on, project_id, evidence)`, when it is held.
    pub(crate) fn award(&self, kind: &str) -> Option<(String, Option<i64>, serde_json::Value)> {
        let guard = self.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT earned_on, project_id, evidence FROM badge_award WHERE kind = ?1",
                [kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)),
            )
            .ok();
        drop(guard);
        row.map(|(on, project, evidence)| (on, project, serde_json::from_str(&evidence).unwrap()))
    }

    /// `SELECT COUNT(*) FROM xp_events`.
    pub(crate) fn xp_event_count(&self) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row("SELECT COUNT(*) FROM xp_events", [], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    }
}

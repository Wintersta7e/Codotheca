//! `AC-P4-55-1`'s core half (§55.2, GA28): a project whose CI configuration is removed has its open
//! `ci_red` item **set aside** — the check reads `notApplicable`, the page's debt list drops it, the
//! row stays `open` — and nothing is paid or recorded either way. Driven through the production
//! `JobRunner` over a real repository: J1 settles the sweep, J7 enumerates the tree.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::accounts::keychain::token_ref;
use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::cancel::CancelToken;
use codotheca_core::detail::get::handle_project_get;
use codotheca_core::detail::DetailCtx;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::health::read_for_project;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, NullJobSink, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{
    AuthKind, CheckOutcome, DebtSource, LocationId, ProjectId, ScopeTier,
};
use codotheca_core::sync::runner::NullSyncSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, FakeGitBackend, FakeMountResolver};
use support::TestRepo;

const T0: i64 = 1_700_000_000;
const CI_FILE: &str = ".github/workflows/ci.yml";

/// A real repository with a CI file, the user's identity, a connected forge account, and a failing
/// concluded run on the primary branch: one J1 settle opens `ci_red`.
struct CiRig {
    repo: TestRepo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl CiRig {
    fn new() -> Self {
        let repo = TestRepo::init();
        // An entry point, so J3 classifies it `cli` — an archetype that proposes nothing N/A.
        repo.write("main.py", b"print('one')\n");
        repo.write(CI_FILE, b"on: push\n");
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), T0).unwrap();
        let path = repo.path().to_string_lossy().into_owned();
        let (project, location) = index
            .with_tx(|tx| {
                // J15 decides authorship against this, so the chain keeps the project non-Reference
                // and enqueues J7 after it.
                tx.execute(
                    "INSERT INTO identity (email, is_user, source, confirmed_at)
                     VALUES ('fixture@example.invalid', 1, 'manual', ?1)",
                    [T0 - 200],
                )?;
                insert_account(
                    tx,
                    &NewAccount {
                        provider: "github".to_owned(),
                        host: "github.com".to_owned(),
                        login: "owner".to_owned(),
                        display_name: None,
                        auth_kind: AuthKind::Device,
                        scope_tier: ScopeTier::Private,
                        granted_scopes: vec!["repo".to_owned()],
                        token_ref: token_ref("github", "github.com", "owner"),
                    },
                    T0 - 200,
                )
                .map_err(|e| codotheca_core::index::IndexError::Corrupt {
                    detail: format!("{e:?}"),
                })?;
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, acknowledged_at, provider,
                                          provider_repo_id, created_at, updated_at)
                     VALUES ('p', 'p', 'fixture', 1, 0, ?1, 'github', 'r1', 0, 0)",
                    [T0 - 100],
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
                    "INSERT INTO remote_ci_run
                        (provider, provider_repo_id, run_id, workflow_name, conclusion, branch,
                         run_number, started_at)
                     VALUES ('github', 'r1', 1, 'build', 'failure', 'main', 1, ?1)",
                    [T0 - 50],
                )?;
                Ok((project, location))
            })
            .unwrap();
        Self {
            repo,
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            project,
            location,
        }
    }

    /// One J1 and its chain on a fresh runner, spun until `until` holds — never a bare sleep.
    fn run(&self, until: &dyn Fn(&Self) -> bool) {
        let clock = Arc::new(FakeClock::new(T0));
        let deps = JobDeps {
            git: Arc::new(SystemGit::new(
                Arc::new(self.repo.exec()),
                Arc::new(GitSlots::new(4)),
                clock.clone(),
            )),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        };
        let runner = JobRunner::new(
            Arc::clone(&self.index),
            deps,
            Arc::new(CollectingSink::default()),
        );
        assert!(runner.enqueue(Job {
            kind: JobKind::J1Refstate,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Interactive,
        }));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !until(self) {
            std::thread::sleep(Duration::from_millis(25));
        }
        runner.request_stop();
        runner.join();
    }

    fn count(&self, sql: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(sql, [self.project.0], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    }

    fn text(&self, sql: &str) -> Option<String> {
        let guard = self.index.lock().unwrap();
        let v = guard
            .conn()
            .query_row(sql, [self.project.0], |r| r.get(0))
            .ok();
        drop(guard);
        v
    }

    fn has_ci(&self) -> Option<String> {
        self.text("SELECT has_ci FROM project_content_scan WHERE project_id = ?1")
    }

    fn ci_green(&self) -> Option<String> {
        self.text("SELECT state FROM project_check WHERE project_id = ?1 AND check_key = 'ciGreen'")
    }

    /// J7 settled: its row is `ok`, so the settle's completion evaluation has run after it.
    fn j7_settled(&self) -> bool {
        self.text("SELECT state FROM project_job_state WHERE project_id = ?1 AND job = 'j7'")
            .as_deref()
            == Some("ok")
    }

    fn open_ci_red(&self) -> i64 {
        self.count(
            "SELECT count(*) FROM debt_item
              WHERE project_id = ?1 AND source = 'ci_red' AND state = 'open'",
        )
    }

    fn paid(&self) -> (i64, i64) {
        (
            self.count(
                "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND kind = 'debt_day'",
            ),
            self.count("SELECT count(*) FROM health_delta WHERE project_id = ?1"),
        )
    }

    /// `projects.get`'s debt list, through the production handler.
    fn page_debt(&self) -> Vec<DebtSource> {
        let guard = self.index.lock().unwrap();
        let ctx = DetailCtx {
            index: &guard,
            git: &FakeGitBackend::new(),
            mount: &FakeMountResolver::new(),
            events: &CollectingSink::default(),
            jobs: &NullJobSink,
            sync: &NullSyncSink,
            now: T0,
            zone: &FixedZone(0),
        };
        let detail = handle_project_get(&ctx, serde_json::json!({ "id": self.project.0 }))
            .unwrap_or_else(|e| panic!("projects.get refused: {e:?}"));
        drop(guard);
        detail.debt.iter().map(|item| item.source).collect()
    }

    fn ci_red_outcome(&self) -> CheckOutcome {
        let guard = self.index.lock().unwrap();
        let (reading, _) = read_for_project(guard.conn(), self.project).unwrap();
        drop(guard);
        reading
            .checks
            .iter()
            .find(|c| c.id == DebtSource::CiRed)
            .map(|c| c.outcome)
            .expect("ci_red is in the reading")
    }
}

#[test]
fn ac_p4_55_1_removing_ci_sets_ci_red_aside_and_pays_nothing() {
    let rig = CiRig::new();

    // Precondition: the item opens and the page lists it. A fixture that cannot open it proves
    // nothing about setting it aside.
    rig.run(&|r| r.open_ci_red() == 1 && r.j7_settled());
    eprintln!(
        "opened: has_ci={:?} ciGreen={:?} open ci_red={}",
        rig.has_ci(),
        rig.ci_green(),
        rig.open_ci_red()
    );
    assert_eq!(rig.open_ci_red(), 1, "the fixture never opened ci_red");
    assert_eq!(rig.has_ci().as_deref(), Some("present"));
    assert!(
        rig.page_debt().contains(&DebtSource::CiRed),
        "projects.get never listed the open ci_red"
    );
    let before = rig.paid();

    // The CI configuration goes. J7 reads its absence; `ciGreen` is N/A by §31.1a's derived rule.
    rig.repo.git(&["rm", "-q", CI_FILE]);
    rig.repo.commit("drop ci");
    rig.run(&|r| r.has_ci().as_deref() == Some("absent") && r.ci_green().as_deref() == Some("na"));
    let after = rig.paid();
    eprintln!(
        "removed: has_ci={:?} ciGreen={:?} open ci_red={} (debt_day, health_delta) {before:?} -> {after:?}",
        rig.has_ci(),
        rig.ci_green(),
        rig.open_ci_red()
    );
    assert_eq!(rig.ci_green().as_deref(), Some("na"));
    assert_eq!(rig.ci_red_outcome(), CheckOutcome::NotApplicable);
    assert!(
        !rig.page_debt().contains(&DebtSource::CiRed),
        "projects.get still lists ci_red with ciGreen na"
    );
    assert_eq!(
        rig.open_ci_red(),
        1,
        "set aside means still open, never closed"
    );
    assert_eq!(
        after, before,
        "setting ci_red aside paid or recorded something"
    );

    // The file comes back: the item is listed again, and still nothing is recorded.
    rig.repo.write(CI_FILE, b"on: push\n");
    rig.repo.commit("restore ci");
    rig.run(&|r| r.has_ci().as_deref() == Some("present") && r.j7_settled());
    let restored = rig.paid();
    eprintln!(
        "restored: has_ci={:?} ciGreen={:?} (debt_day, health_delta) {restored:?}",
        rig.has_ci(),
        rig.ci_green()
    );
    assert_eq!(rig.has_ci().as_deref(), Some("present"));
    assert!(rig.page_debt().contains(&DebtSource::CiRed));
    assert_eq!(rig.open_ci_red(), 1);
    assert_eq!(
        restored.1, before.1,
        "restoring the file recorded a health_delta"
    );
}

//! §38.16 item 14, `AC-P4-38-14` — **the `debt_day` gates hold at every production payout
//! site**, each driven by its real caller: the scheduler's settle, J7's item build and the
//! advisory settle.
//!
//! Per site the control runs first and pays exactly one row, so a negative case proves something:
//! each differs from the control in one variable. The gates live inside `pay_debt_day`, so every
//! site takes them; this proves each one is on **that caller's** path. Rust integration tests
//! cannot import one another, so the rig shapes are copied from `restoration_callers.rs` and
//! `advisory_sweep_settle.rs`, never shared.
//!
//! Some cases cannot discriminate at a site, and each is printed with the reason rather than
//! dropped: J7 runs no content sweep for an unenrolled or a Reference project, so it closes
//! nothing there; and no site can reach a closure on a project with no subject — the scheduler
//! and J7 carry a location row, and the advisory sync opens nothing for a subject-less project.
//! The no-subject rule is proved at the writer, in `debt_xp.rs`.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitSlots, JobClass, JobContext, RepoHandle, SystemGit, TreeEntry};
use codotheca_core::health::switches::write_switches;
use codotheca_core::http::HttpResponse;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{j7_markers, Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    DebtSource, HealthCheckSwitch, HealthDetectedIn, LocationId, ProjectId, SyncTaskKind,
    SyncTaskState,
};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::state::SyncTaskStateRow;
use codotheca_core::sync::store::{load, put};
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeGitBackend, FakeTokenStore, FakeTransport, GitReply};
use rusqlite::Connection;
use support::TestRepo;

/// The one variable a case changes against its site's control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    /// A `scored` closure on an enrolled, non-Reference project with a subject: one row.
    Control,
    /// The closed item was `shown_only`.
    ShownOnly,
    /// The project's `acknowledged_at` is NULL at the closing observation.
    Unenrolled,
    /// The project is Reference at the closing observation (R219).
    Reference,
    /// The project has no lineage and no location row.
    NoSubject,
}

/// What one case did to the `debt_day` rows, by identity.
struct Outcome {
    site: &'static str,
    case: Case,
    before: BTreeSet<String>,
    after: BTreeSet<String>,
    /// Why a case that cannot discriminate at this site cannot, printed beside it.
    note: String,
}

impl Outcome {
    fn paid(&self) -> Vec<&String> {
        self.after.difference(&self.before).collect()
    }

    /// The control pays exactly one `debt_day` row; every other case pays none.
    fn holds(&self) -> bool {
        let paid = self.paid();
        match self.case {
            Case::Control => paid.len() == 1 && paid[0].starts_with("debt_day:"),
            _ => paid.is_empty(),
        }
    }
}

/// Every `debt_day` row's key. Git-derived rows a chained job may write are not this writer's.
fn debt_days(conn: &Connection) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare("SELECT dedupe_key FROM xp_events WHERE kind = 'debt_day'")
        .unwrap();
    let keys = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);
    keys
}

#[derive(Debug, Default)]
struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

// ---------------------------------------------------------------------------------------------
// Site 1 — the scheduler's settle, through the real `JobRunner`.
// ---------------------------------------------------------------------------------------------

/// The clock the runner fixture is frozen at.
const T0: i64 = 1_700_000_000;
const OPEN_NO_RELEASE: &str =
    "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'no_release'";

/// A real repository, a real `SystemGit` and the real `JobRunner`. `no_release` opens with no tag
/// and closes on `git tag v1` at the next J1 settle — the scheduler's call of the payout.
struct ReleaseRig {
    repo: TestRepo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl ReleaseRig {
    fn new() -> Self {
        let repo = TestRepo::init();
        // An entry point, so J3 classifies it `cli`, for which `release` applies.
        repo.write("main.py", b"print('one')\n");
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), T0).unwrap();
        let path = repo.path().to_string_lossy().into_owned();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, acknowledged_at, created_at, updated_at)
                     VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
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
                write_switches(
                    tx,
                    &[
                        HealthCheckSwitch {
                            check: DebtSource::MissingReadme,
                            enabled: false,
                        },
                        HealthCheckSwitch {
                            check: DebtSource::MissingLicense,
                            enabled: false,
                        },
                    ],
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

    /// One J1 on a fresh runner, spun until `until` holds — never a bare sleep — then stopped.
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
            tz_offset_min: 0,
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, Arc::new(NullSink));
        assert!(runner.enqueue(Job {
            kind: JobKind::J1Refstate,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(30);
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

    fn exec(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, [self.project.0]).unwrap();
        drop(guard);
    }

    fn ledger(&self) -> BTreeSet<String> {
        let guard = self.index.lock().unwrap();
        let keys = debt_days(guard.conn());
        drop(guard);
        keys
    }
}

fn scheduler_case(case: Case) -> Outcome {
    let rig = ReleaseRig::new();
    rig.run(&|this| this.count(OPEN_NO_RELEASE) == 1);
    assert_eq!(
        rig.count(OPEN_NO_RELEASE),
        1,
        "scheduler {case:?}: the no_release item never opened within the wait"
    );

    // Between the phases. J1.5 may have classified the fixture since, so every case sets both
    // columns back first; then the case's one variable.
    rig.exec("UPDATE project SET authored_by_user = 1, is_reference = 0 WHERE id = ?1");
    match case {
        Case::Control => {}
        Case::ShownOnly => rig.exec(
            "UPDATE debt_item SET scoring = 'shown_only'
              WHERE project_id = ?1 AND source = 'no_release'",
        ),
        Case::Unenrolled => rig.exec("UPDATE project SET acknowledged_at = NULL WHERE id = ?1"),
        Case::Reference => {
            rig.exec("UPDATE project SET authored_by_user = 0, is_reference = 1 WHERE id = ?1");
        }
        Case::NoSubject => unreachable!("a scheduler job carries a location row"),
    }

    let before = rig.ledger();
    rig.repo.git(&["tag", "v1"]);
    rig.run(&|this| this.count(OPEN_NO_RELEASE) == 0);
    assert_eq!(
        rig.count(OPEN_NO_RELEASE),
        0,
        "scheduler {case:?}: the release closed nothing within the wait"
    );
    Outcome {
        site: "scheduler",
        case,
        before,
        after: rig.ledger(),
        note: String::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Site 2 — J7's item build, through the real `run_j7` over a scripted tree.
// ---------------------------------------------------------------------------------------------

/// The clock the J7 fixture writes at.
const NOW: i64 = 1_800_000_000;
const OPEN_TODOS: &str = "SELECT count(*) FROM debt_item
                           WHERE project_id = ?1 AND source = 'todo_marker' AND state = 'open'";

struct J7Rig {
    _dir: tempfile::TempDir,
    index: Mutex<Index>,
    git: Arc<FakeGitBackend>,
    project: ProjectId,
    location: LocationId,
    repo: RepoHandle,
}

impl J7Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), NOW).unwrap();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, acknowledged_at, created_at, updated_at)
                     VALUES ('p', 'p', 'fixture', 1, 0, ?1, 0, 0)",
                    [NOW - 1_000],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [],
                )?;
                tx.execute(
                    "INSERT INTO location
                       (project_id, kind, path_bytes, path_key, path_display, store_key,
                        presence, repo_kind, head_oid)
                     VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store-a', 'present',
                             'worktree', 'head-one')",
                    [project.0],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                Ok((project, location))
            })
            .unwrap();
        Self {
            _dir: dir,
            index: Mutex::new(index),
            git: Arc::new(FakeGitBackend::new()),
            project,
            location,
            repo: RepoHandle::bare(
                std::path::Path::new("/does/not/matter"),
                codotheca_core::git::StoreKey::new("test-store"),
                StoreClass::Local,
            ),
        }
    }

    /// A HEAD of one file carrying `markers` distinct TODO lines.
    fn tree(&self, head: &str, markers: usize) {
        let mut bytes = String::new();
        for i in 0..markers {
            let _ = writeln!(bytes, "// TODO: marker number {i}");
        }
        let oid = format!("{head:a>40}");
        self.git.script_blob(&oid, bytes.as_bytes());
        self.git.always_head_tree(GitReply::Ok(vec![TreeEntry {
            mode: "100644".to_owned(),
            kind: "blob".to_owned(),
            oid,
            path: b"src/a.rs".to_vec(),
        }]));
        self.exec_with(
            "UPDATE location SET head_oid = ?2 WHERE id = ?1",
            rusqlite::params![self.location.0, head],
        );
    }

    fn run(&self, now: i64) {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        j7_markers::run_j7(
            &self.index,
            self.git.as_ref(),
            &self.repo,
            &ctx,
            j7_markers::ScanRun {
                project: self.project,
                location: self.location,
                cursor: None,
                now,
                tz_offset_min: 0,
                detected_in: HealthDetectedIn::Background,
                announce: None,
            },
        )
        .unwrap();
    }

    fn exec_with(&self, sql: &str, params: impl rusqlite::Params) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, params).unwrap();
        drop(guard);
    }

    fn exec(&self, sql: &str) {
        self.exec_with(sql, [self.project.0]);
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

    /// The latest `todo_marker` sweep: `(outcome, observed_at)`.
    fn sweep(&self) -> (String, i64) {
        let guard = self.index.lock().unwrap();
        let sweep = guard
            .conn()
            .query_row(
                "SELECT outcome, observed_at FROM debt_sweep
                  WHERE project_id = ?1 AND source = 'todo_marker'",
                [self.project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(guard);
        sweep
    }

    fn ledger(&self) -> BTreeSet<String> {
        let guard = self.index.lock().unwrap();
        let keys = debt_days(guard.conn());
        drop(guard);
        keys
    }
}

fn j7_case(case: Case) -> Outcome {
    let rig = J7Rig::new();
    rig.tree("head-one", 5);
    rig.run(NOW);
    assert_eq!(rig.count(OPEN_TODOS), 5, "j7 {case:?}: five markers opened");

    match case {
        Case::Control => {}
        Case::ShownOnly => rig.exec(
            "UPDATE debt_item SET scoring = 'shown_only'
              WHERE project_id = ?1 AND source = 'todo_marker'",
        ),
        Case::Unenrolled => rig.exec("UPDATE project SET acknowledged_at = NULL WHERE id = ?1"),
        Case::Reference => {
            rig.exec("UPDATE project SET authored_by_user = 0, is_reference = 1 WHERE id = ?1");
        }
        Case::NoSubject => unreachable!("a J7 run carries a location row"),
    }

    let before = rig.ledger();
    rig.tree("head-two", 4);
    rig.run(NOW + 60);
    let open = rig.count(OPEN_TODOS);
    let (outcome, observed_at) = rig.sweep();
    let note = match case {
        Case::Control | Case::ShownOnly => {
            assert_eq!(open, 4, "j7 {case:?}: the fifth marker did not close");
            String::new()
        }
        _ => {
            let run = if observed_at == NOW {
                "first"
            } else {
                "second"
            };
            format!(
                "non-discriminating: J7 returned before its item build — the latest sweep is the \
                 {run} run's ({outcome}), and nothing closed ({open} open)"
            )
        }
    };
    Outcome {
        site: "j7",
        case,
        before,
        after: rig.ledger(),
        note,
    }
}

// ---------------------------------------------------------------------------------------------
// Site 3 — the advisory settle, through the real `SyncRunner` with scripted forge answers.
// ---------------------------------------------------------------------------------------------

const DAY: i64 = 86_400;
const HOST: &str = "forge.example.invalid";
const DEADLINE: Duration = Duration::from_secs(10);

struct AdvisoryRig {
    index: Arc<Mutex<Index>>,
    transport: Arc<FakeTransport>,
    project: ProjectId,
    /// The working copy the lockfile is read from. For the no-subject case no location row
    /// names it: the lockfile read takes a path and needs none.
    work: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

impl AdvisoryRig {
    /// An enrolled, authored project; `located` gives it a lineage and one location row.
    fn new(located: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
        let work = tempfile::tempdir().unwrap();
        let mut guard = index.lock().unwrap();
        let project = guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at,
                                          authored_by_user, is_reference, created_at, updated_at)
                     VALUES ('alpha', 'alpha', ?1, ?2, 1, 0, 1, 1)",
                    rusqlite::params![located.then_some("alpha"), NOW],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                if located {
                    let path = work.path().to_string_lossy().into_owned();
                    tx.execute(
                        "INSERT INTO location (project_id, kind, path_bytes, path_key,
                                               path_display, store_key, presence, repo_kind)
                         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                        rusqlite::params![project.0, path.as_bytes(), path],
                    )?;
                }
                Ok(project)
            })
            .unwrap();
        drop(guard);
        Self {
            index,
            transport: Arc::new(FakeTransport::new()),
            project,
            work,
            _dir: dir,
        }
    }

    /// The working copy resolves `left` at `version`, and the read J6 runs has seen it.
    fn lock(&self, version: &str, at: i64) {
        std::fs::write(
            self.work.path().join("package-lock.json"),
            format!(
                r#"{{"lockfileVersion":3,"packages":{{"":{{}},"node_modules/left":{{"version":"{version}"}}}}}}"#
            ),
        )
        .unwrap();
        let mut guard = self.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                read_lockfiles(tx, self.project, self.work.path(), at).unwrap();
                Ok(())
            })
            .unwrap();
    }

    /// Queue the sweep, script the answer, run the real runner at `at` until it settles.
    fn sweep(&self, at: i64, response: &str) {
        {
            let mut guard = self.index.lock().unwrap();
            guard
                .with_tx(|tx| {
                    put(
                        tx,
                        &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, at),
                    )
                })
                .unwrap();
        }
        self.transport.push(HttpResponse {
            status: 200,
            headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
            body: response.as_bytes().to_vec(),
        });
        let clock = Arc::new(FakeClock::new(at));
        let observing = Arc::new(ObservingTransport::new(
            self.transport.clone(),
            clock.clone(),
        ));
        let provider = Arc::new(GitHubProvider::new(observing.clone(), HOST.to_owned()));
        let deps = SyncDeps {
            provider,
            transport: observing,
            tokens: Arc::new(FakeTokenStore::available()),
            clock,
            cancel: CancelToken::new(),
            tz_offset_min: 0,
        };
        let runner = SyncRunner::new(Arc::clone(&self.index), deps, Arc::new(NullSink));
        runner.start();
        let deadline = Instant::now() + DEADLINE;
        loop {
            let state = {
                let guard = self.index.lock().unwrap();
                load(guard.conn(), SyncTaskKind::Advisories, None)
                    .unwrap()
                    .map(|r| r.state)
            };
            if state == Some(SyncTaskState::Ok) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the sweep never settled: {state:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        runner.request_stop();
        runner.join();
    }

    fn exec(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, [self.project.0]).unwrap();
        drop(guard);
    }

    /// Every `dependency_advisory` item: `(state, scoring)`.
    fn items(&self) -> Vec<(String, String)> {
        let guard = self.index.lock().unwrap();
        let mut stmt = guard
            .conn()
            .prepare(
                "SELECT state, scoring FROM debt_item
                  WHERE source = 'dependency_advisory' ORDER BY fingerprint",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        drop(guard);
        rows
    }

    fn ledger(&self) -> BTreeSet<String> {
        let guard = self.index.lock().unwrap();
        let keys = debt_days(guard.conn());
        drop(guard);
        keys
    }
}

/// One critical advisory against `left`, vulnerable below 2.0.0. `fixed` names 2.0.0 as the
/// first patched version; without it the item opens `shown_only`, since no fix exists.
fn advisory(fixed: bool) -> String {
    let patched = if fixed { "\"2.0.0\"" } else { "null" };
    format!(
        r#"[{{"ghsa_id":"GHSA-aaaa","cve_id":"CVE-2026-0001","summary":"s","html_url":"u",
             "severity":"critical","withdrawn_at":null,
             "identifiers":[{{"value":"GHSA-aaaa","type":"GHSA"}},{{"value":"CVE-2026-0001","type":"CVE"}}],
             "vulnerabilities":[{{"package":{{"ecosystem":"npm","name":"left"}},
               "vulnerable_version_range":"< 2.0.0","first_patched_version":{patched}}}]}}]"#
    )
}

fn advisory_case(case: Case) -> Outcome {
    let rig = AdvisoryRig::new(case != Case::NoSubject);
    rig.lock("1.0.0", NOW);
    rig.sweep(NOW, &advisory(case != Case::ShownOnly));
    let opened = rig.items();
    let note = match case {
        Case::NoSubject => {
            assert!(
                opened.is_empty(),
                "a subject-less project opened {opened:?}"
            );
            "non-discriminating: the sync opens nothing for a project with no subject".to_owned()
        }
        Case::ShownOnly => {
            // The producer derives the scoring from the missing fix; assert it was stored.
            assert_eq!(
                opened,
                vec![("open".to_owned(), "shown_only".to_owned())],
                "advisory {case:?}"
            );
            String::new()
        }
        _ => {
            assert_eq!(
                opened,
                vec![("open".to_owned(), "scored".to_owned())],
                "advisory {case:?}"
            );
            String::new()
        }
    };

    match case {
        Case::Unenrolled => rig.exec("UPDATE project SET acknowledged_at = NULL WHERE id = ?1"),
        Case::Reference => {
            rig.exec("UPDATE project SET authored_by_user = 0, is_reference = 1 WHERE id = ?1");
        }
        Case::Control | Case::ShownOnly | Case::NoSubject => {}
    }

    // The user upgrades past the range; the forge has nothing to say about the new version.
    let before = rig.ledger();
    rig.lock("2.0.0", NOW + DAY);
    rig.sweep(NOW + DAY, "[]");
    let left = rig.items();
    assert!(
        left.is_empty(),
        "advisory {case:?}: the upgrade closed nothing: {left:?}"
    );
    Outcome {
        site: "advisory",
        case,
        before,
        after: rig.ledger(),
        note,
    }
}

// ---------------------------------------------------------------------------------------------
// The criterion
// ---------------------------------------------------------------------------------------------

/// One payout site: its name, the driver that runs a case through it, and the one extra case only
/// it can drive.
type PayoutSite = (&'static str, fn(Case) -> Outcome, Option<Case>);

/// **`AC-P4-38-14`.** Through each of the three production callers, a `shown_only` closure, a
/// closure on an unenrolled project, on a Reference project (R219) and on a project with no
/// subject write no row, while a `scored` closure on an enrolled project writes one.
///
/// Every case runs before any is judged, so a failure lists every case that failed.
#[test]
fn ac_p4_38_14_every_payout_site_pays_only_scored_closures_of_enrolled_projects() {
    let negatives = [Case::ShownOnly, Case::Unenrolled, Case::Reference];
    let mut outcomes = Vec::new();
    let mut sites = 0;
    let payout_sites: [PayoutSite; 3] = [
        ("scheduler", scheduler_case, None),
        ("j7", j7_case, None),
        ("advisory", advisory_case, Some(Case::NoSubject)),
    ];

    for (site, run, extra) in payout_sites {
        let control = run(Case::Control);
        assert!(
            control.holds(),
            "{site}: the control did not pay exactly one row: {:?}",
            control.paid()
        );
        outcomes.push(control);
        for case in negatives.into_iter().chain(extra) {
            outcomes.push(run(case));
        }
        sites += 1;
    }

    let mut failed = Vec::new();
    for outcome in &outcomes {
        eprintln!(
            "{} {:?}: rows {} → {} {}",
            outcome.site,
            outcome.case,
            outcome.before.len(),
            outcome.after.len(),
            outcome.note
        );
        if !outcome.holds() {
            failed.push(format!(
                "{} {:?} paid {:?}",
                outcome.site,
                outcome.case,
                outcome.paid()
            ));
        }
    }
    eprintln!("AC-P4-38-14 payout sites executed: {sites}");
    assert_eq!(sites, 3, "a payout site did not run");
    assert!(failed.is_empty(), "the gates did not hold: {failed:#?}");
}

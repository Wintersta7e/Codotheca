//! §54 — `projects.recheck`: the user-started re-read, driven through the product's own path.
//!
//! No user action used to queue J1: the walk queued it with a `Walk` origin, and a page's own J7
//! compared the **stored** head. So a fix made in an editor since the last walk could never reach
//! §34's restoration as `foreground`. These tests send the command through `CoreHandler::handle`
//! and run its chain on a real `JobPump` over a real `SystemGit`.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::cancel::CancelToken;
use codotheca_core::debt::registry_for;
use codotheca_core::detail::{dispatch_detail_command, DetailCtx};
use codotheca_core::git::{GitBackend, GitSlots, SystemGit};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{JobDeps, JobKind, JobOrigin, JobSink as _, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::{Publisher, PublisherSink};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    DebtSource, HealthCheckSwitch, HealthDetectedIn, LocationId, ProjectHealthDelta, ProjectId,
};
use codotheca_core::sync::runner::NullSyncSink;
use codotheca_core::testing::{FakeClock, FakeMountResolver};
use support::TestRepo;

/// The clock every runner here is frozen at.
const T0: i64 = 1_700_000_000;
/// The fixture's store.
const STORE: &str = "store";
/// How long a chain may take to settle before the wait dumps the index and fails.
const WAIT: Duration = Duration::from_secs(60);

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

impl RecordingSink {
    fn named(&self, topic: &str, event: &str) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, e, _)| t == topic && e == event)
            .map(|(_, _, payload)| payload.clone())
            .collect()
    }

    fn j7_settled(&self) -> bool {
        self.named("scan", "job_done")
            .iter()
            .any(|done| done["job"] == "j7")
    }
}

/// One real repository with one commit and no LICENSE, enrolled, authored by the user's own
/// identity (so J1.5 keeps it authored and not Reference), the content scan granted, and every
/// debt source but `missing_license` switched off — so the one layer moves exactly when that
/// item does.
struct RecheckRig {
    repo: TestRepo,
    dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl RecheckRig {
    fn new() -> Self {
        let repo = TestRepo::init();
        // An entry point, so J3 classifies it `cli` rather than `docs`.
        repo.write("main.py", b"print('one')\n");
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), T0).unwrap();
        let path = repo.path().to_string_lossy().into_owned();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO identity (email, name, source, confirmed_at)
                     VALUES ('fixture@example.invalid', 'Fixture', 'manual', ?1)",
                    [T0 - 200],
                )?;
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
                     VALUES (?1, 'linux', ?2, ?2, ?3, ?4, 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path, STORE],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [],
                )?;
                let off: Vec<HealthCheckSwitch> = DebtSource::ALL
                    .into_iter()
                    .filter(|&check| check != DebtSource::MissingLicense)
                    .map(|check| HealthCheckSwitch {
                        check,
                        enabled: false,
                    })
                    .collect();
                write_switches(tx, &off)?;
                Ok((project, location))
            })
            .unwrap();
        Self {
            repo,
            dir,
            index: Arc::new(Mutex::new(index)),
            project,
            location,
        }
    }

    fn git(&self, clock: &Arc<FakeClock>) -> Arc<dyn GitBackend> {
        Arc::new(SystemGit::new(
            Arc::new(self.repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        ))
    }

    fn deps(&self) -> JobDeps {
        let clock = Arc::new(FakeClock::new(T0));
        JobDeps {
            exclusions: Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default()),
            git: self.git(&clock),
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
        }
    }

    /// The walk found the repository: the production hand-off queues a `Walk` J1, and its chain
    /// runs on a real runner until `missing_license` is open and nothing is due.
    fn walk_seed(&self) {
        let runner = JobRunner::new(
            Arc::clone(&self.index),
            self.deps(),
            Arc::new(RecordingSink::default()),
        );
        runner.on_location_indexed(self.project, self.location, STORE, StoreClass::Local);
        runner.start(2);
        let open = || self.open_license_items() == 1 && runner.is_idle();
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline && !open() {
            std::thread::sleep(Duration::from_millis(25));
        }
        runner.request_stop();
        runner.join();
        assert_eq!(
            self.open_license_items(),
            1,
            "the walk's chain never opened missing_license\n{}",
            self.dump()
        );
        assert_eq!(
            self.count("SELECT count(*) FROM health_delta WHERE project_id = ?1"),
            0,
            "the first observation wrote a row\n{}",
            self.dump()
        );
    }

    /// A `CoreHandler` whose command arms and job pump both run real git against the fixture,
    /// the pump's events recorded.
    fn handler(&self, pump_events: Arc<RecordingSink>) -> CoreHandler {
        let clock = Arc::new(FakeClock::new(T0));
        let git = self.git(&clock);
        let events = Arc::new(PublisherSink::new(Publisher::detached()));
        let http: Arc<dyn codotheca_core::http::HttpTransport> =
            Arc::new(codotheca_core::testing::FakeTransport::new());
        let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
            Arc::clone(&http),
            clock.clone(),
        ));
        let sync_http: Arc<dyn codotheca_core::http::HttpTransport> = sync_observing.clone();
        let exclusions = Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default());
        CoreHandler::new(CoreDeps {
            exclusions: Arc::clone(&exclusions),
            index: Arc::clone(&self.index),
            provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                Arc::clone(&http),
                codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
            )),
            tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
            http,
            client_id: "test-client-id".to_owned(),
            clock: clock.clone(),
            git: Arc::clone(&git),
            write_git: Arc::new(codotheca_core::testing::FakeMutatingGit::new(
                codotheca_core::testing::CloneBehaviour::Succeed,
            )),
            trash: Arc::new(codotheca_core::testing::CountingTrash::new()),
            mount: Arc::new(FakeMountResolver::default()),
            spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
            sessions: codotheca_core::session::manager::SessionManager::new(
                clock.clone(),
                events.clone(),
                Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
                Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
                Arc::new(codotheca_core::testing::zone::FixedZone(0)),
                Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default()),
            ),
            scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
                codotheca_core::testing::ScanLauncherFake::new(),
            )),
            scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
            firstrun: codotheca_core::firstrun::FirstRunEnv {
                sources: codotheca_core::firstrun::sources::SourceEnv {
                    home: self.dir.path().to_path_buf(),
                    app_data: None,
                    xdg_config: None,
                },
                classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
                    vec![],
                )),
                distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
                platform: codotheca_core::index::path::PathPlatform::Unix,
                exclusions,
                cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
            },
            jobs: codotheca_core::assembly::jobs::JobPump::start(
                Arc::clone(&self.index),
                git,
                clock.clone(),
                pump_events,
                Arc::new(codotheca_core::testing::zone::FixedZone(0)),
                Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default()),
            ),
            sync: codotheca_core::assembly::sync::SyncPump::start(
                Arc::clone(&self.index),
                codotheca_core::sync::SyncDeps {
                    exclusions: Arc::new(codotheca_core::exclusion::gate::ExclusionGate::default()),
                    provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                        Arc::clone(&sync_http),
                        codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                    )),
                    transport: sync_observing,
                    tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                    clock,
                    cancel: CancelToken::new(),
                    zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
                },
                events.clone(),
            ),
            events,
            tz_offset_min: 0,
            zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
        })
    }

    /// `projects.recheck` through the detail dispatcher, under the one index guard, asking `jobs`.
    fn recheck_with(&self, jobs: &dyn codotheca_core::jobs::JobSink, id: i64) -> serde_json::Value {
        let guard = self.index.lock().unwrap();
        let mounts = FakeMountResolver::default();
        let events = RecordingSink::default();
        let ctx = DetailCtx {
            exclusions: &codotheca_core::exclusion::gate::ExclusionGate::default(),
            index: &guard,
            git: &codotheca_core::testing::FakeGitBackend::new(),
            mount: &mounts,
            events: &events,
            jobs,
            sync: &NullSyncSink,
            now: T0,
            zone: &codotheca_core::testing::zone::FixedZone(0),
        };
        let answer =
            dispatch_detail_command(&ctx, "projects.recheck", serde_json::json!({ "id": id }))
                .expect("the detail dispatcher owns projects.recheck")
                .unwrap();
        drop(guard);
        answer
    }

    fn open_license_items(&self) -> i64 {
        self.count(
            "SELECT count(*) FROM debt_item WHERE project_id = ?1 \
             AND source = 'missing_license' AND state = 'open'",
        )
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

    /// Everything the delta's precondition reads, as text, for a wait that timed out.
    fn dump(&self) -> String {
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        let mut out = String::new();
        for (label, sql) in [
            (
                "project",
                "SELECT 'authored=' || ifnull(authored_by_user, 'NULL') || ' reference=' || \
                 ifnull(is_reference, 'NULL') || ' ack=' || ifnull(acknowledged_at, 'NULL') || \
                 ' archetype=' || ifnull(archetype, 'NULL') FROM project WHERE id = ?1",
            ),
            (
                "content_scan",
                "SELECT 'head=' || ifnull(complete_head_oid, 'NULL') || ' license=' || \
                 ifnull(has_license, 'NULL') FROM project_content_scan WHERE project_id = ?1",
            ),
            (
                "debt_item",
                "SELECT source || ' ' || state || ' ' || scoring FROM debt_item \
                 WHERE project_id = ?1",
            ),
            (
                "job_state",
                "SELECT job || ' ' || state || ' ' || ifnull(reason, '') \
                 FROM project_job_state WHERE project_id = ?1",
            ),
            (
                "health_delta",
                "SELECT layer || ' ' || from_value || '->' || to_value || ' ' || detected_in \
                 FROM health_delta WHERE project_id = ?1",
            ),
        ] {
            let mut statement = conn.prepare(sql).unwrap();
            let rows: Vec<String> = statement
                .query_map([self.project.0], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let _ = writeln!(out, "  {label}: {rows:?}");
        }
        drop(guard);
        out
    }
}

/// A runner nobody starts: what a command queues stays queued for the test to read.
fn idle_runner(rig: &RecheckRig) -> Arc<JobRunner> {
    JobRunner::new(
        Arc::clone(&rig.index),
        rig.deps(),
        Arc::new(RecordingSink::default()),
    )
}

/// AC-P4-54-1: a LICENSE committed in an editor since the last walk, then `projects.recheck`
/// through the production handler — the chain it starts closes `missing_license` and §34 records
/// the decrease as `foreground`.
#[test]
fn ac_p4_54_1_restoration_by_press_through_production() {
    let rig = RecheckRig::new();
    rig.walk_seed();

    rig.repo.write("LICENSE", b"Permission is granted.\n");
    rig.repo.commit("add a licence");

    let events = Arc::new(RecordingSink::default());
    let mut handler = rig.handler(Arc::clone(&events));
    let answer = handler
        .handle(
            "projects.recheck",
            serde_json::json!({ "id": rig.project.0 }),
        )
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline && !(events.j7_settled() && rig.open_license_items() == 0) {
        std::thread::sleep(Duration::from_millis(25));
    }
    handler.shutdown();

    assert_eq!(answer, serde_json::json!({ "queued": true }));
    assert!(
        events.j7_settled(),
        "the re-check's chain never settled J7\n{}",
        rig.dump()
    );
    let deltas: Vec<ProjectHealthDelta> = events
        .named("projects", "health_delta")
        .into_iter()
        .map(|payload| serde_json::from_value(payload).unwrap())
        .collect();
    // stderr, never stdout: `print_stdout` is denied, because stdout carries protocol frames.
    eprintln!("health_delta events: {deltas:#?}");
    assert_eq!(
        deltas.len(),
        1,
        "exactly one delta for one closure\n{}",
        rig.dump()
    );
    let delta = &deltas[0];
    assert_eq!(
        delta.detected_in,
        HealthDetectedIn::Foreground,
        "a chain the user started is foreground\n{}",
        rig.dump()
    );
    let layer = registry_for(DebtSource::MissingLicense).layer;
    let moved = delta
        .layers
        .iter()
        .find(|l| l.layer == layer)
        .unwrap_or_else(|| panic!("no {layer:?} entry in {delta:?}"));
    assert!(
        moved.to_value < moved.from_value,
        "{layer:?} did not decrease: {moved:?}"
    );
}

/// AC-P4-54-7: no current primary copy answers `queued: false` and queues nothing — a project
/// with no location, and one whose only copy was removed.
#[test]
fn ac_p4_54_7_no_current_primary_answers_false() {
    let rig = RecheckRig::new();
    let runner = idle_runner(&rig);

    let bare = {
        let guard = rig.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO project (name, seed_basename, created_at, updated_at)
                 VALUES ('bare', 'bare', 0, 0)",
                [],
            )
            .unwrap();
        let id = guard.conn().last_insert_rowid();
        drop(guard);
        id
    };
    assert_eq!(
        rig.recheck_with(runner.as_ref(), bare),
        serde_json::json!({ "queued": false })
    );
    assert!(runner.queued().is_empty(), "{:?}", runner.queued());

    {
        let guard = rig.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE location SET removed_at = ?2 WHERE id = ?1",
                rusqlite::params![rig.location.0, T0],
            )
            .unwrap();
        drop(guard);
    }
    assert_eq!(
        rig.recheck_with(runner.as_ref(), rig.project.0),
        serde_json::json!({ "queued": false })
    );
    assert!(runner.queued().is_empty(), "{:?}", runner.queued());
}

/// AC-P4-54-7: a walk's J1 already queued for the copy is the same `(kind, location)`, so the
/// re-check lifts it rather than queueing a second — one J1, now the user's.
#[test]
fn ac_p4_54_7_an_already_queued_ref_state_coalesces() {
    let rig = RecheckRig::new();
    let runner = idle_runner(&rig);
    runner.on_location_indexed(rig.project, rig.location, STORE, StoreClass::Local);
    assert_eq!(
        runner.queued(),
        vec![(
            JobKind::J1Refstate,
            rig.location,
            Priority::RefState,
            JobOrigin::Walk
        )]
    );

    assert_eq!(
        rig.recheck_with(runner.as_ref(), rig.project.0),
        serde_json::json!({ "queued": true })
    );
    assert_eq!(
        runner.queued(),
        vec![(
            JobKind::J1Refstate,
            rig.location,
            Priority::Interactive,
            JobOrigin::Interactive
        )]
    );
}

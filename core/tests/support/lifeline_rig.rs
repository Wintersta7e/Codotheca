//! §40.6's day close driven the way the product drives it: a real `CoreHandler` over a
//! `FakeClock`, whose job pump runs real git against real repositories — each a clone tracking
//! a bare remote — so the daily pass reads real refs.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::assembly::jobs::JobPump;
use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::git::{ensure_empty_hooks_dir, GitExec, GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::{Publisher, PublisherSink};
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::FakeClock;

use super::{test_git, TestRepo};

/// Seconds in a day.
pub(crate) const DAY: i64 = 86_400;

/// The first day every Lifeline fixture runs on, a day number in late 2024.
pub(crate) const D0: i64 = 20_000;

/// One copy: a clone, and the bare remote its branch tracks.
struct Pair {
    clone: TestRepo,
    /// Kept alive for the clone's pushes and J1's reachability read.
    _remote: TestRepo,
    project: ProjectId,
}

/// A handler over `index` whose job pump runs real git, its jobs' events kept in `jobs_events`.
pub(crate) fn real_handler(
    dir: &Path,
    index: &Arc<Mutex<Index>>,
    clock: &Arc<FakeClock>,
) -> (CoreHandler, JobPump, Arc<ValidatingSink>) {
    let events = Arc::new(PublisherSink::new(Publisher::detached()));
    let jobs_events = Arc::new(ValidatingSink::default());
    let hooks = ensure_empty_hooks_dir(dir).unwrap();
    let git = Arc::new(SystemGit::new(
        Arc::new(GitExec::new(test_git(), hooks)),
        Arc::new(GitSlots::new(4)),
        clock.clone(),
    ));
    let pump = JobPump::start(
        Arc::clone(index),
        git,
        clock.clone(),
        jobs_events.clone(),
        // UTC, so a test's local date never depends on the machine running it.
        Arc::new(FixedZone(0)),
    );
    let http: Arc<dyn codotheca_core::http::HttpTransport> =
        Arc::new(codotheca_core::testing::FakeTransport::new());
    let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        Arc::clone(&http),
        clock.clone(),
    ));
    let sync_http: Arc<dyn codotheca_core::http::HttpTransport> = sync_observing.clone();
    let handler = CoreHandler::new(CoreDeps {
        index: Arc::clone(index),
        provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&http),
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        http,
        client_id: "test-client-id".to_owned(),
        clock: clock.clone(),
        git: Arc::new(codotheca_core::testing::FakeGitBackend::new()),
        write_git: Arc::new(codotheca_core::testing::FakeMutatingGit::new(
            codotheca_core::testing::CloneBehaviour::Succeed,
        )),
        trash: Arc::new(codotheca_core::testing::CountingTrash::new()),
        mount: Arc::new(codotheca_core::testing::FakeMountResolver::default()),
        spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
        sessions: codotheca_core::session::manager::SessionManager::new(
            clock.clone(),
            events.clone(),
            Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
            Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
            Arc::new(FixedZone(0)),
        ),
        scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
            codotheca_core::testing::ScanLauncherFake::new(),
        )),
        scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
        firstrun: codotheca_core::firstrun::FirstRunEnv {
            sources: codotheca_core::firstrun::sources::SourceEnv {
                home: dir.to_path_buf(),
                app_data: None,
                xdg_config: None,
            },
            classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
                vec![],
            )),
            distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
            platform: codotheca_core::index::path::PathPlatform::Unix,
            skip: codotheca_core::scan::skiplist::SkipList::default(),
            cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
        },
        jobs: pump.clone(),
        sync: codotheca_core::assembly::sync::SyncPump::start(
            Arc::clone(index),
            codotheca_core::sync::SyncDeps {
                provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                    Arc::clone(&sync_http),
                    codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                )),
                transport: Arc::clone(&sync_observing),
                tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                clock: clock.clone(),
                cancel: codotheca_core::cancel::CancelToken::new(),
                zone: Arc::new(FixedZone(0)),
            },
            events.clone(),
        ),
        events,
        tz_offset_min: 0,
        zone: Arc::new(FixedZone(0)),
    });
    (handler, pump, jobs_events)
}

/// Pump `handler` once and wait until every job it queued has settled.
pub(crate) fn tick_and_settle(handler: &mut CoreHandler, pump: &JobPump) {
    handler.pump();
    let start = Instant::now();
    while !pump.is_idle() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the pass never settled"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One library, the handler a core process would run over it, and the clock that process reads.
pub(crate) struct LifelineRig {
    dir: tempfile::TempDir,
    pub(crate) index: Arc<Mutex<Index>>,
    pub(crate) clock: Arc<FakeClock>,
    handler: Option<CoreHandler>,
    pump: JobPump,
    /// Every event the current process's job pump emitted.
    pub(crate) jobs_events: Arc<ValidatingSink>,
    pairs: Vec<Pair>,
}

impl LifelineRig {
    /// An empty library at noon UTC on `D0`, its process not yet ticked.
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let at = D0 * DAY + DAY / 2;
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), at).unwrap()));
        let clock = Arc::new(FakeClock::new(at));
        let (handler, pump, jobs_events) = real_handler(dir.path(), &index, &clock);
        Self {
            dir,
            index,
            clock,
            handler: Some(handler),
            pump,
            jobs_events,
            pairs: Vec::new(),
        }
    }

    /// A project of **L**: acknowledged, live, not Reference, its authorship computed, on one
    /// present copy whose `main` tracks a bare remote it is level with.
    pub(crate) fn project(&mut self, name: &str) -> ProjectId {
        let project = self.unacknowledged(name);
        self.acknowledge(project);
        project
    }

    /// As [`Self::project`], but never acknowledged — outside **L**.
    pub(crate) fn unacknowledged(&mut self, name: &str) -> ProjectId {
        let remote = TestRepo::init_bare();
        let clone = TestRepo::init();
        clone.write("a.txt", b"one\n");
        clone.commit("first");
        let url = remote.path().to_string_lossy().into_owned();
        clone.git(&["remote", "add", "origin", &url]);
        clone.git(&["push", "-q", "-u", "origin", "main"]);
        let path = clone.path().to_string_lossy().into_owned();
        let mut guard = self.index.lock().unwrap();
        let (project, _location) = guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, created_at, updated_at)
                     VALUES (?1, ?1, ?1, 1, 0, 0, 0)",
                    [name],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
                Ok((project, LocationId(tx.last_insert_rowid())))
            })
            .unwrap();
        drop(guard);
        self.pairs.push(Pair {
            clone,
            _remote: remote,
            project,
        });
        project
    }

    /// Stamp `acknowledged_at`, which puts `project` in **L**.
    pub(crate) fn acknowledge(&self, project: ProjectId) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET acknowledged_at = ?2 WHERE id = ?1",
                rusqlite::params![project.0, D0 * DAY],
            )
            .unwrap();
        drop(guard);
    }

    /// `project`'s clone.
    pub(crate) fn clone_of(&self, project: ProjectId) -> &TestRepo {
        &self
            .pairs
            .iter()
            .find(|p| p.project == project)
            .expect("a project this rig planted")
            .clone
    }

    /// A local commit the remote does not have.
    pub(crate) fn commit_unpushed(&self, project: ProjectId) {
        let clone = self.clone_of(project);
        clone.write("b.txt", b"two\n");
        clone.commit("second");
    }

    /// Push `main` to the remote.
    pub(crate) fn push(&self, project: ProjectId) {
        self.clone_of(project)
            .git(&["push", "-q", "origin", "main"]);
    }

    /// One tick of the running process, every job it queued settled.
    pub(crate) fn tick(&mut self) {
        tick_and_settle(
            self.handler.as_mut().expect("a running handler"),
            &self.pump,
        );
    }

    /// Let `days` pass, then tick.
    pub(crate) fn after_days(&mut self, days: i64) {
        self.clock.advance(days * DAY);
        self.tick();
    }

    /// The day number the clock is on.
    pub(crate) fn today(&self) -> i64 {
        use codotheca_core::clock::Clock as _;
        self.clock.now_unix().div_euclid(DAY)
    }

    /// Stop this process and start another over the same index and clock, then tick it once:
    /// a second pass on the same date, through the first tick of a core start.
    pub(crate) fn restart(&mut self) {
        if let Some(mut old) = self.handler.take() {
            old.shutdown();
        }
        let (handler, pump, jobs_events) = real_handler(self.dir.path(), &self.index, &self.clock);
        self.handler = Some(handler);
        self.pump = pump;
        self.jobs_events = jobs_events;
        self.tick();
    }

    /// A command through the running handler.
    pub(crate) fn handle(&mut self, command: &str, args: serde_json::Value) -> serde_json::Value {
        self.handler
            .as_mut()
            .expect("a running handler")
            .handle(command, args)
            .unwrap_or_else(|e| panic!("{command}: {e:?}"))
    }

    /// `badges.lifeline_run`, decoded; `None` before the first close writes it.
    pub(crate) fn run(&self) -> Option<serde_json::Value> {
        let guard = self.index.lock().unwrap();
        let text = guard.app_meta("badges.lifeline_run").unwrap();
        drop(guard);
        text.map(|t| serde_json::from_str(&t).unwrap())
    }

    /// `(runStart, runEnd)` of the stored run.
    pub(crate) fn run_span(&self) -> (Option<String>, Option<String>) {
        let run = self.run().expect("a run state was written");
        let date = |key: &str| run[key].as_str().map(str::to_owned);
        (date("runStart"), date("runEnd"))
    }

    /// `project`'s stored `pushed` state and the day its pass location's refs were last read.
    pub(crate) fn reading(&self, project: ProjectId) -> (Option<String>, Option<i64>) {
        let guard = self.index.lock().unwrap();
        let conn = guard.conn();
        let state = conn
            .query_row(
                "SELECT state FROM project_check WHERE project_id = ?1 AND check_key = 'pushed'",
                [project.0],
                |r| r.get(0),
            )
            .ok();
        let observed: Option<i64> = conn
            .query_row(
                "SELECT refstate_observed_at FROM location WHERE project_id = ?1",
                [project.0],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        (state, observed.map(|t| t.div_euclid(DAY)))
    }

    /// The `job` of every `scan/job_done` the current process's pump emitted.
    pub(crate) fn jobs_done(&self) -> Vec<String> {
        self.jobs_events
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|(topic, event, _)| topic == "scan" && event == "job_done")
            .map(|(_, _, payload)| payload["job"].as_str().unwrap_or("?").to_owned())
            .collect()
    }
}

impl Drop for LifelineRig {
    fn drop(&mut self) {
        if let Some(mut handler) = self.handler.take() {
            handler.shutdown();
        }
    }
}

/// `day`'s `YYYY-MM-DD`.
pub(crate) fn date(day: i64) -> String {
    codotheca_core::clock::local_date(day)
}

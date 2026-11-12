//! One full scan by the production stack — `ThreadScanLauncher`, `SqliteScanStore`, real git —
//! and what it recorded: the run's problem rows and the locations it indexed.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::git::{GitExec, GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::protocol::{LocationId, ProjectId, ScanMode};
use codotheca_core::scan::launcher::{ScanLauncherDeps, ThreadScanLauncher};
use codotheca_core::scan::presence::ScanStore as _;
use codotheca_core::scan::run::platform_of;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::testing::{FakeClock, FakeMountResolver, ScanEventFake};

use super::TestRepo;

const NOW: i64 = 1_700_000_000;

const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// Counts the locations the hand-off indexed.
#[derive(Debug, Default)]
struct RecordingJobs {
    indexed: Mutex<Vec<i64>>,
}

impl codotheca_core::jobs::JobSink for RecordingJobs {
    fn on_location_indexed(
        &self,
        _project: ProjectId,
        location: LocationId,
        _store_key: &str,
        _store_kind: StoreClass,
    ) {
        self.indexed.lock().unwrap().push(location.0);
    }

    fn on_visible(&self, _: ProjectId, _: LocationId, _: &str, _: StoreClass, _: bool, _: bool) {}
    fn on_recheck(&self, _: ProjectId, _: LocationId, _: &str, _: StoreClass) {}
}

/// What one scan recorded.
pub(crate) struct Scanned {
    /// `problem_count` for the run: the figure §11.1's header shows.
    pub(crate) problem_rows: u64,
    /// `(kind, pathDisplay)` of every `scan/problem` event.
    pub(crate) problems: Vec<(String, String)>,
    /// Locations handed to the scheduler.
    pub(crate) indexed: usize,
    /// `(path_display, repo_kind)` of every `location` row.
    pub(crate) locations: Vec<(String, String)>,
    /// The index the scan wrote, for what reads its rows next.
    pub(crate) index: Arc<Mutex<Index>>,
    /// The index's directory when the rig made it; `None` when the caller keeps its own.
    _dir: Option<tempfile::TempDir>,
}

/// One full scan of `root`, with `repo`'s git underneath.
pub(crate) fn scan(repo: &TestRepo, root: &Path) -> Scanned {
    scan_with(repo.exec(), root)
}

/// One full scan of `root`, running git through `exec`.
pub(crate) fn scan_with(exec: GitExec, root: &Path) -> Scanned {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(dir.path()).unwrap()));
    Scanned {
        _dir: Some(dir),
        ..scan_into(exec, root, &index)
    }
}

/// The mount table every scan here runs under: `root` is one local store, `store-a`.
pub(crate) fn mounts_for(root: &Path) -> FakeMountResolver {
    let mounts = FakeMountResolver::new();
    mounts.map(
        root,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    mounts
}

/// [`scan`] into an index the caller keeps, so a later command can read what it recorded.
/// Adds `root` as an enabled `scan_root` first.
pub(crate) fn scan_in(repo: &TestRepo, root: &Path, index: &Arc<Mutex<Index>>) -> Scanned {
    scan_into(repo.exec(), root, index)
}

/// [`scan_in`], running git through `exec`.
fn scan_into(exec: GitExec, root: &Path, index: &Arc<Mutex<Index>>) -> Scanned {
    index
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                    enabled, added_by, descend_into_repos, added_at)
             VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, 1)",
            rusqlite::params![
                host_kind(),
                root.to_string_lossy().as_bytes(),
                path_key(root, platform_of(host_kind())),
                root.display().to_string()
            ],
        )
        .unwrap();
    let store = Arc::new(SqliteScanStore::new(Arc::clone(index)));
    let mounts = mounts_for(root);
    let clock = Arc::new(FakeClock::new(NOW));
    let events = Arc::new(ScanEventFake::default());
    let jobs = Arc::new(RecordingJobs::default());
    let launcher = Arc::new(ThreadScanLauncher::new(ScanLauncherDeps {
        store: store.clone(),
        index: Arc::clone(index),
        git: Arc::new(SystemGit::new(
            Arc::new(exec),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        mounts: Arc::new(mounts),
        clock,
        exclusions: Arc::default(),
        wsl: None,
        jobs: jobs.clone(),
        events: events.clone(),
    }));
    let scans = ScanSupervisor::new(launcher);
    scans.start(ScanMode::Full, NOW).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while scans.live().is_some() {
        assert!(Instant::now() < deadline, "the scan never finished");
        std::thread::sleep(Duration::from_millis(5));
    }

    let run = store.latest_scan_run().unwrap().expect("a scan_run row");
    let problem_rows = store.problem_count(run.id).unwrap();
    let problems = events
        .named("scan", "problem")
        .iter()
        .map(|p| {
            (
                p["kind"].as_str().unwrap_or_default().to_owned(),
                p["pathDisplay"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let indexed = jobs.indexed.lock().unwrap().len();
    let locations = index
        .lock()
        .unwrap()
        .conn()
        .prepare("SELECT path_display, repo_kind FROM location ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    Scanned {
        problem_rows,
        problems,
        indexed,
        locations,
        index: Arc::clone(index),
        _dir: None,
    }
}

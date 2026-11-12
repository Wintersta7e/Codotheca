//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.1's header counts folders: one scan run records at most one problem row per path.
//!
//! These run the production launcher over a real tree with real git, through the real
//! `SqliteScanStore`, because the double count was the walk and the hand-off each reporting
//! the same folder — a fake on either side would hide one of the two writers.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::protocol::{LocationId, ProjectId, ScanMode};
use codotheca_core::scan::launcher::{ScanLauncherDeps, ThreadScanLauncher};
use codotheca_core::scan::presence::ScanStore as _;
use codotheca_core::scan::run::platform_of;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::testing::{FakeClock, FakeMountResolver, ScanEventFake};
use support::TestRepo;

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
}

struct Scanned {
    problem_rows: u64,
    problem_paths: Vec<String>,
    indexed: usize,
}

/// One full scan of `root` by the production stack, real git underneath.
fn scan(repo: &TestRepo, root: &Path) -> Scanned {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(dir.path()).unwrap()));
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
    let store = Arc::new(SqliteScanStore::new(Arc::clone(&index)));
    let mounts = FakeMountResolver::new();
    mounts.map(
        root,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    let clock = Arc::new(FakeClock::new(NOW));
    let events = Arc::new(ScanEventFake::default());
    let jobs = Arc::new(RecordingJobs::default());
    let launcher = Arc::new(ThreadScanLauncher::new(ScanLauncherDeps {
        store: store.clone(),
        index: Arc::clone(&index),
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        mounts: Arc::new(mounts),
        clock,
        skip: Arc::new(SkipList::default()),
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
    let problem_paths = events
        .named("scan", "problem")
        .iter()
        .map(|p| p["pathDisplay"].as_str().unwrap_or_default().to_owned())
        .collect();
    let indexed = jobs.indexed.lock().unwrap().len();
    Scanned {
        problem_rows,
        problem_paths,
        indexed,
    }
}

/// A `.git` file whose pointer leads nowhere: the walk's probe fails and classifies it from the
/// pointer, and the hand-off then cannot index it. One folder, one row — the hand-off's, which
/// carries git's own error — beside a healthy repository that indexes and reports nothing.
#[test]
fn a_broken_gitdir_pointer_is_one_problem_row_not_two() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"a\n");
    repo.commit("first");
    let root = support::canonical(repo.scratch());
    let lane = root.join("lane-a");
    std::fs::create_dir_all(&lane).unwrap();
    std::fs::write(
        lane.join(".git"),
        format!(
            "gitdir: {}\n",
            root.join("repo/.git/worktrees/lane-a").display()
        ),
    )
    .unwrap();

    let scanned = scan(&repo, &root);

    assert_eq!(scanned.indexed, 1, "the healthy repository still indexes");
    assert_eq!(
        scanned.problem_rows, 1,
        "the header counts folders: {:?}",
        scanned.problem_paths
    );
    assert_eq!(scanned.problem_paths.len(), 1);
    assert!(
        scanned.problem_paths.iter().all(|p| p.contains("lane-a")),
        "the row names the broken folder: {:?}",
        scanned.problem_paths
    );
}

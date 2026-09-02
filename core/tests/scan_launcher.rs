//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! **R40's second half** — the production `ScanLauncher`.
//!
//! Nothing in the plan set produced one: only plan 07's `ScanLauncherFake`, which is what made
//! plan 21's Task 7 uncompletable. This runs a real walk over a real temp tree, through the real
//! `SqliteScanStore`, and asserts what reaches the `scan` topic.

use codotheca_core::git::RepoFacts;
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::protocol::ScanMode;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::scan::launcher::ThreadScanLauncher;
use codotheca_core::scan::presence::ScanStore;
use codotheca_core::scan::run::platform_of;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, GitReply, ScanEventFake,
};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NOW: i64 = 1_700_000_000;

fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

fn seed_root(index: &Arc<Mutex<Index>>, path: &Path) {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .execute(
            "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                    enabled, added_by, descend_into_repos, added_at)
             VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, 1)",
            rusqlite::params![
                host_kind(),
                path.to_string_lossy().as_bytes(),
                path_key(path, platform_of(host_kind())),
                path.display().to_string()
            ],
        )
        .unwrap();
}

fn repo_at(base: &Path, rel: &str) -> std::path::PathBuf {
    let p = base.join(rel);
    std::fs::create_dir_all(p.join(".git")).unwrap();
    p
}

/// Spin until the worker has reported the run stopped, or fail loudly. A bare sleep would either
/// be flaky or slow; this is bounded and says what it was waiting for.
fn await_idle(scans: &ScanSupervisor) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if scans.live().is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the scan worker never cleared the supervisor's slot");
}

/// Counts the hand-offs the production launcher makes. `NullJobSink` used to be wired in here,
/// and it is why a scan produced a library nothing ever computed a fact about.
#[derive(Debug, Default)]
struct RecordingJobs {
    indexed: Mutex<Vec<(i64, i64)>>,
}

impl codotheca_core::jobs::JobSink for RecordingJobs {
    fn on_location_indexed(
        &self,
        project: ProjectId,
        location: LocationId,
        _store_key: &str,
        _store_kind: StoreClass,
    ) {
        self.indexed.lock().unwrap().push((project.0, location.0));
    }

    fn on_visible(&self, _: ProjectId, _: LocationId, _: &str, _: StoreClass) {}
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    store: Arc<SqliteScanStore>,
    events: Arc<ScanEventFake>,
    jobs: Arc<RecordingJobs>,
    scans: Arc<ScanSupervisor>,
}

fn rig(root: &Path) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(dir.path()).unwrap()));
    seed_root(&index, root);

    let index_for_rig = Arc::clone(&index);
    let store = Arc::new(SqliteScanStore::new(Arc::clone(&index)));
    let events = Arc::new(ScanEventFake::default());
    let mounts = FakeMountResolver::new();
    mounts.map(
        root,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    // The fixtures are bare `.git` directories, so the hand-off's probe answers from the fake.
    // One common dir for all of them: two locations under one project is a real shape, and the
    // question this file asks is what reaches the `scan` topic, not how identity decides.
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: root.join(".git"),
        common_dir: root.join(".git"),
    }));
    git.always_root_commits(GitReply::Ok(Vec::new()));
    git.always_remote_urls(GitReply::Ok(Vec::new()));
    let jobs = Arc::new(RecordingJobs::default());
    let launcher = Arc::new(ThreadScanLauncher::new(
        codotheca_core::scan::launcher::ScanLauncherDeps {
            store: Arc::clone(&store) as Arc<_>,
            index: Arc::clone(&index),
            git: Arc::new(git),
            mounts: Arc::new(mounts),
            clock: Arc::new(FakeClock::new(NOW)),
            skip: Arc::new(SkipList::default()),
            jobs: Arc::clone(&jobs) as Arc<_>,
            events: Arc::clone(&events) as Arc<_>,
        },
    ));
    Rig {
        _dir: dir,
        index: index_for_rig,
        store,
        events,
        jobs,
        scans: Arc::new(ScanSupervisor::new(launcher as Arc<_>)),
    }
}

#[test]
fn a_launched_run_walks_finishes_and_clears_its_own_slot() {
    let tree = tempfile::tempdir().unwrap();
    repo_at(tree.path(), "a");
    repo_at(tree.path(), "b");
    let r = rig(tree.path());

    let (live, started) = r.scans.start(ScanMode::Full, NOW).unwrap();
    assert!(started);
    assert_eq!(live.generation, 1);
    assert!(
        live.roots.len() == 1,
        "the launcher reports the roots it actually enabled"
    );
    await_idle(&r.scans);

    let row = r
        .store
        .latest_scan_run()
        .unwrap()
        .expect("the launcher wrote a scan_run row before returning");
    assert_eq!(row.id, live.scan_run_id.0);
    assert_eq!(row.mode, "full");
    assert_eq!(row.ended_at, Some(NOW), "finish_scan_run ran");
    assert_eq!(row.found_repos, 2);
    assert!(!row.cancelled);
    assert!(row.walked_dirs >= 3, "the root plus its two repositories");
}

/// The launcher is what maps `WalkEvent` onto the `scan` topic (R8). It publishes `finished`;
/// `run_started` is the *command's*, because only the command knows a run actually started
/// rather than coalescing.
#[test]
fn a_finished_run_publishes_progress_and_finished_on_the_scan_topic() {
    let tree = tempfile::tempdir().unwrap();
    repo_at(tree.path(), "a");
    let r = rig(tree.path());

    r.scans.start(ScanMode::Incremental, NOW).unwrap();
    await_idle(&r.scans);

    let finished = r.events.named("scan", "finished");
    assert_eq!(finished.len(), 1);
    let payload = finished.first().unwrap();
    assert_eq!(payload["foundRepos"], 1);
    assert_eq!(payload["endedAt"], NOW);
    assert_eq!(
        payload["problemCount"], 0,
        "counted from scan_problem, not assumed"
    );
    assert_eq!(payload["ambiguousLineageCount"], 0);

    assert!(
        !r.events.named("scan", "progress").is_empty(),
        "at least the discovery of one repository reports progress"
    );
    assert!(
        r.events.named("scan", "cancelled").is_empty(),
        "a run that completed is not a run that was cancelled"
    );
    assert!(
        r.events.named("scan", "run_started").is_empty(),
        "run_started belongs to scan.start, which alone knows it did not coalesce"
    );
}

/// §4.8: a cancelled run stops, writes `cancelled = 1` on its own row, and applies no presence.
#[test]
fn a_cancelled_run_publishes_cancelled_and_marks_its_row() {
    let tree = tempfile::tempdir().unwrap();
    repo_at(tree.path(), "a");
    let r = rig(tree.path());

    let (live, _) = r.scans.start(ScanMode::Full, NOW).unwrap();
    live.cancel.cancel();
    await_idle(&r.scans);

    let row = r.store.latest_scan_run().unwrap().expect("row");
    assert!(row.cancelled);
    assert_eq!(row.ended_at, Some(NOW));
    let cancelled = r.events.named("scan", "cancelled");
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled.first().unwrap()["endedAt"], NOW);
    assert!(
        r.events.named("scan", "finished").is_empty(),
        "a cancelled run did not finish, and saying so would be a lie about the library"
    );
}

/// A repository the walk could not read reaches `scan_problem` and the topic. `repo_found` is
/// **not** published: its payload needs a `projectId` and a `LocationRef`, which only identity
/// resolution produces, and fabricating either would put a project id on the wire for a project
/// that does not exist.
#[test]
fn an_unreadable_root_reaches_scan_problem_and_the_topic() {
    let tree = tempfile::tempdir().unwrap();
    let missing = tree.path().join("never-existed");
    let r = rig(&missing);

    r.scans.start(ScanMode::Full, NOW).unwrap();
    await_idle(&r.scans);

    let row = r.store.latest_scan_run().unwrap().expect("row");
    assert_eq!(r.store.problem_count(row.id).unwrap(), 1);
    let problems = r.events.named("scan", "problem");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems.first().unwrap()["kind"], "offline_store");
    assert!(
        problems.first().unwrap()["detail"]
            .as_str()
            .is_some_and(|d| d.contains("not reachable")),
        "the diagnostic reaches the shell, which owns the prose the user reads"
    );
    assert!(
        problems.first().unwrap()["pathDisplay"]
            .as_str()
            .is_some_and(|p| p.contains("never-existed")),
        "a problem row with no path is unactionable in §11.1's window"
    );
    assert!(
        r.events.named("scan", "repo_found").is_empty(),
        "repo_found needs a projectId, and identity resolution is plan 08's"
    );
}

/// The production launcher installs the real queue, not `NullJobSink`.
///
/// That substitution is the whole reason a scan produced a library nothing computed a fact
/// about, and it would pass every other assertion in this file.
#[test]
fn the_production_launcher_hands_every_indexed_location_to_the_scheduler() {
    let tree = tempfile::tempdir().unwrap();
    repo_at(tree.path(), "a");
    repo_at(tree.path(), "b");
    let r = rig(tree.path());

    r.scans.start(ScanMode::Full, NOW).unwrap();
    await_idle(&r.scans);

    let handed = r.jobs.indexed.lock().unwrap().clone();
    assert_eq!(
        handed.len(),
        2,
        "one per repository the walk found: {handed:?}"
    );
    let guard = r.index.lock().unwrap();
    let locations: i64 = guard
        .conn()
        .query_row("SELECT COUNT(*) FROM location", [], |row| row.get(0))
        .unwrap();
    drop(guard);
    assert_eq!(locations, 2, "and each one is a row, not just an event");
}

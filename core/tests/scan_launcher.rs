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

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::RepoFacts;
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::protocol::ScanMode;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::scan::launcher::ThreadScanLauncher;
use codotheca_core::scan::presence::{ScanStore, ScanStoreError};
use codotheca_core::scan::run::platform_of;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::{LaunchedScan, ScanLauncher, ScanProgressCell, ScanSupervisor};
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, GitReply, ScanEventFake,
};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NOW: i64 = 1_700_000_000;

const fn host_kind() -> &'static str {
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
    std::fs::write(p.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
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

    fn on_visible(&self, _: ProjectId, _: LocationId, _: &str, _: StoreClass, _: bool, _: bool) {}
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
    rig_with(&[root], shared_git(root), |launcher| launcher)
}

/// The fixtures are bare `.git` directories, so the hand-off's probe answers from the fake.
/// One common dir for all of them: two locations under one project is a real shape, and the
/// question most of this file asks is what reaches the `scan` topic, not how identity decides.
fn shared_git(root: &Path) -> FakeGitBackend {
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: root.join(".git"),
        common_dir: root.join(".git"),
    }));
    git.always_root_commits(GitReply::Ok(Vec::new()));
    git.always_remote_urls(GitReply::Ok(Vec::new()));
    git
}

/// `wrap` sits between the real `ThreadScanLauncher` and the supervisor. Only the cancellation
/// test uses it, and it takes the seam rather than a flag so every other test builds the exact
/// production stack. Every root is seeded enabled, in order, on one store.
fn rig_with(
    roots: &[&Path],
    git: FakeGitBackend,
    wrap: impl FnOnce(Arc<dyn ScanLauncher>) -> Arc<dyn ScanLauncher>,
) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(dir.path()).unwrap()));
    for root in roots {
        seed_root(&index, root);
    }

    let index_for_rig = Arc::clone(&index);
    let store = Arc::new(SqliteScanStore::new(Arc::clone(&index)));
    let events = Arc::new(ScanEventFake::default());
    let mounts = FakeMountResolver::new();
    for root in roots {
        mounts.map(
            *root,
            MountFacts {
                store_key: "store-a".to_owned(),
                volume_key: Some("vol-a".to_owned()),
                class: StoreClass::Local,
            },
        );
    }
    let jobs = Arc::new(RecordingJobs::default());
    let launcher = Arc::new(ThreadScanLauncher::new(
        codotheca_core::scan::launcher::ScanLauncherDeps {
            store: store.clone(),
            index: Arc::clone(&index),
            git: Arc::new(git),
            mounts: Arc::new(mounts),
            clock: Arc::new(FakeClock::new(NOW)),
            skip: Arc::new(SkipList::default()),
            wsl: None,
            jobs: jobs.clone(),
            events: events.clone(),
        },
    ));
    Rig {
        _dir: dir,
        index: index_for_rig,
        store,
        events,
        jobs,
        scans: Arc::new(ScanSupervisor::new(wrap(launcher))),
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

/// Cancels the run's token **before** the walk is launched, so the cancellation the test below
/// asserts is a fact rather than a race.
///
/// `ScanSupervisor::start` creates the token and hands it to `launch`, so a test can only reach it
/// after `start` has returned — by which time a one-repository tree has often already been walked,
/// and the run finished normally. That is not hypothetical: it passed on this machine and on the
/// `core` runner and failed in the `acceptance` job on the same commit, `assertion failed:
/// row.cancelled`. Cancelling here is deterministic because `core/src/scan/run.rs:415` reads the
/// token before the walk loop, and `launch` writes the `scan_run` row on the calling thread.
struct CancelAtLaunch(Arc<dyn ScanLauncher>);

impl ScanLauncher for CancelAtLaunch {
    fn launch(
        &self,
        mode: ScanMode,
        now: i64,
        cancel: CancelToken,
        progress: Arc<ScanProgressCell>,
    ) -> Result<LaunchedScan, ScanStoreError> {
        cancel.cancel();
        self.0.launch(mode, now, cancel, progress)
    }
}

/// §4.8: a cancelled run stops, writes `cancelled = 1` on its own row, and applies no presence.
#[test]
fn a_cancelled_run_publishes_cancelled_and_marks_its_row() {
    let tree = tempfile::tempdir().unwrap();
    repo_at(tree.path(), "a");
    let r = rig_with(&[tree.path()], shared_git(tree.path()), |launcher| {
        Arc::new(CancelAtLaunch(launcher))
    });

    let (_live, _) = r.scans.start(ScanMode::Full, NOW).unwrap();
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

/// A fake whose every repository under `names` is its own project: each resolves to its own
/// common dir. `copies` replies are queued per repository because each run probes once.
fn distinct_git(base: &Path, names: &[&str], copies: usize) -> FakeGitBackend {
    let git = shared_git(base);
    for name in names {
        let git_dir = base.join(name).join(".git");
        for _ in 0..copies {
            git.on_repo_facts(
                Some(name),
                GitReply::Ok(RepoFacts {
                    is_bare: false,
                    is_shallow: false,
                    git_dir: git_dir.clone(),
                    common_dir: git_dir.clone(),
                }),
            );
        }
    }
    git
}

fn project_rows(index: &Arc<Mutex<Index>>) -> i64 {
    let guard = index.lock().unwrap();
    let n = guard
        .conn()
        .query_row("SELECT COUNT(*) FROM project", [], |row| row.get(0))
        .unwrap();
    drop(guard);
    n
}

fn last_indexed_projects(events: &ScanEventFake) -> serde_json::Value {
    events
        .named("scan", "progress")
        .last()
        .expect("a run that found repositories reports progress")["indexedProjects"]
        .clone()
}

/// §10.3a's headline is `indexedProjects`. It used to be the count at `begin`, re-sent on every
/// frame, so a first run read `0 projects` from its first repository to its last.
#[test]
fn the_last_progress_counts_every_project_the_library_holds_as_it_commits() {
    let tree = tempfile::tempdir().unwrap();
    let names = ["p0", "p1", "p2"];
    for name in names {
        repo_at(tree.path(), name);
    }
    let r = rig_with(
        &[tree.path()],
        distinct_git(tree.path(), &names, 2),
        |launcher| launcher,
    );

    r.scans.start(ScanMode::Full, NOW).unwrap();
    await_idle(&r.scans);
    assert_eq!(project_rows(&r.index), 3, "the fixture is three projects");
    assert_eq!(last_indexed_projects(&r.events), 3);

    // A second run re-indexes the same three: the library still holds three, not six.
    r.scans.start(ScanMode::Full, NOW).unwrap();
    await_idle(&r.scans);
    assert_eq!(project_rows(&r.index), 3);
    assert_eq!(last_indexed_projects(&r.events), 3);
}

/// A directory holding `n` empty subdirectories: enough to cross the walk's 512-directory
/// progress interval.
fn dirs_under(base: &Path, n: usize) {
    for i in 0..n {
        std::fs::create_dir_all(base.join(format!("d{i:04}"))).unwrap();
    }
}

/// `walkedDirs` is a running count. Discoveries used to send `0` between the walk's counter
/// frames, and a second root's counter restarted below the first root's total, so the first-run
/// figure fell back as it went.
#[test]
fn walked_dirs_never_falls_and_carries_across_roots() {
    let tree = tempfile::tempdir().unwrap();
    let first = tree.path().join("first");
    let second = tree.path().join("second");
    dirs_under(&first, 600);
    dirs_under(&second, 600);
    for name in ["r0", "r1", "r2"] {
        repo_at(&second, name);
    }
    let r = rig_with(&[&first, &second], shared_git(&second), |launcher| launcher);

    r.scans.start(ScanMode::Full, NOW).unwrap();
    await_idle(&r.scans);

    let walked: Vec<i64> = r
        .events
        .named("scan", "progress")
        .iter()
        .map(|p| p["walkedDirs"].as_i64().unwrap())
        .collect();
    assert!(
        walked.len() >= 4,
        "two counter frames and three discoveries: {walked:?}"
    );
    assert!(
        walked.windows(2).all(|w| w[0] <= w[1]),
        "a progress frame lowered the walked count: {walked:?}"
    );
    let last = *walked.last().unwrap();
    assert!(
        last > 601,
        "the second root's counter adds to the first root's 601 directories: {walked:?}"
    );
    let finished = r.events.named("scan", "finished");
    assert!(
        finished.first().unwrap()["walkedDirs"].as_i64().unwrap() >= last,
        "the run's total is at least what progress last said"
    );
}

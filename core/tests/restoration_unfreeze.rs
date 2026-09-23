//! §30.1's Unfreeze boundary on the real write path: **the first observation after a store comes
//! back writes no `health_delta`**, whatever changed while it was away.
//!
//! A store that goes offline freezes its projects; nothing sweeps a root that is not there, so
//! the evidence stored for it is whatever was observed before it left. When it returns, the next
//! sweep diffs against that pre-freeze evidence — and without a record of the return, a TODO
//! closed on another machine while the drive was unplugged reads as a restoration the user just
//! performed, surge and all. These tests drive the product's own path end to end: the walk's
//! hand-off, the presence sweep, J1 and J7 through `run_one` and the real `JobRunner`.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx};
use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitBackend, GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{DebtSource, HealthCheckSwitch, LocationId, ProjectId};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::presence::{apply_presence, PresenceContext, ScanRootRow};
use codotheca_core::scan::run::{platform_of, Discovered};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::testing::FakeClock;
use support::TestRepo;

const T0: i64 = 1_700_000_000;
const STORE: &str = "store-a";

#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, _payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned()));
    }
}

impl RecordingSink {
    fn health_deltas(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(topic, event)| topic == "projects" && event == "health_delta")
            .count()
    }
}

/// One real repository, indexed through the walk's hand-off, enrolled, authored, granted, with
/// `overgrowth` fed by `todo_marker` alone so the layer moves exactly when J7's item set does.
struct Rig {
    repo: TestRepo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    git: Arc<dyn GitBackend>,
    clock: Arc<FakeClock>,
    project: ProjectId,
    location: LocationId,
    generation: std::cell::Cell<i64>,
}

impl Rig {
    fn new(switched_off: &[DebtSource], grant: bool) -> Rig {
        let repo = TestRepo::init();
        repo.write("src/a.rs", markers(5).as_bytes());
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), T0).unwrap()));
        let clock = Arc::new(FakeClock::new(T0));
        let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        ));
        let mut rig = Rig {
            repo,
            _dir: dir,
            index,
            git,
            clock,
            project: ProjectId(0),
            location: LocationId(0),
            generation: std::cell::Cell::new(0),
        };
        let indexed = rig.hand_off();
        rig.project = indexed.0;
        rig.location = indexed.1;
        let mut guard = rig.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                tx.execute(
                    "UPDATE project SET authored_by_user = 1, is_reference = 0,
                                        acknowledged_at = ?2
                      WHERE id = ?1",
                    rusqlite::params![rig.project.0, T0 - 100],
                )?;
                if grant {
                    tx.execute(
                        "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                        [],
                    )?;
                }
                let off: Vec<HealthCheckSwitch> = switched_off
                    .iter()
                    .map(|&check| HealthCheckSwitch {
                        check,
                        enabled: false,
                    })
                    .collect();
                write_switches(tx, &off)?;
                Ok(())
            })
            .unwrap();
        drop(guard);
        rig
    }

    /// The walk found the repository: the production hand-off, one generation later each time.
    fn hand_off(&self) -> (ProjectId, LocationId) {
        self.generation.set(self.generation.get() + 1);
        let path = self.repo.path();
        let handle = RepoHandle::resolve(path, StoreKey::new(STORE), StoreClass::Local).unwrap();
        let discovered = Discovered {
            candidate: RepoCandidate {
                path: path.to_path_buf(),
                kind: RepoKind::WorkTree,
                git_dir: handle.git_dir.clone(),
                common_dir: handle.common_dir.clone(),
            },
            root_id: 1,
            kind: "linux".to_owned(),
            distro: String::new(),
            path_bytes: path_bytes(path),
            path_key: path_key(path, platform_of("linux")),
            path_display: path_display(path),
            store_key: STORE.to_owned(),
            volume_key: Some("vol-a".to_owned()),
        };
        let cancel = CancelToken::new();
        let ctx = HandoffCtx {
            git: self.git.as_ref(),
            cancel: &cancel,
            store_class: StoreClass::Local,
            generation: self.generation.get(),
            now: T0,
        };
        let indexed = hand_off_discovered(&self.index, &ctx, &discovered).unwrap();
        (indexed.project, indexed.location)
    }

    /// A completed walk that could not reach the store: §4.6's own presence sweep marks the
    /// location `offline`, and its project freezes.
    fn store_goes_away(&self) {
        self.generation.set(self.generation.get() + 1);
        let parent: &Path = self.repo.path().parent().unwrap();
        let roots = [ScanRootRow {
            root_id: 1,
            kind: "linux".to_owned(),
            distro: String::new(),
            path_bytes: path_bytes(parent),
            path_key: path_key(parent, platform_of("linux")),
            enabled: true,
            descend_into_repos: false,
        }];
        let skip = SkipList::default();
        let present_stores = BTreeSet::new();
        let ctx = PresenceContext {
            generation: self.generation.get(),
            roots: &roots,
            skip: &skip,
            present_stores: &present_stores,
        };
        let store = SqliteScanStore::new(Arc::clone(&self.index));
        let summary = apply_presence(&store, &ctx).unwrap();
        assert_eq!(summary.offline, 1, "the fixture's store did not go offline");
    }

    fn deps(&self) -> JobDeps {
        JobDeps {
            git: Arc::clone(&self.git),
            clock: Arc::clone(&self.clock) as Arc<dyn codotheca_core::clock::Clock>,
            cancel: CancelToken::new(),
            tz_offset_min: 0,
        }
    }

    fn job(&self, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: self.project,
            location_id: self.location,
            store_key: STORE.to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Interactive,
        }
    }

    /// J1 alone, through the production dispatch: it records the head J7 compares against.
    fn refstate(&self) {
        let announce = RefCell::new(Vec::new());
        run_one(
            &self.index,
            &self.deps(),
            &self.job(JobKind::J1Refstate),
            &announce,
        )
        .unwrap();
    }

    /// One job through the real runner — its execute, its settle, its announcements — spun until
    /// nothing is due. Returns how many `health_delta` events the runner announced.
    fn run_job(&self, kind: JobKind) -> usize {
        let events = Arc::new(RecordingSink::default());
        let runner = JobRunner::new(
            Arc::clone(&self.index),
            self.deps(),
            Arc::clone(&events) as Arc<dyn EventSink>,
        );
        assert!(runner.enqueue(self.job(kind)));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(20));
        }
        runner.request_stop();
        runner.join();
        events.health_deltas()
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

    fn rows(&self) -> i64 {
        self.count("SELECT count(*) FROM health_delta WHERE project_id = ?1")
    }

    fn open_todos(&self) -> i64 {
        self.count(
            "SELECT count(*) FROM debt_item
              WHERE project_id = ?1 AND source = 'todo_marker' AND state = 'open'",
        )
    }

    fn todo_sweep(&self) -> String {
        let guard = self.index.lock().unwrap();
        let outcome = guard
            .conn()
            .query_row(
                "SELECT outcome FROM debt_sweep WHERE project_id = ?1 AND source = 'todo_marker'",
                [self.project.0],
                |r| r.get(0),
            )
            .unwrap();
        drop(guard);
        outcome
    }

    /// Scan the repository's HEAD for markers, the way a returning walk queues it.
    fn content_scan(&self) -> usize {
        self.refstate();
        self.run_job(JobKind::J7Markers)
    }
}

fn markers(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        let _ = writeln!(body, "// TODO: marker number {i}");
    }
    body
}

/// `todo_marker` alone feeds `overgrowth`.
const TODO_ONLY: [DebtSource; 2] = [DebtSource::MissingTests, DebtSource::UnpushedCommits];

/// **The case this file exists for.** Five TODOs, the drive goes away, two are removed and
/// committed elsewhere, the drive comes back: J7 rescans the moved head and closes the two — and
/// writes no `health_delta` row and announces no event, because both ends of that difference are
/// not observations of one continuous presence.
#[test]
fn a_todo_closed_while_the_store_was_away_writes_no_delta_when_it_returns() {
    let rig = Rig::new(&TODO_ONLY, true);
    assert_eq!(rig.content_scan(), 0, "a first observation was announced");
    assert_eq!(rig.open_todos(), 5);
    assert_eq!(rig.rows(), 0);

    rig.store_goes_away();
    rig.repo.write("src/a.rs", markers(3).as_bytes());
    rig.repo
        .commit("two fewer, committed while the drive was elsewhere");
    rig.hand_off();

    let announced = rig.content_scan();
    let rows = rig.rows();
    eprintln!(
        "after the return: {} open, {rows} health_delta row(s), {announced} event(s)",
        rig.open_todos()
    );
    assert_eq!(
        rig.open_todos(),
        3,
        "the returning scan did not observe the closures"
    );
    assert_eq!(rig.todo_sweep(), "complete");
    assert_eq!(rows, 0, "a delta crossed the unfreeze");
    assert_eq!(
        announced, 0,
        "a restoration was announced for work done while away"
    );
}

/// The control: the same two closures on a store that never left write exactly one row and
/// announce exactly one event — the rule above is the unfreeze, not a producer that never writes.
#[test]
fn a_todo_closed_while_the_store_stayed_present_writes_one_delta() {
    let rig = Rig::new(&TODO_ONLY, true);
    assert_eq!(rig.content_scan(), 0);

    rig.repo.write("src/a.rs", markers(3).as_bytes());
    rig.repo.commit("two fewer");
    rig.hand_off();

    let announced = rig.content_scan();
    let rows = rig.rows();
    eprintln!("online close: {rows} health_delta row(s), {announced} event(s)");
    assert_eq!(rows, 1);
    assert_eq!(announced, 1);
    let (layer, from, to): (String, f64, f64) = {
        let guard = rig.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT layer, from_value, to_value FROM health_delta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        drop(guard);
        row
    };
    assert_eq!((layer.as_str(), from, to), ("overgrowth", 5.0, 3.0));
}

/// A store that comes back with its HEAD where it left it: J7's evidence is re-observed rather
/// than trusted — no row, and the items read open again rather than staying *not counted* until
/// the next commit — and the next genuine close afterwards writes its row as usual.
#[test]
fn an_unchanged_head_is_re_observed_when_the_store_returns() {
    let rig = Rig::new(&TODO_ONLY, true);
    rig.content_scan();
    assert_eq!(rig.open_todos(), 5);

    rig.store_goes_away();
    rig.hand_off();
    let announced = rig.content_scan();
    let unverified =
        rig.count("SELECT count(*) FROM debt_item WHERE project_id = ?1 AND state = 'unverified'");
    eprintln!(
        "unchanged head after the return: sweep {}, {} open, {unverified} unverified, {} row(s)",
        rig.todo_sweep(),
        rig.open_todos(),
        rig.rows()
    );
    assert_eq!(
        rig.todo_sweep(),
        "complete",
        "J7 skipped the unchanged head and left the markers uncounted"
    );
    assert_eq!(rig.open_todos(), 5);
    assert_eq!(unverified, 0);
    assert_eq!(rig.rows(), 0);
    assert_eq!(announced, 0);

    // Present again, so the next close is an ordinary one.
    rig.repo.write("src/a.rs", markers(4).as_bytes());
    rig.repo.commit("one fewer");
    rig.hand_off();
    assert_eq!(rig.content_scan(), 1);
    assert_eq!(rig.rows(), 1);
}

/// **J7 is not the only source of this shape.** A singleton is swept at every job settle, so it
/// records `unobservable` while a store is away — but only if some job for that project settles
/// during the freeze, and nobody opens a project whose drive is unplugged. Here none does: a tag
/// made while the drive was elsewhere closes `no_release` at the first J1 settle after it returns,
/// and that settle writes no row.
#[test]
fn a_release_made_while_the_store_was_away_writes_no_delta_when_it_returns() {
    let rig = Rig::new(
        &[DebtSource::MissingReadme, DebtSource::MissingLicense],
        false,
    );
    rig.run_job(JobKind::J1Refstate);
    assert_eq!(
        rig.count(
            "SELECT count(*) FROM debt_item
              WHERE project_id = ?1 AND source = 'no_release' AND state = 'open'"
        ),
        1,
        "the fixture opened no no_release item"
    );
    // The chain after J1 may have classified the fixture; the reading needs it authored.
    {
        let guard = rig.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET authored_by_user = 1, is_reference = 0 WHERE id = ?1",
                [rig.project.0],
            )
            .unwrap();
        drop(guard);
    }
    assert_eq!(rig.rows(), 0);

    rig.store_goes_away();
    rig.repo.git(&["tag", "v1"]);
    rig.hand_off();
    let announced = rig.run_job(JobKind::J1Refstate);
    let open = rig.count(
        "SELECT count(*) FROM debt_item
          WHERE project_id = ?1 AND source = 'no_release' AND state = 'open'",
    );
    eprintln!(
        "release after the return: {open} no_release open, {} row(s), {announced} event(s)",
        rig.rows()
    );
    assert_eq!(open, 0, "the returning settle did not observe the tag");
    assert_eq!(rig.rows(), 0, "a delta crossed the unfreeze");
    assert_eq!(announced, 0);
}

/// **The return spans a chunked rescan.** More files than one J7 chunk covers, the marker file
/// sorting last: the first chunk after the return is `partial` and does not reach it, so it may
/// neither close nor re-verify the markers there. Withdrawing the sweep alone would let that
/// partial chunk stand as the *before* of the chunk that completes — and the closures made while
/// away would be written after all. The items withdrawn with it are what hold the layer
/// unobserved until a complete sweep has seen every one of them again.
#[test]
fn a_chunked_rescan_after_the_return_still_writes_no_delta() {
    let rig = Rig::new(&TODO_ONLY, true);
    for i in 0..600 {
        rig.repo.write(
            &format!("src/f{i:03}.rs"),
            format!("// file {i}\n").as_bytes(),
        );
    }
    // The markers move to the one file that sorts after every filler.
    std::fs::remove_file(rig.repo.path().join("src/a.rs")).unwrap();
    rig.repo.write("src/zz.rs", markers(5).as_bytes());
    rig.repo.commit("a tree wider than one chunk");
    rig.hand_off();
    rig.content_scan();
    assert_eq!(rig.open_todos(), 5, "the chunked first scan did not finish");
    let total = rig.count("SELECT blobs_total FROM project_content_scan WHERE project_id = ?1");
    eprintln!("files J7 enumerates: {total}");
    assert!(
        usize::try_from(total).unwrap() > codotheca_core::jobs::markers::J7_CHUNK_BLOBS,
        "the tree fits one chunk, so no partial chunk is exercised"
    );
    let before = rig.rows();

    rig.store_goes_away();
    rig.repo.write("src/zz.rs", markers(3).as_bytes());
    rig.repo
        .commit("two fewer, committed while the drive was elsewhere");
    rig.hand_off();
    let announced = rig.content_scan();
    let written = rig.rows() - before;
    eprintln!(
        "chunked rescan after the return: {} open, {written} new row(s), {announced} event(s)",
        rig.open_todos()
    );
    assert_eq!(rig.open_todos(), 3);
    assert_eq!(rig.todo_sweep(), "complete");
    assert_eq!(
        written, 0,
        "a delta crossed the unfreeze through a partial chunk"
    );
    assert_eq!(announced, 0);
}

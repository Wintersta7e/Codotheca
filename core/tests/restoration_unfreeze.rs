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
use codotheca_core::clock::Clock as _;
use codotheca_core::debt::store::{DebtStore as _, SqliteDebtStore};
use codotheca_core::detail::{dispatch_detail_command, DetailCtx};
use codotheca_core::git::{GitBackend, GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{
    run_one, Job, JobDeps, JobKind, JobOrigin, JobSink, NullJobSink, Priority,
};
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{DebtSource, HealthCheckSwitch, LocationId, ProjectId};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::presence::{apply_presence, PresenceContext, ScanRootRow};
use codotheca_core::scan::run::{platform_of, Discovered};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::sync::runner::NullSyncSink;
use codotheca_core::testing::{FakeClock, FakeMountResolver};
use support::TestRepo;

const T0: i64 = 1_700_000_000;
const STORE: &str = "store-a";

/// The host's own location kind. The fixture's paths are real temp directories, so a `linux`
/// key over a `C:\` path puts the repository under no root and the store never goes offline.
const fn native_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

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
    fn new(switched_off: &[DebtSource], grant: bool) -> Self {
        let repo = TestRepo::init();
        repo.write("src/a.rs", markers(5).as_bytes());
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), T0).unwrap()));
        let clock = Arc::new(FakeClock::new(T0));
        let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        ));
        let mut rig = Self {
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
                common_dir: handle.common_dir,
            },
            root_id: 1,
            kind: native_kind().to_owned(),
            distro: String::new(),
            path_bytes: path_bytes(path),
            path_key: path_key(path, platform_of(native_kind())),
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
            now: self.clock.now_unix(),
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
            kind: native_kind().to_owned(),
            distro: String::new(),
            path_bytes: path_bytes(parent),
            path_key: path_key(parent, platform_of(native_kind())),
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
            clock: self.clock.clone(),
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
        let (runner, events) = self.runner();
        assert!(runner.enqueue(self.job(kind)));
        drain(&runner);
        events.health_deltas()
    }

    fn runner(&self) -> (Arc<JobRunner>, Arc<RecordingSink>) {
        let events = Arc::new(RecordingSink::default());
        let runner = JobRunner::new(Arc::clone(&self.index), self.deps(), events.clone());
        (runner, events)
    }

    /// `projects.get`, answered by the command's own handler under the one index guard — its
    /// §6 and §29.7 asks go to `jobs`.
    fn page(&self, jobs: &dyn JobSink) -> serde_json::Value {
        let guard = self.index.lock().unwrap();
        let mounts = FakeMountResolver::default();
        let events = RecordingSink::default();
        let ctx = DetailCtx {
            index: &guard,
            git: self.git.as_ref(),
            mount: &mounts,
            events: &events,
            jobs,
            sync: &NullSyncSink,
            now: self.clock.now_unix(),
        };
        let detail = dispatch_detail_command(
            &ctx,
            "projects.get",
            serde_json::json!({ "id": self.project.0 }),
        )
        .unwrap()
        .unwrap();
        drop(guard);
        detail
    }

    /// The user opens the project's page, and the jobs it asks for run on the real runner.
    fn open_page(&self) {
        let (runner, _events) = self.runner();
        self.page(runner.as_ref());
        drain(&runner);
    }

    /// The bytes leave with the drive: the working copy is no longer on disk at its path.
    fn unplug(&self) {
        std::fs::rename(self.repo.path(), self.repo.scratch().join("unplugged")).unwrap();
    }

    /// `locations.uninstall`'s own writes, in its order: `removed_at` and the cleared columns,
    /// then §28.3's guard marking the project's open items `unverified`. `presence` stays
    /// `present`, which is the shape the removal leaves.
    fn uninstall(&self) {
        let clears = codotheca_core::uninstall::command::cleared_columns()
            .iter()
            .map(|column| format!("{column} = NULL"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut guard = self.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                tx.execute(
                    &format!("UPDATE location SET removed_at = ?2, {clears} WHERE id = ?1"),
                    rusqlite::params![self.location.0, self.clock.now_unix()],
                )?;
                SqliteDebtStore.mark_unverified(tx, self.project).unwrap();
                Ok(())
            })
            .unwrap();
        drop(guard);
    }

    /// J7's stored scan: the head it completed, when it last enumerated, and the four presence
    /// answers.
    fn content_row(&self) -> (Option<String>, i64, [String; 4]) {
        let guard = self.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT complete_head_oid, enumerated_at,
                        has_readme, has_license, has_tests, has_ci
                   FROM project_content_scan WHERE project_id = ?1",
                [self.project.0],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                    ))
                },
            )
            .unwrap();
        drop(guard);
        row
    }

    /// J7's settled job state, which says whether the page's content scan ran and how it ended.
    fn j7_state(&self) -> codotheca_core::jobs::state::JobStateRow {
        let guard = self.index.lock().unwrap();
        let row = codotheca_core::jobs::state::load(guard.conn(), self.project)
            .unwrap()
            .into_iter()
            .find(|row| row.job == JobKind::J7Markers)
            .expect("J7 has never settled for this project");
        drop(guard);
        row
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

/// Run the pool until nothing is due, then stop it. A retry parked behind its backoff is not due.
fn drain(runner: &Arc<JobRunner>) {
    runner.start(1);
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !runner.is_idle() {
        std::thread::sleep(Duration::from_millis(20));
    }
    runner.request_stop();
    runner.join();
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

/// A copy that came back and has not been rescanned yet: J7 completed at `T0`, the store went
/// away, and the walk's hand-off brought it back and withdrew J7's sweep.
fn returned_unscanned() -> (Rig, (Option<String>, i64, [String; 4])) {
    let rig = Rig::new(&TODO_ONLY, true);
    rig.content_scan();
    let scanned = rig.content_row();
    assert!(scanned.0.is_some(), "the first scan did not complete");
    rig.store_goes_away();
    rig.hand_off();
    assert_eq!(
        rig.todo_sweep(),
        "unobservable",
        "the return withdrew nothing"
    );
    (rig, scanned)
}

/// Open the page an hour on and assert J7 ran there and read nothing: the stored scan is the one
/// from before, and the job settled clean rather than failing into a retry.
fn assert_page_reads_nothing(rig: &Rig, scanned: &(Option<String>, i64, [String; 4])) {
    rig.clock.advance(3_600);
    let opened_at = rig.clock.now_unix();
    rig.open_page();
    let after = rig.content_row();
    let j7 = rig.j7_state();
    eprintln!(
        "page opened: scan before {scanned:?}, after {after:?}; j7 {:?} x{} at {} ({:?})",
        j7.state, j7.fail_count, j7.at, j7.reason
    );
    assert!(
        j7.at >= opened_at,
        "the page queued no content scan, so nothing here was exercised"
    );
    assert_eq!(&after, scanned, "J7 re-read a copy that is not there");
    assert_eq!(
        (j7.state, j7.fail_count),
        (codotheca_core::jobs::JobState::Done, 0)
    );
    assert_eq!(rig.todo_sweep(), "unobservable");
    assert_eq!(rig.rows(), 0);
}

/// The control for the two below, through the same page: a returned copy that is still there is
/// re-observed when its page opens.
#[test]
fn a_returned_copy_is_re_observed_when_its_page_opens() {
    let (rig, _) = returned_unscanned();
    rig.clock.advance(3_600);
    rig.open_page();
    eprintln!(
        "page opened on a present copy: sweep {}, {} open",
        rig.todo_sweep(),
        rig.open_todos()
    );
    assert_eq!(rig.todo_sweep(), "complete");
    assert_eq!(rig.open_todos(), 5);
    assert_eq!(rig.rows(), 0);
}

/// **The copy leaves again before its rescan runs.** The page still queues J7 against the only
/// copy there is, and a rescan of a root that is gone reads nothing: it would store `not_read`
/// over the four presence answers the last complete scan established, fail, and retry. The
/// withdrawn sweep waits for the copy to be back.
#[test]
fn a_copy_that_left_again_before_its_rescan_is_not_read() {
    let (rig, scanned) = returned_unscanned();
    rig.store_goes_away();
    rig.unplug();
    assert_page_reads_nothing(&rig, &scanned);
}

/// The same for a copy uninstalled before its rescan: `presence` still reads `present` and only
/// `removed_at` says the bytes are gone.
#[test]
fn an_uninstalled_copy_is_not_read_when_its_page_opens() {
    let (rig, scanned) = returned_unscanned();
    rig.uninstall();
    rig.unplug();
    assert_page_reads_nothing(&rig, &scanned);
}

/// **A withdrawal is dated when it happened.** The return is the moment the stored evidence stopped
/// being current, so the sweep it rewrites says so: the page's sweep list dates the withdrawn
/// `todo_marker` sweep at the return, and a reading left with no verdict dates its basis there
/// too — never at the observation from before the copy left, which the row no longer vouches for.
#[test]
fn a_withdrawn_sweep_is_dated_at_the_return() {
    let others: Vec<DebtSource> = DebtSource::ALL
        .into_iter()
        .filter(|source| *source != DebtSource::TodoMarker)
        .collect();
    let rig = Rig::new(&others, true);
    rig.content_scan();
    rig.clock.advance(3_600);
    rig.store_goes_away();
    rig.clock.advance(3_600);
    let returned_at = rig.clock.now_unix();
    rig.hand_off();

    let detail = rig.page(&NullJobSink);
    let sweep = detail["debtSweeps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sweep| sweep["source"] == "todo_marker")
        .cloned()
        .unwrap();
    let basis = detail["health"]["basis"].clone();
    eprintln!("returned at {returned_at}: sweep {sweep}, basis {basis}");
    assert_eq!(sweep["outcome"], "unobservable");
    assert_eq!(
        sweep["observedAt"], returned_at,
        "the withdrawn sweep kept the date of the observation it withdrew"
    );
    assert_eq!(basis["ran"], 0, "a check still has a verdict");
    assert_eq!(basis["observedAt"], returned_at);
}

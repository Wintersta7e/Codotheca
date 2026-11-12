//! A content scan whose cache is cleared under it restarts at its first blob.
//!
//! J7 reads a tree larger than one chunk across several runs, resuming at a durable cursor, and
//! builds the project's items over everything the cache holds after each chunk. Revoking the
//! grant, or the exclusion purge, deletes that cache. A scan that then resumes at its old cursor
//! reads only the blobs after it, marks itself complete, and builds the items over a cache missing
//! every blob before it: the items whose evidence sat there close and pay as if the user had
//! resolved them. These tests keep the markers in `src/a.rs`, ordinal 0, so the evidence a resume
//! would skip is the evidence for live items, and drive the real runner end to end.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{Clock as _, LocalZone};
use codotheca_core::exclusion::gate::ExclusionGate;
use codotheca_core::git::{GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::Index;
use codotheca_core::jobs::content_scan::{reset_scan_progress, ResetScope};
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{
    DebtSource, HealthCheckSwitch, LocationId, ProjectId, SettingsPatch,
};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};
use codotheca_core::testing::{FakeClock, RecordingGitBackend};
use support::TestRepo;

const T0: i64 = 1_700_000_000;
const STORE: &str = "store-a";
/// Files the tree holds beside `src/a.rs`: enough that J7 needs two chunks.
const FILLERS: usize = 600;
/// `todo_marker` alone feeds `overgrowth`.
const TODO_ONLY: [DebtSource; 2] = [DebtSource::MissingTests, DebtSource::UnpushedCommits];

const fn native_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// Counts the runner's `health_delta` events.
#[derive(Debug, Default)]
struct Sink {
    deltas: Mutex<usize>,
}

impl EventSink for Sink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
        if topic == "projects" && event == "health_delta" {
            *self.deltas.lock().unwrap() += 1;
        }
    }
}

struct Rig {
    repo: TestRepo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    git: Arc<RecordingGitBackend<SystemGit>>,
    clock: Arc<FakeClock>,
    project: ProjectId,
    location: LocationId,
    generation: std::cell::Cell<i64>,
}

impl Rig {
    /// One real repository wider than one chunk, five TODOs in its first file, indexed through
    /// the walk's hand-off, enrolled, authored and granted, its head recorded by J1.
    fn new() -> Self {
        let repo = TestRepo::init();
        repo.write("src/a.rs", markers(5).as_bytes());
        for i in 0..FILLERS {
            repo.write(
                &format!("src/f{i:03}.rs"),
                format!("// file {i}\n").as_bytes(),
            );
        }
        repo.commit("first");
        let dir = tempfile::tempdir().unwrap();
        let index = Arc::new(Mutex::new(Index::open_at(dir.path(), T0).unwrap()));
        let clock = Arc::new(FakeClock::new(T0));
        let git = Arc::new(RecordingGitBackend::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )));
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
        (rig.project, rig.location) = rig.hand_off();
        rig.index
            .lock()
            .unwrap()
            .with_tx(|tx| {
                tx.execute(
                    "UPDATE project SET authored_by_user = 1, is_reference = 0,
                                        acknowledged_at = ?2
                      WHERE id = ?1",
                    rusqlite::params![rig.project.0, T0 - 100],
                )?;
                let off: Vec<HealthCheckSwitch> = TODO_ONLY
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
        grant(&rig.index, true);
        rig.refstate();
        rig
    }

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
            exclusions: &ExclusionGate::default(),
            store_class: StoreClass::Local,
            generation: self.generation.get(),
            now: self.clock.now_unix(),
        };
        let indexed = hand_off_discovered(&self.index, &ctx, &discovered).unwrap();
        (indexed.project, indexed.location)
    }

    fn deps(&self) -> JobDeps {
        JobDeps {
            exclusions: Arc::new(ExclusionGate::default()),
            git: self.git.clone(),
            clock: self.clock.clone(),
            cancel: CancelToken::new(),
            zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
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

    /// J1 through the production dispatch: it records the head J7 compares against.
    fn refstate(&self) {
        let announce = std::cell::RefCell::new(Vec::new());
        codotheca_core::jobs::run_one(
            &self.index,
            &self.deps(),
            &self.job(JobKind::J1Refstate),
            &announce,
        )
        .unwrap();
    }

    fn runner(&self) -> (Arc<JobRunner>, Arc<Sink>) {
        let events = Arc::new(Sink::default());
        let runner = JobRunner::new(Arc::clone(&self.index), self.deps(), events.clone());
        assert!(runner.enqueue(self.job(JobKind::J7Markers)));
        (runner, events)
    }

    /// J7 through the real runner — its execute, its settle, its requeues — until nothing is due.
    /// Returns the `health_delta` events it announced.
    fn scan_to_end(&self) -> usize {
        let (runner, events) = self.runner();
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let finished = runner.is_idle();
        runner.request_stop();
        runner.join();
        assert!(
            finished,
            "J7 never finished: it requeued itself until the deadline"
        );
        let announced = *events.deltas.lock().unwrap();
        announced
    }

    /// One J7 chunk through the real runner, settled, and nothing after it: the worker is asked
    /// to stop while the chunk reads, so its settle is the last thing it does.
    fn first_chunk(&self) {
        let (runner, _) = self.runner();
        let stop = Arc::downgrade(&runner);
        self.git.before_next("read_blobs", move || {
            if let Some(stopping) = stop.upgrade() {
                stopping.request_stop();
            }
        });
        runner.start(1);
        runner.join();
        assert_eq!(
            self.cursor().as_deref(),
            Some("512"),
            "the first chunk did not settle at its cursor"
        );
        assert_eq!(
            self.open_todos(),
            5,
            "the first chunk did not open the items"
        );
    }

    fn cursor(&self) -> Option<String> {
        self.j7_row().cursor
    }

    fn j7_row(&self) -> codotheca_core::jobs::state::JobStateRow {
        let guard = self.index.lock().unwrap();
        let row = codotheca_core::jobs::state::load(guard.conn(), self.project)
            .unwrap()
            .into_iter()
            .find(|row| row.job == JobKind::J7Markers);
        drop(guard);
        row.expect("J7 has never settled for this project")
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

    fn open_todos(&self) -> i64 {
        self.count(
            "SELECT count(*) FROM debt_item
              WHERE project_id = ?1 AND source = 'todo_marker' AND state = 'open'",
        )
    }

    fn pending_and_total(&self) -> (Option<i64>, Option<i64>) {
        let guard = self.index.lock().unwrap();
        let row = guard
            .conn()
            .query_row(
                "SELECT blobs_pending, blobs_total FROM project_content_scan
                  WHERE project_id = ?1",
                [self.project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(guard);
        row
    }

    fn reads_of(&self, op: &str) -> usize {
        self.git.calls().iter().filter(|c| c.op == op).count()
    }

    /// What the resume must not have done, printed and asserted: no item closed, no `debt_day`
    /// paid, no `health_delta` row written or announced.
    fn assert_nothing_closed(&self, announced: usize, case: &str) {
        let open = self.open_todos();
        let days = self
            .count("SELECT count(*) FROM xp_events WHERE project_id = ?1 AND kind = 'debt_day'");
        let deltas = self.count("SELECT count(*) FROM health_delta WHERE project_id = ?1");
        eprintln!(
            "{case}: {} closed, {days} debt_day, {deltas} health_delta row(s), {announced} event(s)",
            5 - open
        );
        assert_eq!(open, 5, "{case}: an item closed over a cleared cache");
        assert_eq!(days, 0, "{case}: a debt_day was paid");
        assert_eq!(deltas, 0, "{case}: a health_delta row was written");
        assert_eq!(announced, 0, "{case}: a health_delta was announced");
    }
}

/// `content_scan_enabled` through the settings command's own writer, which revokes on `false`.
fn grant(index: &Mutex<Index>, on: bool) {
    let patch = SettingsPatch {
        effects_tier: None,
        reduced_motion_override: None,
        autostart: None,
        resident_shortcut: None,
        roast_enabled: None,
        log_level: None,
        install_root_id: None,
        content_scan_enabled: Some(on),
        health_checks: None,
        focus_target_minutes: None,
        rhythm_target: None,
        motivation_shown: None,
    };
    let guard = index.lock().unwrap();
    codotheca_core::surfaces::settings::write(guard.conn(), &patch, T0).unwrap();
    drop(guard);
}

/// The purge's half that concerns J7: the library cache goes, and the scans restart.
fn clear_cache(index: &Mutex<Index>) {
    index
        .lock()
        .unwrap()
        .with_tx(|tx| {
            tx.execute("DELETE FROM blob_finding", [])?;
            tx.execute("DELETE FROM blob_scan", [])?;
            reset_scan_progress(tx, ResetScope::IncompleteExcept(&[]))
        })
        .unwrap();
}

fn markers(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        let _ = writeln!(body, "// TODO: marker number {i}");
    }
    body
}

/// GA19 (b): revoke at cursor 512, re-grant, resume. The resume must start at ordinal 0, and must
/// not waste a chunk on the way there.
#[test]
fn a_regranted_scan_restarts_at_its_first_blob() {
    let rig = Rig::new();
    rig.first_chunk();
    grant(&rig.index, false);
    let revoked_cursor = rig.cursor();
    grant(&rig.index, true);
    let before = rig.reads_of("read_blobs");
    let announced = rig.scan_to_end();
    let chunks = rig.reads_of("read_blobs") - before;
    eprintln!("chunks read after the re-grant: {chunks}");
    rig.assert_nothing_closed(announced, "revoke -> re-grant");
    assert_eq!(rig.pending_and_total().0, Some(0));
    assert_eq!(revoked_cursor, None, "revocation left J7's cursor behind");
    assert_eq!(chunks, 2, "the restart read a chunk it then threw away");
}

/// GA19's purge between two chunks: the next chunk starts at ordinal 0.
#[test]
fn a_scan_whose_cache_was_purged_between_chunks_restarts() {
    let rig = Rig::new();
    rig.first_chunk();
    clear_cache(&rig.index);
    let (pending, total) = rig.pending_and_total();
    assert_eq!(
        pending, total,
        "the reset left blobs_pending short of the total"
    );
    assert_eq!(rig.cursor(), None, "the reset left J7's cursor behind");
    let announced = rig.scan_to_end();
    rig.assert_nothing_closed(announced, "purge between chunks");
}

/// The cursor a chunk resumes at is read when the job is dispatched. A reset landing after that
/// read and before the enumeration's write leaves the run holding the old cursor over a row that
/// now says every blob is pending: the chunk must land no item build.
#[test]
fn a_reset_after_dispatch_lands_no_item_build() {
    let rig = Rig::new();
    rig.first_chunk();
    let index = Arc::clone(&rig.index);
    rig.git
        .before_next("head_tree", move || clear_cache(&index));
    let announced = rig.scan_to_end();
    assert_eq!(
        rig.pending_and_total().0,
        Some(0),
        "the scan never completed"
    );
    rig.assert_nothing_closed(announced, "reset before the enumeration");
}

/// The same reset landing while the chunk reads its blobs.
#[test]
fn a_reset_during_the_read_lands_no_item_build() {
    let rig = Rig::new();
    rig.first_chunk();
    let index = Arc::clone(&rig.index);
    rig.git
        .before_next("read_blobs", move || clear_cache(&index));
    let announced = rig.scan_to_end();
    assert_eq!(
        rig.pending_and_total().0,
        Some(0),
        "the scan never completed"
    );
    rig.assert_nothing_closed(announced, "reset during the read");
}

/// A reset landing after a chunk commits and before its settle: the settle must not write back
/// the cursor the reset cleared. The settle's zone reading is the one call between the chunk's
/// commit and the settle's transaction a test can reach; the reset lands there.
#[test]
fn a_settle_after_a_reset_writes_no_cursor() {
    let rig = Rig::new();
    let zone = Arc::new(SettleZone {
        index: Arc::clone(&rig.index),
        armed: AtomicBool::new(false),
        fired: AtomicBool::new(false),
        runner: Mutex::new(None),
    });
    let mut deps = rig.deps();
    deps.zone = zone.clone();
    let runner = JobRunner::new(Arc::clone(&rig.index), deps, Arc::new(Sink::default()));
    *zone.runner.lock().unwrap() = Some(Arc::downgrade(&runner));
    assert!(runner.enqueue(rig.job(JobKind::J7Markers)));
    let arm = Arc::clone(&zone);
    rig.git.before_next("read_blobs", move || {
        arm.armed.store(true, Ordering::SeqCst);
    });
    runner.start(1);
    runner.join();
    assert!(zone.fired.load(Ordering::SeqCst), "the reset never landed");
    let (pending, total) = rig.pending_and_total();
    eprintln!(
        "after the settle: cursor {:?}, pending {pending:?} of {total:?}",
        rig.cursor()
    );
    assert_eq!(pending, total, "the reset did not land before the settle");
    assert_eq!(
        rig.cursor(),
        None,
        "the settle restored a cursor the reset cleared"
    );
}

/// A fixed UTC zone that, once armed, runs the purge's reset at the first reading it gets with
/// the index free — the settle's — and then asks the runner to stop after that settle.
#[derive(Debug)]
struct SettleZone {
    index: Arc<Mutex<Index>>,
    armed: AtomicBool,
    fired: AtomicBool,
    runner: Mutex<Option<std::sync::Weak<JobRunner>>>,
}

impl LocalZone for SettleZone {
    fn offset_min_at(&self, _unix: i64) -> i32 {
        if self.armed.load(Ordering::SeqCst) && !self.fired.load(Ordering::SeqCst) {
            if let Ok(mut guard) = self.index.try_lock() {
                guard
                    .with_tx(|tx| {
                        tx.execute("DELETE FROM blob_finding", [])?;
                        tx.execute("DELETE FROM blob_scan", [])?;
                        reset_scan_progress(tx, ResetScope::IncompleteExcept(&[]))
                    })
                    .unwrap();
                drop(guard);
                self.fired.store(true, Ordering::SeqCst);
                let runner = self
                    .runner
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade);
                if let Some(runner) = runner {
                    runner.request_stop();
                }
            }
        }
        0
    }
}

/// A complete scan is left complete, and an unchanged head reads nothing after the reset.
#[test]
fn a_complete_scan_is_untouched_by_the_reset() {
    let rig = Rig::new();
    rig.scan_to_end();
    assert_eq!(rig.open_todos(), 5);
    clear_cache(&rig.index);
    assert_eq!(
        rig.pending_and_total().0,
        Some(0),
        "the reset reopened a complete scan"
    );
    let complete = rig.count(
        "SELECT count(*) FROM project_content_scan
          WHERE project_id = ?1 AND complete_head_oid = head_oid",
    );
    assert_eq!(complete, 1, "the reset cleared a completion");
    let before = rig.git.calls().len();
    let announced = rig.scan_to_end();
    let ops: Vec<&str> = rig.git.calls()[before..].iter().map(|c| c.op).collect();
    eprintln!("git after the reset at an unchanged head: {ops:?}");
    assert!(!ops.contains(&"head_tree") && !ops.contains(&"read_blobs"));
    assert_eq!(rig.count("SELECT count(*) FROM blob_scan WHERE ?1 > 0"), 0);
    rig.assert_nothing_closed(announced, "complete scan");
}

/// §29.8: a revocation landing while the second chunk reads. The chunk's reads were licensed by a
/// grant that no longer stands when it commits, so none is recorded; and the revocation nulled
/// `blobs_pending`, so the chunk must not read that as *nothing pending* and build the items over
/// the cache the revocation just emptied.
#[test]
fn a_revocation_during_the_read_leaves_no_cache_row_and_closes_nothing() {
    let rig = Rig::new();
    rig.first_chunk();
    let index = Arc::clone(&rig.index);
    rig.git
        .before_next("read_blobs", move || grant(&index, false));
    let announced = rig.scan_to_end();
    let scans = rig.count("SELECT count(*) FROM blob_scan WHERE ?1 > 0");
    let findings = rig.count("SELECT count(*) FROM blob_finding WHERE ?1 > 0");
    eprintln!("after a revocation mid-read: {scans} blob_scan, {findings} blob_finding");
    rig.assert_nothing_closed(announced, "revocation during the read");
    assert_eq!(scans + findings, 0, "a revoked grant's reads were recorded");
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! AC-P4-56-13 — §56.5's gate audit: one covered copy, every row of the table driven through its
//! real entry point against the core's one gate, with git backends that record every call and a
//! mount resolver that counts every resolve. The covering entry is inserted as a row, applied
//! with `apply_coverage` and put in force with a swap, as an add does. Each row must refuse at its
//! own site in the gate's testkit record, read nothing, and answer what §56.5 says it answers.
//!
//! Rows 11 and 12 hand the whole list to a reader that applies it itself, so their record is an
//! admitted snapshot (GA37). Row 2's and row 6's write checks, and row 17's verifying read, are
//! refused for an entry added while the read they follow was running, which is the only way a
//! write check or step 6 is reached for a covered copy.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::analyser::remote::{RemoteReading, RemoteVerifier};
use codotheca_core::analyser::AnalyserSeams;
use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, HandoffError};
use codotheca_core::cancel::CancelToken;
use codotheca_core::exclusion::gate::{check_location_read, ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, PathEntry};
use codotheca_core::exclusion::store::{
    apply_coverage, insert_restored, load_list, location_key, RestoredExclusion,
};
use codotheca_core::git::{
    Authorship, CommitSubject, Divergence, GitBackend, GitExec, GitResult, GitVersion, JobContext,
    RefState, RepoFacts, RepoHandle, RootCommit, RunLimits, StatusOptions, StoreKey,
    TrackedInventory, WorktreeStatus,
};
use codotheca_core::http::HttpTransport;
use codotheca_core::index::path::native_platform;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobError, JobKind, JobOrigin, Priority};
use codotheca_core::mount::{MountError, MountFacts, MountResolver, StoreClass};
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::protocol::{
    Bytes, ErrorCode, ExclusionKind, LocationId, Presence, ProjectId, SyncTaskKind,
};
use codotheca_core::scan::discover::{ProbeCtx, RepoCandidate, RepoKind};
use codotheca_core::scan::links::LinkPolicy;
use codotheca_core::scan::presence::{
    classify_presence, LocationPresenceRow, PresenceContext, ScanRootRow,
};
use codotheca_core::scan::run::Discovered;
use codotheca_core::scan::walk::{walk_root, WalkCtx};
use codotheca_core::scan::{ScanLauncher, WalkEvent, WalkOptions};
use codotheca_core::session::activity::{GitIgnoreCheck, ScopeFilter};
use codotheca_core::session::segment::Signal;
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{
    core_handler_with, CoreHandlerParts, CountingTrash, FakeClock, FakeGitBackend,
    FakeMountResolver, FakeTokenStore, FakeTransport, GitReply, HeldScanLauncher,
    RecordingGitBackend,
};
use serde_json::{json, Value};

const NOW: i64 = 1_800_000_000;
const DEADLINE: Duration = Duration::from_secs(20);

/// The host's `location.kind` and `scan_root.kind`.
const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// A directory shaped as a repository's work tree, as the walk recognises one.
fn repo_at(path: &Path) {
    std::fs::create_dir_all(path.join(".git/objects")).unwrap();
    std::fs::create_dir_all(path.join(".git/refs")).unwrap();
    std::fs::write(path.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
}

fn covering(dir: &Path) -> Arc<ExclusionList> {
    Arc::new(ExclusionList::new(
        &[],
        vec![PathEntry {
            distro: String::new(),
            platform: native_platform(),
            match_key: path_key(dir, native_platform()),
        }],
    ))
}

/// Poll until `f` holds or the deadline passes.
fn wait_for(mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while !f() {
        if start.elapsed() >= DEADLINE {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Counts every resolve and answers none: an audited command must not ask.
#[derive(Debug, Default)]
struct CountingMounts(AtomicUsize);

impl MountResolver for CountingMounts {
    fn resolve(&self, _path: &Path) -> Result<MountFacts, MountError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(MountError::Unsupported(String::new()))
    }
    fn is_volume_mounted(&self, _volume_key: &str) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst);
        false
    }
}

/// Counts every remote read: row 17's `VerifyRead` must not be invoked.
#[derive(Debug, Default)]
struct CountingVerifier(AtomicUsize);

impl RemoteVerifier for CountingVerifier {
    fn read(&self, _repo: &RepoHandle, _remote: &str, _ctx: &JobContext<'_>) -> RemoteReading {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("row 17: a verifying read of a covered copy was invoked");
    }
}

struct Audit {
    _dir: tempfile::TempDir,
    zone: PathBuf,
    copy: PathBuf,
    elsewhere: PathBuf,
    index: Arc<Mutex<Index>>,
    parts: std::cell::RefCell<CoreHandlerParts>,
    git: Arc<RecordingGitBackend<FakeGitBackend>>,
    mounts: Arc<CountingMounts>,
    project: i64,
    location: i64,
    root_id: i64,
    problem: i64,
}

fn exec(index: &Index, sql: &str, params: impl rusqlite::Params) -> i64 {
    index.conn().execute(sql, params).unwrap();
    index.conn().last_insert_rowid()
}

fn audit() -> Audit {
    let dir = tempfile::tempdir().unwrap();
    let zone = dir.path().join("zone");
    let copy = zone.join("widget");
    let elsewhere = zone.join("moved");
    repo_at(&copy);
    std::fs::create_dir_all(&elsewhere).unwrap();
    let index = Index::open_at(&dir.path().join("index"), NOW).unwrap();
    let project = exec(
        &index,
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('widget', 'widget', 0, 0)",
        [],
    );
    let location = exec(
        &index,
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, 'store', 'present', 'worktree')",
        rusqlite::params![
            project,
            host_kind(),
            path_bytes(&copy),
            path_key(&copy, native_platform()),
            path_display(&copy)
        ],
    );
    let root_id = exec(
        &index,
        "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display, enabled,
                                added_by, descend_into_repos, added_at)
         VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, 1)",
        rusqlite::params![
            host_kind(),
            path_bytes(&zone),
            path_key(&zone, native_platform()),
            path_display(&zone)
        ],
    );
    let run = exec(
        &index,
        "INSERT INTO scan_run (generation, started_at, mode, roots_json)
         VALUES (1, 1, 'full', '[]')",
        [],
    );
    let problem = exec(
        &index,
        "INSERT INTO scan_problem (scan_run_id, kind, path_display, path_bytes, root_id)
         VALUES (?1, 'untrusted_repo', 'widget', ?2, ?3)",
        rusqlite::params![run, path_bytes(&copy), root_id],
    );
    let index = Arc::new(Mutex::new(index));
    let git = Arc::new(RecordingGitBackend::new(FakeGitBackend::new()));
    let mounts = Arc::new(CountingMounts::default());
    let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let parts = core_handler_with(
        dir.path(),
        &index,
        NOW,
        launcher,
        git.clone(),
        mounts.clone(),
    );
    Audit {
        _dir: dir,
        zone,
        copy,
        elsewhere,
        index,
        parts: std::cell::RefCell::new(parts),
        git,
        mounts,
        project,
        location,
        root_id,
        problem,
    }
}

impl Audit {
    fn gate(&self) -> Arc<ExclusionGate> {
        Arc::clone(self.parts.borrow().handler.exclusions())
    }

    /// Put in force exactly the user path entries `dirs`, as an add does: the rows, then the
    /// copies' `excluded` flags, then the swap, all under the index guard.
    fn cover(&self, dirs: &[&Path]) {
        let mut guard = self.index.lock().unwrap();
        let gate = self.gate();
        guard
            .with_tx(|tx| {
                tx.execute("DELETE FROM scan_exclusion", [])?;
                for (at, dir) in (1..).zip(dirs) {
                    insert_restored(
                        tx,
                        &RestoredExclusion {
                            kind: ExclusionKind::Path,
                            entry: path_display(dir),
                            path_bytes: Some(Bytes(path_bytes(dir))),
                            distro: String::new(),
                            created_at: at,
                        },
                    )?;
                }
                let list = load_list(tx)?;
                apply_coverage(tx, &list)?;
                let _ = gate.swap(Arc::new(list));
                Ok(())
            })
            .unwrap();
        drop(guard);
    }

    /// Cover the copy itself, and check `apply_coverage` marked it.
    fn cover_copy(&self) {
        self.cover(&[&self.copy]);
        assert!(self.copy_excluded(), "apply_coverage did not mark the copy");
    }

    fn row_path(&self) -> Vec<u8> {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT path_bytes FROM location WHERE id = ?1",
                [self.location],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn copy_excluded(&self) -> bool {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT excluded FROM location WHERE id = ?1",
                [self.location],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn checks_from(&self, from: usize) -> Vec<(GateSite, bool)> {
        self.gate().checks()[from..].to_vec()
    }

    /// Run `command`; assert it answers `EXCLUDED` having asked git and the mounts nothing, and
    /// that the gate refused it at `site`.
    fn refused(&self, row: &str, site: GateSite, command: &str, args: Value) {
        let calls = self.git.calls().len();
        let resolves = self.mounts.0.load(Ordering::SeqCst);
        let checks = self.gate().checks().len();
        let answer = self.parts.borrow_mut().handler.handle(command, args);
        let failure = answer
            .err()
            .unwrap_or_else(|| panic!("{row} {command}: answered, not refused"));
        assert_eq!(
            failure.code,
            ErrorCode::Excluded,
            "{row} {command}: {failure:?}"
        );
        let spawned = &self.git.calls()[calls..];
        assert!(spawned.is_empty(), "{row} {command} asked git {spawned:?}");
        assert_eq!(
            self.mounts.0.load(Ordering::SeqCst),
            resolves,
            "{row} {command} resolved a mount"
        );
        let made = self.checks_from(checks);
        assert!(
            made.contains(&(site, false)),
            "{row} {command}: no refused {site:?} check in {made:?}"
        );
    }

    fn job(&self, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: ProjectId(self.project),
            location_id: LocationId(self.location),
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }
    }

    fn runner(&self, git: Arc<dyn GitBackend>) -> Arc<JobRunner> {
        JobRunner::new(
            Arc::clone(&self.index),
            JobDeps {
                exclusions: self.gate(),
                git,
                clock: Arc::new(FakeClock::new(NOW)),
                cancel: CancelToken::new(),
                zone: Arc::new(FixedZone(0)),
            },
            Arc::new(ValidatingSink::default()),
        )
    }

    fn head_oid(&self) -> Option<String> {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT head_oid FROM location WHERE id = ?1",
                [self.location],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn locations(&self) -> i64 {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row("SELECT COUNT(*) FROM location", [], |r| r.get(0))
            .unwrap()
    }
}

fn drain(runner: &Arc<JobRunner>) {
    runner.start(1);
    assert!(wait_for(|| runner.is_idle()), "the runner never went idle");
    runner.request_stop();
    runner.join();
}

fn ref_state() -> RefState {
    RefState {
        head_oid: Some("a".repeat(40)),
        branch: Some("main".to_owned()),
        upstream: None,
        ahead: None,
        behind: None,
        tag_count: 0,
        stash_count: Some(0),
        is_shallow: false,
        is_bare: false,
        interrupted_op: None,
        fetch_head_at: None,
        reflog_tail_at: None,
        basis: codotheca_core::git::RefFingerprint::from_hex(&"0".repeat(64)).unwrap(),
        observed_at: NOW,
        push: codotheca_core::git::PushObservation::without_push(
            codotheca_core::git::PushRecord::NoRemote,
        ),
    }
}

// ---------------------------------------------------------------------------
// The scan: rows 1, 2, 3 and the hand-off's row 4.
// ---------------------------------------------------------------------------

fn row_1_walk(a: &Audit) {
    let gate = a.gate();
    let opts = WalkOptions {
        threads: 1,
        ..WalkOptions::default()
    };
    let git = RecordingGitBackend::new(FakeGitBackend::new());
    let cancel = CancelToken::new();
    let probe = ProbeCtx::new(&git, StoreKey::new("store-a"), StoreClass::Local, &cancel);
    let mounts = FakeMountResolver::new();
    mounts.map(
        &a.zone,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    let ctx = WalkCtx {
        opts: &opts,
        exclusions: &gate,
        platform: native_platform(),
        probe: &probe,
        links: Arc::new(LinkPolicy::new(
            false,
            vec![path_key(&a.zone, native_platform())],
            BTreeSet::from(["store-a".to_owned()]),
            Arc::new(mounts),
        )),
    };
    let found = Mutex::new(Vec::new());
    let _ = walk_root(&a.zone, &ctx, &|event| {
        if let WalkEvent::Repo(c) = event {
            found.lock().unwrap().push(c.path);
        }
    });
    let found = found.into_inner().unwrap();
    assert!(
        found.is_empty(),
        "row 1: the covered copy was found {found:?}"
    );
    assert!(git.calls().is_empty(), "row 1 asked git {:?}", git.calls());
}

fn found_at(path: &Path, root_id: i64) -> Discovered {
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: path.join(".git"),
            common_dir: path.join(".git"),
        },
        root_id,
        kind: host_kind().to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, native_platform()),
        path_display: path_display(path),
        store_key: "store-a".to_owned(),
        volume_key: Some("vol-a".to_owned()),
    }
}

/// Answers the reads the ownership probe and `probe_identity` make, so a hand-off reaches a row.
fn indexable(common_dir: &Path) -> FakeGitBackend {
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: common_dir.to_path_buf(),
        common_dir: common_dir.to_path_buf(),
    }));
    git.always_root_commits(GitReply::Ok(Vec::new()));
    git.always_remote_urls(GitReply::Ok(Vec::new()));
    git
}

fn hand_off(a: &Audit, git: &dyn GitBackend) -> Result<(), HandoffError> {
    let gate = a.gate();
    let cancel = CancelToken::new();
    let ctx = HandoffCtx {
        git,
        cancel: &cancel,
        exclusions: &gate,
        store_class: StoreClass::Local,
        generation: 1,
        now: NOW,
    };
    hand_off_discovered(&a.index, &ctx, &found_at(&a.copy, a.root_id)).map(|_| ())
}

/// Row 4 at the hand-off: a covered candidate is refused before its ownership probe.
fn row_4_hand_off(a: &Audit) {
    let git = RecordingGitBackend::new(indexable(&a.copy.join(".git")));
    let err = hand_off(a, &git).unwrap_err();
    assert!(matches!(err, HandoffError::Excluded), "row 4: {err:?}");
    assert!(git.calls().is_empty(), "row 4 asked git {:?}", git.calls());
}

/// Row 2: the hand-off's write check, for an entry added while its reads ran.
fn row_2_hand_off_write(a: &Audit) {
    a.cover(&[]);
    let before = a.locations();
    let inner = indexable(&a.copy.join(".git"));
    let git = SwapOnFirstCall::new(&inner, a.gate(), covering(&a.copy));
    let err = hand_off(a, &git).unwrap_err();
    assert!(git.fired(), "row 2: the hand-off read nothing");
    assert!(matches!(err, HandoffError::Excluded), "row 2: {err:?}");
    assert_eq!(a.locations(), before, "row 2 wrote a location");
}

fn row_3_presence(a: &Audit) {
    let gate = a.gate();
    let roots = [ScanRootRow {
        root_id: a.root_id,
        kind: host_kind().to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(&a.zone),
        path_key: path_key(&a.zone, native_platform()),
        enabled: true,
        descend_into_repos: false,
    }];
    let present = BTreeSet::from(["store".to_owned()]);
    let ctx = PresenceContext {
        generation: 2,
        roots: &roots,
        exclusions: &gate,
        present_stores: &present,
    };
    let row = LocationPresenceRow {
        location_id: a.location,
        project_id: a.project,
        path_bytes: path_bytes(&a.copy),
        path_key: path_key(&a.copy, native_platform()),
        store_key: "store".to_owned(),
        scan_generation: 1,
        presence: Presence::Present,
    };
    assert_eq!(classify_presence(&row, &ctx), Presence::Unscanned, "row 3");
}

// ---------------------------------------------------------------------------
// Jobs: rows 4 (the job's probe), 5, 6, 7, 8, 9 and 10.
// ---------------------------------------------------------------------------

fn row_4_job_probe(a: &Audit) {
    let git = Arc::new(FakeGitBackend::new());
    let deps = JobDeps {
        exclusions: a.gate(),
        git: git.clone(),
        clock: Arc::new(FakeClock::new(NOW)),
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    };
    let announce = std::cell::RefCell::new(Vec::new());
    let out = run_one(&a.index, &deps, &a.job(JobKind::J1Refstate), &announce);
    assert!(matches!(out, Err(JobError::Excluded)), "row 4: {out:?}");
    assert!(git.calls().is_empty(), "row 4 asked git {:?}", git.calls());
}

fn row_5_dispatch(a: &Audit) {
    let git = Arc::new(FakeGitBackend::new());
    let runner = a.runner(git.clone());
    assert!(
        runner.enqueue(a.job(JobKind::J1Refstate)),
        "row 5: not queued"
    );
    drain(&runner);
    assert!(git.calls().is_empty(), "row 5 asked git {:?}", git.calls());
    assert!(
        runner.queued().is_empty(),
        "row 5 chained {:?}",
        runner.queued()
    );
}

/// Row 6: a job admitted at dispatch and at its probe, whose copy is covered while it reads.
fn row_6_job_write(a: &Audit) {
    a.cover(&[]);
    let inner: &'static FakeGitBackend = Box::leak(Box::new(FakeGitBackend::new()));
    inner.always_ref_state(GitReply::Ok(ref_state()));
    inner.always_divergence(GitReply::Ok(None));
    inner.always_archive_reach(GitReply::Ok(false));
    let git = Arc::new(SwapOnFirstCall::new(inner, a.gate(), covering(&a.copy)));
    let runner = a.runner(git.clone());
    assert!(runner.enqueue(a.job(JobKind::J1Refstate)));
    drain(&runner);
    assert!(git.fired(), "row 6: the job read nothing");
    assert_eq!(a.head_oid(), None, "row 6: J1 wrote under the new list");
    assert!(
        runner.queued().is_empty(),
        "row 6 chained {:?}",
        runner.queued()
    );
}

fn row_7_page_open(a: &Audit) {
    let runner = a.runner(Arc::new(FakeGitBackend::new()));
    let guard = a.index.lock().unwrap();
    codotheca_core::jobs::visible::notify_visible(
        &guard,
        &FakeMountResolver::new(),
        runner.as_ref(),
        &a.gate(),
        ProjectId(a.project),
        LocationId(a.location),
        true,
    );
    drop(guard);
    assert!(
        runner.queued().is_empty(),
        "row 7 queued {:?}",
        runner.queued()
    );
}

fn row_8_requeue(a: &Audit) {
    exec(
        &a.index.lock().unwrap(),
        "INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
         VALUES (?1, 'j2', 'failed', 3, 'permission denied', 10)",
        [a.project],
    );
    let answer = a
        .parts
        .borrow_mut()
        .handler
        .handle("projects.requeue", json!({ "id": a.project }))
        .unwrap();
    assert_eq!(answer["jobsRequeued"], json!(0), "row 8: {answer}");
}

/// Row 9 has no production caller (plan Disagreement 4): driven as its caller would drive it.
fn row_9_watch_set(a: &Audit) {
    let gate = a.gate();
    let guard = a.index.lock().unwrap();
    let targets = codotheca_core::freshness::watch::select_targets(
        &[(LocationId(a.location), a.copy.clone(), NOW)],
        &[LocationId(a.location)],
        30,
        &|id| check_location_read(guard.conn(), &gate, GateSite::WatchSet, id).is_ok(),
    );
    drop(guard);
    assert!(targets.is_empty(), "row 9 watched {targets:?}");
}

fn row_10_remote_sync(a: &Audit) {
    let forge = Arc::new(FakeTransport::new());
    let clock = Arc::new(FakeClock::new(NOW));
    let http: Arc<dyn HttpTransport> = forge.clone();
    let observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        http,
        clock.clone(),
    ));
    let provider_http: Arc<dyn HttpTransport> = observing.clone();
    let deps = SyncDeps {
        exclusions: a.gate(),
        provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
            provider_http,
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        transport: observing,
        tokens: Arc::new(FakeTokenStore::unavailable()),
        clock,
        cancel: CancelToken::new(),
        zone: Arc::new(FixedZone(0)),
    };
    let runner = codotheca_core::sync::runner::SyncRunner::new(
        Arc::clone(&a.index),
        deps,
        Arc::new(ValidatingSink::default()),
    );
    // A project with no live copy is not excluded: its row proves the inbox was drained.
    let other = ProjectId(exec(
        &a.index.lock().unwrap(),
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('other', 'other', 0, 0)",
        [],
    ));
    let row = |project: ProjectId| {
        codotheca_core::sync::store::load(
            a.index.lock().unwrap().conn(),
            SyncTaskKind::ProjectRemote,
            Some(project.0),
        )
        .unwrap()
    };
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: ProjectId(a.project),
    });
    runner.enqueue(SyncTask::ProjectRemote { project_id: other });
    runner.start();
    let drained = wait_for(|| row(other).is_some());
    runner.request_stop();
    runner.join();
    assert!(drained, "row 10: the inbox was never drained");
    assert!(row(ProjectId(a.project)).is_none(), "row 10 queued a task");
    assert_eq!(forge.request_count(), 0, "row 10 read the forge");
}

// ---------------------------------------------------------------------------
// Rows 11 and 12: the whole list handed to a reader that applies it itself.
// ---------------------------------------------------------------------------

fn row_11_wsl(a: &Audit) {
    let request = codotheca_core::wsl::dispatch::walk_request(
        &a.gate(),
        "distro-a",
        "/home/u/dev",
        &WalkOptions::default(),
    );
    // The copy's entry is native, so it never crosses into a distro.
    assert!(request.skip_paths.is_empty(), "row 11: {request:?}");
}

fn row_12_first_run(a: &Audit) {
    let root = a
        .copy
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("second-root");
    std::fs::create_dir_all(&root).unwrap();
    let answer = a
        .parts
        .borrow_mut()
        .handler
        .handle(
            "roots.add",
            json!({"pathBytes": Bytes(path_bytes(&root)), "confirmLarge": false, "distro": ""}),
        )
        .unwrap();
    eprintln!("  row 12: roots.add answered {answer}");
}

// ---------------------------------------------------------------------------
// The session row (GA33) and row 17.
// ---------------------------------------------------------------------------

fn row_session(a: &Audit) {
    let hooks = a.zone.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    // A git that does not exist: a spawn would fail, so only the check's own record and the
    // empty cache tell a refusal from a failed read.
    let check = Arc::new(GitIgnoreCheck {
        exec: Arc::new(GitExec::new(a.zone.join("no-such-git"), hooks)),
        limits: RunLimits::none(),
    });
    let key = location_key(a.index.lock().unwrap().conn(), LocationId(a.location)).unwrap();
    let mut filter = ScopeFilter::new(check, a.gate(), Some(key));
    let repo = RepoHandle::resolve(&a.copy, StoreKey::new("store"), StoreClass::Local).unwrap();
    assert_eq!(
        filter.fold(&repo, &[PathBuf::from("src/main.rs")]),
        Signal::None,
        "session row"
    );
    assert_eq!(filter.cached(), 0, "session row cached a verdict");
}

/// Row 17: an analysis admitted at its probe, whose copy is covered while it reads, does not
/// verify a remote.
fn row_17_verifying_read(a: &Audit) {
    let lib = support::analyser_world::Library::new();
    // Pushed in full to a network origin: uncovered, step 6 would verify it and answer `safe`.
    let path = lib.pushed_repo("widget");
    let id = lib.register(&path);
    let git = SwapOnFirstCall::new(&lib.read_git, a.gate(), covering(&path));
    let remotes = CountingVerifier::default();
    let trash = CountingTrash::new();
    let gate = a.gate();
    let seams = AnalyserSeams {
        exclusions: &gate,
        git: &git,
        remotes: &remotes,
        trash: &trash,
        before_act: None,
    };
    let answer = codotheca_core::uninstall::handle_preflight_off_lock(
        &lib.index,
        &seams,
        json!({ "locationId": id }),
        NOW,
    )
    .unwrap();
    eprintln!("  row 17: the pre-flight answered {answer}");
    assert!(git.fired(), "row 17: the analysis read nothing");
    assert_eq!(
        remotes.0.load(Ordering::SeqCst),
        0,
        "row 17 verified a remote"
    );
    assert_eq!(trash.sends(), 0);
}

/// The command rows 13–16 and the install case, each answering `EXCLUDED` before any read.
fn command_rows(a: &Audit) -> Vec<(&'static str, GateSite, bool)> {
    let before = a.row_path();
    let (p, l) = (a.project, a.location);
    let moved = json!(Bytes(path_bytes(&a.elsewhere)));
    a.cover_copy();
    let mut per_row = Vec::new();
    let commands: [(&str, GateSite, &str, Value); 6] = [
        (
            "13",
            GateSite::Readme,
            "projects.readme",
            json!({"projectId": p, "locationId": l}),
        ),
        (
            "13",
            GateSite::Readme,
            "projects.readmeAssets",
            json!({"projectId": p, "locationId": l, "local": ["logo.png"], "remote": []}),
        ),
        (
            "14 (the copy)",
            GateSite::Relocate,
            "locations.relocate",
            json!({"locationId": l, "pathBytes": moved}),
        ),
        (
            "15",
            GateSite::Uninstall,
            "locations.uninstallPreflight",
            json!({"locationId": l}),
        ),
        (
            "15",
            GateSite::Uninstall,
            "locations.uninstall",
            json!({"locationId": l}),
        ),
        (
            "16",
            GateSite::Trust,
            "problems.trust",
            json!({"problemId": a.problem}),
        ),
    ];
    for (row, site, command, args) in commands {
        a.refused(row, site, command, args);
        per_row.push((row, site, false));
    }

    // Row 14's other half: an allowed copy is not moved onto an excluded path.
    a.cover(&[&a.elsewhere]);
    assert!(!a.copy_excluded(), "apply_coverage did not clear the copy");
    a.refused(
        "14 (the new path)",
        GateSite::Relocate,
        "locations.relocate",
        json!({"locationId": l, "pathBytes": moved}),
    );
    assert_eq!(a.row_path(), before, "a refused relocate changed the row");
    per_row.push(("14 (the new path)", GateSite::Relocate, false));

    // An install whose destination is covered is refused before it is queued.
    let gadget = exec(
        &a.index.lock().unwrap(),
        "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
         VALUES ('gadget', 'gadget', 'github.com/owner/gadget', 0, 0)",
        [],
    );
    a.cover(&[&a.zone.join("gadget")]);
    a.refused(
        "install",
        GateSite::Handoff,
        "install.start",
        json!({"projectId": gadget, "rootId": a.root_id}),
    );
    per_row.push(("install", GateSite::Handoff, false));
    per_row
}

#[test]
fn ac_p4_56_13_the_gate_audit_drives_every_row_against_one_covered_copy() {
    let a = audit();
    let gate = a.gate();
    let mut per_row: Vec<(&str, GateSite, bool)> = Vec::new();
    let mut drive = |row: &'static str, site: GateSite, admitted: bool, f: &dyn Fn(&Audit)| {
        let from = gate.checks().len();
        f(&a);
        let made = a.checks_from(from);
        assert!(
            made.contains(&(site, admitted)),
            "row {row}: no {} {site:?} check in {made:?}",
            if admitted { "admitted" } else { "refused" }
        );
        per_row.push((row, site, admitted));
    };

    a.cover_copy();
    drive("1", GateSite::WalkDir, false, &row_1_walk);
    drive("3", GateSite::Presence, false, &row_3_presence);
    drive(
        "4 (hand-off)",
        GateSite::OwnershipProbe,
        false,
        &row_4_hand_off,
    );
    drive("4 (job)", GateSite::OwnershipProbe, false, &row_4_job_probe);
    drive("5", GateSite::Dispatch, false, &row_5_dispatch);
    drive("7", GateSite::VisibleEnqueue, false, &row_7_page_open);
    drive("8", GateSite::Requeue, false, &row_8_requeue);
    drive("9", GateSite::WatchSet, false, &row_9_watch_set);
    drive("10", GateSite::RemoteSync, false, &row_10_remote_sync);
    drive("11", GateSite::Wsl, true, &row_11_wsl);
    drive("12", GateSite::FirstRun, true, &row_12_first_run);
    drive("session", GateSite::Session, false, &row_session);
    drive("2", GateSite::Handoff, false, &row_2_hand_off_write);
    drive("6", GateSite::JobWrite, false, &row_6_job_write);
    drive("17", GateSite::Fetch, false, &row_17_verifying_read);

    per_row.extend(command_rows(&a));

    let exercised: Vec<GateSite> = GateSite::ALL
        .into_iter()
        .filter(|site| per_row.iter().any(|(_, s, _)| s == site))
        .collect();
    for (row, site, admitted) in &per_row {
        let how = if *admitted {
            "admitted snapshot"
        } else {
            "refused"
        };
        eprintln!("  row {row}: {site:?} {how}");
    }
    let missing: Vec<GateSite> = GateSite::ALL
        .into_iter()
        .filter(|site| !exercised.contains(site))
        .collect();
    eprintln!(
        "entry points exercised: {} of {} (not driven: {missing:?} — Recheck lands with p4-54)",
        exercised.len(),
        GateSite::ALL.len()
    );
    assert!(
        exercised.len() >= GateSite::ALL.len() - 1,
        "below the table's count: {missing:?}"
    );
    assert_eq!(missing, [GateSite::Recheck], "only Recheck may be undriven");
}

// ---------------------------------------------------------------------------
// The git seam that swaps on its first call: every read passes through to `inner`, after the
// first has put `next` in force — an exclusion added while a read was running.
// ---------------------------------------------------------------------------

struct SwapOnFirstCall<'a> {
    inner: &'a dyn GitBackend,
    gate: Arc<ExclusionGate>,
    next: Mutex<Option<Arc<ExclusionList>>>,
    calls: AtomicUsize,
}

impl std::fmt::Debug for SwapOnFirstCall<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwapOnFirstCall").finish_non_exhaustive()
    }
}

impl<'a> SwapOnFirstCall<'a> {
    fn new(inner: &'a dyn GitBackend, gate: Arc<ExclusionGate>, next: Arc<ExclusionList>) -> Self {
        Self {
            inner,
            gate,
            next: Mutex::new(Some(next)),
            calls: AtomicUsize::new(0),
        }
    }

    fn fired(&self) -> bool {
        self.calls.load(Ordering::SeqCst) > 0
    }

    fn fire(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let next = self.next.lock().unwrap().take();
        if let Some(next) = next {
            let _ = self.gate.swap(next);
        }
    }
}
impl GitBackend for SwapOnFirstCall<'_> {
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        self.fire();
        self.inner.version(ctx)
    }
    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.fire();
        self.inner.repo_facts(repo, ctx)
    }
    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState> {
        self.fire();
        self.inner.ref_state(repo, ctx)
    }
    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.fire();
        self.inner.divergence(repo, state, ctx)
    }
    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        self.fire();
        self.inner.worktree_status(repo, opts, ctx)
    }
    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.fire();
        self.inner.tracked_inventory(repo, ctx)
    }
    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<std::collections::BTreeMap<Vec<u8>, String>> {
        self.fire();
        self.inner.submodule_gitlinks(repo, paths, ctx)
    }
    fn remote_urls(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<(String, String)>> {
        self.fire();
        self.inner.remote_urls(repo, ctx)
    }
    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.fire();
        self.inner.root_commits(repo, ctx)
    }
    fn unpushed_refs(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<String>> {
        self.fire();
        self.inner.unpushed_refs(repo, ctx)
    }
    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.fire();
        self.inner.authorship(repo, ctx)
    }
    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.fire();
        self.inner.commit_subjects(repo, limit, ctx)
    }
    fn head_tree(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<codotheca_core::git::TreeEntry>> {
        self.fire();
        self.inner.head_tree(repo, ctx)
    }
    fn read_blobs(
        &self,
        repo: &RepoHandle,
        oids: &[String],
        byte_cap: u64,
        budget_bytes: u64,
        ctx: &JobContext<'_>,
    ) -> GitResult<codotheca_core::git::BlobBatch> {
        self.fire();
        self.inner
            .read_blobs(repo, oids, byte_cap, budget_bytes, ctx)
    }
    fn enumerate_refs(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<codotheca_core::git::RefListing> {
        self.fire();
        self.inner.enumerate_refs(repo, ctx)
    }
    fn stash_entries(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<codotheca_core::git::StashEntries> {
        self.fire();
        self.inner.stash_entries(repo, ctx)
    }
    fn worktree_scan(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<codotheca_core::git::WorktreeScan> {
        self.fire();
        self.inner.worktree_scan(repo, ctx)
    }
    fn objects_present(
        &self,
        repo: &RepoHandle,
        oids: &[String],
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<bool>> {
        self.fire();
        self.inner.objects_present(repo, oids, ctx)
    }
    fn any_uncovered(
        &self,
        repo: &RepoHandle,
        roots: &[String],
        covered: &[String],
        ctx: &JobContext<'_>,
    ) -> GitResult<bool> {
        self.fire();
        self.inner.any_uncovered(repo, roots, covered, ctx)
    }
    fn interrupted_ops(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<codotheca_core::git::InterruptedOperation>> {
        self.fire();
        self.inner.interrupted_ops(repo, ctx)
    }
    fn release_tags(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<codotheca_core::git::TagFact>> {
        self.fire();
        self.inner.release_tags(repo, ctx)
    }
    fn archive_reach(
        &self,
        repo: &RepoHandle,
        detached_head: Option<&str>,
        extra_excluded: &[String],
        ctx: &JobContext<'_>,
    ) -> GitResult<bool> {
        self.fire();
        self.inner
            .archive_reach(repo, detached_head, extra_excluded, ctx)
    }
}

//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §56.5 rows 4–10 against the real `JobRunner`, job writers, requeue and sync runner: a copy the
//! exclusion list covers is dropped at dispatch, refused before its ownership probe, written by
//! no job and settled by none, queued by no page-open, revived by no requeue, and read by no
//! remote sync — whether the remote read was asked for after the add or queued before it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::exclusion::gate::{ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, PathEntry};
use codotheca_core::git::{
    Authorship, CommitSubject, Divergence, GitBackend, GitResult, GitVersion, JobContext, RefState,
    RepoFacts, RepoHandle, RootCommit, StatusOptions, TrackedInventory, WorktreeStatus,
};
use codotheca_core::http::HttpTransport;
use codotheca_core::index::path::native_platform;
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{run_one, Job, JobDeps, JobError, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::{LocationId, ProjectId, SyncTaskKind, SyncTaskState};
use codotheca_core::surfaces::SurfaceCtx;
use codotheca_core::sync::state::SyncTaskStateRow;
use codotheca_core::sync::task::SyncTask;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, FakeTokenStore, FakeTransport, GitReply,
};

const NOW: i64 = 1_800_000_000;
const DEADLINE: Duration = Duration::from_secs(20);

/// The host's `location.kind`, whose path rules are `native_platform()`'s.
const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// A list whose one user path entry covers `dir` on the host.
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

/// One project on one present copy. The copy's directory exists and holds nothing: every git
/// answer comes from a fake.
struct Rig {
    _dir: tempfile::TempDir,
    copy: PathBuf,
    index: Arc<Mutex<Index>>,
    gate: Arc<ExclusionGate>,
    project: ProjectId,
    location: LocationId,
}

impl Rig {
    /// `covered`: whether the gate covers the copy from the start.
    fn new(covered: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("zone").join("widget");
        std::fs::create_dir_all(&copy).unwrap();
        let index = Index::open_at(&dir.path().join("index"), NOW).unwrap();
        let project = insert_project(&index, "widget");
        index
            .conn()
            .execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'store', 'present', 'worktree')",
                rusqlite::params![
                    project.0,
                    host_kind(),
                    path_bytes(&copy),
                    path_key(&copy, native_platform()),
                    path_display(&copy)
                ],
            )
            .unwrap();
        let location = LocationId(index.conn().last_insert_rowid());
        let gate = Arc::new(if covered {
            ExclusionGate::new((*covering(&copy)).clone())
        } else {
            ExclusionGate::default()
        });
        Self {
            _dir: dir,
            copy,
            index: Arc::new(Mutex::new(index)),
            gate,
            project,
            location,
        }
    }

    fn job(&self, kind: JobKind) -> Job {
        Job {
            kind,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }
    }

    fn deps(&self, git: Arc<dyn GitBackend>) -> JobDeps {
        JobDeps {
            exclusions: Arc::clone(&self.gate),
            git,
            // Never moves, so a requeued job's backoff never elapses and it is not run again.
            clock: Arc::new(FakeClock::new(NOW)),
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        }
    }

    fn runner(&self, git: Arc<dyn GitBackend>) -> Arc<JobRunner> {
        JobRunner::new(
            Arc::clone(&self.index),
            self.deps(git),
            Arc::new(ValidatingSink::default()),
        )
    }

    /// Every `project_job_state` row of the project, as `(job, state)`.
    fn job_rows(&self) -> Vec<(String, String)> {
        let guard = self.index.lock().unwrap();
        let mut st = guard
            .conn()
            .prepare("SELECT job, state FROM project_job_state WHERE project_id = ?1 ORDER BY job")
            .unwrap();
        let rows = st
            .query_map([self.project.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(st);
        drop(guard);
        rows
    }

    fn head_oid(&self) -> Option<String> {
        self.index
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT head_oid FROM location WHERE id = ?1",
                [self.location.0],
                |r| r.get(0),
            )
            .unwrap()
    }
}

fn insert_project(index: &Index, name: &str) -> ProjectId {
    index
        .conn()
        .execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES (?1, ?1, 0, 0)",
            [name],
        )
        .unwrap();
    ProjectId(index.conn().last_insert_rowid())
}

/// Poll until `f` holds or the deadline passes. The pools are asynchronous by construction.
fn wait_for(mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if f() {
            return true;
        }
        if start.elapsed() >= DEADLINE {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Start `runner` on one worker, let it run everything due, and stop it.
fn drain(runner: &Arc<JobRunner>) {
    runner.start(1);
    // `is_idle` is true before the worker takes the first job only if nothing is due.
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
// Row 5: dispatch drops a covered copy's job of every kind but J5.
// ---------------------------------------------------------------------------

#[test]
fn dispatch_drops_every_job_kind_on_a_covered_copy_and_j5_is_exempt() {
    let mut exercised = 0;
    for kind in JobKind::ALL {
        let rig = Rig::new(true);
        let git = Arc::new(FakeGitBackend::new());
        let runner = rig.runner(git.clone());
        assert!(runner.enqueue(rig.job(kind)), "{kind:?} was not queued");
        drain(&runner);
        if kind == JobKind::J5Art {
            // J5 reads no repository: it is asked nothing, and its settle writes its row.
            assert!(
                rig.gate.checks().is_empty(),
                "J5 asked the gate: {:?}",
                rig.gate.checks()
            );
            assert_eq!(rig.job_rows().len(), 1, "J5 did not run");
            continue;
        }
        assert_eq!(
            rig.gate.checks(),
            [(GateSite::Dispatch, false)],
            "{kind:?} was not dropped at dispatch"
        );
        assert!(
            rig.job_rows().is_empty(),
            "{kind:?} settled: {:?}",
            rig.job_rows()
        );
        assert!(
            git.calls().is_empty(),
            "{kind:?} read git: {:?}",
            git.calls()
        );
        assert!(
            runner.queued().is_empty(),
            "{kind:?} chained {:?}",
            runner.queued()
        );
        exercised += 1;
    }
    eprintln!("exclusion_jobs: exercised {exercised} job kinds");
    assert!(exercised >= JobKind::ALL.len() - 1);
}

// ---------------------------------------------------------------------------
// Row 4: the ownership probe asks before any read, for a job that reached `run_one`.
// ---------------------------------------------------------------------------

#[test]
fn run_one_refuses_a_covered_copy_before_its_ownership_probe() {
    let rig = Rig::new(true);
    let git = Arc::new(FakeGitBackend::new());
    git.always_ref_state(GitReply::Ok(ref_state()));
    git.always_divergence(GitReply::Ok(None));
    git.always_archive_reach(GitReply::Ok(false));
    let deps = rig.deps(git.clone());
    let announce = RefCell::new(Vec::new());
    let out = run_one(&rig.index, &deps, &rig.job(JobKind::J1Refstate), &announce);
    assert!(matches!(out, Err(JobError::Excluded)), "{out:?}");
    assert_eq!(rig.gate.checks(), [(GateSite::OwnershipProbe, false)]);
    assert!(git.calls().is_empty(), "read git: {:?}", git.calls());
    assert_eq!(rig.head_oid(), None);
}

// ---------------------------------------------------------------------------
// Row 6: a job admitted before the add writes nothing, and its settle writes nothing.
// ---------------------------------------------------------------------------

/// Every read passes through to `inner`; the first one swaps `next` into the gate, so the add
/// lands after dispatch and the ownership probe admitted the job.
struct SwapOnFirstRead {
    inner: FakeGitBackend,
    gate: Arc<ExclusionGate>,
    next: Mutex<Option<Arc<ExclusionList>>>,
    swapped: AtomicBool,
}

impl std::fmt::Debug for SwapOnFirstRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwapOnFirstRead").finish_non_exhaustive()
    }
}

impl SwapOnFirstRead {
    fn new(inner: FakeGitBackend, rig: &Rig) -> Self {
        Self {
            inner,
            gate: Arc::clone(&rig.gate),
            next: Mutex::new(Some(covering(&rig.copy))),
            swapped: AtomicBool::new(false),
        }
    }

    fn fire(&self) {
        let next = self.next.lock().unwrap().take();
        if let Some(next) = next {
            let _ = self.gate.swap(next);
            self.swapped.store(true, Ordering::SeqCst);
        }
    }
}

#[test]
fn a_job_admitted_before_the_add_writes_nothing_and_chains_nothing() {
    let rig = Rig::new(false);
    let inner = FakeGitBackend::new();
    inner.always_ref_state(GitReply::Ok(ref_state()));
    inner.always_divergence(GitReply::Ok(None));
    inner.always_archive_reach(GitReply::Ok(false));
    let git = Arc::new(SwapOnFirstRead::new(inner, &rig));
    let runner = rig.runner(git.clone());
    assert!(runner.enqueue(rig.job(JobKind::J1Refstate)));
    drain(&runner);
    assert!(git.swapped.load(Ordering::SeqCst), "the job never read git");
    let checks = rig.gate.checks();
    assert!(checks.contains(&(GateSite::Dispatch, true)), "{checks:?}");
    assert!(
        checks.contains(&(GateSite::OwnershipProbe, true)),
        "{checks:?}"
    );
    assert!(checks.contains(&(GateSite::JobWrite, false)), "{checks:?}");
    // The writer's own check: J1's facts never reach the row.
    assert_eq!(rig.head_oid(), None, "J1 wrote under the new list");
    assert!(rig.job_rows().is_empty(), "settled: {:?}", rig.job_rows());
    assert!(runner.queued().is_empty(), "chained {:?}", runner.queued());
}

/// The job fails after the add, so no writer runs and only the settle's own check stands
/// between it and a `failed` row with a requeue behind it.
#[test]
fn the_settle_of_a_job_that_failed_after_the_add_writes_nothing() {
    let rig = Rig::new(false);
    // Unconfigured: `ref_state` fails, so `run_one` never reaches a writer.
    let git = Arc::new(SwapOnFirstRead::new(FakeGitBackend::new(), &rig));
    let runner = rig.runner(git.clone());
    assert!(runner.enqueue(rig.job(JobKind::J1Refstate)));
    drain(&runner);
    assert!(git.swapped.load(Ordering::SeqCst), "the job never read git");
    assert!(
        rig.job_rows().is_empty(),
        "the settle wrote {:?}",
        rig.job_rows()
    );
    assert!(runner.queued().is_empty(), "requeued {:?}", runner.queued());
}

// ---------------------------------------------------------------------------
// Row 7: page-open queues nothing for a covered copy, with the index guard held.
// ---------------------------------------------------------------------------

fn visible_queues(rig: &Rig) -> usize {
    let runner = rig.runner(Arc::new(FakeGitBackend::new()));
    let (index, gate, sink) = (
        Arc::clone(&rig.index),
        Arc::clone(&rig.gate),
        Arc::clone(&runner),
    );
    let (project, location) = (rig.project, rig.location);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // What `Assembly::handle` holds while a projects or detail command asks.
        let guard = index.lock().unwrap();
        codotheca_core::jobs::visible::notify_visible(
            &guard,
            &FakeMountResolver::new(),
            sink.as_ref(),
            &gate,
            project,
            location,
            true,
        );
        drop(guard);
        let _ = done_tx.send(());
    });
    assert!(
        done_rx.recv_timeout(DEADLINE).is_ok(),
        "notify_visible did not return while the index guard was held"
    );
    runner.queued().len()
}

#[test]
fn a_covered_copy_opened_on_the_page_queues_nothing() {
    // Control: the same copy, uncovered, is queued a reading.
    assert!(visible_queues(&Rig::new(false)) > 0);
    let rig = Rig::new(true);
    assert_eq!(visible_queues(&rig), 0);
    assert_eq!(rig.gate.checks(), [(GateSite::VisibleEnqueue, false)]);
}

// ---------------------------------------------------------------------------
// Row 8: requeue revives nothing for a project whose every copy is covered.
// ---------------------------------------------------------------------------

#[test]
fn requeue_answers_zero_for_a_fully_covered_project() {
    let rig = Rig::new(true);
    let guard = rig.index.lock().unwrap();
    guard
        .conn()
        .execute(
            "INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
             VALUES (?1, 'j2', 'failed', 3, 'permission denied', 10)",
            [rig.project.0],
        )
        .unwrap();
    let args = serde_json::json!({ "id": rig.project.0 });
    let ctx = SurfaceCtx {
        index: &guard,
        now: NOW,
        exclusions: &rig.gate,
    };
    let answer = codotheca_core::surfaces::repair::handle_requeue(&ctx, args.clone()).unwrap();
    assert_eq!(answer.jobs_requeued, 0);
    assert_eq!(rig.gate.checks(), [(GateSite::Requeue, false)]);
    let state: String = guard
        .conn()
        .query_row(
            "SELECT state FROM project_job_state WHERE project_id = ?1",
            [rig.project.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "failed");
    // Control: once the list no longer covers it, the same requeue revives the row.
    let _ = rig.gate.swap(Arc::new(ExclusionList::default()));
    let revived = codotheca_core::surfaces::repair::handle_requeue(&ctx, args).unwrap();
    assert_eq!(revived.jobs_requeued, 1);
    drop(guard);
}

// ---------------------------------------------------------------------------
// Row 10: no remote read for a fully covered project, asked after the add or queued before it.
// ---------------------------------------------------------------------------

fn sync_runner(
    rig: &Rig,
    forge: &Arc<FakeTransport>,
) -> Arc<codotheca_core::sync::runner::SyncRunner> {
    let clock = Arc::new(FakeClock::new(NOW));
    let http: Arc<dyn HttpTransport> = forge.clone();
    let observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        http,
        clock.clone(),
    ));
    let provider_http: Arc<dyn HttpTransport> = observing.clone();
    let deps = SyncDeps {
        exclusions: Arc::clone(&rig.gate),
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
    codotheca_core::sync::runner::SyncRunner::new(
        Arc::clone(&rig.index),
        deps,
        Arc::new(ValidatingSink::default()),
    )
}

fn remote_row(rig: &Rig, project: ProjectId) -> Option<SyncTaskStateRow> {
    let guard = rig.index.lock().unwrap();
    codotheca_core::sync::store::load(guard.conn(), SyncTaskKind::ProjectRemote, Some(project.0))
        .unwrap()
}

#[test]
fn a_remote_read_asked_for_a_fully_covered_project_is_never_queued() {
    let rig = Rig::new(true);
    // A project with no live copy is not excluded: its row proves the inbox was drained.
    let other = insert_project(&rig.index.lock().unwrap(), "other");
    let forge = Arc::new(FakeTransport::new());
    let runner = sync_runner(&rig, &forge);
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: rig.project,
    });
    runner.enqueue(SyncTask::ProjectRemote { project_id: other });
    runner.start();
    assert!(
        wait_for(|| remote_row(&rig, other).is_some()),
        "the inbox was never drained"
    );
    runner.request_stop();
    runner.join();
    assert!(remote_row(&rig, rig.project).is_none());
    assert!(rig.gate.checks().contains(&(GateSite::RemoteSync, false)));
}

#[test]
fn a_remote_read_queued_before_the_add_is_refused_where_it_runs() {
    let rig = Rig::new(true);
    {
        let mut guard = rig.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                codotheca_core::sync::store::put(
                    tx,
                    &SyncTaskStateRow::queued(
                        SyncTaskKind::ProjectRemote,
                        Some(rig.project.0),
                        NOW,
                    ),
                )
            })
            .unwrap();
    }
    let forge = Arc::new(FakeTransport::new());
    let runner = sync_runner(&rig, &forge);
    runner.start();
    let settled = wait_for(|| {
        remote_row(&rig, rig.project).is_some_and(|r| r.state != SyncTaskState::Queued)
    });
    runner.request_stop();
    runner.join();
    assert!(settled, "the queued row was never taken");
    let row = remote_row(&rig, rig.project).unwrap();
    assert_eq!(row.state, SyncTaskState::Deferred, "{row:?}");
    assert_eq!(row.reason.as_deref(), Some("excluded"));
    assert_eq!(forge.request_count(), 0, "the forge was read");
    assert!(rig.gate.checks().contains(&(GateSite::RemoteSync, false)));
}

// ---------------------------------------------------------------------------
// The git seam that swaps on its first read: every read passes through to `inner`.
// ---------------------------------------------------------------------------

impl GitBackend for SwapOnFirstRead {
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

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The scan hand-off: a discovered repository becomes one `project` row and one `location` row.
//!
//! Until `assembly::handoff` existed, `resolve_identity` and `identity::store::upsert_location`
//! had no production caller at all, so a walk found repositories and persisted nothing (R1).
//! What is asserted here is what would each fail silently: that one repository is one of each,
//! that a rescan is not a second row, that two checkouts of one clone are two locations under
//! **one** project, that an unreadable repository is reported rather than dropped — and that the
//! index lock is not held while git runs (**R39**).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, HandoffError};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{
    Authorship, CommitSubject, Divergence, GitBackend, GitExec, GitResult, GitSlots, GitVersion,
    JobContext, RefState, RepoFacts, RepoHandle, RootCommit, StatusOptions, StoreKey, SystemGit,
    TrackedInventory, WorktreeStatus,
};
use codotheca_core::index::Index;
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};

const NOW: i64 = 1_760_000_000;
const GENERATION: i64 = 7;

/// A neutral git, so the machine's own config cannot change what a fixture is.
fn git_at(cwd: &Path, home: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Rig {
    dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let index = Arc::new(Mutex::new(Index::open(&dir.path().join("index")).unwrap()));
        Self { dir, index }
    }

    fn home(&self) -> std::path::PathBuf {
        self.dir.path().join("home")
    }

    /// One repository with one commit, at `<tmp>/<name>`.
    fn repo(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        git_at(&path, &self.home(), &["init", "-b", "main", "."]);
        std::fs::write(path.join("a.txt"), b"one\n").unwrap();
        git_at(&path, &self.home(), &["add", "-A"]);
        git_at(&path, &self.home(), &["commit", "-m", "first"]);
        path
    }

    fn git(&self) -> Arc<dyn GitBackend> {
        let hooks =
            codotheca_core::git::ensure_empty_hooks_dir(&self.dir.path().join("hooks")).unwrap();
        Arc::new(SystemGit::new(
            Arc::new(GitExec::system(hooks)),
            Arc::new(GitSlots::for_machine()),
            Arc::new(SystemClock::new()),
        ))
    }

    fn count(&self, table: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let n = guard.conn().query_row(&sql, [], |r| r.get(0)).unwrap();
        drop(guard);
        n
    }
}

/// What the walk hands on, built from what `classify` would have resolved.
fn discovered_at(path: &Path) -> Discovered {
    let handle = RepoHandle::resolve(path, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: handle.git_dir.clone(),
            common_dir: handle.common_dir,
        },
        root_id: 1,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, platform_of("linux")),
        path_display: path_display(path),
        store_key: "store-a".to_owned(),
        volume_key: Some("vol-a".to_owned()),
    }
}

fn ctx<'a>(git: &'a dyn GitBackend, cancel: &'a CancelToken) -> HandoffCtx<'a> {
    HandoffCtx {
        git,
        cancel,
        store_class: StoreClass::Local,
        generation: GENERATION,
        now: NOW,
    }
}

#[test]
fn one_repository_becomes_one_project_and_one_location() {
    let rig = Rig::new();
    let path = rig.repo("widget");
    let git = rig.git();
    let cancel = CancelToken::new();

    let indexed = hand_off_discovered(
        &rig.index,
        &ctx(git.as_ref(), &cancel),
        &discovered_at(&path),
    )
    .expect("a repository with one commit is indexable");
    assert!(indexed.project.0 > 0);
    assert!(indexed.location.0 > 0);
    assert_eq!(rig.count("project"), 1);
    assert_eq!(rig.count("location"), 1);

    let guard = rig.index.lock().unwrap();
    let (name, generation, presence): (String, i64, String) = guard
        .conn()
        .query_row(
            "SELECT p.name, l.scan_generation, l.presence
               FROM location l JOIN project p ON p.id = l.project_id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    drop(guard);
    assert_eq!(
        name, "widget",
        "the project's first name is its directory's"
    );
    assert_eq!(generation, GENERATION);
    assert_eq!(
        presence, "present",
        "the walk has just looked at it; anything else claims something about a directory \
         nobody visited"
    );
}

#[test]
fn the_observation_columns_are_left_uncomputed() {
    // "Never render unknown as zero" starts here: the row a scan writes carries no branch, no
    // dirty flag and no counts, so the shelf can say *not computed* rather than *clean*.
    let rig = Rig::new();
    let path = rig.repo("widget");
    let git = rig.git();
    let cancel = CancelToken::new();
    hand_off_discovered(
        &rig.index,
        &ctx(git.as_ref(), &cancel),
        &discovered_at(&path),
    )
    .unwrap();

    let guard = rig.index.lock().unwrap();
    let (branch, dirty, observed): (Option<String>, Option<i64>, Option<i64>) = guard
        .conn()
        .query_row(
            "SELECT branch, is_dirty, refstate_observed_at FROM location",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    drop(guard);
    assert_eq!((branch, dirty, observed), (None, None, None));
}

#[test]
fn scanning_the_same_tree_twice_writes_one_row() {
    let rig = Rig::new();
    let path = rig.repo("widget");
    let git = rig.git();
    let cancel = CancelToken::new();
    let discovered = discovered_at(&path);

    let first = hand_off_discovered(&rig.index, &ctx(git.as_ref(), &cancel), &discovered).unwrap();
    let again = hand_off_discovered(&rig.index, &ctx(git.as_ref(), &cancel), &discovered).unwrap();

    assert_eq!(
        (first.project, first.location),
        (again.project, again.location),
        "`(kind, distro, path_key)` is one row; a second insert would put one repository on the \
         shelf twice"
    );
    assert_eq!(rig.count("project"), 1);
    assert_eq!(rig.count("location"), 1);
}

#[test]
fn two_checkouts_of_one_clone_are_two_locations_under_one_project() {
    let rig = Rig::new();
    let main = rig.repo("widget");
    // A linked worktree shares its `commondir`, which is §1.1's *definitive* evidence: the same
    // repository in a second place, never a second project.
    let linked = rig.dir.path().join("widget-wt");
    git_at(
        &main,
        &rig.home(),
        &["worktree", "add", linked.to_str().unwrap(), "-b", "side"],
    );
    let git = rig.git();
    let cancel = CancelToken::new();

    let a = hand_off_discovered(
        &rig.index,
        &ctx(git.as_ref(), &cancel),
        &discovered_at(&main),
    )
    .unwrap();
    let b = hand_off_discovered(
        &rig.index,
        &ctx(git.as_ref(), &cancel),
        &discovered_at(&linked),
    )
    .unwrap();

    assert_eq!(
        a.project, b.project,
        "the shared common dir is definitive evidence; two projects here would put one \
         repository on the shelf twice"
    );
    assert_ne!(a.location, b.location);
    assert_eq!(rig.count("project"), 1);
    assert_eq!(rig.count("location"), 2);
}

#[test]
fn an_unreadable_repository_is_reported_and_writes_nothing() {
    let rig = Rig::new();
    // A directory the walk classified as a repository whose git dir has since gone. The
    // hand-off must say so: a silent skip would make the summary claim a completeness the scan
    // did not have, and a panic would take the whole run down with it.
    let path = rig.repo("widget");
    let discovered = discovered_at(&path);
    std::fs::remove_dir_all(path.join(".git")).unwrap();

    let git = rig.git();
    let cancel = CancelToken::new();
    let err =
        hand_off_discovered(&rig.index, &ctx(git.as_ref(), &cancel), &discovered).unwrap_err();
    assert!(matches!(err, HandoffError::Git(_)), "{err:?}");
    assert_eq!(rig.count("project"), 0);
    assert_eq!(rig.count("location"), 0);
}

#[test]
fn an_unrecognised_location_kind_is_refused_rather_than_guessed() {
    let rig = Rig::new();
    let path = rig.repo("widget");
    let mut discovered = discovered_at(&path);
    discovered.kind = "plan9".to_owned();
    let git = rig.git();
    let cancel = CancelToken::new();

    let err =
        hand_off_discovered(&rig.index, &ctx(git.as_ref(), &cancel), &discovered).unwrap_err();
    assert!(matches!(err, HandoffError::UnknownKind(_)), "{err:?}");
    assert_eq!(rig.count("location"), 0);
}

// ---------------------------------------------------------------------------
// R39: the lock is taken to read the inputs, released, git runs, and taken
// again to write. Nothing else in the suite would notice it being held.
// ---------------------------------------------------------------------------

/// Wraps a backend and, on every call, asks whether the index mutex is free.
///
/// `std::sync::Mutex::try_lock` answers `WouldBlock` for a lock this same thread already holds,
/// so a hand-off that kept the guard across a git invocation is observable here rather than
/// being a deadlock the suite would time out on.
struct LockWatch {
    inner: Arc<dyn GitBackend>,
    index: Arc<Mutex<Index>>,
    held_during_a_call: Arc<AtomicBool>,
    calls: Arc<AtomicBool>,
}

impl std::fmt::Debug for LockWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockWatch").finish_non_exhaustive()
    }
}

impl LockWatch {
    fn observe(&self) {
        self.calls.store(true, Ordering::SeqCst);
        match self.index.try_lock() {
            Ok(guard) => drop(guard),
            Err(_) => self.held_during_a_call.store(true, Ordering::SeqCst),
        }
    }
}

impl GitBackend for LockWatch {
    fn version(&self, ctx: &JobContext<'_>) -> GitResult<GitVersion> {
        self.observe();
        self.inner.version(ctx)
    }
    fn repo_facts(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RepoFacts> {
        self.observe();
        self.inner.repo_facts(repo, ctx)
    }
    fn ref_state(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<RefState> {
        self.observe();
        self.inner.ref_state(repo, ctx)
    }
    fn divergence(
        &self,
        repo: &RepoHandle,
        state: &RefState,
        ctx: &JobContext<'_>,
    ) -> GitResult<Option<Divergence>> {
        self.observe();
        self.inner.divergence(repo, state, ctx)
    }
    fn worktree_status(
        &self,
        repo: &RepoHandle,
        opts: StatusOptions,
        ctx: &JobContext<'_>,
    ) -> GitResult<WorktreeStatus> {
        self.observe();
        self.inner.worktree_status(repo, opts, ctx)
    }
    fn tracked_inventory(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<TrackedInventory> {
        self.observe();
        self.inner.tracked_inventory(repo, ctx)
    }
    fn submodule_gitlinks(
        &self,
        repo: &RepoHandle,
        paths: &[Vec<u8>],
        ctx: &JobContext<'_>,
    ) -> GitResult<std::collections::BTreeMap<Vec<u8>, String>> {
        self.observe();
        self.inner.submodule_gitlinks(repo, paths, ctx)
    }
    fn remote_urls(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<(String, String)>> {
        self.observe();
        self.inner.remote_urls(repo, ctx)
    }
    fn root_commits(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<RootCommit>> {
        self.observe();
        self.inner.root_commits(repo, ctx)
    }
    fn unpushed_refs(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Vec<String>> {
        self.observe();
        self.inner.unpushed_refs(repo, ctx)
    }
    fn authorship(&self, repo: &RepoHandle, ctx: &JobContext<'_>) -> GitResult<Authorship> {
        self.observe();
        self.inner.authorship(repo, ctx)
    }
    fn commit_subjects(
        &self,
        repo: &RepoHandle,
        limit: u32,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<CommitSubject>> {
        self.observe();
        self.inner.commit_subjects(repo, limit, ctx)
    }
    fn head_tree(
        &self,
        repo: &RepoHandle,
        ctx: &JobContext<'_>,
    ) -> GitResult<Vec<codotheca_core::git::TreeEntry>> {
        self.observe();
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
        self.observe();
        self.inner
            .read_blobs(repo, oids, byte_cap, budget_bytes, ctx)
    }
}

#[test]
fn the_index_lock_is_not_held_across_a_git_invocation() {
    let rig = Rig::new();
    let path = rig.repo("widget");
    let held = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicBool::new(false));
    let watch = LockWatch {
        inner: rig.git(),
        index: Arc::clone(&rig.index),
        held_during_a_call: Arc::clone(&held),
        calls: Arc::clone(&calls),
    };
    let cancel = CancelToken::new();

    hand_off_discovered(&rig.index, &ctx(&watch, &cancel), &discovered_at(&path)).unwrap();

    assert!(
        calls.load(Ordering::SeqCst),
        "a run that made no git call would pass the assertion below while proving nothing"
    );
    assert!(
        !held.load(Ordering::SeqCst),
        "R39: the one rusqlite connection was held while git ran. A twenty-second history read \
         would block every command in the process"
    );
}

/// The probe's deadline is the walk's own, not a second number: `scan::discover`'s
/// `GIT_PROBE_TIMEOUT` is what a per-repository probe is allowed, and one repository taking 28%
/// of a whole scan is why that ceiling exists at all.
#[test]
fn the_probe_carries_a_deadline() {
    assert_eq!(
        codotheca_core::scan::discover::GIT_PROBE_TIMEOUT,
        Duration::from_secs(5)
    );
}

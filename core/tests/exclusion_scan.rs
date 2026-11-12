//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §56.5 rows 1–4: the walk, the hand-off, presence and the hand-off's ownership probe each ask
//! the exclusion gate before they read, and the hand-off asks again before it writes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, HandoffError};
use codotheca_core::cancel::CancelToken;
use codotheca_core::exclusion::gate::{ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, PathEntry};
use codotheca_core::git::{
    Authorship, CommitSubject, Divergence, GitBackend, GitResult, GitVersion, JobContext, RefState,
    RepoFacts, RepoHandle, RootCommit, StatusOptions, StoreKey, TrackedInventory, WorktreeStatus,
};
use codotheca_core::index::path::native_platform;
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::Presence;
use codotheca_core::scan::discover::{ProbeCtx, RepoCandidate, RepoKind};
use codotheca_core::scan::links::LinkPolicy;
use codotheca_core::scan::presence::{
    classify_presence, LocationPresenceRow, PresenceContext, ScanRootRow,
};
use codotheca_core::scan::run::Discovered;
use codotheca_core::scan::walk::{walk_root, WalkCtx};
use codotheca_core::scan::{WalkEvent, WalkOptions};
use codotheca_core::testing::{FakeGitBackend, FakeMountResolver, GitReply, RecordingGitBackend};

const NOW: i64 = 1_760_000_000;

/// The host's `location.kind`, whose path rules are `native_platform()`'s.
const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

fn repo_at(base: &Path, rel: &str) -> PathBuf {
    let p = base.join(rel);
    std::fs::create_dir_all(p.join(".git/objects")).unwrap();
    std::fs::create_dir_all(p.join(".git/refs")).unwrap();
    std::fs::write(p.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    p
}

/// A user path entry on the host for `dir`.
fn host_entry(dir: &Path) -> PathEntry {
    PathEntry {
        distro: String::new(),
        platform: native_platform(),
        match_key: path_key(dir, native_platform()),
    }
}

fn covering(dir: &Path) -> Arc<ExclusionList> {
    Arc::new(ExclusionList::new(&[], vec![host_entry(dir)]))
}

// ---------------------------------------------------------------------------
// Row 1: the walk re-reads the gate when its epoch moves.
// ---------------------------------------------------------------------------

fn links_for(root: &Path) -> Arc<LinkPolicy> {
    let mounts = FakeMountResolver::new();
    mounts.map(
        root,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    Arc::new(LinkPolicy::new(
        false,
        vec![path_key(root, native_platform())],
        BTreeSet::from(["store-a".to_owned()]),
        Arc::new(mounts),
    ))
}

/// Walk `root` on one thread; when `swap_in` is given, the sink puts it in force on the first
/// repository found. Answers the repositories found and the walk's directory count.
fn walk_swapping(
    root: &Path,
    gate: &ExclusionGate,
    swap_in: Option<Arc<ExclusionList>>,
) -> (Vec<PathBuf>, u64) {
    let opts = WalkOptions {
        threads: 1,
        ..WalkOptions::default()
    };
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let probe = ProbeCtx::new(&git, StoreKey::new("store-a"), StoreClass::Local, &cancel);
    let ctx = WalkCtx {
        opts: &opts,
        exclusions: gate,
        platform: native_platform(),
        probe: &probe,
        links: links_for(root),
    };
    let pending = Mutex::new(swap_in);
    let found = Mutex::new(Vec::new());
    let stats = walk_root(root, &ctx, &|event| {
        if let WalkEvent::Repo(c) = event {
            found.lock().unwrap().push(c.path);
            let next = pending.lock().unwrap().take();
            if let Some(next) = next {
                let _ = gate.swap(next);
            }
        }
    });
    eprintln!(
        "walk: {} dirs walked, {} repositories found, cancelled {}",
        stats.walked_dirs, stats.repos_found, stats.cancelled
    );
    let mut found = found.into_inner().unwrap();
    found.sort();
    (found, stats.walked_dirs)
}

#[test]
fn an_entry_added_mid_walk_stops_descent_at_the_next_directory_tested() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path();
    let zone = root.join("zone");
    for rel in ["zone/a", "zone/b/c", "zone/d/e"] {
        repo_at(root, rel);
    }

    // Control: nothing is swapped in, so all three are found.
    let (all, _) = walk_swapping(root, &ExclusionGate::default(), None);
    assert_eq!(all.len(), 3, "{all:?}");

    // The list covering `zone` goes in force once the first repository is found. Every other
    // directory under `zone` is tested after that, so none of them is descended or classified.
    let gate = ExclusionGate::default();
    let (found, _) = walk_swapping(root, &gate, Some(covering(&zone)));
    assert_eq!(
        found.len(),
        1,
        "one repository before the swap, none after: {found:?}"
    );
    assert!(found[0].starts_with(&zone));
    let refused = gate
        .checks()
        .iter()
        .filter(|c| **c == (GateSite::WalkDir, false))
        .count();
    eprintln!("walk: {refused} directory test(s) refused after the swap");
    assert!(
        refused >= 2,
        "the two other branches are refused at their test"
    );
}

// ---------------------------------------------------------------------------
// Rows 2 and 4: the hand-off reads nothing and writes nothing for a covered candidate.
// ---------------------------------------------------------------------------

fn found_at(path: &Path) -> Discovered {
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: path.join(".git"),
            common_dir: path.join(".git"),
        },
        root_id: 1,
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

fn locations(index: &Mutex<Index>) -> i64 {
    index
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT COUNT(*) FROM location", [], |r| r.get(0))
        .unwrap()
}

fn seed_root(index: &Mutex<Index>, root: &Path) {
    index
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO scan_root
               (kind, distro, path_bytes, path_key, path_display, added_by, added_at)
             VALUES (?1, '', ?2, ?3, ?4, 'user', 1)",
            rusqlite::params![
                host_kind(),
                path_bytes(root),
                path_key(root, native_platform()),
                path_display(root)
            ],
        )
        .unwrap();
}

fn hand_off_with(
    index: &Mutex<Index>,
    git: &dyn GitBackend,
    gate: &ExclusionGate,
    path: &Path,
) -> Result<(), HandoffError> {
    let cancel = CancelToken::new();
    let ctx = HandoffCtx {
        git,
        cancel: &cancel,
        exclusions: gate,
        store_class: StoreClass::Local,
        generation: 1,
        now: NOW,
    };
    hand_off_discovered(index, &ctx, &found_at(path)).map(|_| ())
}

#[test]
fn a_covered_candidate_is_handed_off_with_no_git_and_no_row() {
    let base = tempfile::tempdir().unwrap();
    let index = Mutex::new(Index::open_at(&base.path().join("index"), NOW).unwrap());
    let root = base.path().join("root");
    let by_path = repo_at(&root, "private/widget");
    let by_name = repo_at(&root, "scratch/gadget");
    let kept = repo_at(&root, "kept");
    seed_root(&index, &root);
    let gate = ExclusionGate::new(ExclusionList::new(
        &["scratch".to_owned()],
        vec![host_entry(&root.join("private"))],
    ));

    for (path, why) in [
        (&by_path, "a path entry"),
        (&by_name, "a user name below the root"),
    ] {
        let git = RecordingGitBackend::new(indexable(&path.join(".git")));
        let err = hand_off_with(&index, &git, &gate, path).unwrap_err();
        assert!(matches!(err, HandoffError::Excluded), "{why}: {err:?}");
        assert!(git.calls().is_empty(), "{why}: no git spawned");
        assert_eq!(locations(&index), 0, "{why}: no location row");
    }
    assert!(gate.checks().contains(&(GateSite::OwnershipProbe, false)));

    // Control: the same gate admits a copy it does not cover, which then reads and writes.
    let git = RecordingGitBackend::new(indexable(&kept.join(".git")));
    hand_off_with(&index, &git, &gate, &kept).unwrap();
    eprintln!(
        "hand-off: covered 0 git calls, 0 rows; kept {} git calls, {} row",
        git.calls().len(),
        locations(&index)
    );
    assert_ne!(git.calls().len(), 0, "the kept copy is read");
    assert_eq!(locations(&index), 1);
    assert!(gate.checks().contains(&(GateSite::Handoff, true)));
}

/// Swaps `next` into `gate` on its first call, then answers as `inner` does: an exclusion added
/// while the hand-off's git reads are running.
#[derive(Debug)]
struct SwapOnFirstCall {
    inner: FakeGitBackend,
    gate: Arc<ExclusionGate>,
    next: Mutex<Option<Arc<ExclusionList>>>,
    swapped: AtomicBool,
}

impl SwapOnFirstCall {
    fn fire(&self) {
        let next = self.next.lock().unwrap().take();
        if let Some(next) = next {
            let _ = self.gate.swap(next);
            self.swapped.store(true, Ordering::SeqCst);
        }
    }
}

#[test]
fn an_entry_added_during_the_hand_offs_reads_stops_its_write() {
    let base = tempfile::tempdir().unwrap();
    let index = Mutex::new(Index::open_at(&base.path().join("index"), NOW).unwrap());
    let path = repo_at(base.path(), "zone/widget");
    let gate = Arc::new(ExclusionGate::default());
    let git = SwapOnFirstCall {
        inner: indexable(&path.join(".git")),
        gate: Arc::clone(&gate),
        next: Mutex::new(Some(covering(&base.path().join("zone")))),
        swapped: AtomicBool::new(false),
    };
    let err = hand_off_with(&index, &git, &gate, &path).unwrap_err();
    assert!(
        git.swapped.load(Ordering::SeqCst),
        "the read check admitted it"
    );
    assert!(matches!(err, HandoffError::Excluded), "{err:?}");
    assert_eq!(
        locations(&index),
        0,
        "the write check refused inside the transaction"
    );
    assert_eq!(
        gate.checks(),
        vec![(GateSite::OwnershipProbe, true), (GateSite::Handoff, false)]
    );
}

// ---------------------------------------------------------------------------
// Row 3: presence of a covered copy is `unscanned`, never `missing`.
// ---------------------------------------------------------------------------

#[test]
fn a_covered_location_is_unscanned_never_missing() {
    let root = Path::new(if cfg!(windows) { "C:\\r" } else { "/r" });
    let roots = [ScanRootRow {
        root_id: 1,
        kind: host_kind().to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(root),
        path_key: path_key(root, native_platform()),
        enabled: true,
        descend_into_repos: false,
    }];
    let row = |id: i64, rel: &str| {
        let p = root.join(rel);
        LocationPresenceRow {
            location_id: id,
            project_id: id,
            path_bytes: path_bytes(&p),
            path_key: path_key(&p, native_platform()),
            store_key: "store-a".to_owned(),
            // Not seen this generation, on a store that was present: `missing` unless covered.
            scan_generation: 1,
            presence: Presence::Present,
        }
    };
    let gate = ExclusionGate::new(ExclusionList::new(
        &["scratch".to_owned()],
        vec![host_entry(&root.join("private"))],
    ));
    let present = BTreeSet::from(["store-a".to_owned()]);
    let ctx = PresenceContext {
        generation: 2,
        roots: &roots,
        exclusions: &gate,
        present_stores: &present,
    };
    let answers = [
        classify_presence(&row(1, "private/widget"), &ctx),
        classify_presence(&row(2, "scratch/gadget"), &ctx),
        classify_presence(&row(3, "kept"), &ctx),
    ];
    eprintln!(
        "presence: path entry {:?}, name {:?}, kept {:?}",
        answers[0], answers[1], answers[2]
    );
    assert_eq!(
        answers,
        [Presence::Unscanned, Presence::Unscanned, Presence::Missing]
    );
    assert!(gate.checks().contains(&(GateSite::Presence, false)));
}

// ---------------------------------------------------------------------------
// The git seam that swaps mid-hand-off: every read passes through to `inner`.
// ---------------------------------------------------------------------------

impl GitBackend for SwapOnFirstCall {
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

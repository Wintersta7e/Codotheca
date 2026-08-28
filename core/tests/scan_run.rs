//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.7 and §4.8 — one scan run.
//!
//! **Deviation from plan 07 Task 10.** The plan reads device identity through
//! `MountResolver::store_key(&Path) -> String` and `volume_key(&Path) -> String`. R4 settled the
//! trait on plan 06's shape — `resolve(&Path) -> Result<MountFacts, MountError>` — and R27 makes
//! `volume_key` an `Option` whose `None` must not become `""`, because a location with no volume
//! key can never be recognised across a remount and inventing one hides that. So `Discovered`
//! carries `Option<String>`, and a root whose mount cannot be resolved is reported rather than
//! walked: without a `store_key` no `location` row can be written for anything found under it.

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::RepoFacts;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::{path_bytes, path_key};
use codotheca_core::scan::discover::RepoKind;
use codotheca_core::scan::presence::{LocationPresenceRow, Presence, PresenceSummary, ScanRootRow};
use codotheca_core::scan::run::{platform_of, ScanMode, ScanRunner};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::{ScanProblemKind, WalkEvent};
use codotheca_core::testing::{
    FakeClock, FakeGitBackend, FakeMountResolver, GitReply, MemScanStore,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const NOW: i64 = 1_700_000_000;

/// A scan root on *this* host carries this host's `kind`, and these fixtures walk real host
/// directories. Declaring `linux` while handing the run `C:\…` paths is not a Windows root: the
/// Unix key rule leaves the backslashes alone, so `is_under` finds no component boundary and
/// every location classifies `unscanned` — which is what the Windows gate caught.
fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// R2: the key is built the way the run builds one for this root's `kind`, through `platform_of`
/// rather than from the host directly. Using the same function on both sides is what makes
/// `d.path_key == key(&repo)` an assertion about the run rather than about the host.
fn key(path: &Path) -> Vec<u8> {
    path_key(path, platform_of(host_kind()))
}

fn repo_at(base: &Path, rel: &str) -> PathBuf {
    let p = base.join(rel);
    std::fs::create_dir_all(p.join(".git")).unwrap();
    p
}

fn root_row(id: i64, path: &Path, enabled: bool) -> ScanRootRow {
    ScanRootRow {
        root_id: id,
        kind: host_kind().to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: key(path),
        enabled,
        descend_into_repos: false,
    }
}

fn mounts_at(base: &Path, store: &str, volume: Option<&str>) -> Arc<FakeMountResolver> {
    let m = FakeMountResolver::new();
    m.map(
        base,
        MountFacts {
            store_key: store.to_owned(),
            volume_key: volume.map(str::to_owned),
            class: StoreClass::Local,
        },
    );
    Arc::new(m)
}

struct Rig {
    store: MemScanStore,
    git: FakeGitBackend,
    clock: FakeClock,
    skip: SkipList,
    cancel: CancelToken,
}

fn rig() -> Rig {
    Rig {
        store: MemScanStore::new(),
        git: FakeGitBackend::new(),
        clock: FakeClock::new(NOW),
        skip: SkipList::default(),
        cancel: CancelToken::new(),
    }
}

fn runner(r: &Rig, mounts: Arc<FakeMountResolver>) -> ScanRunner<'_> {
    ScanRunner {
        store: &r.store,
        git: &r.git,
        mounts,
        clock: &r.clock,
        skip: &r.skip,
        cancel: &r.cancel,
    }
}

#[test]
fn every_discovery_carries_both_device_identities_and_its_root() {
    let base = tempfile::tempdir().unwrap();
    let repo = repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));

    let seen = Mutex::new(Vec::new());
    let outcome = runner
        .run(ScanMode::Full, &|event| {
            if let WalkEvent::Discovered(d) = event {
                seen.lock().unwrap().push(*d);
            }
        })
        .unwrap();

    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 1);
    let d = seen.first().unwrap();
    assert_eq!(d.candidate.path, repo);
    assert_eq!(d.root_id, 1);
    assert_eq!(d.kind, host_kind());
    assert_eq!(d.distro, "");
    assert_eq!(d.store_key, "store-a");
    assert_eq!(d.volume_key.as_deref(), Some("vol-a"));
    assert_eq!(d.path_key, key(&repo));
    assert!(outcome.present_stores.contains("store-a"));
    assert_eq!(outcome.generation, 1);
    assert!(!outcome.cancelled);
}

/// R27: `None` means no stable identifier exists — a bind mount, overlayfs, tmpfs. Mapping it to
/// `""` invents one, and the column is nullable precisely so it does not have to be.
#[test]
fn a_store_with_no_volume_key_reports_absence_rather_than_an_empty_string() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    let runner = runner(&r, mounts_at(base.path(), "store-a", None));

    let seen = Mutex::new(Vec::new());
    runner
        .run(ScanMode::Full, &|event| {
            if let WalkEvent::Discovered(d) = event {
                seen.lock().unwrap().push(*d);
            }
        })
        .unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.first().and_then(|d| d.volume_key.clone()), None);
}

#[test]
fn a_disabled_root_is_not_walked() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), false));
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));
    let outcome = runner.run(ScanMode::Full, &|_| {}).unwrap();
    assert_eq!(outcome.walked_dirs, 0);
    assert!(outcome.present_stores.is_empty());
}

#[test]
fn a_root_that_is_gone_is_reported_and_its_store_is_absent() {
    let base = tempfile::tempdir().unwrap();
    let r = rig();
    r.store
        .push_root(root_row(1, &base.path().join("never-existed"), true));
    let runner = runner(&r, mounts_at(base.path(), "usb", Some("vol-usb")));
    let outcome = runner.run(ScanMode::Full, &|_| {}).unwrap();
    assert!(outcome.present_stores.is_empty());
    assert_eq!(r.store.recorded_problems(), 1);
    assert_eq!(
        r.store.problems().first().map(|p| p.kind),
        Some(ScanProblemKind::OfflineStore)
    );
}

/// Without a `store_key` nothing found under the root could be persisted — `location.store_key`
/// is NOT NULL and R27 forbids inventing one — so the run reports and moves on rather than
/// producing discoveries plan 08 cannot write.
#[test]
fn a_root_whose_mount_cannot_be_resolved_is_reported_and_not_walked() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    // An empty mount table: every resolve is `Unsupported`.
    let runner = runner(&r, Arc::new(FakeMountResolver::new()));

    let seen = Mutex::new(Vec::new());
    let outcome = runner
        .run(ScanMode::Full, &|event| {
            if let WalkEvent::Discovered(d) = event {
                seen.lock().unwrap().push(*d);
            }
        })
        .unwrap();
    assert!(seen.into_inner().unwrap().is_empty());
    assert_eq!(outcome.walked_dirs, 0);
    assert!(outcome.present_stores.is_empty());
    assert_eq!(r.store.recorded_problems(), 1);
}

/// An unplugged drive is `NotMounted`, which is exactly the store-absent case §4.6 turns into
/// `offline` — and it must not read as "the root is gone".
#[test]
fn an_unmounted_volume_makes_its_root_unwalkable_without_deleting_anything() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    r.store.push_location(LocationPresenceRow {
        location_id: 1,
        project_id: 1,
        path_bytes: path_bytes(&base.path().join("p")),
        path_key: key(&base.path().join("p")),
        store_key: "usb".to_owned(),
        scan_generation: 0,
        presence: Presence::Present,
    });
    let mounts = mounts_at(base.path(), "usb", Some("vol-usb"));
    mounts.unmount("vol-usb");

    let runner = runner(&r, mounts);
    let outcome = runner.run(ScanMode::Full, &|_| {}).unwrap();
    assert!(outcome.present_stores.is_empty());
    assert_eq!(r.store.location_count(), 1, "nothing is removed");
    assert_eq!(r.store.presence_of(1), Some(Presence::Offline));
}

#[cfg(unix)]
#[test]
fn a_repository_at_a_non_utf8_path_indexes_and_says_its_displayed_path_is_lossy() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let base = tempfile::tempdir().unwrap();
    let odd = base.path().join(OsStr::from_bytes(b"pro\xffject"));
    std::fs::create_dir_all(odd.join(".git")).unwrap();

    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));

    let found = Mutex::new(Vec::new());
    let problems = Mutex::new(Vec::new());
    runner
        .run(ScanMode::Full, &|event| match event {
            WalkEvent::Discovered(d) => found.lock().unwrap().push(*d),
            WalkEvent::Problem(p) => problems.lock().unwrap().push(p),
            _ => {}
        })
        .unwrap();

    let found = found.into_inner().unwrap();
    assert_eq!(
        found.len(),
        1,
        "criterion 5: it indexes rather than vanishing"
    );
    assert_eq!(
        found.first().map(|d| d.path_bytes.clone()),
        Some(path_bytes(&odd))
    );
    let problems = problems.into_inner().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems.first().map(|p| p.kind),
        Some(ScanProblemKind::NonUtf8Path)
    );
    assert_eq!(
        r.store.recorded_problems(),
        1,
        "and it reaches scan_problem, not just the wire"
    );
}

#[test]
fn a_resumed_run_never_shows_a_counter_below_the_indexed_count() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    for id in 1..=9 {
        r.store.push_location(LocationPresenceRow {
            location_id: id,
            project_id: id,
            path_bytes: path_bytes(&base.path().join(format!("old{id}"))),
            path_key: key(&base.path().join(format!("old{id}"))),
            store_key: "store-a".to_owned(),
            scan_generation: 0,
            presence: Presence::Present,
        });
    }
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));
    let outcome = runner.run(ScanMode::Incremental, &|_| {}).unwrap();
    assert_eq!(outcome.resumed_from, 9);
    assert_eq!(
        outcome.found_repos, 9,
        "one found this run is floored at the indexed count"
    );
}

#[test]
fn a_cancelled_run_leaves_presence_alone_and_records_the_cancellation() {
    let base = tempfile::tempdir().unwrap();
    repo_at(base.path(), "p");
    let r = rig();
    r.store.push_root(root_row(1, base.path(), true));
    r.store.push_location(LocationPresenceRow {
        location_id: 1,
        project_id: 1,
        path_bytes: path_bytes(&base.path().join("elsewhere")),
        path_key: key(&base.path().join("elsewhere")),
        store_key: "store-a".to_owned(),
        scan_generation: 0,
        presence: Presence::Present,
    });
    r.cancel.cancel();
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));
    let outcome = runner.run(ScanMode::Full, &|_| {}).unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.presence, PresenceSummary::default());
    assert_eq!(
        r.store.presence_of(1),
        Some(Presence::Present),
        "a cancelled run must not mark unvisited locations missing"
    );
    assert_eq!(r.store.finished_runs().len(), 1);
    assert!(r.store.finished_runs().first().is_some_and(|f| f.cancelled));
}

/// R2: the root's `kind` decides case-folding, not the host. `win` is the only case-insensitive
/// one; folding a `linux` root's paths on a Windows host merges two locations into one row.
#[test]
fn the_roots_kind_decides_case_folding_and_never_the_host() {
    assert_ne!(
        path_key(Path::new("/r/Repo"), platform_of("linux")),
        path_key(Path::new("/r/repo"), platform_of("linux"))
    );
    assert_eq!(
        path_key(Path::new(r"C:\R\Repo"), platform_of("win")),
        path_key(Path::new("c:/r/repo"), platform_of("win"))
    );
    assert_eq!(platform_of("wsl"), platform_of("linux"));
}

// ---- Task 11: one shelf's worth of shapes -------------------------------------------------

/// Builds, with plain filesystem calls and a scripted git, one of every shape §4.2 names:
///
/// ```text
/// root/
///   work/.git/             an ordinary working tree
///   work/vendor/lib/.git/  a submodule of it, declared in work/.gitmodules
///   wt/.git                a `.git` FILE pointing into work/.git/worktrees/wt
///   archive.git/           bare: HEAD + objects/ + refs/
///   node_modules/pkg/.git/ excluded, and must not appear
///   deep/nested/plain/     no repository anywhere
/// ```
fn build_shelf(base: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let work = base.join("work");
    std::fs::create_dir_all(work.join(".git/worktrees/wt")).unwrap();
    std::fs::write(
        work.join(".gitmodules"),
        b"[submodule \"lib\"]\n\tpath = vendor/lib\n",
    )
    .unwrap();
    let sub = work.join("vendor/lib");
    std::fs::create_dir_all(sub.join(".git")).unwrap();

    let wt = base.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(
        wt.join(".git"),
        format!("gitdir: {}\n", work.join(".git/worktrees/wt").display()).as_bytes(),
    )
    .unwrap();

    let archive = base.join("archive.git");
    std::fs::create_dir_all(archive.join("objects")).unwrap();
    std::fs::create_dir_all(archive.join("refs")).unwrap();
    std::fs::write(archive.join("HEAD"), b"ref: refs/heads/main\n").unwrap();

    std::fs::create_dir_all(base.join("node_modules/pkg/.git")).unwrap();
    std::fs::create_dir_all(base.join("deep/nested/plain")).unwrap();
    (work, sub, wt, archive)
}

/// Scripts `repo_facts` for the two shapes that need git: the linked worktree and the bare one.
fn shelf_git(work: &Path) -> FakeGitBackend {
    let git = FakeGitBackend::new();
    git.on_repo_facts(
        Some("wt"),
        GitReply::Ok(RepoFacts {
            is_bare: false,
            is_shallow: false,
            git_dir: work.join(".git/worktrees/wt"),
            common_dir: work.join(".git"),
        }),
    );
    git.on_repo_facts(
        Some("archive.git"),
        GitReply::Ok(RepoFacts {
            is_bare: true,
            is_shallow: false,
            git_dir: PathBuf::from("/unused"),
            common_dir: PathBuf::from("/unused"),
        }),
    );
    let mut links = BTreeMap::new();
    links.insert(b"vendor/lib".to_vec(), "8f1c2b3d".repeat(5));
    git.always_submodule_gitlinks(GitReply::Ok(links));
    git
}

/// Acceptance criterion 1 in miniature: every repository under the enabled roots appears
/// **exactly once**, with the kind that tells plan 08 what to do with it.
#[test]
fn every_shape_appears_exactly_once_with_the_kind_that_identifies_it() {
    let base = tempfile::tempdir().unwrap();
    let (work, sub, wt, archive) = build_shelf(base.path());

    let r = Rig {
        store: MemScanStore::new(),
        git: shelf_git(&work),
        clock: FakeClock::new(NOW),
        skip: SkipList::default(),
        cancel: CancelToken::new(),
    };
    r.store.push_root(root_row(1, base.path(), true));
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));

    let found = Mutex::new(Vec::new());
    let edges = Mutex::new(Vec::new());
    let outcome = runner
        .run(ScanMode::Full, &|event| match event {
            WalkEvent::Discovered(d) => found
                .lock()
                .unwrap()
                .push((d.candidate.path.clone(), d.candidate.kind)),
            WalkEvent::SubmoduleEdge(e) => edges.lock().unwrap().push(*e),
            _ => {}
        })
        .unwrap();

    let mut found = found.into_inner().unwrap();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let mut want = vec![
        (archive, RepoKind::Bare),
        (work.clone(), RepoKind::WorkTree),
        (sub.clone(), RepoKind::WorkTree),
        (wt, RepoKind::LinkedWorktree),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(found, want);

    let edges = edges.into_inner().unwrap();
    assert_eq!(edges.len(), 1);
    let edge = edges.first().unwrap();
    assert_eq!(edge.parent_worktree, work);
    assert_eq!(edge.child_worktree, sub);
    assert_eq!(edge.path_bytes, b"vendor/lib".to_vec());
    assert!(
        edge.gitlink_oid.is_some(),
        "the parent index names a commit"
    );

    assert_eq!(outcome.found_repos, 4);
    assert!(!outcome.cancelled);
    assert!(outcome.isolated_roots.is_empty());
    // The excluded tree was never read, so its `.git` never became a repository.
    assert!(found
        .iter()
        .all(|(p, _)| !p.to_string_lossy().contains("node_modules")));
}

/// §17: phase 1 has no destructive operation. `apply_presence` reclassifies four ways and the
/// store has no removal method at all, so a full offline sweep still holds every row.
#[test]
fn nothing_in_the_scanner_can_remove_a_row() {
    let base = tempfile::tempdir().unwrap();
    let (work, _, _, _) = build_shelf(base.path());

    let r = Rig {
        store: MemScanStore::new(),
        git: shelf_git(&work),
        clock: FakeClock::new(NOW),
        skip: SkipList::default(),
        cancel: CancelToken::new(),
    };
    r.store.push_root(root_row(1, base.path(), true));
    for id in 1..=4 {
        r.store.push_location(LocationPresenceRow {
            location_id: id,
            project_id: id,
            path_bytes: path_bytes(&base.path().join(format!("gone{id}"))),
            path_key: key(&base.path().join(format!("gone{id}"))),
            store_key: "vanished".to_owned(),
            scan_generation: 0,
            presence: Presence::Present,
        });
    }
    let before = r.store.location_count();
    let runner = runner(&r, mounts_at(base.path(), "store-a", Some("vol-a")));
    let outcome = runner.run(ScanMode::Full, &|_| {}).unwrap();

    assert_eq!(r.store.location_count(), before);
    assert_eq!(outcome.presence.offline, 4);
    assert_eq!(outcome.presence.offline_projects, 4);
    for id in 1..=4 {
        assert_eq!(r.store.presence_of(id), Some(Presence::Offline));
    }
}

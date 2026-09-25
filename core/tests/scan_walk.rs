//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! J0 — the walk. Four rules in this order: never descend into a directory named `.git`; never
//! traverse the WSL bridge; apply the exclusion list to descendants only; classify, then stop
//! unless `descend_into_repos` is set.

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{RepoFacts, StoreKey};
use codotheca_core::index::path::native_platform;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::scan::discover::{ProbeCtx, RepoKind};
use codotheca_core::scan::links::LinkPolicy;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::walk::{walk_root, WalkCtx};
use codotheca_core::scan::{WalkEvent, WalkOptions};
use codotheca_core::testing::{FakeGitBackend, FakeMountResolver, GitReply};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn repo_at(base: &Path, rel: &str) -> PathBuf {
    let p = base.join(rel);
    std::fs::create_dir_all(p.join(".git")).unwrap();
    std::fs::write(p.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(p.join(".git/objects")).unwrap();
    std::fs::create_dir_all(p.join(".git/refs")).unwrap();
    p
}

fn bare_at(base: &Path, rel: &str) -> PathBuf {
    let p = base.join(rel);
    std::fs::create_dir_all(p.join("objects")).unwrap();
    std::fs::create_dir_all(p.join("refs")).unwrap();
    std::fs::write(p.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    p
}

/// Every `rev-parse` answers "bare", which is what makes the `.git` guard the only thing between
/// one repository and two.
fn bare_saying_git() -> FakeGitBackend {
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: true,
        is_shallow: false,
        git_dir: PathBuf::from("/unused"),
        common_dir: PathBuf::from("/unused"),
    }));
    git
}

fn links_for(root: &Path, opts: &WalkOptions) -> Arc<LinkPolicy> {
    let mut stores = BTreeSet::new();
    stores.insert("store-a".to_owned());
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
        opts.follow_links,
        vec![path_key(root, native_platform())],
        stores,
        Arc::new(mounts),
    ))
}

/// Walk `root` and return every repository found, sorted, plus the directory count.
fn collect(root: &Path, opts: WalkOptions) -> (Vec<(PathBuf, RepoKind)>, u64) {
    let skip = SkipList::default();
    let git = bare_saying_git();
    let cancel = CancelToken::new();
    let probe = ProbeCtx::new(&git, StoreKey::new("store-a"), StoreClass::Local, &cancel);
    let ctx = WalkCtx {
        opts: &opts,
        skip: &skip,
        probe: &probe,
        links: links_for(root, &opts),
    };
    let found = Mutex::new(Vec::new());
    let stats = walk_root(root, &ctx, &|event| {
        if let WalkEvent::Repo(c) = event {
            found.lock().unwrap().push((c.path, c.kind));
        }
    });
    let mut v = found.into_inner().unwrap();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    (v, stats.walked_dirs)
}

#[test]
fn a_repository_is_found_exactly_once() {
    let base = tempfile::tempdir().unwrap();
    let repo = repo_at(base.path(), "p");
    let (found, _) = collect(base.path(), WalkOptions::default());
    assert_eq!(found, vec![(repo, RepoKind::WorkTree)]);
}

/// **The `.git` guard, gated where it is actually reachable.**
///
/// Plan 07 Task 8 step 5 names `a_repository_is_found_once_and_its_dot_git_is_never_entered` as
/// the test that fails when the guard is removed. It does not: with `descend_into_repos` off the
/// walk returns `Skip` at the repository root and never reaches its `.git` at all, so the guard
/// is unreachable and the test is green either way — a bar written past the defect. The guard
/// matters exactly when descent is on, and that is what this asserts. Verified by deleting the
/// three-line guard: this test then reports a phantom `Bare` at every `.git`.
#[test]
fn descending_into_repositories_still_never_enters_a_dot_git() {
    let base = tempfile::tempdir().unwrap();
    let outer = repo_at(base.path(), "p");
    let inner = repo_at(base.path(), "p/inner");
    let (found, _) = collect(
        base.path(),
        WalkOptions {
            descend_into_repos: true,
            ..WalkOptions::default()
        },
    );
    assert_eq!(
        found,
        vec![(outer, RepoKind::WorkTree), (inner, RepoKind::WorkTree)],
        "a `.git` has the bare shape, so entering one invents a second repository for each real one"
    );
}

#[test]
fn the_walk_stops_at_a_repository_root_unless_told_to_descend() {
    let base = tempfile::tempdir().unwrap();
    let outer = repo_at(base.path(), "p");
    let inner = repo_at(base.path(), "p/inner");
    let (stopped, _) = collect(base.path(), WalkOptions::default());
    assert_eq!(stopped, vec![(outer.clone(), RepoKind::WorkTree)]);

    let (descended, _) = collect(
        base.path(),
        WalkOptions {
            descend_into_repos: true,
            ..WalkOptions::default()
        },
    );
    assert_eq!(
        descended,
        vec![(outer, RepoKind::WorkTree), (inner, RepoKind::WorkTree)]
    );
}

#[test]
fn an_excluded_directory_is_never_read_but_the_root_itself_always_is() {
    let base = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base.path().join("node_modules/pkg")).unwrap();
    repo_at(base.path(), "node_modules/pkg/dep");
    let keep = repo_at(base.path(), "p");
    let (found, _) = collect(base.path(), WalkOptions::default());
    assert_eq!(found, vec![(keep, RepoKind::WorkTree)]);

    // A root named `build` is a root, not an exclusion.
    let rooted = tempfile::tempdir().unwrap();
    let inside = repo_at(rooted.path(), "x");
    let build_root = rooted.path().join("build");
    std::fs::create_dir_all(&build_root).unwrap();
    let in_build = repo_at(&build_root, "y");
    let (a, _) = collect(&build_root, WalkOptions::default());
    assert_eq!(a, vec![(in_build, RepoKind::WorkTree)]);
    let (b, _) = collect(rooted.path(), WalkOptions::default());
    assert_eq!(b, vec![(inside, RepoKind::WorkTree)]);
}

#[test]
fn a_bare_repository_beside_a_working_one_is_found_and_the_dot_git_is_not() {
    let base = tempfile::tempdir().unwrap();
    let work = repo_at(base.path(), "w");
    let mirror = bare_at(base.path(), "b.git");
    let (found, _) = collect(base.path(), WalkOptions::default());
    let mut want = vec![(mirror, RepoKind::Bare), (work, RepoKind::WorkTree)];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(found, want);
}

#[test]
fn cancellation_stops_the_walk() {
    let base = tempfile::tempdir().unwrap();
    for i in 0..40 {
        repo_at(base.path(), &format!("p{i}"));
    }
    let opts = WalkOptions {
        threads: 1,
        ..WalkOptions::default()
    };
    let skip = SkipList::default();
    let git = bare_saying_git();
    let cancel = CancelToken::new();
    cancel.cancel();
    let probe = ProbeCtx::new(&git, StoreKey::new("store-a"), StoreClass::Local, &cancel);
    let ctx = WalkCtx {
        opts: &opts,
        skip: &skip,
        probe: &probe,
        links: links_for(base.path(), &opts),
    };
    let stats = walk_root(base.path(), &ctx, &|_| {});
    assert!(stats.cancelled);
    assert_eq!(stats.repos_found, 0);
}

#[test]
fn walked_directories_are_counted() {
    let base = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base.path().join("a/b/c")).unwrap();
    let (_, walked) = collect(base.path(), WalkOptions::default());
    assert_eq!(walked, 4, "the root plus a, a/b, a/b/c");
}

/// §4.3's standard filters are gitignore behaviours. This is a filesystem census, not a git
/// operation: a repository listed in a `.gitignore` is still a repository the user owns.
#[test]
fn a_gitignored_directory_is_still_walked() {
    let base = tempfile::tempdir().unwrap();
    std::fs::write(base.path().join(".gitignore"), b"hidden/\n").unwrap();
    let hidden = repo_at(base.path(), "hidden");
    let (found, _) = collect(base.path(), WalkOptions::default());
    assert_eq!(found, vec![(hidden, RepoKind::WorkTree)]);
}

/// A dot-directory is not hidden from a census either — `.config/…` holds real repositories.
#[test]
fn a_dot_directory_that_is_not_dot_git_is_walked() {
    let base = tempfile::tempdir().unwrap();
    let repo = repo_at(base.path(), ".local-projects/p");
    let (found, _) = collect(base.path(), WalkOptions::default());
    assert_eq!(found, vec![(repo, RepoKind::WorkTree)]);
}

/// §4.4: a submodule is never reached by descent, so the walk enumerates it explicitly and the
/// edge reaches the sink even though the walk stopped at the superproject's root.
#[test]
fn a_submodule_under_a_repository_root_is_reached_without_descending() {
    let base = tempfile::tempdir().unwrap();
    let parent = repo_at(base.path(), "p");
    std::fs::write(
        parent.join(".gitmodules"),
        b"[submodule \"lib\"]\n\tpath = vendor/lib\n",
    )
    .unwrap();
    let child = parent.join("vendor/lib");
    std::fs::create_dir_all(child.join(".git")).unwrap();

    let opts = WalkOptions::default();
    let skip = SkipList::default();
    let git = bare_saying_git();
    let cancel = CancelToken::new();
    let probe = ProbeCtx::new(&git, StoreKey::new("store-a"), StoreClass::Local, &cancel);
    let ctx = WalkCtx {
        opts: &opts,
        skip: &skip,
        probe: &probe,
        links: links_for(base.path(), &opts),
    };
    let repos = Mutex::new(Vec::new());
    let edges = Mutex::new(Vec::new());
    let stats = walk_root(base.path(), &ctx, &|event| match event {
        WalkEvent::Repo(c) => repos.lock().unwrap().push(c.path),
        WalkEvent::SubmoduleEdge(e) => edges.lock().unwrap().push(*e),
        _ => {}
    });
    let mut repos = repos.into_inner().unwrap();
    repos.sort();
    assert_eq!(repos, vec![parent, child.clone()]);
    let edges = edges.into_inner().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].child_worktree, child);
    assert_eq!(stats.repos_found, 2);
}
